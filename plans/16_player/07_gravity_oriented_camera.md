# Gravity-Oriented Camera

## Problem

On a planet surface, "up" is not world +Y — it is the direction away from the planet's center. The Nebula Engine uses cubesphere-voxel planets (05_cubesphere), which means a player walking across the surface gradually transitions from one face of the cube to another, and the surface normal rotates continuously. If the camera's up vector remains fixed at world +Y, the player will feel increasingly "tilted" as they move away from the equator, and at the poles the horizon will be completely sideways.

The camera must dynamically align its up vector to match the local gravity direction (the inverse of the gravity vector, which points toward the planet center). This alignment must be smooth — not snapping frame-to-frame — so that walking over the curved surface feels natural. The gravity-oriented camera works in conjunction with the first person camera (story 01) or third person camera (story 02), modifying their up vector rather than replacing them.

## Solution

### Gravity direction component

The gravity system (34_gravity) provides the gravity vector for each entity. This story consumes that information:

```rust
use bevy_ecs::prelude::*;
use glam::Vec3;

/// The normalized gravity direction at the entity's position.
/// Points toward the planet center (i.e., "down" in local terms).
/// Set by the gravity system each tick based on the entity's WorldPos
/// and the nearest planet's center.
#[derive(Component, Clone, Copy, Debug)]
pub struct GravityDirection(pub Vec3);

impl Default for GravityDirection {
    fn default() -> Self {
        Self(Vec3::NEG_Y) // Default: standard downward gravity
    }
}
```

### Gravity-oriented camera component

```rust
/// Configures gravity-based up-vector alignment for a camera entity.
/// Works alongside FirstPersonCamera or ThirdPersonCamera — it modifies
/// the up vector used by those controllers rather than replacing them.
#[derive(Component, Clone, Debug)]
pub struct GravityOrientedCamera {
    /// How quickly the camera's up vector aligns to the gravity direction.
    /// 0.0 = no alignment (up stays fixed). 1.0 = instant snap.
    /// Values around 0.05..0.15 produce smooth, comfortable alignment.
    pub alignment_speed: f32,
    /// The camera's current effective up vector, smoothly tracking the
    /// anti-gravity direction. Initialized to world +Y.
    pub current_up: Vec3,
}

impl Default for GravityOrientedCamera {
    fn default() -> Self {
        Self {
            alignment_speed: 0.1,
            current_up: Vec3::Y,
        }
    }
}
```

### Up-vector alignment system

Each tick, the system lerps the camera's effective up vector toward the anti-gravity direction:

```rust
use glam::Vec3;

pub fn gravity_up_alignment_system(
    mut query: Query<(&GravityDirection, &mut GravityOrientedCamera)>,
) {
    for (gravity, mut grav_cam) in query.iter_mut() {
        // The desired "up" is the opposite of the gravity direction.
        let target_up = -gravity.0;

        // Guard against zero-length gravity (e.g., in deep space between planets).
        if target_up.length_squared() < 1e-6 {
            continue;
        }

        let target_up = target_up.normalize();

        // Lerp the current up toward the target up.
        // Using Vec3::lerp + normalize produces a smooth rotation of the up vector.
        let new_up = grav_cam.current_up
            .lerp(target_up, grav_cam.alignment_speed)
            .normalize_or_zero();

        if new_up.length_squared() > 0.5 {
            grav_cam.current_up = new_up;
        }
    }
}
```

### Camera rotation reorientation system

After the up vector is updated, the camera's rotation quaternion must be adjusted so that its local +Y aligns with the new up vector while preserving the player's intended look direction as much as possible:

```rust
use glam::{Quat, Vec3, Mat3};

pub fn gravity_orient_rotation_system(
    mut query: Query<(&GravityOrientedCamera, &mut Rotation, Option<&FirstPersonCamera>)>,
) {
    for (grav_cam, mut rotation, fps_cam) in query.iter_mut() {
        let up = grav_cam.current_up;

        // Extract the current forward direction from the camera's rotation.
        let current_forward = rotation.0 * Vec3::NEG_Z;

        // Project forward onto the plane perpendicular to the new up vector.
        // This preserves the player's horizontal look direction.
        let forward_on_plane = (current_forward - up * current_forward.dot(up))
            .normalize_or_zero();

        if forward_on_plane.length_squared() < 0.5 {
            // Camera is looking straight up or down along the gravity axis.
            // Use an arbitrary perpendicular direction to avoid degeneracy.
            continue;
        }

        // Reconstruct the rotation from the new basis vectors.
        let right = forward_on_plane.cross(up).normalize();
        let corrected_up = right.cross(forward_on_plane).normalize();

        // Build a look rotation: forward = forward_on_plane, up = corrected_up
        let basis = Mat3::from_cols(right, corrected_up, -forward_on_plane);
        let base_rotation = Quat::from_mat3(&basis).normalize();

        // If this is a first-person camera, reapply pitch on top of the
        // gravity-aligned base rotation. The pitch is relative to the
        // local horizon (the plane perpendicular to `up`).
        if let Some(fps) = fps_cam {
            let pitch_quat = Quat::from_axis_angle(right, fps.pitch);
            rotation.0 = (pitch_quat * base_rotation).normalize();
        } else {
            rotation.0 = base_rotation;
        }
    }
}
```

