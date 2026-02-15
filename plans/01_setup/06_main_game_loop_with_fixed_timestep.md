# Main Game Loop with Fixed Timestep

## Problem

Games need deterministic physics and simulation, which requires a fixed timestep update loop decoupled from variable-rate rendering. Without a fixed timestep:

- **Physics instability** — Variable delta times cause physics engines to produce different results on different hardware. A player jumping on a 144 Hz machine gets different airtime than on a 30 Hz machine. Collision detection can miss fast-moving objects at low frame rates (tunneling).
- **Non-deterministic multiplayer** — Nebula Engine's TCP multiplayer requires that the server and all clients produce identical simulation results from the same inputs. Variable timesteps make this impossible. With fixed timesteps, the server can replay client inputs deterministically.
- **Accumulating floating-point error** — Variable time steps accumulate different floating-point rounding errors over time, causing simulations to diverge. Fixed timesteps ensure every run follows the exact same numerical path.
- **Speed tied to frame rate** — Naive `update(dt)` approaches cause the game to speed up on fast machines and slow down on slow machines if any code accidentally uses frame count instead of time.

The industry-standard solution is the "Fix Your Timestep" pattern described by Glenn Fiedler, which decouples simulation from rendering using an accumulator.

## Solution

### Constants

```rust
/// Fixed simulation timestep: 60 Hz (16.666... ms per tick)
pub const FIXED_DT: f64 = 1.0 / 60.0;

/// Maximum frame time clamp to prevent spiral of death.
/// If a frame takes longer than this, we clamp and accept slowdown
/// rather than trying to catch up with dozens of simulation steps.
pub const MAX_FRAME_TIME: f64 = 0.25; // 250ms = 4 FPS minimum
```

### GameLoop Struct

```rust
use std::time::Instant;

pub struct GameLoop {
    previous_time: Instant,
    accumulator: f64,
    total_sim_time: f64,
    frame_count: u64,
    update_count: u64,
}

impl GameLoop {
    pub fn new() -> Self {
        Self {
            previous_time: Instant::now(),
            accumulator: 0.0,
            total_sim_time: 0.0,
            frame_count: 0,
            update_count: 0,
        }
    }

    /// Call this once per frame. Returns the interpolation alpha
    /// for rendering between the previous and current simulation states.
    pub fn tick(
        &mut self,
        mut update_fn: impl FnMut(f64, f64),  // (fixed_dt, total_sim_time)
        mut render_fn: impl FnMut(f64),         // (interpolation_alpha)
    ) {
        let current_time = Instant::now();
        let mut frame_time = current_time
            .duration_since(self.previous_time)
            .as_secs_f64();
        self.previous_time = current_time;

        // Clamp frame time to prevent spiral of death
        if frame_time > MAX_FRAME_TIME {
            log::warn!(
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
        let alpha = self.accumulator / FIXED_DT;
        debug_assert!(
            (0.0..1.0).contains(&alpha),
            "Interpolation alpha {} out of range [0, 1)",
            alpha
        );

        render_fn(alpha);
        self.frame_count += 1;
    }

    /// Returns the interpolation alpha without running a tick.
    /// Useful for querying the current interpolation state.
    pub fn alpha(&self) -> f64 {
        self.accumulator / FIXED_DT
    }

    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }

    pub fn update_count(&self) -> u64 {
        self.update_count
    }

    pub fn total_sim_time(&self) -> f64 {
        self.total_sim_time
    }
}
```

### How Interpolation Works

The simulation state is always at a discrete point in time (a multiple of `FIXED_DT`). But the renderer runs at arbitrary times between simulation steps. The `alpha` value (0.0 to 1.0) tells the renderer how far between the previous and current simulation states the current frame falls.

For example, if the simulation has states at t=0.0000 and t=0.0167 (one tick), and the renderer runs at t=0.0100, then:
- `alpha = 0.0100 / 0.0167 = 0.599`
- The renderer interpolates 59.9% of the way between the two states

This requires storing both the previous and current state for any interpolated quantities (position, rotation). The rendering system will use:

```rust
fn interpolated_position(prev: Vec3, curr: Vec3, alpha: f64) -> Vec3 {
    prev + (curr - prev) * alpha as f32
}
```

### Spiral of Death Prevention

If the simulation falls behind (each frame takes longer than real time), the accumulator grows without bound, causing more and more simulation steps per frame, which makes each frame take even longer. This is the "spiral of death." The `MAX_FRAME_TIME` clamp prevents this by accepting that the simulation will run slower than real time rather than trying to catch up. This is the correct behavior for a game: it is better to run in slow motion than to freeze.

