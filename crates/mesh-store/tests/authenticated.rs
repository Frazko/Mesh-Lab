mod support;
use mesh_crypto::{IdentitySigningKey, Scope};
use mesh_object::{digest, PreparedObject};
use mesh_protocol::{self as protocol, CertificateClaims, DurableRecord, VerifiedRoster};
use mesh_replication::{RelayCache, RelayDecision, RelayFrame, RelayId, RoutedRecord};
use mesh_store::{Limits, Store};
use mesh_types::durable::*;
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use support::*;
use zeroize::Zeroizing;
static SERIAL: AtomicU64 = AtomicU64::new(0);
const OP: OperationId = OperationId([1; 16]);
struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "mesh-auth-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn open(&self, name: &str, member: u8) -> Store {
        open(&self.path(name), member)
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn open(path: &Path, member: u8) -> Store {
    Store::open(
        path,
        Zeroizing::new([7; 32]),
        MemberId([member; 32]),
        Limits::default(),
    )
    .unwrap()
}
fn stage(s: &mut Store, lab: &Lab, message: &protocol::SealedMessage) {
    let id = s
        .announce_authenticated(message.announcement(), &lab.roster, 1)
        .unwrap();
    for (i, b) in message.object().chunks().iter().enumerate() {
        s.stage_authenticated_chunk(id, i, b, 2).unwrap();
    }
}

