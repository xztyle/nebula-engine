//! Material animation system: advances frame indices for animated materials
//! and provides GPU-uploadable UV offset data.

use bytemuck::{Pod, Zeroable};
use serde::{Deserialize, Serialize};

use crate::material::MaterialId;

// ---------------------------------------------------------------------------
// MaterialAnimation
// ---------------------------------------------------------------------------

/// Describes an animated material's frame sequence.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MaterialAnimation {
    /// Ordered list of atlas tile indices forming the animation loop.
    pub frames: Vec<u32>,
    /// Playback speed in frames per second.
    pub fps: f32,
    /// Whether the animation ping-pongs (reverses at end) or loops.
    pub ping_pong: bool,
}

// ---------------------------------------------------------------------------
// AnimationGpuData
// ---------------------------------------------------------------------------

/// Per-material animation data uploaded to the GPU each frame.
///
/// The shader adds `uv_offset` to the base UV coordinates when sampling the atlas.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct AnimationGpuData {
    /// UV offset from the base tile to the current animation frame.
    pub uv_offset: [f32; 2],
    /// Padding to maintain 16-byte alignment.
    pub _padding: [f32; 2],
}

// ---------------------------------------------------------------------------
// AnimationState (internal)
// ---------------------------------------------------------------------------

/// Internal per-material animation tracking.
struct AnimationState {
    animation: MaterialAnimation,
    /// Accumulated time in seconds since last frame change.
    elapsed: f32,
    /// Current index into the `frames` array.
    current_frame_index: usize,
    /// Direction for ping-pong: `true` = forward, `false` = backward.
    forward: bool,
}

// ---------------------------------------------------------------------------
// MaterialAnimator
// ---------------------------------------------------------------------------

/// Tracks animation state for all animated materials.
///
/// Non-animated materials are represented as `None` in the internal state array.
/// Call [`MaterialAnimator::tick`] each frame to advance animations.
pub struct MaterialAnimator {
    /// Per-material animation state. Indexed by `MaterialId.0`.
    states: Vec<Option<AnimationState>>,
}

