//! Player join/leave session management.
//!
//! Handles the full lifecycle of player connections: authentication,
//! entity spawning, initial world state delivery, disconnect detection,
//! state persistence, and cleanup.

use std::time::{Duration, Instant};

use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::authority::{AuthoritativeWorld, PlayerState};
use crate::chunk_streaming::ChunkDataMessage;
use crate::replication::{NetworkId, ReplicationServerSystem, SpawnEntity};

// ---------------------------------------------------------------------------
// Protocol version
// ---------------------------------------------------------------------------

/// Current multiplayer protocol version.
pub const PROTOCOL_VERSION: u32 = 1;

/// Default timeout duration for detecting disconnected clients.
pub const DEFAULT_TIMEOUT_SECS: u64 = 30;

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

/// Client-to-server connection request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConnectionRequest {
    /// Human-readable player name.
    pub player_name: String,
    /// Authentication token (opaque string).
    pub auth_token: String,
    /// Protocol version the client speaks.
    pub protocol_version: u32,
}

/// Server-to-client authentication result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AuthResult {
    /// Connection accepted.
    Accepted {
        /// Assigned client identifier.
        client_id: u64,
        /// The player's network entity id.
        network_id: NetworkId,
    },
    /// Connection rejected.
    Rejected {
        /// Human-readable reason.
        reason: String,
    },
}

/// Snapshot of world state sent to a freshly-joined client.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InitialWorldState {
    /// The joining player's own network entity id.
    pub your_network_id: NetworkId,
    /// Current server tick.
    pub server_tick: u64,
    /// In-game world time in seconds.
    pub world_time: f64,
    /// Pre-loaded chunk data within the player's interest radius.
    pub nearby_chunks: Vec<ChunkDataMessage>,
    /// Entity spawn messages for nearby entities.
    pub nearby_entities: Vec<SpawnEntity>,
}

/// Client-to-server voluntary disconnect.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DisconnectRequest {
    /// Why the client is disconnecting.
    pub reason: DisconnectReason,
}

/// Reason for a player leaving the server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum DisconnectReason {
    /// Player quit voluntarily.
    Voluntary,
    /// Server kicked the player.
    Kicked,
    /// Connection timed out.
    Timeout,
}

// ---------------------------------------------------------------------------
// Connection tracking
// ---------------------------------------------------------------------------

/// Tracks liveness of a connected client.
pub struct ConnectionState {
    /// Unique client identifier.
    pub client_id: u64,
    /// Timestamp of the last received heartbeat (or any message).
    pub last_heartbeat: Instant,
    /// How long to wait before considering the client dead.
    pub timeout_duration: Duration,
}

impl ConnectionState {
    /// Creates a new connection state with the default 30-second timeout.
    pub fn new(client_id: u64) -> Self {
        Self {
            client_id,
            last_heartbeat: Instant::now(),
            timeout_duration: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        }
    }

    /// Returns `true` if the client has exceeded the timeout window.
    pub fn is_timed_out(&self) -> bool {
        self.last_heartbeat.elapsed() > self.timeout_duration
    }

    /// Records a heartbeat (resets the timeout clock).
    pub fn record_heartbeat(&mut self) {
        self.last_heartbeat = Instant::now();
    }
}

// ---------------------------------------------------------------------------
// Player save data
// ---------------------------------------------------------------------------

/// Persisted player state for session continuity across joins.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlayerSaveData {
    /// Player display name.
    pub player_name: String,
    /// Last known X position in millimeters.
    pub x: i64,
    /// Last known Y position in millimeters.
    pub y: i64,
    /// Last known Z position in millimeters.
    pub z: i64,
    /// Last server tick when the player was online.
    pub last_seen_tick: u64,
}

// ---------------------------------------------------------------------------
// spawn / despawn helpers
// ---------------------------------------------------------------------------

