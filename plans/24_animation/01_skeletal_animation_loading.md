# Skeletal Animation Loading

## Problem

Before any character, creature, or articulated object can animate, the engine must extract skeletal animation data from artist-authored assets and store it in an efficient, GPU-friendly runtime format. glTF 2.0 (loaded via the `gltf` crate version 1.4) is the canonical asset format, and it packs skeletal data across multiple accessor indirections: skins reference joint nodes, inverse bind matrices live in a separate buffer view, and animation channels scatter keyframes across samplers with different interpolation modes. Parsing this correctly is non-trivial — joint indices must map to the engine's internal joint array, inverse bind matrices must be extracted in the right order, and animation clips must collate per-joint channels with properly sorted keyframes. Getting any of this wrong produces invisible bones, exploded meshes, or hitching playback.

Additionally, the raw glTF data model is unsuitable for real-time use. Joints are referenced by node index (which can be sparse and non-contiguous), animations store timestamps and values in separate accessors, and the hierarchy is implicit through the node parent chain. The engine needs a flattened, cache-friendly representation where the joint hierarchy is explicit, keyframes are pre-sorted and ready for binary search, and the entire skeleton plus its clips can be uploaded or queried without re-traversing the glTF node graph.

## Solution

### Data Structures

Define the runtime skeletal types in the `nebula_animation` crate:

```rust
use glam::{Vec3, Quat, Mat4};

/// A single joint in the skeleton hierarchy.
pub struct Joint {
    /// Index of this joint in the skeleton's joint array.
    pub index: u16,
    /// Index of the parent joint, or `None` for root joints.
    pub parent: Option<u16>,
    /// Inverse bind matrix: transforms from mesh space to this joint's local space in bind pose.
    pub inverse_bind_matrix: Mat4,
    /// Human-readable name from the glTF node (e.g., "Hips", "LeftForeArm").
    pub name: String,
    /// Bind-pose local translation relative to parent.
    pub bind_translation: Vec3,
    /// Bind-pose local rotation relative to parent.
    pub bind_rotation: Quat,
    /// Bind-pose local scale relative to parent.
    pub bind_scale: Vec3,
}

/// The complete skeleton extracted from a glTF skin.
pub struct Skeleton {
    /// Ordered joint array. Index 0 is always the root joint.
    /// Children always appear after their parent (topological order).
    pub joints: Vec<Joint>,
}

/// Interpolation method for keyframes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interpolation {
    Linear,
    Step,
    CubicSpline,
}

/// A single keyframe for one property of one joint.
#[derive(Debug, Clone)]
pub struct Keyframe<T: Clone> {
    pub time: f32,
    pub value: T,
    /// In-tangent and out-tangent, present only for CubicSpline interpolation.
    pub tangents: Option<(T, T)>,
}

/// A channel targets one joint and one transform property.
pub struct Channel<T: Clone> {
    pub joint_index: u16,
    pub interpolation: Interpolation,
    pub keyframes: Vec<Keyframe<T>>,
}

/// A complete animation clip with all channels.
pub struct AnimationClip {
    pub name: String,
    /// Total duration in seconds (max timestamp across all channels).
    pub duration: f32,
    pub translation_channels: Vec<Channel<Vec3>>,
    pub rotation_channels: Vec<Channel<Quat>>,
    pub scale_channels: Vec<Channel<Vec3>>,
}
```

### Loading Pipeline

`load_skeleton_and_clips(path: &Path) -> Result<(Skeleton, Vec<AnimationClip>), AnimationLoadError>`:

1. **Parse the glTF file** using `gltf::open(path)` which returns the document, buffers, and images. Only the document and buffers are needed for skeletal data.

2. **Extract the skin**. Iterate `document.skins()`, take the first skin (multi-skin support deferred to a later story). The skin provides:
   - `skin.joints()`: an ordered list of glTF `Node` references.
   - `skin.inverse_bind_matrices()`: an accessor pointing to a contiguous `[Mat4; joint_count]` array in the buffer.
   - `skin.skeleton()`: optional root node hint.

3. **Build the joint index map**. glTF joint nodes are referenced by their global node index, but the engine's `Joint::index` is the position within the skin's joint array. Build a `HashMap<usize, u16>` mapping glTF node index to engine joint index.

4. **Extract joints in topological order**. Walk the glTF joint list. For each joint node:
   - Read the node's local transform (translation, rotation, scale via `node.transform().decomposed()`).
   - Look up the parent node. If the parent is also a joint (present in the index map), record `parent: Some(mapped_index)`. Otherwise, `parent: None`.
   - Read the corresponding inverse bind matrix from the accessor.
   - Store as a `Joint`.

   After construction, sort the `joints` vector so that every parent appears before its children (topological sort). Update all `parent` and `index` fields to reflect the new ordering. This guarantees that a single forward pass through the array can compute world transforms without backtracking.

5. **Validate the skeleton**:
   - Exactly one root joint (one joint with `parent: None`), or multiple roots if the glTF defines them — but at least one.
   - No cycles in the hierarchy (guaranteed by topological sort succeeding).
   - Every inverse bind matrix is a valid affine transform (determinant is non-zero, no NaN).

6. **Extract animation clips**. Iterate `document.animations()`. For each animation:
   - Read the name (or generate "Animation_N" if unnamed).
   - Iterate `animation.channels()`. Each channel has:
     - A target node (map to engine joint index via the joint index map; skip channels targeting non-joint nodes).
     - A property (`Translation`, `Rotation`, `Scale`).
     - A sampler with an interpolation mode and input/output accessors.
   - Read the input accessor (timestamps as `Vec<f32>`) and output accessor (values as `Vec<Vec3>` or `Vec<Quat>`).
   - For `CubicSpline`, the output accessor contains `3 * keyframe_count` elements (in-tangent, value, out-tangent triples). Deinterleave these into `Keyframe` structs with populated `tangents`.
   - Assert keyframes are sorted by time. If not (malformed asset), sort them.
   - Compute `duration` as the maximum timestamp across all channels.

