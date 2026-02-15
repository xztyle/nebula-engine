# Network Diagnostics

## Problem

Knowing how many bytes flow per second (story 09) is not enough to diagnose connection quality. The Nebula Engine also needs latency metrics: round-trip time (RTT), jitter (variance in RTT), and packet loss rate (how often expected responses never arrive). These metrics are critical for two purposes. First, adaptive quality: when RTT is high or loss is significant, the server should reduce the entity update rate for that client, send lower-LOD chunks, or skip cosmetic updates — without diagnostics, the engine has no signal to trigger these adaptations. Second, player-facing information: the debug overlay (Epic 28) should display ping, jitter, and loss so players can diagnose their own connection issues. RTT is measured via the existing ping/pong heartbeat mechanism (story 02) by including timestamps in each ping and computing the round-trip on pong receipt. Loss rate is computed from the ratio of unacknowledged pings to total pings sent.

## Solution

### RTT measurement

Each `Ping` message (story 04) carries a `sequence` number and a `timestamp_ms` (milliseconds since an arbitrary epoch, typically `Instant::now()` at connection start, converted to ms). When the corresponding `Pong` arrives, the RTT is: `now_ms - ping_timestamp_ms`. The sequence number matches pings to pongs.

```rust
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Outstanding ping awaiting a pong response.
struct PendingPing {
    sequence: u32,
    sent_at: Instant,
}

/// Rolling window of RTT samples for diagnostics.
pub struct DiagnosticsTracker {
    /// Configuration
    config: DiagnosticsConfig,
    /// Rolling window of RTT samples.
    rtt_samples: VecDeque<Duration>,
    /// Outstanding pings that have not received a pong.
    pending_pings: VecDeque<PendingPing>,
    /// Next sequence number for pings.
    next_sequence: u32,
    /// Total pings sent (lifetime).
    total_pings_sent: u64,
    /// Total pongs received (lifetime).
    total_pongs_received: u64,
    /// Timeout after which a pending ping is considered lost.
    ping_timeout: Duration,
}

pub struct DiagnosticsConfig {
    /// Number of RTT samples to keep in the rolling window. Default: 100.
    pub window_size: usize,
    /// Timeout for pending pings. Default: 10s.
    pub ping_timeout: Duration,
}

impl Default for DiagnosticsConfig {
    fn default() -> Self {
        Self {
            window_size: 100,
            ping_timeout: Duration::from_secs(10),
        }
    }
}

impl DiagnosticsTracker {
    pub fn new(config: DiagnosticsConfig) -> Self {
        Self {
            rtt_samples: VecDeque::with_capacity(config.window_size),
            pending_pings: VecDeque::new(),
            next_sequence: 0,
            total_pings_sent: 0,
            total_pongs_received: 0,
            ping_timeout: config.ping_timeout,
            config,
        }
    }

    /// Record that a ping was sent. Returns the sequence number to include
    /// in the Ping message.
    pub fn on_ping_sent(&mut self) -> u32 {
        let seq = self.next_sequence;
        self.next_sequence = self.next_sequence.wrapping_add(1);
        self.pending_pings.push_back(PendingPing {
            sequence: seq,
            sent_at: Instant::now(),
        });
        self.total_pings_sent += 1;
        seq
    }

    /// Record that a pong was received with the given sequence number.
    /// Computes and stores the RTT sample.
    pub fn on_pong_received(&mut self, sequence: u32) {
        // Find and remove the matching pending ping
        if let Some(pos) = self
            .pending_pings
            .iter()
            .position(|p| p.sequence == sequence)
        {
            let ping = self.pending_pings.remove(pos).unwrap();
            let rtt = ping.sent_at.elapsed();

            // Add to rolling window
            if self.rtt_samples.len() >= self.config.window_size {
                self.rtt_samples.pop_front();
            }
            self.rtt_samples.push_back(rtt);
            self.total_pongs_received += 1;
        }
    }

    /// Expire pending pings that have timed out (considered lost).
    pub fn expire_pending(&mut self) {
        let timeout = self.ping_timeout;
        while let Some(front) = self.pending_pings.front() {
            if front.sent_at.elapsed() > timeout {
                self.pending_pings.pop_front();
            } else {
                break; // Remaining pings are newer
            }
        }
    }

    /// Average RTT over the rolling window.
    pub fn average_rtt(&self) -> Option<Duration> {
        if self.rtt_samples.is_empty() {
            return None;
        }
        let sum: Duration = self.rtt_samples.iter().sum();
        Some(sum / self.rtt_samples.len() as u32)
    }

    /// Minimum RTT in the rolling window.
    pub fn min_rtt(&self) -> Option<Duration> {
        self.rtt_samples.iter().min().copied()
    }

    /// Maximum RTT in the rolling window.
    pub fn max_rtt(&self) -> Option<Duration> {
        self.rtt_samples.iter().max().copied()
    }

    /// Jitter: standard deviation of RTT samples in the rolling window.
    pub fn jitter(&self) -> Option<Duration> {
        if self.rtt_samples.len() < 2 {
            return None;
        }

        let avg = self.average_rtt()?.as_secs_f64();
        let variance: f64 = self
            .rtt_samples
            .iter()
            .map(|s| {
                let diff = s.as_secs_f64() - avg;
                diff * diff
            })
            .sum::<f64>()
            / (self.rtt_samples.len() - 1) as f64;

        Some(Duration::from_secs_f64(variance.sqrt()))
    }

    /// Packet loss rate as a fraction (0.0 to 1.0).
    /// Computed from lifetime counts: (sent - received) / sent.
    pub fn loss_rate(&self) -> f64 {
        if self.total_pings_sent == 0 {
            return 0.0;
        }
        let lost = self.total_pings_sent.saturating_sub(self.total_pongs_received);
        // Subtract still-pending pings that haven't timed out yet
        let in_flight = self.pending_pings.len() as u64;
        let actual_lost = lost.saturating_sub(in_flight);
        actual_lost as f64 / self.total_pings_sent as f64
    }

    /// Number of RTT samples currently in the rolling window.
    pub fn sample_count(&self) -> usize {
        self.rtt_samples.len()
    }
}
```

