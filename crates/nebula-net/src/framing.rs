//! Length-prefixed framing for TCP streams.
//!
//! Every message on the wire is a length-prefixed frame:
//!
//! ```text
//! +-------------------+--------------------+
//! | length (4 bytes)  |   payload          |
//! | u32 little-endian |   (length bytes)   |
//! +-------------------+--------------------+
//! ```
//!
//! The 4-byte length prefix encodes the payload size as a `u32` in little-endian
//! byte order. The length does **not** include the 4 prefix bytes themselves.
//! A length of 0 is a valid no-op frame (used for keepalive padding).

use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Configuration for the framing layer.
#[derive(Debug, Clone)]
pub struct FrameConfig {
    /// Maximum allowed payload size in bytes. Default: 1 MB.
    pub max_payload_size: u32,
}

impl Default for FrameConfig {
    fn default() -> Self {
        Self {
            max_payload_size: 1_048_576,
        }
    }
}

/// Errors that can occur during framing operations.
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    /// The payload size exceeds the configured maximum.
    #[error("payload size {size} exceeds maximum {max}")]
    PayloadTooLarge {
        /// The actual payload size.
        size: u32,
        /// The configured maximum.
        max: u32,
    },

    /// The connection was closed before a complete frame was received.
    #[error("connection closed")]
    ConnectionClosed,

    /// An I/O error occurred.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Read a single length-prefixed frame from the stream.
///
/// Returns the payload bytes. Blocks until the full frame is available.
/// Returns [`FrameError::ConnectionClosed`] if the peer closes the connection
/// before the frame is complete.
pub async fn read_frame<R: AsyncReadExt + Unpin>(
    reader: &mut R,
    config: &FrameConfig,
) -> Result<Vec<u8>, FrameError> {
    // Read the 4-byte length prefix
    let mut len_buf = [0u8; 4];
    match reader.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Err(FrameError::ConnectionClosed);
        }
        Err(e) => return Err(FrameError::Io(e)),
    }

    let payload_len = u32::from_le_bytes(len_buf);

    // Enforce maximum size
    if payload_len > config.max_payload_size {
        return Err(FrameError::PayloadTooLarge {
            size: payload_len,
            max: config.max_payload_size,
        });
    }

    // Read the payload
    let mut payload = vec![0u8; payload_len as usize];
    if payload_len > 0 {
        reader.read_exact(&mut payload).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::UnexpectedEof {
                FrameError::ConnectionClosed
            } else {
                FrameError::Io(e)
            }
        })?;
    }

    Ok(payload)
}

/// Write a single length-prefixed frame to the stream.
///
/// The payload is prefixed with its length as a `u32` little-endian value.
pub async fn write_frame<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    payload: &[u8],
    config: &FrameConfig,
) -> Result<(), FrameError> {
    let len = payload.len() as u32;
    if len > config.max_payload_size {
        return Err(FrameError::PayloadTooLarge {
            size: len,
            max: config.max_payload_size,
        });
    }

    writer.write_all(&len.to_le_bytes()).await?;
    if !payload.is_empty() {
        writer.write_all(payload).await?;
    }
    writer.flush().await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    fn default_config() -> FrameConfig {
        FrameConfig::default()
    }

    #[tokio::test]
    async fn test_single_message_roundtrip() {
        let (mut client, mut server) = duplex(8192);
        let config = default_config();
        let payload = b"hello world";

        write_frame(&mut client, payload, &config).await.unwrap();
        let received = read_frame(&mut server, &config).await.unwrap();
        assert_eq!(received, payload);
    }

    #[tokio::test]
    async fn test_multiple_messages_in_sequence() {
        let (mut client, mut server) = duplex(8192);
        let config = default_config();

        let messages: Vec<&[u8]> = vec![b"first", b"second", b"third"];
        for msg in &messages {
            write_frame(&mut client, msg, &config).await.unwrap();
        }

        for expected in &messages {
            let received = read_frame(&mut server, &config).await.unwrap();
            assert_eq!(received, *expected);
        }
    }

    #[tokio::test]
    async fn test_partial_read_resumes_correctly() {
        // duplex with a tiny buffer forces partial writes/reads
        let (mut client, mut server) = duplex(8); // Very small buffer
        let config = default_config();
        let payload = b"this message is larger than the buffer";

        let write_config = config.clone();
        let write_task = tokio::spawn(async move {
            write_frame(&mut client, payload, &write_config)
                .await
                .unwrap();
        });

        let received = read_frame(&mut server, &config).await.unwrap();
        write_task.await.unwrap();
        assert_eq!(received, payload);
    }

    #[tokio::test]
    async fn test_oversized_message_rejected_on_read() {
        let (mut client, mut server) = duplex(8192);
        let config = FrameConfig {
            max_payload_size: 16,
        };

        // Manually write a frame with a length prefix that exceeds the limit
        let fake_len: u32 = 1024;
        client.write_all(&fake_len.to_le_bytes()).await.unwrap();
        client.flush().await.unwrap();

        let result = read_frame(&mut server, &config).await;
        assert!(
            matches!(result, Err(FrameError::PayloadTooLarge { .. })),
            "Should reject oversized frame"
        );
    }

    #[tokio::test]
    async fn test_oversized_message_rejected_on_write() {
        let (mut client, _server) = duplex(8192);
        let config = FrameConfig {
            max_payload_size: 16,
        };

        let big_payload = vec![0u8; 1024];
        let result = write_frame(&mut client, &big_payload, &config).await;
        assert!(
            matches!(result, Err(FrameError::PayloadTooLarge { .. })),
            "Should reject oversized frame on write"
        );
    }

    #[tokio::test]
    async fn test_zero_length_message_handled() {
        let (mut client, mut server) = duplex(8192);
        let config = default_config();

        write_frame(&mut client, &[], &config).await.unwrap();
        let received = read_frame(&mut server, &config).await.unwrap();
        assert!(received.is_empty(), "Zero-length payload should be valid");
    }

    #[tokio::test]
    async fn test_back_to_back_messages_dont_merge() {
        let (mut client, mut server) = duplex(8192);
        let config = default_config();

        write_frame(&mut client, b"aaa", &config).await.unwrap();
        write_frame(&mut client, b"bbb", &config).await.unwrap();

        let first = read_frame(&mut server, &config).await.unwrap();
        let second = read_frame(&mut server, &config).await.unwrap();

        assert_eq!(first, b"aaa");
        assert_eq!(second, b"bbb");
        // Verify they were not merged into "aaabbb"
        assert_ne!(first.len(), 6);
    }

    #[tokio::test]
    async fn test_connection_closed_during_length_read() {
        let (client, mut server) = duplex(8192);
        // Drop the writer side immediately
        drop(client);

        let config = default_config();
        let result = read_frame(&mut server, &config).await;
        assert!(
            matches!(result, Err(FrameError::ConnectionClosed)),
            "Should detect closed connection"
        );
    }

    #[tokio::test]
    async fn test_length_prefix_is_little_endian() {
        let (mut client, mut server) = duplex(8192);
        let config = default_config();

        // Manually write a frame: length 5 in little-endian, then "hello"
        let len_bytes: [u8; 4] = 5u32.to_le_bytes();
        client.write_all(&len_bytes).await.unwrap();
        client.write_all(b"hello").await.unwrap();
        client.flush().await.unwrap();

        let received = read_frame(&mut server, &config).await.unwrap();
        assert_eq!(received, b"hello");
    }
}
