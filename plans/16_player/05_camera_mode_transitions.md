# Camera Mode Transitions

## Problem

The engine supports multiple camera modes — first person (story 01), third person (story 02), spaceship (story 03), free-fly debug (story 06), and gravity-oriented (story 07). When switching between modes, the camera must not teleport or snap abruptly. Instead, it should smoothly interpolate position, rotation, and field of view from the old state to the new state over a configurable duration. Without smooth transitions, mode switches are visually jarring and disorienting, especially when the new mode's camera position is significantly different from the old one (e.g., switching from first person inside a cockpit to third person behind the ship).

## Solution

### Camera snapshot

A lightweight capture of the camera's state at a point in time, used as the start and end points of a transition:

```rust
use glam::{Vec3, Quat};

/// A snapshot of camera state for interpolation.
#[derive(Clone, Copy, Debug)]
pub struct CameraSnapshot {
    /// Position in local f32 space.
    pub position: Vec3,
    /// Orientation as a unit quaternion.
    pub rotation: Quat,
    /// Vertical field of view in radians.
    pub fov_y: f32,
}

impl CameraSnapshot {
    pub fn from_camera(position: Vec3, rotation: Quat, fov_y: f32) -> Self {
        Self { position, rotation, fov_y }
    }
}
```

### Transition component

```rust
use bevy_ecs::prelude::*;

/// When attached to a camera entity, drives a smooth transition from
/// one camera state to another. The transition system removes this
/// component when the transition completes.
#[derive(Component, Clone, Debug)]
pub struct CameraTransition {
    /// The camera state at the start of the transition.
    pub from: CameraSnapshot,
    /// The camera state at the end of the transition.
    pub to: CameraSnapshot,
    /// Total duration of the transition in ticks.
    pub duration_ticks: u32,
    /// Current tick within the transition (0..=duration_ticks).
    pub elapsed_ticks: u32,
    /// Easing function to use for interpolation.
    pub easing: EasingFunction,
}

/// Easing curves for camera transitions.
#[derive(Clone, Copy, Debug, Default)]
pub enum EasingFunction {
    /// Constant speed, no acceleration.
    Linear,
    /// Slow start, fast end.
    EaseIn,
    /// Fast start, slow end.
    #[default]
    EaseOut,
    /// Slow start, fast middle, slow end. Default for camera transitions.
    EaseInOut,
}
```

### Easing math

```rust
impl EasingFunction {
    /// Map a linear progress value (0.0..=1.0) to an eased value.
    pub fn apply(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            EasingFunction::Linear => t,
            EasingFunction::EaseIn => t * t,
            EasingFunction::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
            EasingFunction::EaseInOut => {
                if t < 0.5 {
                    2.0 * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
                }
            }
        }
    }
}
```

### Transition system

```rust
use glam::{Vec3, Quat};

pub fn camera_transition_system(
    mut commands: Commands,
    mut query: Query<(
        Entity,
        &mut CameraTransition,
        &mut LocalPos,
        &mut Rotation,
        &mut Camera,
    )>,
) {
    for (entity, mut transition, mut local_pos, mut rotation, mut camera) in query.iter_mut() {
        transition.elapsed_ticks += 1;

        if transition.elapsed_ticks >= transition.duration_ticks {
            // Transition complete: snap to final state and remove the component.
            local_pos.0 = LocalPosition::new(
                transition.to.position.x,
                transition.to.position.y,
                transition.to.position.z,
            );
            rotation.0 = transition.to.rotation;
            if let Projection::Perspective { fov_y, .. } = &mut camera.projection {
                *fov_y = transition.to.fov_y;
            }
            commands.entity(entity).remove::<CameraTransition>();
            continue;
        }

        // Compute eased progress
        let linear_t = transition.elapsed_ticks as f32 / transition.duration_ticks as f32;
        let t = transition.easing.apply(linear_t);

        // Interpolate position (lerp)
        let pos = transition.from.position.lerp(transition.to.position, t);
        local_pos.0 = LocalPosition::new(pos.x, pos.y, pos.z);

        // Interpolate rotation (slerp for shortest path)
        rotation.0 = transition.from.rotation.slerp(transition.to.rotation, t);

        // Interpolate FOV (linear lerp)
        let fov = transition.from.fov_y + (transition.to.fov_y - transition.from.fov_y) * t;
        if let Projection::Perspective { fov_y, .. } = &mut camera.projection {
            *fov_y = fov;
        }
    }
}
```

### Transition initiation

When a mode switch is requested, the calling code captures the current camera state, computes the target state from the new mode, and inserts a `CameraTransition` component:

```rust
impl CameraTransition {
    /// Create a transition from the current camera state to a new state.
    /// Duration of 0 means instant snap (transition completes on the next frame).
    pub fn new(
        from: CameraSnapshot,
        to: CameraSnapshot,
        duration_ticks: u32,
        easing: EasingFunction,
    ) -> Self {
        Self {
            from,
            to,
            duration_ticks: duration_ticks.max(1), // at least 1 tick
            elapsed_ticks: 0,
            easing,
        }
    }

    /// Instant transition — snaps to the target state on the next tick.
    pub fn instant(from: CameraSnapshot, to: CameraSnapshot) -> Self {
        Self::new(from, to, 1, EasingFunction::Linear)
    }
}
```

### Design notes

- During a transition, the new camera mode's controller systems should be inactive (or the transition system should take priority). The simplest approach is to check for the presence of `CameraTransition` as a filter: camera controllers skip entities that have this component.
- Rotation interpolation uses `Quat::slerp` to follow the shortest arc between orientations, preventing the camera from spinning the long way around.
- The transition operates in local f32 space. Since both the old and new camera states are expressed relative to the floating origin, this is consistent. If the origin shifts during a transition (because the camera moved significantly), the positions are still relative and the interpolation remains smooth.

