//! First-person camera controller: mouse look and WASD movement.

use glam::{Quat, Vec3};
use nebula_ecs::{Rotation, WorldPos};
use nebula_input::{KeyboardState, MouseState};
use nebula_math::Vec3I128;
use winit::keyboard::{KeyCode, PhysicalKey};

/// Marker component that tags an entity as a first-person camera.
/// The entity must also have `WorldPos`, `LocalPos`, and `Rotation` components.
#[derive(Clone, Debug)]
pub struct FirstPersonCamera {
    /// Horizontal rotation in radians. Positive yaw rotates left (counter-clockwise
    /// when viewed from above), matching right-handed coordinate conventions.
    pub yaw: f32,
    /// Vertical rotation in radians. Positive pitch looks up.
    pub pitch: f32,
    /// Mouse sensitivity multiplier applied to raw mouse delta.
    pub mouse_sensitivity: f32,
    /// Movement speed in millimeters per tick (i128 units).
    pub move_speed: i128,
    /// Maximum pitch angle in radians. Clamped to ±pitch_limit.
    pub pitch_limit: f32,
}

impl Default for FirstPersonCamera {
    fn default() -> Self {
        Self {
            yaw: 0.0,
            pitch: 0.0,
            mouse_sensitivity: 0.003,
            move_speed: 100,
            pitch_limit: 89.0_f32.to_radians(),
        }
    }
}

impl FirstPersonCamera {
    /// Compute the rotation quaternion from the current yaw and pitch.
    /// Negated yaw around Y, then pitch around X. The negation maps
    /// positive yaw (counter-clockwise from above) to the correct rotation
    /// direction in glam's right-handed coordinate system.
    #[must_use]
    pub fn rotation(&self) -> Quat {
        Quat::from_rotation_y(-self.yaw) * Quat::from_rotation_x(self.pitch)
    }

    /// Apply mouse delta to yaw and pitch, clamping pitch to the configured limit.
    pub fn apply_mouse_delta(&mut self, dx: f32, dy: f32) {
        self.yaw -= dx * self.mouse_sensitivity;
        self.pitch -= dy * self.mouse_sensitivity;
        self.pitch = self.pitch.clamp(-self.pitch_limit, self.pitch_limit);
    }
}

/// Update yaw/pitch from mouse delta and write the resulting rotation.
///
/// Call once per frame with the current `MouseState`. Updates `cam` fields
/// in-place and writes the quaternion into `rotation`.
pub fn first_person_look_system(
    mouse: &MouseState,
    cam: &mut FirstPersonCamera,
    rotation: &mut Rotation,
) {
    let delta = mouse.delta();
    cam.apply_mouse_delta(delta.x, delta.y);
    rotation.0 = cam.rotation();
}

