# First Person Camera

## Problem

The engine needs an FPS-style first person camera controller that lets the player look around with the mouse and move with the keyboard. Mouse movement must rotate the view — horizontal mouse delta rotates yaw around the world up axis, vertical mouse delta rotates pitch around the camera's local right axis. WASD movement must be relative to the camera's current facing direction so that pressing W always moves "forward" from the player's perspective. Pitch must be clamped to prevent gimbal lock — without clamping, the player could pitch past vertical and invert their controls. The camera entity carries a `WorldPos` component for its 128-bit position in the universe, but the view matrix is computed in local f32 space after origin rebasing, consistent with the engine's precision bridge (02_math/07).

## Solution

### Component

```rust
use bevy_ecs::prelude::*;

/// Marker component that tags an entity as a first-person camera.
/// The entity must also have `WorldPos`, `LocalPos`, and `Rotation` components.
#[derive(Component, Clone, Debug)]
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
            move_speed: 100, // 100 mm/tick = 0.1 m/tick
            pitch_limit: 89.0_f32.to_radians(),
        }
    }
}
```

### Look system

The look system runs every frame, consuming mouse delta from `MouseState` (the mouse equivalent of `KeyboardState` from 15_input/01) and updating yaw and pitch:

```rust
use glam::Quat;
use nebula_math::WorldPosition;

pub fn first_person_look_system(
    mouse: Res<MouseState>,
    mut query: Query<(&mut FirstPersonCamera, &mut Rotation)>,
) {
    let (dx, dy) = mouse.delta();
    for (mut cam, mut rotation) in query.iter_mut() {
        // Yaw: mouse X rotates around world up axis
        cam.yaw -= dx * cam.mouse_sensitivity;

        // Pitch: mouse Y rotates around local right axis, clamped
        cam.pitch -= dy * cam.mouse_sensitivity;
        cam.pitch = cam.pitch.clamp(-cam.pitch_limit, cam.pitch_limit);

        // Reconstruct rotation quaternion from yaw and pitch.
        // Yaw first (around Y), then pitch (around X). This order prevents
        // the pitch axis from tilting with yaw, avoiding gimbal lock.
        let yaw_quat = Quat::from_rotation_y(cam.yaw);
        let pitch_quat = Quat::from_rotation_x(cam.pitch);
        rotation.0 = yaw_quat * pitch_quat;
    }
}
```

### Movement system

The movement system runs during FixedUpdate, reading keyboard state and applying movement relative to the camera's facing direction:

```rust
use nebula_math::Vec3I128;

pub fn first_person_move_system(
    keyboard: Res<KeyboardState>,
    mut query: Query<(&FirstPersonCamera, &Rotation, &mut WorldPos)>,
) {
    for (cam, rotation, mut world_pos) in query.iter_mut() {
        // Extract forward and right vectors from rotation, projected onto
        // the horizontal plane (Y=0) and normalized, so movement is always
        // horizontal regardless of pitch.
        let forward_f32 = rotation.0 * glam::Vec3::NEG_Z;
        let right_f32 = rotation.0 * glam::Vec3::X;

        let forward_horiz = glam::Vec3::new(forward_f32.x, 0.0, forward_f32.z).normalize_or_zero();
        let right_horiz = glam::Vec3::new(right_f32.x, 0.0, right_f32.z).normalize_or_zero();

        let mut direction = glam::Vec3::ZERO;
        if keyboard.is_pressed(KeyCode::KeyW) { direction += forward_horiz; }
        if keyboard.is_pressed(KeyCode::KeyS) { direction -= forward_horiz; }
        if keyboard.is_pressed(KeyCode::KeyD) { direction += right_horiz; }
        if keyboard.is_pressed(KeyCode::KeyA) { direction -= right_horiz; }

        if direction.length_squared() > 0.0 {
            direction = direction.normalize();
        }

        // Convert f32 direction to i128 displacement
        let displacement = Vec3I128::new(
            (direction.x * cam.move_speed as f32) as i128,
            (direction.y * cam.move_speed as f32) as i128,
            (direction.z * cam.move_speed as f32) as i128,
        );

        world_pos.0 = world_pos.0 + displacement;
    }
}
```

### Initial state

A newly spawned `FirstPersonCamera` has yaw = 0 and pitch = 0, which produces `Quat::IDENTITY`. Since the engine uses a right-handed coordinate system where -Z is forward, the camera starts looking along -Z, +Y is up, and +X is right.

### View matrix

The view matrix is not computed in this module. The existing `Camera` struct from 04_rendering/06 computes the view matrix from the entity's `Rotation` and `LocalPos` (which is the camera's position in f32 space after origin rebasing). This controller only updates `Rotation` and `WorldPos`; the rendering pipeline handles the rest.

