//! Versioned laboratory subset of A002/A003: certified roster, protected objects,
//! authenticated announcements and recipient receipts. No Noise or radio framing.
use mesh_codec::canonical::{Reader, Writer};
use mesh_crypto::{
    self as crypto, ChunkContext, ContentSealer, DeliverySecret, Domain, IdentitySigningKey,
    RandomSource, Recipient, RecipientWrap, Scope,
};
use mesh_object::{digest, Manifest, ObjectPolicy, PreparedObject};
use mesh_types::durable::*;
use std::collections::{BTreeMap, BTreeSet};
use zeroize::Zeroizing;

pub const MAX_PLAINTEXT: usize = 48 * 1024;
// A routed record carries 93 bytes of canonical hop metadata around this
// record. Keep the largest valid announcement inside one 4096-byte Noise
// application datagram; relay hosts must never accept an object they cannot
// transmit on the authenticated neighbor session.
pub const MAX_ANNOUNCEMENT: usize = 3998;
pub const MAX_RECEIPT: usize = 1024;
pub const MAX_RECEIPT_ACK: usize = 1024;
pub const MAX_ENROLLMENT_REQUEST: usize = 512;
pub const MAX_PRESENCE_CLAIM: usize = 256;

/// Canonical application record carried inside an already authenticated Noise
/// session. This is deliberately separate from the session frame: hosts must
/// persist/verify the announced object before forwarding it, and a receipt is
/// never mistaken for an ordinary chat payload.
pub const DURABLE_RECORD_MAGIC: u8 = 0x6d;
pub const DURABLE_RECORD_VERSION: u8 = 1;
pub const MAX_DURABLE_RECORD: usize = MAX_ANNOUNCEMENT + 5;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DurableRecord {
    Announcement(Vec<u8>),
    Chunk {
        object: ObjectId,
        index: u16,
        bytes: Vec<u8>,
    },
    Receipt(Vec<u8>),
    ReceiptAck(Vec<u8>),
}

impl DurableRecord {
    /// The record only establishes canonical boundaries and bounds. Callers
    /// must authenticate announcements/receipts against the verified roster,
    /// and verify each chunk against the announced manifest before custody.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut out = vec![DURABLE_RECORD_MAGIC, DURABLE_RECORD_VERSION];
        match self {
            Self::Announcement(bytes) => {
                if bytes.is_empty() || bytes.len() > MAX_ANNOUNCEMENT {
                    return Err(DurableError::InvalidInput);
                }
                out.push(1);
                out.extend((bytes.len() as u16).to_be_bytes());
                out.extend(bytes);
            }
            Self::Chunk {
                object,
                index,
                bytes,
            } => {
                if bytes.is_empty() || bytes.len() > CHUNK_BYTES {
                    return Err(DurableError::InvalidInput);
                }
                out.push(2);
                out.extend(object.0);
                out.extend(index.to_be_bytes());
                out.extend((bytes.len() as u16).to_be_bytes());
                out.extend(bytes);
            }
            Self::Receipt(bytes) => {
                if bytes.is_empty() || bytes.len() > MAX_RECEIPT {
                    return Err(DurableError::InvalidInput);
                }
                out.push(3);
                out.extend((bytes.len() as u16).to_be_bytes());
                out.extend(bytes);
            }
            Self::ReceiptAck(bytes) => {
                if bytes.is_empty() || bytes.len() > MAX_RECEIPT_ACK {
                    return Err(DurableError::InvalidInput);
                }
                out.push(4);
                out.extend((bytes.len() as u16).to_be_bytes());
                out.extend(bytes);
            }
        }
        Ok(out)
    }

    pub fn decode(input: &[u8]) -> Result<Self> {
        if input.len() < 3
            || input.len() > MAX_DURABLE_RECORD
            || input[0] != DURABLE_RECORD_MAGIC
            || input[1] != DURABLE_RECORD_VERSION
        {
            return Err(DurableError::InvalidInput);
        }
        match input[2] {
            1 | 3 | 4 => {
                if input.len() < 5 {
                    return Err(DurableError::InvalidInput);
                }
                let len = u16::from_be_bytes([input[3], input[4]]) as usize;
                let limit = match input[2] {
                    1 => MAX_ANNOUNCEMENT,
                    3 => MAX_RECEIPT,
                    4 => MAX_RECEIPT_ACK,
                    _ => return Err(DurableError::UnsupportedSchema),
                };
                if len == 0 || len > limit || input.len() != 5 + len {
                    return Err(DurableError::InvalidInput);
                }
                let bytes = input[5..].to_vec();
                match input[2] {
                    1 => Ok(Self::Announcement(bytes)),
                    3 => Ok(Self::Receipt(bytes)),
                    4 => Ok(Self::ReceiptAck(bytes)),
                    _ => Err(DurableError::UnsupportedSchema),
                }
            }
            2 => {
                if input.len() < 39 {
                    return Err(DurableError::InvalidInput);
                }
                let object = ObjectId(
                    input[3..35]
                        .try_into()
                        .map_err(|_| DurableError::InvalidInput)?,
                );
                let index = u16::from_be_bytes([input[35], input[36]]);
                let len = u16::from_be_bytes([input[37], input[38]]) as usize;
                if len == 0 || len > CHUNK_BYTES || input.len() != 39 + len {
                    return Err(DurableError::InvalidInput);
                }
                Ok(Self::Chunk {
                    object,
                    index,
                    bytes: input[39..].to_vec(),
                })
            }
            _ => Err(DurableError::UnsupportedSchema),
        }
    }
}
/// A 50-member certified roster with revocations, carried over an authenticated
/// transport. A QR invitation is deliberately a short-lived token, not this
/// complete bundle.
pub const MAX_POLICY_BUNDLE: usize = 64 * 1024;
/// A bounded public endorsement that lets the next Convoy leader replace the
/// authority without ever copying the prior leader's private key.
pub const MAX_AUTHORITY_HANDOFF: usize = 512;
fn auth(_: crypto::CryptoError) -> DurableError {
    DurableError::AuthenticationFailed
}
fn random(_: crypto::CryptoError) -> DurableError {
    DurableError::CryptoUnavailable
}
fn version(v: u64) -> Result<()> {
    if v != 1 {
        return Err(DurableError::UnsupportedSchema);
    }
    Ok(())
}
fn time(now: u64) -> Result<()> {
    if now > MAX_LOGICAL_TIME {
        return Err(DurableError::InvalidInput);
    }
    Ok(())
}
fn domain_hash(label: &[u8], bytes: &[u8]) -> [u8; 32] {
    let mut b = label.to_vec();
    b.extend(bytes);
    digest(&b)
}
pub fn delivery_key_id(public: [u8; 32]) -> [u8; 32] {
    domain_hash(b"MeshLab/DeliveryKeyId/v1\0", &public)
}
fn signed(body: &[u8], signature: [u8; 64]) -> Vec<u8> {
    let mut w = Writer::default();
    w.array(2);
    w.bytes(body);
    w.bytes(&signature);
    w.finish()
}
fn signed_parts(bytes: &[u8], max: usize) -> Result<(&[u8], [u8; 64])> {
    let mut r = Reader::new(bytes, max)?;
    r.array(2)?;
    let body = r.bytes(max)?;
    let sig = r.fixed()?;
    r.end()?;
    Ok((body, sig))
}

