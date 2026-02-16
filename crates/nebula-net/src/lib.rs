//! TCP networking: connection management, message framing, serialization, and connection lifecycle.

pub mod framing;
pub mod tcp_client;
pub mod tcp_server;

pub use framing::{FrameConfig, FrameError, read_frame, write_frame};
pub use tcp_client::{ConnectionState, ConnectionStateWatch, GameClient};
pub use tcp_server::{
    ConnectionId, ConnectionLimitReached, ConnectionMap, GameServer, IdGenerator, ServerConfig,
};
