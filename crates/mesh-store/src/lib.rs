//! Encrypted, host-only durable storage. Not wired to the mobile runtime yet.
//! Use authenticated ingestion for protected objects; legacy methods are synthetic-only.
//! LocalCommit is local evidence of persistence, NEVER a network delivery receipt.
use mesh_crypto::Scope;
use mesh_object::{digest, Manifest, PreparedObject};
use mesh_protocol::{AuthorityHandoff, PolicyBundle, VerifiedRoster};
use mesh_replication::{RelayFrame, RelayId};
use mesh_types::durable::*;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use std::{path::Path, time::Duration};
use zeroize::Zeroizing;

const SCHEMA: &str = include_str!("../../../schema/store/001.sql");
const AUTH_SCHEMA: &str = include_str!("../../../schema/store/002-authentication.sql");
const POLICY_SCHEMA: &str = include_str!("../../../schema/store/003-policy.sql");
const GROUP_CAPACITY_SCHEMA: &str = include_str!("../../../schema/store/004-group-capacity.sql");
const RELAY_CUSTODY_SCHEMA: &str = include_str!("../../../schema/store/005-relay-custody.sql");
const RELAY_INGRESS_SCHEMA: &str = include_str!("../../../schema/store/006-relay-ingress.sql");
const RELAY_RECEIPT_SCHEMA: &str = include_str!("../../../schema/store/007-relay-receipts.sql");
const RECEIPT_ACK_SCHEMA: &str = include_str!("../../../schema/store/008-receipt-acks.sql");
const RELAY_RECEIPT_ACK_SCHEMA: &str =
    include_str!("../../../schema/store/009-relay-receipt-acks.sql");
const LOGICAL_DELIVERY_SCHEMA: &str =
    include_str!("../../../schema/store/010-logical-delivery.sql");
type PolicyRow = ([u8; 32], [u8; 32], u64, [u8; 32]);
type RelayMetadataRow = (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, u8, u8, u64);
fn policy_schema_hash() -> [u8; 32] {
    digest(
        [
            SCHEMA.as_bytes(),
            AUTH_SCHEMA.as_bytes(),
            POLICY_SCHEMA.as_bytes(),
        ]
        .concat()
        .as_slice(),
    )
}
fn group_capacity_schema_hash() -> [u8; 32] {
    digest(
        [
            SCHEMA.as_bytes(),
            AUTH_SCHEMA.as_bytes(),
            POLICY_SCHEMA.as_bytes(),
            GROUP_CAPACITY_SCHEMA.as_bytes(),
        ]
        .concat()
        .as_slice(),
    )
}
fn relay_custody_schema_hash() -> [u8; 32] {
    digest(
        [
            SCHEMA.as_bytes(),
            AUTH_SCHEMA.as_bytes(),
            POLICY_SCHEMA.as_bytes(),
            GROUP_CAPACITY_SCHEMA.as_bytes(),
            RELAY_CUSTODY_SCHEMA.as_bytes(),
        ]
        .concat()
        .as_slice(),
    )
}
fn schema_hash() -> [u8; 32] {
    digest(
        [
            SCHEMA.as_bytes(),
            AUTH_SCHEMA.as_bytes(),
            POLICY_SCHEMA.as_bytes(),
            GROUP_CAPACITY_SCHEMA.as_bytes(),
            RELAY_CUSTODY_SCHEMA.as_bytes(),
            RELAY_INGRESS_SCHEMA.as_bytes(),
        ]
        .concat()
        .as_slice(),
    )
}
fn relay_receipt_schema_hash() -> [u8; 32] {
    digest(
        [
            SCHEMA.as_bytes(),
            AUTH_SCHEMA.as_bytes(),
            POLICY_SCHEMA.as_bytes(),
            GROUP_CAPACITY_SCHEMA.as_bytes(),
            RELAY_CUSTODY_SCHEMA.as_bytes(),
            RELAY_INGRESS_SCHEMA.as_bytes(),
            RELAY_RECEIPT_SCHEMA.as_bytes(),
        ]
        .concat()
        .as_slice(),
    )
}
fn receipt_ack_schema_hash() -> [u8; 32] {
    digest(
        [
            SCHEMA.as_bytes(),
            AUTH_SCHEMA.as_bytes(),
            POLICY_SCHEMA.as_bytes(),
            GROUP_CAPACITY_SCHEMA.as_bytes(),
            RELAY_CUSTODY_SCHEMA.as_bytes(),
            RELAY_INGRESS_SCHEMA.as_bytes(),
            RELAY_RECEIPT_SCHEMA.as_bytes(),
            RECEIPT_ACK_SCHEMA.as_bytes(),
        ]
        .concat()
        .as_slice(),
    )
}
fn relay_receipt_ack_schema_hash() -> [u8; 32] {
    digest(
        [
            SCHEMA.as_bytes(),
            AUTH_SCHEMA.as_bytes(),
            POLICY_SCHEMA.as_bytes(),
            GROUP_CAPACITY_SCHEMA.as_bytes(),
            RELAY_CUSTODY_SCHEMA.as_bytes(),
            RELAY_INGRESS_SCHEMA.as_bytes(),
            RELAY_RECEIPT_SCHEMA.as_bytes(),
            RECEIPT_ACK_SCHEMA.as_bytes(),
            RELAY_RECEIPT_ACK_SCHEMA.as_bytes(),
        ]
        .concat()
        .as_slice(),
    )
}
fn logical_delivery_schema_hash() -> [u8; 32] {
    digest(
        [
            SCHEMA.as_bytes(),
            AUTH_SCHEMA.as_bytes(),
            POLICY_SCHEMA.as_bytes(),
            GROUP_CAPACITY_SCHEMA.as_bytes(),
            RELAY_CUSTODY_SCHEMA.as_bytes(),
            RELAY_INGRESS_SCHEMA.as_bytes(),
            RELAY_RECEIPT_SCHEMA.as_bytes(),
            RECEIPT_ACK_SCHEMA.as_bytes(),
            RELAY_RECEIPT_ACK_SCHEMA.as_bytes(),
            LOGICAL_DELIVERY_SCHEMA.as_bytes(),
        ]
        .concat()
        .as_slice(),
    )
}
mod authenticated;
pub use authenticated::{ChunkStored, ReceiptCommit};
#[derive(Clone, Copy)]
pub struct Limits {
    pub objects: usize,
    pub bytes: usize,
    pub operations: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            // One active 50-phone field group can retain several bounded
            // audiences per action. Keep enough encrypted headroom for the
            // live expiry window; [Store::prune_expired] still enforces a hard
            // upper bound and removes records once they can no longer route.
            objects: 4096,
            bytes: 64 * 1024 * 1024,
            operations: 8192,
        }
    }
}
pub struct Store {
    db: Connection,
    member: MemberId,
    limits: Limits,
    #[cfg(feature = "fault-injection")]
    hook: Option<fn(Stage)>,
}
impl Store {
    /// The public member identity bound to this encrypted store. Hosts use it
    /// only to derive deterministic radio-neighbor candidates from a verified
    /// policy; it is never a secret or a Flutter-visible database row.
    pub fn local_member(&self) -> MemberId {
        self.member
    }

