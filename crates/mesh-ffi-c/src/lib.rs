//! Sole C unsafe boundary. Hosts own runtime handles and release each output once.
use mesh_crypto::{DeliverySecret, IdentitySigningKey, OsRandom, RandomSource, Scope};
use mesh_link::Frame;
use mesh_object::ObjectPolicy;
use mesh_protocol::{
    issue_certificate, issue_receipt_ack, receipt_ack_route, receipt_route, seal_message,
    verify_enrollment_request, verify_receipt_ack, CertificateClaims, DurableRecord, PolicyBundle,
    SealRequest, VerifiedRoster,
};
use mesh_replication::{
    broadcast_audiences, neighbor_plan, RelayCache, RelayDecision, RelayFrame, RelayId,
    RoutedRecord, RELAY_FRAME_BYTES,
};
use mesh_runtime::Runtime;
use mesh_session::{Config as SessionConfig, Handshake, Incoming, Role, Session, SessionSecret};
use mesh_store::{Limits, Store};
use mesh_types::durable::{MemberId, Namespace, OperationId, MAX_GROUP_MEMBERS, MAX_LOGICAL_TIME};
use mesh_types::{Command, Error, ABI_VERSION, MAX_COUNTER, MAX_INPUT, MAX_OUTPUT, MAX_RUNTIMES};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{Mutex, OnceLock},
};

struct Registry {
    next: u64,
    runtimes: BTreeMap<u64, Runtime>,
}
struct StoreRegistry {
    next: u64,
    stores: BTreeMap<u64, Store>,
}
enum NativeSession {
    Handshake(Box<Handshake>),
    Ready(Box<Session>),
    Closed,
}
struct SessionRecord {
    store_handle: u64,
    state: NativeSession,
}
struct SessionRegistry {
    next: u64,
    sessions: BTreeMap<u64, SessionRecord>,
}
struct RelayGateRegistry {
    next: u64,
    gates: BTreeMap<u64, RelayCache>,
}
fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        Mutex::new(Registry {
            next: 1,
            runtimes: BTreeMap::new(),
        })
    })
}
fn stores() -> &'static Mutex<StoreRegistry> {
    static STORES: OnceLock<Mutex<StoreRegistry>> = OnceLock::new();
    STORES.get_or_init(|| {
        Mutex::new(StoreRegistry {
            next: 1,
            stores: BTreeMap::new(),
        })
    })
}
fn sessions() -> &'static Mutex<SessionRegistry> {
    static SESSIONS: OnceLock<Mutex<SessionRegistry>> = OnceLock::new();
    SESSIONS.get_or_init(|| {
        Mutex::new(SessionRegistry {
            next: 1,
            sessions: BTreeMap::new(),
        })
    })
}
fn relay_gates() -> &'static Mutex<RelayGateRegistry> {
    static GATES: OnceLock<Mutex<RelayGateRegistry>> = OnceLock::new();
    GATES.get_or_init(|| {
        Mutex::new(RelayGateRegistry {
            next: 1,
            gates: BTreeMap::new(),
        })
    })
}
fn session_error(error: mesh_session::Error) -> Error {
    match error {
        mesh_session::Error::InvalidInput
        | mesh_session::Error::WrongPhase
        | mesh_session::Error::Replay => Error::InvalidArgument,
        mesh_session::Error::Expired | mesh_session::Error::Closed => Error::StaleRequest,
        mesh_session::Error::Exhausted => Error::ResourcePressure,
        mesh_session::Error::Authentication | mesh_session::Error::RandomUnavailable => {
            Error::InternalInvariant
        }
    }
}
fn verified_roster(store_handle: u64, now: u64) -> Result<VerifiedRoster, Error> {
    if now == 0 {
        return Err(Error::InvalidArgument);
    }
    let registry = stores().lock().map_err(|_| Error::InternalInvariant)?;
    let store = registry
        .stores
        .get(&store_handle)
        .ok_or(Error::InvalidHandle)?;
    let bundle = store
        .policy_bundle()
        .map_err(|_| Error::InternalInvariant)?
        .ok_or(Error::StaleRequest)?;
    bundle.verify(now).map_err(|_| Error::StaleRequest)
}
pub fn guarded<T>(f: impl FnOnce() -> Result<T, Error>) -> Result<T, Error> {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(Err(Error::InternalInvariant))
}
pub fn create(version: u32) -> Result<u64, Error> {
    guarded(|| {
        if version != ABI_VERSION {
            return Err(Error::IncompatibleVersion);
        }
        let mut registry = registry().lock().map_err(|_| Error::InternalInvariant)?;
        if registry.runtimes.len() >= MAX_RUNTIMES || registry.next >= MAX_COUNTER {
            return Err(Error::ResourcePressure);
        }
        let id = registry.next;
        registry.next += 1; // never reused: stale handles cannot alias a later instance
        registry.runtimes.insert(id, Runtime::new(id));
        Ok(id)
    })
}
pub fn request(handle: u64, bytes: &[u8]) -> Result<Vec<u8>, Error> {
    guarded(|| {
        let command = mesh_codec::decode_request(bytes)?;
        let mut registry = registry().lock().map_err(|_| Error::InternalInvariant)?;
        let runtime = registry
            .runtimes
            .get_mut(&handle)
            .ok_or(Error::InvalidHandle)?;
        let output = match command {
            Command::EngineInfo => mesh_codec::encode_info(),
            Command::Subscribe { cursor } => {
                mesh_codec::encode_snapshot(1, &runtime.subscribe(cursor))
            }
            Command::VerifyBridge { request_id } => {
                mesh_codec::encode_snapshot(2, &runtime.verify_bridge(request_id)?)
            }
        };
        if output.len() > MAX_OUTPUT {
            return Err(Error::ResourcePressure);
        }
        Ok(output)
    })
}
pub fn release(handle: u64) -> Result<(), Error> {
    guarded(|| {
        registry()
            .lock()
            .map_err(|_| Error::InternalInvariant)?
            .runtimes
            .remove(&handle);
        Ok(())
    })
}

/// Open the encrypted store using only host-supplied, protected material. This
/// intentionally exposes no database handle, rows or key bytes through C/Flutter.
pub fn secure_store_probe(key: &[u8], member: &[u8], path: &str) -> Result<(), Error> {
    guarded(|| {
        if key.len() != 32
            || member.len() != 32
            || !(1..=1024).contains(&path.len())
            || path.as_bytes().contains(&0)
        {
            return Err(Error::InvalidArgument);
        }
        let mut database_key = zeroize::Zeroizing::new([0; 32]);
        database_key.copy_from_slice(key);
        let member = MemberId(member.try_into().map_err(|_| Error::InvalidArgument)?);
        let store = Store::open(
            std::path::Path::new(path),
            database_key,
            member,
            Limits::default(),
        )
        .map_err(|_| Error::InternalInvariant)?;
        let _ = store.stats().map_err(|_| Error::InternalInvariant)?;
        Ok(())
    })
}
/// Opens an encrypted store for the native host executor. The opaque handle never
/// crosses the Flutter channel, and the key is consumed while opening SQLCipher.
pub fn secure_store_open(key: &[u8], member: &[u8], path: &str) -> Result<u64, Error> {
    guarded(|| {
        if key.len() != 32
            || member.len() != 32
            || !(1..=1024).contains(&path.len())
            || path.as_bytes().contains(&0)
        {
            return Err(Error::InvalidArgument);
        }
        let mut database_key = zeroize::Zeroizing::new([0; 32]);
        database_key.copy_from_slice(key);
        let member = MemberId(member.try_into().map_err(|_| Error::InvalidArgument)?);
        let store = Store::open(
            std::path::Path::new(path),
            database_key,
            member,
            Limits::default(),
        )
        .map_err(|_| Error::InternalInvariant)?;
        let _ = store.stats().map_err(|_| Error::InternalInvariant)?;
        let mut registry = stores().lock().map_err(|_| Error::InternalInvariant)?;
        if registry.stores.len() >= MAX_RUNTIMES || registry.next >= MAX_COUNTER {
            return Err(Error::ResourcePressure);
        }
        let id = registry.next;
        registry.next += 1;
        registry.stores.insert(id, store);
        Ok(id)
    })
}
pub fn secure_store_release(handle: u64) -> Result<(), Error> {
    guarded(|| {
        stores()
            .lock()
            .map_err(|_| Error::InternalInvariant)?
            .stores
            .remove(&handle);
        Ok(())
    })
}
/// Returns the active policy epoch, or zero when this device has not joined a
/// group. The query exposes no certificates, group identifier, rows or secrets.
pub fn secure_store_policy_epoch(handle: u64, now: u64) -> Result<u64, Error> {
    guarded(|| {
        let registry = stores().lock().map_err(|_| Error::InternalInvariant)?;
        let store = registry.stores.get(&handle).ok_or(Error::InvalidHandle)?;
        Ok(store
            .active_policy(now)
            .map_err(|_| Error::InternalInvariant)?
            .map_or(0, |policy| policy.scope.epoch))
    })
}

/// Returns a fixed-size, domain-separated discovery tag for the currently
/// verified group. It contains no raw group id, policy bundle, certificate, or
/// private key. Radios use it only to ignore unrelated Mesh Lab advertisements;
/// every actual link is still authenticated by the native Noise session.
pub fn secure_store_discovery_tag(handle: u64, now: u64) -> Result<[u8; 16], Error> {
    guarded(|| {
        let registry = stores().lock().map_err(|_| Error::InternalInvariant)?;
        let store = registry.stores.get(&handle).ok_or(Error::InvalidHandle)?;
        let policy = store
            .active_policy(now)
            .map_err(|_| Error::InternalInvariant)?
            .ok_or(Error::StaleRequest)?;
        let mut hash = Sha256::new();
        hash.update(b"MeshLab/WifiAwareDiscovery/v1\0");
        hash.update(policy.scope.group);
        hash.update(policy.scope.epoch.to_be_bytes());
        hash.update(policy.roster_digest);
        let digest: [u8; 32] = hash.finalize().into();
        digest[..16]
            .try_into()
            .map_err(|_| Error::InternalInvariant)
    })
}

/// Returns only the short public discovery identifiers of this member's
/// deterministic Wi-Fi Aware neighbors. The roster itself, certificates,
/// group id, and all private material remain inside the encrypted native
/// store. A caller can open at most two direct NDPs from this result.
pub fn secure_store_aware_neighbors(handle: u64, now: u64) -> Result<Vec<u8>, Error> {
    guarded(|| {
        let registry = stores().lock().map_err(|_| Error::InternalInvariant)?;
        let store = registry.stores.get(&handle).ok_or(Error::InvalidHandle)?;
        let bundle = store
            .policy_bundle()
            .map_err(|_| Error::InternalInvariant)?
            .ok_or(Error::StaleRequest)?;
        let roster = bundle.verify(now).map_err(|_| Error::StaleRequest)?;
        let members = roster
            .member_claims()
            .into_iter()
            .map(|claim| claim.member)
            .collect::<Vec<_>>();
        let plan =
            neighbor_plan(store.local_member(), &members).map_err(|_| Error::InternalInvariant)?;
        let mut result = Vec::with_capacity(plan.wifi_aware.len() * 8);
        for member in plan.wifi_aware {
            let digest: [u8; 32] = Sha256::digest(member.0).into();
            result.extend_from_slice(&digest[..8]);
        }
        Ok(result)
    })
}

/// Starts one Noise XX attempt bound to the store's currently verified roster.
/// Handles are process-local and deliberately contain no material visible to Dart.
pub fn secure_session_start(
    store_handle: u64,
    session_seed: &[u8],
    member: &[u8],
    role: u8,
    now: u64,
) -> Result<u64, Error> {
    guarded(|| {
        if session_seed.len() != 32 || member.len() != 32 || now == 0 {
            return Err(Error::InvalidArgument);
        }
        let role = match role {
            0 => Role::Initiator,
            1 => Role::Responder,
            _ => return Err(Error::InvalidArgument),
        };
        let roster = verified_roster(store_handle, now)?;
        let local = MemberId(member.try_into().map_err(|_| Error::InvalidArgument)?);
        let mut secret = zeroize::Zeroizing::new([0; 32]);
        secret.copy_from_slice(session_seed);
        let key = SessionSecret::import(secret);
        let mut rng = OsRandom;
        let handshake = Handshake::start(
            SessionConfig {
                role,
                local,
                expected_peer: None,
                now,
            },
            &roster,
            &key,
            &mut rng,
        )
        .map_err(session_error)?;
        let mut registry = sessions().lock().map_err(|_| Error::InternalInvariant)?;
        if registry.sessions.len() >= MAX_RUNTIMES || registry.next >= MAX_COUNTER {
            return Err(Error::ResourcePressure);
        }
        let handle = registry.next;
        registry.next += 1;
        registry.sessions.insert(
            handle,
            SessionRecord {
                store_handle,
                state: NativeSession::Handshake(Box::new(handshake)),
            },
        );
        Ok(handle)
    })
}
pub fn secure_session_write(handle: u64, now: u64) -> Result<Vec<u8>, Error> {
    guarded(|| {
        let mut registry = sessions().lock().map_err(|_| Error::InternalInvariant)?;
        let record = registry
            .sessions
            .get_mut(&handle)
            .ok_or(Error::InvalidHandle)?;
        match &mut record.state {
            NativeSession::Handshake(handshake) => handshake.write(now).map_err(session_error),
            NativeSession::Ready(_) | NativeSession::Closed => Err(Error::StaleRequest),
        }
    })
}
pub fn secure_session_read(handle: u64, bytes: &[u8], now: u64) -> Result<(), Error> {
    guarded(|| {
        if bytes.is_empty() || bytes.len() > 96 || now == 0 {
            return Err(Error::InvalidArgument);
        }
        let mut registry = sessions().lock().map_err(|_| Error::InternalInvariant)?;
        let record = registry
            .sessions
            .get_mut(&handle)
            .ok_or(Error::InvalidHandle)?;
        match &mut record.state {
            NativeSession::Handshake(handshake) => {
                handshake.read(bytes, now).map_err(session_error)
            }
            NativeSession::Ready(_) | NativeSession::Closed => Err(Error::StaleRequest),
        }
    })
}
fn session_store_handle(handle: u64) -> Result<u64, Error> {
    let registry = sessions().lock().map_err(|_| Error::InternalInvariant)?;
    registry
        .sessions
        .get(&handle)
        .map(|record| record.store_handle)
        .ok_or(Error::InvalidHandle)
}
pub fn secure_session_finish(handle: u64, now: u64) -> Result<(), Error> {
    guarded(|| {
        let roster = verified_roster(session_store_handle(handle)?, now)?;
        let mut registry = sessions().lock().map_err(|_| Error::InternalInvariant)?;
        let record = registry
            .sessions
            .get_mut(&handle)
            .ok_or(Error::InvalidHandle)?;
        let state = std::mem::replace(&mut record.state, NativeSession::Closed);
        let NativeSession::Handshake(handshake) = state else {
            return Err(Error::StaleRequest);
        };
        match handshake.finish(&roster, now).map_err(session_error) {
            Ok(session) => {
                record.state = NativeSession::Ready(Box::new(session));
                Ok(())
            }
            Err(error) => Err(error),
        }
    })
}
pub fn secure_session_authenticate(
    handle: u64,
    identity_seed: &[u8],
    now: u64,
) -> Result<Vec<u8>, Error> {
    guarded(|| {
        if identity_seed.len() != 32 {
            return Err(Error::InvalidArgument);
        }
        let roster = verified_roster(session_store_handle(handle)?, now)?;
        let mut seed = zeroize::Zeroizing::new([0; 32]);
        seed.copy_from_slice(identity_seed);
        let signer = IdentitySigningKey::import(seed);
        let mut registry = sessions().lock().map_err(|_| Error::InternalInvariant)?;
        let record = registry
            .sessions
            .get_mut(&handle)
            .ok_or(Error::InvalidHandle)?;
        match &mut record.state {
            NativeSession::Ready(session) => session
                .authentication(&signer, &roster, now)
                .map_err(session_error),
            NativeSession::Handshake(_) | NativeSession::Closed => Err(Error::StaleRequest),
        }
    })
}
pub fn secure_session_send(handle: u64, bytes: &[u8], now: u64) -> Result<Vec<u8>, Error> {
    guarded(|| {
        if bytes.is_empty() || bytes.len() > mesh_session::MAX_PAYLOAD {
            return Err(Error::InvalidArgument);
        }
        let roster = verified_roster(session_store_handle(handle)?, now)?;
        let mut registry = sessions().lock().map_err(|_| Error::InternalInvariant)?;
        let record = registry
            .sessions
            .get_mut(&handle)
            .ok_or(Error::InvalidHandle)?;
        match &mut record.state {
            NativeSession::Ready(session) => {
                session.send(bytes, &roster, now).map_err(session_error)
            }
            NativeSession::Handshake(_) | NativeSession::Closed => Err(Error::StaleRequest),
        }
    })
}
/// An empty successful result means that a peer-authentication proof was accepted;
/// non-empty bytes are authenticated application data for the native host only.
pub fn secure_session_receive(handle: u64, frame: &[u8], now: u64) -> Result<Vec<u8>, Error> {
    guarded(|| {
        if frame.len() < 58 || frame.len() > mesh_session::MAX_FRAME {
            return Err(Error::InvalidArgument);
        }
        let roster = verified_roster(session_store_handle(handle)?, now)?;
        let mut registry = sessions().lock().map_err(|_| Error::InternalInvariant)?;
        let record = registry
            .sessions
            .get_mut(&handle)
            .ok_or(Error::InvalidHandle)?;
        match &mut record.state {
            NativeSession::Ready(session) => match session
                .receive(frame, &roster, now)
                .map_err(session_error)?
            {
                Incoming::Authenticated(_) => Ok(Vec::new()),
                Incoming::Data(bytes) => Ok(bytes.to_vec()),
            },
            NativeSession::Handshake(_) | NativeSession::Closed => Err(Error::StaleRequest),
        }
    })
}
pub fn secure_session_authenticated(handle: u64) -> Result<bool, Error> {
    guarded(|| {
        let registry = sessions().lock().map_err(|_| Error::InternalInvariant)?;
        let record = registry.sessions.get(&handle).ok_or(Error::InvalidHandle)?;
        match &record.state {
            NativeSession::Ready(session) => Ok(session.is_authenticated()),
            NativeSession::Handshake(_) | NativeSession::Closed => Ok(false),
        }
    })
}
/// Returns the certified identity of the authenticated remote Noise peer.
/// No radio identifier or key material is exposed.
pub fn secure_session_peer(handle: u64) -> Result<[u8; 32], Error> {
    guarded(|| {
        let registry = sessions().lock().map_err(|_| Error::InternalInvariant)?;
        let record = registry.sessions.get(&handle).ok_or(Error::InvalidHandle)?;
        match &record.state {
            NativeSession::Ready(session) => session
                .peer()
                .map(|member| member.0)
                .ok_or(Error::StaleRequest),
            NativeSession::Handshake(_) | NativeSession::Closed => Err(Error::StaleRequest),
        }
    })
}
pub fn secure_session_release(handle: u64) -> Result<(), Error> {
    guarded(|| {
        sessions()
            .lock()
            .map_err(|_| Error::InternalInvariant)?
            .sessions
            .remove(&handle);
        Ok(())
    })
}

