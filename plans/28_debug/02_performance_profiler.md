# Performance Profiler

## Problem

When the frame rate drops, the immediate question is "where is the time being spent?" Without a hierarchical profiler, the answer requires guesswork, manual `Instant::now()` instrumentation scattered throughout the codebase, or attaching an external tool like Tracy or perf. None of these are convenient during live development:

- **Manual timing** requires adding and removing boilerplate everywhere, and it does not show parent-child relationships (e.g., "terrain generation" contains "noise evaluation" and "voxel writing").
- **External profilers** require a separate application, special build flags, and context-switching away from the game window. They also do not integrate with the engine's own UI.
- **Flat timing lists** (system A took 2ms, system B took 3ms) miss the hierarchical structure. If "render" takes 8ms, you need to know that 5ms was in "shadow pass" and 3ms was in "main pass," and within shadow pass, 4ms was "terrain shadows."

The engine already uses `tracing` for structured logging (story `01_setup/08_logging_and_tracing.md`). The `tracing` spans naturally form a hierarchy. A custom `tracing` subscriber layer can capture span enter/exit timings and present them as a frame profile without any additional instrumentation beyond the `#[instrument]` attributes already in place.

## Solution

### Architecture

The profiler consists of three components:

1. **`ProfilingLayer`** — A custom `tracing_subscriber::Layer` that records span timings into a per-frame buffer.
2. **`ProfileData`** resource — Stores the completed frame profile for display.
3. **`ProfilerWindow`** — An egui window that renders the profile as a flame graph or hierarchical table.

### The Profiling Layer

Implement a `tracing_subscriber::Layer` that hooks into span lifecycle events:

```rust
use tracing::{Id, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use std::sync::{Arc, Mutex};
use std::time::Instant;

struct SpanTiming {
    name: String,
    target: String,
    enter_time: Instant,
    exit_time: Option<Instant>,
    parent_id: Option<Id>,
    depth: u32,
}

pub struct ProfilingLayer {
    current_frame: Arc<Mutex<Vec<SpanTiming>>>,
    enabled: Arc<AtomicBool>,
}

impl<S: Subscriber> Layer<S> for ProfilingLayer {
    fn on_enter(&self, id: &Id, ctx: Context<'_, S>) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        // Record span name, target, enter time, and parent span ID
    }

    fn on_exit(&self, id: &Id, ctx: Context<'_, S>) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        // Record exit time for the span
    }
}
```

At the end of each frame (in a dedicated `flush_profiler` system), the current frame buffer is swapped out and handed to `ProfileData` for display. This double-buffering approach ensures the UI always reads a complete frame's data while the next frame is being recorded.

### Profile Data Structure

The completed frame profile is stored as a tree:

```rust
pub struct ProfileNode {
    pub name: String,
    pub target: String,
    pub duration: Duration,
    pub percentage: f32,       // Percentage of total frame time
    pub children: Vec<ProfileNode>,
    pub over_budget: bool,     // True if duration exceeds the per-system budget
}

pub struct ProfileData {
    pub root_nodes: Vec<ProfileNode>,
    pub total_frame_time: Duration,
    pub frame_budget: Duration, // Typically 16.67ms for 60 FPS
    pub enabled: bool,
    pub history: VecDeque<Vec<ProfileNode>>, // Last N frames for timeline view
}
```

The tree is constructed from the flat span list by matching parent IDs. Each node's `percentage` is computed as `(node.duration / total_frame_time) * 100.0`. A node is `over_budget` if its duration exceeds a configurable threshold (default: a proportional share of the frame budget based on the system's historical average).

### Budget Highlighting

Systems are highlighted in red when they exceed their expected budget. The budget for each system is determined by:

1. **Historical average** — The profiler tracks a rolling average duration for each named span across frames. If a span takes more than 2x its average, it is flagged.
2. **Absolute threshold** — Any individual span exceeding 4ms is flagged regardless of history, since a single system consuming 25% of a 16.67ms frame budget is concerning.

### Flame Graph Rendering

The egui window renders the profile as a horizontal flame graph:

```rust
fn draw_profiler_window(
    mut egui_ctx: ResMut<EguiContext>,
    profile: Res<ProfileData>,
) {
    if !profile.enabled {
        return;
    }

    egui::Window::new("Performance Profiler")
        .default_size([600.0, 400.0])
        .show(egui_ctx.get_mut(), |ui| {
            // Frame time header
            ui.label(format!(
                "Frame: {:.2} ms ({:.0} FPS) | Budget: {:.2} ms",
                profile.total_frame_time.as_secs_f64() * 1000.0,
                1.0 / profile.total_frame_time.as_secs_f64(),
                profile.frame_budget.as_secs_f64() * 1000.0,
            ));

            ui.separator();

            // Flame graph: each row is a depth level, each bar is a span
            let available_width = ui.available_width();
            let row_height = 20.0;
            let frame_ms = profile.total_frame_time.as_secs_f64() * 1000.0;

            for node in &profile.root_nodes {
                draw_flame_node(ui, node, available_width, row_height, frame_ms, 0);
            }
        });
}

fn draw_flame_node(
    ui: &mut egui::Ui,
    node: &ProfileNode,
    total_width: f32,
    row_height: f32,
    frame_ms: f64,
    depth: u32,
) {
    let width = (node.duration.as_secs_f64() * 1000.0 / frame_ms) as f32 * total_width;
    let color = if node.over_budget {
        egui::Color32::from_rgb(220, 60, 60)
    } else {
        depth_color(depth)
    };

    // Draw bar with label
    let (rect, _response) = ui.allocate_exact_size(
        egui::vec2(width.max(2.0), row_height),
        egui::Sense::hover(),
    );
    ui.painter().rect_filled(rect, 2.0, color);
    ui.painter().text(
        rect.left_center() + egui::vec2(4.0, 0.0),
        egui::Align2::LEFT_CENTER,
        format!("{} ({:.2}ms)", node.name, node.duration.as_secs_f64() * 1000.0),
        egui::FontId::proportional(11.0),
        egui::Color32::WHITE,
    );

    // Recurse into children
    for child in &node.children {
        draw_flame_node(ui, child, total_width, row_height, frame_ms, depth + 1);
    }
}
```

