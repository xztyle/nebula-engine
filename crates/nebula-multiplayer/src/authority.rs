//! Server-authoritative world state, client intents, and tick scheduling.
//!
//! The server owns the canonical world state. Clients submit [`ClientIntent`]
//! messages describing *what they want to do*, and the server validates and
//! applies them each tick via [`IntentValidator`] and [`AuthoritativeWorld`].

use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Server tick rate in Hz.
pub const SERVER_TICK_RATE: u32 = 60;

/// Duration of a single server tick in seconds.
pub const TICK_DURATION_SECS: f64 = 1.0 / SERVER_TICK_RATE as f64;

/// Maximum movement distance per tick in millimeters (prevents teleport hacks).
/// ~10 m/s at 60 Hz ≈ 167 mm/tick.
const MAX_MOVE_DISTANCE_MM: i128 = 200;

/// Maximum interaction range in millimeters (5 meters).
const MAX_INTERACT_RANGE_MM: i128 = 5_000;

// ---------------------------------------------------------------------------
// ClientIntent
// ---------------------------------------------------------------------------

/// A client's declared intention for one tick. The server validates each
/// intent against the authoritative world state before applying it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ClientIntent {
    /// Move the player's entity by a delta (dx, dy, dz) in millimeters.
    Move {
        /// Player identifier.
        player_id: u64,
        /// X displacement in millimeters.
        dx: i64,
        /// Y displacement in millimeters.
        dy: i64,
        /// Z displacement in millimeters.
        dz: i64,
    },

    /// Place a voxel at the given world coordinates.
    PlaceVoxel {
        /// Player identifier.
        player_id: u64,
        /// Voxel type to place.
        voxel_type: u16,
        /// Target X coordinate in millimeters.
        x: i64,
        /// Target Y coordinate in millimeters.
        y: i64,
        /// Target Z coordinate in millimeters.
        z: i64,
    },

    /// Break/remove a voxel at the given world coordinates.
    BreakVoxel {
        /// Player identifier.
        player_id: u64,
        /// Target X coordinate in millimeters.
        x: i64,
        /// Target Y coordinate in millimeters.
        y: i64,
        /// Target Z coordinate in millimeters.
        z: i64,
    },

    /// Interact with an entity (e.g. open a container, talk to NPC).
    Interact {
        /// Player identifier.
        player_id: u64,
        /// Target entity identifier.
        target_entity: u64,
    },

    /// Rotate the player's view (yaw/pitch in milliradians).
    Rotate {
        /// Player identifier.
        player_id: u64,
        /// Yaw delta in milliradians.
        yaw_mrad: i32,
        /// Pitch delta in milliradians.
        pitch_mrad: i32,
    },
}

impl ClientIntent {
    /// Returns the player ID associated with this intent.
    pub fn player_id(&self) -> u64 {
        match self {
            Self::Move { player_id, .. }
            | Self::PlaceVoxel { player_id, .. }
            | Self::BreakVoxel { player_id, .. }
            | Self::Interact { player_id, .. }
            | Self::Rotate { player_id, .. } => *player_id,
        }
    }
}

// ---------------------------------------------------------------------------
// IntentValidationError
// ---------------------------------------------------------------------------

/// Reasons an intent may be rejected by the server.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IntentValidationError {
    /// Player entity not found in the authoritative world.
    #[error("unknown player {0}")]
    UnknownPlayer(u64),

    /// Movement delta exceeds the per-tick speed limit.
    #[error("move too fast: distance {distance} > max {max}")]
    MoveTooFast {
        /// Actual movement magnitude.
        distance: i128,
        /// Maximum allowed.
        max: i128,
    },

    /// Target position is out of interaction range.
    #[error("target out of range: distance {distance} > max {max}")]
    OutOfRange {
        /// Actual distance.
        distance: i128,
        /// Maximum allowed.
        max: i128,
    },

    /// Invalid voxel type ID.
    #[error("invalid voxel type {0}")]
    InvalidVoxelType(u16),
}

