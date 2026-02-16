//! Floating-origin coordinate bridge.
//!
//! Every frame the active camera's [`WorldPosition`] becomes the
//! [`FloatingOrigin`]. Each entity's [`LocalPos`] is then recomputed as
//! `(entity.WorldPos − origin)` cast to f32.  The subtraction happens
//! entirely in i128 arithmetic so precision is independent of absolute
//! magnitude.

use bevy_ecs::prelude::*;
use nebula_ecs::{LocalPos, WorldPos};
use nebula_math::{LocalPosition, WorldPosition};

// ---------------------------------------------------------------------------
// Resource
// ---------------------------------------------------------------------------

/// The current frame's floating origin — set to the active camera's
/// [`WorldPosition`] at the start of each frame.  All [`LocalPos`]
/// components are computed relative to this origin.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct FloatingOrigin(pub WorldPosition);

// ---------------------------------------------------------------------------
// Marker
// ---------------------------------------------------------------------------

/// Marker component for the currently active camera entity.
#[derive(Component)]
pub struct ActiveCamera;

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Copies the first [`ActiveCamera`] entity's world position into the
/// [`FloatingOrigin`] resource.
pub fn update_floating_origin_system(
    camera_query: Query<&WorldPos, With<ActiveCamera>>,
    mut origin: ResMut<FloatingOrigin>,
) {
    if let Some(cam_pos) = camera_query.iter().next() {
        origin.0 = cam_pos.0;
    }
}

/// Recomputes every entity's [`LocalPos`] as `(WorldPos − FloatingOrigin)`
/// cast to f32.  The subtraction is performed in i128 before the cast so
/// nearby objects retain millimetre precision regardless of absolute coords.
pub fn recompute_local_positions_system(
    origin: Res<FloatingOrigin>,
    mut query: Query<(&WorldPos, &mut LocalPos)>,
) {
    let origin_pos = origin.0;
    for (world_pos, mut local_pos) in query.iter_mut() {
        let delta = world_pos.0 - origin_pos; // Vec3I128, exact
        local_pos.0 = LocalPosition::new(delta.x as f32, delta.y as f32, delta.z as f32);
    }
}

/// Convenience helper: adds the two systems to `schedule` with a chain
/// guarantee (origin update runs before local-position recomputation).
pub fn build_local_position_schedule(schedule: &mut Schedule) {
    schedule.add_systems(
        (
            update_floating_origin_system,
            recompute_local_positions_system,
        )
            .chain(),
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_math::{LocalPosition, WorldPosition};

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
        let entity_world = WorldPos::new(1_000_001_500, 0, 2_000);
        let delta = entity_world.0 - origin.0;
        let local = LocalPosition::new(delta.x as f32, delta.y as f32, delta.z as f32);
        assert!((local.x - 1500.0).abs() < 1e-6);
        assert!((local.y).abs() < 1e-6);
        assert!((local.z - 2000.0).abs() < 1e-6);
    }

    #[test]
    fn test_distant_entity_has_correct_local_pos() {
        let origin = FloatingOrigin(WorldPosition::new(0, 0, 0));
        let entity_world = WorldPos::new(50_000_000, 0, 0);
        let delta = entity_world.0 - origin.0;
        let local = LocalPosition::new(delta.x as f32, delta.y as f32, delta.z as f32);
        assert!((local.x - 50_000_000.0).abs() < 1.0);
    }

    #[test]
    fn test_origin_shift_updates_all_local_positions() {
        let entity_a = WorldPos::new(1000, 2000, 3000);
        let entity_b = WorldPos::new(4000, 5000, 6000);

        let origin_1 = FloatingOrigin(WorldPosition::new(0, 0, 0));
        let delta_a1 = entity_a.0 - origin_1.0;
        let local_a1 = LocalPosition::new(delta_a1.x as f32, delta_a1.y as f32, delta_a1.z as f32);
        assert!((local_a1.x - 1000.0).abs() < 1e-6);

        let origin_2 = FloatingOrigin(WorldPosition::new(1000, 1000, 1000));
        let delta_a2 = entity_a.0 - origin_2.0;
        let local_a2 = LocalPosition::new(delta_a2.x as f32, delta_a2.y as f32, delta_a2.z as f32);
        assert!((local_a2.x).abs() < 1e-6);
        assert!((local_a2.y - 1000.0).abs() < 1e-6);

        let delta_b2 = entity_b.0 - origin_2.0;
        let local_b2 = LocalPosition::new(delta_b2.x as f32, delta_b2.y as f32, delta_b2.z as f32);
        assert!((local_b2.x - 3000.0).abs() < 1e-6);
        assert!((local_b2.y - 4000.0).abs() < 1e-6);
    }

    #[test]
    fn test_f32_precision_valid_within_render_distance() {
        let origin = FloatingOrigin(WorldPosition::new(0, 0, 0));
        let distances: Vec<i128> = vec![1, 100, 10_000, 1_000_000, 8_000_000];
        for d in distances {
            let entity_world = WorldPos::new(d, 0, 0);
            let delta = entity_world.0 - origin.0;
            let local = LocalPosition::new(delta.x as f32, delta.y as f32, delta.z as f32);
            assert!(
                (local.x - d as f32).abs() <= 1.0,
                "Precision loss at distance {d}: local.x = {}, expected {d}",
                local.x,
            );
        }
    }

    #[test]
    fn test_large_absolute_coords_small_delta() {
        let ly_mm: i128 = 9_460_730_472_580_800_000;
        let origin = FloatingOrigin(WorldPosition::new(50 * ly_mm, 50 * ly_mm, 50 * ly_mm));
        let entity = WorldPos::new(50 * ly_mm + 500, 50 * ly_mm - 300, 50 * ly_mm + 100);
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
        assert!((local.z).abs() < 1e-6);
    }

    #[test]
    fn test_floating_origin_default_is_world_origin() {
        let origin = FloatingOrigin::default();
        assert_eq!(origin.0, WorldPosition::default());
    }
}
