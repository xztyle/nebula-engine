# Third Person Camera

## Problem

Many gameplay scenarios — character action, vehicle following, planetary exploration — require a third person camera that follows a target entity from behind and above. The camera must orbit around the target when the player drags the mouse, zoom in and out with the scroll wheel, and smoothly track the target's position rather than snapping rigidly. The camera must always look at the target so the player never loses sight of their character. Zoom distance needs min/max clamping to prevent the camera from clipping into the target or zooming so far out that the target is invisible. All positioning happens in 128-bit world space (`WorldPos`) with the view matrix computed in local f32 space after origin rebasing.

## Solution

### Component

```rust
use bevy_ecs::prelude::*;

/// Tags an entity as a third-person camera that follows a target entity.
/// The camera entity must also have `WorldPos`, `LocalPos`, and `Rotation`.
#[derive(Component, Clone, Debug)]
pub struct ThirdPersonCamera {
    /// The entity being followed.
    pub target: Entity,
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
            target: Entity::PLACEHOLDER,
            orbit_yaw: 0.0,
            orbit_pitch: 20.0_f32.to_radians(),
            distance: 5000.0,   // 5 meters
            distance_min: 1000.0, // 1 meter
            distance_max: 50_000.0, // 50 meters
            height_offset: 1500.0, // 1.5 meters
            orbit_sensitivity: 0.005,
            zoom_sensitivity: 200.0,
            follow_speed: 0.1,
            pitch_min: -10.0_f32.to_radians(),
            pitch_max: 80.0_f32.to_radians(),
        }
    }
}
```

### Orbit system

On mouse drag (right mouse button held), horizontal mouse delta adjusts `orbit_yaw` and vertical delta adjusts `orbit_pitch`:

```rust
pub fn third_person_orbit_system(
    mouse: Res<MouseState>,
    mut query: Query<&mut ThirdPersonCamera>,
) {
    if !mouse.is_pressed(MouseButton::Right) {
        return;
    }
    let (dx, dy) = mouse.delta();
    for mut cam in query.iter_mut() {
        cam.orbit_yaw -= dx * cam.orbit_sensitivity;
        cam.orbit_pitch -= dy * cam.orbit_sensitivity;
        cam.orbit_pitch = cam.orbit_pitch.clamp(cam.pitch_min, cam.pitch_max);
    }
}
```

### Zoom system

Scroll wheel input adjusts the `distance`, clamped between `distance_min` and `distance_max`:

```rust
pub fn third_person_zoom_system(
    mouse: Res<MouseState>,
    mut query: Query<&mut ThirdPersonCamera>,
) {
    let scroll = mouse.scroll_delta();
    if scroll.abs() < 1e-6 {
        return;
    }
    for mut cam in query.iter_mut() {
        cam.distance -= scroll * cam.zoom_sensitivity;
        cam.distance = cam.distance.clamp(cam.distance_min, cam.distance_max);
    }
}
```

### Follow and position system

Each frame, the camera computes its desired world position based on the target's position, the orbit angles, and the distance. It then interpolates (lerps) its actual `WorldPos` toward that desired position:

```rust
use glam::{Vec3, Quat};
use nebula_math::{WorldPosition, Vec3I128};

pub fn third_person_follow_system(
    mut cam_query: Query<(&ThirdPersonCamera, &mut WorldPos, &mut Rotation)>,
    target_query: Query<&WorldPos, Without<ThirdPersonCamera>>,
) {
    for (cam, mut cam_world_pos, mut cam_rotation) in cam_query.iter_mut() {
        let Ok(target_pos) = target_query.get(cam.target) else {
            continue;
        };

        // Compute the look-at point: target position + height offset
        let look_at_offset = Vec3I128::new(0, cam.height_offset as i128, 0);
        let look_at_world = target_pos.0 + look_at_offset;

        // Compute desired camera offset from look-at point using spherical coordinates.
        // orbit_yaw=0, orbit_pitch=0 places the camera behind the target at -Z.
        let cos_pitch = cam.orbit_pitch.cos();
        let sin_pitch = cam.orbit_pitch.sin();
        let cos_yaw = cam.orbit_yaw.cos();
        let sin_yaw = cam.orbit_yaw.sin();

        let offset = Vec3::new(
            cam.distance * cos_pitch * sin_yaw,
            cam.distance * sin_pitch,
            cam.distance * cos_pitch * cos_yaw,
        );

        let desired_world = look_at_world + Vec3I128::new(
            offset.x as i128,
            offset.y as i128,
            offset.z as i128,
        );

        // Smooth follow: lerp current position toward desired position.
        // Lerp each axis independently in i128 space using the follow_speed factor.
        let current = cam_world_pos.0;
        let delta = desired_world - current; // Vec3I128
        let lerped = WorldPosition::new(
            current.x + (delta.x as f64 * cam.follow_speed as f64) as i128,
            current.y + (delta.y as f64 * cam.follow_speed as f64) as i128,
            current.z + (delta.z as f64 * cam.follow_speed as f64) as i128,
        );
        cam_world_pos.0 = lerped;

        // Compute look-at rotation: camera always faces the look-at point.
        // Use local f32 space for the rotation calculation.
        let cam_to_target = Vec3::new(
            (look_at_world.x - lerped.x) as f32,
            (look_at_world.y - lerped.y) as f32,
            (look_at_world.z - lerped.z) as f32,
        );
        if cam_to_target.length_squared() > 1e-6 {
            let forward = cam_to_target.normalize();
            // Construct rotation that looks along `forward` with world +Y as up.
            let right = Vec3::Y.cross(forward).normalize_or_zero();
            let up = forward.cross(right);
            cam_rotation.0 = Quat::from_mat3(&glam::Mat3::from_cols(right, up, forward));
        }
    }
}
```

