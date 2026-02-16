//! TCP networking: connection management, message framing, serialization, and connection lifecycle.

pub mod compression;
pub mod framing;
pub mod messages;
pub mod reconnection;
pub mod routing;
pub mod session;
pub mod tcp_client;
pub mod tcp_server;

pub use compression::{
    COMPRESSION_FLAG_LZ4, COMPRESSION_FLAG_NONE, CompressionConfig, CompressionError,
    compress_payload, decompress_payload,
};
pub use framing::{FrameConfig, FrameError, read_frame, write_frame};
pub use messages::{
    ChunkData, EntityUpdate, LoginRequest, LoginResponse, Logout, Message, MessageError,
    PROTOCOL_VERSION, Ping, PlayerAction, PlayerPosition, Pong, TimeSync, deserialize_message,
    serialize_message,
};
pub use reconnection::{
    ExtendedSessionState, GraceConfig, ReconnectConfig, ReconnectError, ReconnectState,
    expire_suspended_sessions, reconnect_loop,
};
pub use routing::{
    HandlerContext, IncomingMessage, MessageHandler, MessageRouter, MessageTag, message_channel,
    process_incoming_messages,
};
pub use session::{AuthError, PlayerSession, SessionManager, SessionState, timeout_check};
pub use tcp_client::{ConnectionState, ConnectionStateWatch, GameClient};
pub use tcp_server::{
    ConnectionId, ConnectionLimitReached, ConnectionMap, GameServer, IdGenerator, ServerConfig,
};
