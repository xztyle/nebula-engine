# Procedural Animation

## Problem

Clip-based animation alone cannot respond to dynamic gameplay context. A character's head should track a point of interest, their torso should lean into turns, and their chest should subtly rise and fall with breathing — none of which are baked into a walk cycle. Without procedural layers, characters look robotic: they stare straight ahead regardless of threats, stand perfectly upright while sprinting around corners, and breathe only when the artist remembered to keyframe it. These deficiencies break immersion.

Procedural animation operates on top of the clip-sampled pose, modifying specific joints after the `AnimationPlayer` produces its output but before the final joint matrices are computed. Each procedural effect is a layer with a blendable weight, so designers can fade effects in and out smoothly. Multiple layers must stack — a character can look at a target, lean into a turn, and breathe, all simultaneously — without the layers clobbering each other.

## Solution

### Procedural Layer Architecture

Define a trait and concrete layer types in `nebula_animation`:

```rust
use glam::{Vec3, Quat};

/// The pose of a single joint: translation, rotation, scale.
pub type JointPose = (Vec3, Quat, Vec3);

/// A procedural animation layer that modifies a pose after clip sampling.
pub trait ProceduralLayer: Send + Sync {
    /// Compute the additive modification to the target joint's pose.
    /// Returns the delta (offset translation, delta rotation, scale multiplier).
    fn evaluate(&self, context: &ProceduralContext) -> JointPose;

    /// The index of the joint this layer modifies.
    fn target_joint(&self) -> u16;

    /// Blend weight in [0.0, 1.0]. 0.0 = no effect, 1.0 = full effect.
    fn weight(&self) -> f32;
}

/// Context provided to procedural layers each frame.
pub struct ProceduralContext {
    /// Delta time for this frame.
    pub dt: f32,
    /// Total elapsed time (for periodic effects like breathing).
    pub elapsed: f32,
    /// Entity's world-space position.
    pub entity_position: Vec3,
    /// Entity's forward direction.
    pub entity_forward: Vec3,
    /// Entity's current velocity (for lean calculations).
    pub velocity: Vec3,
}
```

### Look-At Layer

Rotates a target joint (typically the head or neck) so that it faces a world-space point:

```rust
pub struct LookAtLayer {
    /// Joint index to rotate (e.g., head joint).
    pub joint_index: u16,
    /// World-space target position to look at.
    pub target: Vec3,
    /// Maximum rotation angle in radians (clamp to avoid neck-breaking rotations).
    pub max_angle: f32,
    /// Blend weight.
    pub weight: f32,
}

impl ProceduralLayer for LookAtLayer {
    fn evaluate(&self, context: &ProceduralContext) -> JointPose {
        let to_target = (self.target - context.entity_position).normalize_or_zero();
        let forward = context.entity_forward;

        // Compute the rotation from forward to the target direction.
        let desired_rotation = Quat::from_rotation_arc(forward, to_target);

        // Clamp the rotation angle.
        let (axis, angle) = desired_rotation.to_axis_angle();
        let clamped_angle = angle.clamp(-self.max_angle, self.max_angle);
        let clamped_rotation = Quat::from_axis_angle(axis, clamped_angle);

        (Vec3::ZERO, clamped_rotation, Vec3::ONE)
    }

    fn target_joint(&self) -> u16 { self.joint_index }
    fn weight(&self) -> f32 { self.weight }
}
```

### Lean Layer

Tilts the torso based on lateral velocity (simulating centripetal lean during turns):

```rust
pub struct LeanLayer {
    /// Joint index to tilt (e.g., spine or torso joint).
    pub joint_index: u16,
    /// Maximum lean angle in radians.
    pub max_lean_angle: f32,
    /// How quickly the lean responds (smoothing factor).
    pub responsiveness: f32,
    /// Current smoothed lean value (internal state).
    current_lean: f32,
    /// Blend weight.
    pub weight: f32,
}

impl ProceduralLayer for LeanLayer {
    fn evaluate(&self, context: &ProceduralContext) -> JointPose {
        // Project velocity onto the entity's right axis to get lateral speed.
        let right = context.entity_forward.cross(Vec3::Y).normalize_or_zero();
        let lateral_speed = context.velocity.dot(right);

        // Smooth the lean toward the target value.
        let target_lean = (lateral_speed * 0.1).clamp(-self.max_lean_angle, self.max_lean_angle);
        // Note: actual smoothing updates self.current_lean in a mutable version.
        let lean_rotation = Quat::from_rotation_z(-target_lean);

        (Vec3::ZERO, lean_rotation, Vec3::ONE)
    }

    fn target_joint(&self) -> u16 { self.joint_index }
    fn weight(&self) -> f32 { self.weight }
}
```