7. **Return** the `Skeleton` and `Vec<AnimationClip>`.

### Error Handling

```rust
#[derive(Debug, thiserror::Error)]
pub enum AnimationLoadError {
    #[error("failed to open glTF file: {0}")]
    Io(#[from] std::io::Error),
    #[error("glTF parse error: {0}")]
    Gltf(#[from] gltf::Error),
    #[error("no skin found in glTF file")]
    NoSkin,
    #[error("inverse bind matrix accessor missing")]
    MissingInverseBindMatrices,
    #[error("joint hierarchy contains a cycle")]
    CyclicHierarchy,
    #[error("inverse bind matrix for joint {0} is degenerate (det = {1})")]
    DegenerateMatrix(u16, f32),
}
```

## Outcome

A `load_skeleton_and_clips` function that takes a path to a glTF file and returns a `Skeleton` (with topologically sorted joints, inverse bind matrices, and bind-pose transforms) and a `Vec<AnimationClip>` (with pre-sorted keyframes, typed channels, and correct interpolation metadata). `cargo test -p nebula-animation` passes all skeleton and clip loading tests. Downstream systems (`AnimationPlayer`, state machine, GPU skinning) consume these types directly without any further glTF parsing.

## Demo Integration

**Demo crate:** `nebula-demo`

A humanoid glTF model with bone hierarchy is loaded. The character stands in T-pose on the terrain surface.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `gltf` | `1.4` | Parse glTF 2.0 files; extract skins, nodes, animations, and buffer data |
| `glam` | `0.32` | `Vec3`, `Quat`, `Mat4` for joint transforms and inverse bind matrices |
| `thiserror` | `2.0` | Derive macro for `AnimationLoadError` |
| `log` | `0.4` | Warn on unnamed animations, skipped non-joint channels, unsorted keyframes |

All dependencies are declared in `[workspace.dependencies]` and consumed via `{ workspace = true }` in the `nebula-animation` crate's `Cargo.toml`. Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// Helper: load the test fixture "simple_humanoid.glb" which contains
    /// a 25-joint humanoid skeleton with a 2-second walk cycle.
    fn load_test_skeleton() -> (Skeleton, Vec<AnimationClip>) {
        load_skeleton_and_clips(Path::new("test_assets/simple_humanoid.glb"))
            .expect("test asset should load without error")
    }

    /// Verify that the skeleton loads with the expected number of joints.
    #[test]
    fn test_skeleton_loads_correct_joint_count() {
        let (skeleton, _) = load_test_skeleton();
        assert_eq!(skeleton.joints.len(), 25, "humanoid skeleton should have 25 joints");
    }

    /// Verify that every inverse bind matrix has a non-zero determinant
    /// (i.e., it is invertible and not degenerate).
    #[test]
    fn test_inverse_bind_matrices_are_valid() {
        let (skeleton, _) = load_test_skeleton();
        for joint in &skeleton.joints {
            let det = joint.inverse_bind_matrix.determinant();
            assert!(
                det.abs() > f32::EPSILON,
                "joint '{}' has degenerate inverse bind matrix (det = {})",
                joint.name, det
            );
            assert!(
                !det.is_nan(),
                "joint '{}' has NaN in inverse bind matrix determinant",
                joint.name
            );
        }
    }

    /// Verify that the animation clip has the expected duration.
    #[test]
    fn test_animation_clip_has_expected_duration() {
        let (_, clips) = load_test_skeleton();
        assert!(!clips.is_empty(), "should have at least one animation clip");
        let walk = &clips[0];
        assert!(
            (walk.duration - 2.0).abs() < 0.01,
            "walk cycle should be ~2.0 seconds, got {}",
            walk.duration
        );
    }

    /// Verify that keyframes in every channel are sorted by ascending time.
    #[test]
    fn test_keyframes_are_sorted_by_time() {
        let (_, clips) = load_test_skeleton();
        for clip in &clips {
            for channel in &clip.translation_channels {
                for window in channel.keyframes.windows(2) {
                    assert!(
                        window[0].time <= window[1].time,
                        "translation keyframes out of order: {} > {}",
                        window[0].time, window[1].time
                    );
                }
            }
            for channel in &clip.rotation_channels {
                for window in channel.keyframes.windows(2) {
                    assert!(
                        window[0].time <= window[1].time,
                        "rotation keyframes out of order: {} > {}",
                        window[0].time, window[1].time
                    );
                }
            }
            for channel in &clip.scale_channels {
                for window in channel.keyframes.windows(2) {
                    assert!(
                        window[0].time <= window[1].time,
                        "scale keyframes out of order: {} > {}",
                        window[0].time, window[1].time
                    );
                }
            }
        }
    }

    /// Verify that the joint hierarchy is a valid tree: every parent index
    /// is less than the child's index (topological ordering) and exactly
    /// one root exists (parent == None).
    #[test]
    fn test_joint_hierarchy_is_valid_tree() {
        let (skeleton, _) = load_test_skeleton();
        let root_count = skeleton.joints.iter().filter(|j| j.parent.is_none()).count();
        assert!(root_count >= 1, "skeleton must have at least one root joint");

        for (i, joint) in skeleton.joints.iter().enumerate() {
            assert_eq!(joint.index as usize, i, "joint index must match array position");
            if let Some(parent) = joint.parent {
                assert!(
                    (parent as usize) < i,
                    "parent index {} must be less than child index {} for topological order",
                    parent, i
                );
            }
        }
    }
}
```
