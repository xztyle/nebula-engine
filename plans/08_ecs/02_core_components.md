# Core Components

## Problem

Every entity in the Nebula Engine needs a standard set of properties: where it is in the universe (128-bit position), where it is relative to the camera (f32 position for GPU consumption), how fast it is moving, which way it faces, how big it is, what it is called, and whether it is active. Without a canonical set of core components, each subsystem invents its own position type, its own velocity representation, and its own "is this thing enabled" flag. This leads to redundant data, inconsistent naming, and systems that cannot interoperate because they query different component types for the same concept.

These components bridge the engine's foundational math types (`WorldPosition`, `LocalPosition`, `Vec3I128` from `nebula_math`) with the ECS, making them query-able, change-detectable, and automatically parallel-safe. They form the shared vocabulary that physics, rendering, gameplay, and networking all speak.

## Solution

Define the following component types in the `nebula_ecs` crate, each deriving `bevy_ecs::component::Component`:

### WorldPos

```rust
use bevy_ecs::prelude::*;
use nebula_math::{WorldPosition, LocalPosition, Vec3I128};
use glam::Quat;

/// The entity's canonical position in the universe, in 128-bit integer
/// coordinates. Each unit is 1 millimeter. This is the source of truth
/// for where an entity exists — all other position representations
/// (LocalPos, GPU transforms) are derived from this.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct WorldPos(pub WorldPosition);

impl WorldPos {
    pub fn new(x: i128, y: i128, z: i128) -> Self {
        Self(WorldPosition::new(x, y, z))
    }
}
```

### LocalPos

```rust
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
    pub fn new(x: f32, y: f32, z: f32) -> Self {
        Self(LocalPosition::new(x, y, z))
    }
}
```

### Velocity

```rust
/// Movement per simulation tick in 128-bit integer units (millimeters
/// per tick). Applied to WorldPos during FixedUpdate. Using i128 ensures
/// velocity accumulation never loses precision at any scale.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Velocity(pub Vec3I128);

impl Velocity {
    pub fn new(x: i128, y: i128, z: i128) -> Self {
        Self(Vec3I128::new(x, y, z))
    }
}
```

### Rotation

```rust
/// Orientation as a unit quaternion. Uses glam::Quat for compatibility
/// with the rendering pipeline and standard 3D math operations (slerp,
/// rotation composition, direction extraction).
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Rotation(pub Quat);

impl Default for Rotation {
    fn default() -> Self {
        Self(Quat::IDENTITY)
    }
}
```

### Scale

```rust
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
```

### Name

```rust
/// Human-readable debug name for an entity. Used in the editor,
/// debug overlays, and log messages. Not used for gameplay logic —
/// entities should be identified by their Entity ID or marker components.
#[derive(Component, Clone, Debug, PartialEq, Eq, Default)]
pub struct Name(pub String);

impl Name {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }
}
```

### Active

```rust
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
```

### Component Bundle

For convenience, provide a bundle that groups the most common components:

```rust
#[derive(Bundle, Default)]
pub struct SpatialBundle {
    pub world_pos: WorldPos,
    pub local_pos: LocalPos,
    pub velocity: Velocity,
    pub rotation: Rotation,
    pub scale: Scale,
    pub active: Active,
}
```

### Design Notes

- `WorldPos`, `Velocity` use `Eq` and `Hash` (integer types). `LocalPos`, `Rotation`, `Scale` use only `PartialEq` (floating-point types).
- `Rotation` defaults to `Quat::IDENTITY` (no rotation), not `Quat::default()` which is also identity but the explicit choice documents intent.
- `Scale` defaults to `1.0`, not `0.0`, because a zero-scale entity would be invisible and cause division-by-zero in inverse transforms.
- `Active` defaults to `true` because most entities should be active when spawned.
- `Name` defaults to an empty string. Systems that display names should handle the empty case gracefully.

## Outcome

After this story is complete:

- Every subsystem in the engine can import `WorldPos`, `LocalPos`, `Velocity`, `Rotation`, `Scale`, `Name`, and `Active` from `nebula_ecs`
- Entities can be spawned with a `SpatialBundle` for common spatial data
- All components are compatible with `bevy_ecs` queries, change detection, and parallel scheduling
- The type system enforces the distinction between world-space (`WorldPos`) and camera-space (`LocalPos`) positions
- Default values are sensible: origin position, zero velocity, identity rotation, unit scale, active, empty name

## Demo Integration

**Demo crate:** `nebula-demo`

The camera, planet, and chunks are now ECS entities with `Transform`, `Visibility`, and `Name` components. The title shows `Entities: 26` (camera + planet + chunks).

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.15` | `Component` derive, `Bundle` derive, ECS integration |
| `nebula-math` | workspace | `WorldPosition`, `LocalPosition`, `Vec3I128` types |
| `glam` | `0.29` | `Quat` type for rotations |

Rust edition 2024. The `nebula-math` crate is a workspace dependency providing the engine's foundational math types. `glam` is used for quaternion math and is already a transitive dependency of `bevy_ecs`.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::prelude::*;

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
        assert_eq!(a.0, false);
    }

    #[test]
    fn test_query_by_component_type() {
        let mut world = World::new();
        world.spawn((WorldPos::new(1, 2, 3), Velocity::new(10, 0, 0)));
        world.spawn(WorldPos::new(4, 5, 6)); // No velocity
        world.spawn(Velocity::new(0, 0, 5)); // No position

        // Query for entities with both WorldPos and Velocity
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
        assert_eq!(world.get::<Active>(entity).unwrap().0, true);
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
        assert_eq!(a.0, true);
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
```