### Breathing Layer

Applies a subtle periodic oscillation to the chest joint to simulate breathing:

```rust
pub struct BreathingLayer {
    /// Joint index to oscillate (e.g., upper chest or spine2).
    pub joint_index: u16,
    /// Breathing frequency in Hz (typical: 0.25 = one breath per 4 seconds).
    pub frequency: f32,
    /// Maximum scale amplitude (e.g., 0.02 for a 2% chest expansion).
    pub amplitude: f32,
    /// Blend weight.
    pub weight: f32,
}

impl ProceduralLayer for BreathingLayer {
    fn evaluate(&self, context: &ProceduralContext) -> JointPose {
        // Sinusoidal oscillation using elapsed time.
        let phase = (context.elapsed * self.frequency * std::f32::consts::TAU).sin();
        let breath_scale = 1.0 + phase * self.amplitude;

        // Slight upward translation to accompany the chest expansion.
        let breath_translation = Vec3::new(0.0, phase * self.amplitude * 0.5, 0.0);

        // Subtle forward rotation (chest lifts during inhale).
        let breath_rotation = Quat::from_rotation_x(phase * self.amplitude * 0.3);

        (breath_translation, breath_rotation, Vec3::new(breath_scale, breath_scale, 1.0))
    }

    fn target_joint(&self) -> u16 { self.joint_index }
    fn weight(&self) -> f32 { self.weight }
}
```

### Layer Stack and Additive Blending

The `ProceduralAnimationStack` component holds ordered layers and applies them after clip sampling:

```rust
pub struct ProceduralAnimationStack {
    pub layers: Vec<Box<dyn ProceduralLayer>>,
}

impl ProceduralAnimationStack {
    /// Apply all procedural layers additively on top of the base clip pose.
    pub fn apply(&self, context: &ProceduralContext, local_poses: &mut [JointPose]) {
        for layer in &self.layers {
            let joint_idx = layer.target_joint() as usize;
            let weight = layer.weight();

            if weight < 1e-6 {
                continue; // Skip zero-weight layers entirely.
            }

            let (delta_t, delta_r, delta_s) = layer.evaluate(context);

            let (ref mut base_t, ref mut base_r, ref mut base_s) = local_poses[joint_idx];

            // Additive blending: apply deltas scaled by weight.
            *base_t += delta_t * weight;
            *base_r = base_r.slerp(*base_r * delta_r, weight);
            *base_s *= Vec3::ONE.lerp(delta_s, weight);
        }
    }
}
```

### System Integration

The `procedural_animation_system` runs after `animation_playback_system` (which samples clips) and before `compute_world_matrices` (which produces the final GPU palette):

```rust
fn procedural_animation_system(
    mut query: Query<(&mut AnimationPlayer, &ProceduralAnimationStack, &Transform, &Velocity)>,
    time: Res<Time>,
) {
    for (mut player, stack, transform, velocity) in query.iter_mut() {
        let context = ProceduralContext {
            dt: time.delta_seconds(),
            elapsed: time.elapsed_seconds(),
            entity_position: transform.translation,
            entity_forward: transform.forward(),
            velocity: velocity.linear,
        };
        stack.apply(&context, &mut player.local_poses);
    }
}
```

## Outcome

A `ProceduralAnimationStack` component with `LookAtLayer`, `LeanLayer`, and `BreathingLayer` implementations that additively modify joint poses on top of clip-sampled animation. Each layer has an independent weight for smooth blending. The stack runs as a system between clip sampling and final matrix computation. `cargo test -p nebula-animation` passes all procedural animation layer and blending tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Feet adjust procedurally to terrain slope. No sliding on hills. The character's stance adapts to uneven ground.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `glam` | `0.32` | `Vec3`, `Quat` for joint transforms, `from_rotation_arc`, `slerp`, trigonometry |
| `bevy_ecs` | `0.18` | `Component` derive for `ProceduralAnimationStack`, `Query`, `Res<Time>` |
| `log` | `0.4` | Warn on out-of-range joint indices, NaN in procedural outputs |

