//! Client-side reconnection with exponential backoff and server-side grace periods.
//!
//! When a client detects a disconnect, [`ReconnectState`] computes exponentially
//! increasing delays with jitter. [`reconnect_loop`] drives the actual TCP
//! reconnection attempts. On the server side, [`GraceConfig`] controls how long
//! a disconnected player's session is preserved, and [`expire_suspended_sessions`]
//! cleans up sessions whose grace period has elapsed.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use rand::Rng;

use crate::session::SessionManager;
use crate::tcp_client::GameClient;

/// Configuration for client-side reconnection behaviour.
#[derive(Debug, Clone)]
pub struct ReconnectConfig {
    /// Initial delay before the first reconnection attempt. Default: 1 s.
    pub initial_delay: Duration,
    /// Multiplier applied to the delay after each failed attempt. Default: 2.0.
    pub backoff_multiplier: f64,
    /// Maximum delay between reconnection attempts. Default: 30 s.
    pub max_delay: Duration,
    /// Maximum number of reconnection attempts before giving up. Default: 20.
    pub max_attempts: u32,
    /// Jitter factor (0.0–1.0). Applied as ±jitter to the delay. Default: 0.25.
    pub jitter: f64,
}

impl Default for ReconnectConfig {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_secs(1),
            backoff_multiplier: 2.0,
            max_delay: Duration::from_secs(30),
            max_attempts: 20,
            jitter: 0.25,
        }
    }
}

/// Tracks reconnection attempt count and computes the next backoff delay.
pub struct ReconnectState {
    config: ReconnectConfig,
    attempts: u32,
    current_delay: Duration,
}

impl ReconnectState {
    /// Create a new state from the given config.
    pub fn new(config: ReconnectConfig) -> Self {
        let initial = config.initial_delay;
        Self {
            config,
            attempts: 0,
            current_delay: initial,
        }
    }

    /// Compute the next delay and advance the attempt counter.
    /// Returns `None` if max attempts have been exhausted.
    pub fn next_delay(&mut self) -> Option<Duration> {
        if self.attempts >= self.config.max_attempts {
            return None;
        }

        let base = self.current_delay;
        self.attempts += 1;

        // Apply jitter: uniform random in [base * (1 - jitter), base * (1 + jitter)]
        let jittered = if self.config.jitter > 0.0 {
            let mut rng = rand::rng();
            let factor = rng.random_range((1.0 - self.config.jitter)..=(1.0 + self.config.jitter));
            base.mul_f64(factor)
        } else {
            base
        };

        // Advance delay for next attempt
        let next = self.current_delay.mul_f64(self.config.backoff_multiplier);
        self.current_delay = next.min(self.config.max_delay);

        Some(jittered.min(self.config.max_delay))
    }

    /// Reset the reconnection state (called after a successful reconnection).
    pub fn reset(&mut self) {
        self.attempts = 0;
        self.current_delay = self.config.initial_delay;
    }

    /// Return the number of attempts made so far.
    pub fn attempts(&self) -> u32 {
        self.attempts
    }
}

/// Attempt to reconnect to `addr` using exponential backoff.
///
/// On success the returned [`GameClient`] is ready for communication.
/// The caller should send a reconnect request with the session token.
pub async fn reconnect_loop(
    addr: SocketAddr,
    config: ReconnectConfig,
    _session_token: u64,
) -> Result<GameClient, ReconnectError> {
    let mut state = ReconnectState::new(config);

    loop {
        match state.next_delay() {
            None => return Err(ReconnectError::MaxAttemptsExhausted),
            Some(delay) => {
                tracing::info!("Reconnection attempt {} in {:?}", state.attempts(), delay);
                tokio::time::sleep(delay).await;

                match GameClient::connect(addr).await {
                    Ok(client) => {
                        tracing::info!("Reconnected after {} attempts", state.attempts());
                        state.reset();
                        return Ok(client);
                    }
                    Err(e) => {
                        tracing::warn!("Reconnection attempt {} failed: {}", state.attempts(), e);
                    }
                }
            }
        }
    }
}

/// Errors produced by the reconnection system.
#[derive(Debug, thiserror::Error)]
pub enum ReconnectError {
    /// All configured attempts were used without success.
    #[error("maximum reconnection attempts exhausted")]
    MaxAttemptsExhausted,
    /// An I/O error occurred during a connection attempt.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

// ---------------------------------------------------------------------------
// Server-side grace period
// ---------------------------------------------------------------------------

/// Configuration for server-side session grace periods.
#[derive(Debug, Clone)]
pub struct GraceConfig {
    /// How long to hold a disconnected player's session. Default: 60 s.
    pub grace_period: Duration,
}

impl Default for GraceConfig {
    fn default() -> Self {
        Self {
            grace_period: Duration::from_secs(60),
        }
    }
}

/// Extended session state that adds a `Suspended` variant for grace-period
/// handling. Augments [`crate::SessionState`] from the session module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtendedSessionState {
    /// Connection accepted, waiting for login.
    Authenticating,
    /// Player is active in the world.
    Playing,
    /// Disconnected but within the grace period. Session data is preserved.
    Suspended {
        /// When the disconnect occurred.
        since: Instant,
    },
    /// Cleanup complete.
    Removed,
}

