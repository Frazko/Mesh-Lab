//! The narrow, radio-independent boundary between the Rust engine and a native
//! transport adapter. Frames are bounded and carry no routing decisions.

pub const WIRE_VERSION: u8 = 1;
pub const MAX_FRAME_BYTES: usize = 4_160;
const HEADER_BYTES: usize = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidFrame,
    Unavailable,
    ResourcePressure,
}

pub type Result<T> = std::result::Result<T, Error>;

/// A single already-protected engine frame. Native adapters may fragment it for
/// an OS transport, but they may not inspect, alter, or synthesize its bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame(Vec<u8>);

impl Frame {
    pub fn new(bytes: Vec<u8>) -> Result<Self> {
        if bytes.is_empty() || bytes.len() > MAX_FRAME_BYTES {
            return Err(Error::InvalidFrame);
        }
        Ok(Self(bytes))
    }

    pub fn bytes(&self) -> &[u8] {
        &self.0
    }

    /// Canonical bounded wire envelope for byte-stream transports.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_BYTES + self.0.len());
        out.push(WIRE_VERSION);
        out.extend((self.0.len() as u16).to_be_bytes());
        out.extend(&self.0);
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < HEADER_BYTES || bytes[0] != WIRE_VERSION {
            return Err(Error::InvalidFrame);
        }
        let len = u16::from_be_bytes([bytes[1], bytes[2]]) as usize;
        if len == 0 || len > MAX_FRAME_BYTES || bytes.len() != HEADER_BYTES + len {
            return Err(Error::InvalidFrame);
        }
        Self::new(bytes[HEADER_BYTES..].to_vec())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkId(pub u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PortEvent {
    LinkUp(LinkId),
    Frame { link: LinkId, frame: Frame },
    LinkDown(LinkId),
}

/// Native owns OS callbacks; Rust owns framing and every protocol transition.
/// `poll` returns copied facts from a bounded native mailbox.
pub trait TransportPort {
    fn send(&mut self, link: LinkId, frame: &Frame) -> Result<()>;
    fn poll(&mut self) -> Result<Vec<PortEvent>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conf_framing_is_exact_and_bounded() {
        let f = Frame::new(vec![0xa1, 0xb2]).unwrap();
        assert_eq!(f.encode(), [1, 0, 2, 0xa1, 0xb2]);
        assert_eq!(Frame::decode(&f.encode()), Ok(f));
        for invalid in [
            vec![],
            vec![2, 0, 1, 7],
            vec![1, 0, 0],
            vec![1, 0, 2, 7],
            vec![1, 0, 1, 7, 8],
        ] {
            assert_eq!(Frame::decode(&invalid), Err(Error::InvalidFrame));
        }
        assert_eq!(
            Frame::new(vec![1; MAX_FRAME_BYTES + 1]),
            Err(Error::InvalidFrame)
        );
    }

    #[test]
    fn arbitrary_truncation_never_constructs_a_frame() {
        let valid = Frame::new(vec![4; 32]).unwrap().encode();
        for end in 0..valid.len() {
            assert_eq!(Frame::decode(&valid[..end]), Err(Error::InvalidFrame));
        }
    }
}
