//! Bounded definite CBOR primitives. Schema callers fix nesting; no recursive parser.
use mesh_types::durable::{DurableError, Result};
#[derive(Default)]
pub struct Writer(Vec<u8>);
impl Writer {
    fn head(&mut self, major: u8, n: u64) {
        let tag = major << 5;
        match n {
            0..=23 => self.0.push(tag | n as u8),
            24..=255 => self.0.extend([tag | 24, n as u8]),
            256..=65535 => {
                self.0.push(tag | 25);
                self.0.extend((n as u16).to_be_bytes())
            }
            65536..=4294967295 => {
                self.0.push(tag | 26);
                self.0.extend((n as u32).to_be_bytes())
            }
            _ => {
                self.0.push(tag | 27);
                self.0.extend(n.to_be_bytes())
            }
        }
    }
    pub fn array(&mut self, n: usize) {
        self.head(4, n as u64)
    }
    pub fn uint(&mut self, n: u64) {
        self.head(0, n)
    }
    pub fn bytes(&mut self, b: &[u8]) {
        self.head(2, b.len() as u64);
        self.0.extend(b)
    }
    pub fn text(&mut self, b: &str) {
        self.head(3, b.len() as u64);
        self.0.extend(b.bytes())
    }
    pub fn finish(self) -> Vec<u8> {
        self.0
    }
}
pub struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}
impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8], limit: usize) -> Result<Self> {
        if bytes.len() > limit {
            return Err(DurableError::InvalidInput);
        }
        Ok(Self { bytes, pos: 0 })
    }
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
        let (n, min) = match tag {
            24 => (1, 24),
            25 => (2, 256),
            26 => (4, 65536),
            27 => (8, 4294967296),
            _ => return Err(DurableError::InvalidInput),
        };
        let mut value = 0;
        for _ in 0..n {
            value = (value << 8) | u64::from(self.byte()?)
        }
        if value < min {
            return Err(DurableError::InvalidInput);
        }
        Ok(value)
    }
    pub fn uint(&mut self) -> Result<u64> {
        self.value(0)
    }
    pub fn array(&mut self, expected: usize) -> Result<()> {
        if self.value(4)? != expected as u64 {
            return Err(DurableError::InvalidInput);
        }
        Ok(())
    }
    pub fn count(&mut self, max: usize) -> Result<usize> {
        let n = self.value(4)?;
        if n > max as u64 {
            return Err(DurableError::InvalidInput);
        }
        Ok(n as usize)
    }
    fn string(&mut self, major: u8, max: usize) -> Result<&'a [u8]> {
        let n = self.value(major)?;
        if n > max as u64 {
            return Err(DurableError::InvalidInput);
        }
        let end = self
            .pos
            .checked_add(n as usize)
            .ok_or(DurableError::InvalidInput)?;
        let b = self
            .bytes
            .get(self.pos..end)
            .ok_or(DurableError::InvalidInput)?;
        self.pos = end;
        Ok(b)
    }
    pub fn bytes(&mut self, max: usize) -> Result<&'a [u8]> {
        self.string(2, max)
    }
    pub fn text(&mut self, max: usize) -> Result<&'a str> {
        std::str::from_utf8(self.string(3, max)?).map_err(|_| DurableError::InvalidInput)
    }
    pub fn fixed<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.bytes(N)?
            .try_into()
            .map_err(|_| DurableError::InvalidInput)
    }
    pub fn end(self) -> Result<()> {
        if self.pos != self.bytes.len() {
            return Err(DurableError::InvalidInput);
        }
        Ok(())
    }
}