/// Public, short-lived request used by a second device to prove possession of
/// its keys before the group authority issues a membership certificate. It does
/// not grant access by itself and contains no private material.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnrollmentRequest {
    pub scope: Scope,
    pub member: MemberId,
    pub signing_key: [u8; 32],
    pub delivery_key: [u8; 32],
    pub valid_from: u64,
    pub valid_until: u64,
    pub nonce: [u8; 16],
}
fn enrollment_body(request: &EnrollmentRequest) -> Result<Vec<u8>> {
    if request.scope.epoch == 0
        || request.scope.epoch > MAX_LOGICAL_TIME
        || request.valid_from >= request.valid_until
        || request.valid_until > MAX_LOGICAL_TIME
        || request.member.0 != request.signing_key
    {
        return Err(DurableError::InvalidInput);
    }
    let mut writer = Writer::default();
    writer.array(9);
    writer.uint(1);
    writer.bytes(&request.scope.group);
    writer.uint(request.scope.epoch);
    writer.bytes(&request.member.0);
    writer.bytes(&request.signing_key);
    writer.bytes(&request.delivery_key);
    writer.uint(request.valid_from);
    writer.uint(request.valid_until);
    writer.bytes(&request.nonce);
    Ok(writer.finish())
}
pub fn create_enrollment_request(
    signer: &IdentitySigningKey,
    delivery: &DeliverySecret,
    scope: Scope,
    valid_from: u64,
    valid_until: u64,
    rng: &mut dyn RandomSource,
) -> Result<Vec<u8>> {
    let mut nonce = [0; 16];
    rng.fill(&mut nonce).map_err(random)?;
    let signing_key = signer.public_key();
    let request = EnrollmentRequest {
        scope,
        member: MemberId(signing_key),
        signing_key,
        delivery_key: delivery.public_key(),
        valid_from,
        valid_until,
        nonce,
    };
    let body = enrollment_body(&request)?;
    Ok(signed(
        &body,
        signer
            .sign(scope, Domain::Enrollment, &body)
            .map_err(auth)?,
    ))
}
pub fn verify_enrollment_request(bytes: &[u8], now: u64) -> Result<EnrollmentRequest> {
    time(now)?;
    let (body, signature) = signed_parts(bytes, MAX_ENROLLMENT_REQUEST)?;
    let mut reader = Reader::new(body, MAX_ENROLLMENT_REQUEST)?;
    reader.array(9)?;
    version(reader.uint()?)?;
    let request = EnrollmentRequest {
        scope: Scope {
            group: reader.fixed()?,
            epoch: reader.uint()?,
        },
        member: MemberId(reader.fixed()?),
        signing_key: reader.fixed()?,
        delivery_key: reader.fixed()?,
        valid_from: reader.uint()?,
        valid_until: reader.uint()?,
        nonce: reader.fixed()?,
    };
    reader.end()?;
    enrollment_body(&request)?;
    if now < request.valid_from || now >= request.valid_until {
        return Err(DurableError::AuthenticationFailed);
    }
    crypto::verify(
        request.signing_key,
        request.scope,
        Domain::Enrollment,
        body,
        signature,
    )
    .map_err(auth)?;
    Ok(request)
}

/// Signed liveness fact from one certified group member. Relays preserve these
/// exact bytes; they never manufacture a new origin sequence for another phone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PresenceRecord {
    pub member: MemberId,
    pub incarnation: u64,
    pub sequence: u64,
    pub expires_at: u64,
}
fn presence_body(scope: Scope, record: PresenceRecord) -> Result<Vec<u8>> {
    if scope.epoch == 0
        || scope.epoch > MAX_LOGICAL_TIME
        || record.incarnation == 0
        || record.incarnation > MAX_LOGICAL_TIME
        || record.sequence == 0
        || record.sequence > MAX_LOGICAL_TIME
        || record.expires_at == 0
        || record.expires_at > MAX_LOGICAL_TIME
    {
        return Err(DurableError::InvalidInput);
    }
    let mut writer = Writer::default();
    writer.array(7);
    writer.uint(1);
    writer.bytes(&scope.group);
    writer.uint(scope.epoch);
    writer.bytes(&record.member.0);
    writer.uint(record.incarnation);
    writer.uint(record.sequence);
    writer.uint(record.expires_at);
    Ok(writer.finish())
}

pub fn sign_presence(
    signer: &IdentitySigningKey,
    record: PresenceRecord,
    roster: &VerifiedRoster,
    now: u64,
) -> Result<Vec<u8>> {
    roster.validate_at(now)?;
    if roster.member(record.member)?.signing_key != signer.public_key()
        || record.expires_at <= now
        || record.expires_at > roster.valid_until
    {
        return Err(DurableError::AuthenticationFailed);
    }
    let body = presence_body(roster.scope, record)?;
    Ok(signed(
        &body,
        signer
            .sign(roster.scope, Domain::Presence, &body)
            .map_err(auth)?,
    ))
}

