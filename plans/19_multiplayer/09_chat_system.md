# Chat System

## Problem

Players in a multiplayer game need to communicate with each other via text chat. The system must support both global messages (visible to all players) and proximity-based messages (only audible to nearby players). Without server-side validation, the chat system would be vulnerable to spam, oversized messages, and abuse. All chat messages must be server-authoritative to ensure consistent timestamping and delivery.

## Solution

### Chat Message Types

The system supports two chat scopes:

```rust
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum ChatScope {
    Global,
    Proximity { radius: f64 }, // default: 50.0 meters
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ChatMessageIntent {
    pub scope: ChatScope,
    pub content: String,
}
```

### Client-to-Server Flow

The client sends a `ChatMessageIntent` to the server. The client never renders its own message locally before server confirmation — the server is authoritative over message delivery and timestamps.

### Server Validation

The server validates each incoming chat message against multiple rules:

```rust
pub struct ChatConfig {
    pub max_message_length: usize,     // default: 500 characters
    pub rate_limit_messages: u32,      // default: 5 messages
    pub rate_limit_window: Duration,   // default: 10 seconds
    pub proximity_radius: f64,         // default: 50.0 meters
}

pub fn validate_chat_message(
    config: &ChatConfig,
    rate_tracker: &mut RateTracker,
    message: &ChatMessageIntent,
) -> Result<(), ChatRejection> {
    // Length check
    if message.content.len() > config.max_message_length {
        return Err(ChatRejection::TooLong);
    }

    // Empty check
    if message.content.trim().is_empty() {
        return Err(ChatRejection::Empty);
    }

    // Rate limit
    if !rate_tracker.allow() {
        return Err(ChatRejection::RateLimited);
    }

    Ok(())
}
```

### Rate Tracking

Per-client rate tracking uses a sliding window:

```rust
pub struct RateTracker {
    pub timestamps: VecDeque<Instant>,
    pub max_count: u32,
    pub window: Duration,
}

impl RateTracker {
    pub fn allow(&mut self) -> bool {
        let now = Instant::now();
        // Remove expired timestamps
        while self.timestamps.front().map_or(false, |t| now.duration_since(*t) > self.window) {
            self.timestamps.pop_front();
        }
        if self.timestamps.len() as u32 >= self.max_count {
            return false;
        }
        self.timestamps.push_back(now);
        true
    }
}
```

### Server-Authoritative Timestamp

The server stamps each validated message with the server's current tick and wall-clock time. This ensures all clients see consistent ordering regardless of their local clocks:

```rust
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ChatMessage {
    pub sender_network_id: NetworkId,
    pub sender_name: String,
    pub scope: ChatScope,
    pub content: String,
    pub server_tick: u64,
    pub timestamp: u64, // Unix millis, server clock
}
```

### Broadcast Logic

- **Global**: The server sends the `ChatMessage` to all connected clients.
- **Proximity**: The server computes the sender's position and sends the message only to clients whose player entities are within the proximity radius. This uses the 128-bit coordinate distance calculation.

```rust
pub fn broadcast_chat(
    message: &ChatMessage,
    sender_pos: &Coord128,
    clients: &[ConnectedClient],
    config: &ChatConfig,
) -> Vec<ClientId> {
    match &message.scope {
        ChatScope::Global => {
            clients.iter().map(|c| c.client_id).collect()
        }
        ChatScope::Proximity { radius } => {
            clients.iter()
                .filter(|c| {
                    let dist = sender_pos.distance_to(&c.position);
                    dist <= *radius
                })
                .map(|c| c.client_id)
                .collect()
        }
    }
}
```

### Client Display

The client receives `ChatMessage` objects and passes them to the chat overlay UI (Epic 22). Messages are displayed with the sender name, content, and a formatted timestamp. The chat system produces data; the UI system consumes it.

## Outcome

- `nebula_multiplayer::chat` module containing `ChatScope`, `ChatMessageIntent`, `ChatMessage`, `ChatConfig`, `RateTracker`, `ChatRejection`, `validate_chat_message`, and `broadcast_chat`.
- Server-validated text chat with global and proximity scopes.
- Per-client rate limiting with sliding window.
- Server-authoritative timestamps for consistent message ordering.
- Integration point for the chat overlay UI in Epic 22.

## Demo Integration

**Demo crate:** `nebula-demo`

Pressing Enter opens a chat input field. Typing a message and pressing Enter sends it. All connected players see the message in a text overlay at the bottom of the screen.

## Crates & Dependencies

| Crate       | Version | Purpose                                        |
| ----------- | ------- | ---------------------------------------------- |
| `tokio`     | 1.49    | Async TCP for message delivery, time tracking   |
| `serde`     | 1.0     | Serialization of chat messages                  |
| `postcard`  | 1.1     | Binary wire format for chat messages            |
| `bevy_ecs`  | 0.18    | ECS queries for player positions (proximity)    |

## Unit Tests

### `test_message_sent_and_received_by_all`
Client A sends a `Global` chat message "Hello". Assert all connected clients (B, C, D) receive a `ChatMessage` with sender set to A's `NetworkId`, content "Hello", and a valid server timestamp.

### `test_proximity_chat_limited_by_distance`
Client A sends a `Proximity` message with radius 50 m. Client B is 30 m away, Client C is 100 m away. Assert Client B receives the message and Client C does not.

### `test_message_length_limit_enforced`
Client sends a message with 600 characters (limit is 500). Assert the server returns `ChatRejection::TooLong` and no `ChatMessage` is broadcast.

### `test_rate_limiting_prevents_spam`
Client sends 6 messages within 10 seconds (limit is 5 per 10s). Assert the first 5 are delivered successfully and the 6th returns `ChatRejection::RateLimited`.

### `test_timestamp_is_server_authoritative`
Client A sends a message. Assert the `ChatMessage` received by Client B has a `timestamp` value set by the server (not the client) and a `server_tick` matching the server's current tick at the time of processing.
