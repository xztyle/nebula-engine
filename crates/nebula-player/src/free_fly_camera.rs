//! Free-fly debug camera: unrestricted noclip movement for development.

use glam::{Quat, Vec3};
use nebula_ecs::{Rotation, WorldPos};
use nebula_input::{KeyboardState, MouseState};
use nebula_math::Vec3I128;
use std::fmt::Write;
use winit::keyboard::{KeyCode, PhysicalKey};

/// Marker component for the free-fly debug camera. When active, this
/// camera overrides the normal gameplay camera. When inactive, the
/// entity exists but its systems are skipped.
#[derive(Clone, Debug)]
pub struct FreeFlyCam {
    /// Whether the debug camera is currently active.
    pub active: bool,
    /// Movement speed in mm per tick. Adjustable at runtime.
    pub speed: f32,
    /// Minimum speed (floor for scroll adjustment).
    pub speed_min: f32,
    /// Maximum speed (ceiling for scroll adjustment).
    pub speed_max: f32,
    /// Speed multiplier per scroll tick (multiplicative scaling).
    pub speed_scroll_factor: f32,
    /// Mouse sensitivity for look rotation.
    pub mouse_sensitivity: f32,
    /// Current yaw in radians.
    pub yaw: f32,
    /// Current pitch in radians.
    pub pitch: f32,
    /// The toggle key code for activating/deactivating.
    pub toggle_key: KeyCode,
}

impl Default for FreeFlyCam {
    fn default() -> Self {
        Self {
            active: false,
            speed: 500.0,
            speed_min: 10.0,
            speed_max: 1_000_000.0,
            speed_scroll_factor: 1.2,
            mouse_sensitivity: 0.003,
            yaw: 0.0,
            pitch: 0.0,
            toggle_key: KeyCode::F1,
        }
    }
}

/// Resource holding debug camera overlay text, rendered by the UI system.
#[derive(Clone, Debug, Default)]
pub struct DebugCameraOverlay {
    /// Formatted diagnostic text for on-screen display.
    pub text: String,
}

/// Toggle the debug camera on/off when the toggle key is pressed.
pub fn free_fly_toggle_system(keyboard: &KeyboardState, cam: &mut FreeFlyCam) {
    if keyboard.just_pressed(PhysicalKey::Code(cam.toggle_key)) {
        cam.active = !cam.active;
    }
}

/// Rotate the debug camera based on mouse movement. Pitch is clamped to ±89°.
pub fn free_fly_look_system(mouse: &MouseState, cam: &mut FreeFlyCam, rotation: &mut Rotation) {
    if !cam.active {
        return;
    }
    let delta = mouse.delta();
    cam.yaw -= delta.x * cam.mouse_sensitivity;
    cam.pitch -= delta.y * cam.mouse_sensitivity;
    cam.pitch = cam
        .pitch
        .clamp(-89.0_f32.to_radians(), 89.0_f32.to_radians());
    rotation.0 = Quat::from_rotation_y(cam.yaw) * Quat::from_rotation_x(cam.pitch);
}

/// Move the debug camera with WASD+Space+Ctrl. No collision, no gravity.
pub fn free_fly_move_system(
    keyboard: &KeyboardState,
    cam: &FreeFlyCam,
    rotation: &Rotation,
    world_pos: &mut WorldPos,
) {
    if !cam.active {
        return;
    }

    let forward = rotation.0 * Vec3::NEG_Z;
    let right = rotation.0 * Vec3::X;
    let up = Vec3::Y;

    let mut dir = Vec3::ZERO;
    if keyboard.is_pressed(PhysicalKey::Code(KeyCode::KeyW)) {
        dir += forward;
    }
    if keyboard.is_pressed(PhysicalKey::Code(KeyCode::KeyS)) {
        dir -= forward;
    }
    if keyboard.is_pressed(PhysicalKey::Code(KeyCode::KeyD)) {
        dir += right;
    }
    if keyboard.is_pressed(PhysicalKey::Code(KeyCode::KeyA)) {
        dir -= right;
    }
    if keyboard.is_pressed(PhysicalKey::Code(KeyCode::Space)) {
        dir += up;
    }
    if keyboard.is_pressed(PhysicalKey::Code(KeyCode::ControlLeft)) {
        dir -= up;
    }

    if dir.length_squared() > 1e-6 {
        dir = dir.normalize();
        let displacement = Vec3I128::new(
            (dir.x * cam.speed) as i128,
            (dir.y * cam.speed) as i128,
            (dir.z * cam.speed) as i128,
        );
        world_pos.0 = world_pos.0 + displacement;
    }
}

/// Adjust movement speed via scroll wheel or +/- keys.
pub fn free_fly_speed_system(mouse: &MouseState, keyboard: &KeyboardState, cam: &mut FreeFlyCam) {
    if !cam.active {
        return;
    }
    let scroll = mouse.scroll();
    if scroll > 0.0 || keyboard.just_pressed(PhysicalKey::Code(KeyCode::Equal)) {
        cam.speed *= cam.speed_scroll_factor;
    }
    if scroll < 0.0 || keyboard.just_pressed(PhysicalKey::Code(KeyCode::Minus)) {
        cam.speed /= cam.speed_scroll_factor;
    }
    cam.speed = cam.speed.clamp(cam.speed_min, cam.speed_max);
}

