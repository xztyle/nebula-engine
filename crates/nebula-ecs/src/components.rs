//! Core ECS components shared across all engine subsystems.
//!
//! These components bridge the engine's foundational math types
//! (`WorldPosition`, `LocalPosition`, `Vec3I128` from `nebula_math`)
//! with the ECS, making them query-able, change-detectable, and
//! automatically parallel-safe.

use bevy_ecs::prelude::*;
use glam::Quat;
use nebula_math::{LocalPosition, Vec3I128, WorldPosition};

/// The entity's canonical position in the universe, in 128-bit integer
/// coordinates. Each unit is 1 millimeter. This is the source of truth
/// for where an entity exists — all other position representations
/// (LocalPos, GPU transforms) are derived from this.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct WorldPos(pub WorldPosition);

impl WorldPos {
    /// Creates a new [`WorldPos`] from integer millimeter coordinates.
    pub fn new(x: i128, y: i128, z: i128) -> Self {
        Self(WorldPosition::new(x, y, z))
    }
}

/// Camera-relative position in f32 space. Computed each frame by
/// subtracting the camera's WorldPosition from the entity's WorldPosition
/// and converting the result to f32. Used by the renderer and any system
/// that needs GPU-compatible coordinates.
///
/// This component is written by the PostUpdate stage and should be
/// treated as read-only by gameplay systems.
#[derive(Component, Clone, Copy, Debug, PartialEq, Default)]
pub struct LocalPos(pub LocalPosition);

impl LocalPos {
    /// Creates a new [`LocalPos`] from f32 coordinates.
    pub fn new(x: f32, y: f32, z: f32) -> Self {
        Self(LocalPosition::new(x, y, z))
    }
}

/// Movement per simulation tick in 128-bit integer units (millimeters
/// per tick). Applied to WorldPos during FixedUpdate. Using i128 ensures
/// velocity accumulation never loses precision at any scale.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Velocity(pub Vec3I128);

impl Velocity {
    /// Creates a new [`Velocity`] from integer millimeter-per-tick values.
    pub fn new(x: i128, y: i128, z: i128) -> Self {
        Self(Vec3I128::new(x, y, z))
    }
}

/// Orientation as a unit quaternion. Uses [`glam::Quat`] for compatibility
/// with the rendering pipeline and standard 3D math operations (slerp,
/// rotation composition, direction extraction).
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Rotation(pub Quat);

impl Default for Rotation {
    fn default() -> Self {
        Self(Quat::IDENTITY)
    }
}

/// Uniform scale factor. 1.0 is the default (no scaling). Multiplied
/// into the entity's model matrix during transform propagation.
/// Non-uniform scale is intentionally not supported at the core level
/// to avoid complications with physics and normal transformations.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Scale(pub f32);

impl Default for Scale {
    fn default() -> Self {
        Self(1.0)
    }
}

/// Human-readable debug name for an entity. Used in the editor,
/// debug overlays, and log messages. Not used for gameplay logic —
/// entities should be identified by their Entity ID or marker components.
#[derive(Component, Clone, Debug, PartialEq, Eq, Default)]
pub struct Name(pub String);

impl Name {
    /// Creates a new [`Name`] from anything that converts to `String`.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }
}

/// Whether the entity is enabled and should be processed by systems.
/// Inactive entities are skipped by physics, rendering, and gameplay
/// systems. This provides a lightweight alternative to despawning and
/// respawning when an entity needs to be temporarily disabled.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Active(pub bool);

impl Default for Active {
    fn default() -> Self {
        Self(true)
    }
}

/// Bundle grouping the most common spatial components for convenience.
#[derive(Bundle, Default)]
pub struct SpatialBundle {
    /// World-space position.
    pub world_pos: WorldPos,
    /// Camera-relative position.
    pub local_pos: LocalPos,
    /// Movement per tick.
    pub velocity: Velocity,
    /// Orientation.
    pub rotation: Rotation,
    /// Uniform scale.
    pub scale: Scale,
    /// Whether the entity is active.
    pub active: Active,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_worldpos_can_be_added_to_entity() {
        let mut world = World::new();
        let entity = world.spawn(WorldPos::new(100, 200, 300)).id();
        let pos = world.get::<WorldPos>(entity).unwrap();
        assert_eq!(pos.0, WorldPosition::new(100, 200, 300));
    }

