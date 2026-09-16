//! Pure bounded transfer planning for the synthetic F1 harness (not radio routing).
use mesh_object::Manifest;
use mesh_protocol::DurableRecord;
use mesh_types::durable::*;
use std::collections::BTreeMap;

/// The field profile keeps at most two authenticated radio neighbors. This is
/// deliberately independent from the roster size: a 50-member group relays
/// through a sparse graph rather than opening 49 links per phone.
pub const MAX_DIRECT_NEIGHBORS: usize = 2;
/// Logical lifetime of one signed presence heartbeat. Callers inject the clock;
/// this crate never reads wall time.
pub const PRESENCE_TTL: u64 = 15;
pub const MAX_PRESENCE_HOPS: u8 = 16;
/// A field object may traverse a bounded number of relays. This protects a
/// partitioned group from an indefinitely circulating frame while allowing a
/// sparse 50-member topology to find a path through several vehicles.
pub const MAX_RELAY_HOPS: u8 = 16;
/// The relay cache is deliberately finite. Durable custody belongs to the
/// store; this cache only suppresses repeated forwarding on live links.
pub const MAX_RELAY_DEDUP: usize = 512;
pub const RELAY_FRAME_BYTES: usize = 91;
const RELAY_FRAME_VERSION: u8 = 1;
/// Canonical envelope for one durable record travelling through the relay
/// overlay. Keeping the frame next to every record makes an authenticated
/// host boundary explicit: a chunk or receipt cannot be forwarded as an
/// un-routed payload by accident.
pub const ROUTED_RECORD_MAGIC: u8 = 0x72;
pub const ROUTED_RECORD_VERSION: u8 = 1;
pub const MAX_ROUTED_RECORD: usize = 2 + RELAY_FRAME_BYTES + mesh_protocol::MAX_DURABLE_RECORD;

/// Deterministically partitions a group-wide logical message into the bounded
/// recipient audiences accepted by one durable encrypted object. The local
/// origin never receives a redundant self-copy. For a 50-member roster this
/// returns five groups (10 + 10 + 10 + 10 + 9), which the host binds to one
/// logical chat ID before presenting it in the UI.
///
/// The input may arrive in any order because a verified roster has no implied
/// database order. Duplicates, a roster larger than the field profile, or an
/// origin outside the roster are rejected rather than producing audience sets
/// which two devices could partition differently.
pub fn broadcast_audiences(local: MemberId, members: &[MemberId]) -> Result<Vec<Vec<MemberId>>> {
    if members.is_empty() || members.len() > MAX_GROUP_MEMBERS {
        return Err(DurableError::InvalidInput);
    }
    let mut roster = members.to_vec();
    roster.sort_unstable();
    if !roster.windows(2).all(|pair| pair[0] < pair[1]) || roster.binary_search(&local).is_err() {
        return Err(DurableError::InvalidInput);
    }
    Ok(roster
        .into_iter()
        .filter(|member| *member != local)
        .collect::<Vec<_>>()
        .chunks(MAX_TARGETS)
        .map(|audience| audience.to_vec())
        .collect())
}

/// The origin supplies this random operation identifier. It is always paired
/// with `origin` below, so two phones choosing the same bytes cannot suppress
/// each other's object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RelayId(pub [u8; 16]);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct RelayKey {
    origin: MemberId,
    id: RelayId,
}

/// Routing metadata which travels outside the protected object bytes. A relay
/// verifies the authenticated envelope before giving this to `RelayCache`; it
/// never creates or changes the object's original content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RelayFrame {
    pub id: RelayId,
    pub origin: MemberId,
    pub previous_hop: MemberId,
    pub hops: u8,
    pub hop_limit: u8,
    pub expires_at: u64,
}

impl RelayFrame {
    /// Fixed-width, canonical host boundary encoding. The encrypted object is
    /// carried separately; this metadata is authenticated by the per-neighbor
    /// session before a host may decode it.
    pub fn encode(self) -> [u8; RELAY_FRAME_BYTES] {
        let mut bytes = [0; RELAY_FRAME_BYTES];
        bytes[0] = RELAY_FRAME_VERSION;
        bytes[1..17].copy_from_slice(&self.id.0);
        bytes[17..49].copy_from_slice(&self.origin.0);
        bytes[49..81].copy_from_slice(&self.previous_hop.0);
        bytes[81] = self.hops;
        bytes[82] = self.hop_limit;
        bytes[83..91].copy_from_slice(&self.expires_at.to_be_bytes());
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != RELAY_FRAME_BYTES || bytes[0] != RELAY_FRAME_VERSION {
            return Err(DurableError::InvalidInput);
        }
        let mut id = [0; 16];
        let mut origin = [0; 32];
        let mut previous_hop = [0; 32];
        id.copy_from_slice(&bytes[1..17]);
        origin.copy_from_slice(&bytes[17..49]);
        previous_hop.copy_from_slice(&bytes[49..81]);
        let frame = Self {
            id: RelayId(id),
            origin: MemberId(origin),
            previous_hop: MemberId(previous_hop),
            hops: bytes[81],
            hop_limit: bytes[82],
            expires_at: u64::from_be_bytes(
                bytes[83..91]
                    .try_into()
                    .map_err(|_| DurableError::InvalidInput)?,
            ),
        };
        if frame.hop_limit == 0
            || frame.hop_limit > MAX_RELAY_HOPS
            || frame.hops > frame.hop_limit
            || frame.expires_at > MAX_LOGICAL_TIME
        {
            return Err(DurableError::InvalidInput);
        }
        Ok(frame)
    }
}