#[test]
fn version_ten_adds_ack_cache_without_losing_reserved_operations() {
    let files = Files::new();
    let path = files.path("version-ten.db");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.pragma_update(None, "key", format!("x'{}'", "07".repeat(32)))
        .unwrap();
    let scripts = [
        include_str!("../../../schema/store/001.sql"),
        include_str!("../../../schema/store/002-authentication.sql"),
        include_str!("../../../schema/store/003-policy.sql"),
        include_str!("../../../schema/store/004-group-capacity.sql"),
        include_str!("../../../schema/store/005-relay-custody.sql"),
        include_str!("../../../schema/store/006-relay-ingress.sql"),
        include_str!("../../../schema/store/007-relay-receipts.sql"),
        include_str!("../../../schema/store/008-receipt-acks.sql"),
        include_str!("../../../schema/store/009-relay-receipt-acks.sql"),
        include_str!("../../../schema/store/010-logical-delivery.sql"),
    ];
    for script in scripts {
        db.execute_batch(script).unwrap();
    }
    let hash = digest(scripts.concat().as_bytes());
    db.execute(
        "INSERT INTO meta VALUES(1,?1,2,?2)",
        rusqlite::params![&[1u8; 32][..], &hash[..]],
    )
    .unwrap();
    db.execute(
        "INSERT INTO operations(id,command_hash,sequence) VALUES(?1,?2,1)",
        rusqlite::params![&OP.0[..], &[0u8; 32][..]],
    )
    .unwrap();
    drop(db);
    let mut store = open(&path, 1);
    assert_eq!(store.reserve(OP, [0; 32]).unwrap().sequence, 1);
    drop(store);
    let mut reopened = open(&path, 1);
    assert_eq!(reopened.reserve(OP, [0; 32]).unwrap().sequence, 1);
}
fn source(s: &mut Store, lab: &Lab, message: &protocol::SealedMessage) {
    s.reserve(OP, [0; 32]).unwrap();
    s.commit_sealed(OP, [0; 32], message, &lab.roster, 1)
        .unwrap();
}
#[test]
fn complete_chunks_are_not_delivery_and_legacy_methods_cannot_bypass_authentication() {
    let files = Files::new();
    let lab = Lab::new();
    let message = lab.message(&[2]);
    let id = message.object().manifest().id();
    let mut receiver = files.open("b.db", 2);
    receiver
        .announce_authenticated(message.announcement(), &lab.roster, 1)
        .unwrap();
    receiver
        .stage_authenticated_chunk(id, 0, &message.object().chunks()[0], 2)
        .unwrap();
    drop(receiver);
    let mut receiver = files.open("b.db", 2);
    assert!(!receiver.missing(id).unwrap().is_empty());
    assert!(receiver
        .verify_received(id, &lab.roster, &lab.secrets[1], 3)
        .is_err());
    for (i, b) in message.object().chunks().iter().enumerate() {
        receiver.stage_authenticated_chunk(id, i, b, 3).unwrap();
    }
    assert!(receiver.missing(id).unwrap().is_empty());
    assert!(receiver.local_commit(id).unwrap().is_none());
    assert!(receiver.local_receipt(id).unwrap().is_none());
    assert!(matches!(
        receiver.put_chunk(id, 0, &message.object().chunks()[0], 4),
        Err(DurableError::AuthenticationFailed)
    ));
    assert!(matches!(
        receiver.announce(message.object().manifest(), 4),
        Err(DurableError::AuthenticationFailed)
    ));
    assert!(receiver
        .verify_received(id, &lab.roster, &lab.secrets[2], 4)
        .is_err());
    assert!(receiver.local_commit(id).unwrap().is_none());
    let proof = receiver
        .verify_received(id, &lab.roster, &lab.secrets[1], 4)
        .unwrap();
    assert!(receiver
        .finalize_received(&proof, &lab.roster, &lab.signers[2], 5)
        .is_err());
    let commit = receiver
        .finalize_received(&proof, &lab.roster, &lab.signers[1], 5)
        .unwrap();
    let receipt = commit.receipt().to_vec();
    drop(receiver);
    let mut receiver = files.open("b.db", 2);
    assert_eq!(
        receiver.local_receipt(id).unwrap().unwrap().receipt(),
        receipt
    );
    assert_eq!(
        receiver
            .finalize_received(&proof, &lab.roster, &lab.signers[1], 6)
            .unwrap()
            .receipt(),
        receipt
    );
    assert_eq!(receiver.stats().unwrap().local_deliveries, 1);
}
#[test]
fn all_targets_must_confirm_before_retiring_outbox_and_source_copy_survives() {
    let files = Files::new();
    let lab = Lab::new();
    let message = lab.message(&[2, 3]);
    let id = message.object().manifest().id();
    let mut sender = files.open("a.db", 1);
    source(&mut sender, &lab, &message);
    for member in [2, 3] {
        let mut receiver = files.open(&format!("{member}.db"), member);
        stage(&mut receiver, &lab, &message);
        let proof = receiver
            .verify_received(id, &lab.roster, &lab.secrets[member as usize - 1], 4)
            .unwrap();
        let receipt = receiver
            .finalize_received(&proof, &lab.roster, &lab.signers[member as usize - 1], 5)
            .unwrap();
        let mut corrupt = receipt.receipt().to_vec();
        *corrupt.last_mut().unwrap() ^= 1;
        assert!(sender
            .verify_target_receipt(id, &corrupt, &lab.roster, 6)
            .is_err());
        let verified = sender
            .verify_target_receipt(id, receipt.receipt(), &lab.roster, 6)
            .unwrap();
        let progress = sender
            .record_target_receipt(&verified, &lab.roster, 6)
            .unwrap();
        assert_eq!(progress.confirmed, member as usize - 1);
        assert_eq!(progress.required, 2);
        sender
            .record_target_receipt(&verified, &lab.roster, 7)
            .unwrap();
        assert_eq!(sender.authenticated_progress(id, 7).unwrap(), progress);
        assert_eq!(sender.outbox(7).unwrap().is_empty(), member == 3);
    }
    drop(sender);
    let sender = files.open("a.db", 1);
    assert_eq!(
        sender.authenticated_progress(id, 101).unwrap().state,
        DeliveryState::Delivered
    );
    assert_eq!(
        sender.object_bytes(id).unwrap(),
        message.object().chunks().concat()
    );
    assert_eq!(sender.stats().unwrap().objects, 1);
    assert_eq!(sender.stats().unwrap().outbox, 0);
}
#[test]
fn signed_but_undecryptable_payload_never_gets_a_delivery_marker() {
    use mesh_codec::canonical::Writer;
    use mesh_crypto::Domain;
    let files = Files::new();
    let lab = Lab::new();
    let message = lab.message(&[2]);
    let mut bytes = message.object().chunks().concat();
    *bytes.last_mut().unwrap() ^= 1;
    let bad = PreparedObject::from_opaque(MemberId([1; 32]), 1, lab.policy(&[2]), &bytes).unwrap();
    let mut w = Writer::default();
    w.array(3);
    w.uint(1);
    w.bytes(&lab.roster.scope().group);
    w.bytes(&bad.manifest().encode());
    let body = w.finish();
    let sig = lab.signers[0]
        .sign(lab.roster.scope(), Domain::Object, &body)
        .unwrap();
    let mut w = Writer::default();
    w.array(2);
    w.bytes(&body);
    w.bytes(&sig);
    let a = protocol::authenticate_announcement(&w.finish(), &lab.roster, 1).unwrap();
    let mut receiver = files.open("b.db", 2);
    let id = receiver.announce_authenticated(&a, &lab.roster, 1).unwrap();
    for (i, b) in bad.chunks().iter().enumerate() {
        receiver.stage_authenticated_chunk(id, i, b, 2).unwrap();
    }
    assert!(receiver
        .verify_received(id, &lab.roster, &lab.secrets[1], 3)
        .is_err());
    assert!(receiver.local_commit(id).unwrap().is_none());
    assert!(receiver.local_receipt(id).unwrap().is_none());
    assert_eq!(receiver.stats().unwrap().local_deliveries, 0);
}
#[test]
fn receipt_for_another_object_and_stale_roster_proof_are_rejected() {
    let files = Files::new();
    let lab = Lab::new();
    let message = lab.message(&[2]);
    let id = message.object().manifest().id();
    let mut receiver = files.open("b.db", 2);
    stage(&mut receiver, &lab, &message);
    let proof = receiver
        .verify_received(id, &lab.roster, &lab.secrets[1], 4)
        .unwrap();
    let changed = VerifiedRoster::verify(
        lab.authority.public_key(),
        lab.roster.scope(),
        &lab.certificates,
        &[999],
        4,
    )
    .unwrap();
    assert!(receiver
        .finalize_received(&proof, &changed, &lab.signers[1], 5)
        .is_err());
    assert!(receiver.local_commit(id).unwrap().is_none());
    let receipt = receiver
        .finalize_received(&proof, &lab.roster, &lab.signers[1], 5)
        .unwrap();
    let other = protocol::seal_message(
        protocol::SealRequest {
            origin: MemberId([1; 32]),
            sequence: 2,
            policy: lab.policy(&[2]),
            plaintext: b"other object",
            now: 1,
        },
        &lab.roster,
        &lab.signers[0],
        &mut TestRandom::new(),
    )
    .unwrap();
    assert!(protocol::verify_receipt(
        receipt.receipt(),
        other.announcement(),
        &other.object().chunks().concat(),
        &lab.roster,
        7
    )
    .is_err());
    assert!(receiver
        .verify_received(id, &lab.roster, &lab.secrets[1], 100)
        .is_err());
    let mut sender = files.open("a.db", 1);
    source(&mut sender, &lab, &message);
    let receipt = sender
        .verify_target_receipt(id, receipt.receipt(), &lab.roster, 101)
        .unwrap();
    assert_eq!(
        sender
            .record_target_receipt(&receipt, &lab.roster, 101)
            .unwrap()
            .state,
        DeliveryState::Delivered
    );
}
#[test]
fn version_one_migrates_without_losing_reserved_sequence_and_bad_hash_rolls_back() {
    for bad in [false, true] {
        let files = Files::new();
        let path = files.path("v1.db");
        let db = rusqlite::Connection::open(&path).unwrap();
        db.pragma_update(None, "key", format!("x'{}'", "07".repeat(32)))
            .unwrap();
        let schema = include_str!("../../../schema/store/001.sql");
        db.execute_batch(schema).unwrap();
        let hash = if bad {
            [0; 32]
        } else {
            digest(schema.as_bytes())
        };
        db.execute(
            "INSERT INTO meta VALUES(1,?1,2,?2)",
            rusqlite::params![&[1u8; 32][..], &hash[..]],
        )
        .unwrap();
        db.execute(
            "INSERT INTO operations(id,command_hash,sequence) VALUES(?1,?2,1)",
            rusqlite::params![&OP.0[..], &[0u8; 32][..]],
        )
        .unwrap();
        drop(db);
        let result = Store::open(
            &path,
            Zeroizing::new([7; 32]),
            MemberId([1; 32]),
            Limits::default(),
        );
        if bad {
            assert!(matches!(result, Err(DurableError::Conflict)));
            let db = rusqlite::Connection::open(&path).unwrap();
            db.pragma_update(None, "key", format!("x'{}'", "07".repeat(32)))
                .unwrap();
            assert_eq!(
                db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                1
            );
        } else {
            let mut s = result.unwrap();
            assert_eq!(s.reserve(OP, [0; 32]).unwrap().sequence, 1);
            assert_eq!(
                s.reserve(OperationId([2; 16]), [0; 32]).unwrap().sequence,
                2
            );
            assert_eq!(s.stats().unwrap().local_deliveries, 0);
            drop(s);
            open(&path, 1);
        }
    }
}
#[test]
fn version_three_expands_policy_roster_to_fifty_without_losing_certificates() {
    let files = Files::new();
    let path = files.path("v3.db");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.pragma_update(None, "key", format!("x'{}'", "07".repeat(32)))
        .unwrap();
    let base = include_str!("../../../schema/store/001.sql");
    let authentication = include_str!("../../../schema/store/002-authentication.sql");
    let policy = include_str!("../../../schema/store/003-policy.sql");
    db.execute_batch(base).unwrap();
    db.execute_batch(authentication).unwrap();
    db.execute_batch(policy).unwrap();
    let old_hash = digest(
        [
            base.as_bytes(),
            authentication.as_bytes(),
            policy.as_bytes(),
        ]
        .concat()
        .as_slice(),
    );
    db.execute(
        "INSERT INTO meta VALUES(1,?1,1,?2)",
        rusqlite::params![&[1u8; 32][..], &old_hash[..]],
    )
    .unwrap();
    db.execute(
        "INSERT INTO policy_certificates VALUES(0,?1)",
        rusqlite::params![&[7u8][..]],
    )
    .unwrap();
    drop(db);

    drop(open(&path, 1));
    let db = rusqlite::Connection::open(&path).unwrap();
    db.pragma_update(None, "key", format!("x'{}'", "07".repeat(32)))
        .unwrap();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        11
    );
    assert_eq!(
        db.query_row(
            "SELECT certificate FROM policy_certificates WHERE idx=0",
            [],
            |r| { r.get::<_, Vec<u8>>(0) }
        )
        .unwrap(),
        vec![7]
    );
    db.execute(
        "INSERT INTO policy_certificates VALUES(49,?1)",
        rusqlite::params![&[8u8][..]],
    )
    .unwrap();
    assert!(db
        .execute(
            "INSERT INTO policy_certificates VALUES(50,?1)",
            rusqlite::params![&[9u8][..]],
        )
        .is_err());
    assert!(
        db.query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='relay_outbox'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
            == 1
    );
}
#[cfg(feature = "fault-injection")]
mod crash {
    use super::*;
    fn die(stage: mesh_store::Stage) {
        if format!("{stage:?}") == std::env::var("MESH_AUTH_CRASH_STAGE").unwrap() {
            std::process::exit(73)
        }
    }
    #[test]
    fn child() {
        let Ok(path) = std::env::var("MESH_AUTH_CRASH_PATH") else {
            return;
        };
        let source = std::env::var("MESH_AUTH_CRASH_SOURCE").is_ok();
        let lab = Lab::new();
        let message = lab.message(&[2]);
        let id = message.object().manifest().id();
        let mut s = open(Path::new(&path), if source { 1 } else { 2 });
        s.set_fault_hook(die);
        if source {
            let bytes = std::fs::read(Path::new(&path).with_file_name("receipt.bin")).unwrap();
            let proof = s.verify_target_receipt(id, &bytes, &lab.roster, 6).unwrap();
            s.record_target_receipt(&proof, &lab.roster, 6).unwrap();
        } else {
            let proof = s
                .verify_received(id, &lab.roster, &lab.secrets[1], 4)
                .unwrap();
            s.finalize_received(&proof, &lab.roster, &lab.signers[1], 5)
                .unwrap();
        }
        panic!("fault point not reached")
    }
    fn kill(path: &Path, stage: &str, source: bool) {
        let mut c = std::process::Command::new(std::env::current_exe().unwrap());
        c.args(["--exact", "crash::child", "--nocapture"])
            .env("MESH_AUTH_CRASH_PATH", path)
            .env("MESH_AUTH_CRASH_STAGE", stage);
        if source {
            c.env("MESH_AUTH_CRASH_SOURCE", "1");
        } else {
            c.env_remove("MESH_AUTH_CRASH_SOURCE");
        }
        let o = c.output().unwrap();
        assert_eq!(
            o.status.code(),
            Some(73),
            "{}",
            String::from_utf8_lossy(&o.stderr)
        );
    }
    #[test]
    fn receipt_and_delivery_are_atomic_across_process_death() {
        for stage_name in ["DeliveryRecorded", "BeforeCommit", "AfterCommit"] {
            let files = Files::new();
            let lab = Lab::new();
            let message = lab.message(&[2]);
            let id = message.object().manifest().id();
            let mut s = files.open("b.db", 2);
            stage(&mut s, &lab, &message);
            drop(s);
            kill(&files.path("b.db"), stage_name, false);
            let mut s = files.open("b.db", 2);
            let committed = stage_name == "AfterCommit";
            assert_eq!(s.local_commit(id).unwrap().is_some(), committed);
            assert_eq!(s.local_receipt(id).unwrap().is_some(), committed);
            let proof = s
                .verify_received(id, &lab.roster, &lab.secrets[1], 6)
                .unwrap();
            let commit = s
                .finalize_received(&proof, &lab.roster, &lab.signers[1], 6)
                .unwrap();
            assert_eq!(commit.local().committed_at(), if committed { 5 } else { 6 });
            assert_eq!(s.stats().unwrap().local_deliveries, 1);
        }
    }
    #[test]
    fn source_receipt_and_outbox_retirement_are_atomic_across_process_death() {
        for stage_name in ["ReceiptRecorded", "BeforeCommit", "AfterCommit"] {
            let files = Files::new();
            let lab = Lab::new();
            let message = lab.message(&[2]);
            let id = message.object().manifest().id();
            let mut a = files.open("a.db", 1);
            source(&mut a, &lab, &message);
            let mut b = files.open("b.db", 2);
            stage(&mut b, &lab, &message);
            let p = b
                .verify_received(id, &lab.roster, &lab.secrets[1], 4)
                .unwrap();
            let receipt = b
                .finalize_received(&p, &lab.roster, &lab.signers[1], 5)
                .unwrap();
            std::fs::write(files.path("receipt.bin"), receipt.receipt()).unwrap();
            drop(a);
            drop(b);
            kill(&files.path("a.db"), stage_name, true);
            let mut a = files.open("a.db", 1);
            let committed = stage_name == "AfterCommit";
            assert_eq!(
                a.authenticated_progress(id, 7).unwrap().confirmed,
                usize::from(committed)
            );
            assert_eq!(a.outbox(7).unwrap().is_empty(), committed);
            let p = a
                .verify_target_receipt(id, receipt.receipt(), &lab.roster, 7)
                .unwrap();
            assert_eq!(
                a.record_target_receipt(&p, &lab.roster, 7).unwrap().state,
                DeliveryState::Delivered
            );
            assert_eq!(a.stats().unwrap().objects, 1);
        }
    }
}

