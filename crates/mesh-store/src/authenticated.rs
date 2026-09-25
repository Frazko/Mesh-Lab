use super::*;
use mesh_crypto::{DeliverySecret, IdentitySigningKey};
use mesh_protocol::{
    self as protocol, AuthenticatedAnnouncement, SealedMessage, VerifiedDelivery, VerifiedReceipt,
    VerifiedReceiptAck, VerifiedRoster,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChunkStored {
    pub object: ObjectId,
    pub index: usize,
    pub all_chunks_present: bool,
}
pub struct ReceiptCommit {
    local: LocalCommit,
    receipt: Vec<u8>,
}
impl ReceiptCommit {
    pub fn local(&self) -> LocalCommit {
        self.local
    }
    pub fn receipt(&self) -> &[u8] {
        &self.receipt
    }
}
/// A protected object cannot be promoted from synthetic state or manipulated
/// through legacy ingestion methods, even with exactly the same object ID.
pub(super) fn check_mode(db: &Connection, id: ObjectId, announcement: Option<&[u8]>) -> Result<()> {
    let existing:Option<Option<Vec<u8>>>=db.query_row("SELECT a.announcement FROM objects o LEFT JOIN auth_announcements a ON a.object_id=o.id WHERE o.id=?1",[&id.0[..]],|r|r.get(0)).optional().map_err(sql)?;
    match (existing, announcement) {
        (None, _) | (Some(None), None) => Ok(()),
        (Some(Some(bytes)), Some(expected)) if bytes == expected => Ok(()),
        _ => Err(DurableError::AuthenticationFailed),
    }
}
fn stored_announcement(db: &Connection, id: ObjectId) -> Result<Vec<u8>> {
    db.query_row(
        "SELECT announcement FROM auth_announcements WHERE object_id=?1",
        [&id.0[..]],
        |r| r.get(0),
    )
    .optional()
    .map_err(sql)?
    .ok_or(DurableError::AuthenticationFailed)
}

/// Relay custody is permitted only after the authenticated ingress path has
/// recorded the exact signed announcement. This is intentionally distinct
/// from the legacy chunk path, which must not promote authenticated objects.
pub(super) fn require_authenticated_announcement(db: &Connection, id: ObjectId) -> Result<()> {
    stored_announcement(db, id).map(|_| ())
}
impl Store {
    pub fn commit_sealed(
        &mut self,
        id: OperationId,
        command_hash: [u8; 32],
        message: &SealedMessage,
        roster: &VerifiedRoster,
        now: u64,
    ) -> Result<ObjectId> {
        message.announcement().validate_roster(roster, now)?;
        self.commit_outgoing_inner(
            id,
            command_hash,
            message.object(),
            now,
            Some(message.announcement()),
            None,
        )
    }
    /// Commits one encrypted audience and its logical group relationship in
    /// the same SQLite transaction. The logical group must have been created
    /// before its first audience is sealed.
    pub fn commit_sealed_logical(
        &mut self,
        id: OperationId,
        command_hash: [u8; 32],
        message: &SealedMessage,
        roster: &VerifiedRoster,
        logical: LogicalMessageId,
        now: u64,
    ) -> Result<ObjectId> {
        message.announcement().validate_roster(roster, now)?;
        self.commit_outgoing_inner(
            id,
            command_hash,
            message.object(),
            now,
            Some(message.announcement()),
            Some(logical),
        )
    }
    pub fn announce_authenticated(
        &mut self,
        a: &AuthenticatedAnnouncement,
        roster: &VerifiedRoster,
        now: u64,
    ) -> Result<ObjectId> {
        a.validate_roster(roster, now)?;
        live(a.manifest(), now)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let id = a.manifest().id();
        check_mode(&tx, id, Some(a.bytes()))?;
        insert_manifest(&tx, a.manifest(), self.limits)?;
        tx.execute(
            "INSERT OR IGNORE INTO auth_announcements VALUES(?1,?2)",
            params![&id.0[..], a.bytes()],
        )
        .map_err(sql)?;
        tx.commit().map_err(sql)?;
        Ok(id)
    }
    pub fn authenticated_announcement(
        &self,
        id: ObjectId,
        roster: &VerifiedRoster,
        now: u64,
    ) -> Result<AuthenticatedAnnouncement> {
        let a =
            protocol::authenticate_announcement(&stored_announcement(&self.db, id)?, roster, now)?;
        if a.manifest() != &self.manifest(id)? {
            return Err(DurableError::Corrupt);
        }
        Ok(a)
    }
    pub fn stage_authenticated_chunk(
        &mut self,
        id: ObjectId,
        index: usize,
        bytes: &[u8],
        now: u64,
    ) -> Result<ChunkStored> {
        let observe = self.observer();
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        stored_announcement(&tx, id)?;
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
        observe(Stage::BeforeCommit);
        tx.commit().map_err(sql)?;
        observe(Stage::AfterCommit);
        Ok(ChunkStored {
            object: id,
            index,
            all_chunks_present: count == m.chunk_count(),
        })
    }
    pub fn object_bytes(&self, id: ObjectId) -> Result<Vec<u8>> {
        let m = self.manifest(id)?;
        let mut bytes = Vec::with_capacity(m.content_len());
        for index in 0..m.chunk_count() {
            bytes.extend(self.chunk(id, index)?)
        }
        if bytes.len() != m.content_len() {
            return Err(DurableError::Corrupt);
        }
        Ok(bytes)
    }
    pub fn verify_received(
        &self,
        id: ObjectId,
        roster: &VerifiedRoster,
        secret: &DeliverySecret,
        now: u64,
    ) -> Result<VerifiedDelivery> {
        let a = self.authenticated_announcement(id, roster, now)?;
        protocol::verify_delivery(a, &self.object_bytes(id)?, roster, self.member, secret, now)
    }
    /// The delivery marker, signed receipt and complete flag form one transaction.
    /// Receipt bytes cannot escape this API until SQLite reports a successful commit.
    pub fn finalize_received(
        &mut self,
        proof: &VerifiedDelivery,
        roster: &VerifiedRoster,
        signer: &IdentitySigningKey,
        now: u64,
    ) -> Result<ReceiptCommit> {
        let id = proof.object_id();
        if proof.member() != self.member {
            return Err(DurableError::AuthenticationFailed);
        }
        proof.announcement().validate_roster(roster, now)?;
        let observe = self.observer();
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        if stored_announcement(&tx, id)? != proof.announcement().bytes() {
            return Err(DurableError::AuthenticationFailed);
        }
        if let Some(existing) = receipt_from(&tx, id, self.member)? {
            return Ok(existing);
        }
        proof.validate_commit(roster, now)?;
        let m = manifest_from(&tx, id)?;
        for i in 0..m.chunk_count() {
            let bytes: Vec<u8> = tx
                .query_row(
                    "SELECT payload FROM chunks WHERE object_id=?1 AND idx=?2",
                    params![&id.0[..], i as i64],
                    |r| r.get(0),
                )
                .optional()
                .map_err(sql)?
                .ok_or(DurableError::NotFound)?;
            m.verify_chunk(i, &bytes)?;
        }
        let receipt = proof.receipt_for_commit(signer, now)?;
        tx.execute(
            "INSERT INTO local_deliveries VALUES(?1,?2)",
            params![&id.0[..], now as i64],
        )
        .map_err(sql)?;
        tx.execute(
            "INSERT INTO auth_deliveries VALUES(?1,?2)",
            params![&id.0[..], &receipt],
        )
        .map_err(sql)?;
        tx.execute("UPDATE objects SET complete=1 WHERE id=?1", [&id.0[..]])
            .map_err(sql)?;
        observe(Stage::DeliveryRecorded);
        observe(Stage::BeforeCommit);
        tx.commit().map_err(sql)?;
        observe(Stage::AfterCommit);
        Ok(ReceiptCommit {
            local: LocalCommit {
                object_id: id,
                member: self.member,
                committed_at: now,
            },
            receipt,
        })
    }
    pub fn local_receipt(&self, id: ObjectId) -> Result<Option<ReceiptCommit>> {
        receipt_from(&self.db, id, self.member)
    }
    /// Signed receipts are durable local work: their scheduler may replay them
    /// after reconnect until the object expires. The origin deduplicates each
    /// actor receipt transactionally, so replay cannot over-count delivery.
    pub fn local_receipt_outbox(&self, now: u64) -> Result<Vec<ObjectId>> {
        clock(now)?;
        let mut statement = self
            .db
            .prepare(
                "SELECT d.object_id FROM local_deliveries d \
                 JOIN auth_deliveries a ON a.object_id=d.object_id \
                 LEFT JOIN receipt_acknowledgements k ON k.object_id=d.object_id \
                 WHERE k.object_id IS NULL ORDER BY d.object_id",
            )
            .map_err(sql)?;
        let rows = statement
            .query_map([], |row| row.get::<_, Vec<u8>>(0))
            .map_err(sql)?;
        let mut result = Vec::new();
        for row in rows {
            let id = oid(row.map_err(sql)?)?;
            let manifest = self.manifest(id)?;
            if manifest.origin() != self.member && manifest.expires_at() > now {
                result.push(id);
            }
        }
        Ok(result)
    }
    /// Returns one complete, authenticated object addressed to this store
    /// which has not yet made its plaintext visible locally. The caller still
    /// verifies and finalizes it; this query is only an atomic-work selector.
    pub fn next_authenticated_delivery(&self, now: u64) -> Result<Option<ObjectId>> {
        clock(now)?;
        let mut statement = self
            .db
            .prepare(
                "SELECT o.id FROM objects o \
                 JOIN auth_announcements a ON a.object_id=o.id \
                 LEFT JOIN local_deliveries d ON d.object_id=o.id \
                 WHERE o.complete=1 AND d.object_id IS NULL ORDER BY o.id",
            )
            .map_err(sql)?;
        let rows = statement
            .query_map([], |row| row.get::<_, Vec<u8>>(0))
            .map_err(sql)?;
        for row in rows {
            let id = oid(row.map_err(sql)?)?;
            let manifest = self.manifest(id)?;
            if manifest.origin() != self.member
                && manifest.expires_at() > now
                && manifest.targets().contains(&self.member)
            {
                return Ok(Some(id));
            }
        }
        Ok(None)
    }
    pub fn verify_target_receipt(
        &self,
        id: ObjectId,
        bytes: &[u8],
        roster: &VerifiedRoster,
        now: u64,
    ) -> Result<VerifiedReceipt> {
        let a = self.authenticated_announcement(id, roster, now)?;
        protocol::verify_receipt(bytes, &a, &self.object_bytes(id)?, roster, now)
    }
    pub fn record_target_receipt(
        &mut self,
        proof: &VerifiedReceipt,
        roster: &VerifiedRoster,
        now: u64,
    ) -> Result<DeliveryProgress> {
        proof.validate_commit(roster, now)?;
        let id = proof.object_id();
        let observe = self.observer();
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let m = manifest_from(&tx, id)?;
        if m.origin() != self.member
            || stored_announcement(&tx, id)? != proof.announcement()
            || !m.targets().contains(&proof.actor())
        {
            return Err(DurableError::AuthenticationFailed);
        }
        let operation: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM operations WHERE object_id=?1)",
                [&id.0[..]],
                |r| r.get(0),
            )
            .map_err(sql)?;
        if !operation {
            return Err(DurableError::NotFound);
        }
        tx.execute(
            "INSERT OR IGNORE INTO target_receipts VALUES(?1,?2,?3)",
            params![&id.0[..], &proof.actor().0[..], proof.bytes()],
        )
        .map_err(sql)?;
        let progress = progress_from(&tx, &m, now)?;
        if progress.state == DeliveryState::Delivered {
            tx.execute("DELETE FROM outbox WHERE object_id=?1", [&id.0[..]])
                .map_err(sql)?;
        }
        observe(Stage::ReceiptRecorded);
        observe(Stage::BeforeCommit);
        tx.commit().map_err(sql)?;
        observe(Stage::AfterCommit);
        Ok(progress)
    }
    /// Returns receipts durably recorded by this origin that still need their
    /// origin-signed ACK placed onto a currently available mesh edge. Replays
    /// are harmless: recipients match the digest to one immutable receipt.
    pub fn target_receipt_ack_outbox(
        &self,
        now: u64,
    ) -> Result<Vec<(ObjectId, MemberId, Vec<u8>)>> {
        clock(now)?;
        let mut statement = self
            .db
            .prepare(
                "SELECT t.object_id,t.actor,t.receipt FROM target_receipts t \
             JOIN objects o ON o.id=t.object_id \
             WHERE o.origin=?1 ORDER BY t.object_id,t.actor",
            )
            .map_err(sql)?;
        let rows = statement
            .query_map([&self.member.0[..]], |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                ))
            })
            .map_err(sql)?;
        let mut result = Vec::new();
        for row in rows {
            let (object, actor, receipt) = row.map_err(sql)?;
            let object = oid(object)?;
            let manifest = self.manifest(object)?;
            if manifest.expires_at() > now {
                result.push((
                    object,
                    MemberId(actor.try_into().map_err(|_| DurableError::Corrupt)?),
                    receipt,
                ));
            }
        }
        Ok(result)
    }
    /// Persist the signed ACK before exposing it to a host. A new drain or
    /// process must replay the same bytes and route ID, not manufacture new
    /// control traffic for every GPS receipt. A changed authorization policy
    /// may require re-signing, but the target receipt must still verify.
    pub fn origin_receipt_ack(
        &self,
        object: ObjectId,
        actor: MemberId,
        roster: &VerifiedRoster,
        signer: &IdentitySigningKey,
        now: u64,
    ) -> Result<Vec<u8>> {
        roster.validate_at(now)?;
        if signer.public_key() != roster.signing_key(self.member)? {
            return Err(DurableError::AuthenticationFailed);
        }
        let receipt: Vec<u8> = self
            .db
            .query_row(
                "SELECT receipt FROM target_receipts WHERE object_id=?1 AND actor=?2",
                params![&object.0[..], &actor.0[..]],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql)?
            .ok_or(DurableError::NotFound)?;
        let cached: Option<Vec<u8>> = self
            .db
            .query_row(
                "SELECT ack FROM origin_receipt_acks WHERE object_id=?1 AND actor=?2",
                params![&object.0[..], &actor.0[..]],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql)?;
        if let Some(ack) = cached {
            if protocol::verify_receipt_ack(&ack, roster, now).is_ok() {
                return Ok(ack);
            }
        }
        self.verify_target_receipt(object, &receipt, roster, now)?;
        let ack = protocol::issue_receipt_ack(
            &receipt,
            object,
            self.member,
            actor,
            roster.scope(),
            signer,
            now,
        )?;
        self.db
            .execute(
                "INSERT INTO origin_receipt_acks VALUES(?1,?2,?3) \
             ON CONFLICT(object_id,actor) DO UPDATE SET ack=excluded.ack",
                params![&object.0[..], &actor.0[..], &ack],
            )
            .map_err(sql)?;
        Ok(ack)
    }

    /// Commits an origin-signed acknowledgement only when it refers to this
    /// member's exact durable receipt. The receipt itself remains local audit
    /// evidence; this row only removes it from the retry scheduler.
    pub fn record_receipt_ack(
        &mut self,
        proof: &VerifiedReceiptAck,
        roster: &VerifiedRoster,
        now: u64,
    ) -> Result<()> {
        roster.validate_at(now)?;
        let route = proof.route();
        if route.actor != self.member {
            return Err(DurableError::AuthenticationFailed);
        }
        let receipt =
            receipt_from(&self.db, route.object, self.member)?.ok_or(DurableError::NotFound)?;
        let manifest = self.manifest(route.object)?;
        if manifest.origin() != route.origin || digest(receipt.receipt()) != route.receipt_id {
            return Err(DurableError::AuthenticationFailed);
        }
        self.db
            .execute(
                "INSERT OR IGNORE INTO receipt_acknowledgements VALUES(?1,?2,?3)",
                params![
                    &route.object.0[..],
                    &route.receipt_id[..],
                    proof.acknowledged_at() as i64
                ],
            )
            .map_err(sql)?;
        Ok(())
    }
    pub fn authenticated_progress(&self, id: ObjectId, now: u64) -> Result<DeliveryProgress> {
        clock(now)?;
        stored_announcement(&self.db, id)?;
        progress_from(&self.db, &self.manifest(id)?, now)
    }
}
fn receipt_from(db: &Connection, id: ObjectId, member: MemberId) -> Result<Option<ReceiptCommit>> {
    let row:Option<(u64,Vec<u8>)>=db.query_row("SELECT d.committed_at,a.receipt FROM local_deliveries d JOIN auth_deliveries a ON a.object_id=d.object_id WHERE d.object_id=?1",[&id.0[..]],|r|Ok((time_column(r,0)?,r.get(1)?))).optional().map_err(sql)?;
    Ok(row.map(|(committed_at, receipt)| ReceiptCommit {
        local: LocalCommit {
            object_id: id,
            member,
            committed_at,
        },
        receipt,
    }))
}
fn progress_from(db: &Connection, m: &Manifest, now: u64) -> Result<DeliveryProgress> {
    let confirmed: usize = db
        .query_row(
            "SELECT count(*) FROM target_receipts WHERE object_id=?1",
            [&m.id().0[..]],
            |r| size_column(r, 0),
        )
        .map_err(sql)?;
    let required = m.targets().len();
    if confirmed > required {
        return Err(DurableError::Corrupt);
    }
    let state = if confirmed == required {
        DeliveryState::Delivered
    } else if now >= m.expires_at() {
        DeliveryState::Expired
    } else if confirmed > 0 {
        DeliveryState::Relaying
    } else {
        DeliveryState::Stored
    };
    Ok(DeliveryProgress {
        state,
        confirmed,
        required,
    })
}
