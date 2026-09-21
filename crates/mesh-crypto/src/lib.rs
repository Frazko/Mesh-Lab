//! A010 primitive boundary. This is not a complete authenticated mesh protocol.
//! Callers supply canonical bytes and validated membership snapshots; public keys
//! alone do not establish membership. No secret type implements Clone/Debug/Serialize.
use chacha20poly1305::{
    aead::{AeadInPlace, KeyInit},
    ChaCha20Poly1305, Nonce,
};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use hpke::{
    aead::ChaCha20Poly1305 as HpkeAead, kdf::HkdfSha256, kem::X25519HkdfSha256, Deserializable,
    Kem as KemTrait, OpModeR, OpModeS, Serializable,
};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, Zeroizing};

type Kem = X25519HkdfSha256;
pub const SUITE_ID: u16 = 1;
pub const MAX_SIGNED_BYTES: usize = 65536;
pub const MAX_CHUNK_BYTES: usize = 1024;
pub const MAX_CHUNKS: u32 = 64;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CryptoError {
    InvalidInput,
    InvalidKey,
    Authentication,
    RandomUnavailable,
    NonceExhausted,
}
impl std::fmt::Display for CryptoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for CryptoError {}
pub type Result<T> = std::result::Result<T, CryptoError>;

/// Implementations MUST use a CSPRNG; deterministic fixtures belong only in tests.
/// Any error (including after a partial fill) aborts the operation.
pub trait RandomSource {
    fn fill(&mut self, dest: &mut [u8]) -> Result<()>;
}
#[cfg(feature = "os-rng")]
pub struct OsRandom;
#[cfg(feature = "os-rng")]
impl RandomSource for OsRandom {
    fn fill(&mut self, dest: &mut [u8]) -> Result<()> {
        getrandom::fill(dest).map_err(|_| CryptoError::RandomUnavailable)
    }
}
fn random32(rng: &mut dyn RandomSource) -> Result<Zeroizing<[u8; 32]>> {
    let mut b = Zeroizing::new([0; 32]);
    rng.fill(&mut b[..])?;
    Ok(b)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Domain {
    Certificate = 1,
    Object = 2,
    Receipt = 3,
    Transition = 4,
    Recovery = 5,
    Session = 6,
    Enrollment = 7,
    Presence = 8,
    /// Confirms that a message origin durably recorded one recipient's signed
    /// receipt. Kept separate from `Receipt` so an acknowledgement cannot be
    /// replayed as delivery evidence.
    ReceiptAck = 9,
    /// Authorizes a product cloud relay to persist a Field-authored action.
    /// Kept separate from radio objects and receipts so a signature for a
    /// server bridge cannot be replayed as any native protocol record.
    CloudRelay = 10,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scope {
    pub group: [u8; 32],
    pub epoch: u64,
}
impl Scope {
    fn prefix(&self, purpose: u8) -> Result<Vec<u8>> {
        if self.epoch == 0 || self.epoch > i64::MAX as u64 {
            return Err(CryptoError::InvalidInput);
        }
        let mut b = b"MeshLab/Crypto/v1\0".to_vec();
        b.extend(SUITE_ID.to_be_bytes());
        b.push(purpose);
        b.extend(self.group);
        b.extend(self.epoch.to_be_bytes());
        Ok(b)
    }
}
fn signature_input(scope: Scope, domain: Domain, bytes: &[u8]) -> Result<Vec<u8>> {
    if bytes.len() > MAX_SIGNED_BYTES {
        return Err(CryptoError::InvalidInput);
    }
    let mut b = scope.prefix(domain as u8)?;
    b.extend((bytes.len() as u32).to_be_bytes());
    b.extend(bytes);
    Ok(b)
}
pub struct IdentitySigningKey(SigningKey);
impl IdentitySigningKey {
    pub fn generate(rng: &mut dyn RandomSource) -> Result<Self> {
        Ok(Self::import(random32(rng)?))
    }
    /// Import only through the host's protected key storage boundary.
    pub fn import(seed: Zeroizing<[u8; 32]>) -> Self {
        Self(SigningKey::from_bytes(&seed))
    }
    pub fn public_key(&self) -> [u8; 32] {
        self.0.verifying_key().to_bytes()
    }
    pub fn sign(&self, scope: Scope, domain: Domain, canonical: &[u8]) -> Result<[u8; 64]> {
        Ok(self
            .0
            .sign(&signature_input(scope, domain, canonical)?)
            .to_bytes())
    }
}
pub fn verify(
    public: [u8; 32],
    scope: Scope,
    domain: Domain,
    canonical: &[u8],
    signature: [u8; 64],
) -> Result<()> {
    let input = signature_input(scope, domain, canonical)?;
    let key = VerifyingKey::from_bytes(&public).map_err(|_| CryptoError::InvalidKey)?;
    key.verify_strict(&input, &Signature::from_bytes(&signature))
        .map_err(|_| CryptoError::Authentication)
}
/// Full-length, purpose-separated public key identifier. Not a membership certificate.
pub fn identity_key_id(public: [u8; 32]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"MeshLab/IdentityKeyId/v1\0");
    h.update(public);
    h.finalize().into()
}

pub struct DeliverySecret(<Kem as KemTrait>::PrivateKey);
impl DeliverySecret {
    pub fn generate(rng: &mut dyn RandomSource) -> Result<Self> {
        let seed = random32(rng)?;
        Ok(Self(Kem::derive_keypair(&seed[..]).0))
    }
    pub fn import(bytes: Zeroizing<[u8; 32]>) -> Result<Self> {
        Ok(Self(
            <Kem as KemTrait>::PrivateKey::from_bytes(&bytes[..])
                .map_err(|_| CryptoError::InvalidKey)?,
        ))
    }
    pub fn public_key(&self) -> [u8; 32] {
        Kem::sk_to_pk(&self.0).to_bytes().into()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recipient {
    pub member: [u8; 32],
    pub delivery_key_id: [u8; 32],
    pub public_key: [u8; 32],
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecipientWrap {
    pub encapsulated: [u8; 32],
    pub ciphertext: [u8; 48],
}
/// Content keys deliberately have no Clone or Debug implementation.
/// ```compile_fail
/// fn cannot_clone(key: mesh_crypto::ContentKey) { let _ = key.clone(); }
/// ```
/// ```compile_fail
/// fn cannot_log(key: mesh_crypto::ContentKey) { println!("{key:?}"); }
/// ```
pub struct ContentKey(Zeroizing<[u8; 32]>);
impl ContentKey {
    pub fn generate(rng: &mut dyn RandomSource) -> Result<Self> {
        Ok(Self(random32(rng)?))
    }
}

// HPKE's RNG interface is infallible. Record the fallible host RNG error and check
// it before accepting any HPKE context/output. Failed buffers are zeroed; there is
// no panic adapter, weak fallback or PRNG seeded from simulation state.
struct HpkeRandom<'a> {
    source: &'a mut dyn RandomSource,
    failed: bool,
}
impl hpke::rand_core::RngCore for HpkeRandom<'_> {
    fn next_u32(&mut self) -> u32 {
        let mut b = [0; 4];
        self.fill_bytes(&mut b);
        u32::from_le_bytes(b)
    }
    fn next_u64(&mut self) -> u64 {
        let mut b = [0; 8];
        self.fill_bytes(&mut b);
        u64::from_le_bytes(b)
    }
    fn fill_bytes(&mut self, dest: &mut [u8]) {
        if self.failed || self.source.fill(dest).is_err() {
            dest.zeroize();
            self.failed = true
        }
    }
}
impl hpke::rand_core::CryptoRng for HpkeRandom<'_> {}
fn wrap_context(
    scope: Scope,
    object_context_id: [u8; 32],
    recipient: Recipient,
) -> Result<Vec<u8>> {
    let mut b = scope.prefix(16)?;
    b.extend(object_context_id);
    b.extend(recipient.member);
    b.extend(recipient.delivery_key_id);
    b.extend(recipient.public_key);
    Ok(b)
}
pub fn wrap_key(
    key: &ContentKey,
    scope: Scope,
    object_context_id: [u8; 32],
    recipient: Recipient,
    rng: &mut dyn RandomSource,
) -> Result<RecipientWrap> {
    let info = wrap_context(scope, object_context_id, recipient)?;
    let pk = <Kem as KemTrait>::PublicKey::from_bytes(&recipient.public_key)
        .map_err(|_| CryptoError::InvalidKey)?;
    let mut rng = HpkeRandom {
        source: rng,
        failed: false,
    };
    let setup =
        hpke::setup_sender::<HpkeAead, HkdfSha256, Kem, _>(&OpModeS::Base, &pk, &info, &mut rng);
    if rng.failed {
        return Err(CryptoError::RandomUnavailable);
    }
    let (enc, mut context) = setup.map_err(|_| CryptoError::InvalidKey)?;
    let wrapped = context
        .seal(&key.0[..], &info)
        .map_err(|_| CryptoError::Authentication)?;
    Ok(RecipientWrap {
        encapsulated: enc.to_bytes().into(),
        ciphertext: wrapped
            .try_into()
            .map_err(|_| CryptoError::Authentication)?,
    })
}
pub fn unwrap_key(
    secret: &DeliverySecret,
    scope: Scope,
    object_context_id: [u8; 32],
    recipient: Recipient,
    wrapped: &RecipientWrap,
) -> Result<ContentKey> {
    if secret.public_key() != recipient.public_key {
        return Err(CryptoError::InvalidKey);
    }
    let info = wrap_context(scope, object_context_id, recipient)?;
    let enc = <Kem as KemTrait>::EncappedKey::from_bytes(&wrapped.encapsulated)
        .map_err(|_| CryptoError::InvalidKey)?;
    let mut context =
        hpke::setup_receiver::<HpkeAead, HkdfSha256, Kem>(&OpModeR::Base, &secret.0, &enc, &info)
            .map_err(|_| CryptoError::Authentication)?;
    let clear = Zeroizing::new(
        context
            .open(&wrapped.ciphertext, &info)
            .map_err(|_| CryptoError::Authentication)?,
    );
    if clear.len() != 32 {
        return Err(CryptoError::Authentication);
    }
    let mut key = Zeroizing::new([0; 32]);
    key.copy_from_slice(&clear);
    Ok(ContentKey(key))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChunkContext {
    pub scope: Scope,
    pub object_context_id: [u8; 32],
    pub origin: [u8; 32],
    pub origin_sequence: u64,
    pub audience_digest: [u8; 32],
    pub namespace: String,
    pub schema: u32,
    pub chunk_count: u32,
    pub lifetime: u64,
    pub priority: u8,
}
impl ChunkContext {
    fn aad(&self, index: u32) -> Result<Vec<u8>> {
        if self.origin_sequence == 0
            || self.origin_sequence > i64::MAX as u64
            || self.schema == 0
            || self.lifetime == 0
            || self.lifetime > i64::MAX as u64
            || self.chunk_count == 0
            || self.chunk_count > MAX_CHUNKS
            || index >= self.chunk_count
            || self.namespace.is_empty()
            || self.namespace.len() > 64
            || !self
                .namespace
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        {
            return Err(CryptoError::InvalidInput);
        }
        let mut b = self.scope.prefix(17)?;
        b.extend(self.object_context_id);
        b.extend(self.origin);
        b.extend(self.origin_sequence.to_be_bytes());
        b.extend(self.audience_digest);
        b.push(self.namespace.len() as u8);
        b.extend(self.namespace.bytes());
        b.extend(self.schema.to_be_bytes());
        b.extend(index.to_be_bytes());
        b.extend(self.chunk_count.to_be_bytes());
        b.extend(self.lifetime.to_be_bytes());
        b.push(self.priority);
        Ok(b)
    }
}
fn nonce_for(base: [u8; 12], index: u32) -> Result<[u8; 12]> {
    let counter = u64::from_be_bytes(
        base[4..]
            .try_into()
            .map_err(|_| CryptoError::InvalidInput)?,
    );
    let mut nonce = base;
    nonce[4..].copy_from_slice(
        &counter
            .checked_add(index as u64)
            .ok_or(CryptoError::NonceExhausted)?
            .to_be_bytes(),
    );
    Ok(nonce)
}
fn seal_raw(key: &[u8; 32], nonce: [u8; 12], aad: &[u8], bytes: &[u8]) -> Result<Vec<u8>> {
    let cipher = ChaCha20Poly1305::new(key.into());
    let mut buf = Zeroizing::new(bytes.to_vec());
    cipher
        .encrypt_in_place(Nonce::from_slice(&nonce), aad, &mut *buf)
        .map_err(|_| CryptoError::Authentication)?;
    Ok(std::mem::take(&mut *buf))
}
fn open_raw(
    key: &[u8; 32],
    nonce: [u8; 12],
    aad: &[u8],
    bytes: &[u8],
) -> Result<Zeroizing<Vec<u8>>> {
    let cipher = ChaCha20Poly1305::new(key.into());
    let mut buf = Zeroizing::new(bytes.to_vec());
    cipher
        .decrypt_in_place(Nonce::from_slice(&nonce), aad, &mut *buf)
        .map_err(|_| CryptoError::Authentication)?;
    Ok(buf)
}
/// Owns a fresh per-object key and enforces increasing chunk indices. Retry must
/// resend stored ciphertext; a sealer cannot rewind or be restored from a key.
pub struct ContentSealer {
    key: ContentKey,
    context: ChunkContext,
    base_nonce: [u8; 12],
    next: u32,
}
impl ContentSealer {
    pub fn generate(context: ChunkContext, rng: &mut dyn RandomSource) -> Result<Self> {
        context.aad(0)?;
        let key = ContentKey::generate(rng)?;
        let mut base_nonce = [0; 12];
        rng.fill(&mut base_nonce)?;
        nonce_for(base_nonce, context.chunk_count - 1)?;
        Ok(Self {
            key,
            context,
            base_nonce,
            next: 0,
        })
    }
    pub fn base_nonce(&self) -> [u8; 12] {
        self.base_nonce
    }
    pub fn wrap_for(
        &self,
        recipient: Recipient,
        rng: &mut dyn RandomSource,
    ) -> Result<RecipientWrap> {
        wrap_key(
            &self.key,
            self.context.scope,
            self.context.object_context_id,
            recipient,
            rng,
        )
    }
    pub fn seal_next(&mut self, plaintext: &[u8]) -> Result<Vec<u8>> {
        if plaintext.is_empty() || plaintext.len() > MAX_CHUNK_BYTES {
            return Err(CryptoError::InvalidInput);
        }
        if self.next >= self.context.chunk_count {
            return Err(CryptoError::NonceExhausted);
        }
        let index = self.next;
        let aad = self.context.aad(index)?;
        let nonce = nonce_for(self.base_nonce, index)?;
        self.next += 1;
        seal_raw(&self.key.0, nonce, &aad, plaintext)
    }
}
pub fn open_chunk(
    key: &ContentKey,
    context: &ChunkContext,
    base_nonce: [u8; 12],
    index: u32,
    ciphertext: &[u8],
) -> Result<Zeroizing<Vec<u8>>> {
    if ciphertext.len() < 17 || ciphertext.len() > MAX_CHUNK_BYTES + 16 {
        return Err(CryptoError::InvalidInput);
    }
    let aad = context.aad(index)?;
    open_raw(&key.0, nonce_for(base_nonce, index)?, &aad, ciphertext)
}

#[cfg(test)]
mod tests;
