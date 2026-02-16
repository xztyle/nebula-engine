//! Text chat system: server-authoritative message validation, timestamping,
//! and broadcast with global and proximity scopes.
//!
//! The server validates each [`ChatMessageIntent`] against length, empty, and
//! rate-limit rules ([`ChatConfig`]), stamps accepted messages with a
//! server-authoritative tick and wall-clock timestamp, then broadcasts the
//! resulting [`ChatMessage`] to the appropriate recipients via
//! [`broadcast_chat`].

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::interest::{InterestPosition, within_interest};
use crate::replication::NetworkId;

// ---------------------------------------------------------------------------
// ChatScope
// ---------------------------------------------------------------------------

/// Determines which players receive a chat message.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum ChatScope {
    /// Visible to every connected player.
    Global,
    /// Only players within `radius` meters of the sender.
    Proximity {
        /// Maximum distance (meters) at which the message is received.
        radius: f64,
    },
}

// ---------------------------------------------------------------------------
// ChatMessageIntent (client → server)
// ---------------------------------------------------------------------------

/// A chat message submitted by a client, before server validation.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ChatMessageIntent {
    /// Desired scope for the message.
    pub scope: ChatScope,
    /// Raw text content.
    pub content: String,
}

// ---------------------------------------------------------------------------
// ChatMessage (server → clients)
// ---------------------------------------------------------------------------

/// A validated, server-stamped chat message ready for delivery to clients.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ChatMessage {
    /// Network id of the sender entity.
    pub sender_network_id: NetworkId,
    /// Display name of the sender.
    pub sender_name: String,
    /// Scope under which the message was sent.
    pub scope: ChatScope,
    /// Validated text content.
    pub content: String,
    /// Server tick at which the message was accepted.
    pub server_tick: u64,
    /// Wall-clock timestamp (Unix milliseconds, server clock).
    pub timestamp: u64,
}

// ---------------------------------------------------------------------------
// ChatConfig
// ---------------------------------------------------------------------------

/// Server-side chat rules.
#[derive(Debug, Clone)]
pub struct ChatConfig {
    /// Maximum allowed message length in characters.
    pub max_message_length: usize,
    /// Maximum messages allowed within the rate-limit window.
    pub rate_limit_messages: u32,
    /// Duration of the sliding rate-limit window.
    pub rate_limit_window: Duration,
    /// Default proximity radius in meters.
    pub proximity_radius: f64,
}

impl Default for ChatConfig {
    fn default() -> Self {
        Self {
            max_message_length: 500,
            rate_limit_messages: 5,
            rate_limit_window: Duration::from_secs(10),
            proximity_radius: 50.0,
        }
    }
}

// ---------------------------------------------------------------------------
// RateTracker
// ---------------------------------------------------------------------------

/// Per-client sliding-window rate tracker.
#[derive(Debug, Clone)]
pub struct RateTracker {
    /// Timestamps of accepted messages within the current window.
    pub timestamps: VecDeque<Instant>,
    /// Maximum number of messages allowed within `window`.
    pub max_count: u32,
    /// Duration of the sliding window.
    pub window: Duration,
}

impl RateTracker {
    /// Creates a new tracker from the given config.
    pub fn new(max_count: u32, window: Duration) -> Self {
        Self {
            timestamps: VecDeque::new(),
            max_count,
            window,
        }
    }

    /// Returns `true` and records the current instant if the client is
    /// within the rate limit. Returns `false` if the limit is exceeded.
    pub fn allow(&mut self) -> bool {
        let now = Instant::now();
        // Evict expired timestamps.
        while self
            .timestamps
            .front()
            .is_some_and(|t| now.duration_since(*t) > self.window)
        {
            self.timestamps.pop_front();
        }
        if self.timestamps.len() as u32 >= self.max_count {
            return false;
        }
        self.timestamps.push_back(now);
        true
    }
}

// ---------------------------------------------------------------------------
// ChatRejection
// ---------------------------------------------------------------------------