### Integration with Winit Event Loop

The game loop integrates with winit's `RedrawRequested` event:

```rust
WindowEvent::RedrawRequested => {
    game_loop.tick(
        |dt, sim_time| {
            // Fixed-rate simulation step
            ecs_world.run_schedule(FixedUpdate);
        },
        |alpha| {
            // Variable-rate rendering
            renderer.render(alpha);
        },
    );
    window.request_redraw(); // Request next frame
}
```

### Integration with ECS

The fixed timestep naturally maps to Bevy ECS schedules:
- **`FixedUpdate`** — Runs inside the `update_fn`, always with `FIXED_DT`. Physics, input processing, game logic, and network tick processing go here.
- **`Update`** — Runs once per frame (outside the fixed loop) for things that should happen every frame regardless of simulation rate (e.g., UI updates, camera smoothing).
- **`Render`** — Runs in `render_fn` with the interpolation alpha.

### Frame Timing Statistics

For profiling and the debug overlay:

```rust
pub struct FrameStats {
    pub frame_time_ms: f64,
    pub updates_this_frame: u32,
    pub alpha: f64,
    pub fps: f64,        // Rolling average
    pub ups: f64,        // Updates per second (should be ~60)
}
```

## Outcome

The simulation runs at exactly 60 steps per second regardless of frame rate. On a 144 Hz display, approximately 2-3 frames are rendered per simulation step, with smooth interpolation between states. On a 30 Hz display, approximately 2 simulation steps run per frame. Physics and game logic are deterministic and reproducible. Frame timing statistics are available for the debug overlay. The spiral of death is prevented by clamping frame time to 250ms.

## Demo Integration

**Demo crate:** `nebula-demo`

The demo ticks at a steady 60Hz simulation rate decoupled from the render frame rate. The clear color subtly pulses between dark blue and black in sync with the tick counter -- proof the simulation loop is alive.

## Crates & Dependencies

- No new external crates -- uses `std::time` for `Instant` and `Duration`.
- The `log` crate (already a dependency from `04_spawn_window.md`) is used for the frame time warning.

## Unit Tests

- **`test_fixed_dt_value`** — Verify `FIXED_DT` equals `1.0 / 60.0` (approximately 0.01666... seconds). Assert that `(FIXED_DT - 1.0/60.0).abs() < f64::EPSILON * 10.0`.

- **`test_accumulator_single_step`** — Create a `GameLoop`, manually set the accumulator to exactly `FIXED_DT`, call a tick-like function, and expect exactly 1 `update_fn` call. Verify the accumulator is approximately 0 afterward.

- **`test_accumulator_multiple_steps`** — Set the accumulator to `3.0 * FIXED_DT`, tick, and expect exactly 3 `update_fn` calls. Verify the total simulation time advanced by `3.0 * FIXED_DT`.

- **`test_accumulator_partial`** — Set the accumulator to `0.5 * FIXED_DT`, tick, and expect 0 `update_fn` calls. Verify the accumulator retains the remainder of `0.5 * FIXED_DT` and `render_fn` is still called once.

- **`test_interpolation_alpha`** — After a tick with a partial accumulator, verify the alpha value passed to `render_fn` is in the range `[0.0, 1.0)`. Test specific values: accumulator of `0.25 * FIXED_DT` should yield alpha approximately 0.25.

- **`test_max_frame_time_clamp`** — Set frame time to 1.0 second (well above `MAX_FRAME_TIME`). Verify that the number of `update_fn` calls is at most `ceil(MAX_FRAME_TIME / FIXED_DT)` = 15 calls, not 60 calls.

- **`test_total_sim_time_advances`** — Run several ticks and verify `total_sim_time` matches `update_count * FIXED_DT` within floating-point tolerance.

- **`test_frame_count_increments`** — Call `tick()` 10 times and verify `frame_count()` returns 10. Each call to `tick()` is one frame regardless of how many simulation steps occur.

- **`test_zero_frame_time`** — If two ticks happen at the same `Instant` (frame time = 0), verify that no `update_fn` calls occur and `render_fn` is called once with alpha = 0.0.

- **`test_deterministic_sequence`** — Feed the exact same sequence of frame times to two independent `GameLoop` instances and verify they produce identical `update_count`, `total_sim_time`, and alpha values. This validates determinism.
