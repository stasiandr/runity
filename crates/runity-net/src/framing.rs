//! Length-prefixed framing shared by every [`crate::Transport`].
//!
//! A frame on the wire is a 4-byte little-endian length header followed by
//! that many bytes of body — the same shape `runity-platform`'s X11 backend
//! uses for its requests and replies. [`Loopback`](crate::Loopback) and
//! [`Tcp`](crate::Tcp) both encode and decode through the functions here, so
//! single-player and networked play run the exact same serialization path.

use std::io::{self, ErrorKind};

/// The largest body a frame may declare. Chosen so a corrupt or hostile
/// length header is rejected before it can be used to force a large
/// allocation.
pub const MAX_FRAME_LEN: usize = 4 * 1024 * 1024;

const HEADER_LEN: usize = 4;

fn too_large(len: usize) -> io::Error {
    io::Error::new(
        ErrorKind::InvalidData,
        format!("frame length {len} exceeds the {MAX_FRAME_LEN}-byte limit"),
    )
}

/// Appends `body` to `out` as one length-prefixed frame.
///
/// Fails without writing anything if `body` is larger than
/// [`MAX_FRAME_LEN`].
pub fn encode(body: &[u8], out: &mut Vec<u8>) -> io::Result<()> {
    if body.len() > MAX_FRAME_LEN {
        return Err(too_large(body.len()));
    }
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(body);
    Ok(())
}

/// Pulls every complete frame out of the front of `buf`, leaving any trailing
/// partial frame in place for the next call.
///
/// A declared length over [`MAX_FRAME_LEN`] is rejected as soon as the header
/// is available, before the body — which may not even have arrived yet — is
/// read or allocated.
pub fn decode_all(buf: &mut Vec<u8>) -> io::Result<Vec<Vec<u8>>> {
    let mut frames = Vec::new();
    let mut pos = 0;
    loop {
        if buf.len() - pos < HEADER_LEN {
            break;
        }
        let header = [buf[pos], buf[pos + 1], buf[pos + 2], buf[pos + 3]];
        let len = u32::from_le_bytes(header) as usize;
        if len > MAX_FRAME_LEN {
            return Err(too_large(len));
        }
        let body_start = pos + HEADER_LEN;
        if buf.len() - body_start < len {
            break;
        }
        frames.push(buf[body_start..body_start + len].to_vec());
        pos = body_start + len;
    }
    buf.drain(..pos);
    Ok(frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_single_frame() {
        let mut wire = Vec::new();
        encode(b"hello", &mut wire).unwrap();
        let frames = decode_all(&mut wire).unwrap();
        assert_eq!(frames, vec![b"hello".to_vec()]);
        assert!(wire.is_empty());
    }

    #[test]
    fn leaves_a_partial_frame_for_next_time() {
        let mut wire = Vec::new();
        encode(b"hello", &mut wire).unwrap();
        wire.truncate(wire.len() - 2); // chop the tail off the body
        let frames = decode_all(&mut wire).unwrap();
        assert!(frames.is_empty());
        assert_eq!(wire.len(), 3 + 4); // header plus the bytes that arrived
    }

    #[test]
    fn decodes_several_queued_frames_at_once() {
        let mut wire = Vec::new();
        encode(b"one", &mut wire).unwrap();
        encode(b"two", &mut wire).unwrap();
        let frames = decode_all(&mut wire).unwrap();
        assert_eq!(frames, vec![b"one".to_vec(), b"two".to_vec()]);
    }

    #[test]
    fn encode_rejects_an_oversized_body() {
        let body = vec![0u8; MAX_FRAME_LEN + 1];
        let mut wire = Vec::new();
        assert!(encode(&body, &mut wire).is_err());
        assert!(wire.is_empty());
    }

    #[test]
    fn decode_rejects_an_oversized_declared_length_without_the_body() {
        // Only the header ever arrives: a real body this size would never be
        // allocated, which is the point of checking the header first.
        let mut wire = ((MAX_FRAME_LEN + 1) as u32).to_le_bytes().to_vec();
        let err = decode_all(&mut wire).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidData);
    }

    #[test]
    fn exactly_the_limit_is_accepted() {
        let body = vec![7u8; MAX_FRAME_LEN];
        let mut wire = Vec::new();
        encode(&body, &mut wire).unwrap();
        let frames = decode_all(&mut wire).unwrap();
        assert_eq!(frames, vec![body]);
    }
}