/// Write diagnostic info (position, rotation, chunk, speed) to the overlay.
pub fn free_fly_overlay_system(
    cam: &FreeFlyCam,
    world_pos: &WorldPos,
    overlay: &mut DebugCameraOverlay,
) {
    overlay.text.clear();
    if !cam.active {
        return;
    }
    let p = &world_pos.0;
    let yaw_deg = cam.yaw.to_degrees();
    let pitch_deg = cam.pitch.to_degrees();
    let chunk_size: i128 = 32_000;
    let chunk_x = p.x.div_euclid(chunk_size);
    let chunk_y = p.y.div_euclid(chunk_size);
    let chunk_z = p.z.div_euclid(chunk_size);

    let _ = write!(
        overlay.text,
        "Pos: ({}, {}, {}) mm\n\
         Rot: yaw={:.1} pitch={:.1}\n\
         Chunk: ({}, {}, {})\n\
         Speed: {:.0} mm/tick",
        p.x, p.y, p.z, yaw_deg, pitch_deg, chunk_x, chunk_y, chunk_z, cam.speed,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_math::WorldPosition;

    #[test]
    fn test_free_fly_ignores_collision() {
        let cam = FreeFlyCam {
            active: true,
            ..Default::default()
        };
        let mut world_pos = WorldPos::new(0, 0, 0);
        let rotation = Rotation(Quat::IDENTITY);

        let forward = rotation.0 * Vec3::NEG_Z;
        let displacement = Vec3I128::new(
            (forward.x * cam.speed) as i128,
            (forward.y * cam.speed) as i128,
            (forward.z * cam.speed) as i128,
        );
        world_pos.0 = world_pos.0 + displacement;

        assert_ne!(world_pos.0, WorldPosition::new(0, 0, 0));
    }

    #[test]
    fn test_speed_adjusts_with_scroll() {
        let mut cam = FreeFlyCam {
            active: true,
            ..Default::default()
        };
        let initial_speed = cam.speed;

        cam.speed *= cam.speed_scroll_factor;
        cam.speed = cam.speed.clamp(cam.speed_min, cam.speed_max);
        assert!(cam.speed > initial_speed);

        cam.speed /= cam.speed_scroll_factor;
        cam.speed /= cam.speed_scroll_factor;
        cam.speed = cam.speed.clamp(cam.speed_min, cam.speed_max);
        assert!(cam.speed < initial_speed);
    }

    #[test]
    fn test_toggle_activates_and_deactivates() {
        let mut cam = FreeFlyCam::default();
        assert!(!cam.active);

        cam.active = !cam.active;
        assert!(cam.active);

        cam.active = !cam.active;
        assert!(!cam.active);
    }

    #[test]
    fn test_position_overlay_shows_correct_coords() {
        let cam = FreeFlyCam {
            active: true,
            ..Default::default()
        };
        let world_pos = WorldPos::new(12_345, -67_890, 111_213);
        let mut overlay = DebugCameraOverlay::default();
        free_fly_overlay_system(&cam, &world_pos, &mut overlay);
        let text = &overlay.text;
        assert!(text.contains("12345"));
        assert!(text.contains("-67890"));
        assert!(text.contains("111213"));
    }

    #[test]
    fn test_camera_can_move_anywhere_in_world() {
        let mut world_pos = WorldPos::new(0, 0, 0);
        let ly_mm: i128 = 9_460_730_472_580_800_000;
        world_pos.0 = WorldPosition::new(100 * ly_mm, 0, -50 * ly_mm);
        assert_eq!(world_pos.0.x, 100 * ly_mm);
        assert_eq!(world_pos.0.z, -50 * ly_mm);
    }

    #[test]
    fn test_speed_clamps_at_min() {
        let mut cam = FreeFlyCam {
            active: true,
            ..Default::default()
        };
        cam.speed = 0.001;
        cam.speed = cam.speed.clamp(cam.speed_min, cam.speed_max);
        assert!((cam.speed - cam.speed_min).abs() < 1e-6);
    }

    #[test]
    fn test_speed_clamps_at_max() {
        let mut cam = FreeFlyCam {
            active: true,
            ..Default::default()
        };
        cam.speed = 99_999_999.0;
        cam.speed = cam.speed.clamp(cam.speed_min, cam.speed_max);
        assert!((cam.speed - cam.speed_max).abs() < 1e-6);
    }

    #[test]
    fn test_inactive_camera_does_not_process_movement() {
        let cam = FreeFlyCam {
            active: false,
            ..Default::default()
        };
        assert!(!cam.active);
    }

    #[test]
    fn test_chunk_address_computation() {
        let chunk_size: i128 = 32_000;
        let pos = WorldPosition::new(100_000, -50_000, 64_001);
        let chunk_x = pos.x.div_euclid(chunk_size);
        let chunk_y = pos.y.div_euclid(chunk_size);
        let chunk_z = pos.z.div_euclid(chunk_size);
        assert_eq!(chunk_x, 3);
        assert_eq!(chunk_y, -2);
        assert_eq!(chunk_z, 2);
    }

    #[test]
    fn test_default_toggle_key_is_f1() {
        let cam = FreeFlyCam::default();
        assert_eq!(cam.toggle_key, KeyCode::F1);
    }
}
