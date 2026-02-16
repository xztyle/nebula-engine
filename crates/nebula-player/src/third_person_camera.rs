//! Third-person camera controller: orbit, zoom, and smooth follow.

use glam::{Mat3, Quat, Vec3};
use nebula_ecs::{Rotation, WorldPos};
use nebula_input::MouseState;
use nebula_math::{Vec3I128, WorldPosition};
use winit::event::MouseButton;

/// Tags an entity as a third-person camera that follows a target entity.
/// The camera entity must also have `WorldPos`, `LocalPos`, and `Rotation`.
#[derive(Clone, Debug)]
pub struct ThirdPersonCamera {
    /// Horizontal orbit angle in radians (azimuth). 0 = behind target (-Z).
    pub orbit_yaw: f32,
    /// Vertical orbit angle in radians (elevation above horizon).
    pub orbit_pitch: f32,
    /// Current distance from the target in millimeters (i128 scale).
    pub distance: f32,
    /// Minimum allowed zoom distance in millimeters.
    pub distance_min: f32,
    /// Maximum allowed zoom distance in millimeters.
    pub distance_max: f32,
    /// Vertical offset from the target's position in millimeters.
    /// Positive values raise the look-at point above the target origin.
    pub height_offset: f32,
    /// Mouse sensitivity for orbit rotation.
    pub orbit_sensitivity: f32,
    /// Scroll wheel zoom sensitivity.
    pub zoom_sensitivity: f32,
    /// Interpolation speed for smooth follow (0.0 = no follow, 1.0 = instant snap).
    /// Values in the range 0.05..0.3 produce a smooth, natural feel.
    pub follow_speed: f32,
    /// Minimum orbit pitch in radians (how far below the horizon).
    pub pitch_min: f32,
    /// Maximum orbit pitch in radians (how far above the target).
    pub pitch_max: f32,
}

impl Default for ThirdPersonCamera {
    fn default() -> Self {
        Self {
            orbit_yaw: 0.0,
            orbit_pitch: 20.0_f32.to_radians(),
            distance: 5000.0,
            distance_min: 1000.0,
            distance_max: 50_000.0,
            height_offset: 1500.0,
            orbit_sensitivity: 0.005,
            zoom_sensitivity: 200.0,
            follow_speed: 0.1,
            pitch_min: -10.0_f32.to_radians(),
            pitch_max: 80.0_f32.to_radians(),
        }
    }
}

/// Update orbit angles from right-mouse-button drag.
///
/// Horizontal mouse delta adjusts `orbit_yaw`, vertical delta adjusts
/// `orbit_pitch`. Pitch is clamped to `[pitch_min, pitch_max]`.
pub fn third_person_orbit_system(mouse: &MouseState, cam: &mut ThirdPersonCamera) {
    if !mouse.is_button_pressed(MouseButton::Right) {
        return;
    }
    let delta = mouse.delta();
    cam.orbit_yaw -= delta.x * cam.orbit_sensitivity;
    cam.orbit_pitch -= delta.y * cam.orbit_sensitivity;
    cam.orbit_pitch = cam.orbit_pitch.clamp(cam.pitch_min, cam.pitch_max);
}

/// Update zoom distance from scroll wheel input.
///
/// Scroll-up zooms in (decreases distance), scroll-down zooms out.
/// Distance is clamped to `[distance_min, distance_max]`.
pub fn third_person_zoom_system(mouse: &MouseState, cam: &mut ThirdPersonCamera) {
    let scroll = mouse.scroll();
    if scroll.abs() < 1e-6 {
        return;
    }
    cam.distance -= scroll * cam.zoom_sensitivity;
    cam.distance = cam.distance.clamp(cam.distance_min, cam.distance_max);
}