// ---------------------------------------------------------------------------
// PlayerState (ECS component)
// ---------------------------------------------------------------------------

/// Server-side canonical state for a connected player.
#[derive(Debug, Clone, Component)]
pub struct PlayerState {
    /// Player identifier (matches login).
    pub player_id: u64,
    /// X position in millimeters.
    pub x: i64,
    /// Y position in millimeters.
    pub y: i64,
    /// Z position in millimeters.
    pub z: i64,
    /// Yaw in milliradians.
    pub yaw_mrad: i32,
    /// Pitch in milliradians.
    pub pitch_mrad: i32,
}

// ---------------------------------------------------------------------------
// AuthoritativeWorld
// ---------------------------------------------------------------------------

/// The server's canonical world state. Wraps a Bevy ECS [`World`] and
/// provides high-level operations for player management and intent
/// application.
pub struct AuthoritativeWorld {
    /// The ECS world holding all authoritative entities.
    world: World,
    /// Monotonically increasing tick counter.
    tick: u64,
}

impl AuthoritativeWorld {
    /// Creates a new empty authoritative world.
    pub fn new() -> Self {
        Self {
            world: World::new(),
            tick: 0,
        }
    }

    /// Returns the current tick number.
    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// Advances the tick counter by one.
    pub fn advance_tick(&mut self) {
        self.tick += 1;
    }

    /// Spawns a player entity with the given initial state. Returns the
    /// ECS [`Entity`] handle.
    pub fn spawn_player(&mut self, state: PlayerState) -> Entity {
        self.world.spawn(state).id()
    }

    /// Looks up a player's [`PlayerState`] by player ID.
    pub fn find_player(&self, player_id: u64) -> Option<&PlayerState> {
        // SAFETY: query requires &mut World in bevy 0.15 but we only read.
        // We cast away mutability; this is safe because we don't modify anything.
        let world_ptr = &self.world as *const World as *mut World;
        unsafe {
            let mut query = (*world_ptr).query::<&PlayerState>();
            query.iter(&*world_ptr).find(|ps| ps.player_id == player_id)
        }
    }

    /// Mutably looks up a player's [`PlayerState`] by player ID.
    pub fn find_player_mut(&mut self, player_id: u64) -> Option<&mut PlayerState> {
        let world_ptr = &mut self.world as *mut World;
        unsafe {
            let mut query = (*world_ptr).query::<&mut PlayerState>();
            for ps in query.iter_mut(&mut *world_ptr) {
                if ps.player_id == player_id {
                    return Some(ps.into_inner());
                }
            }
        }
        None
    }

    /// Returns a reference to the inner ECS [`World`].
    pub fn world(&self) -> &World {
        &self.world
    }

    /// Returns a mutable reference to the inner ECS [`World`].
    pub fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }

    /// Returns the number of player entities.
    pub fn player_count(&self) -> usize {
        let world_ptr = &self.world as *const World as *mut World;
        unsafe {
            let mut query = (*world_ptr).query::<&PlayerState>();
            query.iter(&*world_ptr).count()
        }
    }
}