#[test]
fn receiver_reducer_emits_only_after_commit_and_recovers_a_lost_callback_after_expiry() {
    use mesh_runtime::reception::*;
    let files = Files::new();
    let lab = Lab::new();
    let message = lab.message(&[2]);
    let id = message.object().manifest().id();
    let mut store = files.open("b.db", 2);
    stage(&mut store, &lab, &message);
    let mut core = DurableReceiver::new(MemberId([2; 32]), 10).unwrap();
    let inspect = core.start(id).unwrap().effect.unwrap().token();
    assert!(core.start(id).unwrap().effect.is_none());
    assert!(core.inspected(inspect, Some(ObjectId([99; 32]))).is_err());
    let verify = core
        .inspected(inspect, None)
        .unwrap()
        .effect
        .unwrap()
        .token();
    assert!(core.inspected(inspect, None).is_err());
    assert!(core.committed(verify, id).is_err());
    let proof = store
        .verify_received(id, &lab.roster, &lab.secrets[1], 4)
        .unwrap();
    let next = core.verified(verify, proof).unwrap();
    assert!(next.event.is_none());
    let ReceiveEffect::Commit { token, proof } = next.effect.unwrap() else {
        panic!("expected commit")
    };
    assert!(store.local_receipt(id).unwrap().is_none());
    store
        .finalize_received(&proof, &lab.roster, &lab.signers[1], 5)
        .unwrap();
    drop(store); // callback lost with process
    let store = files.open("b.db", 2);
    let mut core = DurableReceiver::new(MemberId([2; 32]), 11).unwrap();
    let inspect = core.start(id).unwrap().effect.unwrap().token();
    assert!(core.committed(token, id).is_err());
    assert!(store
        .verify_received(id, &lab.roster, &lab.secrets[1], 101)
        .is_err());
    let persisted = store.local_receipt(id).unwrap().unwrap();
    let event = core
        .inspected(inspect, Some(persisted.local().object_id()))
        .unwrap()
        .event;
    assert_eq!(event, Some(ReceiveEvent::Received { object: id }));
    assert!(core.pending().is_empty());
    assert!(core.inspected(inspect, Some(id)).is_err());
    assert_eq!(store.stats().unwrap().local_deliveries, 1);
}