/// Move the camera on the horizontal plane based on WASD keys.
///
/// Forward/back and strafe directions are derived from the camera's current
/// rotation projected onto Y=0 so that movement is always horizontal
/// regardless of pitch.
pub fn first_person_move_system(
    keyboard: &KeyboardState,
    cam: &FirstPersonCamera,
    rotation: &Rotation,
    world_pos: &mut WorldPos,
) {
    let forward_f32 = rotation.0 * Vec3::NEG_Z;
    let right_f32 = rotation.0 * Vec3::X;

    let forward_horiz = Vec3::new(forward_f32.x, 0.0, forward_f32.z).normalize_or_zero();
    let right_horiz = Vec3::new(right_f32.x, 0.0, right_f32.z).normalize_or_zero();

    let mut direction = Vec3::ZERO;
    if keyboard.is_pressed(PhysicalKey::Code(KeyCode::KeyW)) {
        direction += forward_horiz;
    }
    if keyboard.is_pressed(PhysicalKey::Code(KeyCode::KeyS)) {
        direction -= forward_horiz;
    }
    if keyboard.is_pressed(PhysicalKey::Code(KeyCode::KeyD)) {
        direction += right_horiz;
    }
    if keyboard.is_pressed(PhysicalKey::Code(KeyCode::KeyA)) {
        direction -= right_horiz;
    }

    if direction.length_squared() > 0.0 {
        direction = direction.normalize();
    }

    let displacement = Vec3I128::new(
        (direction.x * cam.move_speed as f32) as i128,
        (direction.y * cam.move_speed as f32) as i128,
        (direction.z * cam.move_speed as f32) as i128,
    );

    world_pos.0 = world_pos.0 + displacement;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_2;

    /// Helper: compute the forward vector for a given yaw and pitch.
    fn forward_from(cam: &FirstPersonCamera) -> Vec3 {
        cam.rotation() * Vec3::NEG_Z
    }

    #[test]
    fn test_mouse_x_rotates_yaw() {
        let mut cam = FirstPersonCamera::default();
        let initial_yaw = cam.yaw;
        let dx = 100.0;
        cam.yaw -= dx * cam.mouse_sensitivity;
        assert!((cam.yaw - initial_yaw).abs() > 0.0);
        assert!(cam.yaw < initial_yaw);
    }

    #[test]
    fn test_mouse_y_rotates_pitch() {
        let mut cam = FirstPersonCamera::default();
        let initial_pitch = cam.pitch;
        let dy = 100.0;
        cam.pitch -= dy * cam.mouse_sensitivity;
        assert!((cam.pitch - initial_pitch).abs() > 0.0);
        assert!(cam.pitch < initial_pitch);
    }

    #[test]
    fn test_pitch_clamps_at_limits() {
        let mut cam = FirstPersonCamera::default();
        cam.pitch = 200.0_f32.to_radians();
        cam.pitch = cam.pitch.clamp(-cam.pitch_limit, cam.pitch_limit);
        assert!((cam.pitch - cam.pitch_limit).abs() < 1e-6);

        cam.pitch = -200.0_f32.to_radians();
        cam.pitch = cam.pitch.clamp(-cam.pitch_limit, cam.pitch_limit);
        assert!((cam.pitch + cam.pitch_limit).abs() < 1e-6);
    }

    #[test]
    fn test_wasd_movement_relative_to_facing() {
        let cam_default = FirstPersonCamera::default();
        let fwd_default = forward_from(&cam_default);
        assert!(fwd_default.z < -0.9);

        let cam_rotated = FirstPersonCamera {
            yaw: FRAC_PI_2,
            ..Default::default()
        };
        let fwd_rotated = forward_from(&cam_rotated);
        assert!(fwd_rotated.x > 0.9);
    }

    #[test]
    fn test_initial_state_looks_forward_along_neg_z() {
        let cam = FirstPersonCamera::default();
        assert_eq!(cam.yaw, 0.0);
        assert_eq!(cam.pitch, 0.0);
        let forward = forward_from(&cam);
        assert!((forward.x).abs() < 1e-6);
        assert!((forward.y).abs() < 1e-6);
        assert!((forward.z + 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_default_sensitivity_is_positive() {
        let cam = FirstPersonCamera::default();
        assert!(cam.mouse_sensitivity > 0.0);
    }

    #[test]
    fn test_default_move_speed_is_positive() {
        let cam = FirstPersonCamera::default();
        assert!(cam.move_speed > 0);
    }

    #[test]
    fn test_yaw_wraps_continuously() {
        let cam = FirstPersonCamera {
            yaw: 10.0 * std::f32::consts::PI,
            ..Default::default()
        };
        let fwd = forward_from(&cam);
        assert!((fwd.z + 1.0).abs() < 1e-4);
    }

    #[test]
    fn test_pitch_limit_prevents_gimbal_lock() {
        let cam = FirstPersonCamera::default();
        assert!(cam.pitch_limit < FRAC_PI_2);
        assert!(cam.pitch_limit > 0.0);
    }
}
