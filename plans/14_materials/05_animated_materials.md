# Animated Materials

## Problem

Static textures work for stone and dirt, but many voxel types demand visual animation — water flows, lava pulses with heat, and crystals twinkle with refracted light. Without an animation system for materials, these surfaces look lifeless and break immersion. The animation must be efficient: updating every vertex's UV each frame is too expensive for millions of visible voxels. Instead, animation should work through the atlas — defining a sequence of sub-regions in the texture atlas that the shader cycles through based on a per-material frame index. This keeps vertex buffers static while producing smooth visual motion.

## Solution

Implement a `MaterialAnimator` system in the `nebula_materials` crate that advances frame indices for animated materials each tick and passes the current frame offsets to the GPU shader via a per-material animation uniform buffer.

### Animation Definition

Each animated material defines a sequence of atlas tile indices and a playback speed:

```rust
/// Describes an animated material's frame sequence.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct MaterialAnimation {
    /// Ordered list of atlas tile indices forming the animation loop.
    pub frames: Vec<u32>,
    /// Playback speed in frames per second.
    pub fps: f32,
    /// Whether the animation ping-pongs (reverses at end) or loops.
    pub ping_pong: bool,
}
```

Animations are defined alongside materials in the RON manifest:

```ron
(
    name: "water",
    albedo: (0.2, 0.4, 0.8, 0.9),
    metallic: 0.0,
    roughness: 0.1,
    emissive_color: (0.0, 0.0, 0.0),
    emissive_intensity: 0.0,
    normal_strength: 1.0,
    opacity: 0.9,
    textures: Uniform(texture: "textures/water_01.png"),
    animation: Some(MaterialAnimation(
        frames: [0, 1, 2, 3, 4, 5, 6, 7],
        fps: 8.0,
        ping_pong: false,
    )),
),
```

The `frames` list references tile indices within the atlas. For water with 8 frames, 8 separate water textures are loaded into the atlas and the animation cycles through them.

### MaterialAnimator System

```rust
/// Tracks animation state for all animated materials.
pub struct MaterialAnimator {
    /// Per-material animation state. Non-animated materials have None.
    states: Vec<Option<AnimationState>>,
}

struct AnimationState {
    animation: MaterialAnimation,
    /// Accumulated time in seconds since animation start.
    elapsed: f32,
    /// Current frame index into the frames array.
    current_frame_index: usize,
    /// Direction for ping-pong: true = forward, false = backward.
    forward: bool,
}

impl MaterialAnimator {
    /// Create the animator from the material registry's animation data.
    pub fn new(animations: &[(MaterialId, MaterialAnimation)]) -> Self {
        // Initialize states for each animated material
    }

    /// Advance all animations by `dt` seconds.
    pub fn tick(&mut self, dt: f32) {
        for state in self.states.iter_mut().flatten() {
            state.elapsed += dt;
            let frame_duration = 1.0 / state.animation.fps;
            let total_frames = state.animation.frames.len();

            while state.elapsed >= frame_duration {
                state.elapsed -= frame_duration;

                if state.animation.ping_pong {
                    if state.forward {
                        if state.current_frame_index + 1 < total_frames {
                            state.current_frame_index += 1;
                        } else {
                            state.forward = false;
                            if state.current_frame_index > 0 {
                                state.current_frame_index -= 1;
                            }
                        }
                    } else {
                        if state.current_frame_index > 0 {
                            state.current_frame_index -= 1;
                        } else {
                            state.forward = true;
                            state.current_frame_index += 1;
                        }
                    }
                } else {
                    state.current_frame_index = (state.current_frame_index + 1) % total_frames;
                }
            }
        }
    }

    /// Returns the current atlas tile index for a material.
    /// For non-animated materials, returns None (the base tile is used).
    pub fn current_tile(&self, id: MaterialId) -> Option<u32> {
        self.states.get(id.0 as usize)
            .and_then(|s| s.as_ref())
            .map(|s| s.animation.frames[s.current_frame_index])
    }

    /// Returns the current frame index (position in the frames array) for a material.
    pub fn current_frame_index(&self, id: MaterialId) -> Option<usize> {
        self.states.get(id.0 as usize)
            .and_then(|s| s.as_ref())
            .map(|s| s.current_frame_index)
    }
}
```