#[test]
fn receiver_reducer_bounds_parallel_work_and_retries_failures_with_new_tokens() {
    use mesh_runtime::reception::*;
    let mut core = DurableReceiver::new(MemberId([2; 32]), 1).unwrap();
    let mut tokens = Vec::new();
    for i in 0..MAX_PENDING_RECEIVES {
        tokens.push(
            core.start(ObjectId([i as u8; 32]))
                .unwrap()
                .effect
                .unwrap()
                .token(),
        )
    }
    assert!(matches!(
        core.start(ObjectId([99; 32])),
        Err(DurableError::ResourcePressure)
    ));
    assert_eq!(
        core.failed(tokens[0], DurableError::StorageUnavailable)
            .unwrap()
            .event,
        Some(ReceiveEvent::NeedsRetry {
            object: ObjectId([0; 32]),
            error: DurableError::StorageUnavailable
        })
    );
    let inspect = core
        .start(ObjectId([0; 32]))
        .unwrap()
        .effect
        .unwrap()
        .token();
    assert_ne!(inspect, tokens[0]);
    assert!(core
        .failed(tokens[0], DurableError::StorageUnavailable)
        .is_err());
}

#[test]
fn self_target_requires_verified_commit_and_synthetic_delivery_cannot_be_promoted() {
    use mesh_runtime::reception::*;
    let files = Files::new();
    let lab = Lab::new();
    let message = lab.message(&[1, 2]);
    let id = message.object().manifest().id();
    let mut store = files.open("a.db", 1);
    source(&mut store, &lab, &message);
    assert!(store.local_commit(id).unwrap().is_none());
    assert!(store.local_receipt(id).unwrap().is_none());
    let mut core = DurableReceiver::new(MemberId([1; 32]), 20).unwrap();
    let inspect = core.start(id).unwrap().effect.unwrap().token();
    let verify = core
        .inspected(inspect, None)
        .unwrap()
        .effect
        .unwrap()
        .token();
    let proof = store
        .verify_received(id, &lab.roster, &lab.secrets[0], 4)
        .unwrap();
    let transition = core.verified(verify, proof).unwrap();
    assert!(transition.event.is_none());
    let ReceiveEffect::Commit { token, proof } = transition.effect.unwrap() else {
        panic!("expected commit")
    };
    let receipt = store
        .finalize_received(&proof, &lab.roster, &lab.signers[0], 5)
        .unwrap();
    assert!(core.committed(token, ObjectId([99; 32])).is_err());
    assert_eq!(
        core.committed(token, receipt.local().object_id())
            .unwrap()
            .event,
        Some(ReceiveEvent::Received { object: id })
    );
    assert!(core.committed(token, id).is_err());
    let proof = store
        .verify_target_receipt(id, receipt.receipt(), &lab.roster, 6)
        .unwrap();
    let progress = store.record_target_receipt(&proof, &lab.roster, 6).unwrap();
    assert_eq!(progress.confirmed, 1);
    assert_eq!(progress.required, 2);
    assert_eq!(store.outbox(6).unwrap(), vec![id]);
    let mut legacy = files.open("legacy.db", 2);
    legacy.announce(message.object().manifest(), 1).unwrap();
    for (i, bytes) in message.object().chunks().iter().enumerate() {
        legacy.put_chunk(id, i, bytes, 2).unwrap();
    }
    assert!(legacy.local_commit(id).unwrap().is_some());
    assert!(matches!(
        legacy.announce_authenticated(message.announcement(), &lab.roster, 3),
        Err(DurableError::AuthenticationFailed)
    ));
    assert!(legacy.local_receipt(id).unwrap().is_none());
}