/// A bounded relay envelope around an already canonical application record.
/// The neighboring Noise session authenticates this envelope; `RelayCache`
/// still decides only once per relay id, when the announcement is accepted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoutedRecord {
    pub frame: RelayFrame,
    pub record: DurableRecord,
}

impl RoutedRecord {
    pub fn encode(&self) -> Result<Vec<u8>> {
        let record = self.record.encode()?;
        let mut out = Vec::with_capacity(2 + RELAY_FRAME_BYTES + record.len());
        out.push(ROUTED_RECORD_MAGIC);
        out.push(ROUTED_RECORD_VERSION);
        out.extend(self.frame.encode());
        out.extend(record);
        Ok(out)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 2 + RELAY_FRAME_BYTES
            || bytes.len() > MAX_ROUTED_RECORD
            || bytes[0] != ROUTED_RECORD_MAGIC
            || bytes[1] != ROUTED_RECORD_VERSION
        {
            return Err(DurableError::InvalidInput);
        }
        Ok(Self {
            frame: RelayFrame::decode(&bytes[2..2 + RELAY_FRAME_BYTES])?,
            record: DurableRecord::decode(&bytes[2 + RELAY_FRAME_BYTES..])?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelayDecision {
    /// Persist/custody the opaque object, then transmit this updated metadata
    /// only to useful neighbors other than `previous_hop`.
    Forward(RelayFrame),
    Duplicate,
    Expired,
    HopLimit,
}

#[derive(Clone, Copy)]
struct SeenRelay {
    expires_at: u64,
    received_at: u64,
}

/// Bounded, deterministic forwarding gate for group objects. It contains no
/// radio policy: hosts choose healthy neighbors, while this reducer guarantees
/// that a valid object has at most one forward decision per phone.
pub struct RelayCache {
    local: MemberId,
    seen: BTreeMap<RelayKey, SeenRelay>,
}

impl RelayCache {
    pub fn new(local: MemberId) -> Self {
        Self {
            local,
            seen: BTreeMap::new(),
        }
    }

    fn valid(frame: RelayFrame, via: MemberId, now: u64) -> Result<()> {
        if now > MAX_LOGICAL_TIME
            || frame.expires_at > MAX_LOGICAL_TIME
            || frame.previous_hop != via
            || frame.hop_limit == 0
            || frame.hop_limit > MAX_RELAY_HOPS
            || frame.hops > frame.hop_limit
        {
            return Err(DurableError::InvalidInput);
        }
        Ok(())
    }

    fn expire_seen(&mut self, now: u64) {
        self.seen.retain(|_, seen| now < seen.expires_at);
    }

    fn reserve(&mut self, key: RelayKey, expires_at: u64, now: u64) {
        if self.seen.len() >= MAX_RELAY_DEDUP {
            // Deterministic eviction chooses the earliest expiry, then the
            // earliest observed frame. This never depends on hash iteration.
            if let Some(oldest) = self
                .seen
                .iter()
                .min_by_key(|(_, seen)| (seen.expires_at, seen.received_at))
                .map(|(key, _)| *key)
            {
                self.seen.remove(&oldest);
            }
        }
        self.seen.insert(
            key,
            SeenRelay {
                expires_at,
                received_at: now,
            },
        );
    }

    /// Decides whether a verified frame may be relayed once. The caller must
    /// retain the object durably before acting on `Forward`.
    pub fn accept(&mut self, frame: RelayFrame, via: MemberId, now: u64) -> Result<RelayDecision> {
        Self::valid(frame, via, now)?;
        if frame.origin == self.local || via == self.local {
            return Err(DurableError::InvalidInput);
        }
        self.expire_seen(now);
        if now >= frame.expires_at {
            return Ok(RelayDecision::Expired);
        }
        if frame.hops >= frame.hop_limit {
            return Ok(RelayDecision::HopLimit);
        }
        let key = RelayKey {
            origin: frame.origin,
            id: frame.id,
        };
        if self.seen.contains_key(&key) {
            return Ok(RelayDecision::Duplicate);
        }
        self.reserve(key, frame.expires_at, now);
        Ok(RelayDecision::Forward(RelayFrame {
            previous_hop: self.local,
            hops: frame.hops + 1,
            ..frame
        }))
    }

    pub fn len(&self) -> usize {
        self.seen.len()
    }

    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }
}

/// Desired direct overlay for one certified member. This is a plan, never a
/// claim that a radio link is healthy: native adapters still authenticate the
/// peer and report availability before opening a transport.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NeighborPlan {
    /// At most two simultaneous Wi-Fi Aware NDP candidates. Across an even
    /// roster these predecessor/successor plans form one deterministic ring.
    pub wifi_aware: Vec<MemberId>,
    /// A distinct authenticated Bluetooth edge that shortens the ring diameter
    /// while preserving the Wi-Fi Aware hardware budget.
    pub bluetooth_fallback: Option<MemberId>,
}

/// Physical bearer of an already authenticated neighbor link. The protocol
/// does not infer this from a scan result: a native adapter may report it only
/// after its Noise session is established.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TransportKind {
    WifiAware,
    Bluetooth,
}

/// Fresh link facts supplied by a native radio adapter. `eta_millis` is a
/// bounded local estimate, not a delivery receipt; the durable outbox remains
/// authoritative for progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransportCandidate {
    pub peer: MemberId,
    pub transport: TransportKind,
    pub authenticated: bool,
    pub healthy: bool,
    pub eta_millis: u32,
}

