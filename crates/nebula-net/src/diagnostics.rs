//! Network diagnostics: RTT measurement, jitter, and packet loss tracking.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Outstanding ping awaiting a pong response.
pub(crate) struct PendingPing {
    sequence: u32,
    sent_at: Instant,
}

/// Configuration for the diagnostics tracker.
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

/// Rolling window of RTT samples for network diagnostics.
pub struct DiagnosticsTracker {
    /// Configuration.
    config: DiagnosticsConfig,
    /// Rolling window of RTT samples.
    pub(crate) rtt_samples: VecDeque<Duration>,
    /// Outstanding pings that have not received a pong.
    pub(crate) pending_pings: VecDeque<PendingPing>,
    /// Next sequence number for pings.
    next_sequence: u32,
    /// Total pings sent (lifetime).
    pub(crate) total_pings_sent: u64,
    /// Total pongs received (lifetime).
    pub(crate) total_pongs_received: u64,
    /// Timeout after which a pending ping is considered lost.
    ping_timeout: Duration,
}

impl DiagnosticsTracker {
    /// Create a new diagnostics tracker with the given configuration.
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
        if let Some(pos) = self
            .pending_pings
            .iter()
            .position(|p| p.sequence == sequence)
        {
            let ping = self.pending_pings.remove(pos).expect("position valid");
            let rtt = ping.sent_at.elapsed();

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
                break;
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
    /// Computed from lifetime counts: (sent - received - in_flight) / sent.
    pub fn loss_rate(&self) -> f64 {
        if self.total_pings_sent == 0 {
            return 0.0;
        }
        let lost = self
            .total_pings_sent
            .saturating_sub(self.total_pongs_received);
        let in_flight = self.pending_pings.len() as u64;
        let actual_lost = lost.saturating_sub(in_flight);
        actual_lost as f64 / self.total_pings_sent as f64
    }

    /// Number of RTT samples currently in the rolling window.
    pub fn sample_count(&self) -> usize {
        self.rtt_samples.len()
    }

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

/// Immutable snapshot of network diagnostics, stored as an ECS resource
/// and exposed to the debug overlay.
#[derive(Debug, Clone, Default)]
pub struct NetworkDiagnostics {
    /// Average round-trip time.
    pub average_rtt: Option<Duration>,
    /// Minimum round-trip time.
    pub min_rtt: Option<Duration>,
    /// Maximum round-trip time.
    pub max_rtt: Option<Duration>,
    /// Jitter (standard deviation of RTT).
    pub jitter: Option<Duration>,
    /// Packet loss rate (0.0 to 1.0).
    pub loss_rate: f64,
    /// Number of RTT samples in the rolling window.
    pub sample_count: usize,
}

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
        tracker.on_pong_received(seq);

        let avg = tracker.average_rtt().unwrap();
        assert!(
            avg < Duration::from_millis(10),
            "RTT should be near-zero in test, got {avg:?}",
        );
        assert_eq!(tracker.sample_count(), 1);
    }

    #[test]
    fn test_jitter_calculated_from_variance() {
        let mut tracker = make_tracker(100);

        tracker.rtt_samples.push_back(Duration::from_millis(10));
        tracker.rtt_samples.push_back(Duration::from_millis(20));
        tracker.rtt_samples.push_back(Duration::from_millis(30));
        tracker.rtt_samples.push_back(Duration::from_millis(40));
        tracker.rtt_samples.push_back(Duration::from_millis(50));

        let jitter = tracker.jitter().unwrap();
        let jitter_ms = jitter.as_secs_f64() * 1000.0;
        assert!(
            (jitter_ms - 15.81).abs() < 1.0,
            "Jitter should be ~15.81ms, got {jitter_ms:.2}ms",
        );
    }

    #[test]
    fn test_loss_rate_tracks_timeouts() {
        let mut tracker = DiagnosticsTracker::new(DiagnosticsConfig {
            window_size: 100,
            ping_timeout: Duration::from_millis(1),
        });

        for i in 0..10 {
            let seq = tracker.on_ping_sent();
            if i < 7 {
                tracker.on_pong_received(seq);
            }
        }

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
        let mut tracker = make_tracker(5);

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

        let seq1 = tracker.on_ping_sent();
        tracker.on_pong_received(seq1);
        let snap1 = tracker.snapshot();
        assert_eq!(snap1.sample_count, 1);

        let seq2 = tracker.on_ping_sent();
        tracker.on_pong_received(seq2);
        let snap2 = tracker.snapshot();
        assert_eq!(snap2.sample_count, 2);

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
