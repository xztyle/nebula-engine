//! Gravity-oriented camera: aligns the camera's up vector to the local
//! anti-gravity direction, enabling natural navigation over cubesphere planets.

use glam::{Mat3, Quat, Vec3};
use nebula_ecs::Rotation;

use crate::first_person_camera::FirstPersonCamera;

/// The normalized gravity direction at the entity's position.
/// Points toward the planet center (i.e., "down" in local terms).
/// Set by the gravity system each tick based on the entity's `WorldPos`
/// and the nearest planet's center.
#[derive(Clone, Copy, Debug)]
pub struct GravityDirection(pub Vec3);

impl Default for GravityDirection {
    fn default() -> Self {
        Self(Vec3::NEG_Y)
    }
}

/// Configures gravity-based up-vector alignment for a camera entity.
/// Works alongside `FirstPersonCamera` or `ThirdPersonCamera` — it modifies
/// the up vector used by those controllers rather than replacing them.
#[derive(Clone, Debug)]
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

/// Lerps the camera's effective up vector toward the anti-gravity direction.
/// Skips alignment when gravity is zero (deep space).
pub fn gravity_up_alignment_system(
    gravity: &GravityDirection,
    grav_cam: &mut GravityOrientedCamera,
) {
    let target_up = -gravity.0;

    // Guard against zero-length gravity (e.g., in deep space between planets).
    if target_up.length_squared() < 1e-6 {
        return;
    }

    let target_up = target_up.normalize();

    // Lerp + normalize produces a smooth rotation of the up vector.
    let new_up = grav_cam
        .current_up
        .lerp(target_up, grav_cam.alignment_speed)
        .normalize_or_zero();

    if new_up.length_squared() > 0.5 {
        grav_cam.current_up = new_up;
    }
}