    #[test]
    fn test_localpos_can_be_added_to_entity() {
        let mut world = World::new();
        let entity = world.spawn(LocalPos::new(1.0, 2.0, 3.0)).id();
        let pos = world.get::<LocalPos>(entity).unwrap();
        assert_eq!(pos.0, LocalPosition::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn test_velocity_can_be_added_to_entity() {
        let mut world = World::new();
        let entity = world.spawn(Velocity::new(10, -20, 30)).id();
        let vel = world.get::<Velocity>(entity).unwrap();
        assert_eq!(vel.0, Vec3I128::new(10, -20, 30));
    }

    #[test]
    fn test_rotation_can_be_added_to_entity() {
        let mut world = World::new();
        let rot = Rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2));
        let entity = world.spawn(rot).id();
        let r = world.get::<Rotation>(entity).unwrap();
        assert_eq!(r.0, rot.0);
    }

    #[test]
    fn test_scale_can_be_added_to_entity() {
        let mut world = World::new();
        let entity = world.spawn(Scale(2.5)).id();
        let s = world.get::<Scale>(entity).unwrap();
        assert_eq!(s.0, 2.5);
    }

    #[test]
    fn test_name_can_be_added_to_entity() {
        let mut world = World::new();
        let entity = world.spawn(Name::new("player_1")).id();
        let n = world.get::<Name>(entity).unwrap();
        assert_eq!(n.0, "player_1");
    }

    #[test]
    fn test_active_can_be_added_to_entity() {
        let mut world = World::new();
        let entity = world.spawn(Active(false)).id();
        let a = world.get::<Active>(entity).unwrap();
        assert!(!a.0);
    }

    #[test]
    fn test_query_by_component_type() {
        let mut world = World::new();
        world.spawn((WorldPos::new(1, 2, 3), Velocity::new(10, 0, 0)));
        world.spawn(WorldPos::new(4, 5, 6));
        world.spawn(Velocity::new(0, 0, 5));

        let mut query = world.query::<(&WorldPos, &Velocity)>();
        let results: Vec<_> = query.iter(&world).collect();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0.0, WorldPosition::new(1, 2, 3));
        assert_eq!(results[0].1.0, Vec3I128::new(10, 0, 0));
    }

    #[test]
    fn test_components_store_correct_values() {
        let mut world = World::new();
        let entity = world
            .spawn((
                WorldPos::new(i128::MAX, i128::MIN, 0),
                Velocity::new(-1, -1, -1),
                Scale(0.001),
                Name::new("extreme_entity"),
                Active(true),
            ))
            .id();

        assert_eq!(world.get::<WorldPos>(entity).unwrap().0.x, i128::MAX);
        assert_eq!(world.get::<WorldPos>(entity).unwrap().0.y, i128::MIN);
        assert_eq!(world.get::<Velocity>(entity).unwrap().0.x, -1);
        assert_eq!(world.get::<Scale>(entity).unwrap().0, 0.001);
        assert_eq!(world.get::<Name>(entity).unwrap().0, "extreme_entity");
        assert!(world.get::<Active>(entity).unwrap().0);
    }

    #[test]
    fn test_default_worldpos_is_origin() {
        let pos = WorldPos::default();
        assert_eq!(pos.0, WorldPosition::default());
    }

    #[test]
    fn test_default_velocity_is_zero() {
        let vel = Velocity::default();
        assert_eq!(vel.0, Vec3I128::new(0, 0, 0));
    }

    #[test]
    fn test_default_rotation_is_identity() {
        let rot = Rotation::default();
        assert_eq!(rot.0, Quat::IDENTITY);
    }

    #[test]
    fn test_default_scale_is_one() {
        let s = Scale::default();
        assert_eq!(s.0, 1.0);
    }

    #[test]
    fn test_default_active_is_true() {
        let a = Active::default();
        assert!(a.0);
    }

    #[test]
    fn test_default_name_is_empty() {
        let n = Name::default();
        assert_eq!(n.0, "");
    }

    #[test]
    fn test_spatial_bundle_spawns_all_components() {
        let mut world = World::new();
        let entity = world.spawn(SpatialBundle::default()).id();
        assert!(world.get::<WorldPos>(entity).is_some());
        assert!(world.get::<LocalPos>(entity).is_some());
        assert!(world.get::<Velocity>(entity).is_some());
        assert!(world.get::<Rotation>(entity).is_some());
        assert!(world.get::<Scale>(entity).is_some());
        assert!(world.get::<Active>(entity).is_some());
    }
}
