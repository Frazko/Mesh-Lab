use mesh_object::{digest, ObjectPolicy, PreparedObject};
use mesh_replication::{RelayFrame, RelayId};
use mesh_store::{Limits, Store};
use mesh_types::durable::*;
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use zeroize::Zeroizing;
static SERIAL: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "mesh-store-test-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn path(&self) -> PathBuf {
        self.0.join("store.db")
    }
    fn open(&self, member: u8) -> Store {
        open(&self.path(), member, Limits::default())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn open(path: &Path, member: u8, limits: Limits) -> Store {
    Store::open(
        path,
        Zeroizing::new([7; 32]),
        MemberId([member; 32]),
        limits,
    )
    .unwrap()
}
fn object(seq: u64, bytes: &[u8]) -> PreparedObject {
    PreparedObject::from_opaque(
        MemberId([1; 32]),
        seq,
        ObjectPolicy {
            namespace: Namespace::new("mesh.lab.synthetic.v1").unwrap(),
            epoch: 1,
            targets: vec![MemberId([2; 32])],
            expires_at: 1000,
            hop_limit: 4,
        },
        bytes,
    )
    .unwrap()
}
const OP: OperationId = OperationId([1; 16]);
fn outgoing(store: &mut Store) -> PreparedObject {
    let seq = store.reserve(OP, digest(b"command")).unwrap().sequence;
    let o = object(seq, &vec![42; 2500]);
    store
        .commit_outgoing(OP, digest(b"command"), &o, 1)
        .unwrap();
    o
}

#[test]
fn outgoing_restart_idempotency_and_source_retention() {
    let f = Fixture::new();
    let mut s = f.open(1);
    let o = outgoing(&mut s);
    let id = o.manifest().id();
    drop(s);
    let mut s = f.open(1);
    let reservation = s.reserve(OP, digest(b"command")).unwrap();
    assert_eq!(reservation.sequence, 1);
    assert_eq!(reservation.object_id, Some(id));
    assert_eq!(
        s.commit_outgoing(OP, digest(b"command"), &o, 1001).unwrap(),
        id
    );
    assert_eq!(
        s.reserve(OP, digest(b"changed")),
        Err(DurableError::Conflict)
    );
    assert_eq!(
        s.commit_outgoing(OP, digest(b"command"), &object(1, b"different"), 1),
        Err(DurableError::Conflict)
    );
    assert_eq!(
        s.reserve(OperationId([2; 16]), [0; 32]).unwrap().sequence,
        2
    );
    assert_eq!(s.outbox(1).unwrap(), vec![id]);
    assert!(s.outbox(1000).unwrap().is_empty());
    assert_eq!(s.chunk(id, 0).unwrap(), vec![42; 1024]);
    assert_eq!(s.stats().unwrap().objects, 1);
    assert_eq!(s.stats().unwrap().outbox, 1);
}
#[test]
fn partial_resume_lost_ack_and_no_duplicate_delivery() {
    let f = Fixture::new();
    let o = object(1, &vec![42; 2500]);
    let id = o.manifest().id();
    let mut s = f.open(2);
    s.announce(o.manifest(), 1).unwrap();
    assert!(s.put_chunk(id, 1, &o.chunks()[1], 2).unwrap().is_none());
    assert!(s.local_commit(id).unwrap().is_none());
    drop(s);
    let mut s = f.open(2);
    assert_eq!(s.missing(id).unwrap(), vec![0, 2]);
    s.announce(o.manifest(), 3).unwrap();
    s.put_chunk(id, 0, &o.chunks()[0], 4).unwrap();
    let commit = s.put_chunk(id, 2, &o.chunks()[2], 5).unwrap().unwrap();
    drop(s);
    let mut s = f.open(2);
    assert_eq!(s.local_commit(id).unwrap(), Some(commit));
    assert!(s.missing(id).unwrap().is_empty());
    for (index, bytes) in o.chunks().iter().enumerate() {
        assert_eq!(s.put_chunk(id, index, bytes, 6).unwrap(), Some(commit));
    }
    let stats = s.stats().unwrap();
    assert_eq!(stats.local_deliveries, 1);
    assert_eq!(stats.chunks, 3);
    assert_eq!(stats.bytes_reserved, 2500);
}
#[test]
fn corruption_conflict_expiry_and_quota_leave_state_unchanged() {
    let f = Fixture::new();
    let o = object(1, &vec![42; 2500]);
    let id = o.manifest().id();
    let mut s = open(
        &f.path(),
        2,
        Limits {
            objects: 1,
            bytes: 2500,
            operations: 1,
        },
    );
    s.announce(o.manifest(), 1).unwrap();
    let before = s.stats().unwrap();
    assert_eq!(
        s.put_chunk(id, 0, &vec![41; 1024], 2),
        Err(DurableError::Corrupt)
    );
    assert_eq!(
        s.put_chunk(id, 64, b"x", 2),
        Err(DurableError::InvalidInput)
    );
    assert_eq!(
        s.put_chunk(id, 0, &o.chunks()[0], 1000),
        Err(DurableError::Expired)
    );
    assert_eq!(
        s.announce(object(1, b"equivocation").manifest(), 2),
        Err(DurableError::Conflict)
    );
    assert_eq!(
        s.announce(object(2, b"quota").manifest(), 2),
        Err(DurableError::ResourcePressure)
    );
    assert_eq!(before, s.stats().unwrap());
    s.reserve(OP, [0; 32]).unwrap();
    assert_eq!(
        s.reserve(OperationId([2; 16]), [0; 32]),
        Err(DurableError::ResourcePressure)
    );
    let small = Fixture::new();
    let mut small = open(
        &small.path(),
        2,
        Limits {
            bytes: 2499,
            ..Limits::default()
        },
    );
    assert_eq!(
        small.announce(o.manifest(), 1),
        Err(DurableError::ResourcePressure)
    );
    assert_eq!(small.stats().unwrap().objects, 0);
}
#[test]
fn encrypted_database_and_wal_reject_wrong_or_missing_key() {
    let f = Fixture::new();
    let mut s = f.open(1);
    let marker = b"SYNTHETIC_SECRET_NOT_PERSONAL_123456789";
    let seq = s.reserve(OP, [0; 32]).unwrap().sequence;
    let o = object(seq, marker);
    s.commit_outgoing(OP, [0; 32], &o, 1).unwrap();
    for p in [f.path(), f.0.join("store.db-wal")] {
        let bytes = std::fs::read(p).unwrap();
        assert!(!bytes.windows(marker.len()).any(|w| w == marker));
        assert!(!bytes
            .windows(b"mesh.lab.synthetic.v1".len())
            .any(|w| w == b"mesh.lab.synthetic.v1"));
    }
    let header = std::fs::read(f.path()).unwrap();
    assert_ne!(&header[..16], b"SQLite format 3\0");
    assert!(matches!(
        Store::open(
            &f.path(),
            Zeroizing::new([8; 32]),
            MemberId([1; 32]),
            Limits::default()
        ),
        Err(DurableError::KeyOrCorruption)
    ));
    let plain = rusqlite::Connection::open(f.path()).unwrap();
    assert!(plain
        .query_row("SELECT count(*) FROM sqlite_master", [], |r| r
            .get::<_, i64>(0))
        .is_err());
    drop(plain);
    drop(s);
    let s = f.open(1);
    assert_eq!(s.chunk(o.manifest().id(), 0).unwrap(), marker);
    assert!(matches!(
        Store::open(
            &f.path(),
            Zeroizing::new([7; 32]),
            MemberId([9; 32]),
            Limits::default()
        ),
        Err(DurableError::Conflict)
    ));
}
#[test]
fn non_target_stores_without_claiming_delivery() {
    let f = Fixture::new();
    let mut s = f.open(3);
    let o = object(1, b"relay");
    let id = s.announce(o.manifest(), 1).unwrap();
    assert!(s.put_chunk(id, 0, &o.chunks()[0], 2).unwrap().is_none());
    assert!(s.local_commit(id).unwrap().is_none());
    assert!(s.missing(id).unwrap().is_empty());
}

#[test]
fn completed_remote_object_has_relay_custody_across_restart_without_receipt_claim() {
    let f = Fixture::new();
    let o = object(1, &vec![19; 2500]);
    let id = o.manifest().id();
    let mut relay = f.open(3);
    relay.announce(o.manifest(), 1).unwrap();
    for (index, bytes) in o.chunks().iter().enumerate() {
        assert!(relay.put_chunk(id, index, bytes, 2).unwrap().is_none());
    }
    assert_eq!(relay.relay_outbox(2).unwrap(), vec![id]);
    assert!(relay.outbox(2).unwrap().is_empty());
    assert!(relay.local_commit(id).unwrap().is_none());
    assert_eq!(relay.stats().unwrap().relay_outbox, 1);
    drop(relay);

    let relay = f.open(3);
    assert_eq!(relay.relay_outbox(3).unwrap(), vec![id]);
    assert!(relay.local_commit(id).unwrap().is_none());
    assert!(relay.relay_outbox(1000).unwrap().is_empty());
}

#[test]
fn relay_rejects_synthetic_objects_before_any_custody_is_created() {
    let f = Fixture::new();
    let o = object(1, &vec![23; 2500]);
    let id = o.manifest().id();
    let frame = RelayFrame {
        id: RelayId([8; 16]),
        origin: MemberId([1; 32]),
        previous_hop: MemberId([2; 32]),
        hops: 1,
        hop_limit: 4,
        expires_at: 999,
    };
    let mut relay = f.open(3);
    relay.announce(o.manifest(), 1).unwrap();
    assert_eq!(
        relay.put_relay_chunk(id, 0, &o.chunks()[0], frame, MemberId([2; 32]), 2),
        Err(DurableError::AuthenticationFailed)
    );
    assert!(relay.relay_queue(2).unwrap().is_empty());
    assert!(relay.relay_outbox(2).unwrap().is_empty());
}

#[cfg(feature = "fault-injection")]
mod crashes {
    use super::*;
    use mesh_store::Stage;
    fn hook(stage: Stage) {
        if format!("{stage:?}") == std::env::var("MESH_TEST_CRASH_STAGE").unwrap() {
            std::process::exit(73)
        }
    }
    #[test]
    fn crash_child() {
        let Ok(path) = std::env::var("MESH_TEST_CRASH_PATH") else {
            return;
        };
        let receiver = std::env::var("MESH_TEST_RECEIVE").is_ok();
        let mut s = open(
            Path::new(&path),
            if receiver { 2 } else { 1 },
            Limits::default(),
        );
        s.set_fault_hook(hook);
        if receiver {
            let o = object(1, &vec![42; 2500]);
            s.put_chunk(o.manifest().id(), 2, &o.chunks()[2], 8)
                .unwrap();
        } else {
            outgoing(&mut s);
        }
        panic!("fault point was not reached");
    }
    fn crash(f: &Fixture, stage: &str, receiver: bool) {
        let mut cmd = std::process::Command::new(std::env::current_exe().unwrap());
        cmd.args(["--exact", "crashes::crash_child", "--nocapture"])
            .env("MESH_TEST_CRASH_PATH", f.path())
            .env("MESH_TEST_CRASH_STAGE", stage);
        if receiver {
            cmd.env("MESH_TEST_RECEIVE", "1");
        } else {
            cmd.env_remove("MESH_TEST_RECEIVE");
        }
        let output = cmd.output().unwrap();
        assert_eq!(
            output.status.code(),
            Some(73),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    #[test]
    fn outgoing_process_death_is_atomic_at_every_boundary() {
        for stage in [
            "Reserved",
            "ObjectInserted",
            "ChunkInserted",
            "BeforeCommit",
            "AfterCommit",
        ] {
            let f = Fixture::new();
            crash(&f, stage, false);
            let mut s = f.open(1);
            let stats = s.stats().unwrap();
            let committed = stage == "AfterCommit";
            assert_eq!(stats.operations, 1, "{stage}");
            assert_eq!(stats.objects, usize::from(committed), "{stage}");
            assert_eq!(stats.outbox, usize::from(committed), "{stage}");
            assert_eq!(stats.chunks, if committed { 3 } else { 0 }, "{stage}");
            let o = outgoing(&mut s);
            assert_eq!(o.manifest().sequence(), 1);
            assert_eq!(s.stats().unwrap().objects, 1);
        }
    }
    #[test]
    fn receiver_process_death_never_acknowledges_partial_message() {
        for stage in ["ChunkInserted", "BeforeCommit", "AfterCommit"] {
            let f = Fixture::new();
            let mut s = f.open(2);
            let o = object(1, &vec![42; 2500]);
            let id = s.announce(o.manifest(), 1).unwrap();
            for i in 0..2 {
                s.put_chunk(id, i, &o.chunks()[i], 2).unwrap();
            }
            drop(s);
            crash(&f, stage, true);
            let mut s = f.open(2);
            let committed = stage == "AfterCommit";
            assert_eq!(s.local_commit(id).unwrap().is_some(), committed, "{stage}");
            assert_eq!(s.stats().unwrap().chunks, if committed { 3 } else { 2 });
            let ack = s.put_chunk(id, 2, &o.chunks()[2], 10).unwrap().unwrap();
            assert_eq!(ack.committed_at(), if committed { 8 } else { 10 });
            assert_eq!(s.stats().unwrap().local_deliveries, 1);
        }
    }
}

#[test]
fn unknown_schema_and_modified_ciphertext_fail_closed() {
    let versioned = Fixture::new();
    drop(versioned.open(1));
    let db = rusqlite::Connection::open(versioned.path()).unwrap();
    db.pragma_update(None, "key", format!("x'{}'", "07".repeat(32)))
        .unwrap();
    db.pragma_update(None, "user_version", 99).unwrap();
    drop(db);
    assert!(matches!(
        Store::open(
            &versioned.path(),
            Zeroizing::new([7; 32]),
            MemberId([1; 32]),
            Limits::default()
        ),
        Err(DurableError::UnsupportedSchema)
    ));
    let corrupt = Fixture::new();
    let mut s = corrupt.open(1);
    outgoing(&mut s);
    drop(s);
    let mut bytes = std::fs::read(corrupt.path()).unwrap();
    bytes[128] ^= 1;
    std::fs::write(corrupt.path(), bytes).unwrap();
    assert!(matches!(
        Store::open(
            &corrupt.path(),
            Zeroizing::new([7; 32]),
            MemberId([1; 32]),
            Limits::default()
        ),
        Err(DurableError::KeyOrCorruption)
    ));
}
#[test]
fn source_in_audience_gets_one_atomic_local_delivery() {
    let f = Fixture::new();
    let mut s = f.open(1);
    let sequence = s.reserve(OP, [0; 32]).unwrap().sequence;
    let o = PreparedObject::from_opaque(
        MemberId([1; 32]),
        sequence,
        ObjectPolicy {
            namespace: Namespace::new("synthetic").unwrap(),
            epoch: 1,
            targets: vec![MemberId([1; 32]), MemberId([2; 32])],
            expires_at: 100,
            hop_limit: 4,
        },
        b"self and peer",
    )
    .unwrap();
    let id = s.commit_outgoing(OP, [0; 32], &o, 1).unwrap();
    drop(s);
    let mut s = f.open(1);
    assert_eq!(s.local_commit(id).unwrap().unwrap().committed_at(), 1);
    s.commit_outgoing(OP, [0; 32], &o, 2).unwrap();
    assert_eq!(s.stats().unwrap().local_deliveries, 1);
}
