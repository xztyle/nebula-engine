//! i128-to-f32 physics bridge: converts between [`WorldPosition`] (i128, mm)
//! and Rapier's f32 local coordinate frame (meters).
//!
//! The key insight: subtraction happens in i128 space (exact), producing a small
//! offset that can be safely cast to f32 via f64 intermediate. This avoids
//! catastrophic precision loss from converting large absolute i128 values directly.

use bevy_ecs::prelude::*;
use glam::Vec3;
use nebula_math::{UNITS_PER_METER, WorldPosition};
use rapier3d::prelude::Vector;

use crate::PhysicsWorld;
use crate::physics_island::{IslandPlayer, IslandWorldPos, RigidBodyHandle};

/// The recenter threshold in meters. When the player moves further than this
/// from the current physics origin, all body positions are shifted.
const RECENTER_THRESHOLD_M: f32 = 64.0;

/// Resource defining the origin of the local physics coordinate frame.
///
/// All Rapier body translations are relative to this origin. The origin is
/// periodically re-centered on the player to keep offsets small.
#[derive(Resource, Debug, Clone, Default)]
pub struct PhysicsOrigin {
    /// The world-space position that maps to Rapier's `(0, 0, 0)`.
    pub world_origin: WorldPosition,
}

/// Convert a [`WorldPosition`] (i128 millimeters) to a local f32 position in meters,
/// relative to the given origin.
///
/// 1. Subtraction in i128 space (exact).
/// 2. Cast to f64 (lossless for offsets under 2^53).
/// 3. Divide by [`UNITS_PER_METER`] to convert mm → meters.
/// 4. Cast to f32.
pub fn world_to_local(world_pos: &WorldPosition, origin: &WorldPosition) -> Vec3 {
    let dx = (world_pos.x - origin.x) as f64 / UNITS_PER_METER as f64;
    let dy = (world_pos.y - origin.y) as f64 / UNITS_PER_METER as f64;
    let dz = (world_pos.z - origin.z) as f64 / UNITS_PER_METER as f64;
    Vec3::new(dx as f32, dy as f32, dz as f32)
}

/// Convert a local f32 position (meters) back to a [`WorldPosition`] (i128 mm),
/// given the origin.
///
/// 1. f32 → f64 (lossless widening).
/// 2. Multiply by [`UNITS_PER_METER`] to convert meters → mm.
/// 3. Round to nearest integer.
/// 4. Add to origin.
pub fn local_to_world(local_pos: &Vec3, origin: &WorldPosition) -> WorldPosition {
    let dx = (local_pos.x as f64 * UNITS_PER_METER as f64).round() as i128;
    let dy = (local_pos.y as f64 * UNITS_PER_METER as f64).round() as i128;
    let dz = (local_pos.z as f64 * UNITS_PER_METER as f64).round() as i128;
    WorldPosition::new(origin.x + dx, origin.y + dy, origin.z + dz)
}

/// Runs **before** the physics step. Syncs each entity's [`IslandWorldPos`]
/// to its Rapier rigid body translation in the local frame.
pub fn bridge_write_to_rapier(
    origin: Res<PhysicsOrigin>,
    mut physics: ResMut<PhysicsWorld>,
    query: Query<(&IslandWorldPos, &RigidBodyHandle)>,
) {
    for (world_pos, handle) in query.iter() {
        let local = world_to_local(&world_pos.0, &origin.world_origin);
        if let Some(body) = physics.rigid_body_set.get_mut(handle.0) {
            body.set_translation(Vector::new(local.x, local.y, local.z), false);
        }
    }
}

/// Runs **after** the physics step. Reads each Rapier body's translation and
/// writes it back into the entity's [`IslandWorldPos`].
pub fn bridge_read_from_rapier(
    origin: Res<PhysicsOrigin>,
    physics: Res<PhysicsWorld>,
    mut query: Query<(&mut IslandWorldPos, &RigidBodyHandle)>,
) {
    for (mut world_pos, handle) in query.iter_mut() {
        if let Some(body) = physics.rigid_body_set.get(handle.0) {
            let t = body.translation();
            let local = Vec3::new(t.x, t.y, t.z);
            world_pos.0 = local_to_world(&local, &origin.world_origin);
        }
    }
}

