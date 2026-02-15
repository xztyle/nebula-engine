# Camera 128-bit World-to-Local Bridge

## Problem

The Nebula Engine stores all positions as 128-bit integers (`WorldPosition`, 1 unit = 1 mm), but the GPU operates exclusively in 32-bit floating-point space. This is the fundamental tension of the engine's coordinate system. The solution — established conceptually in 02_math/07 and 04_rendering/06 — is "origin rebasing": every frame, the camera's `WorldPosition` becomes the floating origin, and every entity's `LocalPos` is recomputed as `entity.WorldPos - camera.WorldPos`, cast to f32. This story implements the ECS system that performs this conversion for all entities every frame.

This is the most performance-critical precision bridge in the engine. Every visible entity, every chunk, every particle must pass through it. The conversion must be correct (subtract before cast, never cast before subtract), must update every frame (the camera moves), and must handle the case where entities are extremely far away (the f32 result will be imprecise but must not be NaN or infinite for any realistic distance).

## Solution

### Floating origin resource

```rust
use bevy_ecs::prelude::*;
use nebula_math::WorldPosition;

/// The current frame's floating origin. Set to the active camera's
/// WorldPosition at the start of each frame. All LocalPos components
/// are computed relative to this origin.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct FloatingOrigin(pub WorldPosition);
```

### Origin update system

Runs first in the frame, before any system that reads `LocalPos`:

```rust
/// Marker component for the currently active camera entity.
#[derive(Component)]
pub struct ActiveCamera;

pub fn update_floating_origin_system(
    camera_query: Query<&WorldPos, With<ActiveCamera>>,
    mut origin: ResMut<FloatingOrigin>,
) {
    // Use the first active camera found. Multi-camera setups would need
    // a priority or selection mechanism, but the common case is one camera.
    if let Some(cam_pos) = camera_query.iter().next() {
        origin.0 = cam_pos.0;
    }
}
```

### Local position recomputation system

Runs after origin update, before rendering. Converts every entity's `WorldPos` to a camera-relative `LocalPos`:

```rust
use nebula_math::LocalPosition;

pub fn recompute_local_positions_system(
    origin: Res<FloatingOrigin>,
    mut query: Query<(&WorldPos, &mut LocalPos)>,
) {
    let origin_pos = origin.0;
    for (world_pos, mut local_pos) in query.iter_mut() {
        let delta = world_pos.0 - origin_pos; // Vec3I128, exact subtraction
        local_pos.0 = LocalPosition::new(
            delta.x as f32,
            delta.y as f32,
            delta.z as f32,
        );
    }
}
```

### Precision characteristics

The key insight is that the subtraction `entity.WorldPos - camera.WorldPos` is performed entirely in i128 arithmetic — it is exact regardless of how large the absolute coordinates are. Only the result (a small delta if the entity is nearby) is cast to f32. This means:

- An entity at the camera's exact position: delta = (0, 0, 0), f32 = (0.0, 0.0, 0.0). Exact.
- An entity 1 meter away: delta = (1000, 0, 0), f32 = (1000.0, 0.0, 0.0). Exact to 1 mm.
- An entity 1 km away: delta = (1_000_000, 0, 0), f32 = (1000000.0, 0.0, 0.0). Exact to 1 mm.
- An entity 100 km away: delta = (100_000_000, 0, 0), f32 = (100000000.0, 0.0, 0.0). Precision ~8 mm — acceptable for distant objects.
- An entity 1 light-year away: delta is ~9.46e18, f32 can represent this but with ~10^12 mm precision. Objects this far would not be rendered as meshes anyway (they would be rendered as points or billboards via LOD).

The system does not clamp or reject large deltas. Entities beyond render distance are culled by the frustum culling system (04_rendering/09), not by the coordinate bridge.

### System ordering

```rust
use bevy_ecs::prelude::*;

pub fn build_local_position_schedule(schedule: &mut Schedule) {
    schedule.add_systems((
        update_floating_origin_system,
        recompute_local_positions_system,
    ).chain());
}
```

The `.chain()` ensures origin update runs before local position recomputation. Both run in the `PostUpdate` stage, after gameplay systems have moved entities but before the renderer reads `LocalPos`.

## Outcome

A `floating_origin.rs` module in `crates/nebula_player/src/` exporting `FloatingOrigin`, `ActiveCamera`, `update_floating_origin_system`, and `recompute_local_positions_system`. After this story, all entities with `WorldPos` and `LocalPos` components automatically have their local positions updated every frame relative to the camera. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

