# Message Types & Serialization

## Problem

The Nebula Engine's networking layer needs a well-defined set of message types that cover all communication between client and server. Messages must be serialized into a compact binary format for efficient transmission — the engine deals with large voxel chunk data and frequent entity position updates (using 128-bit coordinates), so verbosity is unacceptable. The serialization format must be deterministic, fast, and safe against malicious input. **bincode is not an option** (RUSTSEC-2025-0141, unmaintained and unsound). `postcard` 1.1 is the replacement — it produces compact output with a well-defined wire format (varint encoding for integers, no padding), is `no_std`-compatible, and actively maintained. Each message needs a type tag so the routing layer (story 05) can dispatch it without fully deserializing first. A version byte at the start of the payload allows backward-compatible evolution.

## Solution

### Protocol version

Every serialized message starts with a single version byte. The current protocol version is `1`. When the server receives a message with an unknown version, it can reject it gracefully or attempt a compatibility path.

```rust
pub const PROTOCOL_VERSION: u8 = 1;
```

### Message enum

The `Message` enum is the top-level type for all network communication. Each variant maps to a category: Auth, World, Player, or System. Serde's internally-tagged representation is not used — instead, postcard serializes the enum discriminant as a varint, which acts as the type tag.

```rust
use serde::{Serialize, Deserialize};

/// Top-level network message. The enum discriminant is the type tag.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Message {
    // --- Auth ---
    /// Client requests login with a player name.
    LoginRequest(LoginRequest),
    /// Server confirms login with assigned player ID.
    LoginResponse(LoginResponse),
    /// Client or server initiates logout.
    Logout(Logout),

    // --- World ---
    /// Server sends chunk voxel data to client.
    ChunkData(ChunkData),
    /// Server sends a batch of entity state updates.
    EntityUpdate(EntityUpdate),

    // --- Player ---
    /// Client sends its current position to the server.
    PlayerPosition(PlayerPosition),
    /// Client sends a player action (e.g., place/break voxel).
    PlayerAction(PlayerAction),

    // --- System ---
    /// Heartbeat ping. Sender expects a Pong in response.
    Ping(Ping),
    /// Heartbeat pong. Response to a Ping.
    Pong(Pong),
    /// Time synchronization message for clock alignment.
    TimeSync(TimeSync),
}
```