#[test]
fn authenticated_noise_channel_carries_the_complete_durable_delivery_and_receipt() {
    use mesh_session::{Config, Handshake, Incoming, Role, SessionSecret};
    let files = Files::new();
    let lab = Lab::new();
    let message = lab.message(&[2]);
    let id = message.object().manifest().id();
    let mut sender = files.open("noise-a.db", 1);
    source(&mut sender, &lab, &message);
    let mut receiver = files.open("noise-b.db", 2);
    let mut rng = TestRandom::new();
    let mut a = Handshake::start(
        Config {
            role: Role::Initiator,
            local: MemberId([1; 32]),
            expected_peer: Some(MemberId([2; 32])),
            now: 1,
        },
        &lab.roster,
        &SessionSecret::import(Zeroizing::new([70; 32])),
        &mut rng,
    )
    .unwrap();
    let mut b = Handshake::start(
        Config {
            role: Role::Responder,
            local: MemberId([2; 32]),
            expected_peer: Some(MemberId([1; 32])),
            now: 1,
        },
        &lab.roster,
        &SessionSecret::import(Zeroizing::new([71; 32])),
        &mut rng,
    )
    .unwrap();
    b.read(&a.write(2).unwrap(), 2).unwrap();
    a.read(&b.write(2).unwrap(), 2).unwrap();
    b.read(&a.write(2).unwrap(), 2).unwrap();
    let (mut a, mut b) = (
        a.finish(&lab.roster, 2).unwrap(),
        b.finish(&lab.roster, 2).unwrap(),
    );
    let aa = a.authentication(&lab.signers[0], &lab.roster, 2).unwrap();
    let ba = b.authentication(&lab.signers[1], &lab.roster, 2).unwrap();
    b.receive(&aa, &lab.roster, 2).unwrap();
    a.receive(&ba, &lab.roster, 2).unwrap();
    let announcement_record = DurableRecord::Announcement(message.announcement().bytes().to_vec())
        .encode()
        .unwrap();
    let frame = a.send(&announcement_record, &lab.roster, 3).unwrap();
    let Incoming::Data(bytes) = b.receive(&frame, &lab.roster, 3).unwrap() else {
        panic!("expected announcement")
    };
    let DurableRecord::Announcement(bytes) = DurableRecord::decode(&bytes).unwrap() else {
        panic!("expected announcement record")
    };
    let announcement = protocol::authenticate_announcement(&bytes, &lab.roster, 3).unwrap();
    receiver
        .announce_authenticated(&announcement, &lab.roster, 3)
        .unwrap();
    for (i, chunk) in message.object().chunks().iter().enumerate() {
        let record = DurableRecord::Chunk {
            object: id,
            index: i.try_into().unwrap(),
            bytes: chunk.clone(),
        }
        .encode()
        .unwrap();
        let frame = a.send(&record, &lab.roster, 3).unwrap();
        let Incoming::Data(bytes) = b.receive(&frame, &lab.roster, 3).unwrap() else {
            panic!("expected chunk")
        };
        let DurableRecord::Chunk {
            object,
            index,
            bytes,
        } = DurableRecord::decode(&bytes).unwrap()
        else {
            panic!("expected chunk record")
        };
        assert_eq!(object, id);
        receiver
            .stage_authenticated_chunk(id, usize::from(index), &bytes, 3)
            .unwrap();
        assert!(matches!(
            b.receive(&frame, &lab.roster, 3),
            Err(mesh_session::Error::Replay)
        ));
    }
    assert!(receiver.local_receipt(id).unwrap().is_none());
    let proof = receiver
        .verify_received(id, &lab.roster, &lab.secrets[1], 4)
        .unwrap();
    let receipt = receiver
        .finalize_received(&proof, &lab.roster, &lab.signers[1], 5)
        .unwrap();
    let receipt_record = DurableRecord::Receipt(receipt.receipt().to_vec())
        .encode()
        .unwrap();
    let frame = b.send(&receipt_record, &lab.roster, 5).unwrap();
    let Incoming::Data(bytes) = a.receive(&frame, &lab.roster, 5).unwrap() else {
        panic!("expected receipt")
    };
    let DurableRecord::Receipt(bytes) = DurableRecord::decode(&bytes).unwrap() else {
        panic!("expected receipt record")
    };
    let proof = sender
        .verify_target_receipt(id, &bytes, &lab.roster, 5)
        .unwrap();
    assert_eq!(
        sender
            .record_target_receipt(&proof, &lab.roster, 5)
            .unwrap()
            .state,
        DeliveryState::Delivered
    );
    assert_eq!(receiver.stats().unwrap().local_deliveries, 1);
    assert!(sender.outbox(5).unwrap().is_empty());
    assert_eq!(
        sender.object_bytes(id).unwrap(),
        message.object().chunks().concat()
    );
}