pub fn link_frame_encode(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    guarded(|| {
        if bytes.is_empty() || bytes.len() > mesh_link::MAX_FRAME_BYTES {
            return Err(Error::InvalidArgument);
        }
        Frame::new(bytes.to_vec())
            .map_err(|_| Error::InvalidArgument)
            .map(|frame| frame.encode())
    })
}
pub fn link_frame_decode(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    guarded(|| {
        if !(3..=mesh_link::MAX_FRAME_BYTES + 3).contains(&bytes.len()) {
            return Err(Error::InvalidArgument);
        }
        Frame::decode(bytes)
            .map_err(|_| Error::InvalidArgument)
            .map(|frame| frame.bytes().to_vec())
    })
}

/// Builds one canonical relay envelope before it enters a neighboring Noise
/// session. Callers provide canonical frame and durable-record bytes; malformed
/// or oversized values fail before any native transport allocation.
pub fn routed_record_encode(frame: &[u8], record: &[u8]) -> Result<Vec<u8>, Error> {
    guarded(|| {
        let frame = RelayFrame::decode(frame).map_err(|_| Error::InvalidArgument)?;
        let record = DurableRecord::decode(record).map_err(|_| Error::InvalidArgument)?;
        RoutedRecord { frame, record }
            .encode()
            .map_err(|_| Error::InvalidArgument)
    })
}

/// Validates one routed envelope and returns `RelayFrame::encode()` followed by
/// `DurableRecord::encode()`. The fixed first 91 bytes keep this FFI boundary
/// unambiguous without exposing store handles, rows, or private material.
pub fn routed_record_decode(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    guarded(|| {
        let routed = RoutedRecord::decode(bytes).map_err(|_| Error::InvalidArgument)?;
        let record = routed
            .record
            .encode()
            .map_err(|_| Error::InternalInvariant)?;
        let mut result = Vec::with_capacity(RELAY_FRAME_BYTES + record.len());
        result.extend(routed.frame.encode());
        result.extend(record);
        Ok(result)
    })
}

/// Result codes returned only to native hosts after authenticated ingress.
/// They are transport scheduling hints, never UI delivery claims.
pub const ROUTED_ACCEPTED_ANNOUNCEMENT: u8 = 1;
pub const ROUTED_ACCEPTED_CHUNK: u8 = 2;
pub const ROUTED_ACCEPTED_RECEIPT: u8 = 3;
pub const ROUTED_ACCEPTED_RECEIPT_ACK: u8 = 4;
// Completion packets are only created after every chunk has verified and the
// receipt transaction has committed. Version 2 deliberately carries the
// certified origin and verification moment next to the opaque object ID so
// product adapters never need to infer them from radio state.
const DELIVERED_TEXT_MAGIC: [u8; 2] = [0x74, 2];
const DELIVERED_TEXT_HEADER_BYTES: usize = 2 + 32 + 32 + 8 + 2;

/// Private native-host completion packet. It is deliberately not a radio
/// record: `{ magic v2, object id, signed origin, verified at, receipt length,
/// receipt, text length, text }` crosses only from the Rust store into the
/// Android/iOS host after SQLCipher
/// has committed the local delivery and signed receipt.
fn encode_delivered_text(
    proof: &mesh_protocol::VerifiedDelivery,
    receipt: &[u8],
    text: &[u8],
) -> Result<Vec<u8>, Error> {
    if receipt.is_empty()
        || receipt.len() > u16::MAX as usize
        || text.is_empty()
        || text.len() > u16::MAX as usize
    {
        return Err(Error::InternalInvariant);
    }
    let object = proof.object_id();
    let origin = proof.origin();
    let mut result =
        Vec::with_capacity(DELIVERED_TEXT_HEADER_BYTES + receipt.len() + 2 + text.len());
    result.extend(DELIVERED_TEXT_MAGIC);
    result.extend(object.0);
    result.extend(origin.0);
    result.extend(proof.verified_at().to_be_bytes());
    result.extend((receipt.len() as u16).to_be_bytes());
    result.extend(receipt);
    result.extend((text.len() as u16).to_be_bytes());
    result.extend(text);
    Ok(result)
}

/// Verifies, decrypts and atomically commits one complete incoming durable
/// text object. An empty result means no pending text exists. Text cannot
/// escape this call if roster verification, decryption, receipt signing or the
/// SQLCipher commit fails.
pub fn secure_store_finalize_next_text(
    handle: u64,
    identity_seed: &[u8],
    delivery_seed: &[u8],
    now: u64,
) -> Result<Vec<u8>, Error> {
    guarded(|| {
        if identity_seed.len() != 32 || delivery_seed.len() != 32 || now == 0 {
            return Err(Error::InvalidArgument);
        }
        let mut registry = stores().lock().map_err(|_| Error::InternalInvariant)?;
        let store = registry
            .stores
            .get_mut(&handle)
            .ok_or(Error::InvalidHandle)?;
        let roster = store
            .policy_bundle()
            .map_err(|_| Error::InternalInvariant)?
            .ok_or(Error::StaleRequest)?
            .verify(now)
            .map_err(|_| Error::StaleRequest)?;
        let Some(object) = store
            .next_authenticated_delivery(now)
            .map_err(|_| Error::InternalInvariant)?
        else {
            return Ok(Vec::new());
        };
        if store
            .manifest(object)
            .map_err(|_| Error::InternalInvariant)?
            .namespace()
            .as_str()
            != "mesh.chat.text.v1"
        {
            return Err(Error::StaleRequest);
        }
        let signer = IdentitySigningKey::import(zeroize::Zeroizing::new(
            identity_seed
                .try_into()
                .map_err(|_| Error::InvalidArgument)?,
        ));
        let secret = DeliverySecret::import(zeroize::Zeroizing::new(
            delivery_seed
                .try_into()
                .map_err(|_| Error::InvalidArgument)?,
        ))
        .map_err(|_| Error::InvalidArgument)?;
        let proof = store
            .verify_received(object, &roster, &secret, now)
            .map_err(|_| Error::InvalidArgument)?;
        let receipt = store
            .finalize_received(&proof, &roster, &signer, now)
            .map_err(|_| Error::InternalInvariant)?;
        encode_delivered_text(&proof, receipt.receipt(), proof.plaintext_after_commit())
    })
}

/// Creates one durable encrypted text object per bounded audience in the
/// current certified roster. The caller supplies the Keychain/Keystore-held
/// identity seed only for this call; no plaintext, seed, SQLCipher row or
/// operation handle crosses Flutter. The returned count is the number of
/// `RoutedRecord` slots currently added to the origin outbox.
pub fn secure_store_enqueue_text(
    handle: u64,
    identity_seed: &[u8],
    plaintext: &[u8],
    now: u64,
) -> Result<u16, Error> {
    enqueue_text(handle, identity_seed, plaintext, None, now)
}

/// As above, but binds the queued audiences to the caller's opaque 16-byte
/// conversation action ID. This ID is metadata local to the origin store; the
/// encrypted payload continues to carry the user-visible chat ID.
pub fn secure_store_enqueue_text_with_logical_id(
    handle: u64,
    identity_seed: &[u8],
    plaintext: &[u8],
    logical_id: [u8; 16],
    now: u64,
) -> Result<u16, Error> {
    enqueue_text(
        handle,
        identity_seed,
        plaintext,
        Some(mesh_store::LogicalMessageId(logical_id)),
        now,
    )
}

fn enqueue_text(
    handle: u64,
    identity_seed: &[u8],
    plaintext: &[u8],
    requested_logical: Option<mesh_store::LogicalMessageId>,
    now: u64,
) -> Result<u16, Error> {
    guarded(|| {
        if identity_seed.len() != 32
            || plaintext.is_empty()
            || plaintext.len() > mesh_protocol::MAX_PLAINTEXT
            || now == 0
        {
            return Err(Error::InvalidArgument);
        }
        let expires_at = now.checked_add(900).ok_or(Error::InvalidArgument)?;
        if expires_at > MAX_LOGICAL_TIME {
            return Err(Error::InvalidArgument);
        }
        let mut registry = stores().lock().map_err(|_| Error::InternalInvariant)?;
        let store = registry
            .stores
            .get_mut(&handle)
            .ok_or(Error::InvalidHandle)?;
        let roster = store
            .policy_bundle()
            .map_err(|_| Error::InternalInvariant)?
            .ok_or(Error::StaleRequest)?
            .verify(now)
            .map_err(|_| Error::StaleRequest)?;
        let local = store.local_member();
        let members = roster
            .member_claims()
            .into_iter()
            .map(|claim| claim.member)
            .collect::<Vec<_>>();
        let audiences = broadcast_audiences(local, &members).map_err(|_| Error::InvalidArgument)?;
        let signer = IdentitySigningKey::import(zeroize::Zeroizing::new(
            identity_seed
                .try_into()
                .map_err(|_| Error::InvalidArgument)?,
        ));
        let mut rng = OsRandom;
        let logical = match requested_logical {
            Some(logical) => logical,
            None => {
                let mut logical = [0u8; 16];
                rng.fill(&mut logical)
                    .map_err(|_| Error::InternalInvariant)?;
                mesh_store::LogicalMessageId(logical)
            }
        };
        let target_count = members.len().saturating_sub(1);
        store
            .begin_logical_message(logical, target_count, audiences.len(), expires_at, now)
            .map_err(|_| Error::InternalInvariant)?;
        let mut records: usize = 0;
        for audience in audiences {
            let mut operation = [0u8; 16];
            rng.fill(&mut operation)
                .map_err(|_| Error::InternalInvariant)?;
            let mut hash_input = Vec::with_capacity(16 + plaintext.len());
            hash_input.extend(operation);
            hash_input.extend(plaintext);
            let command_hash: [u8; 32] = Sha256::digest(&hash_input).into();
            let operation = OperationId(operation);
            let reservation = store
                .reserve(operation, command_hash)
                .map_err(|_| Error::InternalInvariant)?;
            let message = seal_message(
                SealRequest {
                    origin: local,
                    sequence: reservation.sequence,
                    policy: ObjectPolicy {
                        namespace: Namespace::new("mesh.chat.text.v1")
                            .map_err(|_| Error::InternalInvariant)?,
                        epoch: roster.scope().epoch,
                        targets: audience,
                        expires_at,
                        hop_limit: 16,
                    },
                    plaintext,
                    now,
                },
                &roster,
                &signer,
                &mut rng,
            )
            .map_err(|_| Error::InvalidArgument)?;
            records = records
                .checked_add(1 + message.object().chunks().len())
                .ok_or(Error::ResourcePressure)?;
            if records > u16::MAX as usize {
                return Err(Error::ResourcePressure);
            }
            store
                .commit_sealed_logical(operation, command_hash, &message, &roster, logical, now)
                .map_err(|_| Error::InternalInvariant)?;
        }
        u16::try_from(records).map_err(|_| Error::ResourcePressure)
    })
}

/// Opaque, bounded origin-side progress for the newest live local action:
/// 16-byte logical ID, total targets, delivered targets and state (0 queued,
/// 1 partial, 2 delivered). An empty vector means there is no live action.
pub fn secure_store_latest_delivery_summary(handle: u64, now: u64) -> Result<Vec<u8>, Error> {
    guarded(|| {
        if now == 0 {
            return Err(Error::InvalidArgument);
        }
        let registry = stores().lock().map_err(|_| Error::InternalInvariant)?;
        let store = registry.stores.get(&handle).ok_or(Error::InvalidHandle)?;
        let Some(summary) = store
            .latest_logical_delivery_summary(now)
            .map_err(|_| Error::InternalInvariant)?
        else {
            return Ok(Vec::new());
        };
        let state = match summary.state {
            mesh_store::LogicalDeliveryState::Queued => 0,
            mesh_store::LogicalDeliveryState::PartiallyDelivered => 1,
            mesh_store::LogicalDeliveryState::Delivered => 2,
            mesh_store::LogicalDeliveryState::Expired => 3,
        };
        let mut output = Vec::with_capacity(19);
        output.extend(summary.id.0);
        output.push(u8::try_from(summary.target_count).map_err(|_| Error::InternalInvariant)?);
        output.push(u8::try_from(summary.delivered_targets).map_err(|_| Error::InternalInvariant)?);
        output.push(state);
        Ok(output)
    })
}

/// Opaque, bounded origin-side progress for one explicit local action. The
/// caller provides its 16-byte logical message ID, so a newer send can never
/// overwrite the receipt status shown for an earlier chat bubble.
pub fn secure_store_delivery_summary(
    handle: u64,
    logical_id: [u8; 16],
    now: u64,
) -> Result<Vec<u8>, Error> {
    guarded(|| {
        if now == 0 {
            return Err(Error::InvalidArgument);
        }
        let registry = stores().lock().map_err(|_| Error::InternalInvariant)?;
        let store = registry.stores.get(&handle).ok_or(Error::InvalidHandle)?;
        let Some(summary) = store
            .logical_delivery_summary(mesh_store::LogicalMessageId(logical_id), now)
            .map_err(|_| Error::InternalInvariant)?
        else {
            return Ok(Vec::new());
        };
        let state = match summary.state {
            mesh_store::LogicalDeliveryState::Queued => 0,
            mesh_store::LogicalDeliveryState::PartiallyDelivered => 1,
            mesh_store::LogicalDeliveryState::Delivered => 2,
            mesh_store::LogicalDeliveryState::Expired => 3,
        };
        let mut output = Vec::with_capacity(19);
        output.extend(summary.id.0);
        output.push(u8::try_from(summary.target_count).map_err(|_| Error::InternalInvariant)?);
        output.push(u8::try_from(summary.delivered_targets).map_err(|_| Error::InternalInvariant)?);
        output.push(state);
        Ok(output)
    })
}

/// Reconstructs one origin outbox record after a radio reconnect or process
/// restart. `RelayId` derives from the opaque object ID, which is already
/// salted by `seal_message`; no new route state must be kept in RAM.
pub fn secure_store_outbox_record(handle: u64, slot: u16, now: u64) -> Result<Vec<u8>, Error> {
    guarded(|| {
        if now == 0 {
            return Err(Error::InvalidArgument);
        }
        let mut registry = stores().lock().map_err(|_| Error::InternalInvariant)?;
        let store = registry
            .stores
            .get_mut(&handle)
            .ok_or(Error::InvalidHandle)?;
        let roster = store
            .policy_bundle()
            .map_err(|_| Error::InternalInvariant)?
            .ok_or(Error::StaleRequest)?
            .verify(now)
            .map_err(|_| Error::StaleRequest)?;
        let mut remaining = usize::from(slot);
        for object in store.outbox(now).map_err(|_| Error::InternalInvariant)? {
            let manifest = store
                .manifest(object)
                .map_err(|_| Error::InternalInvariant)?;
            let count = 1 + manifest.chunk_count();
            if remaining >= count {
                remaining -= count;
                continue;
            }
            let mut relay_id = [0u8; 16];
            relay_id.copy_from_slice(&object.0[..16]);
            let frame = RelayFrame {
                id: RelayId(relay_id),
                origin: manifest.origin(),
                previous_hop: store.local_member(),
                hops: 0,
                hop_limit: manifest.hop_limit(),
                expires_at: manifest.expires_at(),
            };
            let record = if remaining == 0 {
                DurableRecord::Announcement(
                    store
                        .authenticated_announcement(object, &roster, now)
                        .map_err(|_| Error::InternalInvariant)?
                        .bytes()
                        .to_vec(),
                )
            } else {
                let index = remaining - 1;
                DurableRecord::Chunk {
                    object,
                    index: u16::try_from(index).map_err(|_| Error::InternalInvariant)?,
                    bytes: store
                        .chunk(object, index)
                        .map_err(|_| Error::InternalInvariant)?,
                }
            };
            return RoutedRecord { frame, record }
                .encode()
                .map_err(|_| Error::InternalInvariant);
        }
        Ok(Vec::new())
    })
}

/// Reconstructs one durable local receipt for retry on reconnect. Receipt
/// scheduling deliberately replays after a radio loss; the origin records an
/// actor only once and does not retire its source outbox until every target
/// has confirmed.
pub fn secure_store_receipt_record(handle: u64, slot: u16, now: u64) -> Result<Vec<u8>, Error> {
    guarded(|| {
        if now == 0 {
            return Err(Error::InvalidArgument);
        }
        let mut registry = stores().lock().map_err(|_| Error::InternalInvariant)?;
        let store = registry
            .stores
            .get_mut(&handle)
            .ok_or(Error::InvalidHandle)?;
        let ids = store
            .local_receipt_outbox(now)
            .map_err(|_| Error::InternalInvariant)?;
        let Some(object) = ids.get(usize::from(slot)).copied() else {
            return Ok(Vec::new());
        };
        let receipt = store
            .local_receipt(object)
            .map_err(|_| Error::InternalInvariant)?
            .ok_or(Error::InternalInvariant)?;
        let manifest = store
            .manifest(object)
            .map_err(|_| Error::InternalInvariant)?;
        let digest: [u8; 32] = Sha256::digest(receipt.receipt()).into();
        let mut relay_id = [0u8; 16];
        relay_id.copy_from_slice(&digest[..16]);
        RoutedRecord {
            frame: RelayFrame {
                id: RelayId(relay_id),
                origin: store.local_member(),
                previous_hop: store.local_member(),
                hops: 0,
                hop_limit: manifest.hop_limit(),
                expires_at: manifest.expires_at(),
            },
            record: DurableRecord::Receipt(receipt.receipt().to_vec()),
        }
        .encode()
        .map_err(|_| Error::InternalInvariant)
    })
}