pub fn authenticate_presence(
    encoded: &[u8],
    roster: &VerifiedRoster,
    now: u64,
) -> Result<PresenceRecord> {
    roster.validate_at(now)?;
    let (body, signature) = signed_parts(encoded, MAX_PRESENCE_CLAIM)?;
    let mut reader = Reader::new(body, MAX_PRESENCE_CLAIM)?;
    reader.array(7)?;
    version(reader.uint()?)?;
    let group: [u8; 32] = reader.fixed()?;
    let epoch = reader.uint()?;
    let record = PresenceRecord {
        member: MemberId(reader.fixed()?),
        incarnation: reader.uint()?,
        sequence: reader.uint()?,
        expires_at: reader.uint()?,
    };
    reader.end()?;
    if group != roster.scope.group || epoch != roster.scope.epoch || record.expires_at <= now {
        return Err(DurableError::AuthenticationFailed);
    }
    let canonical = presence_body(roster.scope, record)?;
    if canonical != body {
        return Err(DurableError::InvalidInput);
    }
    crypto::verify(
        roster.member(record.member)?.signing_key,
        roster.scope,
        Domain::Presence,
        body,
        signature,
    )
    .map_err(auth)?;
    Ok(record)
}

/// Canonical public transport for a verified roster. Certificates remain signed
/// by the authority; this container adds deterministic bounds for QR or BLE.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyBundle {
    pub authority: [u8; 32],
    pub scope: Scope,
    pub certificates: Vec<Vec<u8>>,
    pub revoked: Vec<u64>,
}
impl PolicyBundle {
    pub fn verify(&self, now: u64) -> Result<VerifiedRoster> {
        VerifiedRoster::verify(
            self.authority,
            self.scope,
            &self.certificates,
            &self.revoked,
            now,
        )
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        if self.scope.epoch == 0
            || self.scope.epoch > MAX_LOGICAL_TIME
            || self.certificates.is_empty()
            || self.certificates.len() > MAX_GROUP_MEMBERS
            || self
                .certificates
                .iter()
                .any(|certificate| certificate.is_empty() || certificate.len() > 1024)
            || self.revoked.len() > 1024
            || self
                .revoked
                .iter()
                .any(|serial| *serial == 0 || *serial > MAX_LOGICAL_TIME)
        {
            return Err(DurableError::InvalidInput);
        }
        let mut writer = Writer::default();
        writer.array(6);
        writer.uint(1);
        writer.bytes(&self.authority);
        writer.bytes(&self.scope.group);
        writer.uint(self.scope.epoch);
        writer.array(self.certificates.len());
        for certificate in &self.certificates {
            writer.bytes(certificate);
        }
        writer.array(self.revoked.len());
        for serial in &self.revoked {
            writer.uint(*serial);
        }
        let encoded = writer.finish();
        if encoded.len() > MAX_POLICY_BUNDLE {
            return Err(DurableError::ResourcePressure);
        }
        Ok(encoded)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut reader = Reader::new(bytes, MAX_POLICY_BUNDLE)?;
        reader.array(6)?;
        version(reader.uint()?)?;
        let authority = reader.fixed()?;
        let scope = Scope {
            group: reader.fixed()?,
            epoch: reader.uint()?,
        };
        let certificate_count = reader.count(MAX_GROUP_MEMBERS)?;
        let mut certificates = Vec::with_capacity(certificate_count);
        for _ in 0..certificate_count {
            certificates.push(reader.bytes(1024)?.to_vec());
        }
        let revoked_count = reader.count(1024)?;
        let mut revoked = Vec::with_capacity(revoked_count);
        for _ in 0..revoked_count {
            revoked.push(reader.uint()?);
        }
        reader.end()?;
        let bundle = Self {
            authority,
            scope,
            certificates,
            revoked,
        };
        bundle.encode()?;
        Ok(bundle)
    }
}

/// Signed authorization from an active authority to a specific successor.
/// The successor must still possess its own protected identity and reissue the
/// next policy epoch; this record is never a transferable private credential.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorityHandoff {
    pub prior_authority: [u8; 32],
    pub prior_scope: Scope,
    pub prior_roster_digest: [u8; 32],
    pub next_authority: [u8; 32],
    pub next_epoch: u64,
    pub valid_until: u64,
    signature: [u8; 64],
}

impl AuthorityHandoff {
    fn body(&self) -> Result<Vec<u8>> {
        if self.prior_scope.epoch == 0
            || self.prior_scope.epoch > MAX_LOGICAL_TIME
            || self.next_epoch
                != self
                    .prior_scope
                    .epoch
                    .checked_add(1)
                    .ok_or(DurableError::InvalidInput)?
            || self.valid_until == 0
            || self.valid_until > MAX_LOGICAL_TIME
            || self.next_authority == self.prior_authority
        {
            return Err(DurableError::InvalidInput);
        }
        let mut writer = Writer::default();
        writer.array(8);
        writer.uint(1);
        writer.bytes(&self.prior_authority);
        writer.bytes(&self.prior_scope.group);
        writer.uint(self.prior_scope.epoch);
        writer.bytes(&self.prior_roster_digest);
        writer.bytes(&self.next_authority);
        writer.uint(self.next_epoch);
        writer.uint(self.valid_until);
        Ok(writer.finish())
    }

    /// Serializes the public signed record. It may cross a backend or an
    /// authenticated radio link, but cannot be replayed against a different
    /// authority, group, epoch, roster, successor, or expiry.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let body = self.body()?;
        let encoded = signed(&body, self.signature);
        if encoded.len() > MAX_AUTHORITY_HANDOFF {
            return Err(DurableError::ResourcePressure);
        }
        Ok(encoded)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let (body, signature) = signed_parts(bytes, MAX_AUTHORITY_HANDOFF)?;
        let mut reader = Reader::new(body, MAX_AUTHORITY_HANDOFF)?;
        reader.array(8)?;
        version(reader.uint()?)?;
        let handoff = Self {
            prior_authority: reader.fixed()?,
            prior_scope: Scope {
                group: reader.fixed()?,
                epoch: reader.uint()?,
            },
            prior_roster_digest: reader.fixed()?,
            next_authority: reader.fixed()?,
            next_epoch: reader.uint()?,
            valid_until: reader.uint()?,
            signature,
        };
        reader.end()?;
        handoff.encode()?;
        Ok(handoff)
    }

    /// Checks the old authority signature and every piece of state that the
    /// receiver pinned in its SQLCipher policy snapshot.
    pub fn verify_for(
        &self,
        authority: [u8; 32],
        scope: Scope,
        roster_digest: [u8; 32],
        now: u64,
    ) -> Result<()> {
        time(now)?;
        if now >= self.valid_until
            || self.prior_authority != authority
            || self.prior_scope != scope
            || self.prior_roster_digest != roster_digest
        {
            return Err(DurableError::AuthenticationFailed);
        }
        let body = self.body()?;
        crypto::verify(authority, scope, Domain::Transition, &body, self.signature).map_err(auth)
    }
}