#[test]
fn three_node_relay_survives_a_restart_then_delivers_the_same_routed_object() {
    // A cannot reach C. B accepts the routed announcement once, takes custody
    // of every authenticated chunk, restarts, and only then forwards the
    // persisted object and route to C.
    let files = Files::new();
    let lab = Lab::new();
    let message = lab.message(&[3]);
    let id = message.object().manifest().id();
    let mut source_store = files.open("relay-a.db", 1);
    source(&mut source_store, &lab, &message);
    let first_hop = RelayFrame {
        id: RelayId([9; 16]),
        origin: MemberId([1; 32]),
        previous_hop: MemberId([1; 32]),
        hops: 0,
        hop_limit: 4,
        expires_at: 100,
    };

    let mut relay_store = files.open("relay-b.db", 2);
    let mut relay_cache = RelayCache::new(MemberId([2; 32]));
    let announce = RoutedRecord {
        frame: first_hop,
        record: DurableRecord::Announcement(message.announcement().bytes().to_vec()),
    };
    let RoutedRecord { frame, record } = RoutedRecord::decode(&announce.encode().unwrap()).unwrap();
    let DurableRecord::Announcement(bytes) = record else {
        panic!("expected announcement")
    };
    let announcement = protocol::authenticate_announcement(&bytes, &lab.roster, 3).unwrap();
    assert_eq!(
        relay_store
            .announce_authenticated(&announcement, &lab.roster, 3)
            .unwrap(),
        id
    );
    let forwarded = match relay_cache.accept(frame, MemberId([1; 32]), 3).unwrap() {
        RelayDecision::Forward(frame) => frame,
        other => panic!("relay did not forward: {other:?}"),
    };

    for (index, chunk) in message.object().chunks().iter().enumerate() {
        let routed = RoutedRecord {
            frame: first_hop,
            record: DurableRecord::Chunk {
                object: id,
                index: index.try_into().unwrap(),
                bytes: chunk.clone(),
            },
        };
        let RoutedRecord { frame, record } =
            RoutedRecord::decode(&routed.encode().unwrap()).unwrap();
        assert_eq!(frame, first_hop);
        let DurableRecord::Chunk {
            object,
            index,
            bytes,
        } = record
        else {
            panic!("expected chunk")
        };
        assert_eq!(object, id);
        relay_store
            .put_relay_chunk(
                id,
                usize::from(index),
                &bytes,
                forwarded,
                MemberId([1; 32]),
                4,
            )
            .unwrap();
    }
    assert_eq!(relay_store.relay_queue(4).unwrap().len(), 1);
    drop(relay_store);

    let relay_store = files.open("relay-b.db", 2);
    let custody = relay_store.relay_queue(5).unwrap().pop().unwrap();
    assert_eq!(custody.frame, forwarded);
    assert_eq!(custody.received_from, MemberId([1; 32]));
    assert_eq!(
        relay_store.object_bytes(id).unwrap(),
        message.object().chunks().concat()
    );
    let persisted_announcement = relay_store
        .authenticated_announcement(id, &lab.roster, 5)
        .unwrap();

    let mut destination = files.open("relay-c.db", 3);
    let routed = RoutedRecord {
        frame: custody.frame,
        record: DurableRecord::Announcement(persisted_announcement.bytes().to_vec()),
    };
    let RoutedRecord { frame, record } = RoutedRecord::decode(&routed.encode().unwrap()).unwrap();
    assert_eq!(frame, forwarded);
    let DurableRecord::Announcement(bytes) = record else {
        panic!("expected forwarded announcement")
    };
    let announcement = protocol::authenticate_announcement(&bytes, &lab.roster, 5).unwrap();
    destination
        .announce_authenticated(&announcement, &lab.roster, 5)
        .unwrap();
    for (index, chunk) in relay_store
        .object_bytes(id)
        .unwrap()
        .chunks(CHUNK_BYTES)
        .enumerate()
    {
        let routed = RoutedRecord {
            frame: custody.frame,
            record: DurableRecord::Chunk {
                object: id,
                index: index.try_into().unwrap(),
                bytes: chunk.to_vec(),
            },
        };
        let RoutedRecord { frame, record } =
            RoutedRecord::decode(&routed.encode().unwrap()).unwrap();
        assert_eq!(frame, forwarded);
        let DurableRecord::Chunk {
            object,
            index,
            bytes,
        } = record
        else {
            panic!("expected forwarded chunk")
        };
        destination
            .stage_authenticated_chunk(object, usize::from(index), &bytes, 5)
            .unwrap();
    }
    let proof = destination
        .verify_received(id, &lab.roster, &lab.secrets[2], 6)
        .unwrap();
    destination
        .finalize_received(&proof, &lab.roster, &lab.signers[2], 7)
        .unwrap();
    assert_eq!(destination.stats().unwrap().local_deliveries, 1);
    assert_eq!(
        destination.object_bytes(id).unwrap(),
        message.object().chunks().concat()
    );
    assert!(matches!(
        relay_cache.accept(first_hop, MemberId([1; 32]), 8),
        Ok(RelayDecision::Duplicate)
    ));
}

