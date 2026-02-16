//! Per-entity gravity computation from multiple gravity sources.
//!
//! Supports inverse-square falloff, constant near-surface gravity,
//! influence radius cutoff, and smooth blending between multiple sources.

use bevy_ecs::prelude::*;
use glam::{DVec3, Vec3};
use nebula_math::WorldPosition;

use rapier3d::prelude::nalgebra;

use crate::{PhysicsWorld, RigidBodyHandle};

/// A component marking an entity as a source of gravity.
///
/// Planets, moons, asteroids, and space stations can all be gravity sources.
#[derive(Component, Clone, Debug)]
pub struct GravitySource {
    /// Mass of the body, in engine mass units.
    /// Determines gravitational pull strength.
    pub mass: f64,
    /// Surface gravity in m/s² — used as an override for gameplay tuning.
    /// If set, replaces the computed inverse-square value at the surface radius.
    pub surface_gravity: f32,
    /// Radius of the body's surface in meters.
    /// Used to define where "surface gravity" applies.
    pub surface_radius: f64,
    /// Maximum influence radius in meters.
    /// Beyond this, the source contributes zero gravity.
    pub influence_radius: f64,
    /// If true, gravity within the atmosphere is approximately constant
    /// (no inverse-square falloff near the surface). Simplifies surface gameplay.
    pub constant_near_surface: bool,
    /// Atmosphere height in meters. Within `surface_radius + atmosphere_height`,
    /// gravity magnitude equals `surface_gravity` if `constant_near_surface` is true.
    pub atmosphere_height: f64,
}

/// Cached per-entity gravity direction and magnitude, updated each fixed tick.
///
/// Other systems (character controller, dynamic body forces) read this to know
/// "which way is down" for a given entity.
#[derive(Component, Clone, Debug)]
pub struct LocalGravity {
    /// Direction of gravity (normalized, points "down" toward the source).
    pub direction: Vec3,
    /// Magnitude in m/s².
    pub magnitude: f32,
}

impl Default for LocalGravity {
    fn default() -> Self {
        Self {
            direction: Vec3::NEG_Y,
            magnitude: 0.0,
        }
    }
}

/// Result of computing combined gravity from all sources at a point.
pub struct GravityResult {
    /// Direction of gravity (normalized, points "down").
    pub direction: Vec3,
    /// Magnitude in m/s².
    pub magnitude: f32,
}

/// Compute the combined gravity vector at `entity_pos` from all given sources.
///
/// Uses f64 intermediate math for distance calculations (i128 deltas → f64).
/// Gravity follows inverse-square falloff from the surface value, with an
/// optional constant-near-surface mode within the atmosphere height.
pub fn compute_gravity(
    entity_pos: &WorldPosition,
    sources: &[(WorldPosition, &GravitySource)],
) -> GravityResult {
    let mut total_accel = DVec3::ZERO;

    for (source_pos, source) in sources {
        let delta_x = (source_pos.x - entity_pos.x) as f64;
        let delta_y = (source_pos.y - entity_pos.y) as f64;
        let delta_z = (source_pos.z - entity_pos.z) as f64;
        let distance_sq = delta_x * delta_x + delta_y * delta_y + delta_z * delta_z;
        let distance = distance_sq.sqrt();

        if distance > source.influence_radius || distance < 1.0 {
            continue;
        }

        let direction = DVec3::new(delta_x / distance, delta_y / distance, delta_z / distance);

        let magnitude = if source.constant_near_surface
            && distance <= source.surface_radius + source.atmosphere_height
        {
            source.surface_gravity as f64
        } else {
            let ratio = source.surface_radius / distance;
            source.surface_gravity as f64 * ratio * ratio
        };

        total_accel += direction * magnitude;
    }

    let total_mag = total_accel.length();
    if total_mag < 1e-6 {
        return GravityResult {
            direction: Vec3::NEG_Y,
            magnitude: 0.0,
        };
    }

    GravityResult {
        direction: Vec3::new(
            (total_accel.x / total_mag) as f32,
            (total_accel.y / total_mag) as f32,
            (total_accel.z / total_mag) as f32,
        ),
        magnitude: total_mag as f32,
    }
}

/// System that computes and caches gravity for each physics entity.
///
/// Runs early in `FixedUpdate` (ForceApplication set) so that downstream systems
/// can read the cached `LocalGravity` component.
pub fn gravity_update_system(
    sources: Query<(&crate::IslandWorldPos, &GravitySource)>,
    mut entities: Query<(&crate::IslandWorldPos, &mut LocalGravity), With<RigidBodyHandle>>,
) {
    let source_list: Vec<(WorldPosition, &GravitySource)> =
        sources.iter().map(|(pos, src)| (pos.0, src)).collect();

    for (entity_pos, mut gravity) in entities.iter_mut() {
        let result = compute_gravity(&entity_pos.0, &source_list);
        gravity.direction = result.direction;
        gravity.magnitude = result.magnitude;
    }
}

