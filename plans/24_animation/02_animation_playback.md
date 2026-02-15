# Animation Playback

## Problem

Having a skeleton and animation clips in memory is useless without a system that advances time, samples keyframes, and computes the final joint matrices that the vertex shader needs for skinning. Every frame, each animated entity must: advance its playback clock, find the two nearest keyframes for each joint channel, interpolate between them, compose the resulting local transforms up the joint hierarchy to produce world-space joint matrices, and finally multiply each by the inverse bind matrix to produce the skinning matrix palette. This is the hottest path in the animation system — it runs for every animated entity every frame — so it must be cache-friendly and avoid unnecessary allocations.

Without a dedicated playback component, animation logic would leak into gameplay code. Without proper looping, clips would play once and freeze. Without a speed multiplier, slow-motion effects, acceleration animations, and time scaling would be impossible. And without correct hierarchical matrix composition, children would detach from parents, limbs would fly off, and the mesh would explode.

## Solution

### AnimationPlayer Component

```rust
use glam::{Vec3, Quat, Mat4};

/// Playback state for a single animation clip on a skeletal mesh entity.
pub struct AnimationPlayer {
    /// Index into the entity's animation clip library.
    pub clip_index: usize,
    /// Current playback time in seconds (always in [0, clip.duration] for looping clips).
    pub time: f32,
    /// Playback speed multiplier. 1.0 = normal, 0.5 = half speed, 2.0 = double, -1.0 = reverse.
    pub speed: f32,
    /// Whether the clip loops when it reaches the end.
    pub looping: bool,
    /// Whether the animation is currently playing.
    pub playing: bool,
    /// Computed joint local transforms for the current frame (translation, rotation, scale per joint).
    pub local_poses: Vec<(Vec3, Quat, Vec3)>,
    /// Final skinning matrix palette, ready to upload to the GPU uniform/storage buffer.
    /// Each entry is: `world_transform[joint] * inverse_bind_matrix[joint]`.
    pub joint_matrices: Vec<Mat4>,
}
```

The `AnimationPlayer` is attached as a component to any entity that has a `SkeletalMesh` and a `Skeleton`. It is created with sensible defaults: `time = 0.0`, `speed = 1.0`, `looping = true`, `playing = true`.

### Per-Frame Update

The `animation_playback_system` runs in the `Update` schedule after input but before rendering:

```rust
fn animation_playback_system(
    mut query: Query<(&mut AnimationPlayer, &Skeleton, &AnimationClipLibrary)>,
    time: Res<Time>,
) {
    let dt = time.delta_seconds();
    for (mut player, skeleton, clips) in query.iter_mut() {
        if !player.playing {
            continue;
        }
        advance_time(&mut player, clips, dt);
        sample_clip(&mut player, skeleton, clips);
        compute_world_matrices(&mut player, skeleton);
    }
}
```

### Time Advancement

```rust
fn advance_time(player: &mut AnimationPlayer, clips: &AnimationClipLibrary, dt: f32) {
    let clip = &clips[player.clip_index];
    player.time += dt * player.speed;

    if player.looping {
        // Wrap around using modular arithmetic. Handles both forward and reverse playback.
        player.time = player.time.rem_euclid(clip.duration);
    } else {
        // Clamp to [0, duration].
        player.time = player.time.clamp(0.0, clip.duration);
        if player.time >= clip.duration || player.time <= 0.0 {
            player.playing = false;
        }
    }
}
```

Using `rem_euclid` ensures correct wrapping even for negative speeds (reverse playback).

### Keyframe Sampling

For each channel in the current clip, perform binary search on the keyframe timestamps to find the two surrounding keyframes, then interpolate:

```rust
fn sample_channel<T: Interpolatable>(
    channel: &Channel<T>,
    time: f32,
) -> T {
    let keyframes = &channel.keyframes;

    // Edge cases: before first or after last keyframe.
    if time <= keyframes[0].time {
        return keyframes[0].value.clone();
    }
    if time >= keyframes.last().unwrap().time {
        return keyframes.last().unwrap().value.clone();
    }

    // Binary search for the right interval.
    let idx = keyframes.partition_point(|kf| kf.time <= time) - 1;
    let kf0 = &keyframes[idx];
    let kf1 = &keyframes[idx + 1];
    let t = (time - kf0.time) / (kf1.time - kf0.time);

    match channel.interpolation {
        Interpolation::Step => kf0.value.clone(),
        Interpolation::Linear => T::lerp(&kf0.value, &kf1.value, t),
        Interpolation::CubicSpline => {
            let dt = kf1.time - kf0.time;
            let (_, out_tangent) = kf0.tangents.as_ref().unwrap();
            let (in_tangent, _) = kf1.tangents.as_ref().unwrap();
            T::cubic_spline(&kf0.value, out_tangent, in_tangent, &kf1.value, t, dt)
        }
    }
}
```