impl MaterialAnimator {
    /// Create the animator from a list of `(MaterialId, MaterialAnimation)` pairs.
    ///
    /// Materials not in the list will have no animation state.
    pub fn new(animations: &[(MaterialId, MaterialAnimation)]) -> Self {
        let max_id = animations
            .iter()
            .map(|(id, _)| id.0 as usize)
            .max()
            .map(|m| m + 1)
            .unwrap_or(0);

        let mut states = Vec::with_capacity(max_id);
        states.resize_with(max_id, || None);

        for (id, anim) in animations {
            let idx = id.0 as usize;
            states[idx] = Some(AnimationState {
                animation: anim.clone(),
                elapsed: 0.0,
                current_frame_index: 0,
                forward: true,
            });
        }

        Self { states }
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
                    } else if state.current_frame_index > 0 {
                        state.current_frame_index -= 1;
                    } else {
                        state.forward = true;
                        if total_frames > 1 {
                            state.current_frame_index = 1;
                        }
                    }
                } else {
                    state.current_frame_index = (state.current_frame_index + 1) % total_frames;
                }
            }
        }
    }

    /// Returns the current atlas tile index for a material.
    ///
    /// For non-animated materials, returns `None` (the base tile is used).
    pub fn current_tile(&self, id: MaterialId) -> Option<u32> {
        self.states
            .get(id.0 as usize)
            .and_then(|s| s.as_ref())
            .map(|s| s.animation.frames[s.current_frame_index])
    }

    /// Returns the current frame index (position in the frames array) for a material.
    pub fn current_frame_index(&self, id: MaterialId) -> Option<usize> {
        self.states
            .get(id.0 as usize)
            .and_then(|s| s.as_ref())
            .map(|s| s.current_frame_index)
    }

    /// Returns the number of material slots tracked by this animator.
    pub fn slot_count(&self) -> usize {
        self.states.len()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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
        let mut animator = MaterialAnimator::new(&[(water_id, water_animation())]);

        assert_eq!(animator.current_frame_index(water_id), Some(0));
        assert_eq!(animator.current_tile(water_id), Some(10));

        animator.tick(0.25);
        assert_eq!(animator.current_frame_index(water_id), Some(1));
        assert_eq!(animator.current_tile(water_id), Some(11));

        animator.tick(0.25);
        assert_eq!(animator.current_frame_index(water_id), Some(2));
        assert_eq!(animator.current_tile(water_id), Some(12));
    }

    #[test]
    fn test_frame_wraps_around_at_end() {
        let water_id = MaterialId(1);
        let mut animator = MaterialAnimator::new(&[(water_id, water_animation())]);

        animator.tick(0.75); // frame 3
        assert_eq!(animator.current_frame_index(water_id), Some(3));
        assert_eq!(animator.current_tile(water_id), Some(13));

        animator.tick(0.25); // wraps to frame 0
        assert_eq!(animator.current_frame_index(water_id), Some(0));
        assert_eq!(animator.current_tile(water_id), Some(10));
    }

    #[test]
    fn test_animation_speed_is_configurable() {
        let slow_anim = MaterialAnimation {
            frames: vec![0, 1, 2, 3],
            fps: 1.0,
            ping_pong: false,
        };
        let fast_anim = MaterialAnimation {
            frames: vec![0, 1, 2, 3],
            fps: 10.0,
            ping_pong: false,
        };

        let slow_id = MaterialId(1);
        let fast_id = MaterialId(2);
        let mut animator = MaterialAnimator::new(&[(slow_id, slow_anim), (fast_id, fast_anim)]);

        animator.tick(0.5);
        // Slow (1fps): still on frame 0
        assert_eq!(animator.current_frame_index(slow_id), Some(0));
        // Fast (10fps): 0.5 * 10 = 5 frames, 5 % 4 = 1
        assert_eq!(animator.current_frame_index(fast_id), Some(1));
    }

    #[test]
    fn test_non_animated_materials_unaffected() {
        let water_id = MaterialId(1);
        let stone_id = MaterialId(5);

        let mut animator = MaterialAnimator::new(&[(water_id, water_animation())]);

        animator.tick(1.0);

        assert!(animator.current_frame_index(water_id).is_some());
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

        let expected_sequence = [
            (0.0, 0, 100),
            (1.0 / 3.0, 1, 101),
            (1.0 / 3.0, 2, 102),
            (1.0 / 3.0, 0, 100),
            (1.0 / 3.0, 1, 101),
        ];

        for (i, &(dt, expected_frame, expected_tile)) in expected_sequence.iter().enumerate() {
            if dt > 0.0 {
                animator.tick(dt);
            }
            assert_eq!(
                animator.current_frame_index(water_id),
                Some(expected_frame),
                "Frame index mismatch at step {i}"
            );
            assert_eq!(
                animator.current_tile(water_id),
                Some(expected_tile),
                "Tile index mismatch at step {i}"
            );
        }
    }

    #[test]
    fn test_ping_pong_reverses_at_end() {
        let lava_id = MaterialId(2);
        let mut animator = MaterialAnimator::new(&[(lava_id, lava_animation())]);

        // At 2fps, each frame lasts 0.5s
        // Sequence: 0->1->2->1->0->1->...
        animator.tick(0.5); // frame 1
        assert_eq!(animator.current_frame_index(lava_id), Some(1));

        animator.tick(0.5); // frame 2 (end, reverse)
        assert_eq!(animator.current_frame_index(lava_id), Some(2));

        animator.tick(0.5); // frame 1 (backward)
        assert_eq!(animator.current_frame_index(lava_id), Some(1));

        animator.tick(0.5); // frame 0 (start, reverse again)
        assert_eq!(animator.current_frame_index(lava_id), Some(0));

        animator.tick(0.5); // frame 1 (forward again)
        assert_eq!(animator.current_frame_index(lava_id), Some(1));
    }

    #[test]
    fn test_animation_gpu_data_size() {
        assert_eq!(std::mem::size_of::<AnimationGpuData>(), 16);
    }
}