/// Spawns a player entity in the authoritative world.
///
/// If `saved_state` is provided the player resumes at their last position;
/// otherwise they start at the default spawn (origin).
///
/// Returns the ECS [`Entity`] and assigned [`NetworkId`].
pub fn spawn_player(
    world: &mut AuthoritativeWorld,
    replication: &mut ReplicationServerSystem,
    client_id: u64,
    saved_state: Option<&PlayerSaveData>,
) -> (Entity, NetworkId) {
    let network_id = replication.allocate_network_id();

    let (x, y, z) = match saved_state {
        Some(save) => (save.x, save.y, save.z),
        None => (0, 0, 0),
    };

    let player_state = PlayerState {
        player_id: client_id,
        x,
        y,
        z,
        yaw_mrad: 0,
        pitch_mrad: 0,
    };

    let entity = world.world_mut().spawn((player_state, network_id)).id();

    replication.add_client(client_id);

    (entity, network_id)
}

/// Saves the current state of a player before disconnect.
///
/// Returns `None` if the player entity cannot be found.
pub fn save_player_state(
    world: &AuthoritativeWorld,
    player_name: &str,
    player_id: u64,
) -> Option<PlayerSaveData> {
    let ps = world.find_player(player_id)?;
    Some(PlayerSaveData {
        player_name: player_name.to_string(),
        x: ps.x,
        y: ps.y,
        z: ps.z,
        last_seen_tick: world.tick(),
    })
}

