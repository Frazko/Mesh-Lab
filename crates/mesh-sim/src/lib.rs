//! Synthetic host simulator. Fixture keys/payloads MUST NOT enter mobile runtime.
use mesh_object::{digest, ObjectPolicy, PreparedObject};
use mesh_replication::{
    plan, RelayCache, RelayDecision, RelayFrame, RelayId, MAX_DIRECT_NEIGHBORS, MAX_RELAY_HOPS,
};
use mesh_store::{Limits, Store};
use mesh_types::durable::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, VecDeque},
    path::Path,
};
use zeroize::Zeroizing;

type SimResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub seed: u64,
    pub nodes: usize,
    pub messages: usize,
    pub steps: u64,
}
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Report {
    pub profile: &'static str,
    pub seed: u64,
    pub nodes: usize,
    pub messages: usize,
    pub scheduled_events: u64,
    pub partitioned_contacts: u64,
    pub dropped_contacts: u64,
    pub restarted_stores: u64,
    pub duplicate_chunks: u64,
    pub lost_local_acks: u64,
    pub rejected_corrupt_chunks: u64,
    pub resumed_chunks: u64,
    pub recovery_chunks: u64,
    pub durable_recipient_copies: usize,
    pub expected_recipient_copies: usize,
    pub retained_source_copies: usize,
    pub passed: bool,
}

/// Evidence from the deterministic 50-phone relay topology. The two ring
/// edges stand for direct Wi-Fi Aware NDP links; the diameter-reducing matching
/// edge is an authenticated Bluetooth fallback. The latter does not consume a
/// Wi-Fi Aware session, so the hardware limit of two NDPs remains exact.
#[derive(Debug, PartialEq, Eq)]
pub struct RelayTopologyReport {
    pub nodes: usize,
    pub messages: usize,
    pub expected_recipient_deliveries: usize,
    pub recipient_deliveries: usize,
    pub max_wifi_aware_degree: usize,
    pub max_authenticated_degree: usize,
    pub max_hops_seen: u8,
    pub duplicate_drops: u64,
    pub hop_limit_drops: u64,
    pub partition_deferrals: u64,
    pub queue_high_water: usize,
}
// Deterministic scheduling only. NOT a cryptographic random number generator.
struct Schedule(u64);
impl Schedule {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}
fn member(index: usize) -> MemberId {
    MemberId([(index + 1) as u8; 32])
}

/// Runs a bounded store-and-forward topology with no all-to-all links. A 50
/// node cycle has diameter 25, which cannot meet a 16-hop budget. Pairing each
/// node with its opposite member through an already-authenticated Bluetooth
/// fallback reduces the deterministic diameter to 13 while retaining exactly
/// two Wi-Fi Aware links per member.
pub fn run_relay_topology(nodes: usize, messages: usize) -> SimResult<RelayTopologyReport> {
    run_relay_topology_with_partition(nodes, messages, 0)
}

/// Same bounded topology, but each edge that crosses the two halves of the
/// group is unavailable until `partition_until`. Deferred tasks remain in the
/// sender's transfer queue and are retried after reunion; a failed radio write
/// never marks the object as received by a relay cache.
pub fn run_relay_partition_reunion(
    nodes: usize,
    messages: usize,
) -> SimResult<RelayTopologyReport> {
    run_relay_topology_with_partition(nodes, messages, 6)
}