    /// Deletes objects whose authenticated manifest can no longer be routed.
    ///
    /// The cleanup is transactional and runs only while the native store
    /// registry holds exclusive access. It also removes abandoned operation
    /// reservations and logical-message shells left by an interrupted enqueue,
    /// so a process restart cannot permanently exhaust a healthy field group.
    pub fn prune_expired(&mut self, now: u64) -> Result<usize> {
        clock(now)?;
        let expired = {
            let mut statement = self
                .db
                .prepare("SELECT id,manifest FROM objects")
                .map_err(sql)?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?))
                })
                .map_err(sql)?;
            let mut ids = Vec::new();
            for row in rows {
                let (id, encoded) = row.map_err(sql)?;
                let object = oid(id)?;
                let manifest = Manifest::decode(&encoded).map_err(|_| DurableError::Corrupt)?;
                if manifest.id() != object {
                    return Err(DurableError::Corrupt);
                }
                if manifest.expires_at() <= now {
                    ids.push(object);
                }
            }
            ids
        };
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        for object in &expired {
            delete_object(&tx, *object)?;
        }
        tx.execute(
            "DELETE FROM logical_messages WHERE expires_at<=?1 OR NOT EXISTS(SELECT 1 FROM logical_message_objects WHERE logical_id=logical_messages.id)",
            [now as i64],
        )
        .map_err(sql)?;
        tx.execute("DELETE FROM operations WHERE object_id IS NULL", [])
            .map_err(sql)?;
        tx.execute(
            "DELETE FROM relay_receipt_outbox WHERE expires_at<=?1",
            [now as i64],
        )
        .map_err(sql)?;
        tx.execute(
            "DELETE FROM relay_receipt_ack_outbox WHERE expires_at<=?1",
            [now as i64],
        )
        .map_err(sql)?;
        tx.commit().map_err(sql)?;
        Ok(expired.len())
    }

    /// Rolls back one enqueue which never became visible to a radio. The host
    /// calls the radio drain only after the whole logical action succeeds, so
    /// these objects cannot have left this store yet.
    pub fn rollback_logical_message(&mut self, logical: LogicalMessageId) -> Result<()> {
        let objects = self
            .db
            .prepare("SELECT object_id FROM logical_message_objects WHERE logical_id=?1")
            .map_err(sql)?
            .query_map([&logical.0[..]], |row| row.get::<_, Vec<u8>>(0))
            .map_err(sql)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(sql)?
            .into_iter()
            .map(oid)
            .collect::<Result<Vec<_>>>()?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        for object in objects {
            delete_object(&tx, object)?;
        }
        tx.execute("DELETE FROM logical_messages WHERE id=?1", [&logical.0[..]])
            .map_err(sql)?;
        tx.execute("DELETE FROM operations WHERE object_id IS NULL", [])
            .map_err(sql)?;
        tx.commit().map_err(sql)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reservation {
    pub sequence: u64,
    pub object_id: Option<ObjectId>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalCommit {
    object_id: ObjectId,
    member: MemberId,
    committed_at: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RelayCustody {
    pub object_id: ObjectId,
    pub frame: RelayFrame,
    /// Neighbor that supplied this object. It is distinct from the forwarded
    /// frame's `previous_hop`, which is this store's local member.
    pub received_from: MemberId,
    pub custodied_at: u64,
}
/// A receipt is distinct from its object custody: a relay can safely retain
/// this signed proof even when it never stored the source object's chunks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelayReceiptCustody {
    pub receipt: Vec<u8>,
    pub frame: RelayFrame,
    pub received_from: MemberId,
    pub custodied_at: u64,
}
/// Durable relay custody for a verified origin ACK. It is intentionally a
/// distinct table because it closes receipt retry work rather than delivering
/// an object or proving a recipient action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelayReceiptAckCustody {
    pub ack: Vec<u8>,
    pub frame: RelayFrame,
    pub received_from: MemberId,
    pub custodied_at: u64,
}
/// Opaque origin-local identity for one user-visible action. It is unrelated
/// to the content and does not travel over a radio.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LogicalMessageId(pub [u8; 16]);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogicalDeliveryState {
    Queued,
    PartiallyDelivered,
    Delivered,
    Expired,
}

/// Aggregate source-side evidence for every bounded audience of one logical
/// group action. `delivered_targets` only counts verified recipient receipts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LogicalDeliverySummary {
    pub id: LogicalMessageId,
    pub target_count: usize,
    pub audience_count: usize,
    pub committed_audiences: usize,
    pub delivered_targets: usize,
    pub state: LogicalDeliveryState,
}
impl LocalCommit {
    pub fn object_id(&self) -> ObjectId {
        self.object_id
    }
    pub fn member(&self) -> MemberId {
        self.member
    }
    pub fn committed_at(&self) -> u64 {
        self.committed_at
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub objects: usize,
    pub chunks: usize,
    pub bytes_reserved: usize,
    pub operations: usize,
    pub outbox: usize,
    pub relay_outbox: usize,
    pub local_deliveries: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PolicySnapshot {
    pub authority: [u8; 32],
    pub scope: Scope,
    pub roster_digest: [u8; 32],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Reserved,
    ObjectInserted,
    ChunkInserted,
    BeforeCommit,
    AfterCommit,
    DeliveryRecorded,
    ReceiptRecorded,
}

fn time_column(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let n: i64 = row.get(index)?;
    u64::try_from(n).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, n))
}
fn size_column(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<usize> {
    let n: i64 = row.get(index)?;
    usize::try_from(n).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, n))
}
fn sql(_: rusqlite::Error) -> DurableError {
    DurableError::StorageUnavailable
}
fn clock(now: u64) -> Result<()> {
    if now > MAX_LOGICAL_TIME {
        Err(DurableError::InvalidInput)
    } else {
        Ok(())
    }
}
fn live(m: &Manifest, now: u64) -> Result<()> {
    clock(now)?;
    if now >= m.expires_at() {
        Err(DurableError::Expired)
    } else {
        Ok(())
    }
}
fn oid(bytes: Vec<u8>) -> Result<ObjectId> {
    Ok(ObjectId(
        bytes.try_into().map_err(|_| DurableError::Corrupt)?,
    ))
}

