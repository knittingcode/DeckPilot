//! Newline-delimited JSON framing with a size limit.

use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Largest accepted message, excluding the newline.
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024;

#[derive(Debug, Error)]
pub enum FrameError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("message exceeds {MAX_MESSAGE_BYTES} bytes")]
    TooLarge,
    #[error("malformed message: {0}")]
    Malformed(#[from] serde_json::Error),
    #[error("message is not valid UTF-8")]
    Encoding,
}

/// Reads one message. Returns `Ok(None)` when the peer closed the connection.
/// After [`FrameError::TooLarge`] the stream is no longer aligned to message
/// boundaries and must be closed.
pub async fn read_message<R, T>(reader: &mut R) -> Result<Option<T>, FrameError>
where
    R: AsyncBufRead + Unpin,
    T: DeserializeOwned,
{
    let mut buffer = Vec::new();
    let limit = (MAX_MESSAGE_BYTES + 1) as u64;
    let read = (&mut *reader)
        .take(limit)
        .read_until(b'\n', &mut buffer)
        .await?;
    if read == 0 {
        return Ok(None);
    }
    if buffer.last() == Some(&b'\n') {
        buffer.pop();
    } else if buffer.len() > MAX_MESSAGE_BYTES {
        return Err(FrameError::TooLarge);
    }
    let text = std::str::from_utf8(&buffer).map_err(|_| FrameError::Encoding)?;
    Ok(Some(serde_json::from_str(text.trim_end_matches('\r'))?))
}

pub async fn write_message<W, T>(writer: &mut W, message: &T) -> Result<(), FrameError>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let mut bytes = serde_json::to_vec(message)?;
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err(FrameError::TooLarge);
    }
    bytes.push(b'\n');
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::BufReader;

    #[tokio::test]
    async fn round_trips_and_enforces_limits() {
        let mut out = Vec::new();
        write_message(&mut out, &serde_json::json!({ "a": 1 }))
            .await
            .unwrap();
        write_message(&mut out, &serde_json::json!([1, 2]))
            .await
            .unwrap();
        let mut reader = BufReader::new(out.as_slice());
        let first: serde_json::Value = read_message(&mut reader).await.unwrap().unwrap();
        assert_eq!(first["a"], 1);
        let second: serde_json::Value = read_message(&mut reader).await.unwrap().unwrap();
        assert_eq!(second[1], 2);
        assert!(
            read_message::<_, serde_json::Value>(&mut reader)
                .await
                .unwrap()
                .is_none()
        );

        let mut reader = BufReader::new(&b"{not json}\n"[..]);
        assert!(matches!(
            read_message::<_, serde_json::Value>(&mut reader).await,
            Err(FrameError::Malformed(_))
        ));

        let huge = vec![b'x'; MAX_MESSAGE_BYTES + 10];
        let mut reader = BufReader::new(huge.as_slice());
        assert!(matches!(
            read_message::<_, serde_json::Value>(&mut reader).await,
            Err(FrameError::TooLarge)
        ));

        let mut reader = BufReader::new(&b"\xff\xfe\n"[..]);
        assert!(matches!(
            read_message::<_, serde_json::Value>(&mut reader).await,
            Err(FrameError::Encoding)
        ));
    }
}
