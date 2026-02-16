//! Network message types and serialization.
//!
//! All messages are serialized with [`postcard`] and prefixed with a protocol
//! version byte. Use [`serialize_message`] and [`deserialize_message`] for
//! encoding/decoding.

use serde::{Deserialize, Serialize};

/// Current wire-protocol version. Prepended to every serialized message.
pub const PROTOCOL_VERSION: u8 = 1;

// ---------------------------------------------------------------------------
// Top-level enum
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Payload structs
// ---------------------------------------------------------------------------

/// Client login request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LoginRequest {
    /// Desired player name.
    pub player_name: String,
}

/// Server login response.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LoginResponse {
    /// Assigned player identifier.
    pub player_id: u64,
    /// Whether the login succeeded.
    pub success: bool,
    /// Human-readable status message.
    pub message: String,
}

/// Logout notification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Logout {
    /// Player that is logging out.
    pub player_id: u64,
    /// Reason for disconnection.
    pub reason: String,
}

/// Chunk voxel data sent from server to client.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChunkData {
    /// Chunk coordinate X on the cubesphere grid.
    pub chunk_x: i64,
    /// Chunk coordinate Y on the cubesphere grid.
    pub chunk_y: i64,
    /// Chunk coordinate Z on the cubesphere grid.
    pub chunk_z: i64,
    /// Face index of the cubesphere (0–5).
    pub face: u8,
    /// Compressed voxel data.
    pub voxel_data: Vec<u8>,
}

/// Entity state update (position + rotation).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EntityUpdate {
    /// Entity identifier.
    pub entity_id: u64,
    /// 128-bit X position, high 64 bits.
    pub pos_x_high: i64,
    /// 128-bit X position, low 64 bits.
    pub pos_x_low: i64,
    /// 128-bit Y position, high 64 bits.
    pub pos_y_high: i64,
    /// 128-bit Y position, low 64 bits.
    pub pos_y_low: i64,
    /// 128-bit Z position, high 64 bits.
    pub pos_z_high: i64,
    /// 128-bit Z position, low 64 bits.
    pub pos_z_low: i64,
    /// Rotation quaternion X component.
    pub rot_x: f32,
    /// Rotation quaternion Y component.
    pub rot_y: f32,
    /// Rotation quaternion Z component.
    pub rot_z: f32,
    /// Rotation quaternion W component.
    pub rot_w: f32,
}

/// Player position update from client.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlayerPosition {
    /// Player identifier.
    pub player_id: u64,
    /// 128-bit X position, high 64 bits.
    pub pos_x_high: i64,
    /// 128-bit X position, low 64 bits.
    pub pos_x_low: i64,
    /// 128-bit Y position, high 64 bits.
    pub pos_y_high: i64,
    /// 128-bit Y position, low 64 bits.
    pub pos_y_low: i64,
    /// 128-bit Z position, high 64 bits.
    pub pos_z_high: i64,
    /// 128-bit Z position, low 64 bits.
    pub pos_z_low: i64,
}

/// Player action (place/break voxel, interact, etc.).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlayerAction {
    /// Player identifier.
    pub player_id: u64,
    /// Action type discriminant.
    pub action_type: u16,
    /// Target X coordinate.
    pub target_x: i64,
    /// Target Y coordinate.
    pub target_y: i64,
    /// Target Z coordinate.
    pub target_z: i64,
    /// Action-specific payload bytes.
    pub payload: Vec<u8>,
}

/// Heartbeat ping.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Ping {
    /// Sender timestamp in milliseconds.
    pub timestamp_ms: u64,
    /// Sequence number.
    pub sequence: u32,
}

/// Heartbeat pong (response to [`Ping`]).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Pong {
    /// Echoed timestamp in milliseconds.
    pub timestamp_ms: u64,
    /// Echoed sequence number.
    pub sequence: u32,
}

/// Time synchronization message for clock alignment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TimeSync {
    /// Client send timestamp (ms).
    pub client_send_ms: u64,
    /// Server receive timestamp (ms).
    pub server_recv_ms: u64,
    /// Server send timestamp (ms).
    pub server_send_ms: u64,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors that can occur during message deserialization.
#[derive(Debug, thiserror::Error)]
pub enum MessageError {
    /// The payload was empty (no version byte).
    #[error("empty payload — no version byte")]
    EmptyPayload,

    /// The version byte does not match [`PROTOCOL_VERSION`].
    #[error("unsupported protocol version: {0}")]
    UnsupportedVersion(u8),

    /// Postcard deserialization failed.
    #[error("deserialization error: {0}")]
    Postcard(#[from] postcard::Error),
}

// ---------------------------------------------------------------------------
// Serialization helpers
// ---------------------------------------------------------------------------

/// Serialize a [`Message`] into a versioned binary payload.
///
/// Wire format: `[version: u8] [postcard-encoded Message]`
pub fn serialize_message(msg: &Message) -> Result<Vec<u8>, postcard::Error> {
    let body = postcard::to_allocvec(msg)?;
    let mut out = Vec::with_capacity(1 + body.len());
    out.push(PROTOCOL_VERSION);
    out.extend_from_slice(&body);
    Ok(out)
}

/// Deserialize a versioned binary payload into a [`Message`].
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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
        assert!(
            result.is_err(),
            "Corrupted payload should fail deserialization"
        );
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