/// Removes a player entity from the authoritative world and cleans up
/// replication state. Other clients are notified via the normal replication
/// despawn path.
pub fn remove_player(
    world: &mut AuthoritativeWorld,
    replication: &mut ReplicationServerSystem,
    client_id: u64,
    entity: Entity,
) {
    world.world_mut().despawn(entity);
    replication.remove_client(client_id);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::replication::{ReplicationServerSystem, ReplicationSet};

    /// Helper: set up a world + replication and join a player, returning
    /// the pieces needed for further assertions.
    fn join_player(
        world: &mut AuthoritativeWorld,
        repl: &mut ReplicationServerSystem,
        client_id: u64,
        saved: Option<&PlayerSaveData>,
    ) -> (Entity, NetworkId) {
        spawn_player(world, repl, client_id, saved)
    }

    // 1. test_join_spawns_entity_visible_to_others
    #[test]
    fn test_join_spawns_entity_visible_to_others() {
        let mut world = AuthoritativeWorld::new();
        let mut repl = ReplicationServerSystem::new();
        let mut rep_set = ReplicationSet::new();
        rep_set.register::<NetworkId>("NetworkId");
        rep_set.register::<PlayerState>("PlayerState");

        // Client A joins first.
        let (_entity_a, net_a) = join_player(&mut world, &mut repl, 1, None);
        // Run replication so A's shadow is initialised.
        let _ = repl.replicate(world.world(), &rep_set, world.tick());

        // Client B joins.
        let (_entity_b, net_b) = join_player(&mut world, &mut repl, 2, None);

        // Replicate — client A should see a SpawnEntity for B.
        let msgs = repl.replicate(world.world(), &rep_set, world.tick());

        let a_msgs = msgs.get(&1).expect("client A should have messages");
        assert!(
            a_msgs.spawns.iter().any(|s| s.network_id == net_b),
            "Client A must see spawn for client B (net_id={net_b:?})"
        );

        // Client B sees client A as a spawn too (first replication for B).
        let b_msgs = msgs.get(&2).expect("client B should have messages");
        assert!(
            b_msgs.spawns.iter().any(|s| s.network_id == net_a),
            "Client B must see spawn for client A (net_id={net_a:?})"
        );
    }

    // 2. test_leave_despawns_entity
    #[test]
    fn test_leave_despawns_entity() {
        let mut world = AuthoritativeWorld::new();
        let mut repl = ReplicationServerSystem::new();
        let mut rep_set = ReplicationSet::new();
        rep_set.register::<NetworkId>("NetworkId");
        rep_set.register::<PlayerState>("PlayerState");

        let (_ea, _na) = join_player(&mut world, &mut repl, 1, None);
        let (eb, nb) = join_player(&mut world, &mut repl, 2, None);

        // Baseline replication.
        let _ = repl.replicate(world.world(), &rep_set, world.tick());

        // Client B leaves.
        remove_player(&mut world, &mut repl, 2, eb);

        // Replicate — A should receive DespawnEntity for B.
        world.advance_tick();
        let msgs = repl.replicate(world.world(), &rep_set, world.tick());

        let a_msgs = msgs.get(&1).expect("client A messages");
        assert!(
            a_msgs.despawns.iter().any(|d| d.network_id == nb),
            "Client A must see despawn for client B"
        );

        // Entity no longer in world.
        assert!(world.find_player(2).is_none());
    }

    // 3. test_initial_state_includes_nearby_data
    #[test]
    fn test_initial_state_includes_nearby_data() {
        let mut world = AuthoritativeWorld::new();
        let mut repl = ReplicationServerSystem::new();
        let mut rep_set = ReplicationSet::new();
        rep_set.register::<NetworkId>("NetworkId");
        rep_set.register::<PlayerState>("PlayerState");

        // Pre-existing entities.
        let _ = join_player(&mut world, &mut repl, 1, None);
        let _ = join_player(&mut world, &mut repl, 2, None);
        // Baseline replication for existing clients.
        let _ = repl.replicate(world.world(), &rep_set, world.tick());

        // Client 3 joins.
        let (_e3, net3) = join_player(&mut world, &mut repl, 3, None);

        // Build InitialWorldState from the first replication pass for client 3.
        let msgs = repl.replicate(world.world(), &rep_set, world.tick());
        let c3_msgs = msgs.get(&3).expect("client 3 messages");

        let initial = InitialWorldState {
            your_network_id: net3,
            server_tick: world.tick(),
            world_time: 0.0,
            nearby_chunks: vec![], // no chunks loaded in this test
            nearby_entities: c3_msgs.spawns.clone(),
        };

        // Client 3 should see at least the 2 pre-existing players.
        assert!(
            initial.nearby_entities.len() >= 2,
            "expected >=2 entities, got {}",
            initial.nearby_entities.len()
        );
        assert_eq!(initial.your_network_id, net3);
    }

    // 4. test_other_players_are_notified
    #[test]
    fn test_other_players_are_notified() {
        let mut world = AuthoritativeWorld::new();
        let mut repl = ReplicationServerSystem::new();
        let mut rep_set = ReplicationSet::new();
        rep_set.register::<NetworkId>("NetworkId");
        rep_set.register::<PlayerState>("PlayerState");

        // Three existing clients.
        let _ = join_player(&mut world, &mut repl, 1, None);
        let _ = join_player(&mut world, &mut repl, 2, None);
        let _ = join_player(&mut world, &mut repl, 3, None);
        let _ = repl.replicate(world.world(), &rep_set, world.tick());

        // Client D (4) joins.
        let (_ed, net_d) = join_player(&mut world, &mut repl, 4, None);
        let msgs = repl.replicate(world.world(), &rep_set, world.tick());

        // All three existing clients must see a spawn for D.
        for cid in [1, 2, 3] {
            let m = msgs
                .get(&cid)
                .unwrap_or_else(|| panic!("client {cid} msgs"));
            assert!(
                m.spawns.iter().any(|s| s.network_id == net_d),
                "Client {cid} must see spawn for D"
            );
        }
    }

    // 5. test_state_persists_across_rejoin
    #[test]
    fn test_state_persists_across_rejoin() {
        let mut world = AuthoritativeWorld::new();
        let mut repl = ReplicationServerSystem::new();

        // Client A joins at default position.
        let (entity_a, _na) = join_player(&mut world, &mut repl, 1, None);

        // Move player to a new position.
        if let Some(ps) = world.find_player_mut(1) {
            ps.x = 5000;
            ps.y = 3000;
            ps.z = 1000;
        }
        world.advance_tick();

        // Save state and disconnect.
        let save = save_player_state(&world, "Alice", 1).expect("save");
        remove_player(&mut world, &mut repl, 1, entity_a);
        assert!(world.find_player(1).is_none());

        // Rejoin with saved state.
        let (_entity_a2, _na2) = join_player(&mut world, &mut repl, 1, Some(&save));
        let ps = world
            .find_player(1)
            .expect("player should exist after rejoin");
        assert_eq!(ps.x, 5000);
        assert_eq!(ps.y, 3000);
        assert_eq!(ps.z, 1000);
        assert_eq!(save.player_name, "Alice");
    }
}