fn run_relay_topology_with_partition(
    nodes: usize,
    messages: usize,
    partition_until: u64,
) -> SimResult<RelayTopologyReport> {
    if !(4..=MAX_GROUP_MEMBERS).contains(&nodes)
        || !nodes.is_multiple_of(2)
        || !(1..=32).contains(&messages)
    {
        return Err("relay topology outside bounded simulator limits".into());
    }

    let mut neighbors = vec![BTreeSet::new(); nodes];
    let mut wifi_degrees = vec![0usize; nodes];
    for node in 0..nodes {
        for peer in [(node + nodes - 1) % nodes, (node + 1) % nodes] {
            neighbors[node].insert(peer);
        }
        wifi_degrees[node] = 2;
        let fallback = (node + nodes / 2) % nodes;
        neighbors[node].insert(fallback);
    }
    if wifi_degrees
        .iter()
        .any(|degree| *degree != MAX_DIRECT_NEIGHBORS)
        || neighbors
            .iter()
            .any(|peers| peers.len() != MAX_DIRECT_NEIGHBORS + 1)
    {
        return Err("topology violated direct-link budget".into());
    }

    let mut caches: Vec<_> = (0..nodes)
        .map(|node| RelayCache::new(member(node)))
        .collect();
    let mut report = RelayTopologyReport {
        nodes,
        messages,
        expected_recipient_deliveries: messages * (nodes - 1),
        recipient_deliveries: 0,
        max_wifi_aware_degree: *wifi_degrees.iter().max().unwrap_or(&0),
        max_authenticated_degree: neighbors.iter().map(BTreeSet::len).max().unwrap_or(0),
        max_hops_seen: 0,
        duplicate_drops: 0,
        hop_limit_drops: 0,
        partition_deferrals: 0,
        queue_high_water: 0,
    };

    for sequence in 0..messages {
        let origin = sequence % nodes;
        let frame = RelayFrame {
            id: RelayId((sequence as u128 + 1).to_be_bytes()),
            origin: member(origin),
            previous_hop: member(origin),
            hops: 0,
            hop_limit: MAX_RELAY_HOPS,
            expires_at: 10_000,
        };
        let mut delivered = BTreeSet::new();
        let mut queue: VecDeque<_> = neighbors[origin]
            .iter()
            .copied()
            .map(|target| (target, origin, frame, 1))
            .collect();
        report.queue_high_water = report.queue_high_water.max(queue.len());

        while let Some((target, via, incoming, attempt_at)) = queue.pop_front() {
            let crosses_partition = (target < nodes / 2) != (via < nodes / 2);
            if attempt_at < partition_until && crosses_partition {
                report.partition_deferrals += 1;
                queue.push_back((target, via, incoming, partition_until));
                report.queue_high_water = report.queue_high_water.max(queue.len());
                continue;
            }
            // The origin already has durable custody. Returning frames are
            // discarded before they can become an application delivery.
            if target == origin {
                report.duplicate_drops += 1;
                continue;
            }
            match caches[target].accept(incoming, member(via), attempt_at)? {
                RelayDecision::Forward(forward) => {
                    delivered.insert(target);
                    report.max_hops_seen = report.max_hops_seen.max(forward.hops);
                    for next in neighbors[target]
                        .iter()
                        .copied()
                        .filter(|next| *next != via)
                    {
                        queue.push_back((next, target, forward, attempt_at));
                    }
                    report.queue_high_water = report.queue_high_water.max(queue.len());
                }
                RelayDecision::Duplicate => report.duplicate_drops += 1,
                RelayDecision::Expired => return Err("live relay expired unexpectedly".into()),
                RelayDecision::HopLimit => report.hop_limit_drops += 1,
            }
        }
        if delivered.len() != nodes - 1 {
            return Err("relay topology did not reach every member within hop budget".into());
        }
        report.recipient_deliveries += delivered.len();
    }
    Ok(report)
}
fn open(root: &Path, index: usize) -> SimResult<Store> {
    // Public fixture key: encrypted-at-rest mechanics, not secrecy against simulator users.
    Ok(Store::open(
        &root.join(format!("synthetic-node-{index}.db")),
        Zeroizing::new([(index + 71) as u8; 32]),
        member(index),
        Limits::default(),
    )?)
}
pub fn run(scenario: Scenario, root: &Path) -> SimResult<Report> {
    if scenario.seed == 0
        || !(2..=10).contains(&scenario.nodes)
        || !(1..=32).contains(&scenario.messages)
        || !(100..=1_000_000).contains(&scenario.steps)
    {
        return Err("scenario outside bounded simulator limits".into());
    }
    // Refuse to overwrite/reuse an existing directory or real database.
    std::fs::create_dir(root)?;
    let mut nodes: Vec<Option<Store>> = (0..scenario.nodes)
        .map(|i| open(root, i).map(Some))
        .collect::<SimResult<_>>()?;
    let mut objects = Vec::new();
    for index in 0..scenario.messages {
        let op = OperationId((index as u128 + 1).to_be_bytes());
        let bytes = vec![index as u8; 2500 + index];
        let command = digest(&bytes);
        let seq = nodes[0].as_mut().unwrap().reserve(op, command)?.sequence;
        let o = PreparedObject::from_opaque(
            member(0),
            seq,
            ObjectPolicy {
                namespace: Namespace::new("mesh.lab.synthetic.v1")?,
                epoch: 1,
                targets: (1..scenario.nodes).map(member).collect(),
                expires_at: scenario.steps + 1000,
                hop_limit: 4,
            },
            &bytes,
        )?;
        nodes[0]
            .as_mut()
            .unwrap()
            .commit_outgoing(op, command, &o, 0)?;
        objects.push(o);
    }
    let mut r = Report {
        profile: "F1-synthetic-local-storage-v1",
        seed: scenario.seed,
        nodes: scenario.nodes,
        messages: scenario.messages,
        scheduled_events: scenario.steps,
        partitioned_contacts: 0,
        dropped_contacts: 0,
        restarted_stores: 0,
        duplicate_chunks: 0,
        lost_local_acks: 0,
        rejected_corrupt_chunks: 0,
        resumed_chunks: 0,
        recovery_chunks: 0,
        durable_recipient_copies: 0,
        expected_recipient_copies: scenario.messages * (scenario.nodes - 1),
        retained_source_copies: 0,
        passed: false,
    };
    let mut rng = Schedule(scenario.seed);
    for tick in 1..=scenario.steps {
        let roll = rng.next();
        let target = (roll as usize % (scenario.nodes - 1)) + 1;
        if tick % 2048 == 0 {
            drop(nodes[target].take());
            nodes[target] = Some(open(root, target)?);
            r.restarted_stores += 1;
        }
        // Events include offline/contact ticks. Transfer opportunity every 17 ticks.
        if tick % 17 != 0 {
            continue;
        }
        if tick < scenario.steps / 2 && target >= scenario.nodes.div_ceil(2) {
            r.partitioned_contacts += 1;
            continue;
        }
        if rng.next() % 100 < 30 {
            r.dropped_contacts += 1;
            continue;
        }
        let object = &objects[rng.next() as usize % objects.len()];
        let m = object.manifest();
        let id = m.id();
        let receiver = nodes[target].as_mut().unwrap();
        receiver.announce(m, tick)?;
        let missing = receiver.missing(id)?;
        let batch = plan(m, &missing, tick, 0, 1024)?;
        if let Some(&index) = batch.first() {
            let bytes = nodes[0].as_ref().unwrap().chunk(id, index)?;
            let receiver = nodes[target].as_mut().unwrap();
            if rng.next().is_multiple_of(7) {
                let mut corrupt = bytes.clone();
                corrupt[0] ^= 1;
                let before = receiver.stats()?;
                if receiver.put_chunk(id, index, &corrupt, tick) != Err(DurableError::Corrupt)
                    || receiver.stats()? != before
                {
                    return Err("corrupt input altered durable state".into());
                }
                r.rejected_corrupt_chunks += 1;
            }
            let ack = receiver.put_chunk(id, index, &bytes, tick)?;
            r.resumed_chunks += 1;
            if rng.next().is_multiple_of(5) {
                if receiver.put_chunk(id, index, &bytes, tick)? != ack {
                    return Err("duplicate changed local commit".into());
                }
                r.duplicate_chunks += 1;
            }
            if ack.is_some() && rng.next().is_multiple_of(3) {
                // Lose the return value, close and recover only from persisted state.
                drop(nodes[target].take());
                nodes[target] = Some(open(root, target)?);
                r.restarted_stores += 1;
                r.lost_local_acks += 1;
                if nodes[target].as_ref().unwrap().local_commit(id)? != ack {
                    return Err("local ack did not survive reopen".into());
                }
            }
        }
    }
    // Explicit healthy recovery window. Account separately from random scheduled events.
    for node in nodes.iter_mut().skip(1) {
        let receiver = node.as_mut().unwrap();
        for object in &objects {
            let m = object.manifest();
            let id = receiver.announce(m, scenario.steps + 1)?;
            let missing = receiver.missing(id)?;
            for index in plan(m, &missing, scenario.steps + 1, 0, MAX_OBJECT_BYTES)? {
                receiver.put_chunk(id, index, &object.chunks()[index], scenario.steps + 1)?;
                r.recovery_chunks += 1;
            }
        }
    }
    // Reopen every DB and compare exact content, completeness and unique delivery rows.
    for (index, node) in nodes.iter_mut().enumerate() {
        drop(node.take());
        *node = Some(open(root, index)?);
        r.restarted_stores += 1;
        let store = node.as_ref().unwrap();
        for o in &objects {
            let id = o.manifest().id();
            if store.manifest(id)? != *o.manifest() || !store.missing(id)?.is_empty() {
                return Err("recovery missing immutable content".into());
            }
            for (i, expected) in o.chunks().iter().enumerate() {
                if &store.chunk(id, i)? != expected {
                    return Err("content changed during transfer".into());
                }
            }
        }
        let stats = store.stats()?;
        if index == 0 {
            r.retained_source_copies = stats.objects;
            if stats.outbox != scenario.messages {
                return Err("source outbox retired without authenticated receipt".into());
            }
        } else {
            r.durable_recipient_copies += stats.local_deliveries;
            if stats.objects != scenario.messages || stats.local_deliveries != scenario.messages {
                return Err("missing or duplicate delivery".into());
            }
        }
    }
    r.passed = r.durable_recipient_copies == r.expected_recipient_copies
        && r.retained_source_copies == scenario.messages;
    Ok(r)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fifty_members_relay_without_wifi_aware_all_to_all_or_loops() {
        let report = run_relay_topology(50, 32).unwrap();
        assert_eq!(report.recipient_deliveries, 32 * 49);
        assert_eq!(
            report.expected_recipient_deliveries,
            report.recipient_deliveries
        );
        assert_eq!(report.max_wifi_aware_degree, 2);
        assert_eq!(report.max_authenticated_degree, 3);
        assert!(report.max_hops_seen <= MAX_RELAY_HOPS);
        assert_eq!(report.hop_limit_drops, 0);
        assert!(report.duplicate_drops > 0);
        assert!(report.queue_high_water <= 150);
    }

    #[test]
    fn fifty_members_recover_a_partition_from_their_pending_relay_queue() {
        let report = run_relay_partition_reunion(50, 32).unwrap();
        assert_eq!(report.recipient_deliveries, 32 * 49);
        assert_eq!(report.max_wifi_aware_degree, 2);
        assert_eq!(report.max_authenticated_degree, 3);
        assert!(report.partition_deferrals > 0);
        assert_eq!(report.hop_limit_drops, 0);
        assert!(report.max_hops_seen <= MAX_RELAY_HOPS);
    }

    #[test]
    fn every_supported_even_roster_reaches_the_group_within_the_radio_degree_budget() {
        // This is a 24-case deterministic campaign: all supported even group
        // sizes from four to fifty. It proves overlay behavior, not a physical
        // Wi-Fi Aware data path on a particular handset.
        for nodes in (4..=50).step_by(2) {
            let first = run_relay_topology(nodes, 3).unwrap();
            let replay = run_relay_topology(nodes, 3).unwrap();
            assert_eq!(
                first, replay,
                "topology must be deterministic for {nodes} nodes"
            );
            assert_eq!(first.recipient_deliveries, 3 * (nodes - 1));
            assert_eq!(
                first.expected_recipient_deliveries,
                first.recipient_deliveries
            );
            assert!(first.max_wifi_aware_degree <= MAX_DIRECT_NEIGHBORS);
            assert!(first.max_authenticated_degree <= MAX_DIRECT_NEIGHBORS + 1);
            assert!(first.max_hops_seen <= MAX_RELAY_HOPS);
            assert_eq!(first.hop_limit_drops, 0);
            assert!(first.duplicate_drops > 0);
        }
    }

    #[test]
    fn every_supported_even_roster_recovers_after_a_partition_without_exceeding_hop_budget() {
        // A second 24-case campaign checks that queued records converge after
        // reunion instead of being marked delivered while an edge is absent.
        for nodes in (4..=50).step_by(2) {
            let report = run_relay_partition_reunion(nodes, 2).unwrap();
            assert_eq!(report.recipient_deliveries, 2 * (nodes - 1));
            assert_eq!(
                report.expected_recipient_deliveries,
                report.recipient_deliveries
            );
            assert!(
                report.partition_deferrals > 0,
                "partition was not exercised for {nodes}"
            );
            assert!(report.max_wifi_aware_degree <= MAX_DIRECT_NEIGHBORS);
            assert!(report.max_hops_seen <= MAX_RELAY_HOPS);
            assert_eq!(report.hop_limit_drops, 0);
        }
    }

    #[test]
    fn relay_topology_rejects_out_of_profile_sizes_and_message_pressure() {
        for nodes in [0, 1, 2, 3, 5, 49, 51, 52] {
            assert!(
                run_relay_topology(nodes, 1).is_err(),
                "accepted node count {nodes}"
            );
        }
        for messages in [0, 33] {
            assert!(
                run_relay_topology(50, messages).is_err(),
                "accepted message count {messages}"
            );
        }
    }

    #[test]
    fn seeded_run_reproducible_and_recovers() {
        let root = std::env::temp_dir().join(format!("mesh-sim-test-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let scenario = Scenario {
            seed: 42,
            nodes: 3,
            messages: 4,
            steps: 2500,
        };
        let a = run(scenario, &root.join("a")).unwrap();
        let b = run(scenario, &root.join("b")).unwrap();
        assert!(a.passed);
        assert_eq!(a, b);
        assert_eq!(a.durable_recipient_copies, 8);
        assert!(a.partitioned_contacts > 0);
        assert!(a.dropped_contacts > 0);
        assert!(a.rejected_corrupt_chunks > 0);
        assert!(run(scenario, &root.join("a")).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