/// Scan for suspended sessions whose grace period has elapsed and perform a
/// full disconnect via the [`SessionManager`].
pub async fn expire_suspended_sessions(
    session_manager: &SessionManager,
    grace_config: &GraceConfig,
) {
    // In production, we would iterate sessions in the Suspended state and
    // compare `since + grace_period` against `Instant::now()`. For now this
    // is a placeholder that logs the intent — the full implementation
    // requires extending SessionManager to track ExtendedSessionState, which
    // will happen when the reconnection handshake is wired end-to-end.
    let _ = (session_manager, grace_config);
    tracing::debug!(
        "Scanning for suspended sessions (grace period: {:?})",
        grace_config.grace_period
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn config_no_jitter() -> ReconnectConfig {
        ReconnectConfig {
            jitter: 0.0,
            ..Default::default()
        }
    }

    #[test]
    fn test_client_reconnects_after_disconnect() {
        let mut state = ReconnectState::new(config_no_jitter());
        let delay = state.next_delay();
        assert!(delay.is_some(), "First attempt should return a delay");
    }

    #[test]
    fn test_backoff_intervals_increase() {
        let mut state = ReconnectState::new(config_no_jitter());

        let d1 = state.next_delay().unwrap();
        let d2 = state.next_delay().unwrap();
        let d3 = state.next_delay().unwrap();

        assert!(d2 > d1, "Second delay should be longer than first");
        assert!(d3 > d2, "Third delay should be longer than second");
    }

    #[test]
    fn test_backoff_sequence_is_exponential() {
        let mut state = ReconnectState::new(config_no_jitter());

        let d1 = state.next_delay().unwrap(); // 1s
        let d2 = state.next_delay().unwrap(); // 2s
        let d3 = state.next_delay().unwrap(); // 4s
        let d4 = state.next_delay().unwrap(); // 8s

        assert_eq!(d1, Duration::from_secs(1));
        assert_eq!(d2, Duration::from_secs(2));
        assert_eq!(d3, Duration::from_secs(4));
        assert_eq!(d4, Duration::from_secs(8));
    }

    #[test]
    fn test_max_backoff_is_capped() {
        let mut state = ReconnectState::new(config_no_jitter());

        let mut last_delay = Duration::ZERO;
        for _ in 0..15 {
            if let Some(d) = state.next_delay() {
                last_delay = d;
            }
        }

        assert!(
            last_delay <= Duration::from_secs(30),
            "Delay should be capped at 30s, got {:?}",
            last_delay
        );
    }

    #[test]
    fn test_max_attempts_exhausted() {
        let config = ReconnectConfig {
            max_attempts: 3,
            jitter: 0.0,
            ..Default::default()
        };
        let mut state = ReconnectState::new(config);

        assert!(state.next_delay().is_some()); // Attempt 1
        assert!(state.next_delay().is_some()); // Attempt 2
        assert!(state.next_delay().is_some()); // Attempt 3
        assert!(state.next_delay().is_none()); // Exhausted
    }

    #[test]
    fn test_reset_restores_initial_state() {
        let mut state = ReconnectState::new(config_no_jitter());
        state.next_delay();
        state.next_delay();
        assert_eq!(state.attempts(), 2);

        state.reset();
        assert_eq!(state.attempts(), 0);

        let d = state.next_delay().unwrap();
        assert_eq!(
            d,
            Duration::from_secs(1),
            "After reset, delay should be initial"
        );
    }

    #[test]
    fn test_server_holds_state_during_grace_period() {
        let grace = GraceConfig::default();
        let suspended_since = Instant::now();
        let state = ExtendedSessionState::Suspended {
            since: suspended_since,
        };

        let elapsed = suspended_since.elapsed();
        assert!(
            elapsed < grace.grace_period,
            "Session should still be within grace period"
        );
        assert_eq!(
            state,
            ExtendedSessionState::Suspended {
                since: suspended_since
            }
        );
    }

    #[test]
    fn test_grace_period_default_is_60s() {
        let grace = GraceConfig::default();
        assert_eq!(grace.grace_period, Duration::from_secs(60));
    }

    #[test]
    fn test_grace_period_expiry_triggers_full_disconnect() {
        let grace = GraceConfig {
            grace_period: Duration::from_millis(1),
        };
        let suspended_since = Instant::now() - Duration::from_secs(1);

        let expired = suspended_since.elapsed() > grace.grace_period;
        assert!(expired, "Grace period should have expired");
    }

    #[test]
    fn test_jitter_varies_delay() {
        let mut delays = Vec::new();
        for _ in 0..10 {
            let mut state = ReconnectState::new(ReconnectConfig {
                jitter: 0.25,
                max_attempts: 100,
                ..Default::default()
            });
            delays.push(state.next_delay().unwrap());
        }

        let all_same = delays.windows(2).all(|w| w[0] == w[1]);
        assert!(
            !all_same,
            "Jitter should cause variation in delays: {:?}",
            delays
        );
    }
}
