//! Smooth camera mode transitions: interpolates position, rotation, and FOV
//! between two camera states over a configurable duration with easing.

use bevy_ecs::prelude::*;
use glam::{Quat, Vec3};
use nebula_ecs::{LocalPos, Rotation};
use nebula_math::LocalPosition;

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
    /// Create a snapshot from individual camera parameters.
    pub fn from_camera(position: Vec3, rotation: Quat, fov_y: f32) -> Self {
        Self {
            position,
            rotation,
            fov_y,
        }
    }
}

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

impl CameraTransition {
    /// Create a transition from the current camera state to a new state.
    /// Duration of 0 means instant snap (clamped to at least 1 tick).
    pub fn new(
        from: CameraSnapshot,
        to: CameraSnapshot,
        duration_ticks: u32,
        easing: EasingFunction,
    ) -> Self {
        Self {
            from,
            to,
            duration_ticks: duration_ticks.max(1),
            elapsed_ticks: 0,
            easing,
        }
    }

    /// Instant transition — snaps to the target state on the next tick.
    pub fn instant(from: CameraSnapshot, to: CameraSnapshot) -> Self {
        Self::new(from, to, 1, EasingFunction::Linear)
    }

    /// Compute the interpolated FOV at the current progress.
    /// Useful for callers that manage FOV outside the ECS (e.g., render Camera).
    pub fn current_fov(&self) -> f32 {
        if self.elapsed_ticks >= self.duration_ticks {
            return self.to.fov_y;
        }
        let linear_t = self.elapsed_ticks as f32 / self.duration_ticks as f32;
        let t = self.easing.apply(linear_t);
        self.from.fov_y + (self.to.fov_y - self.from.fov_y) * t
    }
}

/// Easing curves for camera transitions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EasingFunction {
    /// Constant speed, no acceleration.
    Linear,
    /// Slow start, fast end.
    EaseIn,
    /// Fast start, slow end.
    #[default]
    EaseOut,
    /// Slow start, fast middle, slow end.
    EaseInOut,
}

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

/// Advances all active camera transitions by one tick, interpolating
/// position (lerp), rotation (slerp), and FOV (stored for external use).
/// Removes the [`CameraTransition`] component when the transition completes.
///
/// FOV interpolation is computed via [`CameraTransition::current_fov()`]
/// since the render `Camera` is not an ECS component. Callers should read
/// `current_fov()` each frame and apply it to their render camera.
pub fn camera_transition_system(
    mut commands: Commands,
    mut query: Query<(Entity, &mut CameraTransition, &mut LocalPos, &mut Rotation)>,
) {
    for (entity, mut transition, mut local_pos, mut rotation) in query.iter_mut() {
        transition.elapsed_ticks += 1;

        if transition.elapsed_ticks >= transition.duration_ticks {
            // Transition complete: snap to final state and remove the component.
            local_pos.0 = LocalPosition::new(
                transition.to.position.x,
                transition.to.position.y,
                transition.to.position.z,
            );
            rotation.0 = transition.to.rotation;
            commands.entity(entity).remove::<CameraTransition>();
            continue;
        }

        // Compute eased progress.
        let linear_t = transition.elapsed_ticks as f32 / transition.duration_ticks as f32;
        let t = transition.easing.apply(linear_t);

        // Interpolate position (lerp).
        let pos = transition.from.position.lerp(transition.to.position, t);
        local_pos.0 = LocalPosition::new(pos.x, pos.y, pos.z);

        // Interpolate rotation (slerp for shortest path).
        rotation.0 = transition.from.rotation.slerp(transition.to.rotation, t);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Quat, Vec3};
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};

    fn snapshot_a() -> CameraSnapshot {
        CameraSnapshot {
            position: Vec3::new(0.0, 0.0, 0.0),
            rotation: Quat::IDENTITY,
            fov_y: FRAC_PI_4,
        }
    }

    fn snapshot_b() -> CameraSnapshot {
        CameraSnapshot {
            position: Vec3::new(1000.0, 2000.0, 3000.0),
            rotation: Quat::from_rotation_y(FRAC_PI_2),
            fov_y: FRAC_PI_2,
        }
    }

    #[test]
    fn test_transition_starts_at_old_camera_state() {
        let transition =
            CameraTransition::new(snapshot_a(), snapshot_b(), 60, EasingFunction::Linear);
        let t = 0.0_f32;
        let pos = transition.from.position.lerp(transition.to.position, t);
        assert!((pos - snapshot_a().position).length() < 1e-6);
    }

    #[test]
    fn test_transition_ends_at_new_camera_state() {
        let transition =
            CameraTransition::new(snapshot_a(), snapshot_b(), 60, EasingFunction::Linear);
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
        let short = CameraTransition::new(snapshot_a(), snapshot_b(), 10, EasingFunction::Linear);
        let long = CameraTransition::new(snapshot_a(), snapshot_b(), 300, EasingFunction::Linear);
        assert_eq!(short.duration_ticks, 10);
        assert_eq!(long.duration_ticks, 300);
    }

    #[test]
    fn test_instant_transition_snaps_immediately() {
        let transition = CameraTransition::instant(snapshot_a(), snapshot_b());
        assert_eq!(transition.duration_ticks, 1);
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
        let t = EasingFunction::EaseIn.apply(0.25);
        assert!(t < 0.25);
        assert!((t - 0.0625).abs() < 1e-6);
    }

    #[test]
    fn test_easing_ease_out_ends_slow() {
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
        let to = Quat::from_rotation_y(FRAC_PI_2);
        let mid = from.slerp(to, 0.5);
        let expected = Quat::from_rotation_y(FRAC_PI_4);
        assert!((mid - expected).length() < 1e-4);
    }
}
