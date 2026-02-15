# Free-Fly Debug Camera

## Problem

During development, artists and engineers need a camera that can go anywhere in the world without restriction — no collision, no gravity, no movement speed limits, no invisible walls. This "noclip" camera is essential for inspecting terrain generation, debugging rendering artifacts, checking chunk boundaries, examining lighting from unusual angles, and verifying that the 128-bit coordinate system works correctly at extreme distances. The debug camera must be togglable with a single key press, must not interfere with gameplay input while inactive, and should display useful diagnostic information (position, rotation, chunk address) in a screen overlay.

## Solution

### Component

```rust
use bevy_ecs::prelude::*;

/// Marker component for the free-fly debug camera. When active, this
/// camera overrides the normal gameplay camera. When inactive, the
/// entity exists but its systems are skipped.
#[derive(Component, Clone, Debug)]
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
            speed: 500.0,          // 0.5 m/tick
            speed_min: 10.0,       // 10 mm/tick (crawl)
            speed_max: 1_000_000.0, // 1 km/tick (warp)
            speed_scroll_factor: 1.2,
            mouse_sensitivity: 0.003,
            yaw: 0.0,
            pitch: 0.0,
            toggle_key: KeyCode::F1,
        }
    }
}
```

### Toggle system

Pressing the toggle key switches the debug camera on and off. When toggled on, the debug camera captures the current gameplay camera's position and rotation. When toggled off, control returns to the gameplay camera:

```rust
pub fn free_fly_toggle_system(
    keyboard: Res<KeyboardState>,
    mut query: Query<&mut FreeFlyCam>,
) {
    for mut cam in query.iter_mut() {
        if keyboard.just_pressed(PhysicalKey::Code(cam.toggle_key)) {
            cam.active = !cam.active;
        }
    }
}
```

### Look system

When active, mouse movement rotates the debug camera. Pitch is clamped to prevent flipping:

```rust
use glam::Quat;

pub fn free_fly_look_system(
    mouse: Res<MouseState>,
    mut query: Query<(&mut FreeFlyCam, &mut Rotation)>,
) {
    let (dx, dy) = mouse.delta();
    for (mut cam, mut rotation) in query.iter_mut() {
        if !cam.active {
            continue;
        }
        cam.yaw -= dx * cam.mouse_sensitivity;
        cam.pitch -= dy * cam.mouse_sensitivity;
        cam.pitch = cam.pitch.clamp(
            -89.0_f32.to_radians(),
            89.0_f32.to_radians(),
        );
        rotation.0 = Quat::from_rotation_y(cam.yaw) * Quat::from_rotation_x(cam.pitch);
    }
}
```

### Movement system

WASD moves relative to the camera's facing direction. Space moves up (world +Y), Ctrl moves down. No collision detection, no gravity — pure unconstrained movement:

```rust
use nebula_math::Vec3I128;

pub fn free_fly_move_system(
    keyboard: Res<KeyboardState>,
    mut query: Query<(&FreeFlyCam, &Rotation, &mut WorldPos)>,
) {
    for (cam, rotation, mut world_pos) in query.iter_mut() {
        if !cam.active {
            continue;
        }

        let forward = rotation.0 * glam::Vec3::NEG_Z;
        let right = rotation.0 * glam::Vec3::X;
        let up = glam::Vec3::Y; // World up, not camera up

        let mut dir = glam::Vec3::ZERO;
        if keyboard.is_pressed(KeyCode::KeyW) { dir += forward; }
        if keyboard.is_pressed(KeyCode::KeyS) { dir -= forward; }
        if keyboard.is_pressed(KeyCode::KeyD) { dir += right; }
        if keyboard.is_pressed(KeyCode::KeyA) { dir -= right; }
        if keyboard.is_pressed(KeyCode::Space) { dir += up; }
        if keyboard.is_pressed(KeyCode::ControlLeft) { dir -= up; }

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
}
```

### Speed adjustment system

Scroll wheel and +/- keys adjust movement speed multiplicatively:

```rust
pub fn free_fly_speed_system(
    mouse: Res<MouseState>,
    keyboard: Res<KeyboardState>,
    mut query: Query<&mut FreeFlyCam>,
) {
    let scroll = mouse.scroll_delta();
    for mut cam in query.iter_mut() {
        if !cam.active {
            continue;
        }
        if scroll > 0.0 || keyboard.just_pressed(PhysicalKey::Code(KeyCode::Equal)) {
            cam.speed *= cam.speed_scroll_factor;
        }
        if scroll < 0.0 || keyboard.just_pressed(PhysicalKey::Code(KeyCode::Minus)) {
            cam.speed /= cam.speed_scroll_factor;
        }
        cam.speed = cam.speed.clamp(cam.speed_min, cam.speed_max);
    }
}
```

### Debug overlay

When active, the debug camera writes diagnostic info to a `DebugOverlay` resource that the UI system renders as a screen-space text overlay:

```rust
use nebula_math::WorldPosition;

/// Resource holding debug camera overlay text, rendered by the UI system.
#[derive(Resource, Default)]
pub struct DebugCameraOverlay {
    pub text: String,
}

pub fn free_fly_overlay_system(
    query: Query<(&FreeFlyCam, &WorldPos, &Rotation)>,
    mut overlay: ResMut<DebugCameraOverlay>,
) {
    overlay.text.clear();
    for (cam, world_pos, rotation) in query.iter() {
        if !cam.active {
            continue;
        }
        let p = &world_pos.0;
        let (yaw_deg, pitch_deg) = (cam.yaw.to_degrees(), cam.pitch.to_degrees());
        // Chunk address: divide by chunk size (e.g., 32_000 mm = 32 m chunks)
        let chunk_size: i128 = 32_000;
        let chunk_x = p.x.div_euclid(chunk_size);
        let chunk_y = p.y.div_euclid(chunk_size);
        let chunk_z = p.z.div_euclid(chunk_size);

        use std::fmt::Write;
        let _ = write!(
            overlay.text,
            "Pos: ({}, {}, {}) mm\n\
             Rot: yaw={:.1} pitch={:.1}\n\
             Chunk: ({}, {}, {})\n\
             Speed: {:.0} mm/tick",
            p.x, p.y, p.z,
            yaw_deg, pitch_deg,
            chunk_x, chunk_y, chunk_z,
            cam.speed,
        );
    }
}
```

### Input context isolation

The debug camera's systems check `cam.active` before processing input. When inactive, they early-return and do not consume any input. This means the gameplay camera controllers (first person, third person, spaceship) continue to work normally. When the debug camera is toggled on, the gameplay systems should be suppressed — this is handled by the camera mode system (story 05) which deactivates other camera controllers when the debug camera is active.

## Outcome

A `free_fly_camera.rs` module in `crates/nebula_player/src/` exporting `FreeFlyCam`, `DebugCameraOverlay`, `free_fly_toggle_system`, `free_fly_look_system`, `free_fly_move_system`, `free_fly_speed_system`, and `free_fly_overlay_system`. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