All dependencies are declared in `[workspace.dependencies]` and consumed via `{ workspace = true }` in the `nebula-animation` crate's `Cargo.toml`. Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Vec3, Quat};
    use std::f32::consts::PI;

    fn base_context() -> ProceduralContext {
        ProceduralContext {
            dt: 0.016,
            elapsed: 0.0,
            entity_position: Vec3::ZERO,
            entity_forward: Vec3::NEG_Z,
            velocity: Vec3::ZERO,
        }
    }

    /// Verify that the look-at layer rotates the head joint toward the target.
    #[test]
    fn test_look_at_rotates_head_toward_target() {
        let layer = LookAtLayer {
            joint_index: 2,
            target: Vec3::new(5.0, 0.0, 0.0), // target is to the right
            max_angle: PI / 2.0,
            weight: 1.0,
        };

        let ctx = base_context();
        let (_, rotation, _) = layer.evaluate(&ctx);

        // The rotation should turn from -Z toward +X.
        let rotated_forward = rotation * Vec3::NEG_Z;
        assert!(
            rotated_forward.x > 0.5,
            "look-at should rotate toward +X, got forward: {:?}",
            rotated_forward
        );
    }

    /// Verify that the lean layer tilts with lateral movement.
    #[test]
    fn test_lean_tilts_with_movement() {
        let layer = LeanLayer {
            joint_index: 1,
            max_lean_angle: PI / 6.0,
            responsiveness: 10.0,
            current_lean: 0.0,
            weight: 1.0,
        };

        let mut ctx = base_context();
        ctx.velocity = Vec3::new(3.0, 0.0, 0.0); // moving right

        let (_, rotation, _) = layer.evaluate(&ctx);

        // Rotation should be a non-identity Z-axis rotation.
        let (axis, angle) = rotation.to_axis_angle();
        assert!(
            angle.abs() > 0.01,
            "lean should produce a non-zero rotation, got angle {}",
            angle
        );
    }

    /// Verify that the breathing layer oscillates periodically.
    #[test]
    fn test_breathing_oscillates_periodically() {
        let layer = BreathingLayer {
            joint_index: 1,
            frequency: 0.25,
            amplitude: 0.02,
            weight: 1.0,
        };

        // Sample at two different phases of the breath cycle.
        let mut ctx1 = base_context();
        ctx1.elapsed = 0.0; // sin(0) = 0
        let (t1, _, s1) = layer.evaluate(&ctx1);

        let mut ctx2 = base_context();
        ctx2.elapsed = 1.0; // sin(TAU * 0.25 * 1.0) = sin(PI/2) = 1.0
        let (t2, _, s2) = layer.evaluate(&ctx2);

        // At elapsed=0, breathing should be at neutral (sin(0)=0).
        assert!(
            t1.y.abs() < 0.001,
            "breathing at t=0 should be neutral, got y={}",
            t1.y
        );

        // At elapsed=1.0 with freq=0.25, sin(PI/2)=1, so scale should be > 1.
        assert!(
            s2.x > 1.0,
            "breathing at peak inhale should expand, got scale.x={}",
            s2.x
        );
    }

    /// Verify that additive blending combines a procedural layer with a base clip pose.
    #[test]
    fn test_additive_blending_combines_with_base_clip() {
        let layer = BreathingLayer {
            joint_index: 0,
            frequency: 0.25,
            amplitude: 0.02,
            weight: 1.0,
        };

        let stack = ProceduralAnimationStack {
            layers: vec![Box::new(layer)],
        };

        // Base pose: joint 0 at translation (1, 2, 3), identity rotation, unit scale.
        let mut poses: Vec<JointPose> = vec![
            (Vec3::new(1.0, 2.0, 3.0), Quat::IDENTITY, Vec3::ONE),
        ];

        let mut ctx = base_context();
        ctx.elapsed = 1.0; // peak inhale

        let original_y = poses[0].0.y;
        stack.apply(&ctx, &mut poses);

        // Translation should have been modified (additive breathing offset).
        assert!(
            (poses[0].0.y - original_y).abs() > 1e-5,
            "additive blending should modify translation, but y is unchanged"
        );

        // Original x and z should be preserved (breathing only affects y).
        assert!(
            (poses[0].0.x - 1.0).abs() < 1e-5,
            "x translation should be preserved"
        );
    }

    /// Verify that a layer with weight=0.0 has absolutely no effect on the pose.
    #[test]
    fn test_zero_weight_has_no_effect() {
        let layer = LookAtLayer {
            joint_index: 0,
            target: Vec3::new(100.0, 0.0, 0.0),
            max_angle: PI,
            weight: 0.0, // zero weight — should be skipped
        };

        let stack = ProceduralAnimationStack {
            layers: vec![Box::new(layer)],
        };

        let original_pose: JointPose = (Vec3::new(1.0, 2.0, 3.0), Quat::IDENTITY, Vec3::ONE);
        let mut poses = vec![original_pose];

        let ctx = base_context();
        stack.apply(&ctx, &mut poses);

        assert_eq!(poses[0].0, original_pose.0, "translation should be unchanged with weight=0");
        assert_eq!(poses[0].1, original_pose.1, "rotation should be unchanged with weight=0");
        assert_eq!(poses[0].2, original_pose.2, "scale should be unchanged with weight=0");
    }
}
```
