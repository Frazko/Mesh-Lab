//! Bounded deterministic CBOR for the F0 API only. Not a mesh wire codec.
use mesh_types::{Command, Error, Snapshot, API_VERSION, MAX_COUNTER, MAX_INPUT};

pub fn decode_request(bytes: &[u8]) -> Result<Command, Error> {
    if bytes.len() > MAX_INPUT || bytes.first() != Some(&0x83) {
        return Err(Error::InvalidArgument);
    }
    let mut pos = 1;
    let version = read_uint(bytes, &mut pos)?;
    let method = read_uint(bytes, &mut pos)?;
    let arg = read_uint(bytes, &mut pos)?;
    if pos != bytes.len() || arg > MAX_COUNTER {
        return Err(Error::InvalidArgument);
    }
    if version != API_VERSION {
        return Err(Error::IncompatibleVersion);
    }
    match (method, arg) {
        (0, 0) => Ok(Command::EngineInfo),
        (1, cursor) => Ok(Command::Subscribe { cursor }),
        (2, request_id) if request_id > 0 => Ok(Command::VerifyBridge { request_id }),
        _ => Err(Error::InvalidArgument),
    }
}

fn read_uint(bytes: &[u8], pos: &mut usize) -> Result<u64, Error> {
    let head = *bytes.get(*pos).ok_or(Error::InvalidArgument)?;
    *pos += 1;
    let (count, minimum) = match head {
        0..=23 => return Ok(head as u64),
        24 => (1, 24),
        25 => (2, 256),
        26 => (4, 65_536),
        27 => (8, 4_294_967_296),
        _ => return Err(Error::InvalidArgument),
    };
    let mut value = 0u64;
    for _ in 0..count {
        value = (value << 8) | u64::from(*bytes.get(*pos).ok_or(Error::InvalidArgument)?);
        *pos += 1;
    }
    if value < minimum {
        Err(Error::InvalidArgument)
    } else {
        Ok(value)
    }
}

fn head(out: &mut Vec<u8>, major: u8, n: u64) {
    let tag = major << 5;
    match n {
        0..=23 => out.push(tag | n as u8),
        24..=255 => out.extend([tag | 24, n as u8]),
        256..=65_535 => {
            out.push(tag | 25);
            out.extend((n as u16).to_be_bytes());
        }
        65_536..=4_294_967_295 => {
            out.push(tag | 26);
            out.extend((n as u32).to_be_bytes());
        }
        _ => {
            out.push(tag | 27);
            out.extend(n.to_be_bytes());
        }
    }
}
fn uint(out: &mut Vec<u8>, n: u64) {
    head(out, 0, n);
}
fn text(out: &mut Vec<u8>, s: &str) {
    head(out, 3, s.len() as u64);
    out.extend(s.bytes());
}

pub fn encode_info() -> Vec<u8> {
    let mut out = vec![0x83, 1, 0, 0x85];
    text(&mut out, env!("CARGO_PKG_VERSION"));
    uint(&mut out, mesh_types::ABI_VERSION as u64);
    uint(&mut out, API_VERSION);
    text(&mut out, "F0");
    text(
        &mut out,
        option_env!("MESH_BUILD_ID").unwrap_or("local-dev"),
    );
    out
}

pub fn encode_snapshot(method: u64, snapshot: &Snapshot) -> Vec<u8> {
    let mut out = vec![0x83, 1];
    uint(&mut out, method);
    out.push(0x86);
    uint(&mut out, snapshot.runtime_id);
    uint(&mut out, snapshot.cursor);
    uint(&mut out, snapshot.probe_count);
    uint(&mut out, 0); // foundationOnly, never a radio/session readiness claim
    out.push(if snapshot.cursor_reset { 0xf5 } else { 0xf4 });
    head(&mut out, 4, snapshot.events.len() as u64);
    for event in &snapshot.events {
        out.push(0x83);
        uint(&mut out, event.sequence);
        uint(&mut out, event.request_id);
        uint(&mut out, 0); // bridgeVerified
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn conf_api_requests() {
        assert_eq!(decode_request(&[0x83, 1, 0, 0]), Ok(Command::EngineInfo));
        assert_eq!(
            decode_request(&[0x83, 1, 2, 0x18, 24]),
            Ok(Command::VerifyBridge { request_id: 24 })
        );
        for bytes in [
            &[0x83, 1, 0, 0, 0][..],
            &[0x83, 1, 2, 0x18, 1],
            &[0x83, 1, 2, 0],
            &[0x9f, 1, 0, 0, 0xff],
            &[
                0x83, 1, 1, 0x1b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            ],
        ] {
            assert_eq!(decode_request(bytes), Err(Error::InvalidArgument));
        }
        assert_eq!(
            decode_request(&[0x83, 2, 0, 0]),
            Err(Error::IncompatibleVersion)
        );
    }
    #[test]
    fn conf_snapshot_vector() {
        let s = Snapshot {
            runtime_id: 1,
            cursor: 0,
            probe_count: 0,
            cursor_reset: false,
            events: vec![],
        };
        assert_eq!(
            encode_snapshot(1, &s),
            [0x83, 1, 1, 0x86, 1, 0, 0, 0, 0xf4, 0x80]
        );
    }
    #[test]
    fn truncated_and_arbitrary_inputs_never_panic() {
        for n in 0..=255 {
            let data = [0x83, 1, 2, n, 0, 0, 0, 0, 0, 0, 0, 0];
            for end in 0..=data.len() {
                let _ = decode_request(&data[..end]);
            }
        }
    }
}

pub mod canonical;