impl Default for AuthoritativeWorld {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// IntentValidator
// ---------------------------------------------------------------------------

/// Validates [`ClientIntent`] messages against the [`AuthoritativeWorld`].
///
/// Each validation method checks constraints (speed limits, range, etc.)
/// and returns `Ok(())` if the intent is legal.
pub struct IntentValidator;

impl IntentValidator {
    /// Validates a single intent. Returns `Ok(())` if it should be applied.
    pub fn validate(
        intent: &ClientIntent,
        world: &AuthoritativeWorld,
    ) -> Result<(), IntentValidationError> {
        match intent {
            ClientIntent::Move {
                player_id,
                dx,
                dy,
                dz,
            } => {
                // Player must exist.
                if world.find_player(*player_id).is_none() {
                    return Err(IntentValidationError::UnknownPlayer(*player_id));
                }
                // Speed check: Euclidean distance of delta.
                let dist_sq = (*dx as i128).pow(2) + (*dy as i128).pow(2) + (*dz as i128).pow(2);
                let max_sq = MAX_MOVE_DISTANCE_MM.pow(2);
                if dist_sq > max_sq {
                    let dist = (dist_sq as f64).sqrt() as i128;
                    return Err(IntentValidationError::MoveTooFast {
                        distance: dist,
                        max: MAX_MOVE_DISTANCE_MM,
                    });
                }
                Ok(())
            }

            ClientIntent::PlaceVoxel {
                player_id,
                voxel_type,
                x,
                y,
                z,
            } => {
                let ps = world
                    .find_player(*player_id)
                    .ok_or(IntentValidationError::UnknownPlayer(*player_id))?;
                // Voxel type 0 (air) is invalid for placement.
                if *voxel_type == 0 {
                    return Err(IntentValidationError::InvalidVoxelType(0));
                }
                // Range check.
                Self::check_range(ps, *x, *y, *z)?;
                Ok(())
            }

            ClientIntent::BreakVoxel { player_id, x, y, z } => {
                let ps = world
                    .find_player(*player_id)
                    .ok_or(IntentValidationError::UnknownPlayer(*player_id))?;
                Self::check_range(ps, *x, *y, *z)?;
                Ok(())
            }

            ClientIntent::Interact {
                player_id,
                target_entity: _,
            } => {
                // Just verify the player exists; target entity checks would
                // require more world state.
                if world.find_player(*player_id).is_none() {
                    return Err(IntentValidationError::UnknownPlayer(*player_id));
                }
                Ok(())
            }

            ClientIntent::Rotate { player_id, .. } => {
                if world.find_player(*player_id).is_none() {
                    return Err(IntentValidationError::UnknownPlayer(*player_id));
                }
                Ok(())
            }
        }
    }

    /// Validates and applies the intent to the world. Returns `Ok(())` on
    /// success.
    pub fn validate_and_apply(
        intent: &ClientIntent,
        world: &mut AuthoritativeWorld,
    ) -> Result<(), IntentValidationError> {
        Self::validate(intent, world)?;
        Self::apply(intent, world);
        Ok(())
    }

    /// Applies a (pre-validated) intent to the authoritative world.
    fn apply(intent: &ClientIntent, world: &mut AuthoritativeWorld) {
        match intent {
            ClientIntent::Move {
                player_id,
                dx,
                dy,
                dz,
            } => {
                if let Some(ps) = world.find_player_mut(*player_id) {
                    ps.x = ps.x.saturating_add(*dx);
                    ps.y = ps.y.saturating_add(*dy);
                    ps.z = ps.z.saturating_add(*dz);
                }
            }
            ClientIntent::Rotate {
                player_id,
                yaw_mrad,
                pitch_mrad,
            } => {
                if let Some(ps) = world.find_player_mut(*player_id) {
                    ps.yaw_mrad = ps.yaw_mrad.wrapping_add(*yaw_mrad);
                    ps.pitch_mrad = ps.pitch_mrad.wrapping_add(*pitch_mrad);
                }
            }
            // PlaceVoxel / BreakVoxel / Interact would modify voxel or
            // entity state; placeholder for now (logged in demo).
            ClientIntent::PlaceVoxel { .. }
            | ClientIntent::BreakVoxel { .. }
            | ClientIntent::Interact { .. } => {
                tracing::debug!("Applied intent: {intent:?}");
            }
        }
    }