### Diagnostics snapshot for ECS

```rust
/// Immutable snapshot of network diagnostics, stored as an ECS resource
/// and exposed to the debug overlay.
#[derive(Debug, Clone, Default)]
pub struct NetworkDiagnostics {
    pub average_rtt: Option<Duration>,
    pub min_rtt: Option<Duration>,
    pub max_rtt: Option<Duration>,
    pub jitter: Option<Duration>,
    pub loss_rate: f64,
    pub sample_count: usize,
}

impl DiagnosticsTracker {
    /// Produce an immutable snapshot of current diagnostics.
    pub fn snapshot(&self) -> NetworkDiagnostics {
        NetworkDiagnostics {
            average_rtt: self.average_rtt(),
            min_rtt: self.min_rtt(),
            max_rtt: self.max_rtt(),
            jitter: self.jitter(),
            loss_rate: self.loss_rate(),
            sample_count: self.sample_count(),
        }
    }
}
```

### Adaptive quality hook

The diagnostics feed into an adaptive quality system (future epic). The basic rule: if `average_rtt > 200ms` or `loss_rate > 5%`, reduce the entity update frequency for that client. This story only provides the data; the adaptation logic lives elsewhere.

## Outcome

A `diagnostics.rs` module in `crates/nebula_net/src/` exporting `DiagnosticsTracker`, `DiagnosticsConfig`, `NetworkDiagnostics`, and the associated methods. RTT, jitter, and packet loss are continuously measured from ping/pong exchanges and exposed as an ECS resource for the debug overlay and adaptive quality systems. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

A diagnostics overlay (toggle key) shows packet loss percentage, latency histogram, message type breakdown, and bytes per message type.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| (none) | — | Uses only `std::collections::VecDeque`, `std::time::Duration`, `std::time::Instant` |