/// System that applies per-entity gravity as a force on dynamic rigid bodies.
///
/// Rapier's world gravity should be set to `(0, 0, 0)` when using this system,
/// so all gravity comes from per-body forces.
pub fn apply_gravity_forces_system(
    mut physics: ResMut<PhysicsWorld>,
    query: Query<(&RigidBodyHandle, &LocalGravity)>,
) {
    for (handle, gravity) in query.iter() {
        if let Some(body) = physics.rigid_body_set.get_mut(handle.0)
            && body.is_dynamic()
        {
            let mass = body.mass();
            let force = gravity.direction * gravity.magnitude * mass;
            body.add_force(
                rapier3d::prelude::vector![force.x, force.y, force.z].into(),
                true,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn earth_source() -> GravitySource {
        GravitySource {
            mass: 5.972e24,
            surface_gravity: 9.81,
            surface_radius: 6_371_000.0,
            influence_radius: 100_000_000.0,
            constant_near_surface: false,
            atmosphere_height: 100_000.0,
        }
    }

    #[test]
    fn test_gravity_points_toward_planet_center() {
        let source_pos = WorldPosition::new(0, 0, 0);
        let source = earth_source();
        let sources = vec![(source_pos, &source)];

        // Entity directly above: gravity should point down (-Y)
        let entity_pos = WorldPosition::new(0, 6_371_000, 0);
        let result = compute_gravity(&entity_pos, &sources);
        assert!((result.direction.x).abs() < 0.01);
        assert!((result.direction.y - (-1.0)).abs() < 0.01);
        assert!((result.direction.z).abs() < 0.01);

        // Entity to the right: gravity should point left (-X)
        let entity_pos = WorldPosition::new(6_371_000, 0, 0);
        let result = compute_gravity(&entity_pos, &sources);
        assert!((result.direction.x - (-1.0)).abs() < 0.01);
        assert!((result.direction.y).abs() < 0.01);
        assert!((result.direction.z).abs() < 0.01);
    }

    #[test]
    fn test_surface_gravity_magnitude() {
        let source_pos = WorldPosition::new(0, 0, 0);
        let source = earth_source();
        let sources = vec![(source_pos, &source)];

        let entity_pos = WorldPosition::new(0, 6_371_000, 0);
        let result = compute_gravity(&entity_pos, &sources);
        assert!(
            (result.magnitude - 9.81).abs() < 0.01,
            "Expected ~9.81, got {}",
            result.magnitude
        );
    }

    #[test]
    fn test_zero_gravity_far_from_sources() {
        let source_pos = WorldPosition::new(0, 0, 0);
        let source = GravitySource {
            influence_radius: 1_000_000.0,
            ..earth_source()
        };
        let sources = vec![(source_pos, &source)];

        let entity_pos = WorldPosition::new(0, 2_000_000, 0);
        let result = compute_gravity(&entity_pos, &sources);
        assert_eq!(result.magnitude, 0.0);
    }

    #[test]
    fn test_two_planets_create_blended_gravity() {
        let source_a_pos = WorldPosition::new(0, 0, 0);
        let source_b_pos = WorldPosition::new(10_000_000, 0, 0);
        let source = GravitySource {
            mass: 1e20,
            surface_gravity: 9.81,
            surface_radius: 1_000_000.0,
            influence_radius: 20_000_000.0,
            constant_near_surface: false,
            atmosphere_height: 0.0,
        };
        let sources = vec![(source_a_pos, &source), (source_b_pos, &source)];

        // Midpoint: gravity cancels out
        let midpoint = WorldPosition::new(5_000_000, 0, 0);
        let result = compute_gravity(&midpoint, &sources);
        assert!(
            result.magnitude < 0.01,
            "Expected near-zero at midpoint, got {}",
            result.magnitude
        );

        // Closer to source A
        let closer_to_a = WorldPosition::new(4_000_000, 0, 0);
        let result = compute_gravity(&closer_to_a, &sources);
        assert!(result.magnitude > 0.0);
        // Should point toward A (negative X)
        assert!(
            result.direction.x < 0.0,
            "Should point toward closer source"
        );
    }

    #[test]
    fn test_gravity_direction_changes_as_entity_orbits() {
        let source_pos = WorldPosition::new(0, 0, 0);
        let source = earth_source();
        let sources = vec![(source_pos, &source)];
        let r: i128 = 6_371_000;

        let positions = [
            (WorldPosition::new(r, 0, 0), Vec3::new(-1.0, 0.0, 0.0)),
            (WorldPosition::new(0, r, 0), Vec3::new(0.0, -1.0, 0.0)),
            (WorldPosition::new(0, 0, r), Vec3::new(0.0, 0.0, -1.0)),
            (WorldPosition::new(-r, 0, 0), Vec3::new(1.0, 0.0, 0.0)),
        ];

        for (pos, expected_dir) in &positions {
            let result = compute_gravity(pos, &sources);
            let diff = (result.direction - *expected_dir).length();
            assert!(
                diff < 0.01,
                "At {:?}, expected dir {:?}, got {:?}",
                pos,
                expected_dir,
                result.direction
            );
        }
    }

    #[test]
    fn test_constant_near_surface_flag() {
        let source_pos = WorldPosition::new(0, 0, 0);
        let source = GravitySource {
            constant_near_surface: true,
            atmosphere_height: 100_000.0,
            ..earth_source()
        };
        let sources = vec![(source_pos, &source)];

        // Within atmosphere: constant gravity
        let within = WorldPosition::new(0, 6_371_000 + 50_000, 0);
        let result = compute_gravity(&within, &sources);
        assert!(
            (result.magnitude - 9.81).abs() < 0.001,
            "Expected exactly 9.81 within atmosphere, got {}",
            result.magnitude
        );

        // Above atmosphere: inverse-square falloff
        let above = WorldPosition::new(0, 6_371_000 + 200_000, 0);
        let result = compute_gravity(&above, &sources);
        assert!(
            result.magnitude < 9.81,
            "Expected less than 9.81 above atmosphere, got {}",
            result.magnitude
        );
    }
}