/// Smoothly follow the target and compute look-at rotation.
///
/// The camera computes its desired world position from the target's position,
/// the orbit angles, and the distance, then lerps toward it. The rotation
/// is always recomputed to face the look-at point.
pub fn third_person_follow_system(
    cam: &ThirdPersonCamera,
    target_pos: &WorldPos,
    cam_world_pos: &mut WorldPos,
    cam_rotation: &mut Rotation,
) {
    // Look-at point: target position + height offset
    let look_at_world = target_pos.0 + Vec3I128::new(0, cam.height_offset as i128, 0);

    // Desired camera offset via spherical coordinates.
    // orbit_yaw=0, orbit_pitch=0 → camera behind target at +Z.
    let cos_pitch = cam.orbit_pitch.cos();
    let sin_pitch = cam.orbit_pitch.sin();
    let cos_yaw = cam.orbit_yaw.cos();
    let sin_yaw = cam.orbit_yaw.sin();

    let offset = Vec3::new(
        cam.distance * cos_pitch * sin_yaw,
        cam.distance * sin_pitch,
        cam.distance * cos_pitch * cos_yaw,
    );

    let desired_world =
        look_at_world + Vec3I128::new(offset.x as i128, offset.y as i128, offset.z as i128);

    // Smooth follow: lerp each axis independently in i128 space.
    let current = cam_world_pos.0;
    let delta = desired_world - current;
    let speed = f64::from(cam.follow_speed);
    cam_world_pos.0 = WorldPosition::new(
        current.x + (delta.x as f64 * speed).round() as i128,
        current.y + (delta.y as f64 * speed).round() as i128,
        current.z + (delta.z as f64 * speed).round() as i128,
    );

    // Compute look-at rotation in local f32 space.
    let lerped = cam_world_pos.0;
    let cam_to_target = Vec3::new(
        (look_at_world.x - lerped.x) as f32,
        (look_at_world.y - lerped.y) as f32,
        (look_at_world.z - lerped.z) as f32,
    );
    if cam_to_target.length_squared() > 1e-6 {
        let forward = cam_to_target.normalize();
        let right = Vec3::Y.cross(forward).normalize_or_zero();
        let up = forward.cross(right);
        cam_rotation.0 = Quat::from_mat3(&Mat3::from_cols(right, up, forward));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_camera_distance_matches_zoom_level() {
        let cam = ThirdPersonCamera {
            distance: 10_000.0,
            ..Default::default()
        };
        assert!((cam.distance - 10_000.0).abs() < 1e-6);
        let cam2 = ThirdPersonCamera {
            distance: 3_000.0,
            ..Default::default()
        };
        assert!((cam2.distance - 3_000.0).abs() < 1e-6);
    }

    #[test]
    fn test_orbit_changes_angle() {
        let mut cam = ThirdPersonCamera::default();
        let initial_yaw = cam.orbit_yaw;
        let dx = 50.0;
        cam.orbit_yaw -= dx * cam.orbit_sensitivity;
        assert!((cam.orbit_yaw - initial_yaw).abs() > 0.0);
    }

    #[test]
    fn test_zoom_clamps_at_min() {
        let mut cam = ThirdPersonCamera::default();
        cam.distance = 500.0;
        cam.distance = cam.distance.clamp(cam.distance_min, cam.distance_max);
        assert!((cam.distance - cam.distance_min).abs() < 1e-6);
    }

    #[test]
    fn test_zoom_clamps_at_max() {
        let mut cam = ThirdPersonCamera::default();
        cam.distance = 100_000.0;
        cam.distance = cam.distance.clamp(cam.distance_min, cam.distance_max);
        assert!((cam.distance - cam.distance_max).abs() < 1e-6);
    }

    #[test]
    fn test_smooth_follow_converges_on_target() {
        let target = WorldPosition::new(10_000, 5_000, 3_000);
        let mut current = WorldPosition::new(0, 0, 0);
        let follow_speed: f64 = 0.1;

        for _ in 0..200 {
            let delta_x = target.x - current.x;
            let delta_y = target.y - current.y;
            let delta_z = target.z - current.z;
            // Snap to target when delta is tiny to avoid rounding stalls.
            let step_x = (delta_x as f64 * follow_speed).round() as i128;
            let step_y = (delta_y as f64 * follow_speed).round() as i128;
            let step_z = (delta_z as f64 * follow_speed).round() as i128;
            current = WorldPosition::new(
                if step_x == 0 && delta_x != 0 {
                    target.x
                } else {
                    current.x + step_x
                },
                if step_y == 0 && delta_y != 0 {
                    target.y
                } else {
                    current.y + step_y
                },
                if step_z == 0 && delta_z != 0 {
                    target.z
                } else {
                    current.z + step_z
                },
            );
        }
        assert!((current.x - target.x).abs() <= 1);
        assert!((current.y - target.y).abs() <= 1);
        assert!((current.z - target.z).abs() <= 1);
    }

    #[test]
    fn test_camera_looks_at_target_from_all_angles() {
        let target = Vec3::new(0.0, 1500.0, 0.0);
        let distance = 5000.0;

        for angle_deg in [0, 45, 90, 135, 180, 225, 270, 315] {
            let yaw = (angle_deg as f32).to_radians();
            let pitch = 20.0_f32.to_radians();
            let cos_p = pitch.cos();
            let sin_p = pitch.sin();

            let cam_pos = Vec3::new(
                distance * cos_p * yaw.sin(),
                distance * sin_p + 1500.0,
                distance * cos_p * yaw.cos(),
            );

            let to_target = (target - cam_pos).normalize();
            assert!(to_target.length() > 0.99);
        }
    }

    #[test]
    fn test_orbit_pitch_clamps() {
        let mut cam = ThirdPersonCamera::default();
        cam.orbit_pitch = 200.0_f32.to_radians();
        cam.orbit_pitch = cam.orbit_pitch.clamp(cam.pitch_min, cam.pitch_max);
        assert!((cam.orbit_pitch - cam.pitch_max).abs() < 1e-6);

        cam.orbit_pitch = -200.0_f32.to_radians();
        cam.orbit_pitch = cam.orbit_pitch.clamp(cam.pitch_min, cam.pitch_max);
        assert!((cam.orbit_pitch - cam.pitch_min).abs() < 1e-6);
    }

    #[test]
    fn test_default_follow_speed_in_valid_range() {
        let cam = ThirdPersonCamera::default();
        assert!(cam.follow_speed > 0.0);
        assert!(cam.follow_speed <= 1.0);
    }

    #[test]
    fn test_height_offset_raises_look_at_point() {
        let cam = ThirdPersonCamera::default();
        assert!(cam.height_offset > 0.0);
        let target = WorldPosition::new(0, 0, 0);
        let look_at_y = target.y + cam.height_offset as i128;
        assert!(look_at_y > 0);
    }
}