impl Store {
    /// Host supplies the raw 256-bit SQLCipher key; never generated, saved or logged here.
    pub fn open(
        path: &Path,
        key: Zeroizing<[u8; 32]>,
        member: MemberId,
        limits: Limits,
    ) -> Result<Self> {
        if !(1..=4096).contains(&limits.objects)
            || !(1..=128 * 1024 * 1024).contains(&limits.bytes)
            || !(1..=8192).contains(&limits.operations)
        {
            return Err(DurableError::InvalidInput);
        }
        let mut db = Connection::open(path).map_err(sql)?;
        let version: String = db
            .query_row("PRAGMA cipher_version", [], |r| r.get(0))
            .map_err(|_| DurableError::CipherUnavailable)?;
        if version.is_empty() {
            return Err(DurableError::CipherUnavailable);
        }
        let mut raw = Zeroizing::new(String::with_capacity(67));
        raw.push_str("x'");
        use std::fmt::Write;
        for b in key.iter() {
            write!(&mut *raw, "{b:02x}").map_err(|_| DurableError::InvalidInput)?;
        }
        raw.push('\'');
        db.pragma_update(None, "key", &*raw).map_err(sql)?;
        drop(raw);
        drop(key);
        db.busy_timeout(Duration::from_millis(250)).map_err(sql)?;
        let schema: i64 = db
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(|_| DurableError::KeyOrCorruption)?;
        if !(0..=10).contains(&schema) {
            return Err(DurableError::UnsupportedSchema);
        }
        db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA temp_store=MEMORY; PRAGMA synchronous=FULL; PRAGMA cipher_memory_security=ON;").map_err(sql)?;
        let mode: String = db
            .query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))
            .map_err(sql)?;
        if mode != "wal" {
            return Err(DurableError::StorageUnavailable);
        }
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let schema: i64 = tx
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(sql)?;
        if schema == 0 {
            let count: i64 = tx
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table'",
                    [],
                    |r| r.get(0),
                )
                .map_err(sql)?;
            if count != 0 {
                return Err(DurableError::UnsupportedSchema);
            }
            tx.execute_batch(SCHEMA).map_err(sql)?;
            tx.execute(
                "INSERT INTO meta VALUES(1,?1,1,?2)",
                params![&member.0[..], &digest(SCHEMA.as_bytes())[..]],
            )
            .map_err(sql)?;
        } else if !(1..=10).contains(&schema) {
            return Err(DurableError::UnsupportedSchema);
        }
        let (stored_member, hash): (Vec<u8>, Vec<u8>) = tx
            .query_row(
                "SELECT origin,schema_hash FROM meta WHERE singleton=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| DurableError::Corrupt)?;
        let expected_hash = match schema {
            0 | 1 => digest(SCHEMA.as_bytes()),
            2 => digest(
                [SCHEMA.as_bytes(), AUTH_SCHEMA.as_bytes()]
                    .concat()
                    .as_slice(),
            ),
            3 => policy_schema_hash(),
            4 => group_capacity_schema_hash(),
            5 => relay_custody_schema_hash(),
            6 => schema_hash(),
            7 => relay_receipt_schema_hash(),
            8 => receipt_ack_schema_hash(),
            9 => relay_receipt_ack_schema_hash(),
            10 => logical_delivery_schema_hash(),
            _ => return Err(DurableError::UnsupportedSchema),
        };
        if stored_member != member.0 || hash != expected_hash {
            return Err(DurableError::Conflict);
        }
        if schema <= 1 {
            tx.execute_batch(AUTH_SCHEMA).map_err(sql)?;
            tx.execute(
                "UPDATE meta SET schema_hash=?1 WHERE singleton=1",
                [&digest(
                    [SCHEMA.as_bytes(), AUTH_SCHEMA.as_bytes()]
                        .concat()
                        .as_slice(),
                )[..]],
            )
            .map_err(sql)?;
        }
        if schema <= 2 {
            tx.execute_batch(POLICY_SCHEMA).map_err(sql)?;
            tx.execute(
                "UPDATE meta SET schema_hash=?1 WHERE singleton=1",
                [&policy_schema_hash()[..]],
            )
            .map_err(sql)?;
        }
        if schema <= 3 {
            tx.execute_batch(GROUP_CAPACITY_SCHEMA).map_err(sql)?;
            tx.execute(
                "UPDATE meta SET schema_hash=?1 WHERE singleton=1",
                [&group_capacity_schema_hash()[..]],
            )
            .map_err(sql)?;
        }
        if schema <= 4 {
            tx.execute_batch(RELAY_CUSTODY_SCHEMA).map_err(sql)?;
            tx.execute(
                "UPDATE meta SET schema_hash=?1 WHERE singleton=1",
                [&relay_custody_schema_hash()[..]],
            )
            .map_err(sql)?;
        }
        if schema <= 5 {
            tx.execute_batch(RELAY_INGRESS_SCHEMA).map_err(sql)?;
            tx.execute(
                "UPDATE meta SET schema_hash=?1 WHERE singleton=1",
                [&schema_hash()[..]],
            )
            .map_err(sql)?;
        }
        if schema <= 6 {
            tx.execute_batch(RELAY_RECEIPT_SCHEMA).map_err(sql)?;
            tx.execute(
                "UPDATE meta SET schema_hash=?1 WHERE singleton=1",
                [&relay_receipt_schema_hash()[..]],
            )
            .map_err(sql)?;
        }
        if schema <= 7 {
            tx.execute_batch(RECEIPT_ACK_SCHEMA).map_err(sql)?;
            tx.execute(
                "UPDATE meta SET schema_hash=?1 WHERE singleton=1",
                [&receipt_ack_schema_hash()[..]],
            )
            .map_err(sql)?;
        }
        if schema <= 8 {
            tx.execute_batch(RELAY_RECEIPT_ACK_SCHEMA).map_err(sql)?;
            tx.execute(
                "UPDATE meta SET schema_hash=?1 WHERE singleton=1",
                [&relay_receipt_ack_schema_hash()[..]],
            )
            .map_err(sql)?;
        }
        if schema <= 9 {
            tx.execute_batch(LOGICAL_DELIVERY_SCHEMA).map_err(sql)?;
            tx.execute(
                "UPDATE meta SET schema_hash=?1 WHERE singleton=1",
                [&logical_delivery_schema_hash()[..]],
            )
            .map_err(sql)?;
        }
        tx.commit().map_err(sql)?;
        Ok(Self {
            db,
            member,
            limits,
            #[cfg(feature = "fault-injection")]
            hook: None,
        })
    }
    #[cfg(feature = "fault-injection")]
    pub fn set_fault_hook(&mut self, hook: fn(Stage)) {
        self.hook = Some(hook)
    }
    fn observer(&self) -> impl Fn(Stage) + use<> {
        #[cfg(feature = "fault-injection")]
        let hook = self.hook;
        move |_stage| {
            #[cfg(feature = "fault-injection")]
            if let Some(h) = hook {
                h(_stage)
            }
        }
    }
    /// Read durable idempotency without allocating an operation or sequence.
    pub fn lookup_operation(
        &self,
        id: OperationId,
        command_hash: [u8; 32],
    ) -> Result<Option<Reservation>> {
        let previous: Option<(Vec<u8>, u64, Option<Vec<u8>>)> = self
            .db
            .query_row(
                "SELECT command_hash,sequence,object_id FROM operations WHERE id=?1",
                [&id.0[..]],
                |r| Ok((r.get(0)?, time_column(r, 1)?, r.get(2)?)),
            )
            .optional()
            .map_err(sql)?;
        previous
            .map(|(hash, sequence, object)| {
                if hash != command_hash {
                    return Err(DurableError::Conflict);
                }
                Ok(Reservation {
                    sequence,
                    object_id: object.map(oid).transpose()?,
                })
            })
            .transpose()
    }
    pub fn reserve(&mut self, id: OperationId, command_hash: [u8; 32]) -> Result<Reservation> {
        let observe = self.observer();
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let previous: Option<(Vec<u8>, u64, Option<Vec<u8>>)> = tx
            .query_row(
                "SELECT command_hash,sequence,object_id FROM operations WHERE id=?1",
                [&id.0[..]],
                |r| Ok((r.get(0)?, time_column(r, 1)?, r.get(2)?)),
            )
            .optional()
            .map_err(sql)?;
        if let Some((hash, sequence, object)) = previous {
            if hash != command_hash {
                return Err(DurableError::Conflict);
            }
            return Ok(Reservation {
                sequence,
                object_id: object.map(oid).transpose()?,
            });
        }
        let count: usize = tx
            .query_row("SELECT count(*) FROM operations", [], |r| size_column(r, 0))
            .map_err(sql)?;
        if count >= self.limits.operations {
            return Err(DurableError::ResourcePressure);
        }
        let sequence: u64 = tx
            .query_row(
                "SELECT next_sequence FROM meta WHERE singleton=1",
                [],
                |r| time_column(r, 0),
            )
            .map_err(sql)?;
        if sequence >= MAX_LOGICAL_TIME {
            return Err(DurableError::ResourcePressure);
        }
        tx.execute(
            "UPDATE meta SET next_sequence=next_sequence+1 WHERE singleton=1",
            [],
        )
        .map_err(sql)?;
        tx.execute(
            "INSERT INTO operations(id,command_hash,sequence) VALUES(?1,?2,?3)",
            params![&id.0[..], &command_hash[..], sequence as i64],
        )
        .map_err(sql)?;
        tx.commit().map_err(sql)?;
        observe(Stage::Reserved);
        Ok(Reservation {
            sequence,
            object_id: None,
        })
    }
    /// Object, chunks, operation result and outbox are committed together.
    pub fn commit_outgoing(
        &mut self,
        id: OperationId,
        command_hash: [u8; 32],
        object: &PreparedObject,
        now: u64,
    ) -> Result<ObjectId> {
        self.commit_outgoing_inner(id, command_hash, object, now, None, None)
    }
    fn commit_outgoing_inner(
        &mut self,
        id: OperationId,
        command_hash: [u8; 32],
        object: &PreparedObject,
        now: u64,
        announcement: Option<&mesh_protocol::AuthenticatedAnnouncement>,
        logical: Option<LogicalMessageId>,
    ) -> Result<ObjectId> {
        let observe = self.observer();
        let m = object.manifest();
        let object_id = m.id();
        clock(now)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let (hash, sequence, previous): (Vec<u8>, u64, Option<Vec<u8>>) = tx
            .query_row(
                "SELECT command_hash,sequence,object_id FROM operations WHERE id=?1",
                [&id.0[..]],
                |r| Ok((r.get(0)?, time_column(r, 1)?, r.get(2)?)),
            )
            .optional()
            .map_err(sql)?
            .ok_or(DurableError::NotFound)?;
        if hash != command_hash || sequence != m.sequence() || m.origin() != self.member {
            return Err(DurableError::Conflict);
        }
        if let Some(previous) = previous {
            authenticated::check_mode(&tx, object_id, announcement.map(|a| a.bytes()))?;
            if let Some(logical) = logical {
                logical_object_matches(&tx, logical, object_id, m.targets().len())?;
            }
            return if oid(previous)? == object_id {
                Ok(object_id)
            } else {
                Err(DurableError::Conflict)
            };
        }
        live(m, now)?;
        authenticated::check_mode(&tx, object_id, announcement.map(|a| a.bytes()))?;
        insert_manifest(&tx, m, self.limits)?;
        if let Some(a) = announcement {
            tx.execute(
                "INSERT OR IGNORE INTO auth_announcements VALUES(?1,?2)",
                params![&object_id.0[..], a.bytes()],
            )
            .map_err(sql)?;
        }
        observe(Stage::ObjectInserted);
        for (i, bytes) in object.chunks().iter().enumerate() {
            m.verify_chunk(i, bytes)?;
            insert_chunk(&tx, object_id, i, bytes)?;
            observe(Stage::ChunkInserted);
        }
        tx.execute(
            "UPDATE objects SET complete=1 WHERE id=?1",
            [&object_id.0[..]],
        )
        .map_err(sql)?;
        tx.execute("INSERT INTO outbox VALUES(?1)", [&object_id.0[..]])
            .map_err(sql)?;
        if let Some(logical) = logical {
            link_logical_object(&tx, logical, object_id, m.targets().len())?;
        }
        if announcement.is_none() && m.targets().contains(&self.member) {
            tx.execute(
                "INSERT OR IGNORE INTO local_deliveries VALUES(?1,?2)",
                params![&object_id.0[..], now as i64],
            )
            .map_err(sql)?;
        }
        tx.execute(
            "UPDATE operations SET object_id=?1 WHERE id=?2",
            params![&object_id.0[..], &id.0[..]],
        )
        .map_err(sql)?;
        observe(Stage::BeforeCommit);
        tx.commit().map_err(sql)?;
        observe(Stage::AfterCommit);
        Ok(object_id)
    }
    /// Only the synthetic host harness currently calls this ingestion boundary.
    pub fn announce(&mut self, manifest: &Manifest, now: u64) -> Result<ObjectId> {
        live(manifest, now)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        authenticated::check_mode(&tx, manifest.id(), None)?;
        insert_manifest(&tx, manifest, self.limits)?;
        tx.commit().map_err(sql)?;
        Ok(manifest.id())
    }
    pub fn put_chunk(
        &mut self,
        id: ObjectId,
        index: usize,
        bytes: &[u8],
        now: u64,
    ) -> Result<Option<LocalCommit>> {
        self.put_chunk_inner(id, index, bytes, now, None)
    }
    /// Stores a chunk received for relay after the authenticated runtime has
    /// accepted its canonical frame. On completion the object, relay custody,
    /// and forwarding metadata commit together.
    pub fn put_relay_chunk(
        &mut self,
        id: ObjectId,
        index: usize,
        bytes: &[u8],
        frame: RelayFrame,
        received_from: MemberId,
        now: u64,
    ) -> Result<Option<LocalCommit>> {
        self.put_chunk_inner(id, index, bytes, now, Some((frame, received_from)))
    }
    fn put_chunk_inner(
        &mut self,
        id: ObjectId,
        index: usize,
        bytes: &[u8],
        now: u64,
        relay_frame: Option<(RelayFrame, MemberId)>,
    ) -> Result<Option<LocalCommit>> {
        let observe = self.observer();
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        if relay_frame.is_some() {
            authenticated::require_authenticated_announcement(&tx, id)?;
        } else {
            authenticated::check_mode(&tx, id, None)?;
        }
        let m = manifest_from(&tx, id)?;
        live(&m, now)?;
        m.verify_chunk(index, bytes)?;
        insert_chunk(&tx, id, index, bytes)?;
        observe(Stage::ChunkInserted);
        let count: usize = tx
            .query_row(
                "SELECT count(*) FROM chunks WHERE object_id=?1",
                [&id.0[..]],
                |r| size_column(r, 0),
            )
            .map_err(sql)?;
        let commit = if count == m.chunk_count() {
            tx.execute("UPDATE objects SET complete=1 WHERE id=?1", [&id.0[..]])
                .map_err(sql)?;
            if m.origin() != self.member {
                tx.execute(
                    "INSERT OR IGNORE INTO relay_outbox VALUES(?1,?2)",
                    params![&id.0[..], now as i64],
                )
                .map_err(sql)?;
                if let Some((frame, received_from)) = relay_frame {
                    if frame.origin != m.origin()
                        || frame.expires_at == 0
                        || frame.expires_at > m.expires_at()
                        || frame.hop_limit > m.hop_limit()
                        || frame.hops > frame.hop_limit
                    {
                        return Err(DurableError::InvalidInput);
                    }
                    let existing: Option<RelayMetadataRow> = tx
                        .query_row(
                            "SELECT relay_id,origin,previous_hop,received_from,hops,hop_limit,expires_at FROM relay_metadata WHERE object_id=?1",
                            [&id.0[..]],
                            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, time_column(row, 6)?)),
                        )
                        .optional()
                        .map_err(sql)?;
                    if let Some((
                        relay_id,
                        origin,
                        previous_hop,
                        prior_received_from,
                        hops,
                        hop_limit,
                        expires_at,
                    )) = existing
                    {
                        let existing = RelayFrame {
                            id: RelayId(relay_id.try_into().map_err(|_| DurableError::Corrupt)?),
                            origin: MemberId(origin.try_into().map_err(|_| DurableError::Corrupt)?),
                            previous_hop: MemberId(
                                previous_hop.try_into().map_err(|_| DurableError::Corrupt)?,
                            ),
                            hops,
                            hop_limit,
                            expires_at,
                        };
                        if existing != frame
                            || prior_received_from.as_slice() != received_from.0.as_slice()
                        {
                            return Err(DurableError::Conflict);
                        }
                    } else {
                        tx.execute(
                            "INSERT INTO relay_metadata (object_id,relay_id,origin,previous_hop,received_from,hops,hop_limit,expires_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                            params![
                                &id.0[..],
                                &frame.id.0[..],
                                &frame.origin.0[..],
                                &frame.previous_hop.0[..],
                                &received_from.0[..],
                                frame.hops as i64,
                                frame.hop_limit as i64,
                                frame.expires_at as i64
                            ],
                        )
                        .map_err(sql)?;
                    }
                }
            }
            // A routed object is only in relay custody at this point. Its
            // chunks may be complete yet still fail recipient decryption, so
            // it must not gain a local-delivery marker until
            // `finalize_received` verifies the sealed envelope and commits a
            // signed receipt. Legacy synthetic ingestion keeps its harness
            // behavior for non-authenticated objects.
            if relay_frame.is_none() && m.targets().contains(&self.member) {
                tx.execute(
                    "INSERT OR IGNORE INTO local_deliveries VALUES(?1,?2)",
                    params![&id.0[..], now as i64],
                )
                .map_err(sql)?;
                let committed_at = tx
                    .query_row(
                        "SELECT committed_at FROM local_deliveries WHERE object_id=?1",
                        [&id.0[..]],
                        |r| time_column(r, 0),
                    )
                    .map_err(sql)?;
                Some(LocalCommit {
                    object_id: id,
                    member: self.member,
                    committed_at,
                })
            } else {
                None
            }
        } else {
            None
        };
        observe(Stage::BeforeCommit);
        tx.commit().map_err(sql)?;
        observe(Stage::AfterCommit);
        Ok(commit)
    }
    pub fn manifest(&self, id: ObjectId) -> Result<Manifest> {
        manifest_from(&self.db, id)
    }
    pub fn chunk(&self, id: ObjectId, index: usize) -> Result<Vec<u8>> {
        let m = self.manifest(id)?;
        if index >= m.chunk_count() {
            return Err(DurableError::InvalidInput);
        }
        let bytes: Vec<u8> = self
            .db
            .query_row(
                "SELECT payload FROM chunks WHERE object_id=?1 AND idx=?2",
                params![&id.0[..], index as i64],
                |r| r.get(0),
            )
            .optional()
            .map_err(sql)?
            .ok_or(DurableError::NotFound)?;
        m.verify_chunk(index, &bytes)?;
        Ok(bytes)
    }
    pub fn missing(&self, id: ObjectId) -> Result<Vec<usize>> {
        let m = self.manifest(id)?;
        let mut missing = Vec::new();
        for index in 0..m.chunk_count() {
            match self.chunk(id, index) {
                Ok(_) => {}
                Err(DurableError::NotFound) => missing.push(index),
                Err(e) => return Err(e),
            }
        }
        Ok(missing)
    }
    pub fn local_commit(&self, id: ObjectId) -> Result<Option<LocalCommit>> {
        let time: Option<u64> = self
            .db
            .query_row(
                "SELECT committed_at FROM local_deliveries WHERE object_id=?1",
                [&id.0[..]],
                |r| time_column(r, 0),
            )
            .optional()
            .map_err(sql)?;
        Ok(time.map(|committed_at| LocalCommit {
            object_id: id,
            member: self.member,
            committed_at,
        }))
    }
    /// Pending source objects; authenticated receipts retire the outbox only after all targets confirm.
    pub fn outbox(&self, now: u64) -> Result<Vec<ObjectId>> {
        clock(now)?;
        let mut stmt = self
            .db
            .prepare(
                "SELECT q.object_id FROM outbox q \
                 LEFT JOIN logical_message_objects lmo ON lmo.object_id=q.object_id \
                 LEFT JOIN logical_messages lm ON lm.id=lmo.logical_id \
                 ORDER BY coalesce(lm.created_at,0) DESC,q.object_id",
            )
            .map_err(sql)?;
        let ids = stmt
            .query_map([], |r| r.get::<_, Vec<u8>>(0))
            .map_err(sql)?;
        let mut result = Vec::new();
        for id in ids {
            let id = oid(id.map_err(sql)?)?;
            if self.manifest(id)?.expires_at() > now {
                result.push(id)
            }
        }
        Ok(result)
    }
    /// Fully persisted objects received from another member. These records are
    /// relay custody only: they neither imply destination delivery nor affect
    /// the origin's receipt-controlled outbox.
    pub fn relay_outbox(&self, now: u64) -> Result<Vec<ObjectId>> {
        clock(now)?;
        let mut stmt = self
            .db
            .prepare("SELECT object_id FROM relay_outbox ORDER BY object_id")
            .map_err(sql)?;
        let ids = stmt
            .query_map([], |r| r.get::<_, Vec<u8>>(0))
            .map_err(sql)?;
        let mut result = Vec::new();
        for id in ids {
            let id = oid(id.map_err(sql)?)?;
            if self.manifest(id)?.expires_at() > now {
                result.push(id);
            }
        }
        Ok(result)
    }
    /// Persistent scheduler input for an authenticated relay. Only rows that
    /// have canonical relay metadata are returned; legacy/synthetic ingestion
    /// may keep opaque custody but cannot cause a production forward.
    pub fn relay_queue(&self, now: u64) -> Result<Vec<RelayCustody>> {
        clock(now)?;
        let mut statement = self
            .db
            .prepare("SELECT q.object_id,q.custodied_at,m.relay_id,m.origin,m.previous_hop,m.received_from,m.hops,m.hop_limit,m.expires_at FROM relay_outbox q JOIN relay_metadata m ON m.object_id=q.object_id ORDER BY q.object_id")
            .map_err(sql)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    time_column(row, 1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, Vec<u8>>(4)?,
                    row.get::<_, Vec<u8>>(5)?,
                    row.get::<_, u8>(6)?,
                    row.get::<_, u8>(7)?,
                    time_column(row, 8)?,
                ))
            })
            .map_err(sql)?;
        let mut queue = Vec::new();
        for row in rows {
            let (
                object_id,
                custodied_at,
                relay_id,
                origin,
                previous_hop,
                received_from,
                hops,
                hop_limit,
                expires_at,
            ) = row.map_err(sql)?;
            let frame = RelayFrame {
                id: RelayId(relay_id.try_into().map_err(|_| DurableError::Corrupt)?),
                origin: MemberId(origin.try_into().map_err(|_| DurableError::Corrupt)?),
                previous_hop: MemberId(previous_hop.try_into().map_err(|_| DurableError::Corrupt)?),
                hops,
                hop_limit,
                expires_at,
            };
            if frame.expires_at > now {
                queue.push(RelayCustody {
                    object_id: oid(object_id)?,
                    frame,
                    received_from: MemberId(
                        received_from
                            .try_into()
                            .map_err(|_| DurableError::Corrupt)?,
                    ),
                    custodied_at,
                });
            }
        }
        Ok(queue)
    }
    /// Persists a signed receipt received from an authenticated neighbor when
    /// this node is not the origin. The caller has already parsed the receipt
    /// route and bound `frame.origin` to its actor. Keeping this small public
    /// proof separate from object custody lets a relay recover after process
    /// death even when it has no source chunks locally.
    pub fn put_relay_receipt(
        &mut self,
        receipt: &[u8],
        frame: RelayFrame,
        received_from: MemberId,
        now: u64,
    ) -> Result<()> {
        clock(now)?;
        if receipt.is_empty()
            || receipt.len() > 1024
            || frame.previous_hop != self.member
            || frame.hops > frame.hop_limit
            || frame.expires_at <= now
        {
            return Err(DurableError::InvalidInput);
        }
        let id = digest(receipt);
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        tx.execute(
            "DELETE FROM relay_receipt_outbox WHERE expires_at<=?1",
            [now as i64],
        )
        .map_err(sql)?;
        let existing: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM relay_receipt_outbox WHERE receipt_id=?1)",
                [&id[..]],
                |row| row.get(0),
            )
            .map_err(sql)?;
        if !existing {
            let queued: usize = tx
                .query_row("SELECT count(*) FROM relay_receipt_outbox", [], |row| {
                    size_column(row, 0)
                })
                .map_err(sql)?;
            if queued >= self.limits.operations {
                return Err(DurableError::ResourcePressure);
            }
            tx.execute(
                "INSERT INTO relay_receipt_outbox VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![
                    &id[..],
                    receipt,
                    &frame.id.0[..],
                    &frame.origin.0[..],
                    &frame.previous_hop.0[..],
                    &received_from.0[..],
                    frame.hops as i64,
                    frame.hop_limit as i64,
                    frame.expires_at as i64,
                    now as i64,
                ],
            )
            .map_err(sql)?;
        }
        tx.commit().map_err(sql)
    }
    /// Returns durable, bounded receipt work for the host relay scheduler.
    /// Rows stay until expiry; replay is harmless because the origin records
    /// each receipt actor only once.
    pub fn relay_receipt_queue(&self, now: u64) -> Result<Vec<RelayReceiptCustody>> {
        clock(now)?;
        let mut statement = self
            .db
            .prepare(
                "SELECT receipt,relay_id,origin,previous_hop,received_from,hops,hop_limit,expires_at,custodied_at \
                 FROM relay_receipt_outbox WHERE expires_at>?1 ORDER BY receipt_id",
            )
            .map_err(sql)?;
        let rows = statement
            .query_map([now as i64], |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, Vec<u8>>(4)?,
                    row.get::<_, u8>(5)?,
                    row.get::<_, u8>(6)?,
                    time_column(row, 7)?,
                    time_column(row, 8)?,
                ))
            })
            .map_err(sql)?;
        let mut queue = Vec::new();
        for row in rows {
            let (
                receipt,
                relay_id,
                origin,
                previous_hop,
                received_from,
                hops,
                hop_limit,
                expires_at,
                custodied_at,
            ) = row.map_err(sql)?;
            queue.push(RelayReceiptCustody {
                receipt,
                frame: RelayFrame {
                    id: RelayId(relay_id.try_into().map_err(|_| DurableError::Corrupt)?),
                    origin: MemberId(origin.try_into().map_err(|_| DurableError::Corrupt)?),
                    previous_hop: MemberId(
                        previous_hop.try_into().map_err(|_| DurableError::Corrupt)?,
                    ),
                    hops,
                    hop_limit,
                    expires_at,
                },
                received_from: MemberId(
                    received_from
                        .try_into()
                        .map_err(|_| DurableError::Corrupt)?,
                ),
                custodied_at,
            });
        }
        Ok(queue)
    }
    /// A verified origin ACK retires matching relay receipt custody. It only
    /// removes public receipt work, never a delivered encrypted object.
    pub fn remove_relay_receipt(&mut self, receipt_id: [u8; 32]) -> Result<()> {
        self.db
            .execute(
                "DELETE FROM relay_receipt_outbox WHERE receipt_id=?1",
                [&receipt_id[..]],
            )
            .map_err(sql)?;
        Ok(())
    }
    /// Persists an ACK only after FFI verified its origin signature and route.
    pub fn put_relay_receipt_ack(
        &mut self,
        ack: &[u8],
        frame: RelayFrame,
        received_from: MemberId,
        now: u64,
    ) -> Result<()> {
        clock(now)?;
        if ack.is_empty()
            || ack.len() > 1024
            || frame.previous_hop != self.member
            || frame.hops > frame.hop_limit
            || frame.expires_at <= now
        {
            return Err(DurableError::InvalidInput);
        }
        let id = digest(ack);
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        tx.execute(
            "DELETE FROM relay_receipt_ack_outbox WHERE expires_at<=?1",
            [now as i64],
        )
        .map_err(sql)?;
        let existing: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM relay_receipt_ack_outbox WHERE ack_id=?1)",
                [&id[..]],
                |row| row.get(0),
            )
            .map_err(sql)?;
        if !existing {
            let queued: usize = tx
                .query_row("SELECT count(*) FROM relay_receipt_ack_outbox", [], |row| {
                    size_column(row, 0)
                })
                .map_err(sql)?;
            if queued >= self.limits.operations {
                return Err(DurableError::ResourcePressure);
            }
            tx.execute(
                "INSERT INTO relay_receipt_ack_outbox VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![
                    &id[..],
                    ack,
                    &frame.id.0[..],
                    &frame.origin.0[..],
                    &frame.previous_hop.0[..],
                    &received_from.0[..],
                    frame.hops as i64,
                    frame.hop_limit as i64,
                    frame.expires_at as i64,
                    now as i64
                ],
            )
            .map_err(sql)?;
        }
        tx.commit().map_err(sql)
    }
    pub fn relay_receipt_ack_queue(&self, now: u64) -> Result<Vec<RelayReceiptAckCustody>> {
        clock(now)?;
        let mut statement = self.db.prepare(
            "SELECT ack,relay_id,origin,previous_hop,received_from,hops,hop_limit,expires_at,custodied_at \
             FROM relay_receipt_ack_outbox WHERE expires_at>?1 ORDER BY ack_id",
        ).map_err(sql)?;
        let rows = statement
            .query_map([now as i64], |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, Vec<u8>>(4)?,
                    row.get::<_, u8>(5)?,
                    row.get::<_, u8>(6)?,
                    time_column(row, 7)?,
                    time_column(row, 8)?,
                ))
            })
            .map_err(sql)?;
        let mut queue = Vec::new();
        for row in rows {
            let (
                ack,
                relay_id,
                origin,
                previous_hop,
                received_from,
                hops,
                hop_limit,
                expires_at,
                custodied_at,
            ) = row.map_err(sql)?;
            queue.push(RelayReceiptAckCustody {
                ack,
                frame: RelayFrame {
                    id: RelayId(relay_id.try_into().map_err(|_| DurableError::Corrupt)?),
                    origin: MemberId(origin.try_into().map_err(|_| DurableError::Corrupt)?),
                    previous_hop: MemberId(
                        previous_hop.try_into().map_err(|_| DurableError::Corrupt)?,
                    ),
                    hops,
                    hop_limit,
                    expires_at,
                },
                received_from: MemberId(
                    received_from
                        .try_into()
                        .map_err(|_| DurableError::Corrupt)?,
                ),
                custodied_at,
            });
        }
        Ok(queue)
    }
    pub fn stats(&self) -> Result<Stats> {
        self.db.query_row("SELECT (SELECT count(*) FROM objects),(SELECT count(*) FROM chunks),(SELECT coalesce(sum(reserved_bytes),0) FROM objects),(SELECT count(*) FROM operations),(SELECT count(*) FROM outbox),(SELECT count(*) FROM relay_outbox),(SELECT count(*) FROM local_deliveries)",[],|r|Ok(Stats{objects:size_column(r,0)?,chunks:size_column(r,1)?,bytes_reserved:size_column(r,2)?,operations:size_column(r,3)?,outbox:size_column(r,4)?,relay_outbox:size_column(r,5)?,local_deliveries:size_column(r,6)?})).map_err(sql)
    }
    /// Starts the origin-local aggregate for a visible group action. The
    /// caller has already derived the audiences from the same certified roster
    /// used for sealing, so counts are constrained to the field profile.
    pub fn begin_logical_message(
        &mut self,
        id: LogicalMessageId,
        target_count: usize,
        audience_count: usize,
        expires_at: u64,
        now: u64,
    ) -> Result<()> {
        clock(now)?;
        if target_count == 0
            || target_count > 49
            || audience_count == 0
            || audience_count > 5
            || expires_at <= now
        {
            return Err(DurableError::InvalidInput);
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let existing: Option<(u64, usize, usize)> = tx
            .query_row(
                "SELECT expires_at,target_count,audience_count FROM logical_messages WHERE id=?1",
                [&id.0[..]],
                |row| {
                    Ok((
                        time_column(row, 0)?,
                        size_column(row, 1)?,
                        size_column(row, 2)?,
                    ))
                },
            )
            .optional()
            .map_err(sql)?;
        if let Some(existing) = existing {
            if existing != (expires_at, target_count, audience_count) {
                return Err(DurableError::Conflict);
            }
            return Ok(());
        }
        tx.execute(
            "INSERT INTO logical_messages VALUES(?1,?2,?3,?4,?5)",
            params![
                &id.0[..],
                now as i64,
                expires_at as i64,
                target_count as i64,
                audience_count as i64,
            ],
        )
        .map_err(sql)?;
        tx.commit().map_err(sql)
    }
    /// Reads aggregate state without exposing an object payload, receipt, or
    /// recipient identity. A message becomes `Delivered` only once every
    /// certified target has a verified receipt and every audience committed.
    pub fn logical_delivery_summary(
        &self,
        id: LogicalMessageId,
        now: u64,
    ) -> Result<Option<LogicalDeliverySummary>> {
        clock(now)?;
        let Some((expires_at, target_count, audience_count)) = self
            .db
            .query_row(
                "SELECT expires_at,target_count,audience_count FROM logical_messages WHERE id=?1",
                [&id.0[..]],
                |row| {
                    Ok((
                        time_column(row, 0)?,
                        size_column(row, 1)?,
                        size_column(row, 2)?,
                    ))
                },
            )
            .optional()
            .map_err(sql)?
        else {
            return Ok(None);
        };
        let committed_audiences: usize = self
            .db
            .query_row(
                "SELECT count(*) FROM logical_message_objects WHERE logical_id=?1",
                [&id.0[..]],
                |row| size_column(row, 0),
            )
            .map_err(sql)?;
        let delivered_targets: usize = self
            .db
            .query_row(
                "SELECT count(*) FROM target_receipts r JOIN logical_message_objects o ON o.object_id=r.object_id WHERE o.logical_id=?1",
                [&id.0[..]],
                |row| size_column(row, 0),
            )
            .map_err(sql)?;
        if committed_audiences > audience_count || delivered_targets > target_count {
            return Err(DurableError::Corrupt);
        }
        let state = if expires_at <= now {
            LogicalDeliveryState::Expired
        } else if committed_audiences == audience_count && delivered_targets == target_count {
            LogicalDeliveryState::Delivered
        } else if delivered_targets > 0 {
            LogicalDeliveryState::PartiallyDelivered
        } else {
            LogicalDeliveryState::Queued
        };
        Ok(Some(LogicalDeliverySummary {
            id,
            target_count,
            audience_count,
            committed_audiences,
            delivered_targets,
            state,
        }))
    }
    /// Returns the newest still-live aggregate created by this source. Hosts
    /// use this to refresh a chat bubble without receiving object IDs or
    /// recipient receipt proofs over their UI boundary.
    pub fn latest_logical_delivery_summary(
        &self,
        now: u64,
    ) -> Result<Option<LogicalDeliverySummary>> {
        clock(now)?;
        let id: Option<Vec<u8>> = self
            .db
            .query_row(
                "SELECT id FROM logical_messages WHERE expires_at>?1 ORDER BY created_at DESC,id DESC LIMIT 1",
                [now as i64],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql)?;
        id.map(|id| {
            let id = LogicalMessageId(id.try_into().map_err(|_| DurableError::Corrupt)?);
            self.logical_delivery_summary(id, now)?
                .ok_or(DurableError::Corrupt)
        })
        .transpose()
    }
    /// Validates a roster before replacing the active policy. Authority and group
    /// are pinned after the first install; only a strictly newer epoch may replace it.
    pub fn install_policy(
        &mut self,
        authority: [u8; 32],
        scope: Scope,
        certificates: &[Vec<u8>],
        revoked: &[u64],
        now: u64,
    ) -> Result<PolicySnapshot> {
        self.install_policy_with_transition(authority, scope, certificates, revoked, now, None)
    }

    /// Replaces a pinned authority only after a verified handoff from the
    /// current authority. The new policy must be the immediate next epoch and
    /// retain the exact group; callers never supply a private authority key.
    pub fn install_rotated_policy(
        &mut self,
        authority: [u8; 32],
        scope: Scope,
        certificates: &[Vec<u8>],
        revoked: &[u64],
        handoff: &AuthorityHandoff,
        now: u64,
    ) -> Result<PolicySnapshot> {
        let current = self
            .active_policy(now)?
            .ok_or(DurableError::AuthenticationFailed)?;
        handoff.verify_for(current.authority, current.scope, current.roster_digest, now)?;
        if authority != handoff.next_authority
            || scope.group != current.scope.group
            || scope.epoch != handoff.next_epoch
        {
            return Err(DurableError::AuthenticationFailed);
        }
        self.install_policy_with_transition(
            authority,
            scope,
            certificates,
            revoked,
            now,
            Some(current),
        )
    }

    fn install_policy_with_transition(
        &mut self,
        authority: [u8; 32],
        scope: Scope,
        certificates: &[Vec<u8>],
        revoked: &[u64],
        now: u64,
        transition: Option<PolicySnapshot>,
    ) -> Result<PolicySnapshot> {
        let roster = VerifiedRoster::verify(authority, scope, certificates, revoked, now)?;
        if !roster.contains_member(self.member) {
            return Err(DurableError::AuthenticationFailed);
        }
        let snapshot = PolicySnapshot {
            authority,
            scope,
            roster_digest: roster.digest(),
        };
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let existing: Option<PolicyRow> = tx
            .query_row(
                "SELECT authority,group_id,epoch,roster_digest FROM policy_state WHERE singleton=1",
                [],
                |r| {
                    Ok((
                        r.get::<_, Vec<u8>>(0)?
                            .try_into()
                            .map_err(|_| rusqlite::Error::InvalidQuery)?,
                        r.get::<_, Vec<u8>>(1)?
                            .try_into()
                            .map_err(|_| rusqlite::Error::InvalidQuery)?,
                        time_column(r, 2)?,
                        r.get::<_, Vec<u8>>(3)?
                            .try_into()
                            .map_err(|_| rusqlite::Error::InvalidQuery)?,
                    ))
                },
            )
            .optional()
            .map_err(sql)?;
        if let Some((old_authority, old_group, old_epoch, old_digest)) = existing {
            let rotation_allowed = transition.is_some_and(|prior| {
                prior.authority == old_authority
                    && prior.scope.group == old_group
                    && prior.scope.epoch == old_epoch
                    && prior.roster_digest == old_digest
                    && authority != old_authority
                    && scope.group == old_group
                    && scope.epoch == old_epoch.saturating_add(1)
            });
            if (old_authority != authority && !rotation_allowed)
                || old_group != scope.group
                || scope.epoch < old_epoch
                || (scope.epoch == old_epoch && old_digest != snapshot.roster_digest)
            {
                return Err(DurableError::Conflict);
            }
            if scope.epoch == old_epoch {
                return Ok(snapshot);
            }
            tx.execute("DELETE FROM policy_certificates", [])
                .map_err(sql)?;
            tx.execute("DELETE FROM policy_revocations", [])
                .map_err(sql)?;
            tx.execute("DELETE FROM policy_state", []).map_err(sql)?;
        }
        tx.execute(
            "INSERT INTO policy_state VALUES(1,?1,?2,?3,?4)",
            params![
                &authority[..],
                &scope.group[..],
                scope.epoch as i64,
                &snapshot.roster_digest[..]
            ],
        )
        .map_err(sql)?;
        for (index, certificate) in certificates.iter().enumerate() {
            tx.execute(
                "INSERT INTO policy_certificates VALUES(?1,?2)",
                params![index as i64, certificate],
            )
            .map_err(sql)?;
        }
        for serial in revoked {
            tx.execute(
                "INSERT INTO policy_revocations VALUES(?1)",
                [*serial as i64],
            )
            .map_err(sql)?;
        }
        tx.commit().map_err(sql)?;
        Ok(snapshot)
    }
    pub fn active_policy(&self, now: u64) -> Result<Option<PolicySnapshot>> {
        let state: Option<PolicyRow> = self
            .db
            .query_row(
                "SELECT authority,group_id,epoch,roster_digest FROM policy_state WHERE singleton=1",
                [],
                |r| {
                    Ok((
                        r.get::<_, Vec<u8>>(0)?
                            .try_into()
                            .map_err(|_| rusqlite::Error::InvalidQuery)?,
                        r.get::<_, Vec<u8>>(1)?
                            .try_into()
                            .map_err(|_| rusqlite::Error::InvalidQuery)?,
                        time_column(r, 2)?,
                        r.get::<_, Vec<u8>>(3)?
                            .try_into()
                            .map_err(|_| rusqlite::Error::InvalidQuery)?,
                    ))
                },
            )
            .optional()
            .map_err(sql)?;
        let Some((authority, group, epoch, digest)) = state else {
            return Ok(None);
        };
        let certificates = self
            .db
            .prepare("SELECT certificate FROM policy_certificates ORDER BY idx")
            .map_err(sql)?
            .query_map([], |r| r.get::<_, Vec<u8>>(0))
            .map_err(sql)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(sql)?;
        let revoked = self
            .db
            .prepare("SELECT serial FROM policy_revocations ORDER BY serial")
            .map_err(sql)?
            .query_map([], |r| time_column(r, 0))
            .map_err(sql)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(sql)?;
        let scope = Scope { group, epoch };
        let roster = VerifiedRoster::verify(authority, scope, &certificates, &revoked, now)?;
        if roster.digest() != digest {
            return Err(DurableError::Corrupt);
        }
        Ok(Some(PolicySnapshot {
            authority,
            scope,
            roster_digest: digest,
        }))
    }
    /// Returns the public policy transport after the caller has validated it via
    /// `active_policy`. It never exposes the SQLCipher key or store member key.
    pub fn policy_bundle(&self) -> Result<Option<PolicyBundle>> {
        let state: Option<PolicyRow> = self
            .db
            .query_row(
                "SELECT authority,group_id,epoch,roster_digest FROM policy_state WHERE singleton=1",
                [],
                |r| {
                    Ok((
                        r.get::<_, Vec<u8>>(0)?
                            .try_into()
                            .map_err(|_| rusqlite::Error::InvalidQuery)?,
                        r.get::<_, Vec<u8>>(1)?
                            .try_into()
                            .map_err(|_| rusqlite::Error::InvalidQuery)?,
                        time_column(r, 2)?,
                        r.get::<_, Vec<u8>>(3)?
                            .try_into()
                            .map_err(|_| rusqlite::Error::InvalidQuery)?,
                    ))
                },
            )
            .optional()
            .map_err(sql)?;
        let Some((authority, group, epoch, _)) = state else {
            return Ok(None);
        };
        let certificates = self
            .db
            .prepare("SELECT certificate FROM policy_certificates ORDER BY idx")
            .map_err(sql)?
            .query_map([], |r| r.get::<_, Vec<u8>>(0))
            .map_err(sql)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(sql)?;
        let revoked = self
            .db
            .prepare("SELECT serial FROM policy_revocations ORDER BY serial")
            .map_err(sql)?
            .query_map([], |r| time_column(r, 0))
            .map_err(sql)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(sql)?;
        Ok(Some(PolicyBundle {
            authority,
            scope: Scope { group, epoch },
            certificates,
            revoked,
        }))
    }
}
fn link_logical_object(
    tx: &Transaction<'_>,
    logical: LogicalMessageId,
    object: ObjectId,
    target_count: usize,
) -> Result<()> {
    if target_count == 0 || target_count > 10 {
        return Err(DurableError::InvalidInput);
    }
    let exists: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM logical_messages WHERE id=?1)",
            [&logical.0[..]],
            |row| row.get(0),
        )
        .map_err(sql)?;
    if !exists {
        return Err(DurableError::NotFound);
    }
    let existing: Option<(Vec<u8>, usize)> = tx
        .query_row(
            "SELECT logical_id,target_count FROM logical_message_objects WHERE object_id=?1",
            [&object.0[..]],
            |row| Ok((row.get(0)?, size_column(row, 1)?)),
        )
        .optional()
        .map_err(sql)?;
    if let Some((existing_logical, existing_count)) = existing {
        return if existing_logical == logical.0 && existing_count == target_count {
            Ok(())
        } else {
            Err(DurableError::Conflict)
        };
    }
    let (target_total, audience_total): (usize, usize) = tx
        .query_row(
            "SELECT target_count,audience_count FROM logical_messages WHERE id=?1",
            [&logical.0[..]],
            |row| Ok((size_column(row, 0)?, size_column(row, 1)?)),
        )
        .map_err(sql)?;
    let (bound_targets, bound_audiences): (usize, usize) = tx
        .query_row(
            "SELECT coalesce(sum(target_count),0),count(*) FROM logical_message_objects WHERE logical_id=?1",
            [&logical.0[..]],
            |row| Ok((size_column(row, 0)?, size_column(row, 1)?)),
        )
        .map_err(sql)?;
    if bound_audiences >= audience_total
        || bound_targets
            .checked_add(target_count)
            .ok_or(DurableError::ResourcePressure)?
            > target_total
    {
        return Err(DurableError::Conflict);
    }
    tx.execute(
        "INSERT INTO logical_message_objects VALUES(?1,?2,?3)",
        params![&logical.0[..], &object.0[..], target_count as i64],
    )
    .map_err(sql)?;
    Ok(())
}