/// One eligible bearer selected for a durable object. It deliberately exposes
/// no socket, address, key or routing table to the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransportChoice {
    pub peer: MemberId,
    pub transport: TransportKind,
    pub eta_millis: u32,
}

/// The decision for the next durable object attempt. `Switch` is emitted only
/// when both the old and new links are already authenticated and healthy, so a
/// host can keep the old bearer until the new one is ready (make-before-break).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportDecision {
    NoRoute,
    Send(TransportChoice),
    Keep(TransportChoice),
    Switch {
        from: TransportChoice,
        to: TransportChoice,
    },
}

fn transport_rank(kind: TransportKind) -> u8 {
    match kind {
        TransportKind::WifiAware => 0,
        TransportKind::Bluetooth => 1,
    }
}

fn candidate_choice(candidate: TransportCandidate) -> TransportChoice {
    TransportChoice {
        peer: candidate.peer,
        transport: candidate.transport,
        eta_millis: candidate.eta_millis,
    }
}

/// Selects a single authenticated next hop for one durable attempt. The
/// previous relay hop is never selected, preventing an immediate loop even if
/// it has the best local signal. A lower ETA wins; equal estimates choose
/// Wi-Fi Aware before Bluetooth and then the stable member ID.
///
/// `current` is the bearer used by the previous attempt for this object. Its
/// presence in `candidates` proves it remains healthy. When a better new link
/// is ready, the caller receives `Switch`; if the current link already died,
/// it receives `Send`, which honestly records that no make-before-break was
/// possible.
pub fn select_transport(
    local: MemberId,
    current: Option<TransportChoice>,
    candidates: &[TransportCandidate],
    received_from: Option<MemberId>,
) -> Result<TransportDecision> {
    if received_from == Some(local) {
        return Err(DurableError::InvalidInput);
    }
    let mut eligible = Vec::new();
    for candidate in candidates {
        if candidate.peer == local {
            return Err(DurableError::InvalidInput);
        }
        if !candidate.authenticated || !candidate.healthy || received_from == Some(candidate.peer) {
            continue;
        }
        if candidate.eta_millis == 0 {
            return Err(DurableError::InvalidInput);
        }
        if eligible.iter().any(|existing: &TransportCandidate| {
            existing.peer == candidate.peer && existing.transport == candidate.transport
        }) {
            return Err(DurableError::InvalidInput);
        }
        eligible.push(*candidate);
    }
    eligible.sort_unstable_by_key(|candidate| {
        (
            candidate.eta_millis,
            transport_rank(candidate.transport),
            candidate.peer,
        )
    });
    let Some(next) = eligible.first().copied().map(candidate_choice) else {
        return Ok(TransportDecision::NoRoute);
    };
    let Some(current) = current else {
        return Ok(TransportDecision::Send(next));
    };
    let current_live = eligible.iter().any(|candidate| {
        candidate.peer == current.peer && candidate.transport == current.transport
    });
    if !current_live {
        return Ok(TransportDecision::Send(next));
    }
    if current.peer == next.peer && current.transport == next.transport {
        return Ok(TransportDecision::Keep(next));
    }
    Ok(TransportDecision::Switch {
        from: current,
        to: next,
    })
}

/// Derives the same sparse overlay on every member from the certified roster,
/// regardless of input order. For a 50-member roster it produces exactly two
/// Wi-Fi Aware candidates and one opposite-member Bluetooth chord per member;
/// the resulting graph has diameter 13 rather than the ring's diameter 25.
///
/// This function deliberately does not score radio signal. The future neighbor
/// controller may temporarily replace an unreachable candidate with a healthy
/// authenticated peer, but must retain this plan as the convergence target.
pub fn neighbor_plan(local: MemberId, members: &[MemberId]) -> Result<NeighborPlan> {
    if members.is_empty() || members.len() > MAX_GROUP_MEMBERS {
        return Err(DurableError::InvalidInput);
    }
    let mut roster = members.to_vec();
    roster.sort_unstable();
    if !roster.windows(2).all(|pair| pair[0] < pair[1]) {
        return Err(DurableError::InvalidInput);
    }
    let index = roster
        .binary_search(&local)
        .map_err(|_| DurableError::InvalidInput)?;
    let count = roster.len();
    let mut wifi_aware = Vec::with_capacity(MAX_DIRECT_NEIGHBORS);
    match count {
        1 => {}
        2 => wifi_aware.push(roster[(index + 1) % count]),
        _ => {
            wifi_aware.push(roster[(index + count - 1) % count]);
            wifi_aware.push(roster[(index + 1) % count]);
        }
    }
    let bluetooth_fallback = if count >= 4 && count.is_multiple_of(2) {
        Some(roster[(index + count / 2) % count])
    } else {
        None
    };
    Ok(NeighborPlan {
        wifi_aware,
        bluetooth_fallback,
    })
}

