# Message Routing

## Problem

When a deserialized `Message` arrives on either the server or the client, it must be dispatched to the correct handler system. The server receives `LoginRequest`, `PlayerPosition`, `PlayerAction`, and `Ping` messages — each must reach a different subsystem. The client receives `LoginResponse`, `ChunkData`, `EntityUpdate`, `Pong`, and `TimeSync` messages — each must update different parts of the game state. Without a routing layer, the network task would need direct knowledge of every game system, creating tight coupling between networking and game logic. The routing layer decouples these by acting as a dispatch table. Messages are passed from the network task (running on the tokio runtime) to the game thread (running the ECS) via channels, so the routing must also bridge the async/sync boundary.

## Solution

### Message tag

Each `Message` variant has a discriminant that serves as its type tag. A helper method extracts it:

```rust
/// Unique tag identifying a message type, used as the key for routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MessageTag {
    LoginRequest,
    LoginResponse,
    Logout,
    ChunkData,
    EntityUpdate,
    PlayerPosition,
    PlayerAction,
    Ping,
    Pong,
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
```

### Handler trait

A handler is any callable that processes a message along with the connection context:

```rust
use std::sync::Arc;

/// Context provided to every message handler.
pub struct HandlerContext {
    /// The connection that sent this message.
    pub connection_id: ConnectionId,
    /// Shared reference to the connection map for sending responses.
    pub connections: Arc<ConnectionMap>,
}

/// Trait for message handlers. Implemented as a boxed async function.
pub trait MessageHandler: Send + Sync {
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
```

### Message router

The `MessageRouter` maps `MessageTag` values to handler functions. Registration happens at startup; dispatch happens on every incoming message.

```rust
use std::collections::HashMap;

pub struct MessageRouter {
    handlers: HashMap<MessageTag, Box<dyn MessageHandler>>,
}

impl MessageRouter {
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
    /// Returns `true` if a handler was found, `false` if the message was dropped.
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
}
```

### Async-to-sync bridge via channels

The network task receives framed and deserialized messages on the tokio runtime. These must reach the game thread (which runs the ECS tick loop). A bounded `tokio::sync::mpsc` channel bridges this gap. The network task sends `(ConnectionId, Message)` tuples into the channel; the game thread drains the channel each tick and feeds each message into the `MessageRouter`.

```rust
use tokio::sync::mpsc;

/// Envelope carrying a message and its source connection.
pub struct IncomingMessage {
    pub connection_id: ConnectionId,
    pub message: Message,
}

/// Create a channel pair for passing messages from network tasks to the game thread.
pub fn message_channel(buffer: usize) -> (mpsc::Sender<IncomingMessage>, mpsc::Receiver<IncomingMessage>) {
    mpsc::channel(buffer)
}
```

On the game thread:

```rust
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
```

### Example: server-side registration

```rust
fn build_server_router() -> MessageRouter {
    let mut router = MessageRouter::new();
    router.register(MessageTag::LoginRequest, |msg, ctx| {
        if let Message::LoginRequest(req) = msg {
            // Process login via auth system (story 06)
        }
    });
    router.register(MessageTag::PlayerPosition, |msg, ctx| {
        if let Message::PlayerPosition(pos) = msg {
            // Update world state
        }
    });
    router.register(MessageTag::Ping, |msg, ctx| {
        if let Message::Ping(ping) = msg {
            // Respond with Pong
        }
    });
    router
}
```

## Outcome

A `routing.rs` module in `crates/nebula_net/src/` exporting `MessageTag`, `MessageRouter`, `MessageHandler`, `HandlerContext`, `IncomingMessage`, `message_channel`, and `process_incoming_messages`. Messages flow from the network task through a channel to the game thread, where they are dispatched to type-safe handlers via the router. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