/// Produces a one-time handoff for the authority currently certified in the
/// policy snapshot. The caller supplies the successor's public Field identity.
pub fn issue_authority_handoff(
    authority: &IdentitySigningKey,
    scope: Scope,
    roster_digest: [u8; 32],
    next_authority: [u8; 32],
    valid_until: u64,
) -> Result<AuthorityHandoff> {
    let next_epoch = scope
        .epoch
        .checked_add(1)
        .ok_or(DurableError::InvalidInput)?;
    let mut handoff = AuthorityHandoff {
        prior_authority: authority.public_key(),
        prior_scope: scope,
        prior_roster_digest: roster_digest,
        next_authority,
        next_epoch,
        valid_until,
        signature: [0; 64],
    };
    let body = handoff.body()?;
    handoff.signature = authority
        .sign(scope, Domain::Transition, &body)
        .map_err(auth)?;
    Ok(handoff)
}

/// Fixed lab role: send/receive durable messages. Invitations, roles/capabilities
/// negotiation and authority recovery are intentionally outside this profile.
#[derive(Clone, Debug)]
pub struct CertificateClaims {
    pub group: [u8; 32],
    pub member: MemberId,
    pub signing_key: [u8; 32],
    pub delivery_key: [u8; 32],
    pub valid_from: u64,
    pub valid_until: u64,
    pub epoch: u64,
    pub serial: u64,
}
fn cert_body(c: &CertificateClaims, authority: [u8; 32]) -> Result<Vec<u8>> {
    if c.epoch == 0
        || c.epoch > MAX_LOGICAL_TIME
        || c.serial == 0
        || c.serial > MAX_LOGICAL_TIME
        || c.valid_from >= c.valid_until
        || c.valid_until > MAX_LOGICAL_TIME
    {
        return Err(DurableError::InvalidInput);
    }
    let mut w = Writer::default();
    w.array(10);
    w.uint(1);
    w.bytes(&c.group);
    w.bytes(&c.member.0);
    w.bytes(&c.signing_key);
    w.bytes(&c.delivery_key);
    w.uint(c.valid_from);
    w.uint(c.valid_until);
    w.uint(c.epoch);
    w.uint(c.serial);
    w.bytes(&crypto::identity_key_id(authority));
    Ok(w.finish())
}
pub fn issue_certificate(
    authority: &IdentitySigningKey,
    claims: &CertificateClaims,
) -> Result<Vec<u8>> {
    let b = cert_body(claims, authority.public_key())?;
    Ok(signed(
        &b,
        authority
            .sign(
                Scope {
                    group: claims.group,
                    epoch: claims.epoch,
                },
                Domain::Certificate,
                &b,
            )
            .map_err(auth)?,
    ))
}
/// A trusted host must provide the pinned authority, current epoch and known
/// revocations. Never populate these values from an untrusted radio announcement.
pub struct VerifiedRoster {
    scope: Scope,
    members: BTreeMap<MemberId, CertificateClaims>,
    digest: [u8; 32],
    valid_from: u64,
    valid_until: u64,
}
impl VerifiedRoster {
    pub fn verify(
        authority: [u8; 32],
        scope: Scope,
        certificates: &[Vec<u8>],
        revoked_serials: &[u64],
        now: u64,
    ) -> Result<Self> {
        time(now)?;
        if scope.epoch == 0
            || scope.epoch > MAX_LOGICAL_TIME
            || certificates.is_empty()
            || certificates.len() > MAX_GROUP_MEMBERS
            || revoked_serials.len() > 1024
        {
            return Err(DurableError::InvalidInput);
        }
        let revoked: BTreeSet<_> = revoked_serials.iter().copied().collect();
        let mut members = BTreeMap::new();
        let mut keys = BTreeSet::new();
        let mut delivery_keys = BTreeSet::new();
        let mut serials = BTreeSet::new();
        let mut canonical = BTreeMap::new();
        let mut valid_from = 0;
        let mut valid_until = MAX_LOGICAL_TIME;
        for encoded in certificates {
            let (body, sig) = signed_parts(encoded, 1024)?;
            let mut r = Reader::new(body, 900)?;
            r.array(10)?;
            version(r.uint()?)?;
            let c = CertificateClaims {
                group: r.fixed()?,
                member: MemberId(r.fixed()?),
                signing_key: r.fixed()?,
                delivery_key: r.fixed()?,
                valid_from: r.uint()?,
                valid_until: r.uint()?,
                epoch: r.uint()?,
                serial: r.uint()?,
            };
            let authority_id: [u8; 32] = r.fixed()?;
            r.end()?;
            if authority_id != crypto::identity_key_id(authority)
                || c.group != scope.group
                || c.epoch != scope.epoch
                || revoked.contains(&c.serial)
                || now < c.valid_from
                || now >= c.valid_until
                || cert_body(&c, authority)? != body
            {
                return Err(DurableError::AuthenticationFailed);
            }
            crypto::verify(authority, scope, Domain::Certificate, body, sig).map_err(auth)?;
            if !keys.insert(c.signing_key)
                || !delivery_keys.insert(c.delivery_key)
                || !serials.insert(c.serial)
                || members.contains_key(&c.member)
            {
                return Err(DurableError::Conflict);
            }
            valid_from = valid_from.max(c.valid_from);
            valid_until = valid_until.min(c.valid_until);
            canonical.insert(c.member, encoded);
            members.insert(c.member, c);
        }
        let mut w = Writer::default();
        w.array(5);
        w.bytes(&authority);
        w.bytes(&scope.group);
        w.uint(scope.epoch);
        w.array(canonical.len());
        for cert in canonical.values() {
            w.bytes(cert)
        }
        w.array(revoked.len());
        for serial in revoked {
            w.uint(serial)
        }
        Ok(Self {
            scope,
            members,
            digest: domain_hash(b"MeshLab/Roster/v1\0", &w.finish()),
            valid_from,
            valid_until,
        })
    }
    pub fn scope(&self) -> Scope {
        self.scope
    }
    pub fn digest(&self) -> [u8; 32] {
        self.digest
    }
    pub fn validate_at(&self, now: u64) -> Result<()> {
        time(now)?;
        if now < self.valid_from || now >= self.valid_until {
            return Err(DurableError::AuthenticationFailed);
        }
        Ok(())
    }
    /// Only keys already authorized by the pinned authority in this snapshot.
    pub fn signing_key(&self, member: MemberId) -> Result<[u8; 32]> {
        Ok(self.member(member)?.signing_key)
    }
    /// A persisted policy is usable only by a device explicitly listed in it.
    pub fn contains_member(&self, member: MemberId) -> bool {
        self.members.contains_key(&member)
    }
    /// Claims are returned only after `VerifiedRoster::verify` has accepted all
    /// certificates. Callers reissue them when advancing an enrollment epoch.
    pub fn member_claims(&self) -> Vec<CertificateClaims> {
        self.members.values().cloned().collect()
    }
    fn member(&self, id: MemberId) -> Result<&CertificateClaims> {
        self.members
            .get(&id)
            .ok_or(DurableError::AuthenticationFailed)
    }
    fn recipient(&self, id: MemberId) -> Result<Recipient> {
        let c = self.member(id)?;
        Ok(Recipient {
            member: id.0,
            delivery_key_id: delivery_key_id(c.delivery_key),
            public_key: c.delivery_key,
        })
    }
}

