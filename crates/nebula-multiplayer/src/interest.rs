//! Spatial interest management: determines which entities are relevant to
//! each connected client based on proximity, and produces spawn/despawn
//! transitions when entities enter or leave a client's interest area.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::replication::NetworkId;

// ---------------------------------------------------------------------------
// InterestArea
// ---------------------------------------------------------------------------

/// Defines the spherical region around a client within which entities are
/// considered relevant. The radius is in meters (f64).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterestArea {
    /// Radius of the interest sphere in meters.
    pub radius: f64,
}

impl Default for InterestArea {
    fn default() -> Self {
        Self { radius: 500.0 }
    }
}

// ---------------------------------------------------------------------------
// ClientInterestSet
// ---------------------------------------------------------------------------

/// Tracks which [`NetworkId`]s are currently within a client's interest area,
/// along with the previous tick's set for computing transitions.
#[derive(Debug, Clone)]
pub struct ClientInterestSet {
    /// Client identifier.
    pub client_id: u64,
    /// Entity network IDs inside the interest area this tick.
    pub current: HashSet<NetworkId>,
    /// Entity network IDs that were inside the interest area last tick.
    pub previous: HashSet<NetworkId>,
}

impl ClientInterestSet {
    /// Creates a new, empty interest set for the given client.
    pub fn new(client_id: u64) -> Self {
        Self {
            client_id,
            current: HashSet::new(),
            previous: HashSet::new(),
        }
    }

    /// Computes which entities entered and exited the interest area between
    /// the previous and current tick.
    pub fn compute_transitions(&self) -> InterestTransitions {
        InterestTransitions {
            entered: self.current.difference(&self.previous).copied().collect(),
            exited: self.previous.difference(&self.current).copied().collect(),
        }
    }

    /// Advances the tick: copies `current` into `previous` and clears
    /// `current` for the next evaluation pass.
    pub fn advance(&mut self) {
        self.previous.clone_from(&self.current);
        self.current.clear();
    }
}

// ---------------------------------------------------------------------------
// InterestTransitions
// ---------------------------------------------------------------------------

/// The set of entities that entered or exited a client's interest area
/// during a single tick.
#[derive(Debug, Clone, Default)]
pub struct InterestTransitions {
    /// Entities that just entered the interest area (need `SpawnEntity`).
    pub entered: HashSet<NetworkId>,
    /// Entities that just left the interest area (need `DespawnEntity`).
    pub exited: HashSet<NetworkId>,
}

// ---------------------------------------------------------------------------
// Position helper
// ---------------------------------------------------------------------------

/// A simple 3D position used for interest distance checks. In a full engine
/// integration these would be derived from `Coord128` / `WorldPosition`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InterestPosition {
    /// X coordinate in meters (f64).
    pub x: f64,
    /// Y coordinate in meters (f64).
    pub y: f64,
    /// Z coordinate in meters (f64).
    pub z: f64,
}

impl InterestPosition {
    /// Creates a new position.
    pub fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }
}

/// Returns `true` if positions `a` and `b` are within `radius` meters of
/// each other. Uses squared-distance comparison to avoid a square root.
pub fn within_interest(a: &InterestPosition, b: &InterestPosition, radius: f64) -> bool {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    let dz = a.z - b.z;
    let dist_sq = dx * dx + dy * dy + dz * dz;
    dist_sq <= radius * radius
}

// ---------------------------------------------------------------------------
// SpatialInterestSystem
// ---------------------------------------------------------------------------

/// An entity known to the interest system: its network ID and current position.
#[derive(Debug, Clone)]
pub struct TrackedEntity {
    /// The entity's network identifier.
    pub network_id: NetworkId,
    /// Current position in world space (meters).
    pub position: InterestPosition,
}

/// Server-side system that evaluates spatial interest for all connected
/// clients each tick. For each client it determines which entities fall
/// within the client's [`InterestArea`] and computes [`InterestTransitions`].
#[derive(Debug, Default)]
pub struct SpatialInterestSystem {
    /// Per-client interest tracking, keyed by client ID.
    interest_sets: Vec<(u64, ClientInterestSet, InterestArea, InterestPosition)>,
}

impl SpatialInterestSystem {
    /// Creates an empty spatial interest system.
    pub fn new() -> Self {
        Self {
            interest_sets: Vec::new(),
        }
    }

    /// Registers a client with a given interest area and initial position.
    pub fn add_client(&mut self, client_id: u64, area: InterestArea, position: InterestPosition) {
        self.interest_sets
            .push((client_id, ClientInterestSet::new(client_id), area, position));
    }

    /// Removes a client from interest tracking.
    pub fn remove_client(&mut self, client_id: u64) {
        self.interest_sets.retain(|(id, _, _, _)| *id != client_id);
    }

    /// Updates a client's position.
    pub fn set_client_position(&mut self, client_id: u64, position: InterestPosition) {
        if let Some(entry) = self
            .interest_sets
            .iter_mut()
            .find(|(id, _, _, _)| *id == client_id)
        {
            entry.3 = position;
        }
    }