### Data Export

Profile data can be saved to a JSON file for offline analysis:

```rust
impl ProfileData {
    pub fn export_to_file(&self, path: &Path) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(&self.history)?;
        std::fs::write(path, json)
    }
}
```

The exported format is compatible with Chrome's `chrome://tracing` viewer and Perfetto, using the Trace Event Format:

```json
[
    {"name": "render", "cat": "nebula_render", "ph": "X", "ts": 0, "dur": 8200, "pid": 1, "tid": 1},
    {"name": "shadow_pass", "cat": "nebula_render", "ph": "X", "ts": 100, "dur": 4800, "pid": 1, "tid": 1}
]
```

### Enable/Disable

The profiler can be toggled via a debug key (F5) or programmatically. When disabled, the `ProfilingLayer` short-circuits on the `enabled` atomic bool, adding near-zero overhead. The profiler window is also hidden when disabled.

## Outcome

An egui window displays a hierarchical flame graph of per-frame system timings, built from existing `tracing` `#[instrument]` spans. Each bar shows the system name, duration in milliseconds, and percentage of frame budget. Systems exceeding their budget are highlighted in red. The profiler can be enabled/disabled with F5 and exports data to a JSON file compatible with Chrome Tracing / Perfetto for offline analysis. The implementation lives in `crates/nebula-debug/src/profiler.rs` and consists of a `ProfilingLayer`, a `ProfileData` resource, and egui rendering systems.

## Demo Integration

**Demo crate:** `nebula-demo`

A flame graph overlay shows per-system frame time as colored bars. The developer can see which ECS system is the bottleneck at a glance.

## Crates & Dependencies

- **`tracing = "0.1"`** — The span lifecycle hooks (`on_enter`, `on_exit`, `on_new_span`) that the profiling layer subscribes to. All instrumented code already emits these spans via `#[instrument]`.
- **`tracing-subscriber = { version = "0.3", features = ["registry"] }`** — The `Layer` trait and `Registry` subscriber that the `ProfilingLayer` integrates with. The registry feature enables per-span data storage.
- **`egui = "0.31"`** — Rendering the flame graph window, bars, labels, and hover tooltips.
- **`serde = { version = "1", features = ["derive"] }`** — Serialization of `ProfileNode` and `ProfileData` for the JSON export.
- **`serde_json = "1"`** — JSON serialization for the exported profile data, using the Chrome Trace Event Format.

## Unit Tests

- **`test_profiler_records_system_durations`** — Create a `ProfilingLayer`, enter a span named "test_system", sleep for 5ms, exit the span, then flush the frame. Assert the resulting `ProfileData` contains one root node named "test_system" with a duration between 4ms and 8ms (allowing for sleep imprecision).

- **`test_hierarchy_parent_contains_children`** — Enter a span "parent", then enter a child span "child_a", exit "child_a", enter "child_b", exit "child_b", exit "parent". Flush and assert: "parent" has two children ("child_a" and "child_b"), and the parent's duration is greater than or equal to the sum of its children's durations.

- **`test_percentages_sum_to_approximately_100`** — Record three root-level spans with known durations (5ms, 8ms, 3ms). Flush and assert the percentages of the root nodes sum to a value between 99.0 and 101.0 (accounting for floating-point rounding).

- **`test_profiler_enable_disable`** — Create a `ProfilingLayer` with `enabled = false`. Enter and exit a span. Flush and assert the `ProfileData` has zero root nodes (nothing was recorded). Set `enabled = true`, repeat, and assert the span appears.

- **`test_data_exports_correctly`** — Build a `ProfileData` with known values. Call `export_to_file` to a temporary path. Read the file back, parse as JSON, and verify the structure contains the expected span names and durations. Verify the format is valid Chrome Trace Event Format with `ph`, `ts`, `dur`, `name`, and `cat` fields.

- **`test_over_budget_flagging`** — Create a `ProfileData` with a frame budget of 16.67ms. Insert a node with duration 5ms (within budget) and another with duration 18ms (over budget). Assert the first node has `over_budget == false` and the second has `over_budget == true`.

- **`test_double_buffer_swap`** — Record spans in frame N, flush, start recording frame N+1. Assert that the display data (frame N) remains stable and complete while frame N+1 is being recorded. Verify no data races or partial frames appear in the display buffer.
