//! Bandwidth budgeting: per-client bandwidth enforcement, priority-based message
//! scheduling, adaptive rate reduction, and bandwidth statistics.

use std::collections::VecDeque;

/// Unique identifier for a connected client.
pub type ClientId = u64;

/// Configures the maximum bandwidth allowed per client.
#[derive(Debug, Clone)]
pub struct BandwidthConfig {
    /// Maximum bytes the server will send per second (default: 125,000 = 1 Mbps).
    pub max_bytes_per_second: usize,
    /// Server tick rate in Hz.
    pub tick_rate: u32,
}

impl Default for BandwidthConfig {
    fn default() -> Self {
        Self {
            max_bytes_per_second: 125_000,
            tick_rate: 60,
        }
    }
}

impl BandwidthConfig {
    /// Bytes the server may send to this client per tick.
    pub fn bytes_per_tick(&self) -> usize {
        self.max_bytes_per_second / self.tick_rate as usize
    }
}

/// Tracks how much bandwidth a single client has consumed in the current tick
/// and maintains a rolling history for averaging.
#[derive(Debug)]
pub struct ClientBandwidthTracker {
    /// Which client this tracker belongs to.
    pub client_id: ClientId,
    /// Budget configuration.
    pub config: BandwidthConfig,
    /// Bytes already sent during the current tick.
    pub bytes_sent_this_tick: usize,
    /// Per-tick send history (most recent at the back).
    pub bytes_sent_history: VecDeque<usize>,
    /// Maximum entries retained in the history ring (default: 600 = 10 s at 60 Hz).
    pub max_history: usize,
}

impl ClientBandwidthTracker {
    /// Create a new tracker for `client_id` with the given config.
    pub fn new(client_id: ClientId, config: BandwidthConfig) -> Self {
        Self {
            client_id,
            config,
            bytes_sent_this_tick: 0,
            bytes_sent_history: VecDeque::new(),
            max_history: 600,
        }
    }

    /// How many bytes remain in this tick's budget.
    pub fn remaining_budget(&self) -> usize {
        self.config
            .bytes_per_tick()
            .saturating_sub(self.bytes_sent_this_tick)
    }

    /// Record that `bytes` were sent to this client.
    pub fn consume(&mut self, bytes: usize) {
        self.bytes_sent_this_tick += bytes;
    }

    /// Finish the current tick: archive usage and reset the counter.
    pub fn end_tick(&mut self) {
        self.bytes_sent_history.push_back(self.bytes_sent_this_tick);
        if self.bytes_sent_history.len() > self.max_history {
            self.bytes_sent_history.pop_front();
        }
        self.bytes_sent_this_tick = 0;
    }

    /// Arithmetic mean of bytes sent per tick over the recorded history.
    pub fn average_usage(&self) -> f64 {
        if self.bytes_sent_history.is_empty() {
            return 0.0;
        }
        let sum: usize = self.bytes_sent_history.iter().sum();
        sum as f64 / self.bytes_sent_history.len() as f64
    }
}

// ---------------------------------------------------------------------------
// Message priority
// ---------------------------------------------------------------------------

/// Priority levels for outgoing messages (lower numeric value = higher priority).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MessagePriority {
    /// The client's own authoritative position – never deferred.
    PlayerState = 0,
    /// Entities within the client's interest area.
    NearbyEntities = 1,
    /// Real-time block changes.
    VoxelEdits = 2,
    /// Streamed terrain data.
    ChunkData = 3,
    /// Text communication.
    Chat = 4,
    /// Stats, debug info, and other non-critical data.
    Metadata = 5,
}

/// A message tagged with a priority and its serialised payload.
#[derive(Debug, Clone)]
pub struct PrioritizedMessage {
    /// Scheduling priority.
    pub priority: MessagePriority,
    /// Serialised payload bytes.
    pub data: Vec<u8>,
    /// Cached byte length of `data`.
    pub size: usize,
}

// ---------------------------------------------------------------------------
// Tick send loop
// ---------------------------------------------------------------------------