/// Reconstructs one origin-signed ACK for a receipt already committed in the
/// origin's encrypted database. The signing seed enters this boundary only for
/// the signing operation and is never persisted by SQLCipher.
pub fn secure_store_receipt_ack_record(
    handle: u64,
    identity_seed: &[u8],
    slot: u16,
    now: u64,
) -> Result<Vec<u8>, Error> {
    guarded(|| {
        if now == 0 || identity_seed.len() != 32 {
            return Err(Error::InvalidArgument);
        }
        let mut registry = stores().lock().map_err(|_| Error::InternalInvariant)?;
        let store = registry
            .stores
            .get_mut(&handle)
            .ok_or(Error::InvalidHandle)?;
        let roster = store
            .policy_bundle()
            .map_err(|_| Error::InternalInvariant)?
            .ok_or(Error::StaleRequest)?
            .verify(now)
            .map_err(|_| Error::StaleRequest)?;
        let Some((object, actor, receipt)) = store
            .target_receipt_ack_outbox(now)
            .map_err(|_| Error::InternalInvariant)?
            .get(usize::from(slot))
            .cloned()
        else {
            return Ok(Vec::new());
        };
        let signer = IdentitySigningKey::import(zeroize::Zeroizing::new(
            identity_seed
                .try_into()
                .map_err(|_| Error::InvalidArgument)?,
        ));
        if signer.public_key()
            != roster
                .signing_key(store.local_member())
                .map_err(|_| Error::StaleRequest)?
        {
            return Err(Error::InvalidArgument);
        }
        let manifest = store
            .manifest(object)
            .map_err(|_| Error::InternalInvariant)?;
        let ack = issue_receipt_ack(
            &receipt,
            object,
            store.local_member(),
            actor,
            roster.scope(),
            &signer,
            now,
        )
        .map_err(|_| Error::InternalInvariant)?;
        let digest: [u8; 32] = Sha256::digest(&ack).into();
        let mut relay_id = [0u8; 16];
        relay_id.copy_from_slice(&digest[..16]);
        RoutedRecord {
            frame: RelayFrame {
                id: RelayId(relay_id),
                origin: store.local_member(),
                previous_hop: store.local_member(),
                hops: 0,
                hop_limit: manifest.hop_limit(),
                expires_at: manifest.expires_at(),
            },
            record: DurableRecord::ReceiptAck(ack),
        }
        .encode()
        .map_err(|_| Error::InternalInvariant)
    })
}

/// Commits one routed announcement or chunk to the encrypted store. This API
/// is intentionally called *after* the native Noise session has authenticated
/// its neighbor. It does not forward anything and never exposes stored bytes.
pub fn secure_store_accept_routed(
    handle: u64,
    bytes: &[u8],
    received_from: &[u8],
    now: u64,
) -> Result<u8, Error> {
    guarded(|| {
        if now == 0
            || bytes.is_empty()
            || bytes.len() > mesh_replication::MAX_ROUTED_RECORD
            || received_from.len() != 32
        {
            return Err(Error::InvalidArgument);
        }
        let received_from = MemberId(
            received_from
                .try_into()
                .map_err(|_| Error::InvalidArgument)?,
        );
        let routed = RoutedRecord::decode(bytes).map_err(|_| Error::InvalidArgument)?;
        let mut registry = stores().lock().map_err(|_| Error::InternalInvariant)?;
        let store = registry
            .stores
            .get_mut(&handle)
            .ok_or(Error::InvalidHandle)?;
        let roster = store
            .policy_bundle()
            .map_err(|_| Error::InternalInvariant)?
            .ok_or(Error::StaleRequest)?
            .verify(now)
            .map_err(|_| Error::StaleRequest)?;
        match routed.record {
            DurableRecord::Announcement(bytes) => {
                let announcement = mesh_protocol::authenticate_announcement(&bytes, &roster, now)
                    .map_err(|_| Error::InvalidArgument)?;
                if announcement.manifest().origin() != routed.frame.origin {
                    return Err(Error::InvalidArgument);
                }
                store
                    .announce_authenticated(&announcement, &roster, now)
                    .map_err(|_| Error::InvalidArgument)?;
                Ok(ROUTED_ACCEPTED_ANNOUNCEMENT)
            }
            DurableRecord::Chunk {
                object,
                index,
                bytes,
            } => {
                if store
                    .manifest(object)
                    .map_err(|_| Error::InvalidArgument)?
                    .origin()
                    != routed.frame.origin
                {
                    return Err(Error::InvalidArgument);
                }
                store
                    .put_relay_chunk(
                        object,
                        usize::from(index),
                        &bytes,
                        routed.frame,
                        received_from,
                        now,
                    )
                    .map_err(|_| Error::InvalidArgument)?;
                Ok(ROUTED_ACCEPTED_CHUNK)
            }
            DurableRecord::Receipt(bytes) => {
                let route = receipt_route(&bytes).map_err(|_| Error::InvalidArgument)?;
                if route.actor != routed.frame.origin {
                    return Err(Error::InvalidArgument);
                }
                // A relay does not need the source object to forward a signed
                // receipt. When this is its origin, it verifies and records it
                // before reporting the same forwarding hint; duplicate actor
                // receipts are idempotent in SQLCipher.
                let is_origin = store
                    .manifest(route.object)
                    .map(|manifest| manifest.origin() == store.local_member())
                    .unwrap_or(false);
                if is_origin {
                    let proof = store
                        .verify_target_receipt(route.object, &bytes, &roster, now)
                        .map_err(|_| Error::InvalidArgument)?;
                    store
                        .record_target_receipt(&proof, &roster, now)
                        .map_err(|_| Error::InternalInvariant)?;
                } else {
                    store
                        .put_relay_receipt(&bytes, routed.frame, received_from, now)
                        .map_err(|_| Error::InvalidArgument)?;
                }
                Ok(ROUTED_ACCEPTED_RECEIPT)
            }
            DurableRecord::ReceiptAck(bytes) => {
                let route = receipt_ack_route(&bytes).map_err(|_| Error::InvalidArgument)?;
                if route.origin != routed.frame.origin {
                    return Err(Error::InvalidArgument);
                }
                let proof =
                    verify_receipt_ack(&bytes, &roster, now).map_err(|_| Error::InvalidArgument)?;
                if route.actor == store.local_member() {
                    store
                        .record_receipt_ack(&proof, &roster, now)
                        .map_err(|_| Error::InvalidArgument)?;
                } else if route.origin != store.local_member() {
                    store
                        .put_relay_receipt_ack(&bytes, routed.frame, received_from, now)
                        .map_err(|_| Error::InvalidArgument)?;
                    store
                        .remove_relay_receipt(route.receipt_id)
                        .map_err(|_| Error::InternalInvariant)?;
                }
                Ok(ROUTED_ACCEPTED_RECEIPT_ACK)
            }
        }
    })
}

/// Returns one persisted routed record from the oldest relay-custody object.
/// Slot zero is its signed announcement; following slots are canonical chunks.
/// An empty result means no custodial object is currently eligible to forward.
pub fn secure_store_relay_record(handle: u64, slot: u16, now: u64) -> Result<Vec<u8>, Error> {
    guarded(|| {
        if now == 0 {
            return Err(Error::InvalidArgument);
        }
        let mut registry = stores().lock().map_err(|_| Error::InternalInvariant)?;
        let store = registry
            .stores
            .get_mut(&handle)
            .ok_or(Error::InvalidHandle)?;
        let roster = store
            .policy_bundle()
            .map_err(|_| Error::InternalInvariant)?
            .ok_or(Error::StaleRequest)?
            .verify(now)
            .map_err(|_| Error::StaleRequest)?;
        let Some(custody) = store
            .relay_queue(now)
            .map_err(|_| Error::InternalInvariant)?
            .into_iter()
            .next()
        else {
            return Ok(Vec::new());
        };
        let record = if slot == 0 {
            DurableRecord::Announcement(
                store
                    .authenticated_announcement(custody.object_id, &roster, now)
                    .map_err(|_| Error::InternalInvariant)?
                    .bytes()
                    .to_vec(),
            )
        } else {
            let index = usize::from(slot - 1);
            let manifest = store
                .manifest(custody.object_id)
                .map_err(|_| Error::InternalInvariant)?;
            if index >= manifest.chunk_count() {
                return Ok(Vec::new());
            }
            DurableRecord::Chunk {
                object: custody.object_id,
                index: index as u16,
                bytes: store
                    .chunk(custody.object_id, index)
                    .map_err(|_| Error::InternalInvariant)?,
            }
        };
        RoutedRecord {
            frame: custody.frame,
            record,
        }
        .encode()
        .map_err(|_| Error::InternalInvariant)
    })
}

/// Returns the authenticated neighbor that supplied the oldest relay-custody
/// object. It pairs with `secure_store_relay_record`: an empty result means
/// that there is no eligible object, never an all-zero member identity.
pub fn secure_store_relay_received_from(handle: u64, now: u64) -> Result<Vec<u8>, Error> {
    guarded(|| {
        if now == 0 {
            return Err(Error::InvalidArgument);
        }
        let mut registry = stores().lock().map_err(|_| Error::InternalInvariant)?;
        let store = registry
            .stores
            .get_mut(&handle)
            .ok_or(Error::InvalidHandle)?;
        let Some(custody) = store
            .relay_queue(now)
            .map_err(|_| Error::InternalInvariant)?
            .into_iter()
            .next()
        else {
            return Ok(Vec::new());
        };
        Ok(custody.received_from.0.to_vec())
    })
}

/// Returns one persisted receipt relay record. Unlike a source receipt, this
/// is work custodied by an intermediate node and therefore survives its
/// process restart before another authenticated neighbor becomes available.
pub fn secure_store_relay_receipt_record(
    handle: u64,
    slot: u16,
    now: u64,
) -> Result<Vec<u8>, Error> {
    guarded(|| {
        if now == 0 {
            return Err(Error::InvalidArgument);
        }
        let mut registry = stores().lock().map_err(|_| Error::InternalInvariant)?;
        let store = registry
            .stores
            .get_mut(&handle)
            .ok_or(Error::InvalidHandle)?;
        let Some(custody) = store
            .relay_receipt_queue(now)
            .map_err(|_| Error::InternalInvariant)?
            .get(usize::from(slot))
            .cloned()
        else {
            return Ok(Vec::new());
        };
        RoutedRecord {
            frame: custody.frame,
            record: DurableRecord::Receipt(custody.receipt),
        }
        .encode()
        .map_err(|_| Error::InternalInvariant)
    })
}

/// Pairs with `secure_store_relay_receipt_record`: it returns the ingress
/// neighbor for the oldest durable receipt relay item, or empty when none is
/// pending. Hosts use it to avoid sending the receipt straight back.
pub fn secure_store_relay_receipt_received_from(handle: u64, now: u64) -> Result<Vec<u8>, Error> {
    guarded(|| {
        if now == 0 {
            return Err(Error::InvalidArgument);
        }
        let mut registry = stores().lock().map_err(|_| Error::InternalInvariant)?;
        let store = registry
            .stores
            .get_mut(&handle)
            .ok_or(Error::InvalidHandle)?;
        let Some(custody) = store
            .relay_receipt_queue(now)
            .map_err(|_| Error::InternalInvariant)?
            .into_iter()
            .next()
        else {
            return Ok(Vec::new());
        };
        Ok(custody.received_from.0.to_vec())
    })
}

/// Returns one restart-safe origin ACK held by an intermediate relay.
pub fn secure_store_relay_receipt_ack_record(
    handle: u64,
    slot: u16,
    now: u64,
) -> Result<Vec<u8>, Error> {
    guarded(|| {
        if now == 0 {
            return Err(Error::InvalidArgument);
        }
        let mut registry = stores().lock().map_err(|_| Error::InternalInvariant)?;
        let store = registry
            .stores
            .get_mut(&handle)
            .ok_or(Error::InvalidHandle)?;
        let Some(custody) = store
            .relay_receipt_ack_queue(now)
            .map_err(|_| Error::InternalInvariant)?
            .get(usize::from(slot))
            .cloned()
        else {
            return Ok(Vec::new());
        };
        RoutedRecord {
            frame: custody.frame,
            record: DurableRecord::ReceiptAck(custody.ack),
        }
        .encode()
        .map_err(|_| Error::InternalInvariant)
    })
}

pub fn secure_store_relay_receipt_ack_received_from(
    handle: u64,
    now: u64,
) -> Result<Vec<u8>, Error> {
    guarded(|| {
        if now == 0 {
            return Err(Error::InvalidArgument);
        }
        let mut registry = stores().lock().map_err(|_| Error::InternalInvariant)?;
        let store = registry
            .stores
            .get_mut(&handle)
            .ok_or(Error::InvalidHandle)?;
        let Some(custody) = store
            .relay_receipt_ack_queue(now)
            .map_err(|_| Error::InternalInvariant)?
            .into_iter()
            .next()
        else {
            return Ok(Vec::new());
        };
        Ok(custody.received_from.0.to_vec())
    })
}

/// Opens a bounded in-memory dedupe gate for one local member. Durable object
/// custody remains in SQLCipher; this gate only prevents live forwarding loops.
pub fn relay_gate_open(member: &[u8]) -> Result<u64, Error> {
    guarded(|| {
        let member = MemberId(member.try_into().map_err(|_| Error::InvalidArgument)?);
        let mut registry = relay_gates().lock().map_err(|_| Error::InternalInvariant)?;
        if registry.gates.len() >= MAX_RUNTIMES || registry.next >= MAX_COUNTER {
            return Err(Error::ResourcePressure);
        }
        let handle = registry.next;
        registry.next += 1;
        registry.gates.insert(handle, RelayCache::new(member));
        Ok(handle)
    })
}

/// Returns `[1 | updated RelayFrame]` when forwarding is allowed, `[2]` for a
/// duplicate, `[3]` for expiry and `[4]` when the hop budget is exhausted.
pub fn relay_gate_accept(
    handle: u64,
    frame: &[u8],
    via: &[u8],
    now: u64,
) -> Result<Vec<u8>, Error> {
    guarded(|| {
        let frame = RelayFrame::decode(frame).map_err(|_| Error::InvalidArgument)?;
        let via = MemberId(via.try_into().map_err(|_| Error::InvalidArgument)?);
        let mut registry = relay_gates().lock().map_err(|_| Error::InternalInvariant)?;
        let gate = registry
            .gates
            .get_mut(&handle)
            .ok_or(Error::InvalidHandle)?;
        match gate
            .accept(frame, via, now)
            .map_err(|_| Error::InvalidArgument)?
        {
            RelayDecision::Forward(frame) => {
                let mut output = Vec::with_capacity(1 + RELAY_FRAME_BYTES);
                output.push(1);
                output.extend(frame.encode());
                Ok(output)
            }
            RelayDecision::Duplicate => Ok(vec![2]),
            RelayDecision::Expired => Ok(vec![3]),
            RelayDecision::HopLimit => Ok(vec![4]),
        }
    })
}
pub fn relay_gate_release(handle: u64) -> Result<(), Error> {
    guarded(|| {
        relay_gates()
            .lock()
            .map_err(|_| Error::InternalInvariant)?
            .gates
            .remove(&handle);
        Ok(())
    })
}

/// # Safety
/// `member` points to exactly 32 public member bytes. `out` is writable and
/// receives an opaque process-local gate handle.
#[no_mangle]
pub unsafe extern "C" fn mesh_relay_gate_open(
    member: *const u8,
    member_len: usize,
    out: *mut u64,
) -> i32 {
    if out.is_null() || !(out as usize).is_multiple_of(std::mem::align_of::<u64>()) {
        return Error::InvalidArgument as i32;
    }
    unsafe { *out = 0 }
    if member.is_null() || member_len != 32 {
        return Error::InvalidArgument as i32;
    }
    status(
        guarded(|| relay_gate_open(unsafe { std::slice::from_raw_parts(member, member_len) })),
        |handle| unsafe { *out = handle },
    )
}
/// # Safety
/// `frame` and `via` point to exact canonical public routing metadata. `out`
/// receives a tagged decision and must be released with `mesh_buffer_release`.
#[no_mangle]
pub unsafe extern "C" fn mesh_relay_gate_accept(
    handle: u64,
    frame: *const u8,
    frame_len: usize,
    via: *const u8,
    via_len: usize,
    now: u64,
    out: *mut MeshBuffer,
) -> i32 {
    if out.is_null() || !(out as usize).is_multiple_of(std::mem::align_of::<MeshBuffer>()) {
        return Error::InvalidArgument as i32;
    }
    unsafe { *out = MeshBuffer::default() }
    if frame.is_null()
        || via.is_null()
        || frame_len != RELAY_FRAME_BYTES
        || via_len != 32
        || now == 0
    {
        return Error::InvalidArgument as i32;
    }
    let result = guarded(|| {
        let output = relay_gate_accept(
            handle,
            unsafe { std::slice::from_raw_parts(frame, frame_len) },
            unsafe { std::slice::from_raw_parts(via, via_len) },
            now,
        )?
        .into_boxed_slice();
        let len = output.len();
        let ptr = Box::into_raw(output) as *mut u8;
        Ok(MeshBuffer { ptr, len })
    });
    status(result, |buffer| unsafe { *out = buffer })
}
#[no_mangle]
pub extern "C" fn mesh_relay_gate_release(handle: u64) -> i32 {
    status(relay_gate_release(handle), |_| {})
}

const GROUP_CERTIFICATE_LIFETIME: u64 = 7 * 24 * 60 * 60;
const ENROLLMENT_REQUEST_LIFETIME: u64 = 15 * 60;

/// Creates the first, self-certified policy for a lab group. Both private inputs
/// arrive only from the native protected-key host and are zeroized before return.
/// Adding a second device requires a separately signed enrollment flow.
pub fn secure_store_create_group(
    handle: u64,
    identity_seed: &[u8],
    delivery_seed: &[u8],
    member: &[u8],
    now: u64,
) -> Result<u64, Error> {
    guarded(|| {
        if identity_seed.len() != 32
            || delivery_seed.len() != 32
            || member.len() != 32
            || now == 0
            || now > MAX_LOGICAL_TIME.saturating_sub(GROUP_CERTIFICATE_LIFETIME)
        {
            return Err(Error::InvalidArgument);
        }
        let mut identity_bytes = zeroize::Zeroizing::new([0; 32]);
        identity_bytes.copy_from_slice(identity_seed);
        let identity = IdentitySigningKey::import(identity_bytes);
        let mut delivery_bytes = zeroize::Zeroizing::new([0; 32]);
        delivery_bytes.copy_from_slice(delivery_seed);
        let delivery =
            DeliverySecret::import(delivery_bytes).map_err(|_| Error::InvalidArgument)?;
        let member = MemberId(member.try_into().map_err(|_| Error::InvalidArgument)?);
        if member.0 != identity.public_key() {
            return Err(Error::InvalidArgument);
        }
        let mut group = [0; 32];
        OsRandom
            .fill(&mut group)
            .map_err(|_| Error::InternalInvariant)?;
        let scope = Scope { group, epoch: 1 };
        let certificate = issue_certificate(
            &identity,
            &CertificateClaims {
                group,
                member,
                signing_key: identity.public_key(),
                delivery_key: delivery.public_key(),
                valid_from: now,
                valid_until: now + GROUP_CERTIFICATE_LIFETIME,
                epoch: scope.epoch,
                serial: 1,
            },
        )
        .map_err(|_| Error::InternalInvariant)?;
        let mut registry = stores().lock().map_err(|_| Error::InternalInvariant)?;
        let store = registry
            .stores
            .get_mut(&handle)
            .ok_or(Error::InvalidHandle)?;
        if store
            .active_policy(now)
            .map_err(|_| Error::InternalInvariant)?
            .is_some()
        {
            return Err(Error::StaleRequest);
        }
        store
            .install_policy(identity.public_key(), scope, &[certificate], &[], now)
            .map_err(|_| Error::InternalInvariant)?;
        Ok(scope.epoch)
    })
}

