//! F0 diagnostic contracts. No wire, identity, radio or durable object is active.
pub const ABI_VERSION: u32 = 1;
pub const API_VERSION: u64 = 1;
pub const MAX_INPUT: usize = 64;
pub const MAX_OUTPUT: usize = 16_384;
pub const MAX_EVENTS: usize = 64;
pub const MAX_RUNTIMES: usize = 16;
// Exactly representable across Dart, Kotlin and Swift.
pub const MAX_COUNTER: u64 = (1 << 53) - 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum Error {
    InvalidArgument = 1,
    IncompatibleVersion = 2,
    InvalidHandle = 3,
    ResourcePressure = 4,
    InternalInvariant = 5,
    StaleRequest = 6,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    EngineInfo,
    Subscribe { cursor: u64 },
    VerifyBridge { request_id: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub sequence: u64,
    pub request_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub runtime_id: u64,
    pub cursor: u64,
    pub probe_count: u64,
    pub cursor_reset: bool,
    pub events: Vec<Event>,
}

pub mod durable;
