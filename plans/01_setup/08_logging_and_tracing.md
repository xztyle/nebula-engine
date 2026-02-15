# Logging & Tracing

## Problem

A game engine generates enormous amounts of diagnostic output across dozens of subsystems running concurrently: rendering (draw calls, shader compilation, surface reconfiguration), networking (connection events, packet counts, latency), voxel systems (chunk loads, mesh rebuilds, palette changes), physics (collision events, body counts), ECS (system execution times, entity counts), and more. Without structured logging:

- **Debugging is a nightmare** — Print statements with no context (no timestamp, no module, no severity) are useless when the issue is an intermittent networking desync that happens 10 minutes into a multiplayer session.
- **Multiplayer issues are nearly impossible to diagnose** — When the server and client disagree on game state, you need correlated logs with precise timestamps from both sides to reconstruct what happened.
- **Performance profiling is blind** — Without span-based tracing, there is no way to see where frame time is being spent without attaching an external profiler.
- **Log noise overwhelms signal** — Without per-module filtering, enabling debug logging for networking also enables debug logging for rendering, burying the relevant information in thousands of irrelevant lines.

The `tracing` ecosystem (the spiritual successor to `log` + `env_logger`) provides structured, span-based, filterable logging that solves all of these problems.

## Solution

### Initialization

Set up the tracing subscriber during engine startup, before any other subsystem initializes:

```rust
use tracing_subscriber::{
    fmt, EnvFilter, layer::SubscriberExt, util::SubscriberInitExt,
};

pub fn init_logging(log_dir: Option<&Path>, debug_build: bool) {
    // Base filter: info by default, overridable via RUST_LOG env var
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| {
            EnvFilter::new("info,wgpu=warn,naga=warn")
        });

    // Console layer: human-readable format with timestamps
    let console_layer = fmt::layer()
        .with_target(true)          // Show module path
        .with_thread_ids(false)     // Not useful for most debugging
        .with_thread_names(true)    // Useful when render/sim threads are named
        .with_level(true)           // Show log level
        .with_timer(fmt::time::uptime());  // Time since engine start

    let subscriber = tracing_subscriber::registry()
        .with(env_filter)
        .with(console_layer);

    // In debug builds, also log to a file for post-mortem analysis
    if debug_build {
        if let Some(log_dir) = log_dir {
            std::fs::create_dir_all(log_dir).ok();

            let log_file = std::fs::File::create(
                log_dir.join("nebula.log")
            ).expect("Failed to create log file");

            let file_layer = fmt::layer()
                .with_writer(log_file)
                .with_ansi(false)  // No ANSI color codes in file output
                .with_target(true)
                .with_timer(fmt::time::uptime())
                .json();  // Structured JSON for machine parsing

            subscriber.with(file_layer).init();
        } else {
            subscriber.init();
        }
    } else {
        subscriber.init();
    }
}
```

### Log Targets Per Subsystem

Each engine crate uses a consistent log target based on its module path. The `tracing` crate automatically uses the module path as the target, but we document the convention explicitly:

| Crate                | Log Target              | Description                                      |
|----------------------|-------------------------|--------------------------------------------------|
| `nebula-app`         | `nebula_app`            | Application lifecycle, startup, shutdown         |
| `nebula-render`      | `nebula_render`         | Draw calls, shader compilation, surface config   |
| `nebula-net`         | `nebula_net`            | TCP connections, packet framing, bandwidth       |
| `nebula-multiplayer` | `nebula_multiplayer`    | Replication, prediction, interest management     |
| `nebula-voxel`       | `nebula_voxel`          | Chunk loading, palette changes, storage          |
| `nebula-mesh`        | `nebula_mesh`           | Meshing operations, vertex counts, timing        |
| `nebula-terrain`     | `nebula_terrain`        | Terrain generation, noise evaluation             |
| `nebula-physics`     | `nebula_physics`        | Collision events, body lifecycle, step timing    |
| `nebula-ecs`         | `nebula_ecs`            | System registration, schedule execution          |
| `nebula-audio`       | `nebula_audio`          | Sound playback, streaming, bus management        |
| `nebula-assets`      | `nebula_assets`         | Asset loading, cache hits/misses, hot-reload     |
| `nebula-input`       | `nebula_input`          | Key/mouse events, action mapping                 |
| `nebula-cubesphere`  | `nebula_cubesphere`     | Face subdivision, quadtree operations            |
| `nebula-lod`         | `nebula_lod`            | LOD transitions, distance calculations           |
| `nebula-planet`      | `nebula_planet`         | Planet rendering, atmosphere, horizon culling    |
| `nebula-math`        | `nebula_math`           | (Rarely used, only for overflow/precision warnings) |

