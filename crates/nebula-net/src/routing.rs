//! Message routing: dispatch incoming messages to type-specific handlers.
//!
//! The [`MessageRouter`] maps [`MessageTag`] values to [`MessageHandler`]
//! implementations. Messages arrive from the network task via a bounded
//! [`tokio::sync::mpsc`] channel and are drained each game tick by
//! [`process_incoming_messages`].

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::mpsc;

use crate::messages::Message;
use crate::tcp_server::{ConnectionId, ConnectionMap};

// ---------------------------------------------------------------------------
// MessageTag
// ---------------------------------------------------------------------------

/// Unique tag identifying a message type, used as the key for routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MessageTag {
    /// Client login request.
    LoginRequest,
    /// Server login response.
    LoginResponse,
    /// Logout notification.
    Logout,
    /// Chunk voxel data.
    ChunkData,
    /// Entity state update.
    EntityUpdate,
    /// Player position update.
    PlayerPosition,
    /// Player action.
    PlayerAction,
    /// Heartbeat ping.
    Ping,
    /// Heartbeat pong.
    Pong,
    /// Time synchronization.
    TimeSync,
}

impl Message {
    /// Extract the routing tag from a message without consuming it.
    pub fn tag(&self) -> MessageTag {
        match self {
            Message::LoginRequest(_) => MessageTag::LoginRequest,
            Message::LoginResponse(_) => MessageTag::LoginResponse,
            Message::Logout(_) => MessageTag::Logout,
            Message::ChunkData(_) => MessageTag::ChunkData,
            Message::EntityUpdate(_) => MessageTag::EntityUpdate,
            Message::PlayerPosition(_) => MessageTag::PlayerPosition,
            Message::PlayerAction(_) => MessageTag::PlayerAction,
            Message::Ping(_) => MessageTag::Ping,
            Message::Pong(_) => MessageTag::Pong,
            Message::TimeSync(_) => MessageTag::TimeSync,
        }
    }
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

/// Context provided to every message handler.
pub struct HandlerContext {
    /// The connection that sent this message.
    pub connection_id: ConnectionId,
    /// Shared reference to the connection map for sending responses.
    pub connections: Arc<ConnectionMap>,
}

/// Trait for message handlers. Implemented as a boxed closure.
pub trait MessageHandler: Send + Sync {
    /// Process a single incoming message.
    fn handle(&self, msg: Message, ctx: &HandlerContext);
}

/// Blanket implementation for closures.
impl<F> MessageHandler for F
where
    F: Fn(Message, &HandlerContext) + Send + Sync,
{
    fn handle(&self, msg: Message, ctx: &HandlerContext) {
        self(msg, ctx);
    }
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Routes incoming messages to registered handlers by [`MessageTag`].
pub struct MessageRouter {
    handlers: HashMap<MessageTag, Box<dyn MessageHandler>>,
}

impl MessageRouter {
    /// Create an empty router.
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }

    /// Register a handler for a specific message tag.
    pub fn register<H: MessageHandler + 'static>(&mut self, tag: MessageTag, handler: H) {
        self.handlers.insert(tag, Box::new(handler));
    }

    /// Route an incoming message to the registered handler.
    ///
    /// Returns `true` if a handler was found, `false` if the message was
    /// dropped.
    pub fn route(&self, msg: Message, ctx: &HandlerContext) -> bool {
        let tag = msg.tag();
        if let Some(handler) = self.handlers.get(&tag) {
            handler.handle(msg, ctx);
            true
        } else {
            tracing::warn!("No handler registered for {:?}, dropping message", tag);
            false
        }
    }

    /// Return an iterator over registered tags (useful for startup logging).
    pub fn registered_tags(&self) -> impl Iterator<Item = &MessageTag> {
        self.handlers.keys()
    }
}