/// Rebuilds the camera rotation quaternion so that its local +Y aligns with
/// the gravity-aligned up vector while preserving the player's intended look
/// direction. Reapplies pitch for first-person cameras.
///
/// Pass `Some(&fps_cam)` when the entity also has a `FirstPersonCamera` to
/// reapply pitch relative to the local horizon.
pub fn gravity_orient_rotation_system(
    grav_cam: &GravityOrientedCamera,
    rotation: &mut Rotation,
    fps_cam: Option<&FirstPersonCamera>,
) {
    let up = grav_cam.current_up;

    // Extract the current forward direction from the camera's rotation.
    let current_forward = rotation.0 * Vec3::NEG_Z;

    // Project forward onto the plane perpendicular to the new up vector.
    let forward_on_plane = (current_forward - up * current_forward.dot(up)).normalize_or_zero();

    if forward_on_plane.length_squared() < 0.5 {
        // Camera is looking straight up or down along the gravity axis.
        return;
    }

    // Reconstruct the rotation from the new basis vectors.
    let right = forward_on_plane.cross(up).normalize();
    let corrected_up = right.cross(forward_on_plane).normalize();

    // Build a look rotation: forward = forward_on_plane, up = corrected_up
    let basis = Mat3::from_cols(right, corrected_up, -forward_on_plane);
    let base_rotation = Quat::from_mat3(&basis).normalize();

    // If this is a first-person camera, reapply pitch on top of the
    // gravity-aligned base rotation.
    if let Some(fps) = fps_cam {
        let pitch_quat = Quat::from_axis_angle(right, fps.pitch);
        rotation.0 = (pitch_quat * base_rotation).normalize();
    } else {
        rotation.0 = base_rotation;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn test_up_vector_points_away_from_planet_center() {
        let gravity = GravityDirection(Vec3::NEG_Y);
        let target_up = -gravity.0;
        assert!((target_up - Vec3::Y).length() < 1e-6);
    }

    #[test]
    fn test_walking_over_surface_keeps_horizon_level() {
        let mut grav_cam = GravityOrientedCamera {
            alignment_speed: 1.0,
            current_up: Vec3::Y,
        };

        let gravity_1 = GravityDirection(Vec3::NEG_Y);
        let target_up_1 = (-gravity_1.0).normalize();
        grav_cam.current_up = grav_cam.current_up.lerp(target_up_1, 1.0).normalize();
        assert!((grav_cam.current_up - Vec3::Y).length() < 1e-4);

        let gravity_2 = GravityDirection(Vec3::NEG_X);
        let target_up_2 = (-gravity_2.0).normalize();
        grav_cam.current_up = grav_cam.current_up.lerp(target_up_2, 1.0).normalize();
        assert!((grav_cam.current_up - Vec3::X).length() < 1e-4);
    }

    #[test]
    fn test_up_vector_changes_smoothly_not_snapping() {
        let mut grav_cam = GravityOrientedCamera {
            alignment_speed: 0.1,
            current_up: Vec3::Y,
        };

        let target_up = Vec3::X;
        grav_cam.current_up = grav_cam
            .current_up
            .lerp(target_up, grav_cam.alignment_speed)
            .normalize();

        assert!(
            grav_cam.current_up.y > 0.5,
            "Still mostly +Y after one tick"
        );
        assert!(grav_cam.current_up.x > 0.0, "Started leaning toward +X");
        assert!(
            (grav_cam.current_up - Vec3::X).length() > 0.1,
            "Not yet at +X"
        );
    }

    #[test]
    fn test_at_pole_up_vector_is_correct() {
        let gravity = GravityDirection(Vec3::NEG_Y);
        let mut grav_cam = GravityOrientedCamera {
            alignment_speed: 1.0,
            current_up: Vec3::Y,
        };
        let target_up = (-gravity.0).normalize();
        grav_cam.current_up = grav_cam.current_up.lerp(target_up, 1.0).normalize();
        assert!((grav_cam.current_up - Vec3::Y).length() < 1e-6);

        let gravity_south = GravityDirection(Vec3::Y);
        let target_up_south = (-gravity_south.0).normalize();
        grav_cam.current_up = grav_cam.current_up.lerp(target_up_south, 1.0).normalize();
        assert!((grav_cam.current_up - Vec3::NEG_Y).length() < 1e-6);
    }

    #[test]
    fn test_transition_from_flat_to_curved_is_smooth() {
        let mut grav_cam = GravityOrientedCamera {
            alignment_speed: 0.1,
            current_up: Vec3::Y,
        };
        let target_up = Vec3::new(1.0, 1.0, 0.0).normalize();

        let mut prev_angle = 0.0_f32;
        for tick in 0..50 {
            grav_cam.current_up = grav_cam
                .current_up
                .lerp(target_up, grav_cam.alignment_speed)
                .normalize();
            let angle = grav_cam.current_up.angle_between(Vec3::Y);
            assert!(
                angle >= prev_angle - 1e-6,
                "Angle decreased at tick {tick}: {angle} < {prev_angle}"
            );
            prev_angle = angle;
        }
        let final_angle = grav_cam.current_up.angle_between(target_up);
        assert!(
            final_angle < 0.01,
            "Should have converged: angle = {final_angle}"
        );
    }

    #[test]
    fn test_zero_gravity_preserves_current_up() {
        let mut grav_cam = GravityOrientedCamera {
            alignment_speed: 0.1,
            current_up: Vec3::new(0.5, 0.8, 0.3).normalize(),
        };
        let saved_up = grav_cam.current_up;

        let gravity = GravityDirection(Vec3::ZERO);
        let target_up = -gravity.0;
        if target_up.length_squared() > 1e-6 {
            grav_cam.current_up = grav_cam
                .current_up
                .lerp(target_up.normalize(), grav_cam.alignment_speed)
                .normalize();
        }
        assert!((grav_cam.current_up - saved_up).length() < 1e-6);
    }

    #[test]
    fn test_alignment_speed_configurable() {
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
            fast.current_up = fast
                .current_up
                .lerp(target_up, fast.alignment_speed)
                .normalize();
            slow.current_up = slow
                .current_up
                .lerp(target_up, slow.alignment_speed)
                .normalize();
        }

        let fast_error = fast.current_up.angle_between(target_up);
        let slow_error = slow.current_up.angle_between(target_up);
        assert!(
            fast_error < slow_error,
            "Fast alignment should converge sooner"
        );
    }

    #[test]
    fn test_arbitrary_gravity_direction() {
        let gravity = GravityDirection(Vec3::new(-0.707, -0.707, 0.0));
        let target_up = (-gravity.0).normalize();
        assert!((target_up.x - 0.707).abs() < 0.01);
        assert!((target_up.y - 0.707).abs() < 0.01);
        assert!((target_up.z).abs() < 1e-6);
    }
}