fn logical_object_matches(
    tx: &Transaction<'_>,
    logical: LogicalMessageId,
    object: ObjectId,
    target_count: usize,
) -> Result<()> {
    let existing: Option<(Vec<u8>, usize)> = tx
        .query_row(
            "SELECT logical_id,target_count FROM logical_message_objects WHERE object_id=?1",
            [&object.0[..]],
            |row| Ok((row.get(0)?, size_column(row, 1)?)),
        )
        .optional()
        .map_err(sql)?;
    match existing {
        Some((stored, count)) if stored == logical.0 && count == target_count => Ok(()),
        _ => Err(DurableError::Conflict),
    }
}

fn delete_object(tx: &Transaction<'_>, object: ObjectId) -> Result<()> {
    let id = &object.0[..];
    // Delete children explicitly. The initial encrypted schema intentionally
    // avoided cascading deletes so every retention boundary stays auditable.
    tx.execute(
        "DELETE FROM receipt_acknowledgements WHERE object_id=?1",
        [id],
    )
    .map_err(sql)?;
    tx.execute("DELETE FROM auth_deliveries WHERE object_id=?1", [id])
        .map_err(sql)?;
    tx.execute("DELETE FROM relay_metadata WHERE object_id=?1", [id])
        .map_err(sql)?;
    tx.execute("DELETE FROM target_receipts WHERE object_id=?1", [id])
        .map_err(sql)?;
    tx.execute("DELETE FROM auth_announcements WHERE object_id=?1", [id])
        .map_err(sql)?;
    tx.execute(
        "DELETE FROM logical_message_objects WHERE object_id=?1",
        [id],
    )
    .map_err(sql)?;
    tx.execute("DELETE FROM chunks WHERE object_id=?1", [id])
        .map_err(sql)?;
    tx.execute("DELETE FROM outbox WHERE object_id=?1", [id])
        .map_err(sql)?;
    tx.execute("DELETE FROM relay_outbox WHERE object_id=?1", [id])
        .map_err(sql)?;
    tx.execute("DELETE FROM local_deliveries WHERE object_id=?1", [id])
        .map_err(sql)?;
    tx.execute("DELETE FROM operations WHERE object_id=?1", [id])
        .map_err(sql)?;
    tx.execute("DELETE FROM objects WHERE id=?1", [id])
        .map_err(sql)?;
    Ok(())
}