### GPU Communication

The current animation tile offsets are uploaded each frame as a storage buffer that the fragment shader reads:

```rust
/// Per-material animation data uploaded to the GPU each frame.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct AnimationGpuData {
    /// UV offset from the base tile to the current animation frame.
    pub uv_offset: [f32; 2],
    /// Padding to maintain 16-byte alignment.
    pub _padding: [f32; 2],
}
```

The shader adds `uv_offset` to the base UV coordinates when sampling the atlas:

```wgsl
@group(2) @binding(3) var<storage, read> anim_data: array<AnimationGpuData>;

// In fs_main:
let anim = anim_data[in.material_id];
let animated_uv = in.uv + anim.uv_offset;
let tex_color = textureSample(atlas_texture, atlas_sampler, animated_uv);
```

### ECS Integration

The `MaterialAnimator` runs as an ECS system in the `Update` stage, before rendering:

```rust
fn animate_materials_system(
    time: Res<Time>,
    mut animator: ResMut<MaterialAnimator>,
    mut anim_buffer: ResMut<AnimationGpuBuffer>,
    queue: Res<wgpu::Queue>,
) {
    animator.tick(time.delta_seconds());
    anim_buffer.update(&animator, &queue);
}
```

## Outcome

A `MaterialAnimator` system in `nebula_materials` that advances frame indices for animated materials (water, lava, crystals) each tick and uploads UV offsets to the GPU. Vertex buffers remain unchanged — animation is entirely driven by per-material UV offsets in a storage buffer. Non-animated materials are completely unaffected (zero UV offset). Running `cargo test -p nebula_materials` passes all animation tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Water surfaces shimmer with an animated UV offset. Lava flows slowly. The texture atlas supports animation frames for dynamic materials.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `serde` | `1.0` with `derive` | Deserialize `MaterialAnimation` from RON |
| `bytemuck` | `1.21` | Pod/Zeroable for `AnimationGpuData` |
| `wgpu` | `28.0` | Animation buffer upload |
| `glam` | `0.32` | UV offset math |

