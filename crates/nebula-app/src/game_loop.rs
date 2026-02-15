//! Fixed-timestep game loop implementing the "Fix Your Timestep" pattern.
//!
//! Decouples simulation (fixed 60 Hz) from rendering (variable rate) using an
//! accumulator. Provides interpolation alpha for smooth rendering between
//! simulation states.

use std::time::Instant;
use tracing::warn;

/// Fixed simulation timestep: 60 Hz (16.666… ms per tick).
pub const FIXED_DT: f64 = 1.0 / 60.0;

/// Maximum frame time clamp to prevent spiral of death.
/// If a frame takes longer than this, we clamp and accept slowdown
/// rather than trying to catch up with dozens of simulation steps.
pub const MAX_FRAME_TIME: f64 = 0.25; // 250ms = 4 FPS minimum

/// Fixed-timestep game loop state.
///
/// Call [`tick`](Self::tick) once per frame to run simulation steps at a fixed
/// rate and render with interpolation.
pub struct GameLoop {
    previous_time: Instant,
    accumulator: f64,
    total_sim_time: f64,
    frame_count: u64,
    update_count: u64,
}

impl GameLoop {
    /// Creates a new `GameLoop` starting from the current instant.
    pub fn new() -> Self {
        Self {
            previous_time: Instant::now(),
            accumulator: 0.0,
            total_sim_time: 0.0,
            frame_count: 0,
            update_count: 0,
        }
    }

    /// Runs one frame: measures elapsed time, runs fixed-rate simulation steps,
    /// then calls the render function with interpolation alpha.
    ///
    /// - `update_fn(fixed_dt, total_sim_time)` is called zero or more times at
    ///   the fixed rate.
    /// - `render_fn(alpha)` is called exactly once with the interpolation alpha
    ///   in `[0.0, 1.0)`.
    pub fn tick(&mut self, mut update_fn: impl FnMut(f64, f64), mut render_fn: impl FnMut(f64)) {
        let current_time = Instant::now();
        let mut frame_time = current_time
            .duration_since(self.previous_time)
            .as_secs_f64();
        self.previous_time = current_time;

        // Clamp frame time to prevent spiral of death
        if frame_time > MAX_FRAME_TIME {
            warn!(
                "Frame time {:.1}ms exceeds maximum, clamping to {:.1}ms",
                frame_time * 1000.0,
                MAX_FRAME_TIME * 1000.0
            );
            frame_time = MAX_FRAME_TIME;
        }

        self.accumulator += frame_time;

        // Run simulation steps at fixed rate
        while self.accumulator >= FIXED_DT {
            update_fn(FIXED_DT, self.total_sim_time);
            self.total_sim_time += FIXED_DT;
            self.accumulator -= FIXED_DT;
            self.update_count += 1;
        }

        // Calculate interpolation alpha for smooth rendering
        let alpha = if self.accumulator > 0.0 {
            self.accumulator / FIXED_DT
        } else {
            0.0
        };

        render_fn(alpha);
        self.frame_count += 1;
    }

    /// Returns the current interpolation alpha without running a tick.
    pub fn alpha(&self) -> f64 {
        if self.accumulator > 0.0 {
            self.accumulator / FIXED_DT
        } else {
            0.0
        }
    }

    /// Returns the total number of frames rendered.
    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }

    /// Returns the total number of simulation update steps executed.
    pub fn update_count(&self) -> u64 {
        self.update_count
    }

    /// Returns the total simulation time in seconds.
    pub fn total_sim_time(&self) -> f64 {
        self.total_sim_time
    }
}

impl Default for GameLoop {
    fn default() -> Self {
        Self::new()
    }
}

/// A testable version of the game loop that accepts explicit frame times
/// instead of measuring wall-clock time.
#[cfg(test)]
struct TestableGameLoop {
    accumulator: f64,
    total_sim_time: f64,
    frame_count: u64,
    update_count: u64,
}

#[cfg(test)]
impl TestableGameLoop {
    fn new() -> Self {
        Self {
            accumulator: 0.0,
            total_sim_time: 0.0,
            frame_count: 0,
            update_count: 0,
        }
    }