### Integration with first/third person cameras

The first person camera (story 01) normally reconstructs its rotation from yaw and pitch using world +Y as the up axis. When `GravityOrientedCamera` is also present on the entity, the gravity orientation system overrides the up axis. The execution order is:

1. `first_person_look_system` — updates yaw/pitch from mouse input
2. `gravity_up_alignment_system` — lerps `current_up` toward anti-gravity
3. `gravity_orient_rotation_system` — rebuilds rotation using the gravity-aligned up vector and the player's yaw/pitch

For third person cameras, the orbit system's vertical axis is replaced by `current_up` instead of world +Y.

### Planet center computation

The `GravityDirection` for an entity on or near a planet surface is computed as:

```rust
// In the gravity system (34_gravity), not implemented in this story:
let planet_center: WorldPosition = /* ... */;
let entity_pos: WorldPosition = /* entity's WorldPos */;
let delta = entity_pos - planet_center; // Vec3I128
let dir_f32 = Vec3::new(delta.x as f32, delta.y as f32, delta.z as f32);
let gravity_direction = -dir_f32.normalize(); // points toward planet center
```

This story consumes `GravityDirection`; it does not produce it.

## Outcome

A `gravity_oriented_camera.rs` module in `crates/nebula_player/src/` exporting `GravityDirection`, `GravityOrientedCamera`, `gravity_up_alignment_system`, and `gravity_orient_rotation_system`. When attached to a camera entity alongside a first or third person camera controller, the camera's up vector smoothly aligns to the planet surface normal, enabling natural navigation over cubesphere planets. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