fn manifest_from(db: &Connection, id: ObjectId) -> Result<Manifest> {
    let bytes: Vec<u8> = db
        .query_row(
            "SELECT manifest FROM objects WHERE id=?1",
            [&id.0[..]],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql)?
        .ok_or(DurableError::NotFound)?;
    let m = Manifest::decode(&bytes).map_err(|_| DurableError::Corrupt)?;
    if m.id() != id {
        return Err(DurableError::Corrupt);
    }
    Ok(m)
}
fn insert_manifest(tx: &Transaction<'_>, m: &Manifest, limits: Limits) -> Result<()> {
    let id = m.id();
    let encoded = m.encode();
    let previous: Option<Vec<u8>> = tx
        .query_row(
            "SELECT manifest FROM objects WHERE origin=?1 AND sequence=?2",
            params![&m.origin().0[..], m.sequence() as i64],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql)?;
    if let Some(previous) = previous {
        return if previous == encoded {
            Ok(())
        } else {
            Err(DurableError::Conflict)
        };
    }
    let (count, bytes): (usize, usize) = tx
        .query_row(
            "SELECT count(*),coalesce(sum(reserved_bytes),0) FROM objects",
            [],
            |r| Ok((size_column(r, 0)?, size_column(r, 1)?)),
        )
        .map_err(sql)?;
    if count >= limits.objects
        || bytes
            .checked_add(m.content_len())
            .ok_or(DurableError::ResourcePressure)?
            > limits.bytes
    {
        return Err(DurableError::ResourcePressure);
    }
    tx.execute(
        "INSERT INTO objects(id,origin,sequence,manifest,reserved_bytes) VALUES(?1,?2,?3,?4,?5)",
        params![
            &id.0[..],
            &m.origin().0[..],
            m.sequence() as i64,
            encoded,
            m.content_len() as i64
        ],
    )
    .map_err(sql)?;
    Ok(())
}
fn insert_chunk(tx: &Transaction<'_>, id: ObjectId, index: usize, bytes: &[u8]) -> Result<()> {
    let existing: Option<Vec<u8>> = tx
        .query_row(
            "SELECT payload FROM chunks WHERE object_id=?1 AND idx=?2",
            params![&id.0[..], index as i64],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql)?;
    if let Some(existing) = existing {
        if existing != bytes {
            return Err(DurableError::Corrupt);
        }
    } else {
        tx.execute(
            "INSERT INTO chunks VALUES(?1,?2,?3)",
            params![&id.0[..], index as i64, bytes],
        )
        .map_err(sql)?;
    }
    Ok(())
}