### Filtering Examples

Users and developers control log output via the `RUST_LOG` environment variable:

```bash
# Default: info level for everything, suppress noisy wgpu/naga internals
RUST_LOG=info,wgpu=warn,naga=warn

# Debug networking only
RUST_LOG=info,nebula_net=debug,nebula_multiplayer=debug

# Trace-level voxel system for diagnosing chunk issues
RUST_LOG=info,nebula_voxel=trace,nebula_mesh=debug

# Silence everything except errors (for benchmarking)
RUST_LOG=error

# Full trace for all nebula crates
RUST_LOG=trace,wgpu=warn,naga=warn
```

### Span-Based Tracing with `#[instrument]`

Key functions are annotated with `#[instrument]` to create tracing spans. This provides hierarchical timing information and context propagation:

```rust
use tracing::instrument;

#[instrument(skip(chunk_data), fields(chunk_pos = %pos))]
pub fn mesh_chunk(pos: ChunkPos, chunk_data: &ChunkData) -> Mesh {
    tracing::debug!("Starting mesh generation");
    // ... meshing logic ...
    tracing::debug!(vertex_count = mesh.vertex_count(), "Mesh complete");
    mesh
}
```

This produces log output like:

```
  0.153s DEBUG nebula_mesh: Starting mesh generation chunk_pos=(4, 2, -1)
  0.158s DEBUG nebula_mesh: Mesh complete chunk_pos=(4, 2, -1) vertex_count=12847
```

### Recommended Span Locations

The `#[instrument]` attribute should be placed on:

- **Frame boundaries** — The main render function, to track per-frame timing.
- **Chunk operations** — Mesh generation, terrain generation, chunk load/unload, to identify bottlenecks in the voxel pipeline.
- **Network operations** — Connection handshake, message send/receive, replication sync, for diagnosing multiplayer issues.
- **Asset loading** — Each asset load operation, to identify slow assets or cache misses.
- **Physics stepping** — The physics world step, to track simulation cost.
- **ECS schedule runs** — Each schedule execution, to identify slow systems.

### Log Levels Convention

| Level   | Usage                                                                          |
|---------|--------------------------------------------------------------------------------|
| `error` | Unrecoverable failures: GPU lost, file corruption, network protocol violation  |
| `warn`  | Recoverable issues: missing asset (using fallback), frame time spike, timeout  |
| `info`  | Lifecycle events: engine started, connected to server, world loaded            |
| `debug` | Operational detail: chunk meshed, packet sent, input action triggered          |
| `trace` | Per-frame/per-tick detail: every voxel access, every draw call, every byte     |

### File Logging (Debug Builds)

In debug builds, logs are additionally written to a file in the platform log directory (see `03_cross_platform_build_validation.md`). The file log uses JSON format for machine parsing:

```json
{"timestamp":"0.153s","level":"DEBUG","target":"nebula_mesh","message":"Starting mesh generation","chunk_pos":"(4, 2, -1)"}
```

This enables post-mortem analysis with tools like `jq`:

```bash
# Find all errors in the log
jq 'select(.level == "ERROR")' nebula.log

# Find networking events in a time range
jq 'select(.target | startswith("nebula_net")) | select(.timestamp > "10.0s")' nebula.log
```

### Integration with Config System

