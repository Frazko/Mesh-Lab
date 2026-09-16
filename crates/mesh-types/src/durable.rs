//! Bounded storage/transfer contracts. Logical times are injected, never read here.
use std::fmt;

pub const CHUNK_BYTES: usize = 1024;
pub const MAX_OBJECT_BYTES: usize = 64 * 1024;
pub const MAX_CHUNKS: usize = MAX_OBJECT_BYTES / CHUNK_BYTES;
/// A protected object can address a bounded subset of a larger field group.
/// Keeping this lower than `MAX_GROUP_MEMBERS` bounds envelope fan-out.
pub const MAX_TARGETS: usize = 10;
/// Maximum certified members in the Wi-Fi Aware field-group profile.
/// Direct radio degree is constrained separately by the transport layer.
pub const MAX_GROUP_MEMBERS: usize = 50;
pub const MAX_MANIFEST_BYTES: usize = 4096;
pub const MAX_LOGICAL_TIME: u64 = i64::MAX as u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MemberId(pub [u8; 32]);
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectId(pub [u8; 32]);
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OperationId(pub [u8; 16]);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DurableError {
    InvalidInput,
    ResourcePressure,
    NotFound,
    Conflict,
    Expired,
    Corrupt,
    KeyOrCorruption,
    CipherUnavailable,
    CryptoUnavailable,
    UnsupportedSchema,
    StorageUnavailable,
    InjectedFailure,
    InvalidReceipt,
    AuthenticationFailed,
}
impl fmt::Display for DurableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for DurableError {}
pub type Result<T> = std::result::Result<T, DurableError>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Namespace(String);
impl Namespace {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 64
            || !value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        {
            return Err(DurableError::InvalidInput);
        }
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeliveryState {
    Stored,
    Relaying,
    Delivered,
    Expired,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeliveryProgress {
    pub state: DeliveryState,
    pub confirmed: usize,
    pub required: usize,
}