    /// Checks that a target position is within interaction range of the player.
    fn check_range(ps: &PlayerState, x: i64, y: i64, z: i64) -> Result<(), IntentValidationError> {
        let dx = (x as i128) - (ps.x as i128);
        let dy = (y as i128) - (ps.y as i128);
        let dz = (z as i128) - (ps.z as i128);
        let dist_sq = dx.pow(2) + dy.pow(2) + dz.pow(2);
        let max_sq = MAX_INTERACT_RANGE_MM.pow(2);
        if dist_sq > max_sq {
            let dist = (dist_sq as f64).sqrt() as i128;
            return Err(IntentValidationError::OutOfRange {
                distance: dist,
                max: MAX_INTERACT_RANGE_MM,
            });
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// ServerTickSchedule
// ---------------------------------------------------------------------------

/// Fixed-rate tick scheduler for the server simulation loop.
///
/// Accumulates real elapsed time and yields discrete ticks at
/// [`SERVER_TICK_RATE`] Hz (default 60 Hz).
pub struct ServerTickSchedule {
    accumulator_secs: f64,
    tick_duration_secs: f64,
    total_ticks: u64,
}

impl ServerTickSchedule {
    /// Creates a new schedule at the default 60 Hz tick rate.
    pub fn new() -> Self {
        Self {
            accumulator_secs: 0.0,
            tick_duration_secs: TICK_DURATION_SECS,
            total_ticks: 0,
        }
    }

    /// Creates a schedule with a custom tick rate.
    pub fn with_tick_rate(hz: u32) -> Self {
        Self {
            accumulator_secs: 0.0,
            tick_duration_secs: 1.0 / hz as f64,
            total_ticks: 0,
        }
    }

    /// Accumulates elapsed time and returns the number of ticks to process.
    pub fn accumulate(&mut self, dt_secs: f64) -> u32 {
        self.accumulator_secs += dt_secs;
        let mut ticks = 0u32;
        while self.accumulator_secs >= self.tick_duration_secs {
            self.accumulator_secs -= self.tick_duration_secs;
            self.total_ticks += 1;
            ticks += 1;
        }
        ticks
    }

    /// Returns the total number of ticks processed since creation.
    pub fn total_ticks(&self) -> u64 {
        self.total_ticks
    }

    /// Returns the tick duration in seconds.
    pub fn tick_duration_secs(&self) -> f64 {
        self.tick_duration_secs
    }
}

impl Default for ServerTickSchedule {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_intent_serialization_roundtrip() {
        let intents = vec![
            ClientIntent::Move {
                player_id: 1,
                dx: 100,
                dy: -50,
                dz: 0,
            },
            ClientIntent::PlaceVoxel {
                player_id: 2,
                voxel_type: 5,
                x: 1000,
                y: 2000,
                z: 3000,
            },
            ClientIntent::BreakVoxel {
                player_id: 3,
                x: -500,
                y: 100,
                z: 200,
            },
            ClientIntent::Interact {
                player_id: 4,
                target_entity: 99,
            },
            ClientIntent::Rotate {
                player_id: 5,
                yaw_mrad: 314,
                pitch_mrad: -157,
            },
        ];

        for intent in &intents {
            // postcard round-trip
            let bytes = postcard::to_allocvec(intent).expect("serialize");
            let decoded: ClientIntent = postcard::from_bytes(&bytes).expect("deserialize");
            assert_eq!(*intent, decoded);

            // serde_json round-trip (proves Serialize+Deserialize work generically)
            let json = serde_json::to_string(intent).expect("json serialize");
            let from_json: ClientIntent = serde_json::from_str(&json).expect("json deserialize");
            assert_eq!(*intent, from_json);
        }
    }

    #[test]
    fn test_authoritative_world_spawn_and_find() {
        let mut world = AuthoritativeWorld::new();
        assert_eq!(world.tick(), 0);
        assert_eq!(world.player_count(), 0);

        let entity = world.spawn_player(PlayerState {
            player_id: 42,
            x: 1000,
            y: 2000,
            z: 3000,
            yaw_mrad: 0,
            pitch_mrad: 0,
        });
        assert_eq!(world.player_count(), 1);

        let ps = world.find_player(42).expect("player 42 should exist");
        assert_eq!(ps.x, 1000);
        assert_eq!(ps.y, 2000);
        assert_eq!(ps.z, 3000);

        // Unknown player returns None.
        assert!(world.find_player(999).is_none());

        // Tick advances.
        world.advance_tick();
        assert_eq!(world.tick(), 1);

        // Entity handle is valid.
        assert!(world.world().get_entity(entity).is_ok());
    }

    #[test]
    fn test_intent_validator_rejects_speed_hack() {
        let mut world = AuthoritativeWorld::new();
        world.spawn_player(PlayerState {
            player_id: 1,
            x: 0,
            y: 0,
            z: 0,
            yaw_mrad: 0,
            pitch_mrad: 0,
        });

        // Legal move (within MAX_MOVE_DISTANCE_MM).
        let legal_move = ClientIntent::Move {
            player_id: 1,
            dx: 100,
            dy: 0,
            dz: 0,
        };
        assert!(IntentValidator::validate(&legal_move, &world).is_ok());

        // Illegal move (way too fast).
        let speed_hack = ClientIntent::Move {
            player_id: 1,
            dx: 10_000,
            dy: 10_000,
            dz: 10_000,
        };
        let err = IntentValidator::validate(&speed_hack, &world).unwrap_err();
        assert!(matches!(err, IntentValidationError::MoveTooFast { .. }));
    }

    #[test]
    fn test_intent_validator_rejects_out_of_range() {
        let mut world = AuthoritativeWorld::new();
        world.spawn_player(PlayerState {
            player_id: 1,
            x: 0,
            y: 0,
            z: 0,
            yaw_mrad: 0,
            pitch_mrad: 0,
        });

        // Place voxel within range.
        let near_place = ClientIntent::PlaceVoxel {
            player_id: 1,
            voxel_type: 1,
            x: 1000,
            y: 0,
            z: 0,
        };
        assert!(IntentValidator::validate(&near_place, &world).is_ok());

        // Place voxel far away.
        let far_place = ClientIntent::PlaceVoxel {
            player_id: 1,
            voxel_type: 1,
            x: 100_000,
            y: 0,
            z: 0,
        };
        let err = IntentValidator::validate(&far_place, &world).unwrap_err();
        assert!(matches!(err, IntentValidationError::OutOfRange { .. }));

        // Place air (voxel_type 0) is invalid.
        let air_place = ClientIntent::PlaceVoxel {
            player_id: 1,
            voxel_type: 0,
            x: 0,
            y: 0,
            z: 0,
        };
        let err = IntentValidator::validate(&air_place, &world).unwrap_err();
        assert!(matches!(err, IntentValidationError::InvalidVoxelType(0)));
    }

    #[test]
    fn test_server_tick_schedule_60hz() {
        // Fresh schedule: accumulate 60 individual ticks worth of time.
        let mut schedule = ServerTickSchedule::new();
        assert_eq!(schedule.total_ticks(), 0);

        // Feed exactly one tick duration 60 times → must yield 60 ticks total.
        for _ in 0..60 {
            let t = schedule.accumulate(TICK_DURATION_SECS);
            assert_eq!(t, 1, "each tick-duration step should yield exactly 1 tick");
        }
        assert_eq!(schedule.total_ticks(), 60);

        // Half-tick accumulations: two halves = one tick.
        let mut schedule2 = ServerTickSchedule::new();
        let t = schedule2.accumulate(TICK_DURATION_SECS * 0.4);
        assert_eq!(t, 0, "0.4 of a tick should not fire");
        let t = schedule2.accumulate(TICK_DURATION_SECS * 0.7);
        assert_eq!(t, 1, "0.4 + 0.7 = 1.1 ticks should fire once");
        assert_eq!(schedule2.total_ticks(), 1);

        // Custom tick rate.
        let mut schedule_30 = ServerTickSchedule::with_tick_rate(30);
        for _ in 0..30 {
            schedule_30.accumulate(1.0 / 30.0);
        }
        assert_eq!(schedule_30.total_ticks(), 30);
    }
}