Depends on stories 14/01 (`MaterialId`), 14/02 (`TextureAtlas` tile UVs), and 14/03 (`MaterialRegistry`). Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn water_animation() -> MaterialAnimation {
        MaterialAnimation {
            frames: vec![10, 11, 12, 13],
            fps: 4.0,
            ping_pong: false,
        }
    }

    fn lava_animation() -> MaterialAnimation {
        MaterialAnimation {
            frames: vec![20, 21, 22],
            fps: 2.0,
            ping_pong: true,
        }
    }

    #[test]
    fn test_animated_material_advances_frames() {
        let water_id = MaterialId(1);
        let mut animator = MaterialAnimator::new(&[
            (water_id, water_animation()),
        ]);

        // Initially at frame 0
        assert_eq!(animator.current_frame_index(water_id), Some(0));
        assert_eq!(animator.current_tile(water_id), Some(10));

        // After 0.25s at 4fps, should advance to frame 1
        animator.tick(0.25);
        assert_eq!(animator.current_frame_index(water_id), Some(1));
        assert_eq!(animator.current_tile(water_id), Some(11));

        // After another 0.25s, frame 2
        animator.tick(0.25);
        assert_eq!(animator.current_frame_index(water_id), Some(2));
        assert_eq!(animator.current_tile(water_id), Some(12));
    }

    #[test]
    fn test_frame_wraps_around_at_end() {
        let water_id = MaterialId(1);
        let mut animator = MaterialAnimator::new(&[
            (water_id, water_animation()),
        ]);

        // 4 frames at 4fps = 1.0s per full cycle
        // Advance to the last frame
        animator.tick(0.75); // frame 3
        assert_eq!(animator.current_frame_index(water_id), Some(3));
        assert_eq!(animator.current_tile(water_id), Some(13));

        // Next tick wraps to frame 0
        animator.tick(0.25);
        assert_eq!(animator.current_frame_index(water_id), Some(0));
        assert_eq!(animator.current_tile(water_id), Some(10));
    }

    #[test]
    fn test_animation_speed_is_configurable() {
        let slow_anim = MaterialAnimation {
            frames: vec![0, 1, 2, 3],
            fps: 1.0, // 1 frame per second
            ping_pong: false,
        };
        let fast_anim = MaterialAnimation {
            frames: vec![0, 1, 2, 3],
            fps: 10.0, // 10 frames per second
            ping_pong: false,
        };

        let slow_id = MaterialId(1);
        let fast_id = MaterialId(2);
        let mut animator = MaterialAnimator::new(&[
            (slow_id, slow_anim),
            (fast_id, fast_anim),
        ]);

        // After 0.5 seconds:
        animator.tick(0.5);
        // Slow (1fps): still on frame 0 (hasn't hit 1.0s yet)
        assert_eq!(animator.current_frame_index(slow_id), Some(0));
        // Fast (10fps): 0.5 * 10 = 5 frames advanced, 5 % 4 = 1
        assert_eq!(animator.current_frame_index(fast_id), Some(1));
    }

    #[test]
    fn test_non_animated_materials_unaffected() {
        let water_id = MaterialId(1);
        let stone_id = MaterialId(5); // not registered as animated

        let mut animator = MaterialAnimator::new(&[
            (water_id, water_animation()),
        ]);

        animator.tick(1.0);

        // Water has animation state
        assert!(animator.current_frame_index(water_id).is_some());

        // Stone has no animation — returns None
        assert_eq!(animator.current_frame_index(stone_id), None);
        assert_eq!(animator.current_tile(stone_id), None);
    }

    #[test]
    fn test_frame_index_correct_at_each_tick() {
        let water_id = MaterialId(1);
        let anim = MaterialAnimation {
            frames: vec![100, 101, 102],
            fps: 3.0,
            ping_pong: false,
        };
        let mut animator = MaterialAnimator::new(&[(water_id, anim)]);

        // At 3fps, each frame lasts 1/3 second
        let expected_sequence = [
            (0.0,        0, 100),
            (1.0 / 3.0,  1, 101),
            (1.0 / 3.0,  2, 102),
            (1.0 / 3.0,  0, 100), // wraps
            (1.0 / 3.0,  1, 101),
        ];

        for (i, &(dt, expected_frame, expected_tile)) in expected_sequence.iter().enumerate() {
            if dt > 0.0 {
                animator.tick(dt);
            }
            assert_eq!(
                animator.current_frame_index(water_id), Some(expected_frame),
                "Frame index mismatch at step {i}"
            );
            assert_eq!(
                animator.current_tile(water_id), Some(expected_tile),
                "Tile index mismatch at step {i}"
            );
        }
    }

    #[test]
    fn test_ping_pong_reverses_at_end() {
        let lava_id = MaterialId(2);
        let mut animator = MaterialAnimator::new(&[
            (lava_id, lava_animation()), // frames: [20, 21, 22], fps: 2.0, ping_pong: true
        ]);

        // At 2fps, each frame lasts 0.5s
        // Sequence: 0->1->2->1->0->1->2->...
        animator.tick(0.5); // frame 1
        assert_eq!(animator.current_frame_index(lava_id), Some(1));

        animator.tick(0.5); // frame 2 (end, reverse)
        assert_eq!(animator.current_frame_index(lava_id), Some(2));

        animator.tick(0.5); // frame 1 (going backward)
        assert_eq!(animator.current_frame_index(lava_id), Some(1));

        animator.tick(0.5); // frame 0 (start, reverse again)
        assert_eq!(animator.current_frame_index(lava_id), Some(0));

        animator.tick(0.5); // frame 1 (going forward again)
        assert_eq!(animator.current_frame_index(lava_id), Some(1));
    }

    #[test]
    fn test_animation_gpu_data_size() {
        // AnimationGpuData must be 16 bytes (2 floats + 2 padding) for GPU alignment
        assert_eq!(std::mem::size_of::<AnimationGpuData>(), 16);
    }
}
```