impl Default for MessageRouter {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Channel bridge
// ---------------------------------------------------------------------------

/// Envelope carrying a message and its source connection.
pub struct IncomingMessage {
    /// The connection that sent this message.
    pub connection_id: ConnectionId,
    /// The deserialized message.
    pub message: Message,
}

/// Create a channel pair for passing messages from network tasks to the game
/// thread.
pub fn message_channel(
    buffer: usize,
) -> (
    mpsc::Sender<IncomingMessage>,
    mpsc::Receiver<IncomingMessage>,
) {
    mpsc::channel(buffer)
}

/// Drain all pending incoming messages and route them.
pub fn process_incoming_messages(
    receiver: &mut mpsc::Receiver<IncomingMessage>,
    router: &MessageRouter,
    connections: &Arc<ConnectionMap>,
) {
    while let Ok(incoming) = receiver.try_recv() {
        let ctx = HandlerContext {
            connection_id: incoming.connection_id,
            connections: Arc::clone(connections),
        };
        router.route(incoming.message, &ctx);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::*;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

    fn dummy_ctx() -> HandlerContext {
        HandlerContext {
            connection_id: ConnectionId(1),
            connections: Arc::new(ConnectionMap::new(16)),
        }
    }

    #[test]
    fn test_message_routed_to_correct_handler() {
        let handled = Arc::new(AtomicBool::new(false));
        let handled_clone = Arc::clone(&handled);

        let mut router = MessageRouter::new();
        router.register(
            MessageTag::Ping,
            move |_msg: Message, _ctx: &HandlerContext| {
                handled_clone.store(true, Ordering::SeqCst);
            },
        );

        let msg = Message::Ping(Ping {
            timestamp_ms: 0,
            sequence: 0,
        });
        let ctx = dummy_ctx();
        router.route(msg, &ctx);

        assert!(
            handled.load(Ordering::SeqCst),
            "Ping handler should have been called"
        );
    }

    #[test]
    fn test_unknown_message_type_dropped() {
        let router = MessageRouter::new(); // No handlers registered
        let msg = Message::Ping(Ping {
            timestamp_ms: 0,
            sequence: 0,
        });
        let ctx = dummy_ctx();
        let routed = router.route(msg, &ctx);
        assert!(!routed, "Message with no handler should return false");
    }

    #[test]
    fn test_handler_receives_correct_payload() {
        let received_name = Arc::new(std::sync::Mutex::new(String::new()));
        let received_clone = Arc::clone(&received_name);

        let mut router = MessageRouter::new();
        router.register(
            MessageTag::LoginRequest,
            move |msg: Message, _ctx: &HandlerContext| {
                if let Message::LoginRequest(req) = msg {
                    *received_clone.lock().unwrap() = req.player_name.clone();
                }
            },
        );

        let msg = Message::LoginRequest(LoginRequest {
            player_name: "TestPlayer".to_string(),
        });
        let ctx = dummy_ctx();
        router.route(msg, &ctx);

        assert_eq!(*received_name.lock().unwrap(), "TestPlayer");
    }

    #[test]
    fn test_routing_is_type_safe() {
        let ping_count = Arc::new(AtomicU32::new(0));
        let ping_clone = Arc::clone(&ping_count);

        let mut router = MessageRouter::new();
        router.register(
            MessageTag::Ping,
            move |_msg: Message, _ctx: &HandlerContext| {
                ping_clone.fetch_add(1, Ordering::SeqCst);
            },
        );

        let ctx = dummy_ctx();

        // Send a Pong — should NOT trigger the Ping handler.
        let pong = Message::Pong(Pong {
            timestamp_ms: 0,
            sequence: 0,
        });
        router.route(pong, &ctx);
        assert_eq!(ping_count.load(Ordering::SeqCst), 0);

        // Send a Ping — should trigger the Ping handler.
        let ping = Message::Ping(Ping {
            timestamp_ms: 0,
            sequence: 0,
        });
        router.route(ping, &ctx);
        assert_eq!(ping_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_multiple_handlers_for_different_types() {
        let login_hit = Arc::new(AtomicBool::new(false));
        let ping_hit = Arc::new(AtomicBool::new(false));
        let position_hit = Arc::new(AtomicBool::new(false));

        let mut router = MessageRouter::new();
        let lh = Arc::clone(&login_hit);
        router.register(
            MessageTag::LoginRequest,
            move |_: Message, _: &HandlerContext| {
                lh.store(true, Ordering::SeqCst);
            },
        );
        let ph = Arc::clone(&ping_hit);
        router.register(MessageTag::Ping, move |_: Message, _: &HandlerContext| {
            ph.store(true, Ordering::SeqCst);
        });
        let posh = Arc::clone(&position_hit);
        router.register(
            MessageTag::PlayerPosition,
            move |_: Message, _: &HandlerContext| {
                posh.store(true, Ordering::SeqCst);
            },
        );

        let ctx = dummy_ctx();

        router.route(
            Message::LoginRequest(LoginRequest {
                player_name: "A".into(),
            }),
            &ctx,
        );
        router.route(
            Message::Ping(Ping {
                timestamp_ms: 0,
                sequence: 0,
            }),
            &ctx,
        );
        router.route(
            Message::PlayerPosition(PlayerPosition {
                player_id: 1,
                pos_x_high: 0,
                pos_x_low: 0,
                pos_y_high: 0,
                pos_y_low: 0,
                pos_z_high: 0,
                pos_z_low: 0,
            }),
            &ctx,
        );

        assert!(login_hit.load(Ordering::SeqCst));
        assert!(ping_hit.load(Ordering::SeqCst));
        assert!(position_hit.load(Ordering::SeqCst));
    }

    #[test]
    fn test_message_tag_extraction() {
        assert_eq!(
            Message::Ping(Ping {
                timestamp_ms: 0,
                sequence: 0,
            })
            .tag(),
            MessageTag::Ping
        );
        assert_eq!(
            Message::ChunkData(ChunkData {
                chunk_x: 0,
                chunk_y: 0,
                chunk_z: 0,
                face: 0,
                voxel_data: vec![],
            })
            .tag(),
            MessageTag::ChunkData
        );
        assert_eq!(
            Message::Logout(Logout {
                player_id: 0,
                reason: String::new(),
            })
            .tag(),
            MessageTag::Logout
        );
    }

    #[tokio::test]
    async fn test_message_channel_delivers_messages() {
        let (tx, mut rx) = message_channel(16);

        tx.send(IncomingMessage {
            connection_id: ConnectionId(5),
            message: Message::Ping(Ping {
                timestamp_ms: 100,
                sequence: 1,
            }),
        })
        .await
        .unwrap();

        let incoming = rx.recv().await.unwrap();
        assert_eq!(incoming.connection_id, ConnectionId(5));
        assert_eq!(incoming.message.tag(), MessageTag::Ping);
    }
}
