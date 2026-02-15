# Structured Logging

## Problem

The basic tracing setup from Epic 01 (`plans/01_setup/08_logging_and_tracing.md`) provides filtered, leveled logging with timestamps and module paths. This is sufficient for simple debugging, but a production game engine needs more:

- **Missing context** -- A log line "chunk meshed in 4.2ms" is useful. But without knowing which chunk, on which tick, for which player's view, during which frame, the information is nearly useless when diagnosing a performance regression across thousands of similar lines.
- **No correlation** -- When multiple async operations run concurrently (meshing chunks, loading assets, processing network messages), interleaved log lines from different operations are indistinguishable. There is no way to follow a single operation from start to finish.
- **No machine-parseable metrics** -- Human-readable log lines like "Frame took 18.3ms" cannot be easily aggregated, graphed, or alerted on. Extracting structured data from free-text messages requires brittle regex parsing.
- **No hierarchical context** -- The engine has a natural hierarchy: frame > tick > system > operation. A log event during meshing should automatically inherit the frame number and tick number without every call site manually including them.

The tracing ecosystem supports all of this through structured fields and span hierarchies. This story enhances the existing tracing setup to use these capabilities systematically.

## Solution

### Structured Fields Convention

Every log event in the engine should include relevant structured fields. Define a standard set of field names used consistently across all crates:

| Field | Type | Usage |
|-------|------|-------|
| `subsystem` | `&str` | Which engine subsystem: `"render"`, `"voxel"`, `"net"`, `"physics"`, `"audio"`, `"ecs"`, `"assets"` |
| `tick` | `u64` | The current simulation tick number |
| `frame` | `u64` | The current render frame number |
| `entity_id` | `u64` | The entity being operated on (when applicable) |
| `chunk_pos` | `[i32; 3]` | The chunk being operated on (when applicable) |
| `duration_ms` | `f64` | Elapsed time for a timed operation |
| `count` | `usize` | A count of items (vertices, entities, packets, etc.) |
| `size_bytes` | `usize` | Size of data (buffers, messages, assets) |

### LogContext for Hierarchical Context

Create a `LogContext` that pushes context onto the current tracing span, so nested operations inherit outer context automatically:

```rust
use tracing::{span, Level, Span};

/// Represents the current engine context for logging.
/// Systems push context at the beginning of their execution
/// and pop it at the end (via Drop).
pub struct LogContext {
    _span_guard: tracing::span::EnteredSpan,
}

impl LogContext {
    /// Create a new context for a frame.
    pub fn frame(frame_number: u64) -> Self {
        let span = span!(Level::INFO, "frame", frame = frame_number);
        Self { _span_guard: span.entered() }
    }

    /// Create a new context for a simulation tick within a frame.
    pub fn tick(tick_number: u64) -> Self {
        let span = span!(Level::INFO, "tick", tick = tick_number);
        Self { _span_guard: span.entered() }
    }

    /// Create a new context for a subsystem within a tick.
    pub fn subsystem(name: &str) -> Self {
        let span = span!(Level::DEBUG, "subsystem", subsystem = name);
        Self { _span_guard: span.entered() }
    }

    /// Create a new context for an entity operation.
    pub fn entity(entity_id: u64) -> Self {
        let span = span!(Level::TRACE, "entity", entity_id = entity_id);
        Self { _span_guard: span.entered() }
    }
}
```

The `LogContext` is used in the game loop to establish hierarchical context:

```rust
pub fn run_frame(&mut self) {
    let _frame_ctx = LogContext::frame(self.frame_count);

    // Fixed timestep ticks within the frame
    while self.accumulator >= TICK_DURATION {
        let _tick_ctx = LogContext::tick(self.tick_count);

        {
            let _physics_ctx = LogContext::subsystem("physics");
            self.run_physics();
            // Any log event inside run_physics() automatically has:
            // frame=42, tick=1337, subsystem="physics"
        }

        {
            let _voxel_ctx = LogContext::subsystem("voxel");
            self.run_voxel_systems();
        }

        self.tick_count += 1;
        self.accumulator -= TICK_DURATION;
    }

    {
        let _render_ctx = LogContext::subsystem("render");
        self.render();
    }

    self.frame_count += 1;
}
```

### Instrumented Functions with `#[instrument]`