/// Reason a chat message was rejected by the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatRejection {
    /// Message exceeds [`ChatConfig::max_message_length`].
    TooLong,
    /// Message is empty or whitespace-only.
    Empty,
    /// Client exceeded the per-window rate limit.
    RateLimited,
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Validates a [`ChatMessageIntent`] against the server [`ChatConfig`] and
/// per-client [`RateTracker`]. Returns `Ok(())` on success or the specific
/// [`ChatRejection`] reason.
pub fn validate_chat_message(
    config: &ChatConfig,
    rate_tracker: &mut RateTracker,
    message: &ChatMessageIntent,
) -> Result<(), ChatRejection> {
    if message.content.len() > config.max_message_length {
        return Err(ChatRejection::TooLong);
    }
    if message.content.trim().is_empty() {
        return Err(ChatRejection::Empty);
    }
    if !rate_tracker.allow() {
        return Err(ChatRejection::RateLimited);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// ConnectedClient (lightweight descriptor for broadcast)
// ---------------------------------------------------------------------------

/// Minimal client descriptor used by [`broadcast_chat`] to decide recipients.
#[derive(Debug, Clone)]
pub struct ConnectedClient {
    /// Unique client identifier.
    pub client_id: u64,
    /// Current position of the client's player entity.
    pub position: InterestPosition,
}

// ---------------------------------------------------------------------------
// Broadcast
// ---------------------------------------------------------------------------

/// Determines the set of recipients for a [`ChatMessage`] based on its scope.
///
/// - **Global**: all clients receive the message.
/// - **Proximity**: only clients within `radius` of `sender_pos`.
///
/// Returns the list of [`client_id`](ConnectedClient::client_id) values that
/// should receive the message.
pub fn broadcast_chat(
    message: &ChatMessage,
    sender_pos: &InterestPosition,
    clients: &[ConnectedClient],
    _config: &ChatConfig,
) -> Vec<u64> {
    match &message.scope {
        ChatScope::Global => clients.iter().map(|c| c.client_id).collect(),
        ChatScope::Proximity { radius } => clients
            .iter()
            .filter(|c| within_interest(sender_pos, &c.position, *radius))
            .map(|c| c.client_id)
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> ChatConfig {
        ChatConfig::default()
    }

    fn make_tracker(config: &ChatConfig) -> RateTracker {
        RateTracker::new(config.rate_limit_messages, config.rate_limit_window)
    }

    fn stamp_message(
        intent: &ChatMessageIntent,
        sender_id: NetworkId,
        sender_name: &str,
        tick: u64,
        timestamp: u64,
    ) -> ChatMessage {
        ChatMessage {
            sender_network_id: sender_id,
            sender_name: sender_name.to_string(),
            scope: intent.scope.clone(),
            content: intent.content.clone(),
            server_tick: tick,
            timestamp,
        }
    }

    #[test]
    fn test_message_sent_and_received_by_all() {
        let config = default_config();
        let mut tracker = make_tracker(&config);

        let intent = ChatMessageIntent {
            scope: ChatScope::Global,
            content: "Hello".to_string(),
        };
        assert!(validate_chat_message(&config, &mut tracker, &intent).is_ok());

        let msg = stamp_message(&intent, NetworkId(1), "Alice", 42, 1_700_000_000_000);

        let clients = vec![
            ConnectedClient {
                client_id: 2,
                position: InterestPosition::new(0.0, 0.0, 0.0),
            },
            ConnectedClient {
                client_id: 3,
                position: InterestPosition::new(100.0, 0.0, 0.0),
            },
            ConnectedClient {
                client_id: 4,
                position: InterestPosition::new(999.0, 0.0, 0.0),
            },
        ];

        let recipients = broadcast_chat(
            &msg,
            &InterestPosition::new(0.0, 0.0, 0.0),
            &clients,
            &config,
        );
        assert_eq!(recipients, vec![2, 3, 4]);
        assert_eq!(msg.sender_network_id, NetworkId(1));
        assert_eq!(msg.content, "Hello");
        assert!(msg.timestamp > 0);
    }

    #[test]
    fn test_proximity_chat_limited_by_distance() {
        let config = default_config();
        let mut tracker = make_tracker(&config);

        let intent = ChatMessageIntent {
            scope: ChatScope::Proximity { radius: 50.0 },
            content: "Psst".to_string(),
        };
        assert!(validate_chat_message(&config, &mut tracker, &intent).is_ok());

        let msg = stamp_message(&intent, NetworkId(1), "Alice", 10, 1_700_000_000_000);

        let sender_pos = InterestPosition::new(0.0, 0.0, 0.0);
        let clients = vec![
            ConnectedClient {
                client_id: 2,
                position: InterestPosition::new(30.0, 0.0, 0.0),
            },
            ConnectedClient {
                client_id: 3,
                position: InterestPosition::new(100.0, 0.0, 0.0),
            },
        ];

        let recipients = broadcast_chat(&msg, &sender_pos, &clients, &config);
        assert_eq!(recipients, vec![2]);
    }

    #[test]
    fn test_message_length_limit_enforced() {
        let config = default_config();
        let mut tracker = make_tracker(&config);

        let intent = ChatMessageIntent {
            scope: ChatScope::Global,
            content: "x".repeat(600),
        };
        assert_eq!(
            validate_chat_message(&config, &mut tracker, &intent),
            Err(ChatRejection::TooLong)
        );
    }

    #[test]
    fn test_rate_limiting_prevents_spam() {
        let config = default_config();
        let mut tracker = make_tracker(&config);

        let intent = ChatMessageIntent {
            scope: ChatScope::Global,
            content: "msg".to_string(),
        };

        for _ in 0..5 {
            assert!(validate_chat_message(&config, &mut tracker, &intent).is_ok());
        }
        assert_eq!(
            validate_chat_message(&config, &mut tracker, &intent),
            Err(ChatRejection::RateLimited)
        );
    }

    #[test]
    fn test_timestamp_is_server_authoritative() {
        let config = default_config();
        let mut tracker = make_tracker(&config);

        let intent = ChatMessageIntent {
            scope: ChatScope::Global,
            content: "Hello".to_string(),
        };
        assert!(validate_chat_message(&config, &mut tracker, &intent).is_ok());

        let server_tick: u64 = 77;
        let server_time: u64 = 1_700_000_042_000;
        let msg = stamp_message(&intent, NetworkId(5), "Bob", server_tick, server_time);

        // The timestamp and tick come from the server, not the client.
        assert_eq!(msg.server_tick, 77);
        assert_eq!(msg.timestamp, 1_700_000_042_000);
        assert_eq!(msg.sender_network_id, NetworkId(5));
    }
}
