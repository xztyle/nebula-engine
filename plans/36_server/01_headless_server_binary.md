# Headless Server Binary

## Problem

Nebula Engine's multiplayer architecture requires a dedicated server that runs the authoritative game simulation. This server must execute the same ECS world, voxel systems, physics stepping, terrain generation, and networking code as the client — but without any rendering, windowing, or GPU dependencies. Without a separate headless binary:

- **Deployment is impossible** — A server binary that links against wgpu, winit, and egui cannot run on headless Linux VMs, Docker containers, or cloud instances that have no GPU or display server. Every major hosting provider (AWS, Hetzner, OVH) offers headless machines at a fraction of the cost of GPU instances. Requiring a GPU for the server is a non-starter.
- **Compilation fails on headless targets** — wgpu requires a graphics backend (Vulkan, Metal, DX12) at link time. winit requires a display server (X11, Wayland, Win32) at link time. Neither compiles on a minimal server OS image without installing hundreds of megabytes of graphics libraries.
- **Resource waste** — Even if the server could compile with GPU crates, loading shader pipelines, allocating GPU memory, and initializing a window manager wastes memory and CPU cycles that should be spent on simulation, physics, and networking.
- **Build times suffer** — Compiling wgpu and its transitive dependencies (naga, gpu-allocator, etc.) adds minutes to every build. The server CI pipeline should not pay this cost.

The solution is a separate binary crate (`nebula-server`) that depends only on the shared simulation crates and excludes all rendering, windowing, audio, and UI crates via Cargo feature gates.

## Solution

### Crate Layout

The `nebula-server` binary crate lives at `crates/nebula-server/` in the workspace. Its `Cargo.toml` declares dependencies only on the shared simulation crates:

```toml
[package]
name = "nebula-server"
version = "0.1.0"
edition = "2024"

[[bin]]
name = "nebula-server"
path = "src/main.rs"

[dependencies]
# Shared simulation crates (no GPU dependency)
nebula-math = { path = "../nebula-math" }
nebula-coords = { path = "../nebula-coords" }
nebula-voxel = { path = "../nebula-voxel" }
nebula-terrain = { path = "../nebula-terrain" }
nebula-physics = { path = "../nebula-physics" }
nebula-ecs = { path = "../nebula-ecs" }
nebula-net = { path = "../nebula-net" }
nebula-multiplayer = { path = "../nebula-multiplayer" }

# External dependencies
tokio = { version = "1.49", features = ["rt-multi-thread", "net", "io-util", "macros", "signal"] }
bevy_ecs = "0.18"
clap = { version = "4", features = ["derive"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "fmt", "json"] }
serde = { version = "1.0", features = ["derive"] }
postcard = { version = "1.1", features = ["alloc"] }

# nebula-render, nebula-ui, nebula-audio, nebula-particles are NOT listed here
```

The key insight is that `nebula-server` simply does not depend on rendering crates at all. There is no `default-features = false` trick needed — the server crate's dependency list is a strict subset of the client's. The shared crates (`nebula-math`, `nebula-voxel`, `nebula-ecs`, etc.) have no transitive dependency on wgpu or winit because they were designed as pure logic crates from the start (see `01_workspace_and_crate_structure.md`).

### Server Entry Point

```rust
// crates/nebula-server/src/main.rs

use clap::Parser;
use tracing_subscriber::{fmt, EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

mod config;
mod tick_loop;
mod cli_admin;
mod monitoring;

use config::ServerConfig;

#[derive(Parser, Debug)]
#[command(name = "nebula-server", about = "Nebula Engine Dedicated Server")]
struct Args {
    /// Path to server config file
    #[arg(long, default_value = "server_config.ron")]
    config: std::path::PathBuf,

    /// Override bind address
    #[arg(long)]
    bind: Option<String>,

    /// Override bind port
    #[arg(long)]
    port: Option<u16>,

    /// Override max players
    #[arg(long)]
    max_players: Option<u32>,
}

fn main() {
    let args = Args::parse();

    // Initialize tracing (console + optional file)
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt::layer().with_target(true).with_timer(fmt::time::uptime()))
        .init();

    tracing::info!("Nebula Engine Dedicated Server starting");

    // Load config and apply CLI overrides
    let mut config = ServerConfig::load_or_default(&args.config);
    if let Some(ref bind) = args.bind {
        config.bind_address = bind.clone();
    }
    if let Some(port) = args.port {
        config.port = port;
    }
    if let Some(max) = args.max_players {
        config.max_players = max;
    }

    tracing::info!(
        bind = %config.bind_address,
        port = config.port,
        max_players = config.max_players,
        "Server configuration loaded"
    );

    // Build the tokio runtime explicitly (no #[tokio::main] so we control shutdown)
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to create tokio runtime");

    runtime.block_on(async {
        run_server(config).await;
    });

    tracing::info!("Server shut down cleanly");
}

async fn run_server(config: ServerConfig) {
    // Initialize the ECS world with shared components and systems
    let mut world = bevy_ecs::world::World::new();
    let mut schedule = bevy_ecs::schedule::Schedule::default();

    // Register shared systems: voxel, terrain, physics, networking
    // (These are the same systems the client registers in FixedUpdate)
    nebula_ecs::register_shared_systems(&mut schedule);

    // Bind the TCP listener
    let bind_addr = format!("{}:{}", config.bind_address, config.port);
    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .expect("Failed to bind TCP listener");

    tracing::info!("Listening on {bind_addr}");

    // Start the server tick loop (story 03)
    // Start the CLI admin reader (story 02)
    // Start the monitoring system (story 06)
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    // Handle OS signals for graceful shutdown
    let signal_shutdown = shutdown_tx.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        tracing::info!("Received SIGINT, initiating shutdown");
        let _ = signal_shutdown.send(true);
    });

    tick_loop::run(world, schedule, listener, config, shutdown_rx).await;
}
```