Expand the use of `#[instrument]` beyond the basic setup in Epic 01. Key functions across the engine are annotated with structured fields:

```rust
use tracing::instrument;

#[instrument(
    name = "mesh_chunk",
    skip(chunk_data),
    fields(
        subsystem = "voxel",
        chunk_pos = ?pos,
        vertex_count = tracing::field::Empty,
        duration_ms = tracing::field::Empty,
    )
)]
pub fn mesh_chunk(pos: ChunkPos, chunk_data: &ChunkData) -> Mesh {
    let start = std::time::Instant::now();

    let mesh = greedy_mesh(chunk_data);

    // Record the values that were not known at function entry
    let span = tracing::Span::current();
    span.record("vertex_count", mesh.vertex_count());
    span.record("duration_ms", start.elapsed().as_secs_f64() * 1000.0);

    mesh
}
```

This produces structured output like:

```json
{
    "timestamp": "12.345s",
    "level": "INFO",
    "span": {
        "name": "mesh_chunk",
        "subsystem": "voxel",
        "chunk_pos": "[4, 2, -1]",
        "vertex_count": 12847,
        "duration_ms": 4.23
    },
    "target": "nebula_mesh",
    "message": "close"
}
```

### Timed Operations Helper

A utility for timing operations and logging the duration as a structured field:

```rust
pub struct TimedOperation {
    name: &'static str,
    start: std::time::Instant,
    span: tracing::Span,
}

impl TimedOperation {
    pub fn start(name: &'static str) -> Self {
        let span = tracing::span!(tracing::Level::DEBUG, "timed_op", op = name, duration_ms = tracing::field::Empty);
        Self {
            name,
            start: std::time::Instant::now(),
            span,
        }
    }
}

impl Drop for TimedOperation {
    fn drop(&mut self) {
        let duration_ms = self.start.elapsed().as_secs_f64() * 1000.0;
        self.span.record("duration_ms", duration_ms);
        tracing::debug!(
            parent: &self.span,
            op = self.name,
            duration_ms = duration_ms,
            "Operation complete"
        );
    }
}
```

Usage:

```rust
fn load_world() {
    let _timer = TimedOperation::start("world_load");
    // ... loading logic ...
    // On drop, logs: "Operation complete" with duration_ms field
}
```

### Dual Output Format

The subscriber configuration from Epic 01 is extended to support simultaneous dual-format output:

- **Console output**: Human-readable with colors, indented span context, and relative timestamps. Designed for developers watching the terminal during development.
- **File output**: JSON with all structured fields preserved. Designed for machine parsing, `jq` queries, and feeding into analysis tools.

Both outputs share the same subscriber and filter configuration -- changing `RUST_LOG` affects both simultaneously.

```rust
pub fn init_structured_logging(log_dir: Option<&Path>) {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,wgpu=warn,naga=warn"));

    let console_layer = fmt::layer()
        .with_target(true)
        .with_thread_names(true)
        .with_level(true)
        .with_timer(fmt::time::uptime())
        .with_span_events(fmt::format::FmtSpan::CLOSE); // Log when spans close (with duration)

    let registry = tracing_subscriber::registry()
        .with(env_filter)
        .with(console_layer);

    if let Some(log_dir) = log_dir {
        std::fs::create_dir_all(log_dir).ok();
        let log_file = std::fs::File::create(log_dir.join("nebula.log"))
            .expect("Failed to create log file");

        let json_layer = fmt::layer()
            .with_writer(log_file)
            .with_ansi(false)
            .with_target(true)
            .with_timer(fmt::time::uptime())
            .with_span_events(fmt::format::FmtSpan::CLOSE)
            .json();

        registry.with(json_layer).init();
    } else {
        registry.init();
    }
}
```

### Per-Subsystem Structured Macros

To enforce consistent field usage, provide convenience macros that auto-include the subsystem field:

```rust
/// Log an info event with the subsystem field set.
macro_rules! engine_info {
    ($subsystem:expr, $($arg:tt)*) => {
        tracing::info!(subsystem = $subsystem, $($arg)*)
    };
}

macro_rules! engine_warn {
    ($subsystem:expr, $($arg:tt)*) => {
        tracing::warn!(subsystem = $subsystem, $($arg)*)
    };
}

macro_rules! engine_error {
    ($subsystem:expr, $($arg:tt)*) => {
        tracing::error!(subsystem = $subsystem, $($arg)*)
    };
}
```

