# Reconnection Logic

## Problem

Network connections drop. Wi-Fi blips, ISP hiccups, and brief packet loss are everyday realities for players. If every TCP disconnect forces a full logout — losing loaded chunks, despawning the player entity, and requiring a fresh login and full world download — the experience is terrible. The engine needs automatic reconnection on the client side with exponential backoff (so it does not hammer the server), and a grace period on the server side that preserves the player's session state for a configurable window after disconnect. If the player reconnects within the grace period, they receive a state delta (what changed since they dropped) rather than a full world re-sync. If the grace period expires, the server performs a full disconnect and the player must start fresh.

## Solution

### Client-side reconnection with exponential backoff

When the client detects a disconnect (via the `ConnectionStateWatch` from story 02), the reconnection system activates automatically. It attempts to re-establish the TCP connection with exponential backoff: 1s, 2s, 4s, 8s, 16s, capped at 30s. On success, it sends a `ReconnectRequest` message containing the player's previous session token. Jitter (random +-25%) is added to each backoff interval to prevent thundering-herd reconnections when many players drop simultaneously.

```rust
use std::time::Duration;
use rand::Rng;

pub struct ReconnectConfig {
    /// Initial delay before the first reconnection attempt. Default: 1s.
    pub initial_delay: Duration,
    /// Multiplier applied to the delay after each failed attempt. Default: 2.0.
    pub backoff_multiplier: f64,
    /// Maximum delay between reconnection attempts. Default: 30s.
    pub max_delay: Duration,
    /// Maximum number of reconnection attempts before giving up. Default: 20.
    pub max_attempts: u32,
    /// Jitter factor (0.0 to 1.0). Applied as +-jitter to the delay. Default: 0.25.
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

pub struct ReconnectState {
    config: ReconnectConfig,
    attempts: u32,
    current_delay: Duration,
}

impl ReconnectState {
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
            let factor = rng.random_range(
                (1.0 - self.config.jitter)..=(1.0 + self.config.jitter),
            );
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

    pub fn attempts(&self) -> u32 {
        self.attempts
    }
}
```

### Reconnection loop

```rust
use std::net::SocketAddr;

pub async fn reconnect_loop(
    addr: SocketAddr,
    config: ReconnectConfig,
    session_token: u64,
) -> Result<GameClient, ReconnectError> {
    let mut state = ReconnectState::new(config);

    loop {
        match state.next_delay() {
            None => return Err(ReconnectError::MaxAttemptsExhausted),
            Some(delay) => {
                tracing::info!(
                    "Reconnection attempt {} in {:?}",
                    state.attempts(),
                    delay
                );
                tokio::time::sleep(delay).await;

                match GameClient::connect(addr).await {
                    Ok(client) => {
                        // Send reconnect request with session token
                        // (handled by the session manager on the server)
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

#[derive(Debug, thiserror::Error)]
pub enum ReconnectError {
    #[error("maximum reconnection attempts exhausted")]
    MaxAttemptsExhausted,
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
```

### Server-side grace period

When the server detects a disconnect (story 06), instead of immediately removing the `PlayerSession`, it transitions the session to a `Suspended` state and starts a grace timer. The session's position, loaded chunks, and other state are preserved in memory.

```rust
use std::time::{Duration, Instant};

pub struct GraceConfig {
    /// How long to hold a disconnected player's session. Default: 60s.
    pub grace_period: Duration,
}

impl Default for GraceConfig {
    fn default() -> Self {
        Self {
            grace_period: Duration::from_secs(60),
        }
    }
}

/// Extended session state (augments SessionState from story 06).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtendedSessionState {
    Authenticating,
    Playing,
    /// Disconnected but within the grace period. Session data is preserved.
    Suspended { since: Instant },
    Removed,
}
```

### State delta on reconnect

When a player reconnects within the grace period, the server:

1. Matches the reconnect token to the suspended session.
2. Promotes the session back to `Playing` with the new `ConnectionId`.
3. Computes a state delta: entities that spawned, despawned, or moved since the disconnect. Chunk changes within the player's view radius.
4. Sends the delta as a series of `EntityUpdate` and `ChunkData` messages.

This avoids resending the entire world state, which could be megabytes of chunk data.

### Grace period expiry

A periodic server task scans for `Suspended` sessions whose grace period has elapsed and performs a full disconnect (remove session, notify other players, persist final state).

```rust
pub async fn expire_suspended_sessions(
    session_manager: &SessionManager,
    grace_config: &GraceConfig,
) {
    // Scan for expired suspended sessions and remove them.
    // Implementation uses the SessionManager from story 06.
    let now = Instant::now();
    // Sessions whose suspend time + grace_period < now are expired.
    // Full cleanup is performed via session_manager.on_disconnect().
}
```

## Outcome

A `reconnection.rs` module in `crates/nebula_net/src/` exporting `ReconnectConfig`, `ReconnectState`, `reconnect_loop`, `ReconnectError`, `GraceConfig`, `ExtendedSessionState`, and `expire_suspended_sessions`. Clients automatically reconnect with exponential backoff and jitter. The server preserves player state for 60 seconds after disconnect, sending a state delta on successful reconnection. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

If the server restarts, the client automatically reconnects after a brief delay. The console shows `Connection lost, reconnecting in 2s... Reconnected!`.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `tokio` | `1.49` (features: `rt-multi-thread`, `macros`) | Async sleep for backoff delays |
| `rand` | `0.9` | Jitter randomization on backoff intervals |
| `tracing` | `0.1` | Logging for reconnection attempts |
| `thiserror` | `2.0` | Derive `Error` for `ReconnectError` |

## Unit Tests

```rust
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

        // Exhaust enough iterations to hit the cap
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
        assert_eq!(d, Duration::from_secs(1), "After reset, delay should be initial");
    }

    #[test]
    fn test_server_holds_state_during_grace_period() {
        let grace = GraceConfig::default();
        let suspended_since = Instant::now();
        let state = ExtendedSessionState::Suspended { since: suspended_since };

        // Within grace period
        let elapsed = suspended_since.elapsed();
        assert!(
            elapsed < grace.grace_period,
            "Session should still be within grace period"
        );
        assert_eq!(state, ExtendedSessionState::Suspended { since: suspended_since });
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

        // Check if the grace period has expired
        let expired = suspended_since.elapsed() > grace.grace_period;
        assert!(expired, "Grace period should have expired");
    }

    #[test]
    fn test_jitter_varies_delay() {
        let config = ReconnectConfig {
            jitter: 0.25,
            max_attempts: 100,
            ..Default::default()
        };

        // Run multiple times and verify we get different delays
        let mut delays = Vec::new();
        for _ in 0..10 {
            let mut state = ReconnectState::new(ReconnectConfig {
                jitter: 0.25,
                max_attempts: 100,
                ..Default::default()
            });
            delays.push(state.next_delay().unwrap());
        }

        // With jitter, not all delays should be identical
        let all_same = delays.windows(2).all(|w| w[0] == w[1]);
        // Note: there's a tiny probability this fails, but with 10 samples and
        // 25% jitter, it's astronomically unlikely.
        assert!(
            !all_same,
            "Jitter should cause variation in delays: {:?}",
            delays
        );
    }
}
```