#[test]
fn validated_policy_snapshot_survives_reopen() {
    let files = Files::new();
    let lab = Lab::new();
    let mut store = files.open("policy.db", 1);
    let scope = lab.roster.scope();
    let installed = store
        .install_policy(lab.authority.public_key(), scope, &lab.certificates, &[], 1)
        .unwrap();
    assert_eq!(store.active_policy(2).unwrap(), Some(installed));
    drop(store);
    let reopened = files.open("policy.db", 1);
    assert_eq!(reopened.active_policy(2).unwrap(), Some(installed));
}

#[test]
fn same_epoch_policy_replacement_is_rejected_without_changing_the_snapshot() {
    let files = Files::new();
    let lab = Lab::new();
    let mut store = files.open("policy-conflict.db", 1);
    let scope = lab.roster.scope();
    let installed = store
        .install_policy(lab.authority.public_key(), scope, &lab.certificates, &[], 1)
        .unwrap();

    let mut replacement = lab.certificates.clone();
    replacement[0] = protocol::issue_certificate(
        &lab.authority,
        &CertificateClaims {
            group: scope.group,
            member: MemberId([1; 32]),
            signing_key: lab.signers[0].public_key(),
            delivery_key: lab.secrets[0].public_key(),
            valid_from: 0,
            valid_until: 1000,
            epoch: scope.epoch,
            serial: 99,
        },
    )
    .unwrap();

    assert!(store
        .install_policy(lab.authority.public_key(), scope, &replacement, &[], 1)
        .is_err());
    assert_eq!(store.active_policy(2).unwrap(), Some(installed));
}