On the planet surface, "up" is always away from the planet center. Walking from the "top" of the cubesphere to the "side" smoothly reorients the camera so ground is always down.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.18` | `Component` derive, `Query`, `Option` filter, system functions |
| `glam` | `0.32` | `Vec3`, `Quat`, `Mat3` for up-vector alignment and rotation |
| `nebula-ecs` | workspace | `Rotation` component, `FirstPersonCamera` for pitch reapplication |
| `nebula-math` | workspace | `WorldPosition`, `Vec3I128` (consumed indirectly via GravityDirection) |

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Quat, Vec3};
    use std::f32::consts::FRAC_PI_2;

    /// Helper: compute the effective up vector from a rotation quaternion.
    fn up_from_rotation(rotation: &Quat) -> Vec3 {
        *rotation * Vec3::Y
    }

    #[test]
    fn test_up_vector_points_away_from_planet_center() {
        // Entity is directly above the planet center (planet center at origin,
        // entity at +Y). Gravity points -Y, so up should be +Y.
        let gravity = GravityDirection(Vec3::NEG_Y);
        let target_up = -gravity.0;
        assert!((target_up - Vec3::Y).length() < 1e-6);
    }

    #[test]
    fn test_walking_over_surface_keeps_horizon_level() {
        // Simulate walking from the "north pole" (+Y) to the "equator" (+X).
        // The gravity direction gradually rotates from -Y toward -X.
        // The camera's up vector should follow, keeping the local horizon level.
        let mut grav_cam = GravityOrientedCamera {
            alignment_speed: 1.0, // instant for testing
            current_up: Vec3::Y,
        };

        // Position 1: on top (+Y face), gravity = -Y, up = +Y
        let gravity_1 = GravityDirection(Vec3::NEG_Y);
        let target_up_1 = (-gravity_1.0).normalize();
        grav_cam.current_up = grav_cam.current_up.lerp(target_up_1, 1.0).normalize();
        assert!((grav_cam.current_up - Vec3::Y).length() < 1e-4);

        // Position 2: on equator (+X face), gravity = -X, up = +X
        let gravity_2 = GravityDirection(Vec3::NEG_X);
        let target_up_2 = (-gravity_2.0).normalize();
        grav_cam.current_up = grav_cam.current_up.lerp(target_up_2, 1.0).normalize();
        assert!((grav_cam.current_up - Vec3::X).length() < 1e-4);
    }

    #[test]
    fn test_up_vector_changes_smoothly_not_snapping() {
        let mut grav_cam = GravityOrientedCamera {
            alignment_speed: 0.1, // slow lerp
            current_up: Vec3::Y,
        };

        // Gravity suddenly changes to -X (entity moved to equator)
        let target_up = Vec3::X;
        grav_cam.current_up = grav_cam.current_up
            .lerp(target_up, grav_cam.alignment_speed)
            .normalize();

        // After one tick, up should be mostly +Y with a slight lean toward +X
        assert!(grav_cam.current_up.y > 0.5, "Still mostly +Y after one tick");
        assert!(grav_cam.current_up.x > 0.0, "Started leaning toward +X");
        assert!((grav_cam.current_up - Vec3::X).length() > 0.1, "Not yet at +X");
    }

    #[test]
    fn test_at_pole_up_vector_is_correct() {
        // At the "north pole" of a planet centered at origin,
        // the entity is at (0, R, 0). Gravity points toward center = -Y.
        // Up should be +Y.
        let gravity = GravityDirection(Vec3::NEG_Y);
        let mut grav_cam = GravityOrientedCamera {
            alignment_speed: 1.0,
            current_up: Vec3::Y,
        };
        let target_up = (-gravity.0).normalize();
        grav_cam.current_up = grav_cam.current_up.lerp(target_up, 1.0).normalize();
        assert!((grav_cam.current_up - Vec3::Y).length() < 1e-6);

        // At the "south pole": entity at (0, -R, 0). Gravity = +Y. Up = -Y.
        let gravity_south = GravityDirection(Vec3::Y);
        let target_up_south = (-gravity_south.0).normalize();
        grav_cam.current_up = grav_cam.current_up.lerp(target_up_south, 1.0).normalize();
        assert!((grav_cam.current_up - Vec3::NEG_Y).length() < 1e-6);
    }

    #[test]
    fn test_transition_from_flat_to_curved_is_smooth() {
        // Simulate 50 ticks of smooth alignment from +Y up to a 45-degree tilt
        let mut grav_cam = GravityOrientedCamera {
            alignment_speed: 0.1,
            current_up: Vec3::Y,
        };
        let target_up = Vec3::new(1.0, 1.0, 0.0).normalize(); // 45 degrees

        let mut prev_angle = 0.0_f32;
        for tick in 0..50 {
            grav_cam.current_up = grav_cam.current_up
                .lerp(target_up, grav_cam.alignment_speed)
                .normalize();
            let angle = grav_cam.current_up.angle_between(Vec3::Y);
            // Angle should be monotonically increasing (converging toward target)
            assert!(
                angle >= prev_angle - 1e-6,
                "Angle decreased at tick {tick}: {angle} < {prev_angle}"
            );
            prev_angle = angle;
        }
        // After 50 ticks with 0.1 speed, should be very close to target
        let final_angle = grav_cam.current_up.angle_between(target_up);
        assert!(final_angle < 0.01, "Should have converged: angle = {final_angle}");
    }

    #[test]
    fn test_zero_gravity_preserves_current_up() {
        // In deep space with no gravity, the up vector should not change.
        let mut grav_cam = GravityOrientedCamera {
            alignment_speed: 0.1,
            current_up: Vec3::new(0.5, 0.8, 0.3).normalize(),
        };
        let saved_up = grav_cam.current_up;

        // Zero gravity: target_up would be (0, 0, 0), so the system skips alignment
        let gravity = GravityDirection(Vec3::ZERO);
        let target_up = -gravity.0;
        if target_up.length_squared() > 1e-6 {
            grav_cam.current_up = grav_cam.current_up
                .lerp(target_up.normalize(), grav_cam.alignment_speed)
                .normalize();
        }
        // Up vector should be unchanged
        assert!((grav_cam.current_up - saved_up).length() < 1e-6);
    }

    #[test]
    fn test_alignment_speed_configurable() {
        // Fast alignment (0.5) converges much quicker than slow (0.05)
        let target_up = Vec3::X;

        let mut fast = GravityOrientedCamera {
            alignment_speed: 0.5,
            current_up: Vec3::Y,
        };
        let mut slow = GravityOrientedCamera {
            alignment_speed: 0.05,
            current_up: Vec3::Y,
        };

        for _ in 0..10 {
            fast.current_up = fast.current_up.lerp(target_up, fast.alignment_speed).normalize();
            slow.current_up = slow.current_up.lerp(target_up, slow.alignment_speed).normalize();
        }

        let fast_error = fast.current_up.angle_between(target_up);
        let slow_error = slow.current_up.angle_between(target_up);
        assert!(fast_error < slow_error, "Fast alignment should converge sooner");
    }

    #[test]
    fn test_arbitrary_gravity_direction() {
        // Gravity pointing in a diagonal direction (e.g., on a cubesphere edge)
        let gravity = GravityDirection(Vec3::new(-0.707, -0.707, 0.0)); // 45 deg between -X and -Y
        let target_up = (-gravity.0).normalize();
        assert!((target_up.x - 0.707).abs() < 0.01);
        assert!((target_up.y - 0.707).abs() < 0.01);
        assert!((target_up.z).abs() < 1e-6);
    }
}
```