/// The authority advances the roster after a verified public enrollment request.
/// The returned bundle is public transport data; each certificate remains bound to
/// the authority and a recipient still validates local membership before install.
pub fn secure_store_issue_enrollment(
    handle: u64,
    identity_seed: &[u8],
    request_bytes: &[u8],
    now: u64,
) -> Result<Vec<u8>, Error> {
    guarded(|| {
        if identity_seed.len() != 32
            || request_bytes.is_empty()
            || request_bytes.len() > 512
            || now == 0
        {
            return Err(Error::InvalidArgument);
        }
        let request =
            verify_enrollment_request(request_bytes, now).map_err(|_| Error::InvalidArgument)?;
        let mut seed = zeroize::Zeroizing::new([0; 32]);
        seed.copy_from_slice(identity_seed);
        let authority = IdentitySigningKey::import(seed);
        let mut registry = stores().lock().map_err(|_| Error::InternalInvariant)?;
        let store = registry
            .stores
            .get_mut(&handle)
            .ok_or(Error::InvalidHandle)?;
        let active = store
            .active_policy(now)
            .map_err(|_| Error::InternalInvariant)?
            .ok_or(Error::InvalidArgument)?;
        if active.authority != authority.public_key() || request.scope != active.scope {
            return Err(Error::InvalidArgument);
        }
        let bundle = store
            .policy_bundle()
            .map_err(|_| Error::InternalInvariant)?
            .ok_or(Error::InternalInvariant)?;
        let roster = bundle.verify(now).map_err(|_| Error::InternalInvariant)?;
        if roster.contains_member(request.member)
            || roster.member_claims().len() >= MAX_GROUP_MEMBERS
        {
            return Err(Error::StaleRequest);
        }
        let next_epoch = active
            .scope
            .epoch
            .checked_add(1)
            .ok_or(Error::InvalidArgument)?;
        if next_epoch > MAX_LOGICAL_TIME {
            return Err(Error::InvalidArgument);
        }
        let valid_until = now
            .checked_add(GROUP_CERTIFICATE_LIFETIME)
            .ok_or(Error::InvalidArgument)?;
        if valid_until > MAX_LOGICAL_TIME {
            return Err(Error::InvalidArgument);
        }
        let scope = Scope {
            group: active.scope.group,
            epoch: next_epoch,
        };
        let mut claims = roster.member_claims();
        for claim in &mut claims {
            claim.epoch = next_epoch;
            claim.valid_from = now;
            claim.valid_until = valid_until;
        }
        let next_serial = claims
            .iter()
            .map(|claim| claim.serial)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or(Error::InvalidArgument)?;
        claims.push(CertificateClaims {
            group: scope.group,
            member: request.member,
            signing_key: request.signing_key,
            delivery_key: request.delivery_key,
            valid_from: now,
            valid_until,
            epoch: next_epoch,
            serial: next_serial,
        });
        let certificates = claims
            .iter()
            .map(|claim| issue_certificate(&authority, claim).map_err(|_| Error::InternalInvariant))
            .collect::<Result<Vec<_>, _>>()?;
        store
            .install_policy(active.authority, scope, &certificates, &bundle.revoked, now)
            .map_err(|_| Error::InternalInvariant)?;
        PolicyBundle {
            authority: active.authority,
            scope,
            certificates,
            revoked: bundle.revoked,
        }
        .encode()
        .map_err(|_| Error::InternalInvariant)
    })
}

/// Reads only the applicant's certified public member identity from a bounded
/// enrollment request. Hosts use this before issuing a policy so a product can
/// compare the request against its server-authorized roster. No private key,
/// delivery key, group secret, or policy is exposed.
pub fn enrollment_request_member(request_bytes: &[u8], now: u64) -> Result<MemberId, Error> {
    guarded(|| {
        if request_bytes.is_empty() || request_bytes.len() > 512 || now == 0 {
            return Err(Error::InvalidArgument);
        }
        Ok(verify_enrollment_request(request_bytes, now)
            .map_err(|_| Error::InvalidArgument)?
            .member)
    })
}

pub fn secure_store_install_policy(
    handle: u64,
    bundle_bytes: &[u8],
    now: u64,
) -> Result<u64, Error> {
    guarded(|| {
        if bundle_bytes.is_empty()
            || bundle_bytes.len() > mesh_protocol::MAX_POLICY_BUNDLE
            || now == 0
        {
            return Err(Error::InvalidArgument);
        }
        let bundle = PolicyBundle::decode(bundle_bytes).map_err(|_| Error::InvalidArgument)?;
        let mut registry = stores().lock().map_err(|_| Error::InternalInvariant)?;
        let store = registry
            .stores
            .get_mut(&handle)
            .ok_or(Error::InvalidHandle)?;
        Ok(store
            .install_policy(
                bundle.authority,
                bundle.scope,
                &bundle.certificates,
                &bundle.revoked,
                now,
            )
            .map_err(|_| Error::InternalInvariant)?
            .scope
            .epoch)
    })
}

/// Exports the creator's currently verified policy as public invitation data.
/// This never exports a private key or opens access on the receiving device.
pub fn secure_store_export_policy(handle: u64, now: u64) -> Result<Vec<u8>, Error> {
    guarded(|| {
        if now == 0 {
            return Err(Error::InvalidArgument);
        }
        let registry = stores().lock().map_err(|_| Error::InternalInvariant)?;
        let store = registry.stores.get(&handle).ok_or(Error::InvalidHandle)?;
        store
            .active_policy(now)
            .map_err(|_| Error::InternalInvariant)?
            .ok_or(Error::InvalidArgument)?;
        store
            .policy_bundle()
            .map_err(|_| Error::InternalInvariant)?
            .ok_or(Error::InternalInvariant)?
            .encode()
            .map_err(|_| Error::InternalInvariant)
    })
}

/// A prospective member verifies the public invitation, then proves possession
/// of its protected keys in a short-lived enrollment request.
pub fn create_enrollment_request_from_policy(
    identity_seed: &[u8],
    delivery_seed: &[u8],
    invitation: &[u8],
    now: u64,
) -> Result<Vec<u8>, Error> {
    guarded(|| {
        if identity_seed.len() != 32
            || delivery_seed.len() != 32
            || invitation.is_empty()
            || invitation.len() > mesh_protocol::MAX_POLICY_BUNDLE
            || now == 0
        {
            return Err(Error::InvalidArgument);
        }
        let invitation = PolicyBundle::decode(invitation).map_err(|_| Error::InvalidArgument)?;
        invitation.verify(now).map_err(|_| Error::InvalidArgument)?;
        let valid_until = now
            .checked_add(ENROLLMENT_REQUEST_LIFETIME)
            .ok_or(Error::InvalidArgument)?;
        let mut identity_bytes = zeroize::Zeroizing::new([0; 32]);
        identity_bytes.copy_from_slice(identity_seed);
        let identity = IdentitySigningKey::import(identity_bytes);
        let mut delivery_bytes = zeroize::Zeroizing::new([0; 32]);
        delivery_bytes.copy_from_slice(delivery_seed);
        let delivery =
            DeliverySecret::import(delivery_bytes).map_err(|_| Error::InvalidArgument)?;
        mesh_protocol::create_enrollment_request(
            &identity,
            &delivery,
            invitation.scope,
            now,
            valid_until,
            &mut OsRandom,
        )
        .map_err(|_| Error::InternalInvariant)
    })
}
#[repr(C)]
pub struct MeshBuffer {
    pub ptr: *mut u8,
    pub len: usize,
}
impl Default for MeshBuffer {
    fn default() -> Self {
        Self {
            ptr: std::ptr::null_mut(),
            len: 0,
        }
    }
}
fn status<T>(result: Result<T, Error>, success: impl FnOnce(T)) -> i32 {
    match result {
        Ok(value) => {
            success(value);
            0
        }
        Err(e) => e as i32,
    }
}
#[no_mangle]
pub extern "C" fn mesh_abi_version() -> u32 {
    ABI_VERSION
}
/// # Safety
/// `out` must be aligned and writable for one u64 for the duration of this call.
#[no_mangle]
pub unsafe extern "C" fn mesh_runtime_create(version: u32, out: *mut u64) -> i32 {
    if out.is_null() || !(out as usize).is_multiple_of(std::mem::align_of::<u64>()) {
        return Error::InvalidArgument as i32;
    }
    // SAFETY: caller guarantees a writable, aligned out pointer.
    unsafe {
        *out = 0;
    }
    status(create(version), |id| unsafe {
        *out = id;
    })
}
/// # Safety
/// `input` must reference `len` readable bytes. `out` must be writable/aligned and
/// must not contain an unreleased output. Input is borrowed only for this call.
#[no_mangle]
pub unsafe extern "C" fn mesh_runtime_request(
    handle: u64,
    input: *const u8,
    len: usize,
    out: *mut MeshBuffer,
) -> i32 {
    if out.is_null() || !(out as usize).is_multiple_of(std::mem::align_of::<MeshBuffer>()) {
        return Error::InvalidArgument as i32;
    }
    unsafe {
        *out = MeshBuffer::default();
    }
    if input.is_null() || len == 0 || len > MAX_INPUT {
        return Error::InvalidArgument as i32;
    }
    let result = guarded(|| {
        // SAFETY: pointer validity is the C caller's contract; length is bounded before slicing.
        let bytes = unsafe { std::slice::from_raw_parts(input, len) };
        let output = request(handle, bytes)?.into_boxed_slice();
        let len = output.len();
        let ptr = Box::into_raw(output) as *mut u8;
        Ok(MeshBuffer { ptr, len })
    });
    status(result, |buffer| unsafe {
        *out = buffer;
    })
}
/// # Safety
/// Pass an unmodified buffer returned by mesh_runtime_request exactly once.
/// Null/empty buffers are allowed. Never pass a Swift/Kotlin allocation here.
#[no_mangle]
pub unsafe extern "C" fn mesh_buffer_release(buffer: MeshBuffer) {
    if buffer.ptr.is_null() {
        return;
    }
    let _ = guarded(|| {
        unsafe {
            drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                buffer.ptr, buffer.len,
            )));
        }
        Ok(())
    });
}
/// # Safety
/// `input` references bounded readable bytes. `out` is writable/aligned and the
/// returned buffer must be released exactly once with `mesh_buffer_release`.
#[no_mangle]
pub unsafe extern "C" fn mesh_link_frame_encode(
    input: *const u8,
    input_len: usize,
    out: *mut MeshBuffer,
) -> i32 {
    if out.is_null() || !(out as usize).is_multiple_of(std::mem::align_of::<MeshBuffer>()) {
        return Error::InvalidArgument as i32;
    }
    unsafe { *out = MeshBuffer::default() }
    if input.is_null() || input_len == 0 || input_len > mesh_link::MAX_FRAME_BYTES {
        return Error::InvalidArgument as i32;
    }
    let result = guarded(|| {
        let bytes = unsafe { std::slice::from_raw_parts(input, input_len) };
        let output = link_frame_encode(bytes)?.into_boxed_slice();
        let len = output.len();
        let ptr = Box::into_raw(output) as *mut u8;
        Ok(MeshBuffer { ptr, len })
    });
    status(result, |buffer| unsafe { *out = buffer })
}
/// # Safety
/// `input` references one complete framed transport record. The output is the
/// unmodified protected-session bytes and must be released exactly once.
#[no_mangle]
pub unsafe extern "C" fn mesh_link_frame_decode(
    input: *const u8,
    input_len: usize,
    out: *mut MeshBuffer,
) -> i32 {
    if out.is_null() || !(out as usize).is_multiple_of(std::mem::align_of::<MeshBuffer>()) {
        return Error::InvalidArgument as i32;
    }
    unsafe { *out = MeshBuffer::default() }
    if input.is_null() || !(3..=mesh_link::MAX_FRAME_BYTES + 3).contains(&input_len) {
        return Error::InvalidArgument as i32;
    }
    let result = guarded(|| {
        let bytes = unsafe { std::slice::from_raw_parts(input, input_len) };
        let output = link_frame_decode(bytes)?.into_boxed_slice();
        let len = output.len();
        let ptr = Box::into_raw(output) as *mut u8;
        Ok(MeshBuffer { ptr, len })
    });
    status(result, |buffer| unsafe { *out = buffer })
}

/// # Safety
/// `frame` references exactly one 91-byte canonical `RelayFrame`; `record`
/// references one bounded canonical durable record. `out` is writable and must
/// be released once with `mesh_buffer_release`.
#[no_mangle]
pub unsafe extern "C" fn mesh_routed_record_encode(
    frame: *const u8,
    frame_len: usize,
    record: *const u8,
    record_len: usize,
    out: *mut MeshBuffer,
) -> i32 {
    if out.is_null() || !(out as usize).is_multiple_of(std::mem::align_of::<MeshBuffer>()) {
        return Error::InvalidArgument as i32;
    }
    unsafe { *out = MeshBuffer::default() }
    if frame.is_null() || record.is_null() || frame_len != RELAY_FRAME_BYTES || record_len == 0 {
        return Error::InvalidArgument as i32;
    }
    let result = guarded(|| {
        let frame = unsafe { std::slice::from_raw_parts(frame, frame_len) };
        let record = unsafe { std::slice::from_raw_parts(record, record_len) };
        let output = routed_record_encode(frame, record)?.into_boxed_slice();
        let len = output.len();
        let ptr = Box::into_raw(output) as *mut u8;
        Ok(MeshBuffer { ptr, len })
    });
    status(result, |buffer| unsafe { *out = buffer })
}

/// # Safety
/// `input` references one bounded routed record. `out` is writable and must be
/// released once with `mesh_buffer_release`.
#[no_mangle]
pub unsafe extern "C" fn mesh_routed_record_decode(
    input: *const u8,
    input_len: usize,
    out: *mut MeshBuffer,
) -> i32 {
    if out.is_null() || !(out as usize).is_multiple_of(std::mem::align_of::<MeshBuffer>()) {
        return Error::InvalidArgument as i32;
    }
    unsafe { *out = MeshBuffer::default() }
    if input.is_null() || input_len == 0 || input_len > mesh_replication::MAX_ROUTED_RECORD {
        return Error::InvalidArgument as i32;
    }
    let result = guarded(|| {
        let input = unsafe { std::slice::from_raw_parts(input, input_len) };
        let output = routed_record_decode(input)?.into_boxed_slice();
        let len = output.len();
        let ptr = Box::into_raw(output) as *mut u8;
        Ok(MeshBuffer { ptr, len })
    });
    status(result, |buffer| unsafe { *out = buffer })
}