The camera's WorldPosition drives the floating origin. Everything renders relative to the camera at local (0,0,0). Walking for minutes shows no jitter or precision drift.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.18` | `Resource`, `Component`, `Res`, `ResMut`, `Query`, `With`, `Schedule` |
| `nebula-math` | workspace | `WorldPosition`, `LocalPosition`, `Vec3I128` for coordinate conversion |
| `nebula-ecs` | workspace | `WorldPos`, `LocalPos` components |

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nebula_math::{WorldPosition, LocalPosition};

    #[test]
    fn test_entity_at_camera_position_has_local_zero() {
        let origin = FloatingOrigin(WorldPosition::new(5_000_000, 10_000_000, 15_000_000));
        let entity_world = WorldPos::new(5_000_000, 10_000_000, 15_000_000);
        let delta = entity_world.0 - origin.0;
        let local = LocalPosition::new(delta.x as f32, delta.y as f32, delta.z as f32);
        assert!((local.x).abs() < 1e-6);
        assert!((local.y).abs() < 1e-6);
        assert!((local.z).abs() < 1e-6);
    }

    #[test]
    fn test_nearby_entity_has_small_accurate_local_pos() {
        let origin = FloatingOrigin(WorldPosition::new(1_000_000_000, 0, 0));
        let entity_world = WorldPos::new(1_000_001_500, 0, 2_000); // 1.5m right, 2m forward
        let delta = entity_world.0 - origin.0;
        let local = LocalPosition::new(delta.x as f32, delta.y as f32, delta.z as f32);
        assert!((local.x - 1500.0).abs() < 1e-6);
        assert!((local.y - 0.0).abs() < 1e-6);
        assert!((local.z - 2000.0).abs() < 1e-6);
    }

    #[test]
    fn test_distant_entity_has_correct_local_pos() {
        // Entity 50 km away. f32 can represent 50_000_000 with some precision loss
        // but the value should still be in the right ballpark.
        let origin = FloatingOrigin(WorldPosition::new(0, 0, 0));
        let entity_world = WorldPos::new(50_000_000, 0, 0); // 50 km
        let delta = entity_world.0 - origin.0;
        let local = LocalPosition::new(delta.x as f32, delta.y as f32, delta.z as f32);
        // f32 can represent 50_000_000 exactly (it's within 2^26)
        assert!((local.x - 50_000_000.0).abs() < 1.0);
    }

    #[test]
    fn test_origin_shift_updates_all_local_positions() {
        // Two entities at fixed world positions. Moving the origin changes
        // both their local positions.
        let entity_a = WorldPos::new(1000, 2000, 3000);
        let entity_b = WorldPos::new(4000, 5000, 6000);

        // Origin at (0,0,0)
        let origin_1 = FloatingOrigin(WorldPosition::new(0, 0, 0));
        let delta_a1 = entity_a.0 - origin_1.0;
        let local_a1 = LocalPosition::new(delta_a1.x as f32, delta_a1.y as f32, delta_a1.z as f32);
        assert!((local_a1.x - 1000.0).abs() < 1e-6);

        // Origin shifts to (1000, 1000, 1000)
        let origin_2 = FloatingOrigin(WorldPosition::new(1000, 1000, 1000));
        let delta_a2 = entity_a.0 - origin_2.0;
        let local_a2 = LocalPosition::new(delta_a2.x as f32, delta_a2.y as f32, delta_a2.z as f32);
        assert!((local_a2.x - 0.0).abs() < 1e-6);
        assert!((local_a2.y - 1000.0).abs() < 1e-6);

        let delta_b2 = entity_b.0 - origin_2.0;
        let local_b2 = LocalPosition::new(delta_b2.x as f32, delta_b2.y as f32, delta_b2.z as f32);
        assert!((local_b2.x - 3000.0).abs() < 1e-6);
        assert!((local_b2.y - 4000.0).abs() < 1e-6);
    }

    #[test]
    fn test_f32_precision_valid_within_render_distance() {
        // Within 8.4 km (~2^23 mm), f32 preserves 1 mm precision.
        let origin = FloatingOrigin(WorldPosition::new(0, 0, 0));
        let distances: Vec<i128> = vec![1, 100, 10_000, 1_000_000, 8_000_000];
        for d in distances {
            let entity_world = WorldPos::new(d, 0, 0);
            let delta = entity_world.0 - origin.0;
            let local = LocalPosition::new(delta.x as f32, delta.y as f32, delta.z as f32);
            // The f32 value should be within 1 mm of the true value
            assert!(
                (local.x - d as f32).abs() <= 1.0,
                "Precision loss at distance {d}: local.x = {}, expected {d}",
                local.x,
            );
        }
    }

    #[test]
    fn test_large_absolute_coords_small_delta() {
        // Camera and entity are both at galactic distances, but close to each other.
        // The subtraction in i128 is exact; the small delta is f32-precise.
        let ly_mm: i128 = 9_460_730_472_580_800_000;
        let origin = FloatingOrigin(WorldPosition::new(
            50 * ly_mm, 50 * ly_mm, 50 * ly_mm,
        ));
        let entity = WorldPos::new(
            50 * ly_mm + 500,
            50 * ly_mm - 300,
            50 * ly_mm + 100,
        );
        let delta = entity.0 - origin.0;
        let local = LocalPosition::new(delta.x as f32, delta.y as f32, delta.z as f32);
        assert!((local.x - 500.0).abs() < 1e-6);
        assert!((local.y - (-300.0)).abs() < 1e-6);
        assert!((local.z - 100.0).abs() < 1e-6);
    }

    #[test]
    fn test_negative_coordinates_handled() {
        let origin = FloatingOrigin(WorldPosition::new(-1_000_000, -2_000_000, -3_000_000));
        let entity = WorldPos::new(-1_000_500, -2_001_000, -3_000_000);
        let delta = entity.0 - origin.0;
        let local = LocalPosition::new(delta.x as f32, delta.y as f32, delta.z as f32);
        assert!((local.x - (-500.0)).abs() < 1e-6);
        assert!((local.y - (-1000.0)).abs() < 1e-6);
        assert!((local.z - 0.0).abs() < 1e-6);
    }

    #[test]
    fn test_floating_origin_default_is_world_origin() {
        let origin = FloatingOrigin::default();
        assert_eq!(origin.0, WorldPosition::default());
    }
}
```