The `Interpolatable` trait is implemented for `Vec3` (using `Vec3::lerp`), `Quat` (using `Quat::slerp` for linear, `Quat::normalize` after cubic), and `Vec3` scale (component-wise lerp). Quaternion interpolation always normalizes the result to avoid drift.

### Hierarchical Matrix Composition

Because joints are stored in topological order (parents before children, guaranteed by story 01), a single forward pass computes world transforms:

```rust
fn compute_world_matrices(player: &mut AnimationPlayer, skeleton: &Skeleton) {
    let joint_count = skeleton.joints.len();

    // Scratch buffer for world-space transforms.
    let mut world_transforms: Vec<Mat4> = Vec::with_capacity(joint_count);

    for (i, joint) in skeleton.joints.iter().enumerate() {
        let (translation, rotation, scale) = &player.local_poses[i];
        let local_matrix = Mat4::from_scale_rotation_translation(*scale, *rotation, *translation);

        let world_matrix = match joint.parent {
            Some(parent_idx) => world_transforms[parent_idx as usize] * local_matrix,
            None => local_matrix,
        };

        world_transforms.push(world_matrix);
    }

    // Compute the skinning palette: world_transform * inverse_bind_matrix.
    for (i, joint) in skeleton.joints.iter().enumerate() {
        player.joint_matrices[i] = world_transforms[i] * joint.inverse_bind_matrix;
    }
}
```

This produces the `joint_matrices` palette that the vertex shader indexes into. Each vertex stores up to 4 joint indices and 4 weights; the shader computes:

```
skinned_position = sum(weight[i] * joint_matrices[index[i]] * position, i = 0..3)
```

### GPU Upload

The `joint_matrices` slice (`&[Mat4]`) is written to a `wgpu::Buffer` (storage buffer or uniform buffer depending on joint count) each frame. A bind group exposes this buffer to the skinning shader. Buffer management is handled by the render system — `AnimationPlayer` only computes the data.

## Outcome

An `AnimationPlayer` component that, when attached to a skeletal mesh entity, drives clip playback with configurable speed and looping. Each frame, the `animation_playback_system` produces a `joint_matrices: Vec<Mat4>` palette ready for GPU upload. `cargo test -p nebula-animation` passes all playback, looping, speed, interpolation, and matrix composition tests.

## Demo Integration

**Demo crate:** `nebula-demo`

The walk cycle animation plays on the humanoid. Limbs move, torso sways. The animation loops smoothly.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `glam` | `0.32` | `Vec3`, `Quat`, `Mat4` for transforms, interpolation (`lerp`, `slerp`), and matrix math |
| `bevy_ecs` | `0.18` | `Component` derive for `AnimationPlayer`, `Query` for system iteration, `Res<Time>` |
| `wgpu` | `28.0` | Storage/uniform buffer for the joint matrix palette (consumed by the render system) |
| `log` | `0.4` | Warn on missing clips, zero-length animations, NaN in interpolated values |