/// # Safety
/// `input` references one routed record obtained from an already authenticated
/// Noise neighbor. `out` is writable and receives a compact scheduling result.
#[no_mangle]
pub unsafe extern "C" fn mesh_secure_store_accept_routed(
    handle: u64,
    input: *const u8,
    input_len: usize,
    received_from: *const u8,
    received_from_len: usize,
    now: u64,
    out: *mut u8,
) -> i32 {
    if out.is_null() {
        return Error::InvalidArgument as i32;
    }
    unsafe { *out = 0 }
    if input.is_null()
        || input_len == 0
        || input_len > mesh_replication::MAX_ROUTED_RECORD
        || received_from.is_null()
        || received_from_len != 32
        || now == 0
    {
        return Error::InvalidArgument as i32;
    }
    status(
        guarded(|| {
            secure_store_accept_routed(
                handle,
                unsafe { std::slice::from_raw_parts(input, input_len) },
                unsafe { std::slice::from_raw_parts(received_from, received_from_len) },
                now,
            )
        }),
        |result| unsafe { *out = result },
    )
}
/// # Safety
/// `identity_seed` is a readable 32-byte protected host seed; `plaintext` is
/// a readable bounded text payload; `out` is writable.
#[no_mangle]
pub unsafe extern "C" fn mesh_secure_store_enqueue_text(
    handle: u64,
    identity_seed: *const u8,
    identity_seed_len: usize,
    plaintext: *const u8,
    plaintext_len: usize,
    now: u64,
    out: *mut u16,
) -> i32 {
    if out.is_null() {
        return Error::InvalidArgument as i32;
    }
    unsafe { *out = 0 }
    if identity_seed.is_null()
        || identity_seed_len != 32
        || plaintext.is_null()
        || plaintext_len == 0
        || plaintext_len > mesh_protocol::MAX_PLAINTEXT
        || now == 0
    {
        return Error::InvalidArgument as i32;
    }
    status(
        guarded(|| {
            secure_store_enqueue_text(
                handle,
                unsafe { std::slice::from_raw_parts(identity_seed, identity_seed_len) },
                unsafe { std::slice::from_raw_parts(plaintext, plaintext_len) },
                now,
            )
        }),
        |result| unsafe { *out = result },
    )
}
/// # Safety
/// Same contract as `mesh_secure_store_enqueue_text`; `logical_id` is a
/// readable 16-byte opaque action identifier created by the UI.
#[no_mangle]
pub unsafe extern "C" fn mesh_secure_store_enqueue_text_with_logical_id(
    handle: u64,
    identity_seed: *const u8,
    identity_seed_len: usize,
    plaintext: *const u8,
    plaintext_len: usize,
    logical_id: *const u8,
    logical_id_len: usize,
    now: u64,
    out: *mut u16,
) -> i32 {
    if out.is_null() {
        return Error::InvalidArgument as i32;
    }
    unsafe { *out = 0 }
    if identity_seed.is_null()
        || identity_seed_len != 32
        || plaintext.is_null()
        || plaintext_len == 0
        || plaintext_len > mesh_protocol::MAX_PLAINTEXT
        || logical_id.is_null()
        || logical_id_len != 16
        || now == 0
    {
        return Error::InvalidArgument as i32;
    }
    status(
        guarded(|| {
            let logical_id: [u8; 16] =
                unsafe { std::slice::from_raw_parts(logical_id, logical_id_len) }
                    .try_into()
                    .map_err(|_| Error::InvalidArgument)?;
            secure_store_enqueue_text_with_logical_id(
                handle,
                unsafe { std::slice::from_raw_parts(identity_seed, identity_seed_len) },
                unsafe { std::slice::from_raw_parts(plaintext, plaintext_len) },
                logical_id,
                now,
            )
        }),
        |result| unsafe { *out = result },
    )
}
/// # Safety
/// `out` receives either an empty buffer or the fixed 19-byte delivery
/// summary. The caller releases a nonempty buffer with `mesh_buffer_release`.
#[no_mangle]
pub unsafe extern "C" fn mesh_secure_store_latest_delivery_summary(
    handle: u64,
    now: u64,
    out: *mut MeshBuffer,
) -> i32 {
    if out.is_null() || !(out as usize).is_multiple_of(std::mem::align_of::<MeshBuffer>()) {
        return Error::InvalidArgument as i32;
    }
    unsafe { *out = MeshBuffer::default() }
    let result = secure_store_latest_delivery_summary(handle, now).map(|bytes| {
        let bytes = bytes.into_boxed_slice();
        let len = bytes.len();
        let ptr = Box::into_raw(bytes) as *mut u8;
        MeshBuffer { ptr, len }
    });
    status(result, |buffer| unsafe { *out = buffer })
}
/// # Safety
/// `logical_id` points to exactly 16 readable bytes. `out` receives either an
/// empty buffer or the fixed 19-byte delivery summary.
#[no_mangle]
pub unsafe extern "C" fn mesh_secure_store_delivery_summary(
    handle: u64,
    logical_id: *const u8,
    logical_id_len: usize,
    now: u64,
    out: *mut MeshBuffer,
) -> i32 {
    if logical_id.is_null()
        || out.is_null()
        || !(out as usize).is_multiple_of(std::mem::align_of::<MeshBuffer>())
    {
        return Error::InvalidArgument as i32;
    }
    unsafe { *out = MeshBuffer::default() }
    let result = guarded(|| {
        let logical_id: [u8; 16] =
            unsafe { std::slice::from_raw_parts(logical_id, logical_id_len) }
                .try_into()
                .map_err(|_| Error::InvalidArgument)?;
        secure_store_delivery_summary(handle, logical_id, now)
    })
    .map(|bytes| {
        let bytes = bytes.into_boxed_slice();
        let len = bytes.len();
        let ptr = Box::into_raw(bytes) as *mut u8;
        MeshBuffer { ptr, len }
    });
    status(result, |buffer| unsafe { *out = buffer })
}
/// # Safety
/// Both seeds reference readable 32-byte host-protected material. `out`
/// receives an empty buffer when no complete text awaits finalization; every
/// nonempty buffer is a native-only delivered-text completion packet and must
/// be released with `mesh_buffer_release`.
#[no_mangle]
pub unsafe extern "C" fn mesh_secure_store_finalize_next_text(
    handle: u64,
    identity_seed: *const u8,
    identity_seed_len: usize,
    delivery_seed: *const u8,
    delivery_seed_len: usize,
    now: u64,
    out: *mut MeshBuffer,
) -> i32 {
    if out.is_null() || !(out as usize).is_multiple_of(std::mem::align_of::<MeshBuffer>()) {
        return Error::InvalidArgument as i32;
    }
    unsafe { *out = MeshBuffer::default() }
    if identity_seed.is_null()
        || identity_seed_len != 32
        || delivery_seed.is_null()
        || delivery_seed_len != 32
        || now == 0
    {
        return Error::InvalidArgument as i32;
    }
    let result = guarded(|| {
        let bytes = secure_store_finalize_next_text(
            handle,
            unsafe { std::slice::from_raw_parts(identity_seed, identity_seed_len) },
            unsafe { std::slice::from_raw_parts(delivery_seed, delivery_seed_len) },
            now,
        )?;
        if bytes.is_empty() {
            return Ok(MeshBuffer::default());
        }
        let output = bytes.into_boxed_slice();
        let len = output.len();
        Ok(MeshBuffer {
            ptr: Box::into_raw(output) as *mut u8,
            len,
        })
    });
    status(result, |buffer| unsafe { *out = buffer })
}
/// # Safety
/// `out` is writable and receives one origin outbox record, or an empty buffer
/// when there is no remaining outbound durable record at this slot.
#[no_mangle]
pub unsafe extern "C" fn mesh_secure_store_outbox_record(
    handle: u64,
    slot: u16,
    now: u64,
    out: *mut MeshBuffer,
) -> i32 {
    if out.is_null() || !(out as usize).is_multiple_of(std::mem::align_of::<MeshBuffer>()) {
        return Error::InvalidArgument as i32;
    }
    unsafe { *out = MeshBuffer::default() }
    if now == 0 {
        return Error::InvalidArgument as i32;
    }
    let result = guarded(|| {
        let bytes = secure_store_outbox_record(handle, slot, now)?;
        if bytes.is_empty() {
            return Ok(MeshBuffer::default());
        }
        let output = bytes.into_boxed_slice();
        let len = output.len();
        Ok(MeshBuffer {
            ptr: Box::into_raw(output) as *mut u8,
            len,
        })
    });
    status(result, |buffer| unsafe { *out = buffer })
}
/// # Safety
/// `out` receives one signed local receipt wrapped for the durable relay, or
/// an empty buffer when no receipt remains within its validity window.
#[no_mangle]
pub unsafe extern "C" fn mesh_secure_store_receipt_record(
    handle: u64,
    slot: u16,
    now: u64,
    out: *mut MeshBuffer,
) -> i32 {
    if out.is_null() || !(out as usize).is_multiple_of(std::mem::align_of::<MeshBuffer>()) {
        return Error::InvalidArgument as i32;
    }
    unsafe { *out = MeshBuffer::default() }
    if now == 0 {
        return Error::InvalidArgument as i32;
    }
    let result = guarded(|| {
        let bytes = secure_store_receipt_record(handle, slot, now)?;
        if bytes.is_empty() {
            return Ok(MeshBuffer::default());
        }
        let output = bytes.into_boxed_slice();
        let len = output.len();
        Ok(MeshBuffer {
            ptr: Box::into_raw(output) as *mut u8,
            len,
        })
    });
    status(result, |buffer| unsafe { *out = buffer })
}
/// # Safety
/// `out` is writable and receives one persisted routed record, or an empty
/// buffer when the durable relay queue has no record at this slot.
/// # Safety
/// The seed is used only to sign a canonical acknowledgement for a receipt
/// already committed by this origin; `out` receives the routed ACK or empty.
#[no_mangle]
pub unsafe extern "C" fn mesh_secure_store_receipt_ack_record(
    handle: u64,
    identity_seed: *const u8,
    identity_seed_len: usize,
    slot: u16,
    now: u64,
    out: *mut MeshBuffer,
) -> i32 {
    if out.is_null() || !(out as usize).is_multiple_of(std::mem::align_of::<MeshBuffer>()) {
        return Error::InvalidArgument as i32;
    }
    unsafe { *out = MeshBuffer::default() }
    if identity_seed.is_null() || identity_seed_len != 32 || now == 0 {
        return Error::InvalidArgument as i32;
    }
    let result = guarded(|| {
        let bytes = secure_store_receipt_ack_record(
            handle,
            unsafe { std::slice::from_raw_parts(identity_seed, identity_seed_len) },
            slot,
            now,
        )?;
        if bytes.is_empty() {
            return Ok(MeshBuffer::default());
        }
        let output = bytes.into_boxed_slice();
        let len = output.len();
        Ok(MeshBuffer {
            ptr: Box::into_raw(output) as *mut u8,
            len,
        })
    });
    status(result, |buffer| unsafe { *out = buffer })
}
#[no_mangle]
pub unsafe extern "C" fn mesh_secure_store_relay_record(
    handle: u64,
    slot: u16,
    now: u64,
    out: *mut MeshBuffer,
) -> i32 {
    if out.is_null() || !(out as usize).is_multiple_of(std::mem::align_of::<MeshBuffer>()) {
        return Error::InvalidArgument as i32;
    }
    unsafe { *out = MeshBuffer::default() }
    if now == 0 {
        return Error::InvalidArgument as i32;
    }
    let result = guarded(|| {
        let bytes = secure_store_relay_record(handle, slot, now)?;
        if bytes.is_empty() {
            return Ok(MeshBuffer::default());
        }
        let output = bytes.into_boxed_slice();
        let len = output.len();
        Ok(MeshBuffer {
            ptr: Box::into_raw(output) as *mut u8,
            len,
        })
    });
    status(result, |buffer| unsafe { *out = buffer })
}
/// # Safety
/// `out` is writable and receives the authenticated ingress member of the
/// oldest custody object, or an empty buffer when the queue is empty.
#[no_mangle]
pub unsafe extern "C" fn mesh_secure_store_relay_received_from(
    handle: u64,
    now: u64,
    out: *mut MeshBuffer,
) -> i32 {
    if out.is_null() || !(out as usize).is_multiple_of(std::mem::align_of::<MeshBuffer>()) {
        return Error::InvalidArgument as i32;
    }
    unsafe { *out = MeshBuffer::default() }
    if now == 0 {
        return Error::InvalidArgument as i32;
    }
    let result = guarded(|| {
        let bytes = secure_store_relay_received_from(handle, now)?;
        if bytes.is_empty() {
            return Ok(MeshBuffer::default());
        }
        let output = bytes.into_boxed_slice();
        let len = output.len();
        Ok(MeshBuffer {
            ptr: Box::into_raw(output) as *mut u8,
            len,
        })
    });
    status(result, |buffer| unsafe { *out = buffer })
}
/// # Safety
/// `out` is writable and receives one persisted receipt relay record, or an
/// empty buffer when no receipt custody is eligible at this slot.
#[no_mangle]
pub unsafe extern "C" fn mesh_secure_store_relay_receipt_record(
    handle: u64,
    slot: u16,
    now: u64,
    out: *mut MeshBuffer,
) -> i32 {
    if out.is_null() || !(out as usize).is_multiple_of(std::mem::align_of::<MeshBuffer>()) {
        return Error::InvalidArgument as i32;
    }
    unsafe { *out = MeshBuffer::default() }
    if now == 0 {
        return Error::InvalidArgument as i32;
    }
    let result = guarded(|| {
        let bytes = secure_store_relay_receipt_record(handle, slot, now)?;
        if bytes.is_empty() {
            return Ok(MeshBuffer::default());
        }
        let output = bytes.into_boxed_slice();
        let len = output.len();
        Ok(MeshBuffer {
            ptr: Box::into_raw(output) as *mut u8,
            len,
        })
    });
    status(result, |buffer| unsafe { *out = buffer })
}
/// # Safety
/// `out` is writable and receives the ingress neighbor for the oldest receipt
/// custody record, or an empty buffer if that queue is empty.
#[no_mangle]
pub unsafe extern "C" fn mesh_secure_store_relay_receipt_received_from(
    handle: u64,
    now: u64,
    out: *mut MeshBuffer,
) -> i32 {
    if out.is_null() || !(out as usize).is_multiple_of(std::mem::align_of::<MeshBuffer>()) {
        return Error::InvalidArgument as i32;
    }
    unsafe { *out = MeshBuffer::default() }
    if now == 0 {
        return Error::InvalidArgument as i32;
    }
    let result = guarded(|| {
        let bytes = secure_store_relay_receipt_received_from(handle, now)?;
        if bytes.is_empty() {
            return Ok(MeshBuffer::default());
        }
        let output = bytes.into_boxed_slice();
        let len = output.len();
        Ok(MeshBuffer {
            ptr: Box::into_raw(output) as *mut u8,
            len,
        })
    });
    status(result, |buffer| unsafe { *out = buffer })
}
/// # Safety
/// `out` is writable and receives one restart-safe relay ACK record, or empty.
#[no_mangle]
pub unsafe extern "C" fn mesh_secure_store_relay_receipt_ack_record(
    handle: u64,
    slot: u16,
    now: u64,
    out: *mut MeshBuffer,
) -> i32 {
    if out.is_null() || !(out as usize).is_multiple_of(std::mem::align_of::<MeshBuffer>()) {
        return Error::InvalidArgument as i32;
    }
    unsafe { *out = MeshBuffer::default() }
    if now == 0 {
        return Error::InvalidArgument as i32;
    }
    let result = guarded(|| {
        let bytes = secure_store_relay_receipt_ack_record(handle, slot, now)?;
        if bytes.is_empty() {
            return Ok(MeshBuffer::default());
        }
        let output = bytes.into_boxed_slice();
        let len = output.len();
        Ok(MeshBuffer {
            ptr: Box::into_raw(output) as *mut u8,
            len,
        })
    });
    status(result, |buffer| unsafe { *out = buffer })
}
/// # Safety
/// `out` is writable and receives the ingress member for the oldest relay ACK.
#[no_mangle]
pub unsafe extern "C" fn mesh_secure_store_relay_receipt_ack_received_from(
    handle: u64,
    now: u64,
    out: *mut MeshBuffer,
) -> i32 {
    if out.is_null() || !(out as usize).is_multiple_of(std::mem::align_of::<MeshBuffer>()) {
        return Error::InvalidArgument as i32;
    }
    unsafe { *out = MeshBuffer::default() }
    if now == 0 {
        return Error::InvalidArgument as i32;
    }
    let result = guarded(|| {
        let bytes = secure_store_relay_receipt_ack_received_from(handle, now)?;
        if bytes.is_empty() {
            return Ok(MeshBuffer::default());
        }
        let output = bytes.into_boxed_slice();
        let len = output.len();
        Ok(MeshBuffer {
            ptr: Box::into_raw(output) as *mut u8,
            len,
        })
    });
    status(result, |buffer| unsafe { *out = buffer })
}
#[no_mangle]
pub extern "C" fn mesh_runtime_release(handle: u64) -> i32 {
    status(release(handle), |_| {})
}

/// # Safety
/// Key/member/path point to readable bounded byte arrays for this call. The key
/// is copied into a zeroizing Rust buffer and is never returned.
#[no_mangle]
pub unsafe extern "C" fn mesh_secure_store_probe(
    key: *const u8,
    key_len: usize,
    member: *const u8,
    member_len: usize,
    path: *const u8,
    path_len: usize,
) -> i32 {
    if key.is_null() || member.is_null() || path.is_null() || key_len != 32 || member_len != 32 {
        return Error::InvalidArgument as i32;
    }
    status(
        guarded(|| {
            if !(1..=1024).contains(&path_len) {
                return Err(Error::InvalidArgument);
            }
            let key = unsafe { std::slice::from_raw_parts(key, key_len) };
            let member = unsafe { std::slice::from_raw_parts(member, member_len) };
            let path = std::str::from_utf8(unsafe { std::slice::from_raw_parts(path, path_len) })
                .map_err(|_| Error::InvalidArgument)?;
            secure_store_probe(key, member, path)
        }),
        |_| {},
    )
}
/// # Safety
/// Key/member/path point to readable bounded byte arrays. `out` is a writable,
/// aligned u64. The returned handle is native-host-only and must be released once.
#[no_mangle]
pub unsafe extern "C" fn mesh_secure_store_open(
    key: *const u8,
    key_len: usize,
    member: *const u8,
    member_len: usize,
    path: *const u8,
    path_len: usize,
    out: *mut u64,
) -> i32 {
    if out.is_null() || !(out as usize).is_multiple_of(std::mem::align_of::<u64>()) {
        return Error::InvalidArgument as i32;
    }
    unsafe {
        *out = 0;
    }
    if key.is_null()
        || member.is_null()
        || path.is_null()
        || key_len != 32
        || member_len != 32
        || !(1..=1024).contains(&path_len)
    {
        return Error::InvalidArgument as i32;
    }
    status(
        guarded(|| {
            let key = unsafe { std::slice::from_raw_parts(key, key_len) };
            let member = unsafe { std::slice::from_raw_parts(member, member_len) };
            let path = std::str::from_utf8(unsafe { std::slice::from_raw_parts(path, path_len) })
                .map_err(|_| Error::InvalidArgument)?;
            secure_store_open(key, member, path)
        }),
        |handle| unsafe { *out = handle },
    )
}
#[no_mangle]
pub extern "C" fn mesh_secure_store_release(handle: u64) -> i32 {
    status(secure_store_release(handle), |_| {})
}
/// # Safety
/// `out` must be a writable aligned u64. Zero denotes no configured group.
#[no_mangle]
pub unsafe extern "C" fn mesh_secure_store_policy_epoch(
    handle: u64,
    now: u64,
    out: *mut u64,
) -> i32 {
    if out.is_null() || !(out as usize).is_multiple_of(std::mem::align_of::<u64>()) {
        return Error::InvalidArgument as i32;
    }
    unsafe {
        *out = 0;
    }
    status(secure_store_policy_epoch(handle, now), |epoch| unsafe {
        *out = epoch
    })
}