/// A minimal send-side abstraction so the budgeting logic can be tested
/// without a real TCP connection.
pub trait MessageSender {
    /// Send raw bytes to the client.
    fn send(&mut self, data: &[u8]);
}

/// Process the outgoing queue for one client during a single tick.
///
/// Messages are sent in priority order until the budget is exhausted.
/// Returns any messages that could not be sent (deferred to the next tick).
pub fn send_tick_messages(
    tracker: &mut ClientBandwidthTracker,
    queue: &mut Vec<PrioritizedMessage>,
    sender: &mut dyn MessageSender,
) -> Vec<PrioritizedMessage> {
    queue.sort_by_key(|m| m.priority);

    let mut deferred = Vec::new();

    for message in queue.drain(..) {
        if tracker.remaining_budget() >= message.size {
            tracker.consume(message.size);
            sender.send(&message.data);
        } else {
            deferred.push(message);
        }
    }

    tracker.end_tick();
    deferred
}

// ---------------------------------------------------------------------------
// Adaptive rate reduction
// ---------------------------------------------------------------------------

/// Adapts the entity-update send interval based on measured RTT.
#[derive(Debug, Clone)]
pub struct AdaptiveRate {
    /// Send entity updates every N ticks (1 = every tick, max 4).
    pub entity_update_interval: u32,
    /// RTT (ms) above which rate reduction begins.
    pub rtt_threshold_ms: u64,
}

impl Default for AdaptiveRate {
    fn default() -> Self {
        Self {
            entity_update_interval: 1,
            rtt_threshold_ms: 150,
        }
    }
}

impl AdaptiveRate {
    /// Re-evaluate the send interval based on the latest RTT sample.
    pub fn adjust(&mut self, rtt_ms: u64) {
        if rtt_ms > self.rtt_threshold_ms * 2 {
            self.entity_update_interval = 4;
        } else if rtt_ms > self.rtt_threshold_ms {
            self.entity_update_interval = 2;
        } else {
            self.entity_update_interval = 1;
        }
    }

    /// Whether an entity update should be emitted on the given tick.
    pub fn should_send_entity_update(&self, tick: u64) -> bool {
        tick.is_multiple_of(self.entity_update_interval as u64)
    }
}

// ---------------------------------------------------------------------------
// Bandwidth statistics
// ---------------------------------------------------------------------------

/// Summary statistics for a single client's bandwidth usage.
#[derive(Debug, Clone)]
pub struct BandwidthStats {
    /// Client this snapshot describes.
    pub client_id: ClientId,
    /// Bytes sent during the most recent tick, scaled to bytes-per-second.
    pub current_bps: usize,
    /// Peak bytes-per-second observed across the history window.
    pub peak_bps: usize,
    /// Average bytes-per-second across the history window.
    pub average_bps: f64,
    /// Messages deferred during the most recent tick.
    pub messages_deferred_this_tick: usize,
    /// Current adaptive entity-update interval.
    pub adaptive_interval: u32,
}

