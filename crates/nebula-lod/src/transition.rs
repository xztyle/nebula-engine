//! LOD pop-in mitigation: transition state machine, crossfade manager, and geometry morphing types.

use nebula_cubesphere::ChunkAddress;
use std::collections::HashMap;

/// The state of a chunk's LOD transition.
#[derive(Clone, Debug)]
pub enum LodTransitionState {
    /// The chunk is stable at its current LOD level. No transition in progress.
    Stable {
        /// Current LOD level.
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

/// Manages LOD transitions for all active chunks.
pub struct LodTransitionManager {
    config: LodTransitionConfig,
    /// Current transition state for each chunk.
    states: HashMap<ChunkAddress, LodTransitionState>,
}

impl LodTransitionManager {
    /// Create a new transition manager with the given configuration.
    pub fn new(config: LodTransitionConfig) -> Self {
        Self {
            config,
            states: HashMap::new(),
        }
    }

    /// Get the transition configuration.
    pub fn config(&self) -> &LodTransitionConfig {
        &self.config
    }

    /// Notify the manager that a chunk's LOD level has changed.
    /// This initiates a transition from the old LOD to the new LOD.
    pub fn on_lod_changed(&mut self, address: ChunkAddress, from_lod: u8, to_lod: u8) {
        self.states.insert(
            address,
            LodTransitionState::Transitioning {
                from_lod,
                to_lod,
                progress: 0.0,
                duration: self.config.crossfade_duration,
            },
        );
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

        completed
    }

    /// Get the current transition state for a chunk.
    pub fn get_state(&self, address: &ChunkAddress) -> Option<&LodTransitionState> {
        self.states.get(address)
    }

    /// Get the alpha values for the old and new LOD meshes during a transition.
    /// Returns `(old_alpha, new_alpha)` where both are in `[0.0, 1.0]`.
    pub fn get_crossfade_alphas(&self, address: &ChunkAddress) -> (f32, f32) {
        match self.states.get(address) {
            Some(LodTransitionState::Transitioning { progress, .. }) => {
                let t = smooth_step(*progress);
                (1.0 - t, t)
            }
            _ => (0.0, 1.0), // Stable: only new mesh is visible
        }
    }

    /// Return the number of currently tracked chunks.
    pub fn active_count(&self) -> usize {
        self.states.len()
    }

    /// Return how many chunks are currently mid-transition.
    pub fn transitioning_count(&self) -> usize {
        self.states
            .values()
            .filter(|s| matches!(s, LodTransitionState::Transitioning { .. }))
            .count()
    }

    /// Remove a chunk from tracking entirely.
    pub fn remove(&mut self, address: &ChunkAddress) {
        self.states.remove(address);
    }
}

/// Hermite smooth step for natural-looking transitions.
pub fn smooth_step(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Vertex data with morph target for LOD transitions.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_cubesphere::CubeFace;

    fn make_address(id: u64) -> ChunkAddress {
        ChunkAddress::new(CubeFace::PosY, 0, id as u32, 0)
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
        assert!(
            completed.is_empty(),
            "transition should not complete in one frame"
        );

        match manager.get_state(&addr) {
            Some(LodTransitionState::Transitioning { progress, .. }) => {
                assert!(
                    *progress > 0.0 && *progress < 1.0,
                    "progress should be between 0 and 1, got {progress}"
                );
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

        assert!(
            old_alpha > 0.0,
            "old mesh should be visible during transition"
        );
        assert!(
            new_alpha > 0.0,
            "new mesh should be visible during transition"
        );
        assert!(
            old_alpha > 0.1,
            "old mesh alpha should be significant at midpoint"
        );
        assert!(
            new_alpha > 0.1,
            "new mesh alpha should be significant at midpoint"
        );
    }

    /// Alpha values should interpolate from (1,0) to (0,1) during the transition.
    #[test]
    fn test_alpha_interpolates_zero_to_one() {
        let mut manager = LodTransitionManager::new(LodTransitionConfig {
            crossfade_duration: 1.0,
            ..Default::default()
        });

        let addr = make_address(1);
        manager.on_lod_changed(addr, 2, 1);

        // At the start, old mesh should be fully visible, new mesh invisible
        let (old_alpha, new_alpha) = manager.get_crossfade_alphas(&addr);
        assert!(
            (old_alpha - 1.0).abs() < 0.01,
            "old alpha should start at ~1.0, got {old_alpha}"
        );
        assert!(
            new_alpha < 0.01,
            "new alpha should start at ~0.0, got {new_alpha}"
        );

        // At the midpoint
        manager.update(0.5);
        let (old_alpha, new_alpha) = manager.get_crossfade_alphas(&addr);
        assert!(
            (old_alpha - 0.5).abs() < 0.15,
            "old alpha should be ~0.5 at midpoint, got {old_alpha}"
        );
        assert!(
            (new_alpha - 0.5).abs() < 0.15,
            "new alpha should be ~0.5 at midpoint, got {new_alpha}"
        );

        // At the end
        manager.update(0.5);
        let _completed = manager.update(0.0);
        let (old_alpha, new_alpha) = manager.get_crossfade_alphas(&addr);
        assert!(
            old_alpha < 0.01,
            "old alpha should be ~0.0 at end, got {old_alpha}"
        );
        assert!(
            (new_alpha - 1.0).abs() < 0.01,
            "new alpha should be ~1.0 at end, got {new_alpha}"
        );
    }

    /// Transition duration should be configurable.
    #[test]
    fn test_transition_duration_configurable() {
        let mut manager_fast = LodTransitionManager::new(LodTransitionConfig {
            crossfade_duration: 0.1,
            ..Default::default()
        });

        let mut manager_slow = LodTransitionManager::new(LodTransitionConfig {
            crossfade_duration: 2.0,
            ..Default::default()
        });

        let addr = make_address(1);
        manager_fast.on_lod_changed(addr, 2, 1);
        manager_slow.on_lod_changed(addr, 2, 1);

        let completed_fast = manager_fast.update(0.15);
        let completed_slow = manager_slow.update(0.15);

        assert!(
            !completed_fast.is_empty(),
            "fast transition should complete in 0.15s"
        );
        assert!(
            completed_slow.is_empty(),
            "slow transition should not complete in 0.15s"
        );
    }

    /// During crossfade, alpha sum should be approximately 1.0.
    #[test]
    fn test_no_visual_artifacts_during_crossfade() {
        let mut manager = LodTransitionManager::new(LodTransitionConfig {
            crossfade_duration: 1.0,
            ..Default::default()
        });

        let addr = make_address(1);
        manager.on_lod_changed(addr, 2, 1);

        for i in 0..20 {
            manager.update(0.05);
            let (old_alpha, new_alpha) = manager.get_crossfade_alphas(&addr);

            let alpha_sum = old_alpha + new_alpha;
            assert!(
                (alpha_sum - 1.0).abs() < 0.15,
                "alpha sum should be ~1.0 at step {i}, got {alpha_sum} (old={old_alpha}, new={new_alpha})"
            );
        }
    }

    /// The `smooth_step` function should produce correct values.
    #[test]
    fn test_smooth_step_correctness() {
        assert!((smooth_step(0.0) - 0.0).abs() < f32::EPSILON);
        assert!((smooth_step(0.5) - 0.5).abs() < f32::EPSILON);
        assert!((smooth_step(1.0) - 1.0).abs() < f32::EPSILON);

        let mut prev = 0.0f32;
        for i in 0..=100 {
            let t = i as f32 / 100.0;
            let v = smooth_step(t);
            assert!(
                v >= prev,
                "smooth_step should be monotonic: {prev} -> {v} at t={t}"
            );
            prev = v;
        }
    }
}
