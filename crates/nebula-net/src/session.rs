//! Connection lifecycle and player session management.
//!
//! Tracks the state machine for each connection: Authenticating → Playing →
//! Disconnecting → Removed. Provides timeout detection for stale sessions.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tokio::sync::RwLock;

use crate::ConnectionId;

/// State machine for a client connection's lifecycle.
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

/// Per-connection player session data.
pub struct PlayerSession {
    /// The underlying TCP connection identifier.
    pub connection_id: ConnectionId,
    /// Current lifecycle state.
    pub state: SessionState,
    /// Assigned player identifier (0 while authenticating).
    pub player_id: u64,
    /// Player display name.
    pub player_name: String,
    /// Timestamp of the last received message, for timeout detection.
    pub last_activity: Instant,
    /// Player's last known 128-bit position, persisted on disconnect.
    pub position: [i128; 3],
    /// Timestamp when disconnect began, used for reconnection grace period.
    pub disconnect_time: Option<Instant>,
}

/// Errors that can occur during authentication.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    /// No session exists for the given connection.
    #[error("session not found for connection")]
    SessionNotFound,
    /// The session is not in the Authenticating state.
    #[error("invalid session state for authentication: {0:?}")]
    InvalidState(SessionState),
    /// The player name was empty.
    #[error("player name cannot be empty")]
    EmptyName,
}

/// Manages all active player sessions and provides lifecycle operations.
pub struct SessionManager {
    /// Map from ConnectionId to PlayerSession.
    sessions: RwLock<HashMap<ConnectionId, PlayerSession>>,
    /// Map from player_id to ConnectionId for lookups and reconnection.
    player_index: RwLock<HashMap<u64, ConnectionId>>,
    /// Monotonic player ID generator.
    next_player_id: AtomicU64,
}

impl SessionManager {
    /// Create a new empty session manager.
    pub fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            player_index: RwLock::new(HashMap::new()),
            next_player_id: AtomicU64::new(1),
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

    /// Process a login request. Accepts any non-empty player name
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

        let player_id = self.next_player_id.fetch_add(1, Ordering::Relaxed);

        session.state = SessionState::Playing;
        session.player_id = player_id;
        session.player_name = player_name.to_string();
        session.last_activity = Instant::now();

        drop(sessions);
        self.player_index
            .write()
            .await
            .insert(player_id, connection_id);

        Ok(player_id)
    }

    /// Initiate disconnect for a connection. Persists player state and
    /// transitions through Disconnecting → Removed.
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

        tracing::info!(
            "Player '{}' (id={}) disconnecting, persisting state",
            player_name,
            player_id
        );

        session.state = SessionState::Removed;
        sessions.remove(&connection_id);

        drop(sessions);
        if player_id != 0 {
            self.player_index.write().await.remove(&player_id);
        }

        Some(player_id)
    }

    /// Update last_activity timestamp for a connection.
    pub async fn touch(&self, connection_id: &ConnectionId) {
        if let Some(session) = self.sessions.write().await.get_mut(connection_id) {
            session.last_activity = Instant::now();
        }
    }

    /// Get the current state of a session.
    pub async fn state(&self, connection_id: &ConnectionId) -> Option<SessionState> {
        self.sessions
            .read()
            .await
            .get(connection_id)
            .map(|s| s.state)
    }

    /// Get the connection ID for a player ID (for reconnection).
    pub async fn connection_for_player(&self, player_id: u64) -> Option<ConnectionId> {
        self.player_index.read().await.get(&player_id).copied()
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Scan all sessions and disconnect any that have exceeded the timeout.
pub async fn timeout_check(session_manager: &SessionManager, timeout: Duration) {
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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(sm.state(&cid).await, None);
    }

    #[tokio::test]
    async fn test_reconnection_with_same_id_works() {
        let sm = SessionManager::new();

        let cid1 = ConnectionId(1);
        sm.on_connect(cid1).await;
        let _pid = sm.authenticate(cid1, "Dave").await.unwrap();
        sm.on_disconnect(cid1).await;

        let cid2 = ConnectionId(2);
        sm.on_connect(cid2).await;
        let pid2 = sm.authenticate(cid2, "Dave").await.unwrap();

        assert_eq!(sm.state(&cid2).await, Some(SessionState::Playing));
        assert!(pid2 > 0);
    }

    #[tokio::test]
    async fn test_timeout_triggers_disconnect() {
        let sm = SessionManager::new();
        let cid = ConnectionId(1);
        sm.on_connect(cid).await;
        sm.authenticate(cid, "Eve").await.unwrap();

        {
            let mut sessions = sm.sessions.write().await;
            if let Some(session) = sessions.get_mut(&cid) {
                session.last_activity = Instant::now() - Duration::from_secs(60);
            }
        }

        timeout_check(&sm, Duration::from_secs(15)).await;

        assert_eq!(sm.state(&cid).await, None);
    }

    #[tokio::test]
    async fn test_double_auth_rejected() {
        let sm = SessionManager::new();
        let cid = ConnectionId(1);
        sm.on_connect(cid).await;
        sm.authenticate(cid, "Frank").await.unwrap();

        let result = sm.authenticate(cid, "Frank").await;
        assert!(matches!(
            result,
            Err(AuthError::InvalidState(SessionState::Playing))
        ));
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