    /// Updates a client's interest area radius.
    pub fn set_client_radius(&mut self, client_id: u64, radius: f64) {
        if let Some(entry) = self
            .interest_sets
            .iter_mut()
            .find(|(id, _, _, _)| *id == client_id)
        {
            entry.2.radius = radius;
        }
    }

    /// Runs one interest evaluation tick. For each client, determines which
    /// of the given `entities` are within range and returns per-client
    /// [`InterestTransitions`].
    pub fn evaluate(&mut self, entities: &[TrackedEntity]) -> Vec<(u64, InterestTransitions)> {
        let mut results = Vec::new();

        for (client_id, interest_set, area, client_pos) in &mut self.interest_sets {
            // Advance: move current → previous, clear current.
            interest_set.advance();

            // Populate current set with entities within range.
            for entity in entities {
                if within_interest(client_pos, &entity.position, area.radius) {
                    interest_set.current.insert(entity.network_id);
                }
            }

            let transitions = interest_set.compute_transitions();
            results.push((*client_id, transitions));
        }

        results
    }

    /// Returns the current interest set for a client, if registered.
    pub fn interest_set(&self, client_id: u64) -> Option<&ClientInterestSet> {
        self.interest_sets
            .iter()
            .find(|(id, _, _, _)| *id == client_id)
            .map(|(_, set, _, _)| set)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn origin() -> InterestPosition {
        InterestPosition::new(0.0, 0.0, 0.0)
    }

    #[test]
    fn test_entity_inside_area_is_replicated() {
        let mut sys = SpatialInterestSystem::new();
        sys.add_client(1, InterestArea { radius: 500.0 }, origin());

        let entities = vec![TrackedEntity {
            network_id: NetworkId(10),
            position: InterestPosition::new(100.0, 0.0, 0.0),
        }];

        let results = sys.evaluate(&entities);
        assert_eq!(results.len(), 1);

        let (client_id, transitions) = &results[0];
        assert_eq!(*client_id, 1);
        assert!(transitions.entered.contains(&NetworkId(10)));
        assert!(transitions.exited.is_empty());

        // Verify interest set contains the entity.
        let set = sys.interest_set(1).unwrap();
        assert!(set.current.contains(&NetworkId(10)));
    }

    #[test]
    fn test_entity_outside_area_is_not_replicated() {
        let mut sys = SpatialInterestSystem::new();
        sys.add_client(1, InterestArea { radius: 500.0 }, origin());

        let entities = vec![TrackedEntity {
            network_id: NetworkId(20),
            position: InterestPosition::new(1000.0, 0.0, 0.0),
        }];

        let results = sys.evaluate(&entities);
        let (_, transitions) = &results[0];
        assert!(transitions.entered.is_empty());
        assert!(transitions.exited.is_empty());

        let set = sys.interest_set(1).unwrap();
        assert!(!set.current.contains(&NetworkId(20)));
    }

    #[test]
    fn test_entity_entering_area_triggers_full_state_send() {
        let mut sys = SpatialInterestSystem::new();
        sys.add_client(1, InterestArea { radius: 500.0 }, origin());

        // Tick 1: entity at 600m (outside).
        let entities_far = vec![TrackedEntity {
            network_id: NetworkId(30),
            position: InterestPosition::new(600.0, 0.0, 0.0),
        }];
        let r1 = sys.evaluate(&entities_far);
        assert!(r1[0].1.entered.is_empty());

        // Tick 2: entity moves to 400m (inside).
        let entities_near = vec![TrackedEntity {
            network_id: NetworkId(30),
            position: InterestPosition::new(400.0, 0.0, 0.0),
        }];
        let r2 = sys.evaluate(&entities_near);
        assert!(r2[0].1.entered.contains(&NetworkId(30)));
    }

    #[test]
    fn test_entity_leaving_area_triggers_despawn() {
        let mut sys = SpatialInterestSystem::new();
        sys.add_client(1, InterestArea { radius: 500.0 }, origin());

        // Tick 1: entity at 400m (inside).
        let entities_near = vec![TrackedEntity {
            network_id: NetworkId(40),
            position: InterestPosition::new(400.0, 0.0, 0.0),
        }];
        let r1 = sys.evaluate(&entities_near);
        assert!(r1[0].1.entered.contains(&NetworkId(40)));

        // Tick 2: entity moves to 600m (outside).
        let entities_far = vec![TrackedEntity {
            network_id: NetworkId(40),
            position: InterestPosition::new(600.0, 0.0, 0.0),
        }];
        let r2 = sys.evaluate(&entities_far);
        assert!(r2[0].1.exited.contains(&NetworkId(40)));
    }

    #[test]
    fn test_interest_radius_is_configurable() {
        let mut sys = SpatialInterestSystem::new();
        sys.add_client(1, InterestArea { radius: 200.0 }, origin());

        let entities = vec![TrackedEntity {
            network_id: NetworkId(50),
            position: InterestPosition::new(300.0, 0.0, 0.0),
        }];

        // With radius 200, entity at 300m is outside.
        let r1 = sys.evaluate(&entities);
        assert!(r1[0].1.entered.is_empty());

        // Change radius to 400m.
        sys.set_client_radius(1, 400.0);

        let r2 = sys.evaluate(&entities);
        assert!(r2[0].1.entered.contains(&NetworkId(50)));
    }
}