impl BandwidthStats {
    /// Build stats from a tracker and adaptive rate state, plus the deferred
    /// count from the most recent `send_tick_messages` call.
    pub fn from_tracker(
        tracker: &ClientBandwidthTracker,
        adaptive: &AdaptiveRate,
        deferred: usize,
    ) -> Self {
        let tick_rate = tracker.config.tick_rate as usize;
        let last_tick_bytes = tracker.bytes_sent_history.back().copied().unwrap_or(0);
        let peak_tick = tracker
            .bytes_sent_history
            .iter()
            .copied()
            .max()
            .unwrap_or(0);
        Self {
            client_id: tracker.client_id,
            current_bps: last_tick_bytes * tick_rate,
            peak_bps: peak_tick * tick_rate,
            average_bps: tracker.average_usage() * tick_rate as f64,
            messages_deferred_this_tick: deferred,
            adaptive_interval: adaptive.entity_update_interval,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Collects sent bytes for verification.
    struct MockSender {
        total_bytes: usize,
    }

    impl MockSender {
        fn new() -> Self {
            Self { total_bytes: 0 }
        }
    }

    impl MessageSender for MockSender {
        fn send(&mut self, data: &[u8]) {
            self.total_bytes += data.len();
        }
    }

    fn make_msg(priority: MessagePriority, size: usize) -> PrioritizedMessage {
        PrioritizedMessage {
            priority,
            data: vec![0u8; size],
            size,
        }
    }

    #[test]
    fn test_bandwidth_stays_within_budget() {
        let config = BandwidthConfig {
            max_bytes_per_second: 10_000 * 60,
            tick_rate: 60,
        };
        let mut tracker = ClientBandwidthTracker::new(1, config);
        let mut queue: Vec<PrioritizedMessage> = (0..20)
            .map(|_| make_msg(MessagePriority::ChunkData, 1_000))
            .collect();
        let mut sender = MockSender::new();

        let deferred = send_tick_messages(&mut tracker, &mut queue, &mut sender);

        assert_eq!(sender.total_bytes, 10_000);
        assert_eq!(deferred.len(), 10);
    }

    #[test]
    fn test_high_priority_messages_always_sent() {
        let config = BandwidthConfig {
            max_bytes_per_second: 5_000 * 60,
            tick_rate: 60,
        };
        let mut tracker = ClientBandwidthTracker::new(1, config);
        let mut queue = vec![make_msg(MessagePriority::PlayerState, 1_000)];
        for _ in 0..5 {
            queue.push(make_msg(MessagePriority::ChunkData, 1_000));
        }
        let mut sender = MockSender::new();

        let deferred = send_tick_messages(&mut tracker, &mut queue, &mut sender);

        // PlayerState (1000) + 4 ChunkData (4000) = 5000 = budget
        assert_eq!(sender.total_bytes, 5_000);
        assert_eq!(deferred.len(), 1);
        assert_eq!(deferred[0].priority, MessagePriority::ChunkData);
    }

    #[test]
    fn test_low_priority_deferred_when_budget_exceeded() {
        let config = BandwidthConfig {
            max_bytes_per_second: 3_000 * 60,
            tick_rate: 60,
        };
        let mut tracker = ClientBandwidthTracker::new(1, config);
        let mut queue: Vec<PrioritizedMessage> = (0..3)
            .map(|_| make_msg(MessagePriority::NearbyEntities, 1_000))
            .collect();
        queue.push(make_msg(MessagePriority::Chat, 500));
        queue.push(make_msg(MessagePriority::Chat, 500));
        let mut sender = MockSender::new();

        let deferred = send_tick_messages(&mut tracker, &mut queue, &mut sender);

        assert_eq!(sender.total_bytes, 3_000);
        assert_eq!(deferred.len(), 2);
        assert!(deferred.iter().all(|m| m.priority == MessagePriority::Chat));
    }

    #[test]
    fn test_per_client_tracking_is_accurate() {
        let config = BandwidthConfig {
            max_bytes_per_second: 100_000 * 60,
            tick_rate: 60,
        };
        let mut tracker = ClientBandwidthTracker::new(1, config);
        let expected: Vec<usize> = (1..=10).map(|i| i * 100).collect();

        for &bytes in &expected {
            tracker.consume(bytes);
            tracker.end_tick();
        }

        assert_eq!(tracker.bytes_sent_history.len(), 10);
        let history: Vec<usize> = tracker.bytes_sent_history.iter().copied().collect();
        assert_eq!(history, expected);

        let mean = expected.iter().sum::<usize>() as f64 / 10.0;
        assert!((tracker.average_usage() - mean).abs() < f64::EPSILON);
    }

    #[test]
    fn test_adaptive_rate_reduction_works() {
        let mut rate = AdaptiveRate::default();
        assert_eq!(rate.rtt_threshold_ms, 150);

        rate.adjust(100);
        assert_eq!(rate.entity_update_interval, 1);

        rate.adjust(200);
        assert_eq!(rate.entity_update_interval, 2);

        rate.adjust(350);
        assert_eq!(rate.entity_update_interval, 4);

        rate.adjust(80);
        assert_eq!(rate.entity_update_interval, 1);
    }
}