/// # Safety
/// `out` is an aligned writable MeshBuffer. The returned 16-byte public
/// discovery tag must be released once with `mesh_buffer_release`.
#[no_mangle]
pub unsafe extern "C" fn mesh_secure_store_discovery_tag(
    handle: u64,
    now: u64,
    out: *mut MeshBuffer,
) -> i32 {
    if out.is_null() || !(out as usize).is_multiple_of(std::mem::align_of::<MeshBuffer>()) {
        return Error::InvalidArgument as i32;
    }
    unsafe { *out = MeshBuffer::default() }
    let result = secure_store_discovery_tag(handle, now).map(|tag| {
        let output = tag.to_vec().into_boxed_slice();
        let len = output.len();
        let ptr = Box::into_raw(output) as *mut u8;
        MeshBuffer { ptr, len }
    });
    status(result, |buffer| unsafe { *out = buffer })
}
/// # Safety
/// Protected session seed and member point to exactly 32 readable bytes. `out`
/// receives an opaque native-only session handle and must be aligned/writable.
#[no_mangle]
pub unsafe extern "C" fn mesh_secure_session_start(
    store_handle: u64,
    session_seed: *const u8,
    session_seed_len: usize,
    member: *const u8,
    member_len: usize,
    role: u8,
    now: u64,
    out: *mut u64,
) -> i32 {
    if out.is_null() || !(out as usize).is_multiple_of(std::mem::align_of::<u64>()) {
        return Error::InvalidArgument as i32;
    }
    unsafe { *out = 0 }
    if session_seed.is_null()
        || member.is_null()
        || session_seed_len != 32
        || member_len != 32
        || now == 0
    {
        return Error::InvalidArgument as i32;
    }
    status(
        guarded(|| {
            let seed = unsafe { std::slice::from_raw_parts(session_seed, session_seed_len) };
            let member = unsafe { std::slice::from_raw_parts(member, member_len) };
            secure_session_start(store_handle, seed, member, role, now)
        }),
        |handle| unsafe { *out = handle },
    )
}
/// # Safety
/// `out` is an aligned writable MeshBuffer and receives one bounded Noise
/// handshake record. Release it once with mesh_buffer_release.
#[no_mangle]
pub unsafe extern "C" fn mesh_secure_session_write(
    handle: u64,
    now: u64,
    out: *mut MeshBuffer,
) -> i32 {
    if out.is_null() || !(out as usize).is_multiple_of(std::mem::align_of::<MeshBuffer>()) {
        return Error::InvalidArgument as i32;
    }
    unsafe { *out = MeshBuffer::default() }
    let result = guarded(|| {
        let output = secure_session_write(handle, now)?.into_boxed_slice();
        let len = output.len();
        let ptr = Box::into_raw(output) as *mut u8;
        Ok(MeshBuffer { ptr, len })
    });
    status(result, |buffer| unsafe { *out = buffer })
}
/// # Safety
/// `input` is one bounded Noise handshake record. It is borrowed only during
/// this call and is never retained by the session.
#[no_mangle]
pub unsafe extern "C" fn mesh_secure_session_read(
    handle: u64,
    input: *const u8,
    input_len: usize,
    now: u64,
) -> i32 {
    if input.is_null() || input_len == 0 || input_len > 96 || now == 0 {
        return Error::InvalidArgument as i32;
    }
    status(
        guarded(|| {
            secure_session_read(
                handle,
                unsafe { std::slice::from_raw_parts(input, input_len) },
                now,
            )
        }),
        |_| {},
    )
}
/// # Safety
/// Completes a successful Noise XX handshake. No pointer parameters are used.
#[no_mangle]
pub extern "C" fn mesh_secure_session_finish(handle: u64, now: u64) -> i32 {
    status(secure_session_finish(handle, now), |_| {})
}
/// # Safety
/// `identity_seed` points to 32 readable protected bytes. `out` is a writable
/// aligned MeshBuffer that receives a protected authentication proof.
#[no_mangle]
pub unsafe extern "C" fn mesh_secure_session_authenticate(
    handle: u64,
    identity_seed: *const u8,
    identity_seed_len: usize,
    now: u64,
    out: *mut MeshBuffer,
) -> i32 {
    if out.is_null() || !(out as usize).is_multiple_of(std::mem::align_of::<MeshBuffer>()) {
        return Error::InvalidArgument as i32;
    }
    unsafe { *out = MeshBuffer::default() }
    if identity_seed.is_null() || identity_seed_len != 32 || now == 0 {
        return Error::InvalidArgument as i32;
    }
    let result = guarded(|| {
        let seed = unsafe { std::slice::from_raw_parts(identity_seed, identity_seed_len) };
        let output = secure_session_authenticate(handle, seed, now)?.into_boxed_slice();
        let len = output.len();
        let ptr = Box::into_raw(output) as *mut u8;
        Ok(MeshBuffer { ptr, len })
    });
    status(result, |buffer| unsafe { *out = buffer })
}
/// # Safety
/// `input` contains bounded authenticated application bytes. `out` receives a
/// protected session frame and must be released once.
#[no_mangle]
pub unsafe extern "C" fn mesh_secure_session_send(
    handle: u64,
    input: *const u8,
    input_len: usize,
    now: u64,
    out: *mut MeshBuffer,
) -> i32 {
    if out.is_null() || !(out as usize).is_multiple_of(std::mem::align_of::<MeshBuffer>()) {
        return Error::InvalidArgument as i32;
    }
    unsafe { *out = MeshBuffer::default() }
    if input.is_null() || input_len == 0 || input_len > mesh_session::MAX_PAYLOAD || now == 0 {
        return Error::InvalidArgument as i32;
    }
    let result = guarded(|| {
        let input = unsafe { std::slice::from_raw_parts(input, input_len) };
        let output = secure_session_send(handle, input, now)?.into_boxed_slice();
        let len = output.len();
        let ptr = Box::into_raw(output) as *mut u8;
        Ok(MeshBuffer { ptr, len })
    });
    status(result, |buffer| unsafe { *out = buffer })
}
/// # Safety
/// `input` contains one bounded protected session frame. A successful empty
/// output denotes peer authentication; non-empty output is authenticated data.
#[no_mangle]
pub unsafe extern "C" fn mesh_secure_session_receive(
    handle: u64,
    input: *const u8,
    input_len: usize,
    now: u64,
    out: *mut MeshBuffer,
) -> i32 {
    if out.is_null() || !(out as usize).is_multiple_of(std::mem::align_of::<MeshBuffer>()) {
        return Error::InvalidArgument as i32;
    }
    unsafe { *out = MeshBuffer::default() }
    if input.is_null() || !(58..=mesh_session::MAX_FRAME).contains(&input_len) || now == 0 {
        return Error::InvalidArgument as i32;
    }
    let result = guarded(|| {
        let input = unsafe { std::slice::from_raw_parts(input, input_len) };
        let output = secure_session_receive(handle, input, now)?;
        if output.is_empty() {
            return Ok(MeshBuffer::default());
        }
        let output = output.into_boxed_slice();
        let len = output.len();
        let ptr = Box::into_raw(output) as *mut u8;
        Ok(MeshBuffer { ptr, len })
    });
    status(result, |buffer| unsafe { *out = buffer })
}
/// # Safety
/// `out` is aligned/writable; it receives one only after the remote signed proof
/// and this device's proof have both been accepted.
#[no_mangle]
pub unsafe extern "C" fn mesh_secure_session_authenticated(handle: u64, out: *mut u8) -> i32 {
    if out.is_null() {
        return Error::InvalidArgument as i32;
    }
    unsafe { *out = 0 }
    status(
        secure_session_authenticated(handle),
        |authenticated| unsafe { *out = u8::from(authenticated) },
    )
}
/// # Safety
/// `out` points to 32 writable bytes and receives the public certified member
/// identity only after the remote Noise proof has been accepted.
#[no_mangle]
pub unsafe extern "C" fn mesh_secure_session_peer(handle: u64, out: *mut u8) -> i32 {
    if out.is_null() {
        return Error::InvalidArgument as i32;
    }
    unsafe { std::ptr::write_bytes(out, 0, 32) }
    status(secure_session_peer(handle), |member| unsafe {
        std::ptr::copy_nonoverlapping(member.as_ptr(), out, 32)
    })
}
#[no_mangle]
pub extern "C" fn mesh_secure_session_release(handle: u64) -> i32 {
    status(secure_session_release(handle), |_| {})
}
/// # Safety
/// Inputs reference exactly 32 readable bytes. Private seeds are copied into
/// zeroizing Rust buffers. `out` must be an aligned writable u64.
#[no_mangle]
pub unsafe extern "C" fn mesh_secure_store_create_group(
    handle: u64,
    identity_seed: *const u8,
    identity_seed_len: usize,
    delivery_seed: *const u8,
    delivery_seed_len: usize,
    member: *const u8,
    member_len: usize,
    now: u64,
    out: *mut u64,
) -> i32 {
    if out.is_null() || !(out as usize).is_multiple_of(std::mem::align_of::<u64>()) {
        return Error::InvalidArgument as i32;
    }
    unsafe {
        *out = 0;
    }
    if identity_seed.is_null()
        || delivery_seed.is_null()
        || member.is_null()
        || identity_seed_len != 32
        || delivery_seed_len != 32
        || member_len != 32
    {
        return Error::InvalidArgument as i32;
    }
    status(
        guarded(|| {
            let identity_seed =
                unsafe { std::slice::from_raw_parts(identity_seed, identity_seed_len) };
            let delivery_seed =
                unsafe { std::slice::from_raw_parts(delivery_seed, delivery_seed_len) };
            let member = unsafe { std::slice::from_raw_parts(member, member_len) };
            secure_store_create_group(handle, identity_seed, delivery_seed, member, now)
        }),
        |epoch| unsafe { *out = epoch },
    )
}
/// # Safety
/// Inputs reference bounded readable data. The identity seed is copied to a
/// zeroizing buffer. `out` is an aligned writable MeshBuffer released normally.
#[no_mangle]
pub unsafe extern "C" fn mesh_secure_store_issue_enrollment(
    handle: u64,
    identity_seed: *const u8,
    identity_seed_len: usize,
    request: *const u8,
    request_len: usize,
    now: u64,
    out: *mut MeshBuffer,
) -> i32 {
    if out.is_null() || !(out as usize).is_multiple_of(std::mem::align_of::<MeshBuffer>()) {
        return Error::InvalidArgument as i32;
    }
    unsafe {
        *out = MeshBuffer::default();
    }
    if identity_seed.is_null()
        || request.is_null()
        || identity_seed_len != 32
        || request_len == 0
        || request_len > 512
    {
        return Error::InvalidArgument as i32;
    }
    let result = guarded(|| {
        let seed = unsafe { std::slice::from_raw_parts(identity_seed, identity_seed_len) };
        let request = unsafe { std::slice::from_raw_parts(request, request_len) };
        let output = secure_store_issue_enrollment(handle, seed, request, now)?.into_boxed_slice();
        let len = output.len();
        let ptr = Box::into_raw(output) as *mut u8;
        Ok(MeshBuffer { ptr, len })
    });
    status(result, |buffer| unsafe { *out = buffer })
}
/// # Safety
/// The request references bounded public bytes. `out` points to 32 writable
/// bytes and receives a verified public member identity only.
#[no_mangle]
pub unsafe extern "C" fn mesh_enrollment_request_member(
    request: *const u8,
    request_len: usize,
    now: u64,
    out: *mut u8,
) -> i32 {
    if request.is_null() || out.is_null() || request_len == 0 || request_len > 512 {
        return Error::InvalidArgument as i32;
    }
    unsafe { std::ptr::write_bytes(out, 0, 32) }
    status(
        guarded(|| {
            let request = unsafe { std::slice::from_raw_parts(request, request_len) };
            enrollment_request_member(request, now)
        }),
        |member| unsafe { std::ptr::copy_nonoverlapping(member.0.as_ptr(), out, 32) },
    )
}
/// # Safety
/// Bundle references bounded public readable bytes. `out` is an aligned writable
/// u64 and receives the installed epoch only after SQLCipher commits it.
#[no_mangle]
pub unsafe extern "C" fn mesh_secure_store_install_policy(
    handle: u64,
    bundle: *const u8,
    bundle_len: usize,
    now: u64,
    out: *mut u64,
) -> i32 {
    if out.is_null() || !(out as usize).is_multiple_of(std::mem::align_of::<u64>()) {
        return Error::InvalidArgument as i32;
    }
    unsafe {
        *out = 0;
    }
    if bundle.is_null() || bundle_len == 0 || bundle_len > mesh_protocol::MAX_POLICY_BUNDLE {
        return Error::InvalidArgument as i32;
    }
    status(
        guarded(|| {
            let bundle = unsafe { std::slice::from_raw_parts(bundle, bundle_len) };
            secure_store_install_policy(handle, bundle, now)
        }),
        |epoch| unsafe { *out = epoch },
    )
}
/// # Safety
/// `out` is an aligned writable MeshBuffer. The exported result is public policy
/// transport only and must be released with mesh_buffer_release exactly once.
#[no_mangle]
pub unsafe extern "C" fn mesh_secure_store_export_policy(
    handle: u64,
    now: u64,
    out: *mut MeshBuffer,
) -> i32 {
    if out.is_null() || !(out as usize).is_multiple_of(std::mem::align_of::<MeshBuffer>()) {
        return Error::InvalidArgument as i32;
    }
    unsafe {
        *out = MeshBuffer::default();
    }
    let result = guarded(|| {
        let output = secure_store_export_policy(handle, now)?.into_boxed_slice();
        let len = output.len();
        let ptr = Box::into_raw(output) as *mut u8;
        Ok(MeshBuffer { ptr, len })
    });
    status(result, |buffer| unsafe { *out = buffer })
}
/// # Safety
/// Seeds and invitation reference bounded readable data. Seeds are copied into
/// zeroizing Rust buffers; `out` receives public request bytes only.
#[no_mangle]
pub unsafe extern "C" fn mesh_create_enrollment_request(
    identity_seed: *const u8,
    identity_seed_len: usize,
    delivery_seed: *const u8,
    delivery_seed_len: usize,
    invitation: *const u8,
    invitation_len: usize,
    now: u64,
    out: *mut MeshBuffer,
) -> i32 {
    if out.is_null() || !(out as usize).is_multiple_of(std::mem::align_of::<MeshBuffer>()) {
        return Error::InvalidArgument as i32;
    }
    unsafe {
        *out = MeshBuffer::default();
    }
    if identity_seed.is_null()
        || delivery_seed.is_null()
        || invitation.is_null()
        || identity_seed_len != 32
        || delivery_seed_len != 32
        || invitation_len == 0
        || invitation_len > mesh_protocol::MAX_POLICY_BUNDLE
    {
        return Error::InvalidArgument as i32;
    }
    let result = guarded(|| {
        let identity_seed = unsafe { std::slice::from_raw_parts(identity_seed, identity_seed_len) };
        let delivery_seed = unsafe { std::slice::from_raw_parts(delivery_seed, delivery_seed_len) };
        let invitation = unsafe { std::slice::from_raw_parts(invitation, invitation_len) };
        let output =
            create_enrollment_request_from_policy(identity_seed, delivery_seed, invitation, now)?
                .into_boxed_slice();
        let len = output.len();
        let ptr = Box::into_raw(output) as *mut u8;
        Ok(MeshBuffer { ptr, len })
    });
    status(result, |buffer| unsafe { *out = buffer })
}

