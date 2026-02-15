# LOD Pop-In Mitigation

## Problem

When a chunk transitions between LOD levels — for example, switching from LOD 2 (8x8x8 coarse mesh) to LOD 1 (16x16x16 finer mesh) as the camera approaches — the visual change is instantaneous. One frame the chunk is rendered with 64 voxels; the next frame it suddenly has 4,096 voxels with completely different geometry. This abrupt change is called "pop-in" and is one of the most visually jarring artifacts in LOD-based rendering systems. Players notice terrain details appearing out of nowhere, especially at the edges of the LOD transition zones where chunks are constantly upgrading and downgrading. On a planet with a smooth horizon, a ring of popping chunks around the player destroys the sense of immersion. The engine needs techniques to make LOD transitions gradual rather than instantaneous.

## Solution

Implement LOD transition smoothing in the `nebula_lod` and `nebula_rendering` crates using three complementary techniques: **cross-fading** (alpha blending between the old and new LOD meshes over several frames), **overlap loading** (keeping both LOD meshes alive during the transition), and **geometry morphing** (smoothly interpolating vertex positions between LOD levels). These techniques are applied in the rendering pipeline, not the LOD selection logic.

### Transition State Machine

```rust
/// The state of a chunk's LOD transition.
#[derive(Clone, Debug)]
pub enum LodTransitionState {
    /// The chunk is stable at its current LOD level. No transition in progress.
    Stable {
        lod: u8,
    },
    /// The chunk is transitioning between two LOD levels.
    Transitioning {
        /// The LOD level being transitioned away from.
        from_lod: u8,
        /// The LOD level being transitioned to.
        to_lod: u8,
        /// Progress of the transition, from 0.0 (just started) to 1.0 (complete).
        progress: f32,
        /// Total duration of the transition in seconds.
        duration: f32,
    },
}

/// Configuration for LOD transition behavior.
#[derive(Clone, Debug)]
pub struct LodTransitionConfig {
    /// Duration of the crossfade in seconds. Default: 0.3 seconds (~18 frames at 60fps).
    pub crossfade_duration: f32,
    /// Whether to use geometry morphing (more GPU cost but smoother).
    pub enable_morph: bool,
    /// Whether to use alpha crossfade (simpler but requires transparency pass).
    pub enable_crossfade: bool,
}

impl Default for LodTransitionConfig {
    fn default() -> Self {
        Self {
            crossfade_duration: 0.3,
            enable_morph: true,
            enable_crossfade: true,
        }
    }
}
```

### Transition Manager

```rust
use std::collections::HashMap;

/// Manages LOD transitions for all active chunks.
pub struct LodTransitionManager {
    config: LodTransitionConfig,
    /// Current transition state for each chunk.
    states: HashMap<ChunkAddress, LodTransitionState>,
}

impl LodTransitionManager {
    pub fn new(config: LodTransitionConfig) -> Self {
        Self {
            config,
            states: HashMap::new(),
        }
    }

    /// Notify the manager that a chunk's LOD level has changed.
    /// This initiates a transition from the old LOD to the new LOD.
    pub fn on_lod_changed(&mut self, address: ChunkAddress, from_lod: u8, to_lod: u8) {
        self.states.insert(address, LodTransitionState::Transitioning {
            from_lod,
            to_lod,
            progress: 0.0,
            duration: self.config.crossfade_duration,
        });
    }

    /// Advance all active transitions by the given delta time (in seconds).
    /// Returns the list of chunks that completed their transitions this frame.
    pub fn update(&mut self, dt: f32) -> Vec<ChunkAddress> {
        let mut completed = Vec::new();

        for (address, state) in &mut self.states {
            if let LodTransitionState::Transitioning {
                to_lod,
                progress,
                duration,
                ..
            } = state
            {
                *progress += dt / *duration;
                if *progress >= 1.0 {
                    completed.push(*address);
                    *state = LodTransitionState::Stable { lod: *to_lod };
                }
            }
        }

        // Clean up completed transitions (keep Stable state for reference)
        completed
    }

    /// Get the current transition state for a chunk.
    pub fn get_state(&self, address: &ChunkAddress) -> Option<&LodTransitionState> {
        self.states.get(address)
    }

    /// Get the alpha values for the old and new LOD meshes during a transition.
    /// Returns (old_alpha, new_alpha) where both are in [0.0, 1.0].
    pub fn get_crossfade_alphas(&self, address: &ChunkAddress) -> (f32, f32) {
        match self.states.get(address) {
            Some(LodTransitionState::Transitioning { progress, .. }) => {
                // Smooth step interpolation for less jarring transitions
                let t = smooth_step(*progress);
                (1.0 - t, t)
            }
            _ => (0.0, 1.0), // Stable: only new mesh is visible
        }
    }
}

/// Hermite smooth step for natural-looking transitions.
fn smooth_step(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
```