    /// Tick with an explicit frame time (in seconds).
    fn tick(
        &mut self,
        frame_time: f64,
        mut update_fn: impl FnMut(f64, f64),
        mut render_fn: impl FnMut(f64),
    ) {
        let clamped = if frame_time > MAX_FRAME_TIME {
            MAX_FRAME_TIME
        } else {
            frame_time
        };

        self.accumulator += clamped;

        while self.accumulator >= FIXED_DT {
            update_fn(FIXED_DT, self.total_sim_time);
            self.total_sim_time += FIXED_DT;
            self.accumulator -= FIXED_DT;
            self.update_count += 1;
        }

        let alpha = if self.accumulator > 0.0 {
            self.accumulator / FIXED_DT
        } else {
            0.0
        };

        render_fn(alpha);
        self.frame_count += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fixed_dt_value() {
        assert!(
            (FIXED_DT - 1.0 / 60.0).abs() < f64::EPSILON * 10.0,
            "FIXED_DT should equal 1/60"
        );
    }

    #[test]
    fn test_accumulator_single_step() {
        let mut loop_ = TestableGameLoop::new();
        let mut updates = 0u32;
        loop_.tick(FIXED_DT, |_, _| updates += 1, |_| {});
        assert_eq!(updates, 1);
        assert!(loop_.accumulator.abs() < 1e-12);
    }

    #[test]
    fn test_accumulator_multiple_steps() {
        let mut loop_ = TestableGameLoop::new();
        let mut updates = 0u32;
        let mut last_sim_time = 0.0;
        loop_.tick(
            3.0 * FIXED_DT,
            |_, sim_time| {
                updates += 1;
                last_sim_time = sim_time + FIXED_DT;
            },
            |_| {},
        );
        assert_eq!(updates, 3);
        assert!((loop_.total_sim_time - 3.0 * FIXED_DT).abs() < 1e-12);
    }

    #[test]
    fn test_accumulator_partial() {
        let mut loop_ = TestableGameLoop::new();
        let mut updates = 0u32;
        let mut render_called = false;
        loop_.tick(
            0.5 * FIXED_DT,
            |_, _| updates += 1,
            |_| render_called = true,
        );
        assert_eq!(updates, 0);
        assert!(render_called);
        assert!((loop_.accumulator - 0.5 * FIXED_DT).abs() < 1e-12);
    }

    #[test]
    fn test_interpolation_alpha() {
        let mut loop_ = TestableGameLoop::new();
        let mut alpha_received = 0.0;
        loop_.tick(0.25 * FIXED_DT, |_, _| {}, |a| alpha_received = a);
        assert!(
            (alpha_received - 0.25).abs() < 1e-10,
            "alpha should be ~0.25, got {alpha_received}"
        );
        assert!((0.0..1.0).contains(&alpha_received));
    }

    #[test]
    fn test_max_frame_time_clamp() {
        let mut loop_ = TestableGameLoop::new();
        let mut updates = 0u32;
        // 1.0 second frame time, should be clamped to MAX_FRAME_TIME
        loop_.tick(1.0, |_, _| updates += 1, |_| {});
        let max_updates = (MAX_FRAME_TIME / FIXED_DT).ceil() as u32;
        assert!(
            updates <= max_updates,
            "Expected at most {max_updates} updates, got {updates}"
        );
        assert!(updates > 0);
    }

    #[test]
    fn test_total_sim_time_advances() {
        let mut loop_ = TestableGameLoop::new();
        for _ in 0..10 {
            loop_.tick(FIXED_DT * 2.0, |_, _| {}, |_| {});
        }
        let expected = loop_.update_count as f64 * FIXED_DT;
        assert!(
            (loop_.total_sim_time - expected).abs() < 1e-10,
            "total_sim_time {} != expected {}",
            loop_.total_sim_time,
            expected
        );
    }

    #[test]
    fn test_frame_count_increments() {
        let mut loop_ = TestableGameLoop::new();
        for _ in 0..10 {
            loop_.tick(FIXED_DT, |_, _| {}, |_| {});
        }
        assert_eq!(loop_.frame_count, 10);
    }

    #[test]
    fn test_zero_frame_time() {
        let mut loop_ = TestableGameLoop::new();
        let mut updates = 0u32;
        let mut alpha_received = 0.0;
        loop_.tick(0.0, |_, _| updates += 1, |a| alpha_received = a);
        assert_eq!(updates, 0);
        assert!((alpha_received - 0.0).abs() < 1e-12);
    }

    #[test]
    fn test_deterministic_sequence() {
        let frame_times = [0.017, 0.015, 0.020, 0.016, 0.033, 0.008, 0.018];

        let mut loop_a = TestableGameLoop::new();
        let mut loop_b = TestableGameLoop::new();

        for &ft in &frame_times {
            let mut alpha_a = 0.0;
            let mut alpha_b = 0.0;
            loop_a.tick(ft, |_, _| {}, |a| alpha_a = a);
            loop_b.tick(ft, |_, _| {}, |a| alpha_b = a);
            assert!(
                (alpha_a - alpha_b).abs() < 1e-15,
                "Alphas diverged: {alpha_a} vs {alpha_b}"
            );
        }

        assert_eq!(loop_a.update_count, loop_b.update_count);
        assert!((loop_a.total_sim_time - loop_b.total_sim_time).abs() < 1e-15);
        assert_eq!(loop_a.frame_count, loop_b.frame_count);
    }

    #[test]
    fn test_game_loop_default() {
        let loop_ = GameLoop::default();
        assert_eq!(loop_.frame_count(), 0);
        assert_eq!(loop_.update_count(), 0);
        assert!((loop_.total_sim_time() - 0.0).abs() < f64::EPSILON);
    }
}
