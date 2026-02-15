# FPS Counter

## Problem

During development and playtesting, there is no immediate feedback about how well the engine is performing. Without a visible FPS counter, developers cannot tell whether a visual change caused a regression, whether a particular planet region is GPU-bound, or whether the frame rate is stable versus wildly fluctuating. Frame time spikes that last only a few milliseconds are invisible without instrumentation, but they cause perceptible micro-stutters that degrade the experience. A raw instantaneous FPS value is noisy and hard to read — it needs smoothing. Additionally, knowing the GPU frame time separately from the CPU frame time is critical for diagnosing whether a performance bottleneck is on the rendering or simulation side. Without this overlay, every performance investigation starts with "let me add a print statement," which is slow, error-prone, and pollutes the log output.

## Solution

### Data Collection

Implement a `FrameTimingStats` resource that accumulates frame timing data each frame:

```rust
use std::collections::VecDeque;
use std::time::{Duration, Instant};

pub struct FrameTimingStats {
    /// Ring buffer of the last 60 frame durations for rolling average
    frame_times: VecDeque<Duration>,
    /// All frame times within the last 1-second window for min/max
    recent_second: VecDeque<(Instant, Duration)>,
    /// Last GPU frame time reported by wgpu timestamp queries (if available)
    gpu_frame_time: Option<Duration>,
    /// Whether the overlay is currently visible
    visible: bool,
    /// Maximum number of frames in the rolling average window
    rolling_window_size: usize,
}
```

Each frame, the system pushes the latest frame delta into the ring buffer. When the buffer exceeds 60 entries, the oldest is dropped. The rolling average FPS is computed as:

```rust
impl FrameTimingStats {
    pub fn average_fps(&self) -> f64 {
        if self.frame_times.is_empty() {
            return 0.0;
        }
        let total: Duration = self.frame_times.iter().sum();
        let avg_seconds = total.as_secs_f64() / self.frame_times.len() as f64;
        if avg_seconds > 0.0 { 1.0 / avg_seconds } else { 0.0 }
    }

    pub fn average_frame_time_ms(&self) -> f64 {
        if self.frame_times.is_empty() {
            return 0.0;
        }
        let total: Duration = self.frame_times.iter().sum();
        (total.as_secs_f64() / self.frame_times.len() as f64) * 1000.0
    }

    pub fn min_frame_time_ms(&self) -> Option<f64> {
        self.recent_second
            .iter()
            .map(|(_, d)| d.as_secs_f64() * 1000.0)
            .reduce(f64::min)
    }

    pub fn max_frame_time_ms(&self) -> Option<f64> {
        self.recent_second
            .iter()
            .map(|(_, d)| d.as_secs_f64() * 1000.0)
            .reduce(f64::max)
    }
}
```

The `recent_second` deque stores timestamped frame durations. Each frame, entries older than 1 second are pruned. This provides an accurate 1-second sliding window for min/max without requiring a fixed sample count.

### GPU Timing

If the wgpu adapter supports timestamp queries (`wgpu::Features::TIMESTAMP_QUERY`), the renderer writes timestamps at the beginning and end of the main render pass into a query set. After the frame completes, the resolved timestamps are read back and the difference is stored as `gpu_frame_time`. If timestamp queries are not supported (common on some backends and mobile), the GPU time field displays "N/A" instead of a number.

```rust
pub fn update_gpu_time(&mut self, gpu_duration: Option<Duration>) {
    self.gpu_frame_time = gpu_duration;
}
```

### Toggle

The F3 key toggles `visible`. The system that reads input checks for `KeyCode::F3` in the `just_pressed` state and flips the boolean:

```rust
fn toggle_fps_overlay(
    input: Res<InputState>,
    mut stats: ResMut<FrameTimingStats>,
) {
    if input.just_pressed(KeyCode::F3) {
        stats.visible = !stats.visible;
    }
}
```

### Rendering the Overlay

When `visible` is true, an egui system draws the overlay in the top-left corner using `egui::Area` with a fixed position and a semi-transparent background:

```rust
fn draw_fps_overlay(
    mut egui_ctx: ResMut<EguiContext>,
    stats: Res<FrameTimingStats>,
) {
    if !stats.visible {
        return;
    }

    let fps = stats.average_fps();
    let color = match fps as u32 {
        61..=u32::MAX => egui::Color32::from_rgb(80, 220, 80),   // Green: >60 FPS
        30..=60       => egui::Color32::from_rgb(220, 200, 50),  // Yellow: 30-60 FPS
        _             => egui::Color32::from_rgb(220, 50, 50),   // Red: <30 FPS
    };

    egui::Area::new(egui::Id::new("fps_overlay"))
        .fixed_pos(egui::pos2(8.0, 8.0))
        .show(egui_ctx.get_mut(), |ui| {
            egui::Frame::NONE
                .fill(egui::Color32::from_black_alpha(180))
                .corner_radius(4.0)
                .inner_margin(egui::Margin::same(6))
                .show(ui, |ui| {
                    ui.colored_label(color, format!("FPS: {:.0}", fps));
                    ui.label(format!(
                        "Frame: {:.2} ms",
                        stats.average_frame_time_ms()
                    ));
                    if let (Some(min), Some(max)) = (
                        stats.min_frame_time_ms(),
                        stats.max_frame_time_ms(),
                    ) {
                        ui.label(format!("Min: {:.2} ms  Max: {:.2} ms", min, max));
                    }
                    match stats.gpu_frame_time {
                        Some(gpu) => ui.label(format!(
                            "GPU: {:.2} ms",
                            gpu.as_secs_f64() * 1000.0
                        )),
                        None => ui.label("GPU: N/A"),
                    };
                });
        });
}
```

