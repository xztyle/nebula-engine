# Connection Lifecycle

## Problem

A raw TCP connection is not the same as an authenticated player session. The server must enforce a strict state machine for each connection: a new connection starts unauthenticated and can only transition to the "playing" state after successful authentication. During gameplay, the server must track per-player state (position, inventory, loaded chunks). When a player disconnects — whether cleanly or due to network failure — the server must persist their state, notify other players, and clean up resources. Without a well-defined lifecycle, the server risks accepting game commands from unauthenticated connections, leaking resources on disconnect, or losing player state when connections drop unexpectedly. Reconnection is handled by story 08, but this story defines the state machine that reconnection plugs into.

## Solution

### Connection state machine

```
                  ┌──────────────┐
   TCP accept ──> │ Authenticating│
                  └──────┬───────┘
                         │ LoginRequest (valid)
                         v
                  ┌──────────────┐
                  │   Playing     │
                  └──────┬───────┘
                         │ Disconnect / Timeout / Logout
                         v
                  ┌──────────────┐
                  │ Disconnecting │
                  └──────┬───────┘
                         │ Cleanup complete
                         v
                  ┌──────────────┐
                  │   Removed     │
                  └──────────────┘
```

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// Connection accepted, waiting for LoginRequest.
    Authenticating,
    /// Login succeeded, player is active in the world.
    Playing,
    /// Disconnect initiated, cleaning up resources.
    Disconnecting,
    /// Cleanup complete, entry can be removed.
    Removed,
}
```

### Player session

Each authenticated connection has a `PlayerSession` that tracks everything the server needs to know about that player:

```rust
use std::time::Instant;

pub struct PlayerSession {
    pub connection_id: ConnectionId,
    pub state: SessionState,
    pub player_id: u64,
    pub player_name: String,
    /// Timestamp of the last received message, for timeout detection.
    pub last_activity: Instant,
    /// Player's last known 128-bit position, persisted on disconnect.
    pub position: [i128; 3],
    /// Timestamp when disconnect began, used for reconnection grace period (story 08).
    pub disconnect_time: Option<Instant>,
}
```

### Session manager

The `SessionManager` owns all active sessions and provides the lifecycle operations:

```rust
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct SessionManager {
    /// Map from ConnectionId to PlayerSession.
    sessions: RwLock<HashMap<ConnectionId, PlayerSession>>,
    /// Map from player_id to ConnectionId for lookups and reconnection.
    player_index: RwLock<HashMap<u64, ConnectionId>>,
    /// Monotonic player ID generator.
    next_player_id: std::sync::atomic::AtomicU64,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            player_index: RwLock::new(HashMap::new()),
            next_player_id: std::sync::atomic::AtomicU64::new(1),
        }
    }

    /// Called when a new TCP connection is accepted. Creates a session in
    /// the Authenticating state.
    pub async fn on_connect(&self, connection_id: ConnectionId) {
        let session = PlayerSession {
            connection_id,
            state: SessionState::Authenticating,
            player_id: 0,
            player_name: String::new(),
            last_activity: Instant::now(),
            position: [0; 3],
            disconnect_time: None,
        };
        self.sessions.write().await.insert(connection_id, session);
    }

    /// Process a login request. For now, accept any non-empty player name
    /// (placeholder authentication — real auth comes in a future epic).
    pub async fn authenticate(
        &self,
        connection_id: ConnectionId,
        player_name: &str,
    ) -> Result<u64, AuthError> {
        let mut sessions = self.sessions.write().await;
        let session = sessions
            .get_mut(&connection_id)
            .ok_or(AuthError::SessionNotFound)?;

        if session.state != SessionState::Authenticating {
            return Err(AuthError::InvalidState(session.state));
        }

        if player_name.is_empty() {
            return Err(AuthError::EmptyName);
        }

        let player_id = self
            .next_player_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        session.state = SessionState::Playing;
        session.player_id = player_id;
        session.player_name = player_name.to_string();
        session.last_activity = Instant::now();

        drop(sessions); // Release write lock before acquiring player_index lock
        self.player_index
            .write()
            .await
            .insert(player_id, connection_id);

        Ok(player_id)
    }

    /// Initiate disconnect for a connection. Persists player state and
    /// notifies other sessions.
    pub async fn on_disconnect(&self, connection_id: ConnectionId) -> Option<u64> {
        let mut sessions = self.sessions.write().await;
        let session = sessions.get_mut(&connection_id)?;

        if session.state == SessionState::Removed {
            return None;
        }

        session.state = SessionState::Disconnecting;
        session.disconnect_time = Some(Instant::now());

        let player_id = session.player_id;
        let player_name = session.player_name.clone();

        // Persist player state (position, inventory) — placeholder for now
        tracing::info!(
            "Player '{}' (id={}) disconnecting, persisting state",
            player_name,
            player_id
        );

        // Mark as removed
        session.state = SessionState::Removed;
        let removed_session = sessions.remove(&connection_id);

        drop(sessions);
        if player_id != 0 {
            self.player_index.write().await.remove(&player_id);
        }

        Some(player_id)
    }

    /// Update last_activity timestamp for a connection (called on every received message).
    pub async fn touch(&self, connection_id: &ConnectionId) {
        if let Some(session) = self.sessions.write().await.get_mut(connection_id) {
            session.last_activity = Instant::now();
        }
    }

    /// Get the current state of a session.
    pub async fn state(&self, connection_id: &ConnectionId) -> Option<SessionState> {
        self.sessions.read().await.get(connection_id).map(|s| s.state)
    }

    /// Get the connection ID for a player ID (for reconnection in story 08).
    pub async fn connection_for_player(&self, player_id: u64) -> Option<ConnectionId> {
        self.player_index.read().await.get(&player_id).copied()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("session not found for connection")]
    SessionNotFound,
    #[error("invalid session state for authentication: {0:?}")]
    InvalidState(SessionState),
    #[error("player name cannot be empty")]
    EmptyName,
}
```

### Timeout detection

A periodic task scans all sessions for stale `last_activity` timestamps. If a session in the `Playing` state has not received any message within the heartbeat timeout (15 seconds, matching story 02), it is forcibly disconnected:

```rust
use std::time::Duration;