No external crates required. All computation uses standard library types.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn make_tracker(window: usize) -> DiagnosticsTracker {
        DiagnosticsTracker::new(DiagnosticsConfig {
            window_size: window,
            ping_timeout: Duration::from_secs(10),
        })
    }

    #[test]
    fn test_rtt_measured_correctly() {
        let mut tracker = make_tracker(100);
        let seq = tracker.on_ping_sent();

        // Simulate a small delay (in real code the delay is real wall-clock time).
        // For unit tests we manipulate the pending_pings directly.
        // Here we just immediately record pong — RTT will be ~0ms.
        tracker.on_pong_received(seq);

        let avg = tracker.average_rtt().unwrap();
        // RTT should be very small (sub-millisecond) since we responded immediately.
        assert!(
            avg < Duration::from_millis(10),
            "RTT should be near-zero in test, got {:?}",
            avg
        );
        assert_eq!(tracker.sample_count(), 1);
    }

    #[test]
    fn test_jitter_calculated_from_variance() {
        let mut tracker = make_tracker(100);

        // Insert synthetic RTT samples directly to test jitter calculation.
        tracker.rtt_samples.push_back(Duration::from_millis(10));
        tracker.rtt_samples.push_back(Duration::from_millis(20));
        tracker.rtt_samples.push_back(Duration::from_millis(30));
        tracker.rtt_samples.push_back(Duration::from_millis(40));
        tracker.rtt_samples.push_back(Duration::from_millis(50));

        let jitter = tracker.jitter().unwrap();
        // Standard deviation of [10, 20, 30, 40, 50] ms = ~15.81 ms
        let jitter_ms = jitter.as_secs_f64() * 1000.0;
        assert!(
            (jitter_ms - 15.81).abs() < 1.0,
            "Jitter should be ~15.81ms, got {:.2}ms",
            jitter_ms
        );
    }

    #[test]
    fn test_loss_rate_tracks_timeouts() {
        let mut tracker = DiagnosticsTracker::new(DiagnosticsConfig {
            window_size: 100,
            ping_timeout: Duration::from_millis(1), // Very short timeout for testing
        });

        // Send 10 pings, respond to only 7
        for i in 0..10 {
            let seq = tracker.on_ping_sent();
            if i < 7 {
                tracker.on_pong_received(seq);
            }
        }

        // Expire the remaining 3 pending pings
        std::thread::sleep(Duration::from_millis(5));
        tracker.expire_pending();

        let loss = tracker.loss_rate();
        assert!(
            (loss - 0.3).abs() < 0.01,
            "Loss rate should be ~30%, got {:.1}%",
            loss * 100.0
        );
    }

    #[test]
    fn test_rolling_window_is_bounded() {
        let mut tracker = make_tracker(5); // Window of 5

        // Insert 10 samples — only last 5 should remain.
        for i in 0..10 {
            tracker.rtt_samples.push_back(Duration::from_millis(i * 10));
            if tracker.rtt_samples.len() > 5 {
                tracker.rtt_samples.pop_front();
            }
        }

        assert_eq!(
            tracker.sample_count(),
            5,
            "Window should be bounded to 5 samples"
        );
    }

    #[test]
    fn test_diagnostics_update_continuously() {
        let mut tracker = make_tracker(100);

        // First ping/pong
        let seq1 = tracker.on_ping_sent();
        tracker.on_pong_received(seq1);
        let snap1 = tracker.snapshot();
        assert_eq!(snap1.sample_count, 1);

        // Second ping/pong
        let seq2 = tracker.on_ping_sent();
        tracker.on_pong_received(seq2);
        let snap2 = tracker.snapshot();
        assert_eq!(snap2.sample_count, 2);

        // Diagnostics should reflect the updated state
        assert!(snap2.average_rtt.is_some());
    }

    #[test]
    fn test_min_max_rtt() {
        let mut tracker = make_tracker(100);
        tracker.rtt_samples.push_back(Duration::from_millis(5));
        tracker.rtt_samples.push_back(Duration::from_millis(15));
        tracker.rtt_samples.push_back(Duration::from_millis(10));

        assert_eq!(tracker.min_rtt(), Some(Duration::from_millis(5)));
        assert_eq!(tracker.max_rtt(), Some(Duration::from_millis(15)));
    }

    #[test]
    fn test_empty_tracker_returns_none() {
        let tracker = make_tracker(100);
        assert!(tracker.average_rtt().is_none());
        assert!(tracker.min_rtt().is_none());
        assert!(tracker.max_rtt().is_none());
        assert!(tracker.jitter().is_none());
        assert_eq!(tracker.loss_rate(), 0.0);
    }

    #[test]
    fn test_sequence_numbers_increment() {
        let mut tracker = make_tracker(100);
        let s1 = tracker.on_ping_sent();
        let s2 = tracker.on_ping_sent();
        let s3 = tracker.on_ping_sent();
        assert_eq!(s1, 0);
        assert_eq!(s2, 1);
        assert_eq!(s3, 2);
    }

    #[test]
    fn test_out_of_order_pong_handled() {
        let mut tracker = make_tracker(100);
        let s1 = tracker.on_ping_sent();
        let s2 = tracker.on_ping_sent();

        // Receive pong for s2 before s1
        tracker.on_pong_received(s2);
        tracker.on_pong_received(s1);

        assert_eq!(tracker.sample_count(), 2);
        assert_eq!(tracker.pending_pings.len(), 0);
    }

    #[test]
    fn test_snapshot_produces_consistent_data() {
        let mut tracker = make_tracker(100);
        tracker.rtt_samples.push_back(Duration::from_millis(20));
        tracker.rtt_samples.push_back(Duration::from_millis(30));
        tracker.total_pings_sent = 10;
        tracker.total_pongs_received = 10;

        let snap = tracker.snapshot();
        assert_eq!(snap.sample_count, 2);
        assert!(snap.average_rtt.is_some());
        assert_eq!(snap.loss_rate, 0.0);
    }
}
```