pub struct AuthenticatedAnnouncement {
    manifest: Manifest,
    encoded: Vec<u8>,
    scope: Scope,
    roster_digest: [u8; 32],
}
impl AuthenticatedAnnouncement {
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    pub fn bytes(&self) -> &[u8] {
        &self.encoded
    }
    pub fn scope(&self) -> Scope {
        self.scope
    }
    pub fn validate_roster(&self, roster: &VerifiedRoster, now: u64) -> Result<()> {
        roster.validate_at(now)?;
        if self.roster_digest != roster.digest {
            return Err(DurableError::AuthenticationFailed);
        }
        Ok(())
    }
}
pub fn authenticate_announcement(
    encoded: &[u8],
    roster: &VerifiedRoster,
    now: u64,
) -> Result<AuthenticatedAnnouncement> {
    roster.validate_at(now)?;
    let (body, sig) = signed_parts(encoded, MAX_ANNOUNCEMENT)?;
    let mut r = Reader::new(body, MAX_ANNOUNCEMENT)?;
    r.array(3)?;
    version(r.uint()?)?;
    let group: [u8; 32] = r.fixed()?;
    let manifest = Manifest::decode(r.bytes(MAX_MANIFEST_BYTES)?)?;
    r.end()?;
    if group != roster.scope.group || manifest.epoch() != roster.scope.epoch {
        return Err(DurableError::AuthenticationFailed);
    }
    for target in manifest.targets() {
        roster.member(*target)?;
    }
    crypto::verify(
        roster.member(manifest.origin())?.signing_key,
        roster.scope,
        Domain::Object,
        body,
        sig,
    )
    .map_err(auth)?;
    Ok(AuthenticatedAnnouncement {
        manifest,
        encoded: encoded.to_vec(),
        scope: roster.scope,
        roster_digest: roster.digest,
    })
}
pub struct SealedMessage {
    object: PreparedObject,
    announcement: AuthenticatedAnnouncement,
}
impl SealedMessage {
    pub fn object(&self) -> &PreparedObject {
        &self.object
    }
    pub fn announcement(&self) -> &AuthenticatedAnnouncement {
        &self.announcement
    }
}
struct Header {
    scope: Scope,
    origin: MemberId,
    sequence: u64,
    namespace: Namespace,
    created: u64,
    expires: u64,
    hops: u8,
    targets: Vec<(MemberId, [u8; 32])>,
    clear_len: usize,
    salt: [u8; 32],
}
impl Header {
    fn encode(&self) -> Vec<u8> {
        let mut w = Writer::default();
        w.array(14);
        w.uint(1);
        w.uint(1);
        w.bytes(&self.scope.group);
        w.bytes(&self.origin.0);
        w.uint(self.sequence);
        w.uint(self.scope.epoch);
        w.text(self.namespace.as_str());
        w.uint(1);
        w.uint(self.created);
        w.uint(self.expires);
        w.uint(self.hops as u64);
        w.array(self.targets.len());
        for (member, key) in &self.targets {
            w.array(2);
            w.bytes(&member.0);
            w.bytes(key)
        }
        w.uint(self.clear_len as u64);
        w.bytes(&self.salt);
        w.finish()
    }
    fn decode(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes, 2048)?;
        r.array(14)?;
        version(r.uint()?)?;
        version(r.uint()?)?;
        let group = r.fixed()?;
        let origin = MemberId(r.fixed()?);
        let sequence = r.uint()?;
        let epoch = r.uint()?;
        let namespace = Namespace::new(r.text(64)?)?;
        version(r.uint()?)?;
        let created = r.uint()?;
        let expires = r.uint()?;
        let hops = u8::try_from(r.uint()?).map_err(|_| DurableError::InvalidInput)?;
        let count = r.count(MAX_TARGETS)?;
        let mut targets = Vec::new();
        for _ in 0..count {
            r.array(2)?;
            targets.push((MemberId(r.fixed()?), r.fixed()?));
        }
        let clear_len = usize::try_from(r.uint()?).map_err(|_| DurableError::InvalidInput)?;
        let salt = r.fixed()?;
        r.end()?;
        if sequence == 0
            || sequence > MAX_LOGICAL_TIME
            || epoch == 0
            || epoch > MAX_LOGICAL_TIME
            || created >= expires
            || expires > MAX_LOGICAL_TIME
            || hops == 0
            || hops > 16
            || targets.is_empty()
            || !targets.windows(2).all(|t| t[0].0 < t[1].0)
            || clear_len == 0
            || clear_len > MAX_PLAINTEXT
        {
            return Err(DurableError::InvalidInput);
        }
        Ok(Self {
            scope: Scope { group, epoch },
            origin,
            sequence,
            namespace,
            created,
            expires,
            hops,
            targets,
            clear_len,
            salt,
        })
    }
    fn context_id(&self) -> [u8; 32] {
        domain_hash(b"MeshLab/EnvelopeContext/v1\0", &self.encode())
    }
    fn context(&self) -> ChunkContext {
        let mut audience = Writer::default();
        audience.array(self.targets.len());
        for (m, k) in &self.targets {
            audience.array(2);
            audience.bytes(&m.0);
            audience.bytes(k)
        }
        ChunkContext {
            scope: self.scope,
            object_context_id: self.context_id(),
            origin: self.origin.0,
            origin_sequence: self.sequence,
            audience_digest: domain_hash(b"MeshLab/Audience/v1\0", &audience.finish()),
            namespace: self.namespace.as_str().into(),
            schema: 1,
            chunk_count: self.clear_len.div_ceil(CHUNK_BYTES) as u32,
            lifetime: self.expires - self.created,
            priority: 1,
        }
    }
    fn matches(&self, a: &AuthenticatedAnnouncement, roster: &VerifiedRoster) -> Result<()> {
        let m = a.manifest();
        if self.scope != a.scope
            || self.origin != m.origin()
            || self.sequence != m.sequence()
            || self.namespace != *m.namespace()
            || self.expires != m.expires_at()
            || self.hops != m.hop_limit()
            || self.targets.iter().map(|(m, _)| *m).collect::<Vec<_>>() != m.targets()
        {
            return Err(DurableError::AuthenticationFailed);
        }
        for (member, key) in &self.targets {
            if *key != roster.recipient(*member)?.delivery_key_id {
                return Err(DurableError::AuthenticationFailed);
            }
        }
        Ok(())
    }
}
/// Produces a complete protected envelope plus a signed storage announcement.
/// The host reserves the global origin sequence before invoking this operation.
pub struct SealRequest<'a> {
    pub origin: MemberId,
    pub sequence: u64,
    pub policy: ObjectPolicy,
    pub plaintext: &'a [u8],
    pub now: u64,
}
pub fn seal_message(
    request: SealRequest<'_>,
    roster: &VerifiedRoster,
    signer: &IdentitySigningKey,
    rng: &mut dyn RandomSource,
) -> Result<SealedMessage> {
    let SealRequest {
        origin,
        sequence,
        mut policy,
        plaintext,
        now,
    } = request;
    roster.validate_at(now)?;
    if roster.member(origin)?.signing_key != signer.public_key()
        || policy.epoch != roster.scope.epoch
    {
        return Err(DurableError::AuthenticationFailed);
    }
    if plaintext.is_empty()
        || plaintext.len() > MAX_PLAINTEXT
        || policy.targets.is_empty()
        || policy.targets.len() > MAX_TARGETS
        || policy.expires_at <= now
    {
        return Err(DurableError::InvalidInput);
    }
    policy.targets.sort();
    if !policy.targets.windows(2).all(|p| p[0] < p[1]) {
        return Err(DurableError::InvalidInput);
    }
    let mut targets = Vec::new();
    for member in &policy.targets {
        targets.push((*member, roster.recipient(*member)?.delivery_key_id));
    }
    let mut salt = [0; 32];
    rng.fill(&mut salt).map_err(random)?;
    let h = Header {
        scope: roster.scope,
        origin,
        sequence,
        namespace: policy.namespace.clone(),
        created: now,
        expires: policy.expires_at,
        hops: policy.hop_limit,
        targets,
        clear_len: plaintext.len(),
        salt,
    };
    let header = h.encode();
    Header::decode(&header)?;
    let mut sealer = ContentSealer::generate(h.context(), rng).map_err(random)?;
    let mut w = Writer::default();
    w.array(5);
    w.uint(1);
    w.bytes(&header);
    w.bytes(&sealer.base_nonce());
    w.array(h.targets.len());
    for (member, _) in &h.targets {
        let wrap = sealer
            .wrap_for(roster.recipient(*member)?, rng)
            .map_err(random)?;
        w.array(2);
        w.bytes(&wrap.encapsulated);
        w.bytes(&wrap.ciphertext)
    }
    w.array(h.context().chunk_count as usize);
    for chunk in plaintext.chunks(CHUNK_BYTES) {
        w.bytes(&sealer.seal_next(chunk).map_err(random)?)
    }
    let object = PreparedObject::from_opaque(origin, sequence, policy, &w.finish())?;
    let mut b = Writer::default();
    b.array(3);
    b.uint(1);
    b.bytes(&roster.scope.group);
    b.bytes(&object.manifest().encode());
    let body = b.finish();
    let bytes = signed(
        &body,
        signer
            .sign(roster.scope, Domain::Object, &body)
            .map_err(auth)?,
    );
    let announcement = authenticate_announcement(&bytes, roster, now)?;
    Ok(SealedMessage {
        object,
        announcement,
    })
}
struct Envelope<'a> {
    header: Header,
    nonce: [u8; 12],
    wraps: Vec<RecipientWrap>,
    chunks: Vec<&'a [u8]>,
}
fn envelope<'a>(
    a: &AuthenticatedAnnouncement,
    bytes: &'a [u8],
    roster: &VerifiedRoster,
) -> Result<Envelope<'a>> {
    if bytes.len() != a.manifest.content_len() {
        return Err(DurableError::Corrupt);
    }
    for (i, b) in bytes.chunks(CHUNK_BYTES).enumerate() {
        a.manifest.verify_chunk(i, b)?;
    }
    let mut r = Reader::new(bytes, MAX_OBJECT_BYTES)?;
    r.array(5)?;
    version(r.uint()?)?;
    let h = Header::decode(r.bytes(2048)?)?;
    h.matches(a, roster)?;
    let nonce = r.fixed()?;
    let count = r.count(MAX_TARGETS)?;
    if count != h.targets.len() {
        return Err(DurableError::InvalidInput);
    }
    let mut wraps = Vec::new();
    for _ in 0..count {
        r.array(2)?;
        wraps.push(RecipientWrap {
            encapsulated: r.fixed()?,
            ciphertext: r.fixed()?,
        });
    }
    let count = r.count(MAX_CHUNKS)?;
    if count != h.context().chunk_count as usize {
        return Err(DurableError::InvalidInput);
    }
    let mut chunks = Vec::new();
    for i in 0..count {
        let b = r.bytes(CHUNK_BYTES + 16)?;
        let clear_len = (h.clear_len - i * CHUNK_BYTES).min(CHUNK_BYTES);
        if b.len() != clear_len + 16 {
            return Err(DurableError::InvalidInput);
        }
        chunks.push(b)
    }
    r.end()?;
    Ok(Envelope {
        header: h,
        nonce,
        wraps,
        chunks,
    })
}
/// Constructed only after signature, manifest, metadata, recipient binding and ALL
/// AEAD chunks validate. Holds no exposed plaintext and cannot be constructed by callers.
pub struct VerifiedDelivery {
    announcement: AuthenticatedAnnouncement,
    member: MemberId,
    signing_key: [u8; 32],
    context_id: [u8; 32],
    verified_at: u64,
    deadline: u64,
    // This stays in the proof until the store has atomically recorded the
    // receipt. Native hosts must not make it visible before `finalize_received`
    // succeeds.
    plaintext: Zeroizing<Vec<u8>>,
}
impl VerifiedDelivery {
    pub fn object_id(&self) -> ObjectId {
        self.announcement.manifest.id()
    }
    /// Certified origin of the durable object. This is distinct from
    /// [member], which is the local recipient whose delivery key was used.
    pub fn origin(&self) -> MemberId {
        self.announcement.manifest().origin()
    }
    pub fn member(&self) -> MemberId {
        self.member
    }
    pub fn announcement(&self) -> &AuthenticatedAnnouncement {
        &self.announcement
    }
    pub fn verified_at(&self) -> u64 {
        self.verified_at
    }
    /// The caller may read this only after its associated `finalize_received`
    /// transaction has committed. Keeping it on the verified proof prevents a
    /// host from accidentally decrypting one object and committing another.
    pub fn plaintext_after_commit(&self) -> &[u8] {
        &self.plaintext
    }
    pub fn validate_commit(&self, roster: &VerifiedRoster, now: u64) -> Result<()> {
        self.announcement.validate_roster(roster, now)?;
        if now < self.verified_at || now >= self.deadline {
            return Err(DurableError::Expired);
        }
        Ok(())
    }
    /// Storage calls this inside its transaction and publishes only AFTER commit.
    pub fn receipt_for_commit(&self, signer: &IdentitySigningKey, now: u64) -> Result<Vec<u8>> {
        if signer.public_key() != self.signing_key || now < self.verified_at || now >= self.deadline
        {
            return Err(DurableError::AuthenticationFailed);
        }
        let m = self.announcement.manifest();
        let body = receipt_body(
            self.announcement.scope,
            self.context_id,
            m.id(),
            m.origin(),
            m.sequence(),
            self.member,
            now,
        );
        Ok(signed(
            &body,
            signer
                .sign(self.announcement.scope, Domain::Receipt, &body)
                .map_err(auth)?,
        ))
    }
}
pub fn verify_delivery(
    a: AuthenticatedAnnouncement,
    bytes: &[u8],
    roster: &VerifiedRoster,
    member: MemberId,
    secret: &DeliverySecret,
    now: u64,
) -> Result<VerifiedDelivery> {
    a.validate_roster(roster, now)?;
    let e = envelope(&a, bytes, roster)?;
    if now < e.header.created || now >= e.header.expires {
        return Err(DurableError::Expired);
    }
    let i = e
        .header
        .targets
        .iter()
        .position(|(m, _)| *m == member)
        .ok_or(DurableError::AuthenticationFailed)?;
    let recipient = roster.recipient(member)?;
    let key = crypto::unwrap_key(
        secret,
        e.header.scope,
        e.header.context_id(),
        recipient,
        &e.wraps[i],
    )
    .map_err(auth)?;
    let mut plaintext = Zeroizing::new(Vec::with_capacity(e.header.clear_len));
    for (i, chunk) in e.chunks.iter().enumerate() {
        let clear = crypto::open_chunk(&key, &e.header.context(), e.nonce, i as u32, chunk)
            .map_err(auth)?;
        plaintext.extend_from_slice(&clear);
    }
    if plaintext.len() != e.header.clear_len {
        return Err(DurableError::Corrupt);
    }
    let context_id = e.header.context_id();
    let deadline = e.header.expires.min(roster.valid_until);
    Ok(VerifiedDelivery {
        announcement: a,
        member,
        signing_key: roster.member(member)?.signing_key,
        context_id,
        verified_at: now,
        deadline,
        plaintext,
    })
}
fn receipt_body(
    scope: Scope,
    context_id: [u8; 32],
    id: ObjectId,
    origin: MemberId,
    sequence: u64,
    actor: MemberId,
    at: u64,
) -> Vec<u8> {
    let mut w = Writer::default();
    w.array(10);
    w.uint(1);
    w.uint(1);
    w.bytes(&scope.group);
    w.uint(scope.epoch);
    w.bytes(&context_id);
    w.bytes(&id.0);
    w.bytes(&origin.0);
    w.uint(sequence);
    w.bytes(&actor.0);
    w.uint(at);
    w.finish()
}
pub struct VerifiedReceipt {
    object: ObjectId,
    actor: MemberId,
    bytes: Vec<u8>,
    announcement: Vec<u8>,
    roster_digest: [u8; 32],
    at: u64,
}
impl VerifiedReceipt {
    pub fn object_id(&self) -> ObjectId {
        self.object
    }
    pub fn actor(&self) -> MemberId {
        self.actor
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn announcement(&self) -> &[u8] {
        &self.announcement
    }
    pub fn validate_commit(&self, roster: &VerifiedRoster, now: u64) -> Result<()> {
        roster.validate_at(now)?;
        if roster.digest != self.roster_digest || now < self.at {
            return Err(DurableError::AuthenticationFailed);
        }
        Ok(())
    }
}
/// Public routing metadata parsed from a bounded receipt before a relay has
/// the original object needed to verify its signature. Relays use it only to
/// bind the outer hop origin; final validation remains `verify_receipt` at the
/// message origin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReceiptRoute {
    pub object: ObjectId,
    pub actor: MemberId,
}
pub fn receipt_route(encoded: &[u8]) -> Result<ReceiptRoute> {
    let (body, _) = signed_parts(encoded, MAX_RECEIPT)?;
    let mut r = Reader::new(body, MAX_RECEIPT)?;
    r.array(10)?;
    version(r.uint()?)?;
    version(r.uint()?)?;
    let _: [u8; 32] = r.fixed()?;
    let _ = r.uint()?;
    let _: [u8; 32] = r.fixed()?;
    let object = ObjectId(r.fixed()?);
    let _: MemberId = MemberId(r.fixed()?);
    let _ = r.uint()?;
    let actor = MemberId(r.fixed()?);
    let _ = r.uint()?;
    r.end()?;
    Ok(ReceiptRoute { object, actor })
}
pub fn verify_receipt(
    encoded: &[u8],
    announcement: &AuthenticatedAnnouncement,
    object_bytes: &[u8],
    roster: &VerifiedRoster,
    now: u64,
) -> Result<VerifiedReceipt> {
    announcement.validate_roster(roster, now)?;
    let e = envelope(announcement, object_bytes, roster)?;
    let (body, sig) = signed_parts(encoded, MAX_RECEIPT)?;
    let mut r = Reader::new(body, MAX_RECEIPT)?;
    r.array(10)?;
    version(r.uint()?)?;
    version(r.uint()?)?;
    let group: [u8; 32] = r.fixed()?;
    let epoch = r.uint()?;
    let context_id: [u8; 32] = r.fixed()?;
    let id = ObjectId(r.fixed()?);
    let origin = MemberId(r.fixed()?);
    let sequence = r.uint()?;
    let actor = MemberId(r.fixed()?);
    let at = r.uint()?;
    r.end()?;
    let m = announcement.manifest();
    if group != announcement.scope.group
        || epoch != announcement.scope.epoch
        || context_id != e.header.context_id()
        || id != m.id()
        || origin != m.origin()
        || sequence != m.sequence()
        || !m.targets().contains(&actor)
        || at < e.header.created
        || at >= e.header.expires
        || at > now
    {
        return Err(DurableError::InvalidReceipt);
    }
    let cert = roster.member(actor)?;
    if at < cert.valid_from || at >= cert.valid_until {
        return Err(DurableError::InvalidReceipt);
    }
    crypto::verify(
        cert.signing_key,
        announcement.scope,
        Domain::Receipt,
        body,
        sig,
    )
    .map_err(|_| DurableError::InvalidReceipt)?;
    Ok(VerifiedReceipt {
        object: id,
        actor,
        bytes: encoded.to_vec(),
        announcement: announcement.bytes().to_vec(),
        roster_digest: roster.digest,
        at,
    })
}

