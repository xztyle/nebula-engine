//! Physics island management: spatial partitioning for bounded physics simulation.
//!
//! Only entities within the island radius have active Rapier rigid bodies.
//! Hysteresis prevents flickering at the boundary.

use std::collections::HashSet;

use bevy_ecs::prelude::*;
use glam::Vec3;
use nebula_math::WorldPosition;

/// Chunk coordinate (face, x, y, z). Simple tuple type for island tracking.
pub type ChunkCoord = (i64, i64, i64, u8);

/// Central physics island resource. Only entities within `radius` of `center`
/// get active Rapier rigid bodies. The island moves with the player.
#[derive(Resource)]
pub struct PhysicsIsland {
    /// Center of the island in world coordinates (i128 millimeters).
    pub center: WorldPosition,
    /// Radius in meters. Objects within this distance gain physics bodies.
    pub radius: f32,
    /// Hysteresis buffer in meters. Objects must exceed `radius + hysteresis` to lose bodies.
    pub hysteresis: f32,
    /// Entity IDs currently inside the island with active physics.
    pub active_entities: HashSet<Entity>,
    /// Chunk coordinates with active colliders.
    pub active_chunks: HashSet<ChunkCoord>,
}

impl Default for PhysicsIsland {
    fn default() -> Self {
        Self::new()
    }
}

impl PhysicsIsland {
    /// Creates a new physics island with default radius (512m) and hysteresis (16m).
    pub fn new() -> Self {
        Self {
            center: WorldPosition::default(),
            radius: 512.0,
            hysteresis: 16.0,
            active_entities: HashSet::new(),
            active_chunks: HashSet::new(),
        }
    }

    /// Sets the island radius and auto-scales hysteresis (3% of radius, min 8m).
    pub fn set_radius(&mut self, radius: f32) {
        self.radius = radius;
        self.hysteresis = (radius * 0.03).max(8.0);
    }

    /// Computes the distance in meters between two world positions.
    ///
    /// Subtracts in i128 space, then converts to f64 for the sqrt.
    /// Units: WorldPosition is in millimeters, result is in meters.
    pub fn distance_meters(a: &WorldPosition, b: &WorldPosition) -> f64 {
        let dx = (a.x - b.x) as f64;
        let dy = (a.y - b.y) as f64;
        let dz = (a.z - b.z) as f64;
        (dx * dx + dy * dy + dz * dz).sqrt() / 1000.0 // mm → meters
    }

    /// Returns true if `distance` is within the enter threshold (radius).
    pub fn should_enter(&self, distance_m: f64) -> bool {
        distance_m <= self.radius as f64
    }

    /// Returns true if `distance` is beyond the leave threshold (radius + hysteresis).
    pub fn should_leave(&self, distance_m: f64) -> bool {
        distance_m > (self.radius + self.hysteresis) as f64
    }
}

/// Cached physics state for entities that leave the island.
///
/// Stores velocity and sleep state so the rigid body can be reconstructed
/// seamlessly when the entity re-enters the island.
#[derive(Component, Clone, Debug)]
pub struct FrozenPhysicsState {
    /// Linear velocity at the time the body was removed (m/s).
    pub linear_velocity: Vec3,
    /// Angular velocity at the time the body was removed (rad/s).
    pub angular_velocity: Vec3,
    /// Whether the body was sleeping when removed.
    pub was_sleeping: bool,
}

impl Default for FrozenPhysicsState {
    fn default() -> Self {
        Self {
            linear_velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            was_sleeping: false,
        }
    }
}

/// Marker component for the player entity (used by the island update system).
#[derive(Component)]
pub struct IslandPlayer;

/// Component that stores a Rapier rigid body handle on an ECS entity.
#[derive(Component)]
pub struct RigidBodyHandle(pub rapier3d::prelude::RigidBodyHandle);

/// Component tagging an entity as physics-eligible (needs a WorldPosition too).
#[derive(Component)]
pub struct PhysicsEligible;