### Cross-Fade Rendering

During a transition, both the old and new LOD meshes are rendered with complementary alpha values:

```rust
/// Render a chunk that may be in a LOD transition.
pub fn render_chunk_with_transition(
    chunk: &LoadedChunk,
    transition: &LodTransitionManager,
    render_pass: &mut RenderPass,
) {
    let (old_alpha, new_alpha) = transition.get_crossfade_alphas(&chunk.address);

    if old_alpha > 0.001 {
        if let Some(old_mesh) = chunk.previous_lod_mesh() {
            render_pass.draw_mesh_with_alpha(old_mesh, old_alpha);
        }
    }

    if new_alpha > 0.001 {
        render_pass.draw_mesh_with_alpha(chunk.current_mesh(), new_alpha);
    }
}
```

### Geometry Morphing

For an even smoother transition, vertex positions can be morphed between LOD levels. Each vertex in the higher-LOD mesh stores a "morph target" position — the position it would have at the lower LOD level. During the transition, the vertex shader interpolates between the two positions:

```rust
/// Vertex data with morph target for LOD transitions.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MorphVertex {
    /// Position at the current (target) LOD level.
    pub position: [f32; 3],
    /// Position at the previous (source) LOD level.
    /// Interpolated toward `position` as the transition progresses.
    pub morph_position: [f32; 3],
    /// Normal at the current LOD level.
    pub normal: [f32; 3],
    /// Normal at the previous LOD level.
    pub morph_normal: [f32; 3],
}
```

The vertex shader performs the interpolation:

```wgsl
// WGSL vertex shader snippet
struct Uniforms {
    morph_factor: f32, // 0.0 = morph_position, 1.0 = position
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    let pos = mix(in.morph_position, in.position, uniforms.morph_factor);
    let nrm = normalize(mix(in.morph_normal, in.normal, uniforms.morph_factor));
    // ... transform and output
}
```

### Overlap Loading Strategy

To ensure both meshes are available during the transition, the chunk system must:
1. When a LOD change is requested, generate the new LOD mesh without discarding the old one.
2. Keep both meshes in GPU memory for the duration of the transition.
3. Only release the old mesh after the transition completes (progress reaches 1.0).

This doubles the memory usage for transitioning chunks, but only a small number of chunks are typically transitioning at any given time (the "ring" at each LOD boundary).

## Outcome

The `nebula_lod` crate exports `LodTransitionState`, `LodTransitionConfig`, and `LodTransitionManager`. The `nebula_rendering` crate exports `MorphVertex`, `render_chunk_with_transition()`, and the corresponding WGSL shader snippet. LOD transitions are smooth, taking `crossfade_duration` seconds instead of being instantaneous. Running `cargo test -p nebula_lod` passes all pop-in mitigation tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Moving the camera causes LOD changes to fade in smoothly over a configurable duration rather than popping abruptly. The transition is barely noticeable during normal movement.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `nebula_lod` | workspace (self) | `ChunkAddress`, LOD types |
| `nebula_rendering` | workspace | Render pass, GPU buffer management |
| `nebula_math` | workspace | Vector interpolation |
| `wgpu` | `24.0` | GPU vertex buffer layout, shader uniforms |
| `bytemuck` | `1.21` | Safe casting of `MorphVertex` to GPU byte buffer |