### No Rendering, No Window, No GPU

The server binary does not:
- Import or link against `wgpu`, `wgpu-core`, `wgpu-hal`, `naga`, or any GPU abstraction crate.
- Import or link against `winit`, `raw-window-handle`, or any windowing crate.
- Import or link against `egui`, `egui-wgpu`, or any UI crate.
- Import or link against any audio crate (`kira`, `rodio`, `cpal`).
- Create a window, an event loop, a GPU device, or a swap chain.
- Allocate any GPU memory or compile any shaders.
- Call any platform display APIs (X11, Wayland, Win32, AppKit).

The `cargo tree` output for `nebula-server` must not contain any of these crates. This is verified in CI (see story 05).

### Shared Code Architecture

The boundary between client-only and shared code is defined at the crate level, not with `#[cfg]` attributes scattered throughout:

```
Shared (server + client):        Client-only:
  nebula-math                      nebula-render
  nebula-coords                    nebula-lighting
  nebula-voxel                     nebula-materials
  nebula-mesh (data structures)    nebula-particles
  nebula-terrain                   nebula-ui
  nebula-physics                   nebula-audio
  nebula-ecs                       nebula-input (OS events)
  nebula-net                       nebula-player (camera)
  nebula-multiplayer               nebula-app
```

Systems that exist in both client and server (e.g., physics stepping, terrain generation, entity replication) are defined in the shared crates and registered into the ECS schedule by both binaries.

### Docker Deployment

The server binary can be deployed in a minimal Docker container:

```dockerfile
FROM rust:1.87-slim AS builder
WORKDIR /app
COPY . .
RUN cargo build --release -p nebula-server

FROM debian:bookworm-slim
COPY --from=builder /app/target/release/nebula-server /usr/local/bin/
EXPOSE 7777
ENTRYPOINT ["nebula-server"]
```

No GPU drivers, no display server, no window manager required in the final image.

## Outcome

A `nebula-server` binary crate at `crates/nebula-server/` that compiles and runs without any GPU, display, windowing, or audio dependencies. The binary initializes the ECS world with shared simulation systems, binds a TCP listener, and runs the server tick loop. It can be deployed on headless machines and in Docker containers. The dependency tree contains zero rendering crates. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

Running `nebula-demo --server` starts a headless process with no window and no GPU. The console shows `Headless server started on 0.0.0.0:7777`.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `tokio` | `1.49` (features: `rt-multi-thread`, `net`, `io-util`, `macros`, `signal`) | Async runtime, TCP listener, graceful shutdown via OS signals |
| `bevy_ecs` | `0.18` | ECS world and schedule for running shared game systems |
| `clap` | `4` (features: `derive`) | Command-line argument parsing for server binary |
| `tracing` | `0.1` | Structured logging for server events |
| `tracing-subscriber` | `0.3` (features: `env-filter`, `fmt`, `json`) | Console and file log output with filtering |
| `serde` | `1.0` (features: `derive`) | Serialization for config and network messages |
| `postcard` | `1.1` (features: `alloc`) | Compact binary serialization for network protocol |

## Unit Tests

- **`test_server_binary_compiles_without_gpu_crates`** — Run `cargo tree -p nebula-server` and assert that the output does not contain the strings `wgpu`, `winit`, `egui`, `naga`, `gpu-allocator`, `raw-window-handle`, `kira`, or `cpal`. This is an integration test that verifies the dependency boundary. The test parses the output line by line and fails with a descriptive message if any GPU/window/audio crate is found.

- **`test_server_starts_and_runs_ecs_loop`** — Create a `World` and `Schedule` with a single test system that increments a counter resource. Run the schedule 10 times. Assert the counter resource equals 10. This validates that the ECS simulation loop functions without any rendering systems present.

- **`test_server_accepts_tcp_connections`** — Bind a `TcpListener` to `127.0.0.1:0` (ephemeral port), spawn the server accept loop, connect a `TcpStream` from the test, and assert the connection succeeds. Verify the server's connection count is 1. This validates that the networking layer works in the headless binary.

- **`test_server_processes_game_logic_without_renderer`** — Register shared systems (physics step, terrain generation stub, entity replication) into a schedule. Run the schedule with a world containing test entities with `WorldPos` and `Velocity` components. Assert that `WorldPos` values change after running the schedule, confirming game logic executes without rendering.

- **`test_server_shuts_down_cleanly`** — Start the server with a `watch::channel` for shutdown. Send the shutdown signal. Assert that the server task completes within 1 second. Connect a TCP client before shutdown and verify it receives EOF after shutdown completes. Verify no panic or resource leak warnings in the log output.