/// Shifts the physics origin to the player position when the player has moved
/// more than [`RECENTER_THRESHOLD_M`] from the current origin.
///
/// All active Rapier body positions are adjusted by the inverse shift so their
/// world-space positions remain unchanged.
pub fn recenter_physics_origin(
    mut origin: ResMut<PhysicsOrigin>,
    mut physics: ResMut<PhysicsWorld>,
    player_query: Query<&IslandWorldPos, With<IslandPlayer>>,
) {
    let Some(player_pos) = player_query.iter().next() else {
        return;
    };

    let shift = world_to_local(&player_pos.0, &origin.world_origin);

    if shift.length() > RECENTER_THRESHOLD_M {
        origin.world_origin = player_pos.0;

        // Shift all Rapier body positions by the inverse offset.
        for (_, body) in physics.rigid_body_set.iter_mut() {
            let t = body.translation();
            let new_t = Vector::new(t.x - shift.x, t.y - shift.y, t.z - shift.z);
            body.set_translation(new_t, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_math::UNITS_PER_METER;

    #[test]
    fn test_world_to_local_accuracy() {
        let origin = WorldPosition::new(1_000_000_000_000, 0, 1_000_000_000_000);
        let pos = WorldPosition::new(
            1_000_000_000_000 + 100 * UNITS_PER_METER,
            50 * UNITS_PER_METER,
            1_000_000_000_000 + 200 * UNITS_PER_METER,
        );
        let local = world_to_local(&pos, &origin);
        assert!((local.x - 100.0).abs() < f32::EPSILON);
        assert!((local.y - 50.0).abs() < f32::EPSILON);
        assert!((local.z - 200.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_local_to_world_roundtrip() {
        let origin =
            WorldPosition::new(9_460_730_472_580_800, 1_000_000_000_000, -5_000_000_000_000);

        let mut rng_state: u64 = 42;
        for _ in 0..100 {
            // Simple LCG for deterministic pseudo-random offsets within 512m.
            rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1);
            let offset_mm = |s: &mut u64| -> i128 {
                *s = s.wrapping_mul(6364136223846793005).wrapping_add(1);
                ((*s >> 33) as i64 % (512 * UNITS_PER_METER as i64)) as i128
            };

            let dx = offset_mm(&mut rng_state);
            let dy = offset_mm(&mut rng_state);
            let dz = offset_mm(&mut rng_state);

            let original = WorldPosition::new(origin.x + dx, origin.y + dy, origin.z + dz);

            let local = world_to_local(&original, &origin);
            let recovered = local_to_world(&local, &origin);

            assert!(
                (recovered.x - original.x).abs() <= 1,
                "x mismatch: {} vs {} (delta_mm={})",
                recovered.x,
                original.x,
                dx
            );
            assert!(
                (recovered.y - original.y).abs() <= 1,
                "y mismatch: {} vs {} (delta_mm={})",
                recovered.y,
                original.y,
                dy
            );
            assert!(
                (recovered.z - original.z).abs() <= 1,
                "z mismatch: {} vs {} (delta_mm={})",
                recovered.z,
                original.z,
                dz
            );
        }
    }

    #[test]
    fn test_recenter_preserves_relative_positions() {
        use rapier3d::prelude::{RigidBodyBuilder, Vector};

        let mut physics = PhysicsWorld::new();

        // Two bodies 50m apart
        let pos_a = WorldPosition::new(0, 0, 0);
        let pos_b = WorldPosition::new(50 * UNITS_PER_METER, 0, 0);

        let origin = PhysicsOrigin::default();
        let local_a = world_to_local(&pos_a, &origin.world_origin);
        let local_b = world_to_local(&pos_b, &origin.world_origin);

        let body_a = RigidBodyBuilder::dynamic()
            .translation(Vector::new(local_a.x, local_a.y, local_a.z))
            .build();
        let body_b = RigidBodyBuilder::dynamic()
            .translation(Vector::new(local_b.x, local_b.y, local_b.z))
            .build();
        let ha = physics.rigid_body_set.insert(body_a);
        let hb = physics.rigid_body_set.insert(body_b);

        let ta = physics.rigid_body_set[ha].translation();
        let tb = physics.rigid_body_set[hb].translation();
        let dist_before =
            ((tb.x - ta.x).powi(2) + (tb.y - ta.y).powi(2) + (tb.z - ta.z).powi(2)).sqrt();

        // Simulate recenter: player moved 100m away
        let player_pos = WorldPosition::new(100 * UNITS_PER_METER, 0, 0);
        let shift = world_to_local(&player_pos, &origin.world_origin);

        for (_, body) in physics.rigid_body_set.iter_mut() {
            let t = body.translation();
            let new_t = Vector::new(t.x - shift.x, t.y - shift.y, t.z - shift.z);
            body.set_translation(new_t, false);
        }

        let ta2 = physics.rigid_body_set[ha].translation();
        let tb2 = physics.rigid_body_set[hb].translation();
        let dist_after =
            ((tb2.x - ta2.x).powi(2) + (tb2.y - ta2.y).powi(2) + (tb2.z - ta2.z).powi(2)).sqrt();

        assert!(
            (dist_after - dist_before).abs() < 1e-4,
            "Relative distance changed: {dist_before} -> {dist_after}"
        );
    }

    #[test]
    fn test_recenter_does_not_teleport_objects() {
        use rapier3d::prelude::{RigidBodyBuilder, Vector};

        let mut physics = PhysicsWorld::new();

        let body_world_pos = WorldPosition::new(30 * UNITS_PER_METER, 10 * UNITS_PER_METER, 0);
        let mut origin = PhysicsOrigin::default();

        let local = world_to_local(&body_world_pos, &origin.world_origin);
        let body = RigidBodyBuilder::dynamic()
            .translation(Vector::new(local.x, local.y, local.z))
            .build();
        let handle = physics.rigid_body_set.insert(body);

        // Recenter to player at 100m
        let player_pos = WorldPosition::new(100 * UNITS_PER_METER, 0, 0);
        let shift = world_to_local(&player_pos, &origin.world_origin);
        origin.world_origin = player_pos;

        for (_, body) in physics.rigid_body_set.iter_mut() {
            let t = body.translation();
            let new_t = Vector::new(t.x - shift.x, t.y - shift.y, t.z - shift.z);
            body.set_translation(new_t, false);
        }

        // Read back world position
        let t = physics.rigid_body_set[handle].translation();
        let recovered = local_to_world(&Vec3::new(t.x, t.y, t.z), &origin.world_origin);

        assert!(
            (recovered.x - body_world_pos.x).abs() <= 1,
            "x teleported: {} vs {}",
            recovered.x,
            body_world_pos.x
        );
        assert!(
            (recovered.y - body_world_pos.y).abs() <= 1,
            "y teleported: {} vs {}",
            recovered.y,
            body_world_pos.y
        );
        assert!(
            (recovered.z - body_world_pos.z).abs() <= 1,
            "z teleported: {} vs {}",
            recovered.z,
            body_world_pos.z
        );
    }

    #[test]
    fn test_bridge_handles_origin_shift() {
        use rapier3d::prelude::{RigidBodyBuilder, Vector};

        let mut physics = PhysicsWorld::new();
        let mut origin = PhysicsOrigin::default();

        // Player moves 100m, recenter
        let player_pos = WorldPosition::new(100 * UNITS_PER_METER, 0, 0);
        let shift = world_to_local(&player_pos, &origin.world_origin);
        origin.world_origin = player_pos;

        for (_, body) in physics.rigid_body_set.iter_mut() {
            let t = body.translation();
            let new_t = Vector::new(t.x - shift.x, t.y - shift.y, t.z - shift.z);
            body.set_translation(new_t, false);
        }

        // Add a new entity 10m from player AFTER recenter
        let entity_pos = WorldPosition::new(110 * UNITS_PER_METER, 0, 0);
        let local = world_to_local(&entity_pos, &origin.world_origin);

        let body = RigidBodyBuilder::dynamic()
            .translation(Vector::new(local.x, local.y, local.z))
            .build();
        let handle = physics.rigid_body_set.insert(body);

        // Verify the local position is ~10m, not 110m
        let t = physics.rigid_body_set[handle].translation();
        assert!(
            (t.x - 10.0).abs() < 0.01,
            "New entity should be at 10m local, got {}",
            t.x
        );

        // Read back world position
        let recovered = local_to_world(&Vec3::new(t.x, t.y, t.z), &origin.world_origin);
        assert!(
            (recovered.x - entity_pos.x).abs() <= 1,
            "World pos mismatch: {} vs {}",
            recovered.x,
            entity_pos.x
        );
    }

    #[test]
    fn test_precision_valid_within_island_radius() {
        let origin = WorldPosition::new(
            1_000_000_000_000_000,
            500_000_000_000,
            -2_000_000_000_000_000,
        );

        for dist_m in [0, 128, 256, 512] {
            let offset_mm = dist_m * UNITS_PER_METER;
            let pos = WorldPosition::new(
                origin.x + offset_mm,
                origin.y + offset_mm,
                origin.z + offset_mm,
            );

            let local = world_to_local(&pos, &origin);
            let recovered = local_to_world(&local, &origin);

            let err_x = (recovered.x - pos.x).abs();
            let err_y = (recovered.y - pos.y).abs();
            let err_z = (recovered.z - pos.z).abs();

            assert!(
                err_x <= 1 && err_y <= 1 && err_z <= 1,
                "Roundtrip error at {}m: ({}, {}, {})",
                dist_m,
                err_x,
                err_y,
                err_z
            );
        }
    }
}
