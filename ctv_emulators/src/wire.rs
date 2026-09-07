//! Length-prefixed JSON shared by the emulator client and server.
use serde::{de::DeserializeOwned, Serialize};
use std::io::{Error, ErrorKind};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// A frame contains at most one million bytes of JSON.
const MAX_MESSAGE_BYTES: usize = 1_000_000;

pub(crate) async fn read_message<T: DeserializeOwned>(
    stream: &mut (impl AsyncRead + Unpin),
) -> Result<T, Error> {
    let length = stream.read_u32().await? as usize;
    if length == 0 || length > MAX_MESSAGE_BYTES {
        return Err(Error::new(
            ErrorKind::InvalidData,
            "Invalid emulator frame length",
        ));
    }
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes).await?;
    serde_json::from_slice(&bytes).map_err(|error| Error::new(ErrorKind::InvalidData, error))
}

pub(crate) async fn write_message<T: Serialize>(
    stream: &mut (impl AsyncWrite + Unpin),
    value: &T,
) -> Result<(), Error> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "Emulator frame exceeds one million bytes",
        ));
    }
    stream.write_u32(bytes.len() as u32).await?;
    stream.write_all(&bytes).await?;
    stream.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    #[tokio::test]
    async fn reads_multiple_frames_without_losing_boundaries() {
        let (mut client, mut server) = tokio::io::duplex(128);
        for value in [json!({"first": 1}), json!(["second", 2])] {
            write_message(&mut client, &value).await.unwrap();
            assert_eq!(read_message::<Value>(&mut server).await.unwrap(), value);
        }
    }

    #[tokio::test]
    async fn rejects_invalid_lengths_before_reading_a_body() {
        for length in [0, MAX_MESSAGE_BYTES as u32 + 1, u32::MAX] {
            let (mut client, mut server) = tokio::io::duplex(16);
            client.write_u32(length).await.unwrap();
            drop(client);
            assert_eq!(
                read_message::<Value>(&mut server).await.unwrap_err().kind(),
                ErrorKind::InvalidData
            );
        }
    }

    #[tokio::test]
    async fn rejects_truncated_frames_and_oversized_responses() {
        let (mut client, mut server) = tokio::io::duplex(16);
        client.write_u32(10).await.unwrap();
        client.write_all(b"{}").await.unwrap();
        drop(client);
        assert_eq!(
            read_message::<Value>(&mut server).await.unwrap_err().kind(),
            ErrorKind::UnexpectedEof
        );
        let mut output = Vec::new();
        let oversized = "x".repeat(MAX_MESSAGE_BYTES);
        assert_eq!(
            write_message(&mut output, &oversized)
                .await
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidInput
        );
        assert!(output.is_empty());
    }
}
