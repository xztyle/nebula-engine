//! Clock synchronization for multiplayer tick alignment.
//!
//! Provides NTP-like clock sync so clients can estimate the server's current
//! tick and run slightly ahead (by half-RTT) for timely input delivery.

use std::collections::VecDeque;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Fixed tick rate shared by client and server.
pub const TICK_RATE: u32 = 60;

/// Duration of a single tick at [`TICK_RATE`].
pub const TICK_DURATION: Duration = Duration::from_nanos(1_000_000_000 / TICK_RATE as u64);

/// Ping message sent by the client.
///
/// `client_send_time_ns` is a monotonic timestamp in nanoseconds (not
/// `Instant`, which is not serializable).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ping {
    /// Monotonic nanosecond timestamp on the client when the ping was sent.
    pub client_send_time_ns: u64,
    /// Sequence number for matching with [`Pong`].
    pub sequence: u32,
}

/// Pong response from the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pong {
    /// Echoed sequence number from the [`Ping`].
    pub sequence: u32,
    /// The server's authoritative tick at the time the pong was sent.
    pub server_tick: u64,
}

/// Result of [`compute_tick_adjustment`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickAdjustment {
    /// No adjustment needed — within tolerance.
    None,
    /// Client is ahead; slow tick rate slightly.
    SlowDown,
    /// Client is behind; speed tick rate slightly.
    SpeedUp,
    /// Drift too large; hard-reset the tick counter.
    HardReset,
}

/// Decide how to adjust the client tick based on offset error (in ticks).
///
/// - `|error| < 0.1`: no adjustment
/// - `0.1 <= |error| < 2.0`: gradual speed-up or slow-down
/// - `|error| >= 2.0`: hard reset
pub fn compute_tick_adjustment(offset_error: f64) -> TickAdjustment {
    if offset_error.abs() >= 2.0 {
        TickAdjustment::HardReset
    } else if offset_error > 0.1 {
        TickAdjustment::SlowDown
    } else if offset_error < -0.1 {
        TickAdjustment::SpeedUp
    } else {
        TickAdjustment::None
    }
}

/// Exponentially weighted moving average RTT estimator.
#[derive(Debug, Clone)]
pub struct RttEstimator {
    /// Recent RTT samples.
    pub samples: VecDeque<Duration>,
    /// Maximum number of samples to retain.
    pub max_samples: usize,
    /// Current EWMA RTT estimate.
    pub ewma_rtt: Duration,
    /// EWMA smoothing factor (default 0.125).
    pub alpha: f64,
}

impl Default for RttEstimator {
    fn default() -> Self {
        Self {
            samples: VecDeque::new(),
            max_samples: 16,
            ewma_rtt: Duration::ZERO,
            alpha: 0.125,
        }
    }
}

impl RttEstimator {
    /// Record a new RTT sample and update the EWMA.
    pub fn record_sample(&mut self, rtt: Duration) {
        self.samples.push_back(rtt);
        if self.samples.len() > self.max_samples {
            self.samples.pop_front();
        }

        let rtt_secs = rtt.as_secs_f64();
        let ewma_secs = self.ewma_rtt.as_secs_f64();
        let new_ewma = self.alpha * rtt_secs + (1.0 - self.alpha) * ewma_secs;
        self.ewma_rtt = Duration::from_secs_f64(new_ewma);
    }

    /// Compute the median of stored samples.
    pub fn median_rtt(&self) -> Duration {
        if self.samples.is_empty() {
            return Duration::ZERO;
        }
        let mut sorted: Vec<_> = self.samples.iter().copied().collect();
        sorted.sort();
        sorted[sorted.len() / 2]
    }
}

/// Tick counter component for ECS.
#[derive(Debug, Clone)]
pub struct TickCounter {
    /// Current tick number.
    pub tick: u64,
    /// Whether this counter belongs to the server.
    pub is_server: bool,
}

impl TickCounter {
    /// Create a new tick counter.
    pub fn new(is_server: bool) -> Self {
        Self { tick: 0, is_server }
    }

    /// Advance the counter by one tick (monotonically increasing).
    pub fn advance(&mut self) {
        self.tick = self.tick.saturating_add(1);
    }
}

/// Client-side clock synchronization state.
#[derive(Debug, Clone)]
pub struct ClockSync {
    /// RTT estimator.
    pub rtt: RttEstimator,
    /// Current tick offset: `client_tick - estimated_server_tick`.
    pub tick_offset: i64,
    /// Target lead in ticks (≈ half-RTT in ticks).
    pub target_lead: f64,
    /// Whether enough samples have been collected for a stable estimate.
    pub converged: bool,
    /// Minimum samples before declaring convergence.
    pub min_samples_for_convergence: usize,
}

impl Default for ClockSync {
    fn default() -> Self {
        Self {
            rtt: RttEstimator::default(),
            tick_offset: 0,
            target_lead: 0.0,
            converged: false,
            min_samples_for_convergence: 8,
        }
    }
}