/// System that updates the physics island each tick.
///
/// Moves the island center to the player position, then activates/deactivates
/// rigid bodies for entities entering/leaving the island radius.
#[allow(clippy::type_complexity)]
pub fn physics_island_update_system(
    mut island: ResMut<PhysicsIsland>,
    mut physics: ResMut<crate::PhysicsWorld>,
    mut commands: Commands,
    player_query: Query<&IslandWorldPos, With<IslandPlayer>>,
    mut entity_query: Query<
        (
            Entity,
            &IslandWorldPos,
            Option<&RigidBodyHandle>,
            Option<&FrozenPhysicsState>,
        ),
        Without<IslandPlayer>,
    >,
) {
    // Update island center to player position.
    if let Some(player_pos) = player_query.iter().next() {
        island.center = player_pos.0;
    }

    let mut to_add = Vec::new();
    let mut to_remove = Vec::new();

    for (entity, world_pos, body_handle, frozen) in entity_query.iter_mut() {
        let distance = PhysicsIsland::distance_meters(&world_pos.0, &island.center);

        if island.should_enter(distance) && body_handle.is_none() {
            to_add.push((entity, world_pos.0, frozen.cloned()));
        } else if island.should_leave(distance) && body_handle.is_some() {
            to_remove.push(entity);
        }
    }

    // Add bodies for entities entering the island.
    for (entity, world_pos, frozen) in to_add {
        let local = world_pos.to_local_f32(&island.center);
        let local_m = local / 1000.0; // mm → meters

        let mut builder = rapier3d::prelude::RigidBodyBuilder::dynamic().translation(
            rapier3d::prelude::Vector::new(local_m.x, local_m.y, local_m.z),
        );

        // Restore frozen state if available.
        if let Some(ref frozen_state) = frozen {
            builder = builder
                .linvel(rapier3d::prelude::Vector::new(
                    frozen_state.linear_velocity.x,
                    frozen_state.linear_velocity.y,
                    frozen_state.linear_velocity.z,
                ))
                .angvel(rapier3d::prelude::Vector::new(
                    frozen_state.angular_velocity.x,
                    frozen_state.angular_velocity.y,
                    frozen_state.angular_velocity.z,
                ));
            if frozen_state.was_sleeping {
                builder = builder.sleeping(true);
            }
        }

        let body = builder.build();
        let handle = physics.rigid_body_set.insert(body);
        commands
            .entity(entity)
            .insert(RigidBodyHandle(handle))
            .remove::<FrozenPhysicsState>();
        island.active_entities.insert(entity);

        tracing::trace!("Physics island: activated body for entity {:?}", entity);
    }

    // Remove bodies for entities leaving the island.
    for entity in to_remove {
        if let Some(handle) = entity_query.get(entity).ok().and_then(|q| q.2) {
            // Cache velocity before removing.
            let rb = &physics.rigid_body_set[handle.0];
            let linvel = rb.linvel();
            let angvel = rb.angvel();
            let sleeping = rb.is_sleeping();

            let frozen = FrozenPhysicsState {
                linear_velocity: Vec3::new(linvel.x, linvel.y, linvel.z),
                angular_velocity: Vec3::new(angvel.x, angvel.y, angvel.z),
                was_sleeping: sleeping,
            };

            let phys = &mut *physics;
            phys.rigid_body_set.remove(
                handle.0,
                &mut phys.island_manager,
                &mut phys.collider_set,
                &mut phys.impulse_joint_set,
                &mut phys.multibody_joint_set,
                true,
            );

            commands
                .entity(entity)
                .insert(frozen)
                .remove::<RigidBodyHandle>();
            island.active_entities.remove(&entity);

            tracing::trace!("Physics island: deactivated body for entity {:?}", entity);
        }
    }
}