All dependencies are declared in `[workspace.dependencies]` and consumed via `{ workspace = true }` in the `nebula-animation` crate's `Cargo.toml`. Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Vec3, Quat, Mat4};

    /// Helper: create a minimal skeleton with 3 joints (root -> spine -> head)
    /// and a 1-second clip with linear translation keyframes.
    fn create_test_rig() -> (Skeleton, AnimationClip) {
        let skeleton = Skeleton {
            joints: vec![
                Joint {
                    index: 0, parent: None,
                    inverse_bind_matrix: Mat4::IDENTITY,
                    name: "Root".into(),
                    bind_translation: Vec3::ZERO, bind_rotation: Quat::IDENTITY, bind_scale: Vec3::ONE,
                },
                Joint {
                    index: 1, parent: Some(0),
                    inverse_bind_matrix: Mat4::from_translation(Vec3::new(0.0, -1.0, 0.0)),
                    name: "Spine".into(),
                    bind_translation: Vec3::new(0.0, 1.0, 0.0), bind_rotation: Quat::IDENTITY, bind_scale: Vec3::ONE,
                },
                Joint {
                    index: 2, parent: Some(1),
                    inverse_bind_matrix: Mat4::from_translation(Vec3::new(0.0, -2.0, 0.0)),
                    name: "Head".into(),
                    bind_translation: Vec3::new(0.0, 1.0, 0.0), bind_rotation: Quat::IDENTITY, bind_scale: Vec3::ONE,
                },
            ],
        };
        let clip = AnimationClip {
            name: "test_anim".into(),
            duration: 1.0,
            translation_channels: vec![
                Channel {
                    joint_index: 0,
                    interpolation: Interpolation::Linear,
                    keyframes: vec![
                        Keyframe { time: 0.0, value: Vec3::ZERO, tangents: None },
                        Keyframe { time: 1.0, value: Vec3::new(2.0, 0.0, 0.0), tangents: None },
                    ],
                },
            ],
            rotation_channels: vec![],
            scale_channels: vec![],
        };
        (skeleton, clip)
    }

    /// Verify that playback time advances by delta_time * speed.
    #[test]
    fn test_playback_advances_time() {
        let (_, clip) = create_test_rig();
        let mut player = AnimationPlayer::new(0, clip.duration, 3);
        player.speed = 1.0;
        player.playing = true;
        player.looping = false;

        let dt = 0.1;
        advance_time(&mut player, &[clip], dt);
        assert!(
            (player.time - 0.1).abs() < 1e-6,
            "time should be 0.1 after one 0.1s step, got {}",
            player.time
        );
    }

    /// Verify that looping wraps time around to the beginning of the clip.
    #[test]
    fn test_looping_wraps_around() {
        let (_, clip) = create_test_rig();
        let mut player = AnimationPlayer::new(0, clip.duration, 3);
        player.time = 0.9;
        player.speed = 1.0;
        player.looping = true;

        advance_time(&mut player, &[clip], 0.2); // 0.9 + 0.2 = 1.1 -> wraps to 0.1
        assert!(
            (player.time - 0.1).abs() < 1e-4,
            "looping time should wrap to ~0.1, got {}",
            player.time
        );
    }

    /// Verify that the speed multiplier scales time advancement.
    #[test]
    fn test_speed_multiplier_works() {
        let (_, clip) = create_test_rig();
        let mut player = AnimationPlayer::new(0, clip.duration, 3);
        player.speed = 2.0;
        player.looping = false;

        advance_time(&mut player, &[clip], 0.1); // 0.0 + 0.1 * 2.0 = 0.2
        assert!(
            (player.time - 0.2).abs() < 1e-6,
            "time should be 0.2 with speed=2.0 after 0.1s, got {}",
            player.time
        );
    }

    /// Verify that linear interpolation at t=0.5 produces a midpoint value.
    #[test]
    fn test_interpolation_produces_smooth_values() {
        let channel = Channel {
            joint_index: 0,
            interpolation: Interpolation::Linear,
            keyframes: vec![
                Keyframe { time: 0.0, value: Vec3::ZERO, tangents: None },
                Keyframe { time: 1.0, value: Vec3::new(4.0, 0.0, 0.0), tangents: None },
            ],
        };

        let result = sample_channel(&channel, 0.5);
        assert!(
            (result.x - 2.0).abs() < 1e-5,
            "linear interp at t=0.5 should give x=2.0, got {}",
            result.x
        );
        assert!(result.y.abs() < 1e-5);
        assert!(result.z.abs() < 1e-5);
    }

    /// Verify that joint matrices are correct for bind pose (time = 0, identity animation).
    /// In bind pose, world_transform * inverse_bind_matrix == IDENTITY for every joint.
    #[test]
    fn test_joint_matrices_correct_for_bind_pose() {
        let (skeleton, _) = create_test_rig();
        let mut player = AnimationPlayer::new(0, 1.0, skeleton.joints.len());

        // Set local poses to bind pose.
        for (i, joint) in skeleton.joints.iter().enumerate() {
            player.local_poses[i] = (joint.bind_translation, joint.bind_rotation, joint.bind_scale);
        }

        compute_world_matrices(&mut player, &skeleton);

        // In bind pose, each joint_matrix should be (close to) identity.
        for (i, mat) in player.joint_matrices.iter().enumerate() {
            let diff = (*mat - Mat4::IDENTITY).abs_diff_eq(Mat4::ZERO, 1e-5);
            assert!(
                diff,
                "joint_matrix[{}] ('{}') should be identity in bind pose, got {:?}",
                i, skeleton.joints[i].name, mat
            );
        }
    }
}
```