/// Native key-port boundary. Returns public Ed25519 bytes only; never a private key.
pub fn identity_public(seed: &[u8]) -> Result<Vec<u8>, Error> {
    guarded(|| {
        let mut secret = zeroize::Zeroizing::new([0; 32]);
        if seed.len() != 32 {
            return Err(Error::InvalidArgument);
        }
        secret.copy_from_slice(seed);
        Ok(mesh_crypto::IdentitySigningKey::import(secret)
            .public_key()
            .to_vec())
    })
}
/// # Safety
/// Seed must point to 32 readable bytes, out to an aligned writable MeshBuffer.
/// Host wipes its seed after return. Returned buffer contains only public bytes.
#[no_mangle]
pub unsafe extern "C" fn mesh_identity_public(
    seed: *const u8,
    len: usize,
    out: *mut MeshBuffer,
) -> i32 {
    if out.is_null() || !(out as usize).is_multiple_of(std::mem::align_of::<MeshBuffer>()) {
        return Error::InvalidArgument as i32;
    }
    unsafe {
        *out = MeshBuffer::default();
    }
    if seed.is_null() || len != 32 {
        return Error::InvalidArgument as i32;
    }
    let result = guarded(|| {
        let bytes =
            identity_public(unsafe { std::slice::from_raw_parts(seed, len) })?.into_boxed_slice();
        let len = bytes.len();
        let ptr = Box::into_raw(bytes) as *mut u8;
        Ok(MeshBuffer { ptr, len })
    });
    status(result, |buffer| unsafe {
        *out = buffer;
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nat_lifecycle_stale_handles_and_buffers() {
        assert_eq!(create(2), Err(Error::IncompatibleVersion));
        for _ in 0..10_000 {
            let mut handle = 0;
            unsafe {
                assert_eq!(mesh_runtime_create(1, &mut handle), 0);
                let mut buffer = MeshBuffer::default();
                let input = [0x83, 1, 1, 0];
                assert_eq!(
                    mesh_runtime_request(handle, input.as_ptr(), input.len(), &mut buffer),
                    0
                );
                assert!(!buffer.ptr.is_null());
                mesh_buffer_release(buffer);
            }
            assert_eq!(mesh_runtime_release(handle), 0);
            assert_eq!(mesh_runtime_release(handle), 0);
            assert_eq!(request(handle, &[0x83, 1, 0, 0]), Err(Error::InvalidHandle));
        }
    }
    #[test]
    fn nat_null_and_oversize_rejected_before_read() {
        unsafe {
            assert_eq!(mesh_runtime_create(1, std::ptr::null_mut()), 1);
            let mut buffer = MeshBuffer::default();
            assert_eq!(mesh_runtime_request(1, std::ptr::null(), 0, &mut buffer), 1);
            let one = 0u8;
            assert_eq!(mesh_runtime_request(1, &one, MAX_INPUT + 1, &mut buffer), 1);
            assert!(buffer.ptr.is_null());
        }
    }
    #[test]
    fn link_framing_is_opaque_exact_and_releases_each_buffer() {
        unsafe {
            let raw = [9, 8, 7];
            let mut framed = MeshBuffer::default();
            assert_eq!(
                mesh_link_frame_encode(raw.as_ptr(), raw.len(), &mut framed),
                0
            );
            assert_eq!(
                std::slice::from_raw_parts(framed.ptr, framed.len),
                [1, 0, 3, 9, 8, 7]
            );
            let mut decoded = MeshBuffer::default();
            assert_eq!(
                mesh_link_frame_decode(framed.ptr, framed.len, &mut decoded),
                0
            );
            assert_eq!(std::slice::from_raw_parts(decoded.ptr, decoded.len), raw);
            mesh_buffer_release(framed);
            mesh_buffer_release(decoded);
            let mut invalid = MeshBuffer::default();
            assert_eq!(
                mesh_link_frame_decode([1, 0, 2, 8].as_ptr(), 4, &mut invalid),
                1
            );
            assert!(invalid.ptr.is_null());
        }
    }
    #[test]
    fn routed_record_ffi_binds_route_and_payload_inside_one_noise_budget() {
        let frame = RelayFrame {
            id: mesh_replication::RelayId([5; 16]),
            origin: MemberId([1; 32]),
            previous_hop: MemberId([2; 32]),
            hops: 1,
            hop_limit: 4,
            expires_at: 100,
        }
        .encode();
        let record = DurableRecord::Announcement(vec![7; mesh_protocol::MAX_ANNOUNCEMENT])
            .encode()
            .unwrap();
        let encoded = routed_record_encode(&frame, &record).unwrap();
        assert_eq!(encoded.len(), mesh_session::MAX_PAYLOAD);
        let unpacked = routed_record_decode(&encoded).unwrap();
        assert_eq!(&unpacked[..RELAY_FRAME_BYTES], &frame);
        assert_eq!(&unpacked[RELAY_FRAME_BYTES..], &record);
        assert_eq!(
            routed_record_encode(&frame[..90], &record),
            Err(Error::InvalidArgument)
        );
        assert_eq!(
            routed_record_decode(&encoded[..encoded.len() - 1]),
            Err(Error::InvalidArgument)
        );
    }
    #[test]
    fn relay_gate_forwards_once_and_never_reuses_a_closed_handle() {
        let gate = relay_gate_open(&[2; 32]).unwrap();
        let frame = RelayFrame {
            id: mesh_replication::RelayId([8; 16]),
            origin: MemberId([1; 32]),
            previous_hop: MemberId([1; 32]),
            hops: 0,
            hop_limit: 4,
            expires_at: 100,
        }
        .encode();
        let first = relay_gate_accept(gate, &frame, &[1; 32], 10).unwrap();
        assert_eq!(first[0], 1);
        assert_eq!(first.len(), 1 + RELAY_FRAME_BYTES);
        assert_eq!(
            RelayFrame::decode(&first[1..]).unwrap().previous_hop,
            MemberId([2; 32])
        );
        assert_eq!(relay_gate_accept(gate, &frame, &[1; 32], 11), Ok(vec![2]));
        relay_gate_release(gate).unwrap();
        assert_eq!(
            relay_gate_accept(gate, &frame, &[1; 32], 11),
            Err(Error::InvalidHandle)
        );
    }
    #[test]
    fn native_identity_returns_only_public_bytes_and_checks_input_before_read() {
        let expected = [
            0x3b, 0x6a, 0x27, 0xbc, 0xce, 0xb6, 0xa4, 0x2d, 0x62, 0xa3, 0xa8, 0xd0, 0x2a, 0x6f,
            0x0d, 0x73, 0x65, 0x32, 0x15, 0x77, 0x1d, 0xe2, 0x43, 0xa6, 0x3a, 0xc0, 0x48, 0xa1,
            0x8b, 0x59, 0xda, 0x29,
        ];
        assert_eq!(identity_public(&[0; 32]).unwrap(), expected);
        assert_eq!(identity_public(&[0; 31]), Err(Error::InvalidArgument));
        unsafe {
            let mut out = MeshBuffer::default();
            assert_eq!(mesh_identity_public(std::ptr::null(), 32, &mut out), 1);
            let one = 0u8;
            assert_eq!(mesh_identity_public(&one, 33, &mut out), 1);
            assert!(out.ptr.is_null());
            assert_eq!(mesh_identity_public([0u8; 32].as_ptr(), 32, &mut out), 0);
            assert_eq!(std::slice::from_raw_parts(out.ptr, out.len), expected);
            mesh_buffer_release(out);
        }
    }
    #[test]
    fn panic_is_contained() {
        assert_eq!(
            guarded::<()>(|| panic!("injected")),
            Err(Error::InternalInvariant)
        );
    }
    #[test]
    fn secure_store_uses_a_bounded_keyed_sqlcipher_file() {
        let path = std::env::temp_dir().join(format!("mesh-ffi-store-{}", std::process::id()));
        let value = path.to_str().unwrap();
        assert_eq!(secure_store_probe(&[9; 32], &[1; 32], value), Ok(()));
        assert_eq!(
            secure_store_probe(&[9; 31], &[1; 32], value),
            Err(Error::InvalidArgument)
        );
        assert_eq!(
            secure_store_probe(&[9; 32], &[1; 31], value),
            Err(Error::InvalidArgument)
        );
        let _ = std::fs::remove_file(path);
    }
    #[test]
    fn secure_store_handle_is_opaque_and_release_is_idempotent() {
        let path = std::env::temp_dir().join(format!(
            "mesh-ffi-store-handle-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let value = path.to_str().unwrap();
        let handle = secure_store_open(&[7; 32], &[2; 32], value).unwrap();
        assert_ne!(handle, 0);
        assert_eq!(secure_store_policy_epoch(handle, 1), Ok(0));
        assert_eq!(secure_store_release(handle), Ok(()));
        assert_eq!(secure_store_release(handle), Ok(()));
        let _ = std::fs::remove_file(path);
    }
    #[test]
    fn group_creation_persists_a_self_certified_policy_once() {
        let path = std::env::temp_dir().join(format!(
            "mesh-ffi-group-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let value = path.to_str().unwrap();
        let member = identity_public(&[8; 32]).unwrap();
        let handle = secure_store_open(&[7; 32], &member, value).unwrap();
        assert_eq!(
            secure_store_create_group(handle, &[8; 32], &[9; 32], &member, 100),
            Ok(1)
        );
        assert_eq!(secure_store_policy_epoch(handle, 101), Ok(1));
        assert_eq!(
            secure_store_create_group(handle, &[8; 32], &[9; 32], &member, 101),
            Err(Error::StaleRequest)
        );
        assert_eq!(secure_store_release(handle), Ok(()));
        let _ = std::fs::remove_file(path);
    }
    #[test]
    fn enrollment_advances_the_roster_and_only_the_requested_member_can_install_it() {
        let root = std::env::temp_dir().join(format!(
            "mesh-ffi-enrollment-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let owner_path = root.with_extension("owner.db");
        let joiner_path = root.with_extension("joiner.db");
        let owner_member = identity_public(&[8; 32]).unwrap();
        let owner =
            secure_store_open(&[7; 32], &owner_member, owner_path.to_str().unwrap()).unwrap();
        assert_eq!(
            secure_store_create_group(owner, &[8; 32], &[9; 32], &owner_member, 100),
            Ok(1)
        );
        let joiner_member = identity_public(&[10; 32]).unwrap();
        let invitation = secure_store_export_policy(owner, 101).unwrap();
        let request =
            create_enrollment_request_from_policy(&[10; 32], &[11; 32], &invitation, 101).unwrap();
        assert_eq!(
            enrollment_request_member(&request, 101),
            Ok(MemberId(joiner_member.as_slice().try_into().unwrap()))
        );
        let mut exposed_member = [0u8; 32];
        unsafe {
            assert_eq!(
                mesh_enrollment_request_member(
                    request.as_ptr(),
                    request.len(),
                    101,
                    exposed_member.as_mut_ptr(),
                ),
                0
            );
        }
        assert_eq!(exposed_member.as_slice(), joiner_member.as_slice());
        let mut rejected_member = [0x55u8; 32];
        unsafe {
            assert_eq!(
                mesh_enrollment_request_member(
                    request.as_ptr(),
                    request.len(),
                    0,
                    rejected_member.as_mut_ptr(),
                ),
                Error::InvalidArgument as i32,
            );
        }
        // A caller must never accidentally reuse a previous identity after a
        // rejected request. The FFI clears its output before validation.
        assert_eq!(rejected_member, [0; 32]);
        assert_eq!(
            enrollment_request_member(&vec![0; 513], 101),
            Err(Error::InvalidArgument)
        );
        let mut tampered_request = request.clone();
        *tampered_request.last_mut().unwrap() ^= 0x01;
        assert_eq!(
            enrollment_request_member(&tampered_request, 101),
            Err(Error::InvalidArgument)
        );
        let bundle = secure_store_issue_enrollment(owner, &[8; 32], &request, 101).unwrap();
        assert_eq!(secure_store_policy_epoch(owner, 101), Ok(2));
        let joiner =
            secure_store_open(&[6; 32], &joiner_member, joiner_path.to_str().unwrap()).unwrap();
        assert_eq!(secure_store_install_policy(joiner, &bundle, 101), Ok(2));
        assert_eq!(secure_store_policy_epoch(joiner, 101), Ok(2));
        let owner_tag = secure_store_discovery_tag(owner, 101).unwrap();
        let joiner_tag = secure_store_discovery_tag(joiner, 101).unwrap();
        assert_eq!(owner_tag, joiner_tag);
        assert_ne!(owner_tag, [0; 16]);
        let owner_neighbors = secure_store_aware_neighbors(owner, 101).unwrap();
        let joiner_neighbors = secure_store_aware_neighbors(joiner, 101).unwrap();
        assert_eq!(owner_neighbors.len(), 8);
        assert_eq!(joiner_neighbors.len(), 8);
        let owner_short: [u8; 32] = Sha256::digest(owner_member).into();
        let joiner_short: [u8; 32] = Sha256::digest(joiner_member).into();
        assert_eq!(owner_neighbors, joiner_short[..8]);
        assert_eq!(joiner_neighbors, owner_short[..8]);
        assert_eq!(secure_store_release(owner), Ok(()));
        assert_eq!(secure_store_release(joiner), Ok(()));
        let _ = std::fs::remove_file(owner_path);
        let _ = std::fs::remove_file(joiner_path);
    }
    #[test]
    fn authority_can_issue_a_bounded_fifty_member_field_roster_and_queue_text_for_every_audience() {
        let path = std::env::temp_dir().join(format!(
            "mesh-ffi-field-roster-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let owner_member = identity_public(&[40; 32]).unwrap();
        let owner = secure_store_open(&[41; 32], &owner_member, path.to_str().unwrap()).unwrap();
        assert_eq!(
            secure_store_create_group(owner, &[40; 32], &[42; 32], &owner_member, 100),
            Ok(1)
        );
        for value in 1..MAX_GROUP_MEMBERS {
            let seed = (value as u8).wrapping_add(80);
            let member = identity_public(&[seed; 32]).unwrap();
            let invitation = secure_store_export_policy(owner, 101).unwrap();
            let request = create_enrollment_request_from_policy(
                &[seed; 32],
                &[seed.wrapping_add(50); 32],
                &invitation,
                101,
            )
            .unwrap();
            secure_store_issue_enrollment(owner, &[40; 32], &request, 101)
                .unwrap_or_else(|error| panic!("enrollment {value} failed: {error:?}"));
            assert_ne!(member, owner_member);
        }
        let bundle = secure_store_export_policy(owner, 101).unwrap();
        assert_eq!(
            PolicyBundle::decode(&bundle).unwrap().certificates.len(),
            MAX_GROUP_MEMBERS
        );

        let next_seed = 200u8;
        let request = create_enrollment_request_from_policy(
            &[next_seed; 32],
            &[next_seed.wrapping_add(50); 32],
            &bundle,
            101,
        )
        .unwrap();
        assert_eq!(
            secure_store_issue_enrollment(owner, &[40; 32], &request, 101),
            Err(Error::StaleRequest)
        );
        // 49 recipients become five bounded encrypted objects (10 + 10 + 10
        // + 10 + 9). Recipient key wraps make each object two chunks even for
        // this short text, so the durable radio queue has 5 + 10 records.
        assert_eq!(
            secure_store_enqueue_text(owner, &[40; 32], b"grupo", 102),
            Ok(15)
        );
        let summary = stores()
            .lock()
            .unwrap()
            .stores
            .get(&owner)
            .unwrap()
            .latest_logical_delivery_summary(102)
            .unwrap()
            .unwrap();
        assert_eq!(summary.target_count, 49);
        assert_eq!(summary.audience_count, 5);
        assert_eq!(summary.committed_audiences, 5);
        assert_eq!(summary.delivered_targets, 0);
        assert_eq!(
            summary.state,
            mesh_store::LogicalDeliveryState::Queued,
            "one visible group chat action starts pending until every recipient confirms"
        );
        // Hosts ask for the exact logical action they rendered. A query for a
        // different ID must not accidentally expose this newest action.
        let exact = secure_store_delivery_summary(owner, summary.id.0, 102).unwrap();
        assert_eq!(exact.len(), 19);
        assert_eq!(&exact[..16], &summary.id.0);
        assert_eq!(exact[16..], [49, 0, 0]);
        let expired = secure_store_delivery_summary(owner, summary.id.0, 1002).unwrap();
        assert_eq!(
            expired[18], 3,
            "expiry is explicit evidence, never an eternal queue"
        );
        assert!(secure_store_delivery_summary(owner, [0x99; 16], 102)
            .unwrap()
            .is_empty());
        let mut announcements = 0;
        let mut chunks = 0;
        for slot in 0..15 {
            let routed =
                RoutedRecord::decode(&secure_store_outbox_record(owner, slot, 102).unwrap())
                    .unwrap();
            assert_eq!(
                routed.frame.origin,
                MemberId(owner_member.clone().try_into().unwrap())
            );
            assert_eq!(
                routed.frame.previous_hop,
                MemberId(owner_member.clone().try_into().unwrap())
            );
            assert_eq!(routed.frame.hops, 0);
            match routed.record {
                DurableRecord::Announcement(_) => announcements += 1,
                DurableRecord::Chunk { .. } => chunks += 1,
                DurableRecord::Receipt(_) => panic!("outbox must not synthesize a receipt"),
                DurableRecord::ReceiptAck(_) => panic!("outbox must not synthesize a receipt ACK"),
            }
        }
        assert_eq!((announcements, chunks), (5, 10));
        assert!(secure_store_outbox_record(owner, 15, 102)
            .unwrap()
            .is_empty());
        assert_eq!(secure_store_release(owner), Ok(()));
        let _ = std::fs::remove_file(path);
    }
    #[test]
    fn native_session_authenticates_two_enrolled_members_before_delivering_data() {
        let root = std::env::temp_dir().join(format!(
            "mesh-ffi-session-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let owner_path = root.with_extension("owner.db");
        let joiner_path = root.with_extension("joiner.db");
        let owner_member = identity_public(&[20; 32]).unwrap();
        let owner =
            secure_store_open(&[21; 32], &owner_member, owner_path.to_str().unwrap()).unwrap();
        assert_eq!(
            secure_store_create_group(owner, &[20; 32], &[22; 32], &owner_member, 100),
            Ok(1)
        );
        let joiner_member = identity_public(&[23; 32]).unwrap();
        let invitation = secure_store_export_policy(owner, 101).unwrap();
        let request =
            create_enrollment_request_from_policy(&[23; 32], &[24; 32], &invitation, 101).unwrap();
        let bundle = secure_store_issue_enrollment(owner, &[20; 32], &request, 101).unwrap();
        let joiner =
            secure_store_open(&[25; 32], &joiner_member, joiner_path.to_str().unwrap()).unwrap();
        assert_eq!(secure_store_install_policy(joiner, &bundle, 101), Ok(2));

        let initiator = secure_session_start(owner, &[26; 32], &owner_member, 0, 102).unwrap();
        let responder = secure_session_start(joiner, &[27; 32], &joiner_member, 1, 102).unwrap();
        let first = secure_session_write(initiator, 102).unwrap();
        assert_eq!(secure_session_read(responder, &first, 102), Ok(()));
        let second = secure_session_write(responder, 102).unwrap();
        assert_eq!(secure_session_read(initiator, &second, 102), Ok(()));
        let third = secure_session_write(initiator, 102).unwrap();
        assert_eq!(secure_session_read(responder, &third, 102), Ok(()));
        assert_eq!(secure_session_finish(initiator, 102), Ok(()));
        let initiator_auth = secure_session_authenticate(initiator, &[20; 32], 102).unwrap();
        assert_eq!(secure_session_finish(responder, 102), Ok(()));
        let responder_auth = secure_session_authenticate(responder, &[23; 32], 102).unwrap();
        assert_eq!(
            secure_session_receive(responder, &initiator_auth, 102),
            Ok(Vec::new())
        );
        assert_eq!(
            secure_session_receive(initiator, &responder_auth, 102),
            Ok(Vec::new())
        );
        assert_eq!(secure_session_authenticated(initiator), Ok(true));
        assert_eq!(secure_session_authenticated(responder), Ok(true));
        assert_eq!(
            secure_session_peer(initiator),
            Ok(joiner_member.clone().try_into().unwrap())
        );
        assert_eq!(
            secure_session_peer(responder),
            Ok(owner_member.clone().try_into().unwrap())
        );
        let protected = secure_session_send(initiator, b"hola", 102).unwrap();
        assert_eq!(
            secure_session_receive(responder, &protected, 102),
            Ok(b"hola".to_vec())
        );

        assert_eq!(secure_session_release(initiator), Ok(()));
        assert_eq!(secure_session_release(responder), Ok(()));
        assert_eq!(secure_store_release(owner), Ok(()));
        assert_eq!(secure_store_release(joiner), Ok(()));
        let _ = std::fs::remove_file(owner_path);
        let _ = std::fs::remove_file(joiner_path);
    }
    #[test]
    fn authenticated_routed_records_commit_only_after_the_signed_announcement() {
        let root = std::env::temp_dir().join(format!(
            "mesh-ffi-routed-ingress-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let owner_path = root.with_extension("owner.db");
        let joiner_path = root.with_extension("joiner.db");
        let owner_member = identity_public(&[30; 32]).unwrap();
        let owner =
            secure_store_open(&[31; 32], &owner_member, owner_path.to_str().unwrap()).unwrap();
        secure_store_create_group(owner, &[30; 32], &[32; 32], &owner_member, 100).unwrap();
        let joiner_member = identity_public(&[33; 32]).unwrap();
        let invitation = secure_store_export_policy(owner, 101).unwrap();
        let request =
            create_enrollment_request_from_policy(&[33; 32], &[34; 32], &invitation, 101).unwrap();
        let bundle = secure_store_issue_enrollment(owner, &[30; 32], &request, 101).unwrap();
        let joiner =
            secure_store_open(&[35; 32], &joiner_member, joiner_path.to_str().unwrap()).unwrap();
        secure_store_install_policy(joiner, &bundle, 101).unwrap();
        let roster = PolicyBundle::decode(&bundle).unwrap().verify(102).unwrap();
        let policy = mesh_object::ObjectPolicy {
            namespace: mesh_types::durable::Namespace::new("mesh.lab.text").unwrap(),
            epoch: 2,
            targets: vec![MemberId(joiner_member.clone().try_into().unwrap())],
            expires_at: 200,
            hop_limit: 4,
        };
        let message = mesh_protocol::seal_message(
            mesh_protocol::SealRequest {
                origin: MemberId(owner_member.clone().try_into().unwrap()),
                sequence: 1,
                policy,
                plaintext: b"relay durable",
                now: 102,
            },
            &roster,
            &IdentitySigningKey::import(zeroize::Zeroizing::new([30; 32])),
            &mut OsRandom,
        )
        .unwrap();
        let frame = RelayFrame {
            id: mesh_replication::RelayId([6; 16]),
            origin: MemberId(owner_member.try_into().unwrap()),
            previous_hop: MemberId([9; 32]),
            hops: 1,
            hop_limit: 4,
            expires_at: 200,
        };
        let announcement = RoutedRecord {
            frame,
            record: DurableRecord::Announcement(message.announcement().bytes().to_vec()),
        }
        .encode()
        .unwrap();
        assert_eq!(
            secure_store_accept_routed(joiner, &announcement, &frame.previous_hop.0, 102),
            Ok(ROUTED_ACCEPTED_ANNOUNCEMENT)
        );
        let object = message.object().manifest().id();
        for (index, bytes) in message.object().chunks().iter().enumerate() {
            let chunk = RoutedRecord {
                frame,
                record: DurableRecord::Chunk {
                    object,
                    index: index.try_into().unwrap(),
                    bytes: bytes.clone(),
                },
            }
            .encode()
            .unwrap();
            assert_eq!(
                secure_store_accept_routed(joiner, &chunk, &frame.previous_hop.0, 102),
                Ok(ROUTED_ACCEPTED_CHUNK)
            );
        }
        assert!(matches!(
            RoutedRecord::decode(&secure_store_relay_record(joiner, 0, 102).unwrap())
                .unwrap()
                .record,
            DurableRecord::Announcement(_)
        ));
        assert!(matches!(
            RoutedRecord::decode(&secure_store_relay_record(joiner, 1, 102).unwrap()).unwrap().record,
            DurableRecord::Chunk { object: drained, index: 0, .. } if drained == object
        ));
        assert!(secure_store_relay_record(joiner, 99, 102)
            .unwrap()
            .is_empty());
        assert_eq!(
            secure_store_relay_received_from(joiner, 102).unwrap(),
            frame.previous_hop.0
        );
        let unattested = RoutedRecord {
            frame,
            record: DurableRecord::Chunk {
                object,
                index: 0,
                bytes: message.object().chunks()[0].clone(),
            },
        }
        .encode()
        .unwrap();
        assert_eq!(
            secure_store_accept_routed(owner, &unattested, &frame.previous_hop.0, 102),
            Err(Error::InvalidArgument)
        );
        secure_store_release(owner).unwrap();
        secure_store_release(joiner).unwrap();
        let _ = std::fs::remove_file(owner_path);
        let _ = std::fs::remove_file(joiner_path);
    }
    #[test]
    fn durable_text_is_visible_only_after_receiver_commit_and_signed_receipt() {
        let root = std::env::temp_dir().join(format!(
            "mesh-ffi-durable-text-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let owner_path = root.with_extension("owner.db");
        let receiver_path = root.with_extension("receiver.db");
        let owner_member = identity_public(&[61; 32]).unwrap();
        let owner =
            secure_store_open(&[62; 32], &owner_member, owner_path.to_str().unwrap()).unwrap();
        secure_store_create_group(owner, &[61; 32], &[63; 32], &owner_member, 100).unwrap();
        let receiver_member = identity_public(&[64; 32]).unwrap();
        let invitation = secure_store_export_policy(owner, 101).unwrap();
        let request =
            create_enrollment_request_from_policy(&[64; 32], &[65; 32], &invitation, 101).unwrap();
        let bundle = secure_store_issue_enrollment(owner, &[61; 32], &request, 101).unwrap();
        let receiver =
            secure_store_open(&[66; 32], &receiver_member, receiver_path.to_str().unwrap())
                .unwrap();
        secure_store_install_policy(receiver, &bundle, 101).unwrap();

        assert!(
            secure_store_finalize_next_text(receiver, &[64; 32], &[65; 32], 102)
                .unwrap()
                .is_empty()
        );
        assert!(secure_store_enqueue_text(owner, &[61; 32], b"texto durable", 102).unwrap() >= 2);
        let mut ingested = 0;
        for slot in 0..u16::MAX {
            let record = secure_store_outbox_record(owner, slot, 102).unwrap();
            if record.is_empty() {
                break;
            }
            let frame = RoutedRecord::decode(&record).unwrap().frame;
            secure_store_accept_routed(receiver, &record, &frame.previous_hop.0, 102).unwrap();
            ingested += 1;
        }
        assert!(ingested >= 2);
        assert!(!secure_store_relay_record(receiver, 0, 102)
            .unwrap()
            .is_empty());

        // A wrong delivery key cannot expose plaintext or record a receipt;
        // the exact same committed ciphertext remains retryable with the key.
        assert_eq!(
            secure_store_finalize_next_text(receiver, &[64; 32], &[99; 32], 102),
            Err(Error::InvalidArgument)
        );
        let delivered =
            secure_store_finalize_next_text(receiver, &[64; 32], &[65; 32], 102).unwrap();
        assert_eq!(&delivered[..2], &DELIVERED_TEXT_MAGIC);
        assert_eq!(&delivered[34..66], &owner_member);
        assert_eq!(
            u64::from_be_bytes(delivered[66..74].try_into().unwrap()),
            102
        );
        let receipt_len = u16::from_be_bytes([delivered[74], delivered[75]]) as usize;
        let text_start = 76 + receipt_len;
        let text_len =
            u16::from_be_bytes([delivered[text_start], delivered[text_start + 1]]) as usize;
        assert!(receipt_len > 64);
        assert_eq!(
            &delivered[text_start + 2..text_start + 2 + text_len],
            b"texto durable"
        );
        // The durable receipt returns through the same routed envelope. The
        // origin validates the actor signature and retires its source outbox
        // only after this commit, rather than on the first radio write.
        // A process loss after showing the message cannot lose its receipt:
        // reopening the same SQLCipher file reconstructs the routed receipt
        // without retaining plaintext or a RAM-only queue.
        secure_store_release(receiver).unwrap();
        let receiver =
            secure_store_open(&[66; 32], &receiver_member, receiver_path.to_str().unwrap())
                .unwrap();
        let receipt_record = secure_store_receipt_record(receiver, 0, 102).unwrap();
        let receipt_frame = RoutedRecord::decode(&receipt_record).unwrap().frame;
        assert_eq!(
            secure_store_accept_routed(owner, &receipt_record, &receipt_frame.previous_hop.0, 102),
            Ok(ROUTED_ACCEPTED_RECEIPT)
        );
        assert!(secure_store_outbox_record(owner, 0, 102)
            .unwrap()
            .is_empty());
        // The persisted local-delivery marker makes a duplicate callback
        // impossible across retries of the host reducer.
        assert!(
            secure_store_finalize_next_text(receiver, &[64; 32], &[65; 32], 102)
                .unwrap()
                .is_empty()
        );

        secure_store_release(owner).unwrap();
        secure_store_release(receiver).unwrap();
        let _ = std::fs::remove_file(owner_path);
        let _ = std::fs::remove_file(receiver_path);
    }
    #[test]
    fn durable_voice_chunks_commit_before_the_receiver_exposes_audio() {
        let root = std::env::temp_dir().join(format!(
            "mesh-ffi-voice-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let owner_path = root.with_extension("owner.db");
        let receiver_path = root.with_extension("receiver.db");
        let owner_member = identity_public(&[71; 32]).unwrap();
        let owner =
            secure_store_open(&[72; 32], &owner_member, owner_path.to_str().unwrap()).unwrap();
        secure_store_create_group(owner, &[71; 32], &[73; 32], &owner_member, 100).unwrap();
        let receiver_member = identity_public(&[74; 32]).unwrap();
        let invite = secure_store_export_policy(owner, 101).unwrap();
        let request =
            create_enrollment_request_from_policy(&[74; 32], &[75; 32], &invite, 101).unwrap();
        let bundle = secure_store_issue_enrollment(owner, &[71; 32], &request, 101).unwrap();
        let receiver =
            secure_store_open(&[76; 32], &receiver_member, receiver_path.to_str().unwrap())
                .unwrap();
        secure_store_install_policy(receiver, &bundle, 101).unwrap();
        let mut voice = vec![0x56, 0, 0, 0x1f, 0x40];
        voice.extend((0..4096).map(|n| (n % 251) as u8));
        // The 4 KiB audio body must cross the durable object as an
        // announcement plus more than one encrypted chunk. This keeps the
        // test from passing through the small-object path used by short text.
        let queued = secure_store_enqueue_text(owner, &[71; 32], &voice, 102).unwrap();
        assert!(queued > 2, "voice should be split across durable chunks");
        for slot in 0..u16::MAX {
            let record = secure_store_outbox_record(owner, slot, 102).unwrap();
            if record.is_empty() {
                break;
            }
            let frame = RoutedRecord::decode(&record).unwrap().frame;
            secure_store_accept_routed(receiver, &record, &frame.previous_hop.0, 102).unwrap();
        }
        let delivered =
            secure_store_finalize_next_text(receiver, &[74; 32], &[75; 32], 102).unwrap();
        let receipt_len = u16::from_be_bytes([delivered[74], delivered[75]]) as usize;
        let text_at = 76 + receipt_len;
        let payload_len = u16::from_be_bytes([delivered[text_at], delivered[text_at + 1]]) as usize;
        assert_eq!(&delivered[text_at + 2..text_at + 2 + payload_len], &voice);
        let receipt = secure_store_receipt_record(receiver, 0, 102).unwrap();
        let frame = RoutedRecord::decode(&receipt).unwrap().frame;
        assert_eq!(
            secure_store_accept_routed(owner, &receipt, &frame.previous_hop.0, 102),
            Ok(ROUTED_ACCEPTED_RECEIPT)
        );
        assert!(secure_store_outbox_record(owner, 0, 102)
            .unwrap()
            .is_empty());
        secure_store_release(owner).unwrap();
        secure_store_release(receiver).unwrap();
        let _ = std::fs::remove_file(owner_path);
        let _ = std::fs::remove_file(receiver_path);
    }

    #[test]
    fn durable_voice_reaches_a_third_member_after_relay_restart_and_completes_receipts() {
        fn with_hop(record: &[u8], previous_hop: Vec<u8>, hops: u8) -> Vec<u8> {
            let mut routed = RoutedRecord::decode(record).unwrap();
            routed.frame.previous_hop = MemberId(previous_hop.try_into().unwrap());
            routed.frame.hops = hops;
            routed.encode().unwrap()
        }

        let root = std::env::temp_dir().join(format!(
            "mesh-ffi-voice-relay-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let owner_path = root.with_extension("owner.db");
        let relay_path = root.with_extension("relay.db");
        let receiver_path = root.with_extension("receiver.db");
        let owner_member = identity_public(&[81; 32]).unwrap();
        let owner =
            secure_store_open(&[82; 32], &owner_member, owner_path.to_str().unwrap()).unwrap();
        secure_store_create_group(owner, &[81; 32], &[83; 32], &owner_member, 100).unwrap();

        let relay_member = identity_public(&[84; 32]).unwrap();
        let invite = secure_store_export_policy(owner, 101).unwrap();
        let request =
            create_enrollment_request_from_policy(&[84; 32], &[85; 32], &invite, 101).unwrap();
        let relay_bundle = secure_store_issue_enrollment(owner, &[81; 32], &request, 101).unwrap();
        let relay =
            secure_store_open(&[86; 32], &relay_member, relay_path.to_str().unwrap()).unwrap();
        secure_store_install_policy(relay, &relay_bundle, 101).unwrap();

        let receiver_member = identity_public(&[87; 32]).unwrap();
        let invite = secure_store_export_policy(owner, 102).unwrap();
        let request =
            create_enrollment_request_from_policy(&[87; 32], &[88; 32], &invite, 102).unwrap();
        let receiver_bundle =
            secure_store_issue_enrollment(owner, &[81; 32], &request, 102).unwrap();
        let receiver =
            secure_store_open(&[89; 32], &receiver_member, receiver_path.to_str().unwrap())
                .unwrap();
        secure_store_install_policy(receiver, &receiver_bundle, 102).unwrap();
        // Existing members install the exact signed roster before relaying to
        // the newly enrolled phone.
        let updated_roster = secure_store_export_policy(owner, 102).unwrap();
        secure_store_install_policy(relay, &updated_roster, 102).unwrap();

        let mut voice = vec![0x56, 0, 0, 0x0f, 0xa0];
        voice.extend((0..4096).map(|n| (n % 251) as u8));
        assert!(secure_store_enqueue_text(owner, &[81; 32], &voice, 103).unwrap() > 2);

        // A reaches B. B persists the transformed frame and every chunk before
        // it can contact C; it then loses process memory completely.
        for slot in 0..u16::MAX {
            let record = secure_store_outbox_record(owner, slot, 103).unwrap();
            if record.is_empty() {
                break;
            }
            let forwarded = with_hop(&record, relay_member.clone(), 1);
            secure_store_accept_routed(relay, &forwarded, &owner_member, 103).unwrap();
        }
        assert!(!secure_store_relay_record(relay, 0, 103).unwrap().is_empty());
        secure_store_release(relay).unwrap();
        let relay =
            secure_store_open(&[86; 32], &relay_member, relay_path.to_str().unwrap()).unwrap();

        // The restarted B reconstructs its custody queue and reaches C. C
        // cannot see any audio until the complete encrypted object commits.
        for slot in 0..u16::MAX {
            let record = secure_store_relay_record(relay, slot, 104).unwrap();
            if record.is_empty() {
                break;
            }
            secure_store_accept_routed(receiver, &record, &relay_member, 104).unwrap();
        }
        let delivered =
            secure_store_finalize_next_text(receiver, &[87; 32], &[88; 32], 104).unwrap();
        let receipt_len = u16::from_be_bytes([delivered[74], delivered[75]]) as usize;
        let payload_at = 76 + receipt_len;
        let payload_len =
            u16::from_be_bytes([delivered[payload_at], delivered[payload_at + 1]]) as usize;
        assert_eq!(
            &delivered[payload_at + 2..payload_at + 2 + payload_len],
            &voice
        );

        // B is a recipient too, so its receipt first confirms only its own
        // audience share. C's receipt makes the return trip through B; only
        // then does A retire the source object for the complete audience.
        assert!(
            !secure_store_finalize_next_text(relay, &[84; 32], &[85; 32], 104)
                .unwrap()
                .is_empty()
        );
        let relay_receipt = secure_store_receipt_record(relay, 0, 104).unwrap();
        let relay_receipt_frame = RoutedRecord::decode(&relay_receipt).unwrap().frame;
        assert_eq!(
            secure_store_accept_routed(
                owner,
                &relay_receipt,
                &relay_receipt_frame.previous_hop.0,
                104
            ),
            Ok(ROUTED_ACCEPTED_RECEIPT)
        );
        assert!(!secure_store_outbox_record(owner, 0, 104)
            .unwrap()
            .is_empty());
        let partial = stores()
            .lock()
            .unwrap()
            .stores
            .get(&owner)
            .unwrap()
            .latest_logical_delivery_summary(104)
            .unwrap()
            .unwrap();
        assert_eq!(partial.target_count, 2);
        assert_eq!(partial.committed_audiences, 1);
        assert_eq!(partial.delivered_targets, 1);
        assert_eq!(
            partial.state,
            mesh_store::LogicalDeliveryState::PartiallyDelivered
        );

        let receiver_receipt = secure_store_receipt_record(receiver, 0, 104).unwrap();
        let returned = with_hop(&receiver_receipt, relay_member.clone(), 1);
        assert_eq!(
            secure_store_accept_routed(relay, &returned, &receiver_member, 104),
            Ok(ROUTED_ACCEPTED_RECEIPT)
        );
        // The receipt itself must survive B's process death. Its durable
        // receipt queue is independent of B's object queue and can carry a
        // confirmation for an object that B did not originate.
        secure_store_release(relay).unwrap();
        let relay =
            secure_store_open(&[86; 32], &relay_member, relay_path.to_str().unwrap()).unwrap();
        let persisted_receipt = secure_store_relay_receipt_record(relay, 0, 104).unwrap();
        assert!(!persisted_receipt.is_empty());
        assert_eq!(
            secure_store_relay_receipt_received_from(relay, 104).unwrap(),
            receiver_member
        );
        assert_eq!(
            secure_store_accept_routed(owner, &persisted_receipt, &relay_member, 104),
            Ok(ROUTED_ACCEPTED_RECEIPT)
        );
        assert!(secure_store_outbox_record(owner, 0, 104)
            .unwrap()
            .is_empty());
        let complete = stores()
            .lock()
            .unwrap()
            .stores
            .get(&owner)
            .unwrap()
            .latest_logical_delivery_summary(104)
            .unwrap()
            .unwrap();
        assert_eq!(complete.delivered_targets, 2);
        assert_eq!(complete.state, mesh_store::LogicalDeliveryState::Delivered);

        // A now signs an ACK from the same protected identity after its
        // receipt transaction. It travels through B and lets C stop replaying
        // its local receipt without deleting C's durable delivery evidence.
        let mut receiver_ack = Vec::new();
        for slot in 0..u16::MAX {
            let ack = secure_store_receipt_ack_record(owner, &[81; 32], slot, 104).unwrap();
            if ack.is_empty() {
                break;
            }
            let routed = RoutedRecord::decode(&ack).unwrap();
            let DurableRecord::ReceiptAck(bytes) = routed.record else {
                panic!("expected ACK");
            };
            if receipt_ack_route(&bytes).unwrap().actor
                == MemberId(receiver_member.clone().try_into().unwrap())
            {
                receiver_ack = ack;
                break;
            }
        }
        assert!(
            !receiver_ack.is_empty(),
            "origin must issue C's durable ACK"
        );
        let via_relay = with_hop(&receiver_ack, relay_member.clone(), 1);
        assert_eq!(
            secure_store_accept_routed(relay, &via_relay, &owner_member, 104),
            Ok(ROUTED_ACCEPTED_RECEIPT_ACK)
        );
        assert!(secure_store_relay_receipt_record(relay, 0, 104)
            .unwrap()
            .is_empty());
        // B loses RAM after accepting A's ACK. Its dedicated ACK custody must
        // still recreate the canonical routed record for C after reopen.
        secure_store_release(relay).unwrap();
        let relay =
            secure_store_open(&[86; 32], &relay_member, relay_path.to_str().unwrap()).unwrap();
        let persisted_ack = secure_store_relay_receipt_ack_record(relay, 0, 104).unwrap();
        assert!(!persisted_ack.is_empty());
        assert_eq!(
            secure_store_relay_receipt_ack_received_from(relay, 104).unwrap(),
            owner_member
        );
        let mut forged_ack = persisted_ack.clone();
        *forged_ack.last_mut().unwrap() ^= 1;
        assert!(secure_store_accept_routed(receiver, &forged_ack, &relay_member, 104).is_err());
        assert!(!secure_store_receipt_record(receiver, 0, 104)
            .unwrap()
            .is_empty());
        assert_eq!(
            secure_store_accept_routed(receiver, &persisted_ack, &relay_member, 104),
            Ok(ROUTED_ACCEPTED_RECEIPT_ACK)
        );
        assert!(secure_store_receipt_record(receiver, 0, 104)
            .unwrap()
            .is_empty());

        secure_store_release(owner).unwrap();
        secure_store_release(relay).unwrap();
        secure_store_release(receiver).unwrap();
        let _ = std::fs::remove_file(owner_path);
        let _ = std::fs::remove_file(relay_path);
        let _ = std::fs::remove_file(receiver_path);
    }
}