pub async fn timeout_check(
    session_manager: &SessionManager,
    timeout: Duration,
) {
    let sessions = session_manager.sessions.read().await;
    let stale: Vec<ConnectionId> = sessions
        .iter()
        .filter(|(_, s)| s.state == SessionState::Playing && s.last_activity.elapsed() > timeout)
        .map(|(id, _)| *id)
        .collect();
    drop(sessions);

    for id in stale {
        tracing::warn!("Connection {:?} timed out", id);
        session_manager.on_disconnect(id).await;
    }
}
```

### Notifying other players

When a player disconnects, the server must send a `Logout` message to all other connected players so they can remove the disconnected player's entity. This is done by iterating all `Playing` sessions and sending the logout message via the `ConnectionMap` (story 01). The actual broadcast is wired through the message routing layer (story 05).

## Outcome

A `session.rs` module in `crates/nebula_net/src/` exporting `SessionState`, `PlayerSession`, `SessionManager`, `AuthError`, and `timeout_check`. The module manages the full connect-authenticate-play-disconnect lifecycle for every client connection, with placeholder authentication that accepts any name. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

The server logs connection events: `Client 1 connected`, `Client 1 disconnected`. The client handles clean server shutdown without crashing.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `tokio` | `1.49` (features: `rt-multi-thread`, `macros`) | `RwLock` for concurrent session access |
| `tracing` | `0.1` | Logging for lifecycle events |
| `thiserror` | `2.0` | Derive `Error` for `AuthError` |

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn test_new_connection_starts_in_auth_state() {
        let sm = SessionManager::new();
        let cid = ConnectionId(1);
        sm.on_connect(cid).await;
        assert_eq!(sm.state(&cid).await, Some(SessionState::Authenticating));
    }

    #[tokio::test]
    async fn test_successful_auth_transitions_to_playing() {
        let sm = SessionManager::new();
        let cid = ConnectionId(1);
        sm.on_connect(cid).await;

        let result = sm.authenticate(cid, "Alice").await;
        assert!(result.is_ok());
        assert_eq!(sm.state(&cid).await, Some(SessionState::Playing));
    }

    #[tokio::test]
    async fn test_auth_returns_player_id() {
        let sm = SessionManager::new();
        let cid = ConnectionId(1);
        sm.on_connect(cid).await;

        let player_id = sm.authenticate(cid, "Bob").await.unwrap();
        assert!(player_id > 0, "Player ID should be positive");
    }

    #[tokio::test]
    async fn test_empty_name_rejected() {
        let sm = SessionManager::new();
        let cid = ConnectionId(1);
        sm.on_connect(cid).await;

        let result = sm.authenticate(cid, "").await;
        assert!(matches!(result, Err(AuthError::EmptyName)));
        // State should remain Authenticating
        assert_eq!(sm.state(&cid).await, Some(SessionState::Authenticating));
    }

    #[tokio::test]
    async fn test_disconnect_cleans_up() {
        let sm = SessionManager::new();
        let cid = ConnectionId(1);
        sm.on_connect(cid).await;
        sm.authenticate(cid, "Charlie").await.unwrap();

        let player_id = sm.on_disconnect(cid).await;
        assert!(player_id.is_some());
        // Session should be fully removed
        assert_eq!(sm.state(&cid).await, None);
    }

    #[tokio::test]
    async fn test_reconnection_with_same_id_works() {
        let sm = SessionManager::new();

        // First connection
        let cid1 = ConnectionId(1);
        sm.on_connect(cid1).await;
        let pid = sm.authenticate(cid1, "Dave").await.unwrap();
        sm.on_disconnect(cid1).await;

        // Second connection (reconnect)
        let cid2 = ConnectionId(2);
        sm.on_connect(cid2).await;
        let pid2 = sm.authenticate(cid2, "Dave").await.unwrap();

        // New connection should be in Playing state
        assert_eq!(sm.state(&cid2).await, Some(SessionState::Playing));
        // Player ID is different because this is placeholder auth
        // (real reconnection with same player_id is story 08)
        assert!(pid2 > 0);
    }

    #[tokio::test]
    async fn test_timeout_triggers_disconnect() {
        let sm = SessionManager::new();
        let cid = ConnectionId(1);
        sm.on_connect(cid).await;
        sm.authenticate(cid, "Eve").await.unwrap();

        // Manually set last_activity far in the past
        {
            let mut sessions = sm.sessions.write().await;
            if let Some(session) = sessions.get_mut(&cid) {
                session.last_activity = Instant::now() - Duration::from_secs(60);
            }
        }

        // Run timeout check with a 15-second threshold
        timeout_check(&sm, Duration::from_secs(15)).await;

        // Session should have been disconnected and removed
        assert_eq!(sm.state(&cid).await, None);
    }

    #[tokio::test]
    async fn test_double_auth_rejected() {
        let sm = SessionManager::new();
        let cid = ConnectionId(1);
        sm.on_connect(cid).await;
        sm.authenticate(cid, "Frank").await.unwrap();

        // Second auth attempt on the same connection should fail
        let result = sm.authenticate(cid, "Frank").await;
        assert!(matches!(result, Err(AuthError::InvalidState(SessionState::Playing))));
    }

    #[tokio::test]
    async fn test_player_index_updated_on_auth() {
        let sm = SessionManager::new();
        let cid = ConnectionId(42);
        sm.on_connect(cid).await;
        let pid = sm.authenticate(cid, "Grace").await.unwrap();

        let found_cid = sm.connection_for_player(pid).await;
        assert_eq!(found_cid, Some(cid));
    }

    #[tokio::test]
    async fn test_player_index_cleared_on_disconnect() {
        let sm = SessionManager::new();
        let cid = ConnectionId(1);
        sm.on_connect(cid).await;
        let pid = sm.authenticate(cid, "Hank").await.unwrap();

        sm.on_disconnect(cid).await;

        let found_cid = sm.connection_for_player(pid).await;
        assert_eq!(found_cid, None);
    }
}
```