## Outcome

A `camera_transition.rs` module in `crates/nebula_player/src/` exporting `CameraSnapshot`, `CameraTransition`, `EasingFunction`, and `camera_transition_system`. Mode-switch code creates a `CameraTransition` to drive the camera smoothly from the old state to the new state. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

Switching between first-person, third-person, and spaceship modes uses a smooth interpolated transition. The camera slides between positions over 0.5 seconds.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.18` | `Component` derive, `Commands`, `Entity`, `Query`, system functions |
| `glam` | `0.32` | `Vec3`, `Quat` for lerp/slerp interpolation |
| `nebula-math` | workspace | `LocalPosition` for local coordinate assignment |
| `nebula-ecs` | workspace | `LocalPos`, `Rotation` components |
| `nebula-render` | workspace | `Camera`, `Projection` for FOV interpolation |

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Quat, Vec3};
    use std::f32::consts::{FRAC_PI_4, FRAC_PI_2};

    fn snapshot_a() -> CameraSnapshot {
        CameraSnapshot {
            position: Vec3::new(0.0, 0.0, 0.0),
            rotation: Quat::IDENTITY,
            fov_y: FRAC_PI_4, // 45 degrees
        }
    }

    fn snapshot_b() -> CameraSnapshot {
        CameraSnapshot {
            position: Vec3::new(1000.0, 2000.0, 3000.0),
            rotation: Quat::from_rotation_y(FRAC_PI_2),
            fov_y: FRAC_PI_2, // 90 degrees
        }
    }

    #[test]
    fn test_transition_starts_at_old_camera_state() {
        let transition = CameraTransition::new(
            snapshot_a(),
            snapshot_b(),
            60,
            EasingFunction::Linear,
        );
        // At elapsed=0 (before any tick), t=0, position should be snapshot_a
        let t = 0.0_f32;
        let pos = transition.from.position.lerp(transition.to.position, t);
        assert!((pos - snapshot_a().position).length() < 1e-6);
    }

    #[test]
    fn test_transition_ends_at_new_camera_state() {
        let transition = CameraTransition::new(
            snapshot_a(),
            snapshot_b(),
            60,
            EasingFunction::Linear,
        );
        // At t=1.0, position should be snapshot_b
        let t = 1.0_f32;
        let pos = transition.from.position.lerp(transition.to.position, t);
        assert!((pos - snapshot_b().position).length() < 1e-4);
    }

    #[test]
    fn test_mid_transition_is_interpolated() {
        let from = snapshot_a();
        let to = snapshot_b();
        let t = 0.5_f32;

        let pos = from.position.lerp(to.position, t);
        let expected_pos = Vec3::new(500.0, 1000.0, 1500.0);
        assert!((pos - expected_pos).length() < 1e-4);

        let fov = from.fov_y + (to.fov_y - from.fov_y) * t;
        let expected_fov = (FRAC_PI_4 + FRAC_PI_2) / 2.0;
        assert!((fov - expected_fov).abs() < 1e-6);
    }

    #[test]
    fn test_transition_duration_is_configurable() {
        let short = CameraTransition::new(
            snapshot_a(), snapshot_b(), 10, EasingFunction::Linear,
        );
        let long = CameraTransition::new(
            snapshot_a(), snapshot_b(), 300, EasingFunction::Linear,
        );
        assert_eq!(short.duration_ticks, 10);
        assert_eq!(long.duration_ticks, 300);
    }

    #[test]
    fn test_instant_transition_snaps_immediately() {
        let transition = CameraTransition::instant(snapshot_a(), snapshot_b());
        assert_eq!(transition.duration_ticks, 1);
        // After 1 tick, elapsed >= duration, so it snaps to `to` state
        let mut t_copy = transition.clone();
        t_copy.elapsed_ticks = 1;
        assert!(t_copy.elapsed_ticks >= t_copy.duration_ticks);
    }

    #[test]
    fn test_easing_linear_midpoint() {
        let t = EasingFunction::Linear.apply(0.5);
        assert!((t - 0.5).abs() < 1e-6);
    }

    #[test]
    fn test_easing_ease_in_starts_slow() {
        // EaseIn: t^2. At t=0.25, eased = 0.0625 (slower than linear 0.25)
        let t = EasingFunction::EaseIn.apply(0.25);
        assert!(t < 0.25);
        assert!((t - 0.0625).abs() < 1e-6);
    }

    #[test]
    fn test_easing_ease_out_ends_slow() {
        // EaseOut: 1 - (1-t)^2. At t=0.75, eased = 0.9375 (faster than linear 0.75)
        let t = EasingFunction::EaseOut.apply(0.75);
        assert!(t > 0.75);
        assert!((t - 0.9375).abs() < 1e-6);
    }

    #[test]
    fn test_easing_all_start_at_zero_end_at_one() {
        let easings = [
            EasingFunction::Linear,
            EasingFunction::EaseIn,
            EasingFunction::EaseOut,
            EasingFunction::EaseInOut,
        ];
        for easing in &easings {
            assert!((easing.apply(0.0) - 0.0).abs() < 1e-6, "{easing:?} at t=0");
            assert!((easing.apply(1.0) - 1.0).abs() < 1e-6, "{easing:?} at t=1");
        }
    }

    #[test]
    fn test_slerp_rotation_follows_shortest_path() {
        let from = Quat::IDENTITY;
        let to = Quat::from_rotation_y(FRAC_PI_2); // 90 degrees
        let mid = from.slerp(to, 0.5);
        // Mid-rotation should be ~45 degrees around Y
        let expected = Quat::from_rotation_y(FRAC_PI_4);
        assert!((mid - expected).length() < 1e-4);
    }
}
```