The crate uses Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn make_address(id: u64) -> ChunkAddress {
        ChunkAddress {
            face: CubeFace::PosY,
            quadtree_path: id,
            lod: 0,
        }
    }

    /// A LOD transition should take multiple frames, not be instantaneous.
    #[test]
    fn test_transition_takes_multiple_frames() {
        let mut manager = LodTransitionManager::new(LodTransitionConfig {
            crossfade_duration: 0.3,
            ..Default::default()
        });

        let addr = make_address(1);
        manager.on_lod_changed(addr, 2, 1);

        // After one frame at 60fps (~16ms), transition should not be complete
        let completed = manager.update(0.016);
        assert!(completed.is_empty(), "transition should not complete in one frame");

        match manager.get_state(&addr) {
            Some(LodTransitionState::Transitioning { progress, .. }) => {
                assert!(*progress > 0.0 && *progress < 1.0,
                    "progress should be between 0 and 1, got {progress}");
            }
            other => panic!("expected Transitioning state, got {other:?}"),
        }
    }

    /// Both LOD meshes should exist during the transition (overlap).
    #[test]
    fn test_both_meshes_exist_during_transition() {
        let mut manager = LodTransitionManager::new(LodTransitionConfig::default());

        let addr = make_address(1);
        manager.on_lod_changed(addr, 2, 1);

        // Midway through the transition
        manager.update(0.15); // half of 0.3 seconds

        let (old_alpha, new_alpha) = manager.get_crossfade_alphas(&addr);

        // Both meshes should have non-zero alpha
        assert!(old_alpha > 0.0, "old mesh should be visible during transition");
        assert!(new_alpha > 0.0, "new mesh should be visible during transition");
        assert!(old_alpha > 0.1, "old mesh alpha should be significant at midpoint");
        assert!(new_alpha > 0.1, "new mesh alpha should be significant at midpoint");
    }

    /// Alpha values should interpolate from (1,0) to (0,1) during the transition.
    #[test]
    fn test_alpha_interpolates_zero_to_one() {
        let mut manager = LodTransitionManager::new(LodTransitionConfig {
            crossfade_duration: 1.0, // 1 second for easy math
            ..Default::default()
        });

        let addr = make_address(1);
        manager.on_lod_changed(addr, 2, 1);

        // At the start, old mesh should be fully visible, new mesh invisible
        let (old_alpha, new_alpha) = manager.get_crossfade_alphas(&addr);
        assert!((old_alpha - 1.0).abs() < 0.01, "old alpha should start at ~1.0, got {old_alpha}");
        assert!(new_alpha < 0.01, "new alpha should start at ~0.0, got {new_alpha}");

        // At the midpoint
        manager.update(0.5);
        let (old_alpha, new_alpha) = manager.get_crossfade_alphas(&addr);
        assert!((old_alpha - 0.5).abs() < 0.15, "old alpha should be ~0.5 at midpoint, got {old_alpha}");
        assert!((new_alpha - 0.5).abs() < 0.15, "new alpha should be ~0.5 at midpoint, got {new_alpha}");

        // At the end
        manager.update(0.5);
        let completed = manager.update(0.0); // check without advancing
        // Transition should have completed
        let (old_alpha, new_alpha) = manager.get_crossfade_alphas(&addr);
        assert!(old_alpha < 0.01, "old alpha should be ~0.0 at end, got {old_alpha}");
        assert!((new_alpha - 1.0).abs() < 0.01, "new alpha should be ~1.0 at end, got {new_alpha}");
    }

    /// Transition duration should be configurable.
    #[test]
    fn test_transition_duration_configurable() {
        // Short transition: 0.1 seconds
        let mut manager_fast = LodTransitionManager::new(LodTransitionConfig {
            crossfade_duration: 0.1,
            ..Default::default()
        });

        // Long transition: 2.0 seconds
        let mut manager_slow = LodTransitionManager::new(LodTransitionConfig {
            crossfade_duration: 2.0,
            ..Default::default()
        });

        let addr = make_address(1);
        manager_fast.on_lod_changed(addr, 2, 1);
        manager_slow.on_lod_changed(addr, 2, 1);

        // After 0.15 seconds, the fast transition should be complete but slow should not
        let completed_fast = manager_fast.update(0.15);
        let completed_slow = manager_slow.update(0.15);

        assert!(!completed_fast.is_empty(), "fast transition should complete in 0.15s");
        assert!(completed_slow.is_empty(), "slow transition should not complete in 0.15s");
    }

    /// During crossfade, there should be no visual artifacts:
    /// the sum of old and new alpha should be approximately 1.0 at all times
    /// to avoid the chunk appearing to flash bright or go dark.
    #[test]
    fn test_no_visual_artifacts_during_crossfade() {
        let mut manager = LodTransitionManager::new(LodTransitionConfig {
            crossfade_duration: 1.0,
            ..Default::default()
        });

        let addr = make_address(1);
        manager.on_lod_changed(addr, 2, 1);

        // Sample at multiple points during the transition
        for i in 0..20 {
            manager.update(0.05); // advance by 50ms each step
            let (old_alpha, new_alpha) = manager.get_crossfade_alphas(&addr);

            // The alpha sum should be close to 1.0 to avoid brightness artifacts.
            // With smooth_step, this won't be exactly 1.0, but it should be close.
            let alpha_sum = old_alpha + new_alpha;
            assert!(
                (alpha_sum - 1.0).abs() < 0.15,
                "alpha sum should be ~1.0 at step {i}, got {alpha_sum} (old={old_alpha}, new={new_alpha})"
            );
        }
    }

    /// The smooth_step function should produce correct values.
    #[test]
    fn test_smooth_step_correctness() {
        assert!((smooth_step(0.0) - 0.0).abs() < f32::EPSILON);
        assert!((smooth_step(0.5) - 0.5).abs() < f32::EPSILON);
        assert!((smooth_step(1.0) - 1.0).abs() < f32::EPSILON);

        // Smooth step should be monotonically increasing
        let mut prev = 0.0f32;
        for i in 0..=100 {
            let t = i as f32 / 100.0;
            let v = smooth_step(t);
            assert!(v >= prev, "smooth_step should be monotonic: {prev} -> {v} at t={t}");
            prev = v;
        }
    }
}
```