### Color Thresholds

| FPS Range | Color  | Meaning                                 |
|-----------|--------|-----------------------------------------|
| > 60      | Green  | Healthy performance, above vsync target |
| 30 -- 60  | Yellow | Acceptable but may need optimization    |
| < 30      | Red    | Poor performance, investigate now       |

The thresholds are hardcoded as sensible defaults but could be made configurable through the debug config section.

### System Registration

Both systems (`toggle_fps_overlay` and `draw_fps_overlay`) run in the `PostUpdate` schedule so they have access to the final frame time from the current frame. The `FrameTimingStats` resource is inserted during engine startup with `visible: false` by default (the overlay is hidden until F3 is pressed).

## Outcome

Pressing F3 toggles a compact, semi-transparent overlay in the top-left corner of the screen. The overlay displays: rolling-average FPS (color-coded green/yellow/red), average frame time in milliseconds, min and max frame time over the last second, and GPU frame time if timestamp queries are supported. The overlay updates every frame with smooth values that do not jitter. The implementation lives in `crates/nebula-debug/src/fps_counter.rs` and exports the `FrameTimingStats` resource and two ECS systems.

## Demo Integration

**Demo crate:** `nebula-demo`

A persistent FPS counter in the top-left corner shows current FPS, frame time, and a rolling min/max/average. The counter updates every 0.5 seconds.

## Crates & Dependencies

- **`egui = "0.31"`** — Immediate-mode UI for rendering the overlay text and background panel. Used via the engine's existing egui integration in `nebula-ui`.
- **`wgpu = "28.0"`** — GPU timestamp queries (`Features::TIMESTAMP_QUERY`, `QuerySet`, `QueryType::Timestamp`) for measuring GPU-side frame time. Only used if the feature is supported by the adapter.
- **`tracing = "0.1"`** — Logging the FPS counter toggle events and any diagnostic messages about GPU timestamp support availability.

## Unit Tests

- **`test_fps_calculation_known_frame_times`** — Push exactly 60 frame durations of `Duration::from_millis(16)` (simulating 60 FPS) into `FrameTimingStats`. Assert `average_fps()` returns a value within 0.5 of `62.5` (1000/16). Push 60 durations of `Duration::from_millis(33)` and assert `average_fps()` is within 0.5 of `30.3`.

- **`test_rolling_average_smooths_spikes`** — Push 59 frames of `Duration::from_millis(16)` followed by 1 frame of `Duration::from_millis(100)` (a spike). Assert `average_fps()` is still above 50 FPS, demonstrating that a single spike does not tank the displayed value. Then push 60 frames of `Duration::from_millis(100)` and assert FPS drops below 12.

- **`test_display_toggles`** — Create a `FrameTimingStats` with `visible: false`. Simulate an F3 press by calling the toggle logic. Assert `visible` is now `true`. Simulate another F3 press and assert `visible` is `false` again.

- **`test_color_thresholds`** — Write a helper function `fps_color(fps: f64) -> Color` that returns the color for a given FPS. Assert:
  - `fps_color(120.0)` returns green.
  - `fps_color(60.0)` returns yellow (boundary: 60 is in the 30-60 range).
  - `fps_color(45.0)` returns yellow.
  - `fps_color(29.0)` returns red.
  - `fps_color(1.0)` returns red.

- **`test_initial_state_before_samples`** — Create a fresh `FrameTimingStats` with no frames pushed. Assert `average_fps()` returns `0.0`, `average_frame_time_ms()` returns `0.0`, `min_frame_time_ms()` returns `None`, and `max_frame_time_ms()` returns `None`. This ensures no division-by-zero or panic on empty data.

- **`test_min_max_over_one_second`** — Push frame times of 10ms, 16ms, 20ms, 8ms, and 50ms all with timestamps within the same second. Assert `min_frame_time_ms()` returns approximately `8.0` and `max_frame_time_ms()` returns approximately `50.0`. Then advance the clock past 1 second and push a 16ms frame. Assert the old entries are pruned and min/max reflect only the most recent frame.

- **`test_gpu_time_none_when_unsupported`** — Create `FrameTimingStats` without calling `update_gpu_time`. Assert `gpu_frame_time` is `None`. Call `update_gpu_time(Some(Duration::from_micros(5200)))` and assert `gpu_frame_time` is `Some` with the correct value.