Usage:

```rust
engine_info!("voxel", chunk_pos = ?pos, vertex_count = count, "Chunk meshed");
// Equivalent to: tracing::info!(subsystem = "voxel", chunk_pos = ?pos, vertex_count = count, "Chunk meshed");
```

## Outcome

All log events in the engine include structured fields: `subsystem`, `tick`, `frame`, and operation-specific fields like `entity_id`, `chunk_pos`, `duration_ms`, and `count`. The `LogContext` type provides hierarchical context that nested operations inherit automatically. Key functions are annotated with `#[instrument]` including structured fields. Console output is human-readable; file output is valid JSON with all fields preserved. Developers can use `jq` to query the JSON log file for specific subsystems, time ranges, or performance outliers.

## Demo Integration

**Demo crate:** `nebula-demo`

All log messages include structured fields: timestamp, system name, severity, entity ID, chunk address. Logs can be filtered by any field.

## Crates & Dependencies

- **`tracing = "0.1"`** -- Core tracing framework. Provides the `span!`, `info!`, `debug!`, `error!`, `warn!`, `trace!` macros with structured field support, the `#[instrument]` attribute macro, and the `Span` type for recording deferred field values.
- **`tracing-subscriber = { version = "0.3", features = ["env-filter", "fmt", "json"] }`** -- Subscriber implementation. The `fmt` feature provides human-readable console output. The `json` feature provides machine-parseable file output. The `env-filter` feature enables `RUST_LOG`-based filtering. The `FmtSpan::CLOSE` configuration logs span durations on close.

## Unit Tests

- **`test_structured_fields_appear_in_output`** -- Set up a tracing subscriber writing to a `Vec<u8>` buffer with JSON format. Emit an event: `tracing::info!(subsystem = "voxel", chunk_pos = ?[1, 2, 3], "Chunk loaded")`. Parse the buffer output as JSON. Assert the JSON object contains `"subsystem": "voxel"` and the chunk position. Assert the `"message"` field contains "Chunk loaded".

- **`test_instrument_captures_function_args`** -- Define a test function annotated with `#[instrument(fields(subsystem = "test"))]` that takes an `id: u32` argument. Set up a JSON-format subscriber writing to a buffer. Call the function with `id = 42`. Parse the output and assert the span fields include `id = 42`.

- **`test_json_output_is_valid`** -- Set up a JSON-format subscriber writing to a buffer. Emit 10 log events with varying levels, targets, and structured fields. Split the buffer by newlines (each line is one JSON object). Parse every line with `serde_json::from_str` and assert all parse successfully. Verify each object has the required fields: `timestamp`, `level`, `target`.

- **`test_subsystem_field_is_set_correctly`** -- Use `LogContext::subsystem("physics")` to enter a subsystem context. Inside the context, emit a log event. Capture the output and verify the `subsystem` field is `"physics"`. Exit the context, enter `LogContext::subsystem("render")`, emit another event, and verify the field changed to `"render"`.

- **`test_tick_number_increments`** -- Enter `LogContext::tick(0)`, emit an event, capture output, verify `tick = 0`. Drop the context. Enter `LogContext::tick(1)`, emit an event, capture output, verify `tick = 1`. Continue for ticks 2 through 5 and verify each has the correct value.

- **`test_hierarchical_context_nesting`** -- Enter `LogContext::frame(10)`. Inside it, enter `LogContext::tick(100)`. Inside that, enter `LogContext::subsystem("voxel")`. Emit a log event. Capture the output and verify all three fields are present: `frame = 10`, `tick = 100`, `subsystem = "voxel"`.

- **`test_timed_operation_records_duration`** -- Create a `TimedOperation::start("test_op")`. Sleep for 50ms. Drop the `TimedOperation`. Capture the log output and verify it contains a `duration_ms` field with a value between 40.0 and 200.0 (allowing for CI variability).

- **`test_empty_field_recorded_later`** -- Create a span with `tracing::field::Empty` for a field named `result_count`. Enter the span, do some work, then call `span.record("result_count", 42)`. Capture the output on span close and verify `result_count = 42` is present.