### Message payload structs

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LoginRequest {
    pub player_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LoginResponse {
    pub player_id: u64,
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Logout {
    pub player_id: u64,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChunkData {
    /// Chunk coordinate on the cubesphere grid.
    pub chunk_x: i64,
    pub chunk_y: i64,
    pub chunk_z: i64,
    /// Face index of the cubesphere (0-5).
    pub face: u8,
    /// Compressed voxel data (compressed by story 07 if above threshold).
    pub voxel_data: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EntityUpdate {
    pub entity_id: u64,
    /// 128-bit position components, serialized as pairs of i64 (high, low).
    pub pos_x_high: i64,
    pub pos_x_low: i64,
    pub pos_y_high: i64,
    pub pos_y_low: i64,
    pub pos_z_high: i64,
    pub pos_z_low: i64,
    /// Rotation as a quaternion (f32 components).
    pub rot_x: f32,
    pub rot_y: f32,
    pub rot_z: f32,
    pub rot_w: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlayerPosition {
    pub player_id: u64,
    pub pos_x_high: i64,
    pub pos_x_low: i64,
    pub pos_y_high: i64,
    pub pos_y_low: i64,
    pub pos_z_high: i64,
    pub pos_z_low: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlayerAction {
    pub player_id: u64,
    pub action_type: u16,
    pub target_x: i64,
    pub target_y: i64,
    pub target_z: i64,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Ping {
    pub timestamp_ms: u64,
    pub sequence: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Pong {
    pub timestamp_ms: u64,
    pub sequence: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TimeSync {
    pub client_send_ms: u64,
    pub server_recv_ms: u64,
    pub server_send_ms: u64,
}
```

### Serialization and deserialization

```rust
/// Serialize a Message into a versioned binary payload suitable for framing (story 03).
///
/// Wire format: [version: u8] [postcard-encoded Message]
pub fn serialize_message(msg: &Message) -> Result<Vec<u8>, postcard::Error> {
    let body = postcard::to_allocvec(msg)?;
    let mut out = Vec::with_capacity(1 + body.len());
    out.push(PROTOCOL_VERSION);
    out.extend_from_slice(&body);
    Ok(out)
}

/// Deserialize a versioned binary payload into a Message.
///
/// Returns an error if the version is unsupported or the payload is malformed.
pub fn deserialize_message(data: &[u8]) -> Result<Message, MessageError> {
    if data.is_empty() {
        return Err(MessageError::EmptyPayload);
    }

    let version = data[0];
    if version != PROTOCOL_VERSION {
        return Err(MessageError::UnsupportedVersion(version));
    }

    let msg = postcard::from_bytes(&data[1..])?;
    Ok(msg)
}

#[derive(Debug, thiserror::Error)]
pub enum MessageError {
    #[error("empty payload — no version byte")]
    EmptyPayload,

    #[error("unsupported protocol version: {0}")]
    UnsupportedVersion(u8),

    #[error("deserialization error: {0}")]
    Postcard(#[from] postcard::Error),
}
```

### 128-bit coordinate serialization strategy

The engine uses 128-bit coordinates (`i128`), but serde's default i128 support varies across formats. To guarantee portability, each 128-bit coordinate is split into two `i64` fields (`high` and `low`) at the message-type level. Reconstruction is: `(high as i128) << 64 | (low as u64 as i128)`. This keeps the wire format simple and avoids format-specific i128 quirks.

### Type tag extraction

For routing (story 05), the message's type tag can be extracted without full deserialization by peeking at the postcard-encoded enum discriminant (the first varint after the version byte). However, for simplicity and safety, the initial implementation fully deserializes and matches on the enum variant. Optimization to peek-based routing can be added later if profiling shows it matters.

## Outcome

A `messages.rs` module in `crates/nebula_net/src/` exporting the `Message` enum, all payload structs, `serialize_message`, `deserialize_message`, `MessageError`, and `PROTOCOL_VERSION`. All network messages are serialized with postcard 1.1 and prefixed with a protocol version byte. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

The client sends a `Ping` message; the server responds with `Pong`. The console shows `Sent Ping, received Pong (1.2ms round trip)`. Messages use postcard serialization.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `serde` | `1.0` (features: `derive`) | `Serialize`, `Deserialize` derive for all message types |
| `postcard` | `1.1` (features: `alloc`) | Compact binary serialization — replaces bincode (RUSTSEC-2025-0141) |
| `thiserror` | `2.0` | Derive `Error` for `MessageError` |

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_login_request_roundtrip() {
        let msg = Message::LoginRequest(LoginRequest {
            player_name: "Alice".to_string(),
        });
        let bytes = serialize_message(&msg).unwrap();
        let decoded = deserialize_message(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_login_response_roundtrip() {
        let msg = Message::LoginResponse(LoginResponse {
            player_id: 42,
            success: true,
            message: "Welcome".to_string(),
        });
        let bytes = serialize_message(&msg).unwrap();
        let decoded = deserialize_message(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_chunk_data_roundtrip() {
        let msg = Message::ChunkData(ChunkData {
            chunk_x: -100,
            chunk_y: 50,
            chunk_z: 200,
            face: 3,
            voxel_data: vec![1, 2, 3, 4, 5, 0, 255],
        });
        let bytes = serialize_message(&msg).unwrap();
        let decoded = deserialize_message(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_entity_update_128bit_coords_roundtrip() {
        let msg = Message::EntityUpdate(EntityUpdate {
            entity_id: 999,
            pos_x_high: i64::MAX,
            pos_x_low: i64::MIN,
            pos_y_high: 0,
            pos_y_low: 1,
            pos_z_high: -1,
            pos_z_low: 0,
            rot_x: 0.0,
            rot_y: 0.707,
            rot_z: 0.0,
            rot_w: 0.707,
        });
        let bytes = serialize_message(&msg).unwrap();
        let decoded = deserialize_message(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_ping_pong_roundtrip() {
        let ping = Message::Ping(Ping {
            timestamp_ms: 1234567890,
            sequence: 42,
        });
        let pong = Message::Pong(Pong {
            timestamp_ms: 1234567891,
            sequence: 42,
        });
        for msg in [ping, pong] {
            let bytes = serialize_message(&msg).unwrap();
            let decoded = deserialize_message(&bytes).unwrap();
            assert_eq!(msg, decoded);
        }
    }

    #[test]
    fn test_time_sync_roundtrip() {
        let msg = Message::TimeSync(TimeSync {
            client_send_ms: 1000,
            server_recv_ms: 1005,
            server_send_ms: 1006,
        });
        let bytes = serialize_message(&msg).unwrap();
        let decoded = deserialize_message(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_player_action_roundtrip() {
        let msg = Message::PlayerAction(PlayerAction {
            player_id: 7,
            action_type: 1,
            target_x: -500,
            target_y: 100,
            target_z: 300,
            payload: vec![0xDE, 0xAD, 0xBE, 0xEF],
        });
        let bytes = serialize_message(&msg).unwrap();
        let decoded = deserialize_message(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_postcard_output_is_compact() {
        let msg = Message::Ping(Ping {
            timestamp_ms: 100,
            sequence: 1,
        });
        let bytes = serialize_message(&msg).unwrap();
        // Version byte (1) + postcard enum discriminant (varint, ~1-2 bytes)
        // + timestamp_ms (varint, ~2 bytes) + sequence (varint, ~1 byte)
        // Total should be well under 20 bytes for such a small message.
        assert!(
            bytes.len() < 20,
            "Ping should be compact, got {} bytes",
            bytes.len()
        );
    }

    #[test]
    fn test_unsupported_version_rejected() {
        let msg = Message::Ping(Ping {
            timestamp_ms: 0,
            sequence: 0,
        });
        let mut bytes = serialize_message(&msg).unwrap();
        // Corrupt the version byte
        bytes[0] = 255;
        let result = deserialize_message(&bytes);
        assert!(matches!(result, Err(MessageError::UnsupportedVersion(255))));
    }

    #[test]
    fn test_empty_payload_rejected() {
        let result = deserialize_message(&[]);
        assert!(matches!(result, Err(MessageError::EmptyPayload)));
    }

    #[test]
    fn test_corrupted_payload_rejected() {
        let result = deserialize_message(&[PROTOCOL_VERSION, 0xFF, 0xFF, 0xFF]);
        assert!(result.is_err(), "Corrupted payload should fail deserialization");
    }

    #[test]
    fn test_all_fields_survive_roundtrip_player_position() {
        let msg = Message::PlayerPosition(PlayerPosition {
            player_id: u64::MAX,
            pos_x_high: i64::MAX,
            pos_x_low: i64::MIN,
            pos_y_high: 0,
            pos_y_low: 0,
            pos_z_high: -1,
            pos_z_low: -1,
        });
        let bytes = serialize_message(&msg).unwrap();
        let decoded = deserialize_message(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_version_byte_is_first_byte() {
        let msg = Message::Logout(Logout {
            player_id: 1,
            reason: "quit".to_string(),
        });
        let bytes = serialize_message(&msg).unwrap();
        assert_eq!(bytes[0], PROTOCOL_VERSION);
    }
}
```