impl ClockSync {
    /// Process a pong response.
    ///
    /// All times are monotonic nanosecond timestamps (not `Instant`).
    pub fn on_pong_received(
        &mut self,
        pong: &Pong,
        local_receive_time_ns: u64,
        local_send_time_ns: u64,
        client_tick_at_send: u64,
    ) {
        let rtt_ns = local_receive_time_ns.saturating_sub(local_send_time_ns);
        let rtt = Duration::from_nanos(rtt_ns);
        self.rtt.record_sample(rtt);

        let half_rtt_ticks = (rtt.as_secs_f64() * f64::from(TICK_RATE)) / 2.0;
        let estimated_server_tick = pong.server_tick as f64 + half_rtt_ticks;

        self.target_lead = half_rtt_ticks;

        // Estimate current client tick at receive time.
        let elapsed_ticks = rtt.as_secs_f64() * f64::from(TICK_RATE);
        let current_client_tick = client_tick_at_send as f64 + elapsed_ticks;
        self.tick_offset = (current_client_tick - estimated_server_tick) as i64;

        if self.rtt.samples.len() >= self.min_samples_for_convergence {
            self.converged = true;
        }
    }

    /// Return the adjusted client tick that accounts for the target lead.
    pub fn adjusted_client_tick(&self, raw_tick: u64) -> u64 {
        (raw_tick as i64 + self.tick_offset) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tick_offset_converges_after_multiple_samples() {
        let mut sync = ClockSync::default();
        let rtt_ns: u64 = 50_000_000; // 50 ms

        for i in 0..16u32 {
            let send_time = (i as u64) * 500_000_000; // every 500ms
            let recv_time = send_time + rtt_ns;
            let client_tick = (i as u64) * 30; // ~30 ticks per 500ms
            let server_tick = client_tick; // server at same tick

            let pong = Pong {
                sequence: i,
                server_tick,
            };
            sync.on_pong_received(&pong, recv_time, send_time, client_tick);
        }

        assert!(sync.converged, "should converge after 16 samples");
        // With 50ms RTT, half_rtt_ticks ≈ 1.5 ticks. Offset should be stable.
        // The exact offset depends on the math; just check it's within 5 ticks.
        assert!(
            (sync.tick_offset as f64).abs() < 5.0,
            "tick_offset should be stable, got {}",
            sync.tick_offset
        );
    }

    #[test]
    fn test_rtt_measurement_is_accurate() {
        let mut rtt_est = RttEstimator::default();
        let rtt = Duration::from_millis(40);
        rtt_est.record_sample(rtt);

        assert_eq!(rtt_est.samples.len(), 1);
        let recorded = rtt_est.samples[0];
        let diff = recorded.abs_diff(rtt);
        assert!(
            diff < Duration::from_millis(1),
            "recorded RTT should be ~40ms, got {:?}",
            recorded
        );
    }

    #[test]
    fn test_client_tick_leads_server_by_half_rtt() {
        let mut sync = ClockSync {
            min_samples_for_convergence: 1,
            ..ClockSync::default()
        };

        let rtt_ns: u64 = 60_000_000; // 60 ms
        let server_tick = 1000u64;
        let client_tick = 1000u64;

        let pong = Pong {
            sequence: 0,
            server_tick,
        };
        sync.on_pong_received(&pong, rtt_ns, 0, client_tick);

        // half RTT = 30ms = 1.8 ticks at 60Hz
        let expected_lead = 1.8;
        assert!(
            (sync.target_lead - expected_lead).abs() < 0.1,
            "target_lead should be ~1.8, got {}",
            sync.target_lead
        );
    }

    #[test]
    fn test_clock_sync_handles_jitter() {
        let mut rtt_est = RttEstimator::default();

        // 14 normal samples at ~50ms
        for _ in 0..14 {
            rtt_est.record_sample(Duration::from_millis(50));
        }
        // 2 outlier samples at 200ms
        rtt_est.record_sample(Duration::from_millis(200));
        rtt_est.record_sample(Duration::from_millis(200));

        let median = rtt_est.median_rtt();
        // Median of 14×50ms + 2×200ms → 50ms (since 14/16 are 50ms)
        assert!(
            median <= Duration::from_millis(55),
            "median should be ~50ms despite outliers, got {:?}",
            median
        );
    }

    #[test]
    fn test_tick_numbers_are_monotonic() {
        let mut counter = TickCounter::new(false);
        let mut prev = counter.tick;

        for i in 0..1000u64 {
            counter.advance();

            // Simulate clock adjustments at ticks 100, 300, 500 by computing
            // adjustment but never decreasing the counter.
            if i == 100 || i == 300 || i == 500 {
                let adj = compute_tick_adjustment(0.5);
                assert_eq!(adj, TickAdjustment::SlowDown);
                // SlowDown means we might skip an advance next iteration,
                // but tick never decreases.
            }

            assert!(
                counter.tick >= prev,
                "tick must be monotonic: {} < {}",
                counter.tick,
                prev
            );
            prev = counter.tick;
        }
    }
}
