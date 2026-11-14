//! Frames: 4-byte big-endian length, then that many bytes of UTF-8 JSON.
//!
//! The reader checks the length against the caller's limit before it
//! allocates, and treats a truncated frame, invalid JSON or a message that
//! does not match the expected type as malformed.

use std::fmt;
use std::io::{self, Read, Write};

use serde::Serialize;
use serde::de::DeserializeOwned;

#[derive(Debug)]
pub enum FrameError {
    /// I/O failed (including EOF in the middle of a frame).
    Io(io::Error),
    /// The length prefix exceeds the limit; nothing was allocated.
    TooLarge { len: usize, max: usize },
    /// The body is not valid JSON of the expected message type.
    Malformed(serde_json::Error),
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "i/o: {e}"),
            Self::TooLarge { len, max } => write!(f, "frame of {len} bytes exceeds {max}"),
            Self::Malformed(e) => write!(f, "malformed frame: {e}"),
        }
    }
}

impl std::error::Error for FrameError {}

impl From<io::Error> for FrameError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// Serializes `msg` and writes it as one frame, then flushes. Fails without
/// writing anything if the body would exceed `max`.
pub fn write_frame<W: Write, T: Serialize>(
    w: &mut W,
    msg: &T,
    max: usize,
) -> Result<(), FrameError> {
    let body = serde_json::to_vec(msg).map_err(FrameError::Malformed)?;
    if body.len() > max {
        return Err(FrameError::TooLarge {
            len: body.len(),
            max,
        });
    }
    let len = u32::try_from(body.len()).map_err(|_| FrameError::TooLarge {
        len: body.len(),
        max,
    })?;
    w.write_all(&len.to_be_bytes())?;
    w.write_all(&body)?;
    w.flush()?;
    Ok(())
}

/// Reads one frame. `Ok(None)` on a clean EOF before the first byte of a
/// frame; EOF inside a frame is an error.
pub fn read_frame<R: Read, T: DeserializeOwned>(
    r: &mut R,
    max: usize,
) -> Result<Option<T>, FrameError> {
    let mut prefix = [0u8; 4];
    let mut got = 0;
    while got < prefix.len() {
        match r.read(&mut prefix[got..]) {
            Ok(0) if got == 0 => return Ok(None),
            Ok(0) => return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into()),
            Ok(n) => got += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.into()),
        }
    }
    let len = u32::from_be_bytes(prefix) as usize;
    if len > max {
        return Err(FrameError::TooLarge { len, max });
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(FrameError::Malformed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ToHost;

    fn frame(body: &[u8]) -> Vec<u8> {
        let mut v = (body.len() as u32).to_be_bytes().to_vec();
        v.extend_from_slice(body);
        v
    }

    #[test]
    fn round_trip() {
        let mut buf = Vec::new();
        write_frame(&mut buf, &ToHost::Shutdown {}, 1024).unwrap();
        let mut r = buf.as_slice();
        let m: Option<ToHost> = read_frame(&mut r, 1024).unwrap();
        assert_eq!(m, Some(ToHost::Shutdown {}));
        let eof: Option<ToHost> = read_frame(&mut r, 1024).unwrap();
        assert_eq!(eof, None);
    }

    #[test]
    fn oversized_prefix_is_rejected_before_allocating() {
        let bytes = u32::MAX.to_be_bytes();
        let r = read_frame::<_, ToHost>(&mut bytes.as_slice(), 1024);
        assert!(matches!(r, Err(FrameError::TooLarge { max: 1024, .. })));
    }

    #[test]
    fn writer_refuses_oversized_bodies() {
        let mut buf = Vec::new();
        let r = write_frame(&mut buf, &"x".repeat(100), 10);
        assert!(matches!(r, Err(FrameError::TooLarge { .. })));
        assert!(buf.is_empty());
    }

    #[test]
    fn truncated_and_malformed_frames_fail() {
        let mut short = frame(br#"{"type":"shutdown"}"#);
        short.truncate(10);
        assert!(matches!(
            read_frame::<_, ToHost>(&mut short.as_slice(), 1024),
            Err(FrameError::Io(_))
        ));
        let partial_prefix = [0u8, 0];
        assert!(read_frame::<_, ToHost>(&mut partial_prefix.as_slice(), 1024).is_err());
        for body in [
            &b"not json"[..],
            b"{}",
            br#"{"type":"nope"}"#,
            b"\xff\xfe",
            b"",
        ] {
            let f = frame(body);
            assert!(
                matches!(
                    read_frame::<_, ToHost>(&mut f.as_slice(), 1024),
                    Err(FrameError::Malformed(_))
                ),
                "{body:?}"
            );
        }
    }
}