## Outcome

A `first_person_camera.rs` module in `crates/nebula_player/src/` exporting the `FirstPersonCamera` component, `first_person_look_system`, and `first_person_move_system`. The component is spawned alongside `WorldPos`, `LocalPos`, `Rotation`, and the rendering `Camera` to produce a fully functional FPS camera. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

The player is on the planet's surface in first-person view. Mouse look rotates yaw and pitch. WASD moves forward/back/strafe. Terrain is visible at ground level for the first time.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.18` | `Component` derive, `Res`, `Query`, system function signatures |
| `glam` | `0.32` | `Quat`, `Vec3` for rotation and direction math |
| `winit` | `0.30` | `KeyCode` for physical key identification |
| `nebula-math` | workspace | `WorldPosition`, `Vec3I128` for 128-bit movement |
| `nebula-input` | workspace | `KeyboardState`, `MouseState` resources |

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Quat, Vec3};
    use std::f32::consts::FRAC_PI_2;

    /// Helper: compute the forward vector for a given yaw and pitch.
    fn forward_from(cam: &FirstPersonCamera) -> Vec3 {
        let yaw_quat = Quat::from_rotation_y(cam.yaw);
        let pitch_quat = Quat::from_rotation_x(cam.pitch);
        let rotation = yaw_quat * pitch_quat;
        rotation * Vec3::NEG_Z
    }

    #[test]
    fn test_mouse_x_rotates_yaw() {
        let mut cam = FirstPersonCamera::default();
        let initial_yaw = cam.yaw;
        // Simulate mouse delta: 100 pixels to the right
        let dx = 100.0;
        cam.yaw -= dx * cam.mouse_sensitivity;
        // Yaw should have changed
        assert!((cam.yaw - initial_yaw).abs() > 0.0);
        // Moving mouse right should decrease yaw (turn right in right-handed coords)
        assert!(cam.yaw < initial_yaw);
    }

    #[test]
    fn test_mouse_y_rotates_pitch() {
        let mut cam = FirstPersonCamera::default();
        let initial_pitch = cam.pitch;
        // Simulate mouse delta: 100 pixels downward
        let dy = 100.0;
        cam.pitch -= dy * cam.mouse_sensitivity;
        // Pitch should have changed
        assert!((cam.pitch - initial_pitch).abs() > 0.0);
        // Moving mouse down should decrease pitch (look down)
        assert!(cam.pitch < initial_pitch);
    }

    #[test]
    fn test_pitch_clamps_at_limits() {
        let mut cam = FirstPersonCamera::default();
        // Attempt to pitch far beyond the limit
        cam.pitch = 200.0_f32.to_radians();
        cam.pitch = cam.pitch.clamp(-cam.pitch_limit, cam.pitch_limit);
        assert!((cam.pitch - cam.pitch_limit).abs() < 1e-6);

        // Attempt to pitch far below the negative limit
        cam.pitch = -200.0_f32.to_radians();
        cam.pitch = cam.pitch.clamp(-cam.pitch_limit, cam.pitch_limit);
        assert!((cam.pitch + cam.pitch_limit).abs() < 1e-6);
    }

    #[test]
    fn test_wasd_movement_relative_to_facing() {
        // Camera looking along -Z (default): W should move in -Z direction
        let cam_default = FirstPersonCamera::default();
        let fwd_default = forward_from(&cam_default);
        assert!(fwd_default.z < -0.9); // Predominantly -Z

        // Camera rotated 90 degrees to the left: W should now move in +X direction
        let mut cam_rotated = FirstPersonCamera::default();
        cam_rotated.yaw = FRAC_PI_2;
        let fwd_rotated = forward_from(&cam_rotated);
        assert!(fwd_rotated.x > 0.9); // Predominantly +X
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
        // Yaw should not be artificially clamped — it can exceed 2*PI or go negative.
        // The quaternion reconstruction handles any angle correctly.
        let mut cam = FirstPersonCamera::default();
        cam.yaw = 10.0 * std::f32::consts::PI;
        let fwd = forward_from(&cam);
        // At 10*PI (= 5 full turns), forward should be back to -Z
        assert!((fwd.z + 1.0).abs() < 1e-4);
    }

    #[test]
    fn test_pitch_limit_prevents_gimbal_lock() {
        let cam = FirstPersonCamera::default();
        // Pitch limit must be strictly less than 90 degrees to prevent gimbal lock
        assert!(cam.pitch_limit < FRAC_PI_2);
        assert!(cam.pitch_limit > 0.0);
    }
}
```