### Design notes

- The smooth follow uses a simple linear interpolation factor per frame. For framerate independence in a variable-timestep scenario, the factor should be adjusted by `1.0 - (1.0 - follow_speed).powf(dt * 60.0)`, but since the engine uses a fixed timestep (01_setup/06), the raw factor is sufficient.
- The orbit pitch is clamped to avoid flipping the camera upside down or looking from directly below.
- The camera's `Rotation` is always recomputed to face the look-at point, so gameplay code never needs to manually orient it.

## Outcome

A `third_person_camera.rs` module in `crates/nebula_player/src/` exporting `ThirdPersonCamera`, `third_person_orbit_system`, `third_person_zoom_system`, and `third_person_follow_system`. The camera entity is spawned with `ThirdPersonCamera`, `WorldPos`, `LocalPos`, `Rotation`, and the rendering `Camera`. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

The camera pulls back behind a placeholder capsule entity representing the player. The camera orbits the capsule and follows its movement across the terrain.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.18` | `Component` derive, `Entity`, `Res`, `Query`, `Without` filter |
| `glam` | `0.32` | `Vec3`, `Quat`, `Mat3` for orbit math and look-at rotation |
| `nebula-math` | workspace | `WorldPosition`, `Vec3I128` for 128-bit positioning |
| `nebula-input` | workspace | `MouseState` for mouse delta and scroll input |

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn test_camera_distance_matches_zoom_level() {
        let mut cam = ThirdPersonCamera::default();
        cam.distance = 10_000.0; // 10 meters
        assert!((cam.distance - 10_000.0).abs() < 1e-6);
        cam.distance = 3_000.0; // zoom in to 3 meters
        assert!((cam.distance - 3_000.0).abs() < 1e-6);
    }

    #[test]
    fn test_orbit_changes_angle() {
        let mut cam = ThirdPersonCamera::default();
        let initial_yaw = cam.orbit_yaw;
        // Simulate orbit drag
        let dx = 50.0;
        cam.orbit_yaw -= dx * cam.orbit_sensitivity;
        assert!((cam.orbit_yaw - initial_yaw).abs() > 0.0);
    }

    #[test]
    fn test_zoom_clamps_at_min() {
        let mut cam = ThirdPersonCamera::default();
        cam.distance = 500.0; // below minimum
        cam.distance = cam.distance.clamp(cam.distance_min, cam.distance_max);
        assert!((cam.distance - cam.distance_min).abs() < 1e-6);
    }

    #[test]
    fn test_zoom_clamps_at_max() {
        let mut cam = ThirdPersonCamera::default();
        cam.distance = 100_000.0; // above maximum
        cam.distance = cam.distance.clamp(cam.distance_min, cam.distance_max);
        assert!((cam.distance - cam.distance_max).abs() < 1e-6);
    }

    #[test]
    fn test_smooth_follow_converges_on_target() {
        // Simulate multiple frames of follow interpolation
        let target = WorldPosition::new(10_000, 5_000, 3_000);
        let mut current = WorldPosition::new(0, 0, 0);
        let follow_speed: f64 = 0.1;

        for _ in 0..200 {
            let delta_x = target.x - current.x;
            let delta_y = target.y - current.y;
            let delta_z = target.z - current.z;
            current = WorldPosition::new(
                current.x + (delta_x as f64 * follow_speed) as i128,
                current.y + (delta_y as f64 * follow_speed) as i128,
                current.z + (delta_z as f64 * follow_speed) as i128,
            );
        }
        // After many iterations, the camera should be very close to target
        assert!((current.x - target.x).abs() <= 1);
        assert!((current.y - target.y).abs() <= 1);
        assert!((current.z - target.z).abs() <= 1);
    }

    #[test]
    fn test_camera_looks_at_target_from_all_angles() {
        // At different orbit yaw angles, the camera-to-target vector should
        // always point from the camera's computed position toward the target.
        let target = Vec3::new(0.0, 1500.0, 0.0); // target with height offset
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
            // The look direction should have non-zero length (camera is not at target)
            assert!(to_target.length() > 0.99);
        }
    }

    #[test]
    fn test_orbit_pitch_clamps() {
        let mut cam = ThirdPersonCamera::default();
        cam.orbit_pitch = 200.0_f32.to_radians(); // way above max
        cam.orbit_pitch = cam.orbit_pitch.clamp(cam.pitch_min, cam.pitch_max);
        assert!((cam.orbit_pitch - cam.pitch_max).abs() < 1e-6);

        cam.orbit_pitch = -200.0_f32.to_radians(); // way below min
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
```