#[test]
fn signed_authority_handoff_allows_only_the_immediate_successor_epoch() {
    let files = Files::new();
    let lab = Lab::new();
    let mut store = files.open("policy-handoff.db", 1);
    let prior = store
        .install_policy(
            lab.authority.public_key(),
            lab.roster.scope(),
            &lab.certificates,
            &[],
            1,
        )
        .unwrap();
    let successor = IdentitySigningKey::import(Zeroizing::new([99; 32]));
    let handoff = protocol::issue_authority_handoff(
        &lab.authority,
        prior.scope,
        prior.roster_digest,
        successor.public_key(),
        100,
    )
    .unwrap();
    let next_scope = Scope {
        group: prior.scope.group,
        epoch: prior.scope.epoch + 1,
    };
    let certificates: Vec<_> = (0..lab.signers.len())
        .map(|index| {
            protocol::issue_certificate(
                &successor,
                &CertificateClaims {
                    group: next_scope.group,
                    member: MemberId([index as u8 + 1; 32]),
                    signing_key: lab.signers[index].public_key(),
                    delivery_key: lab.secrets[index].public_key(),
                    valid_from: 2,
                    valid_until: 1000,
                    epoch: next_scope.epoch,
                    serial: index as u64 + 1,
                },
            )
            .unwrap()
        })
        .collect();

    // A new signing key alone can never replace the pinned authority.
    assert!(store
        .install_policy(successor.public_key(), next_scope, &certificates, &[], 2,)
        .is_err());
    let rotated = store
        .install_rotated_policy(
            successor.public_key(),
            next_scope,
            &certificates,
            &[],
            &handoff,
            2,
        )
        .unwrap();
    assert_eq!(rotated.authority, successor.public_key());
    assert_eq!(rotated.scope, next_scope);
}

#[test]
fn policy_cannot_be_installed_on_a_device_missing_from_the_roster() {
    let files = Files::new();
    let lab = Lab::new();
    let mut store = files.open("policy-member.db", 9);
    assert!(store
        .install_policy(
            lab.authority.public_key(),
            lab.roster.scope(),
            &lab.certificates,
            &[],
            1,
        )
        .is_err());
    assert_eq!(store.active_policy(2).unwrap(), None);
}