The debug config (see `07_configuration_system.md`) includes a `log_level` field that can override the default filter. On startup:

```rust
let filter = if !config.debug.log_level.is_empty() {
    EnvFilter::new(&config.debug.log_level)
} else {
    EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,wgpu=warn,naga=warn"))
};
```

This allows users to set the log level in their config file without needing to set environment variables.

### Future: Tracy/Puffin Integration

The `tracing` ecosystem supports additional subscribers. In the future, a `tracing-tracy` or `tracing-puffin` layer can be added for visual profiling without changing any instrumentation code. The `#[instrument]` attributes and `tracing::span!` calls will automatically feed data to these profilers.

## Outcome

Running the engine produces structured log output to the console with timestamps, module paths, and severity levels. Setting `RUST_LOG=nebula_net=debug` shows only networking debug logs while keeping everything else at info level. In debug builds, a JSON log file is written to the platform log directory for post-mortem analysis. Key functions have `#[instrument]` attributes that create hierarchical spans for performance analysis. The log level can be configured via environment variable, config file, or CLI flag.

## Demo Integration

**Demo crate:** `nebula-demo`

The console shows structured, timestamped log output with severity levels and module paths. Setting `RUST_LOG=nebula_demo=debug` reveals the internal startup sequence.

## Crates & Dependencies

- **`tracing = "0.1"`** — Structured logging and tracing framework. Provides macros (`info!`, `debug!`, `warn!`, `error!`, `trace!`), span creation (`#[instrument]`, `span!`), and the subscriber infrastructure.
- **`tracing-subscriber = { version = "0.3", features = ["env-filter", "fmt", "json"] }`** — Subscriber implementation with environment-based filtering (`EnvFilter`), formatted console output (`fmt`), and JSON output for file logging. The `env-filter` feature enables `RUST_LOG` parsing. The `fmt` feature provides the human-readable console formatter. The `json` feature provides structured file output.

## Unit Tests

- **`test_default_log_level`** — Create an `EnvFilter` with the default filter string `"info,wgpu=warn,naga=warn"`. Verify that:
  - A log event at `info` level with target `nebula_render` is enabled (passes the filter).
  - A log event at `debug` level with target `nebula_render` is NOT enabled.
  - A log event at `warn` level with target `wgpu_core` is enabled.
  - A log event at `info` level with target `wgpu_core` is NOT enabled.

- **`test_subsystem_filter`** — Create an `EnvFilter` with `"info,nebula_net=debug"`. Verify that:
  - A `debug` event with target `nebula_net` IS enabled.
  - A `debug` event with target `nebula_render` is NOT enabled.
  - An `info` event with target `nebula_render` IS enabled.

- **`test_log_output_format`** — Set up a subscriber with a `fmt::layer()` writing to a `Vec<u8>` buffer. Emit a log event and capture the output. Verify the output string contains:
  - A timestamp (matches a pattern like `0.XXXs` or a date format).
  - The module path (e.g., `nebula_test`).
  - The log level (e.g., `INFO`).
  - The message text.

- **`test_json_format`** — Set up a subscriber with a JSON `fmt::layer()` writing to a buffer. Emit a log event, parse the output as JSON, and verify it contains the expected fields: `timestamp`, `level`, `target`, `message`.

- **`test_env_filter_parsing`** — Verify that various `RUST_LOG` strings parse without error:
  - `"info"`
  - `"debug,nebula_render=trace"`
  - `"warn,nebula_net=debug,nebula_voxel=trace"`
  - `"error"`
  An invalid filter string (e.g., `"not a valid filter!!!"`) should either return a parse error or fall back to a default.

- **`test_file_logger_creation`** — Create a temporary directory, call the file logging setup code, and verify that a `nebula.log` file is created in the directory. Write a log event and verify the file is non-empty.

- **`test_uptime_timer_starts_near_zero`** — Emit a log event immediately after subscriber initialization and verify the timestamp is close to 0 seconds (within 1 second). This validates that the uptime timer starts at engine initialization, not at Unix epoch.
