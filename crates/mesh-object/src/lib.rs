//! Immutable LOCAL storage manifests. This format is not the A003 mesh wire.
//! The storage boundary handles opaque bytes; no authentication is inferred from a digest.
use mesh_types::durable::*;
use sha2::{Digest, Sha256};

pub fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    origin: MemberId,
    sequence: u64,
    namespace: Namespace,
    epoch: u64,
    targets: Vec<MemberId>,
    expires_at: u64,
    hop_limit: u8,
    content_len: usize,
    hashes: Vec<[u8; 32]>,
}
pub struct PreparedObject {
    manifest: Manifest,
    chunks: Vec<Vec<u8>>,
}
#[derive(Clone)]
pub struct ObjectPolicy {
    pub namespace: Namespace,
    pub epoch: u64,
    pub targets: Vec<MemberId>,
    pub expires_at: u64,
    pub hop_limit: u8,
}
impl PreparedObject {
    /// Accepts bytes already protected/validated by the caller. F1 harness uses synthetic fixtures.
    /// This does not sign/encrypt payloads and is not exposed by the mobile SDK.
    pub fn from_opaque(
        origin: MemberId,
        sequence: u64,
        policy: ObjectPolicy,
        bytes: &[u8],
    ) -> Result<Self> {
        if bytes.is_empty() || bytes.len() > MAX_OBJECT_BYTES {
            return Err(DurableError::InvalidInput);
        }
        let chunks: Vec<Vec<u8>> = bytes.chunks(CHUNK_BYTES).map(|b| b.to_vec()).collect();
        let mut manifest = Manifest {
            origin,
            sequence,
            namespace: policy.namespace,
            epoch: policy.epoch,
            targets: policy.targets,
            expires_at: policy.expires_at,
            hop_limit: policy.hop_limit,
            content_len: bytes.len(),
            hashes: chunks.iter().map(|b| digest(b)).collect(),
        };
        manifest.targets.sort();
        manifest.validate()?;
        Ok(Self { manifest, chunks })
    }
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    pub fn chunks(&self) -> &[Vec<u8>] {
        &self.chunks
    }
}
impl Manifest {
    fn validate(&self) -> Result<()> {
        if self.sequence == 0
            || self.sequence > MAX_LOGICAL_TIME
            || self.epoch == 0
            || self.epoch > MAX_LOGICAL_TIME
            || self.expires_at == 0
            || self.expires_at > MAX_LOGICAL_TIME
            || self.hop_limit == 0
            || self.hop_limit > 16
            || self.targets.is_empty()
            || self.targets.len() > MAX_TARGETS
            || !self.targets.windows(2).all(|p| p[0] < p[1])
            || self.content_len == 0
            || self.content_len > MAX_OBJECT_BYTES
            || self.hashes.len() != self.content_len.div_ceil(CHUNK_BYTES)
        {
            return Err(DurableError::InvalidInput);
        }
        Ok(())
    }
    pub fn id(&self) -> ObjectId {
        let mut bytes = b"MeshLab/StoreManifest/v1\0".to_vec();
        bytes.extend(self.encode());
        ObjectId(digest(&bytes))
    }
    pub fn origin(&self) -> MemberId {
        self.origin
    }
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
    pub fn namespace(&self) -> &Namespace {
        &self.namespace
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    pub fn targets(&self) -> &[MemberId] {
        &self.targets
    }
    pub fn expires_at(&self) -> u64 {
        self.expires_at
    }
    pub fn hop_limit(&self) -> u8 {
        self.hop_limit
    }
    pub fn content_len(&self) -> usize {
        self.content_len
    }
    pub fn chunk_count(&self) -> usize {
        self.hashes.len()
    }
    pub fn verify_chunk(&self, index: usize, bytes: &[u8]) -> Result<()> {
        if index >= self.chunk_count() {
            return Err(DurableError::InvalidInput);
        }
        let expected = if index + 1 == self.chunk_count() {
            self.content_len - index * CHUNK_BYTES
        } else {
            CHUNK_BYTES
        };
        if bytes.len() != expected || digest(bytes) != self.hashes[index] {
            return Err(DurableError::Corrupt);
        }
        Ok(())
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut b = vec![0x8a, 1];
        blob(&mut b, &self.origin.0);
        uint(&mut b, self.sequence);
        text(&mut b, self.namespace.as_str());
        uint(&mut b, self.epoch);
        head(&mut b, 4, self.targets.len() as u64);
        for target in &self.targets {
            blob(&mut b, &target.0);
        }
        uint(&mut b, self.expires_at);
        uint(&mut b, self.hop_limit as u64);
        uint(&mut b, self.content_len as u64);
        head(&mut b, 4, self.hashes.len() as u64);
        for hash in &self.hashes {
            blob(&mut b, hash);
        }
        b
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(DurableError::InvalidInput);
        }
        let mut r = Reader { bytes, pos: 0 };
        r.array(10)?;
        if r.value(0)? != 1 {
            return Err(DurableError::UnsupportedSchema);
        }
        let origin = MemberId(r.hash()?);
        let sequence = r.value(0)?;
        let namespace = Namespace::new(
            std::str::from_utf8(r.bytes(3, 64)?).map_err(|_| DurableError::InvalidInput)?,
        )?;
        let epoch = r.value(0)?;
        let count = r.value(4)?;
        if count == 0 || count > MAX_TARGETS as u64 {
            return Err(DurableError::InvalidInput);
        }
        let mut targets = Vec::new();
        for _ in 0..count {
            targets.push(MemberId(r.hash()?));
        }
        let expires_at = r.value(0)?;
        let hop_limit = u8::try_from(r.value(0)?).map_err(|_| DurableError::InvalidInput)?;
        let content_len = usize::try_from(r.value(0)?).map_err(|_| DurableError::InvalidInput)?;
        let count = r.value(4)?;
        if count == 0 || count > MAX_CHUNKS as u64 {
            return Err(DurableError::InvalidInput);
        }
        let mut hashes = Vec::new();
        for _ in 0..count {
            hashes.push(r.hash()?);
        }
        if r.pos != bytes.len() {
            return Err(DurableError::InvalidInput);
        }
        let manifest = Self {
            origin,
            sequence,
            namespace,
            epoch,
            targets,
            expires_at,
            hop_limit,
            content_len,
            hashes,
        };
        manifest.validate()?;
        Ok(manifest)
    }
}
fn head(b: &mut Vec<u8>, major: u8, n: u64) {
    let tag = major << 5;
    match n {
        0..=23 => b.push(tag | n as u8),
        24..=255 => b.extend([tag | 24, n as u8]),
        256..=65535 => {
            b.push(tag | 25);
            b.extend((n as u16).to_be_bytes());
        }
        65536..=4294967295 => {
            b.push(tag | 26);
            b.extend((n as u32).to_be_bytes());
        }
        _ => {
            b.push(tag | 27);
            b.extend(n.to_be_bytes());
        }
    }
}
fn uint(b: &mut Vec<u8>, n: u64) {
    head(b, 0, n)
}
fn blob(b: &mut Vec<u8>, v: &[u8]) {
    head(b, 2, v.len() as u64);
    b.extend(v)
}
fn text(b: &mut Vec<u8>, s: &str) {
    head(b, 3, s.len() as u64);
    b.extend(s.bytes())
}
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}
impl<'a> Reader<'a> {
    fn byte(&mut self) -> Result<u8> {
        let b = *self.bytes.get(self.pos).ok_or(DurableError::InvalidInput)?;
        self.pos += 1;
        Ok(b)
    }
    fn value(&mut self, major: u8) -> Result<u64> {
        let b = self.byte()?;
        if b >> 5 != major {
            return Err(DurableError::InvalidInput);
        }
        let tag = b & 31;
        if tag < 24 {
            return Ok(tag as u64);
        }
        let (count, min) = match tag {
            24 => (1, 24),
            25 => (2, 256),
            26 => (4, 65536),
            27 => (8, 4294967296),
            _ => return Err(DurableError::InvalidInput),
        };
        let mut n = 0u64;
        for _ in 0..count {
            n = (n << 8) | u64::from(self.byte()?)
        }
        if n < min {
            return Err(DurableError::InvalidInput);
        }
        Ok(n)
    }
    fn array(&mut self, n: u64) -> Result<()> {
        if self.value(4)? != n {
            return Err(DurableError::InvalidInput);
        }
        Ok(())
    }
    fn bytes(&mut self, major: u8, max: usize) -> Result<&'a [u8]> {
        let n = self.value(major)?;
        if n > max as u64 {
            return Err(DurableError::InvalidInput);
        }
        let end = self
            .pos
            .checked_add(n as usize)
            .ok_or(DurableError::InvalidInput)?;
        let result = self
            .bytes
            .get(self.pos..end)
            .ok_or(DurableError::InvalidInput)?;
        self.pos = end;
        Ok(result)
    }
    fn hash(&mut self) -> Result<[u8; 32]> {
        self.bytes(2, 32)?
            .try_into()
            .map_err(|_| DurableError::InvalidInput)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn object() -> PreparedObject {
        PreparedObject::from_opaque(
            MemberId([1; 32]),
            1,
            ObjectPolicy {
                namespace: Namespace::new("mesh.lab.synthetic.v1").unwrap(),
                epoch: 1,
                targets: vec![MemberId([2; 32])],
                expires_at: 100,
                hop_limit: 4,
            },
            &vec![9; 2500],
        )
        .unwrap()
    }
    #[test]
    fn canonical_roundtrip_and_chunk_bounds() {
        let o = object();
        let m = o.manifest();
        assert_eq!(Manifest::decode(&m.encode()).unwrap(), *m);
        assert_eq!(m.chunk_count(), 3);
        for (i, b) in o.chunks().iter().enumerate() {
            m.verify_chunk(i, b).unwrap()
        }
        assert_eq!(m.verify_chunk(3, &[]), Err(DurableError::InvalidInput));
        let mut b = o.chunks()[0].clone();
        b[0] ^= 1;
        assert_eq!(m.verify_chunk(0, &b), Err(DurableError::Corrupt));
    }
    #[test]
    fn truncated_noncanonical_trailing_and_oversized_rejected() {
        let encoded = object().manifest.encode();
        for i in 0..encoded.len() {
            assert!(Manifest::decode(&encoded[..i]).is_err())
        }
        let mut tail = encoded.clone();
        tail.push(0);
        assert!(Manifest::decode(&tail).is_err());
        let mut overlong = encoded.clone();
        overlong.splice(1..2, [0x18, 1]);
        assert!(Manifest::decode(&overlong).is_err());
        assert!(Manifest::decode(&vec![0; MAX_MANIFEST_BYTES + 1]).is_err());
    }
    #[test]
    fn audience_and_expiry_are_bound_to_id() {
        let mut m = object().manifest;
        let id = m.id();
        m.expires_at += 1;
        assert_ne!(id, m.id());
        m.targets.push(m.targets[0]);
        assert!(m.validate().is_err());
    }
}

#[cfg(test)]
mod vectors {
    use super::*;
    fn unhex(s: &str) -> Vec<u8> {
        s.trim()
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
            .collect()
    }
    #[test]
    fn independent_local_manifest_vector() {
        let o = PreparedObject::from_opaque(
            MemberId([1; 32]),
            1,
            ObjectPolicy {
                namespace: Namespace::new("synthetic").unwrap(),
                epoch: 1,
                targets: vec![MemberId([2; 32])],
                expires_at: 100,
                hop_limit: 4,
            },
            b"abc",
        )
        .unwrap();
        let encoded = unhex(include_str!("../../../vectors/store/manifest-v1.hex"));
        assert_eq!(o.manifest().encode(), encoded);
        assert_eq!(Manifest::decode(&encoded).unwrap(), *o.manifest());
        assert_eq!(
            o.manifest().id().0.to_vec(),
            unhex(include_str!("../../../vectors/store/manifest-v1.id"))
        );
    }
}