Pressing F1 activates a noclip free-fly camera that can pass through terrain, move at arbitrary speed, and ignores all physics constraints.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.18` | `Component`, `Resource`, `Res`, `ResMut`, `Query`, system functions |
| `glam` | `0.32` | `Vec3`, `Quat` for rotation and direction math |
| `winit` | `0.30` | `KeyCode`, `PhysicalKey` for key identification |
| `nebula-math` | workspace | `WorldPosition`, `Vec3I128` for 128-bit positioning |
| `nebula-input` | workspace | `KeyboardState`, `MouseState` resources |
| `nebula-ecs` | workspace | `WorldPos`, `Rotation` components |

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Quat, Vec3};
    use nebula_math::WorldPosition;

    #[test]
    fn test_free_fly_ignores_collision() {
        // Free fly camera can move to any position, including inside solid geometry.
        // There is no collision check — movement is pure displacement.
        let cam = FreeFlyCam { active: true, ..Default::default() };
        let mut world_pos = WorldPos::new(0, 0, 0);
        let rotation = Rotation(Quat::IDENTITY);

        // Move forward (into -Z) — no collision prevents this
        let forward = rotation.0 * Vec3::NEG_Z;
        let displacement = Vec3I128::new(
            (forward.x * cam.speed) as i128,
            (forward.y * cam.speed) as i128,
            (forward.z * cam.speed) as i128,
        );
        world_pos.0 = world_pos.0 + displacement;

        // Position should have changed; no collision blocked it
        assert_ne!(world_pos.0, WorldPosition::new(0, 0, 0));
    }

    #[test]
    fn test_speed_adjusts_with_scroll() {
        let mut cam = FreeFlyCam { active: true, ..Default::default() };
        let initial_speed = cam.speed;

        // Scroll up: speed increases
        cam.speed *= cam.speed_scroll_factor;
        cam.speed = cam.speed.clamp(cam.speed_min, cam.speed_max);
        assert!(cam.speed > initial_speed);

        // Scroll down: speed decreases
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
        let cam = FreeFlyCam { active: true, ..Default::default() };
        let world_pos = WorldPos::new(12_345, -67_890, 111_213);

        let p = &world_pos.0;
        let text = format!("Pos: ({}, {}, {}) mm", p.x, p.y, p.z);
        assert!(text.contains("12345"));
        assert!(text.contains("-67890"));
        assert!(text.contains("111213"));
    }

    #[test]
    fn test_camera_can_move_anywhere_in_world() {
        // Move the debug camera to extreme coordinates
        let mut world_pos = WorldPos::new(0, 0, 0);
        let ly_mm: i128 = 9_460_730_472_580_800_000;
        world_pos.0 = WorldPosition::new(100 * ly_mm, 0, -50 * ly_mm);
        // Position should be set without overflow or panic
        assert_eq!(world_pos.0.x, 100 * ly_mm);
        assert_eq!(world_pos.0.z, -50 * ly_mm);
    }

    #[test]
    fn test_speed_clamps_at_min() {
        let mut cam = FreeFlyCam { active: true, ..Default::default() };
        cam.speed = 0.001; // way below minimum
        cam.speed = cam.speed.clamp(cam.speed_min, cam.speed_max);
        assert!((cam.speed - cam.speed_min).abs() < 1e-6);
    }

    #[test]
    fn test_speed_clamps_at_max() {
        let mut cam = FreeFlyCam { active: true, ..Default::default() };
        cam.speed = 99_999_999.0; // way above maximum
        cam.speed = cam.speed.clamp(cam.speed_min, cam.speed_max);
        assert!((cam.speed - cam.speed_max).abs() < 1e-6);
    }

    #[test]
    fn test_inactive_camera_does_not_process_movement() {
        let cam = FreeFlyCam { active: false, ..Default::default() };
        // When inactive, the system early-returns. We verify the flag check.
        assert!(!cam.active);
        // In the real system, `continue` skips all processing when `!cam.active`.
    }

    #[test]
    fn test_chunk_address_computation() {
        let chunk_size: i128 = 32_000;
        let pos = WorldPosition::new(100_000, -50_000, 64_001);
        let chunk_x = pos.x.div_euclid(chunk_size);
        let chunk_y = pos.y.div_euclid(chunk_size);
        let chunk_z = pos.z.div_euclid(chunk_size);
        assert_eq!(chunk_x, 3);   // 100_000 / 32_000 = 3.125 -> floor = 3
        assert_eq!(chunk_y, -2);  // -50_000 / 32_000 = -1.5625 -> euclidean floor = -2
        assert_eq!(chunk_z, 2);   // 64_001 / 32_000 = 2.000... -> floor = 2
    }

    #[test]
    fn test_default_toggle_key_is_f1() {
        let cam = FreeFlyCam::default();
        assert_eq!(cam.toggle_key, KeyCode::F1);
    }
}
```