/// A receipt acknowledgement is issued only by the certified origin after its
/// SQLCipher transaction recorded the target receipt. It carries the receipt
/// digest rather than the complete proof, so relays can stop replaying a
/// matching receipt without handling more recipient material.
fn receipt_ack_body(
    scope: Scope,
    receipt_id: [u8; 32],
    object: ObjectId,
    origin: MemberId,
    actor: MemberId,
    at: u64,
) -> Vec<u8> {
    let mut w = Writer::default();
    w.array(9);
    w.uint(1);
    w.uint(1);
    w.bytes(&scope.group);
    w.uint(scope.epoch);
    w.bytes(&receipt_id);
    w.bytes(&object.0);
    w.bytes(&origin.0);
    w.bytes(&actor.0);
    w.uint(at);
    w.finish()
}

pub fn issue_receipt_ack(
    receipt: &[u8],
    object: ObjectId,
    origin: MemberId,
    actor: MemberId,
    scope: Scope,
    signer: &IdentitySigningKey,
    now: u64,
) -> Result<Vec<u8>> {
    time(now)?;
    if receipt.is_empty() || receipt.len() > MAX_RECEIPT || signer.public_key() != origin.0 {
        return Err(DurableError::InvalidReceipt);
    }
    let receipt_id = digest(receipt);
    let body = receipt_ack_body(scope, receipt_id, object, origin, actor, now);
    Ok(signed(
        &body,
        signer
            .sign(scope, Domain::ReceiptAck, &body)
            .map_err(auth)?,
    ))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReceiptAckRoute {
    pub receipt_id: [u8; 32],
    pub object: ObjectId,
    pub origin: MemberId,
    pub actor: MemberId,
}

pub struct VerifiedReceiptAck {
    route: ReceiptAckRoute,
    at: u64,
}
impl VerifiedReceiptAck {
    pub fn route(&self) -> ReceiptAckRoute {
        self.route
    }
    pub fn acknowledged_at(&self) -> u64 {
        self.at
    }
}

fn receipt_ack_parts(encoded: &[u8]) -> Result<(ReceiptAckRoute, Scope, u64, &[u8], [u8; 64])> {
    let (body, signature) = signed_parts(encoded, MAX_RECEIPT_ACK)?;
    let mut r = Reader::new(body, MAX_RECEIPT_ACK)?;
    r.array(9)?;
    version(r.uint()?)?;
    version(r.uint()?)?;
    let group: [u8; 32] = r.fixed()?;
    let epoch = r.uint()?;
    let receipt_id: [u8; 32] = r.fixed()?;
    let object = ObjectId(r.fixed()?);
    let origin = MemberId(r.fixed()?);
    let actor = MemberId(r.fixed()?);
    let at = r.uint()?;
    r.end()?;
    Ok((
        ReceiptAckRoute {
            receipt_id,
            object,
            origin,
            actor,
        },
        Scope { group, epoch },
        at,
        body,
        signature,
    ))
}

pub fn receipt_ack_route(encoded: &[u8]) -> Result<ReceiptAckRoute> {
    Ok(receipt_ack_parts(encoded)?.0)
}

pub fn verify_receipt_ack(
    encoded: &[u8],
    roster: &VerifiedRoster,
    now: u64,
) -> Result<VerifiedReceiptAck> {
    roster.validate_at(now)?;
    let (route, scope, at, body, signature) = receipt_ack_parts(encoded)?;
    if scope != roster.scope || at > now || at < roster.valid_from || at >= roster.valid_until {
        return Err(DurableError::InvalidReceipt);
    }
    let cert = roster.member(route.origin)?;
    if at < cert.valid_from || at >= cert.valid_until {
        return Err(DurableError::InvalidReceipt);
    }
    crypto::verify(cert.signing_key, scope, Domain::ReceiptAck, body, signature)
        .map_err(|_| DurableError::InvalidReceipt)?;
    Ok(VerifiedReceiptAck { route, at })
}

#[cfg(test)]
mod tests;