/// WorldPosition wrapper component used by the island system.
///
/// This avoids a hard dependency on nebula-ecs's `WorldPos` component,
/// allowing nebula-physics to remain low in the dependency graph.
#[derive(Component, Clone, Copy, Debug)]
pub struct IslandWorldPos(pub WorldPosition);

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: set up a minimal ECS world with PhysicsIsland and PhysicsWorld.
    fn setup_world() -> (bevy_ecs::world::World, bevy_ecs::schedule::Schedule) {
        let mut world = bevy_ecs::world::World::new();
        world.insert_resource(PhysicsIsland::new());
        world.insert_resource(crate::PhysicsWorld::new());

        let mut schedule = bevy_ecs::schedule::Schedule::default();
        schedule.add_systems(physics_island_update_system);

        (world, schedule)
    }

    /// Spawn a player entity at the given position (in millimeters).
    fn spawn_player(world: &mut bevy_ecs::world::World, x: i128, y: i128, z: i128) -> Entity {
        world
            .spawn((IslandPlayer, IslandWorldPos(WorldPosition::new(x, y, z))))
            .id()
    }

    /// Spawn a physics-eligible entity at the given position (in millimeters).
    fn spawn_entity(world: &mut bevy_ecs::world::World, x: i128, y: i128, z: i128) -> Entity {
        world
            .spawn(IslandWorldPos(WorldPosition::new(x, y, z)))
            .id()
    }

    #[test]
    fn test_object_inside_island_has_body() {
        let (mut world, mut schedule) = setup_world();
        spawn_player(&mut world, 0, 0, 0);
        // 100m from origin in mm
        let entity = spawn_entity(&mut world, 100_000, 0, 100_000);

        schedule.run(&mut world);

        assert!(world.get::<RigidBodyHandle>(entity).is_some());
        let physics = world.resource::<crate::PhysicsWorld>();
        assert!(!physics.rigid_body_set.is_empty());
    }

    #[test]
    fn test_object_outside_island_has_no_body() {
        let (mut world, mut schedule) = setup_world();
        spawn_player(&mut world, 0, 0, 0);
        // ~1414m from origin in mm
        let entity = spawn_entity(&mut world, 1_000_000, 0, 1_000_000);

        schedule.run(&mut world);

        assert!(world.get::<RigidBodyHandle>(entity).is_none());
        let physics = world.resource::<crate::PhysicsWorld>();
        assert_eq!(physics.rigid_body_set.len(), 0);
    }

    #[test]
    fn test_object_crossing_boundary_gains_body() {
        let (mut world, mut schedule) = setup_world();
        spawn_player(&mut world, 0, 0, 0);
        // 600m — outside
        let entity = spawn_entity(&mut world, 600_000, 0, 0);

        schedule.run(&mut world);
        assert!(world.get::<RigidBodyHandle>(entity).is_none());

        // Move to 400m — inside
        world.get_mut::<IslandWorldPos>(entity).unwrap().0 = WorldPosition::new(400_000, 0, 0);
        schedule.run(&mut world);
        assert!(world.get::<RigidBodyHandle>(entity).is_some());
    }

    #[test]
    fn test_object_crossing_boundary_loses_body() {
        let (mut world, mut schedule) = setup_world();
        spawn_player(&mut world, 0, 0, 0);
        // 400m — inside
        let entity = spawn_entity(&mut world, 400_000, 0, 0);

        schedule.run(&mut world);
        assert!(world.get::<RigidBodyHandle>(entity).is_some());

        // Move beyond radius + hysteresis (512 + 16 = 528m) → 600m
        world.get_mut::<IslandWorldPos>(entity).unwrap().0 = WorldPosition::new(600_000, 0, 0);
        schedule.run(&mut world);
        assert!(world.get::<RigidBodyHandle>(entity).is_none());
    }

    #[test]
    fn test_island_moves_with_player() {
        let (mut world, mut schedule) = setup_world();
        let player = spawn_player(&mut world, 0, 0, 0);
        // Entity at (10100m, 0, 0) — far from origin
        let entity = spawn_entity(&mut world, 10_100_000, 0, 0);

        schedule.run(&mut world);
        assert!(world.get::<RigidBodyHandle>(entity).is_none());

        // Move player to (10000m, 0, 0) — entity is now 100m away
        world.get_mut::<IslandWorldPos>(player).unwrap().0 = WorldPosition::new(10_000_000, 0, 0);
        schedule.run(&mut world);

        let island = world.resource::<PhysicsIsland>();
        assert_eq!(island.center, WorldPosition::new(10_000_000, 0, 0));
        assert!(world.get::<RigidBodyHandle>(entity).is_some());
    }

    #[test]
    fn test_island_radius_configurable() {
        let (mut world, mut schedule) = setup_world();
        spawn_player(&mut world, 0, 0, 0);
        // Entity at 300m
        let entity = spawn_entity(&mut world, 300_000, 0, 0);

        // Set radius to 256m — entity at 300m is outside
        world.resource_mut::<PhysicsIsland>().set_radius(256.0);
        schedule.run(&mut world);
        assert!(world.get::<RigidBodyHandle>(entity).is_none());

        let island = world.resource::<PhysicsIsland>();
        assert_eq!(island.radius, 256.0);
        assert_eq!(island.hysteresis, 8.0); // 256 * 0.03 = 7.68, max(7.68, 8.0) = 8.0

        // Set radius to 512m — entity at 300m is inside
        world.resource_mut::<PhysicsIsland>().set_radius(512.0);
        schedule.run(&mut world);
        assert!(world.get::<RigidBodyHandle>(entity).is_some());
    }

    #[test]
    fn test_hysteresis_prevents_flicker() {
        let (mut world, mut schedule) = setup_world();
        spawn_player(&mut world, 0, 0, 0);
        // Entity at exactly 512m (radius boundary)
        let entity = spawn_entity(&mut world, 512_000, 0, 0);

        schedule.run(&mut world);
        assert!(world.get::<RigidBodyHandle>(entity).is_some());

        // Move to 520m — inside hysteresis band (512 < 520 < 528), body should stay
        world.get_mut::<IslandWorldPos>(entity).unwrap().0 = WorldPosition::new(520_000, 0, 0);
        schedule.run(&mut world);
        assert!(
            world.get::<RigidBodyHandle>(entity).is_some(),
            "Body should persist in hysteresis band"
        );

        // Move to 530m — beyond hysteresis (512 + 16 = 528), body should be removed
        world.get_mut::<IslandWorldPos>(entity).unwrap().0 = WorldPosition::new(530_000, 0, 0);
        schedule.run(&mut world);
        assert!(
            world.get::<RigidBodyHandle>(entity).is_none(),
            "Body should be removed beyond hysteresis"
        );
    }
}