Messages are routed by type to their respective handlers. The routing table is logged at startup showing which handler processes which message type.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `tokio` | `1.49` (features: `rt-multi-thread`, `macros`) | `mpsc` channel for async-to-sync message passing |
| `tracing` | `0.1` | Logging for unrouted messages |

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, atomic::{AtomicBool, AtomicU32, Ordering}};

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
        router.register(MessageTag::Ping, move |_msg, _ctx| {
            handled_clone.store(true, Ordering::SeqCst);
        });

        let msg = Message::Ping(Ping { timestamp_ms: 0, sequence: 0 });
        let ctx = dummy_ctx();
        router.route(msg, &ctx);

        assert!(handled.load(Ordering::SeqCst), "Ping handler should have been called");
    }

    #[test]
    fn test_unknown_message_type_dropped() {
        let router = MessageRouter::new(); // No handlers registered
        let msg = Message::Ping(Ping { timestamp_ms: 0, sequence: 0 });
        let ctx = dummy_ctx();
        let routed = router.route(msg, &ctx);
        assert!(!routed, "Message with no handler should return false");
    }

    #[test]
    fn test_handler_receives_correct_payload() {
        let received_name = Arc::new(std::sync::Mutex::new(String::new()));
        let received_clone = Arc::clone(&received_name);

        let mut router = MessageRouter::new();
        router.register(MessageTag::LoginRequest, move |msg, _ctx| {
            if let Message::LoginRequest(req) = msg {
                *received_clone.lock().unwrap() = req.player_name.clone();
            }
        });

        let msg = Message::LoginRequest(LoginRequest {
            player_name: "TestPlayer".to_string(),
        });
        let ctx = dummy_ctx();
        router.route(msg, &ctx);

        assert_eq!(*received_name.lock().unwrap(), "TestPlayer");
    }

    #[test]
    fn test_routing_is_type_safe() {
        // Register a handler for Ping only; sending a Pong should not trigger it.
        let ping_count = Arc::new(AtomicU32::new(0));
        let ping_clone = Arc::clone(&ping_count);

        let mut router = MessageRouter::new();
        router.register(MessageTag::Ping, move |_msg, _ctx| {
            ping_clone.fetch_add(1, Ordering::SeqCst);
        });

        let ctx = dummy_ctx();

        // Send a Pong — should NOT trigger the Ping handler.
        let pong = Message::Pong(Pong { timestamp_ms: 0, sequence: 0 });
        router.route(pong, &ctx);
        assert_eq!(ping_count.load(Ordering::SeqCst), 0);

        // Send a Ping — should trigger the Ping handler.
        let ping = Message::Ping(Ping { timestamp_ms: 0, sequence: 0 });
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
        router.register(MessageTag::LoginRequest, move |_, _| {
            lh.store(true, Ordering::SeqCst);
        });
        let ph = Arc::clone(&ping_hit);
        router.register(MessageTag::Ping, move |_, _| {
            ph.store(true, Ordering::SeqCst);
        });
        let posh = Arc::clone(&position_hit);
        router.register(MessageTag::PlayerPosition, move |_, _| {
            posh.store(true, Ordering::SeqCst);
        });

        let ctx = dummy_ctx();

        router.route(
            Message::LoginRequest(LoginRequest { player_name: "A".into() }),
            &ctx,
        );
        router.route(
            Message::Ping(Ping { timestamp_ms: 0, sequence: 0 }),
            &ctx,
        );
        router.route(
            Message::PlayerPosition(PlayerPosition {
                player_id: 1,
                pos_x_high: 0, pos_x_low: 0,
                pos_y_high: 0, pos_y_low: 0,
                pos_z_high: 0, pos_z_low: 0,
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
            Message::Ping(Ping { timestamp_ms: 0, sequence: 0 }).tag(),
            MessageTag::Ping
        );
        assert_eq!(
            Message::ChunkData(ChunkData {
                chunk_x: 0, chunk_y: 0, chunk_z: 0,
                face: 0, voxel_data: vec![],
            }).tag(),
            MessageTag::ChunkData
        );
        assert_eq!(
            Message::Logout(Logout { player_id: 0, reason: String::new() }).tag(),
            MessageTag::Logout
        );
    }

    #[tokio::test]
    async fn test_message_channel_delivers_messages() {
        let (tx, mut rx) = message_channel(16);

        tx.send(IncomingMessage {
            connection_id: ConnectionId(5),
            message: Message::Ping(Ping { timestamp_ms: 100, sequence: 1 }),
        }).await.unwrap();

        let incoming = rx.recv().await.unwrap();
        assert_eq!(incoming.connection_id, ConnectionId(5));
        assert_eq!(incoming.message.tag(), MessageTag::Ping);
    }
}
```