/// Monotonic origin version carried by a signed presence announcement. A new
/// device incarnation is ordered above every sequence from its old process.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PresenceVersion {
    pub incarnation: u64,
    pub sequence: u64,
}

/// The authenticated frame layer must verify this record's signature and group
/// membership before passing it to `PresenceTable`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PresenceClaim {
    pub member: MemberId,
    pub version: PresenceVersion,
}

/// Current locally observable route to a group member. `Direct` means an
/// authenticated link is live now; `Routed` is a fresh signed claim relayed by
/// an authenticated neighbor. It never means that an unverified radio sighting
/// is a member of the group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresencePath {
    Direct,
    Routed { via: MemberId, hops: u8 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PresenceEntry {
    pub member: MemberId,
    pub version: Option<PresenceVersion>,
    pub path: PresencePath,
}

#[derive(Clone, Copy)]
struct RoutedPresence {
    version: PresenceVersion,
    via: MemberId,
    hops: u8,
    received_at: u64,
}

#[derive(Default)]
struct KnownMember {
    direct_at: Option<u64>,
    routed: Option<RoutedPresence>,
}

/// Bounded presence reducer for the future authenticated relay layer. It is
/// intentionally transport-agnostic: Kotlin and Swift may report radio facts,
/// but only an authenticated Rust session may call the observation methods.
pub struct PresenceTable {
    local: MemberId,
    members: BTreeMap<MemberId, KnownMember>,
}

impl PresenceTable {
    pub fn new(local: MemberId) -> Self {
        Self {
            local,
            members: BTreeMap::new(),
        }
    }

    fn valid_time(now: u64) -> Result<()> {
        if now > MAX_LOGICAL_TIME {
            Err(DurableError::InvalidInput)
        } else {
            Ok(())
        }
    }

    fn fresh(seen_at: u64, now: u64) -> bool {
        now >= seen_at && now - seen_at <= PRESENCE_TTL
    }

    fn direct_neighbors(&self, now: u64) -> usize {
        self.members
            .values()
            .filter(|known| known.direct_at.is_some_and(|seen| Self::fresh(seen, now)))
            .count()
    }

    fn reserve_member(&mut self, member: MemberId, now: u64) -> Result<&mut KnownMember> {
        self.expire(now)?;
        if !self.members.contains_key(&member) && self.members.len() >= MAX_GROUP_MEMBERS {
            return Err(DurableError::ResourcePressure);
        }
        Ok(self.members.entry(member).or_default())
    }

    /// Registers an already-authenticated direct neighbor heartbeat. The caller
    /// may refresh an existing neighbor, but a third simultaneous direct link is
    /// rejected before it can alter routing state.
    pub fn observe_direct(&mut self, member: MemberId, now: u64) -> Result<bool> {
        Self::valid_time(now)?;
        if member == self.local {
            return Err(DurableError::InvalidInput);
        }
        let already_direct = self
            .members
            .get(&member)
            .and_then(|known| known.direct_at)
            .is_some_and(|seen| Self::fresh(seen, now));
        if !already_direct && self.direct_neighbors(now) >= MAX_DIRECT_NEIGHBORS {
            return Err(DurableError::ResourcePressure);
        }
        let known = self.reserve_member(member, now)?;
        let changed = known.direct_at != Some(now);
        known.direct_at = Some(now);
        Ok(changed)
    }

    /// Removes only the direct edge. A fresh relayed path, if present, remains
    /// usable; this is the make-before-break behavior needed for radio failover.
    pub fn disconnect_direct(&mut self, member: MemberId) -> bool {
        self.members
            .get_mut(&member)
            .and_then(|known| known.direct_at.take())
            .is_some()
    }

    /// Accepts a *previously verified* presence claim received through `via`.
    /// The origin version is monotonic; stale claims cannot resurrect a member.
    /// Equal versions only replace the route when they reduce hops.
    pub fn observe_relay(
        &mut self,
        claim: PresenceClaim,
        via: MemberId,
        hops: u8,
        now: u64,
    ) -> Result<bool> {
        Self::valid_time(now)?;
        if claim.member == self.local
            || claim.member == via
            || claim.version.incarnation == 0
            || claim.version.sequence == 0
            || !(2..=MAX_PRESENCE_HOPS).contains(&hops)
        {
            return Err(DurableError::InvalidInput);
        }
        let known = self.reserve_member(claim.member, now)?;
        let replacement = match known.routed {
            None => true,
            Some(old) if claim.version > old.version => true,
            Some(old) if claim.version == old.version && hops < old.hops => true,
            _ => false,
        };
        if replacement {
            known.routed = Some(RoutedPresence {
                version: claim.version,
                via,
                hops,
                received_at: now,
            });
        }
        Ok(replacement)
    }

    /// Returns the preferred path: fresh direct links win; otherwise the best
    /// fresh signed relayed claim is exposed. This function does not mutate
    /// state, which makes UI snapshots deterministic under an injected clock.
    pub fn member(&self, member: MemberId, now: u64) -> Result<Option<PresenceEntry>> {
        Self::valid_time(now)?;
        let Some(known) = self.members.get(&member) else {
            return Ok(None);
        };
        if known.direct_at.is_some_and(|seen| Self::fresh(seen, now)) {
            return Ok(Some(PresenceEntry {
                member,
                version: known.routed.map(|route| route.version),
                path: PresencePath::Direct,
            }));
        }
        let Some(route) = known
            .routed
            .filter(|route| Self::fresh(route.received_at, now))
        else {
            return Ok(None);
        };
        Ok(Some(PresenceEntry {
            member,
            version: Some(route.version),
            path: PresencePath::Routed {
                via: route.via,
                hops: route.hops,
            },
        }))
    }

    pub fn active(&self, now: u64) -> Result<Vec<PresenceEntry>> {
        Self::valid_time(now)?;
        self.members
            .keys()
            .copied()
            .map(|member| self.member(member, now))
            .filter_map(|entry| entry.transpose())
            .collect()
    }

    /// Drops contacts only after all direct and relayed evidence is stale. The
    /// returned IDs are the exact transition that the route/UI layer broadcasts
    /// as disconnected; no timer silently reports a still-live member as gone.
    pub fn expire(&mut self, now: u64) -> Result<Vec<MemberId>> {
        Self::valid_time(now)?;
        let expired: Vec<_> = self
            .members
            .iter()
            .filter_map(|(member, known)| {
                let direct_live = known.direct_at.is_some_and(|seen| Self::fresh(seen, now));
                let routed_live = known
                    .routed
                    .is_some_and(|route| Self::fresh(route.received_at, now));
                (!direct_live && !routed_live).then_some(*member)
            })
            .collect();
        for member in &expired {
            self.members.remove(member);
        }
        Ok(expired)
    }
}

/// Select missing chunks in index order. No elapsed time, I/O or randomness here.
/// `hops` is the already-traversed count supplied by the future authenticated route layer.
pub fn plan(
    manifest: &Manifest,
    missing: &[usize],
    now: u64,
    hops: u8,
    byte_budget: usize,
) -> Result<Vec<usize>> {
    if now > MAX_LOGICAL_TIME
        || missing.len() > MAX_CHUNKS
        || !missing.windows(2).all(|v| v[0] < v[1])
        || missing.iter().any(|i| *i >= manifest.chunk_count())
    {
        return Err(DurableError::InvalidInput);
    }
    if now >= manifest.expires_at() {
        return Err(DurableError::Expired);
    }
    if hops >= manifest.hop_limit() {
        return Ok(Vec::new());
    }
    let mut budget = byte_budget;
    let mut result = Vec::new();
    for &index in missing {
        let length = (manifest.content_len() - index * CHUNK_BYTES).min(CHUNK_BYTES);
        if length > budget {
            break;
        }
        budget -= length;
        result.push(index);
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    use mesh_object::{ObjectPolicy, PreparedObject};
    use std::collections::VecDeque;
    #[test]
    fn budgets_expiry_hops_and_invalid_inventory() {
        let o = PreparedObject::from_opaque(
            MemberId([1; 32]),
            1,
            ObjectPolicy {
                namespace: Namespace::new("synthetic").unwrap(),
                epoch: 1,
                targets: vec![MemberId([2; 32])],
                expires_at: 100,
                hop_limit: 2,
            },
            &vec![0; 2500],
        )
        .unwrap();
        let m = o.manifest();
        assert_eq!(plan(m, &[0, 1, 2], 1, 0, 2048).unwrap(), vec![0, 1]);
        assert_eq!(plan(m, &[2], 1, 0, 451).unwrap(), Vec::<usize>::new());
        assert_eq!(plan(m, &[2], 1, 0, 452).unwrap(), vec![2]);
        assert!(plan(m, &[0], 1, 2, 4096).unwrap().is_empty());
        assert_eq!(plan(m, &[0], 100, 0, 4096), Err(DurableError::Expired));
        for bad in [vec![0, 0], vec![2, 1], vec![3]] {
            assert_eq!(plan(m, &bad, 1, 0, 4096), Err(DurableError::InvalidInput));
        }
    }

    fn member(value: u8) -> MemberId {
        MemberId([value; 32])
    }

    #[test]
    fn fifty_member_broadcast_uses_five_deterministic_bounded_audiences() {
        let roster: Vec<_> = (1..=50).map(member).collect();
        let groups = broadcast_audiences(member(1), &roster).unwrap();
        assert_eq!(groups.len(), 5);
        assert_eq!(
            groups.iter().map(Vec::len).collect::<Vec<_>>(),
            vec![10, 10, 10, 10, 9]
        );
        assert!(groups
            .iter()
            .flatten()
            .all(|recipient| *recipient != member(1)));
        assert_eq!(groups.iter().flatten().count(), 49);
        assert!(groups
            .iter()
            .all(|group| group.windows(2).all(|pair| pair[0] < pair[1])));

        let mut reversed = roster;
        reversed.reverse();
        assert_eq!(broadcast_audiences(member(1), &reversed), Ok(groups));
    }

    #[test]
    fn broadcast_audiences_rejects_ambiguous_or_uncertified_rosters() {
        assert_eq!(
            broadcast_audiences(member(1), &[member(1), member(1)]),
            Err(DurableError::InvalidInput)
        );
        assert_eq!(
            broadcast_audiences(member(3), &[member(1), member(2)]),
            Err(DurableError::InvalidInput)
        );
        let oversized: Vec<_> = (1..=51).map(member).collect();
        assert_eq!(
            broadcast_audiences(member(1), &oversized),
            Err(DurableError::InvalidInput)
        );
    }

    #[test]
    fn certified_fifty_member_overlay_is_connected_with_two_aware_edges() {
        let roster: Vec<_> = (1..=50).map(member).collect();
        let mut graph = vec![Vec::new(); roster.len()];
        for (index, local) in roster.iter().copied().enumerate() {
            let plan = neighbor_plan(local, &roster).unwrap();
            let mut reversed = roster.clone();
            reversed.reverse();
            assert_eq!(neighbor_plan(local, &reversed), Ok(plan.clone()));
            assert_eq!(plan.wifi_aware.len(), MAX_DIRECT_NEIGHBORS);
            let fallback = plan
                .bluetooth_fallback
                .expect("even field roster has chord");
            for peer in plan.wifi_aware.into_iter().chain([fallback]) {
                let peer = roster.binary_search(&peer).unwrap();
                graph[index].push(peer);
            }
        }
        for peers in &mut graph {
            peers.sort_unstable();
            peers.dedup();
            assert_eq!(peers.len(), MAX_DIRECT_NEIGHBORS + 1);
        }
        for source in 0..roster.len() {
            let mut distance = vec![None; roster.len()];
            distance[source] = Some(0u8);
            let mut queue = VecDeque::from([source]);
            while let Some(current) = queue.pop_front() {
                let next_distance = distance[current].unwrap() + 1;
                for next in &graph[current] {
                    if distance[*next].is_none() {
                        distance[*next] = Some(next_distance);
                        queue.push_back(*next);
                    }
                }
            }
            assert!(distance.iter().all(Option::is_some));
            assert!(distance
                .into_iter()
                .flatten()
                .all(|hops| hops <= MAX_RELAY_HOPS));
        }
    }

    #[test]
    fn neighbor_plan_rejects_uncertified_or_ambiguous_rosters() {
        assert_eq!(
            neighbor_plan(member(1), &[member(1), member(1)]),
            Err(DurableError::InvalidInput)
        );
        assert_eq!(
            neighbor_plan(member(3), &[member(1), member(2)]),
            Err(DurableError::InvalidInput)
        );
        let roster: Vec<_> = (1..=51).map(member).collect();
        assert_eq!(
            neighbor_plan(member(1), &roster),
            Err(DurableError::InvalidInput)
        );
    }

    #[test]
    fn transport_selection_uses_only_authenticated_healthy_neighbors() {
        let decision = select_transport(
            member(1),
            None,
            &[
                TransportCandidate {
                    peer: member(2),
                    transport: TransportKind::WifiAware,
                    authenticated: true,
                    healthy: true,
                    eta_millis: 20,
                },
                TransportCandidate {
                    peer: member(3),
                    transport: TransportKind::Bluetooth,
                    authenticated: true,
                    healthy: true,
                    eta_millis: 20,
                },
                TransportCandidate {
                    peer: member(4),
                    transport: TransportKind::WifiAware,
                    authenticated: false,
                    healthy: true,
                    eta_millis: 1,
                },
                TransportCandidate {
                    peer: member(5),
                    transport: TransportKind::Bluetooth,
                    authenticated: true,
                    healthy: false,
                    eta_millis: 1,
                },
            ],
            Some(member(2)),
        )
        .unwrap();
        assert_eq!(
            decision,
            TransportDecision::Send(TransportChoice {
                peer: member(3),
                transport: TransportKind::Bluetooth,
                eta_millis: 20,
            })
        );
    }

    #[test]
    fn transport_switches_only_after_a_better_link_is_healthy() {
        let current = TransportChoice {
            peer: member(2),
            transport: TransportKind::WifiAware,
            eta_millis: 80,
        };
        let wifi = TransportCandidate {
            peer: member(2),
            transport: TransportKind::WifiAware,
            authenticated: true,
            healthy: true,
            eta_millis: 80,
        };
        let bluetooth = TransportCandidate {
            peer: member(3),
            transport: TransportKind::Bluetooth,
            authenticated: true,
            healthy: true,
            eta_millis: 30,
        };
        assert_eq!(
            select_transport(member(1), Some(current), &[wifi, bluetooth], None),
            Ok(TransportDecision::Switch {
                from: current,
                to: TransportChoice {
                    peer: member(3),
                    transport: TransportKind::Bluetooth,
                    eta_millis: 30,
                },
            })
        );
        assert_eq!(
            select_transport(
                member(1),
                Some(current),
                &[
                    wifi,
                    TransportCandidate {
                        healthy: false,
                        ..bluetooth
                    }
                ],
                None,
            ),
            Ok(TransportDecision::Keep(current))
        );
        assert_eq!(
            select_transport(member(1), Some(current), &[bluetooth], None),
            Ok(TransportDecision::Send(TransportChoice {
                peer: member(3),
                transport: TransportKind::Bluetooth,
                eta_millis: 30,
            }))
        );
    }

    #[test]
    fn transport_selection_rejects_ambiguous_or_invalid_link_facts() {
        let valid = TransportCandidate {
            peer: member(2),
            transport: TransportKind::WifiAware,
            authenticated: true,
            healthy: true,
            eta_millis: 1,
        };
        assert_eq!(
            select_transport(member(1), None, &[valid, valid], None),
            Err(DurableError::InvalidInput)
        );
        assert_eq!(
            select_transport(
                member(1),
                None,
                &[TransportCandidate {
                    peer: member(1),
                    ..valid
                }],
                None,
            ),
            Err(DurableError::InvalidInput)
        );
        assert_eq!(
            select_transport(
                member(1),
                None,
                &[TransportCandidate {
                    eta_millis: 0,
                    ..valid
                }],
                None,
            ),
            Err(DurableError::InvalidInput)
        );
        assert_eq!(
            select_transport(
                member(1),
                None,
                &[
                    TransportCandidate {
                        eta_millis: 0,
                        authenticated: false,
                        ..valid
                    },
                    TransportCandidate {
                        peer: member(3),
                        transport: TransportKind::Bluetooth,
                        eta_millis: 2,
                        ..valid
                    },
                ],
                None,
            ),
            Ok(TransportDecision::Send(TransportChoice {
                peer: member(3),
                transport: TransportKind::Bluetooth,
                eta_millis: 2,
            }))
        );
    }

    #[test]
    fn presence_limits_direct_neighbors_and_prefers_authenticated_direct_path() {
        let mut presence = PresenceTable::new(member(1));
        assert!(presence.observe_direct(member(2), 10).unwrap());
        assert!(presence.observe_direct(member(3), 10).unwrap());
        assert_eq!(
            presence.observe_direct(member(4), 10),
            Err(DurableError::ResourcePressure)
        );
        assert!(presence
            .observe_relay(
                PresenceClaim {
                    member: member(2),
                    version: PresenceVersion {
                        incarnation: 1,
                        sequence: 1,
                    },
                },
                member(3),
                2,
                11,
            )
            .unwrap());
        assert_eq!(
            presence.member(member(2), 11).unwrap().unwrap().path,
            PresencePath::Direct
        );
        assert!(presence.disconnect_direct(member(2)));
        assert_eq!(
            presence.member(member(2), 11).unwrap().unwrap().path,
            PresencePath::Routed {
                via: member(3),
                hops: 2
            }
        );
    }

    #[test]
    fn presence_rejects_stale_relay_and_expires_only_when_every_path_is_stale() {
        let mut presence = PresenceTable::new(member(1));
        let fresh = PresenceClaim {
            member: member(4),
            version: PresenceVersion {
                incarnation: 3,
                sequence: 8,
            },
        };
        assert!(presence.observe_relay(fresh, member(2), 4, 20).unwrap());
        assert!(!presence
            .observe_relay(
                PresenceClaim {
                    member: member(4),
                    version: PresenceVersion {
                        incarnation: 3,
                        sequence: 7,
                    },
                },
                member(3),
                2,
                21,
            )
            .unwrap());
        assert!(presence.observe_relay(fresh, member(3), 2, 21).unwrap());
        assert_eq!(
            presence.member(member(4), 21).unwrap().unwrap().path,
            PresencePath::Routed {
                via: member(3),
                hops: 2
            }
        );
        assert!(presence.expire(21 + PRESENCE_TTL).unwrap().is_empty());
        assert_eq!(presence.expire(22 + PRESENCE_TTL).unwrap(), vec![member(4)]);
        assert!(presence
            .member(member(4), 22 + PRESENCE_TTL)
            .unwrap()
            .is_none());
    }

    #[test]
    fn presence_keeps_group_cache_bounded_to_fifty_members() {
        let mut presence = PresenceTable::new(member(1));
        for value in 2..=51 {
            assert!(presence
                .observe_relay(
                    PresenceClaim {
                        member: member(value),
                        version: PresenceVersion {
                            incarnation: 1,
                            sequence: 1,
                        },
                    },
                    member(99),
                    2,
                    1,
                )
                .unwrap());
        }
        assert_eq!(presence.active(1).unwrap().len(), MAX_GROUP_MEMBERS);
        assert_eq!(
            presence.observe_relay(
                PresenceClaim {
                    member: member(52),
                    version: PresenceVersion {
                        incarnation: 1,
                        sequence: 1,
                    },
                },
                member(99),
                2,
                1,
            ),
            Err(DurableError::ResourcePressure)
        );
    }

    fn relay(
        origin: u8,
        id: u8,
        previous_hop: u8,
        hops: u8,
        limit: u8,
        expires_at: u64,
    ) -> RelayFrame {
        RelayFrame {
            id: RelayId([id; 16]),
            origin: member(origin),
            previous_hop: member(previous_hop),
            hops,
            hop_limit: limit,
            expires_at,
        }
    }

    #[test]
    fn relay_frame_codec_is_fixed_width_and_rejects_noncanonical_metadata() {
        let frame = relay(7, 9, 4, 2, 16, 123_456);
        let encoded = frame.encode();
        assert_eq!(encoded.len(), RELAY_FRAME_BYTES);
        assert_eq!(RelayFrame::decode(&encoded), Ok(frame));

        let mut wrong_version = encoded;
        wrong_version[0] = RELAY_FRAME_VERSION + 1;
        assert_eq!(
            RelayFrame::decode(&wrong_version),
            Err(DurableError::InvalidInput)
        );
        assert_eq!(
            RelayFrame::decode(&encoded[..RELAY_FRAME_BYTES - 1]),
            Err(DurableError::InvalidInput)
        );

        let mut invalid_limit = encoded;
        invalid_limit[82] = 0;
        assert_eq!(
            RelayFrame::decode(&invalid_limit),
            Err(DurableError::InvalidInput)
        );
        invalid_limit[82] = MAX_RELAY_HOPS + 1;
        assert_eq!(
            RelayFrame::decode(&invalid_limit),
            Err(DurableError::InvalidInput)
        );

        let mut invalid_hops = encoded;
        invalid_hops[81] = 17;
        assert_eq!(
            RelayFrame::decode(&invalid_hops),
            Err(DurableError::InvalidInput)
        );

        let mut invalid_expiry = encoded;
        invalid_expiry[83..91].copy_from_slice(&(MAX_LOGICAL_TIME + 1).to_be_bytes());
        assert_eq!(
            RelayFrame::decode(&invalid_expiry),
            Err(DurableError::InvalidInput)
        );
    }

    #[test]
    fn routed_record_binds_every_durable_payload_to_canonical_relay_metadata() {
        let routed = RoutedRecord {
            frame: relay(1, 9, 4, 2, 16, 123_456),
            record: DurableRecord::Chunk {
                object: ObjectId([7; 32]),
                index: 3,
                bytes: vec![8; CHUNK_BYTES],
            },
        };
        let encoded = routed.encode().unwrap();
        assert_eq!(RoutedRecord::decode(&encoded), Ok(routed.clone()));

        let mut wrong_frame = encoded.clone();
        wrong_frame[2] = 0;
        assert_eq!(
            RoutedRecord::decode(&wrong_frame),
            Err(DurableError::InvalidInput)
        );
        assert_eq!(
            RoutedRecord::decode(&encoded[..encoded.len() - 1]),
            Err(DurableError::InvalidInput)
        );
    }

    #[test]
    fn largest_routed_announcement_fits_one_authenticated_neighbor_datagram() {
        let routed = RoutedRecord {
            frame: relay(1, 9, 4, 2, 16, 123_456),
            record: DurableRecord::Announcement(vec![4; mesh_protocol::MAX_ANNOUNCEMENT]),
        };
        let encoded = routed.encode().unwrap();
        assert_eq!(encoded.len(), MAX_ROUTED_RECORD);
        assert_eq!(encoded.len(), mesh_session::MAX_PAYLOAD);
        assert_eq!(RoutedRecord::decode(&encoded), Ok(routed));
    }

    #[test]
    fn relay_forwards_one_copy_across_hops_without_looping() {
        // A is out of range of C. B forwards exactly one verified object to C;
        // a later copy arriving back at B is suppressed.
        let mut b = RelayCache::new(member(2));
        let first = relay(1, 9, 1, 0, 16, 100);
        let through_b = match b.accept(first, member(1), 10).unwrap() {
            RelayDecision::Forward(frame) => frame,
            other => panic!("unexpected decision: {other:?}"),
        };
        assert_eq!(through_b.previous_hop, member(2));
        assert_eq!(through_b.hops, 1);

        let mut c = RelayCache::new(member(3));
        let through_c = match c.accept(through_b, member(2), 11).unwrap() {
            RelayDecision::Forward(frame) => frame,
            other => panic!("unexpected decision: {other:?}"),
        };
        assert_eq!(through_c.previous_hop, member(3));
        assert_eq!(through_c.hops, 2);
        assert_eq!(
            b.accept(through_c, member(3), 12),
            Ok(RelayDecision::Duplicate)
        );
        assert_eq!(b.len(), 1);
    }

    #[test]
    fn relay_stops_at_ttl_and_hop_budget_and_recovers_after_expiry() {
        let mut cache = RelayCache::new(member(2));
        assert_eq!(
            cache.accept(relay(1, 1, 1, 16, 16, 100), member(1), 10),
            Ok(RelayDecision::HopLimit)
        );
        assert_eq!(
            cache.accept(relay(1, 2, 1, 0, 16, 10), member(1), 10),
            Ok(RelayDecision::Expired)
        );
        let frame = relay(1, 3, 1, 0, 16, 20);
        assert!(matches!(
            cache.accept(frame, member(1), 10),
            Ok(RelayDecision::Forward(_))
        ));
        assert_eq!(
            cache.accept(frame, member(1), 11),
            Ok(RelayDecision::Duplicate)
        );
        assert!(matches!(
            cache.accept(frame, member(1), 21),
            Ok(RelayDecision::Expired)
        ));
    }

    #[test]
    fn relay_cache_stays_bounded_under_fifty_member_pressure() {
        let mut cache = RelayCache::new(member(1));
        for id in 0..(MAX_RELAY_DEDUP + 16) {
            let origin = if id % 2 == 0 { member(2) } else { member(3) };
            let mut bytes = [0; 16];
            bytes[..2].copy_from_slice(&(id as u16).to_be_bytes());
            let frame = RelayFrame {
                id: RelayId(bytes),
                origin,
                previous_hop: origin,
                hops: 0,
                hop_limit: 16,
                expires_at: 10_000,
            };
            assert!(matches!(
                cache.accept(frame, origin, 1),
                Ok(RelayDecision::Forward(_))
            ));
        }
        // The cache never exceeds its fixed live-memory budget even when live
        // objects outnumber the local forwarding window.
        assert!(cache.len() <= MAX_RELAY_DEDUP);
    }
}
