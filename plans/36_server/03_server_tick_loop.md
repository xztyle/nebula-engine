# Server Tick Loop

## Problem

The server must run its simulation at a fixed rate that matches the client's `FixedUpdate` schedule (60 ticks per second, as established in `01_setup/06_main_game_loop_with_fixed_timestep.md`). Deterministic multiplayer requires that the server and all clients step their simulation at identical intervals — if the server ticks at a different rate than clients expect, physics diverges, prediction breaks, and entity positions desync. Without a proper server tick loop:

- **Simulation rate is uncontrolled** — A naive `loop { process(); }` runs as fast as the CPU allows, consuming 100% of a core and producing simulation steps at irregular intervals. Physics integrators produce different results at different timesteps, breaking determinism.
- **Network timing is inconsistent** — Clients rely on receiving state updates at a predictable cadence. If the server sends updates at erratic intervals, client-side interpolation stutters and prediction overcorrects.
- **Overrun goes undetected** — If the game logic, physics, or network processing takes longer than the 16.67ms tick budget, the server falls behind real time. Without detection, this cascading delay builds silently until the server is seconds behind and players experience massive lag.
- **No rendering to pace the loop** — The client's tick loop is paced by `winit`'s `RedrawRequested` event and VSync. The server has no window, no VSync, and no redraw events. It must pace itself using sleep and timing.
- **Spiral of death on the server** — If one tick takes too long, the accumulator grows, causing the next frame to process multiple ticks, which takes even longer, causing more accumulation. The server must cap catch-up attempts.

The server tick loop uses the same accumulator pattern as the client but replaces the variable-rate render step with a fixed-rate sleep.

## Solution

### Constants

```rust
/// Fixed simulation timestep: 60 Hz (matches client FixedUpdate)
pub const SERVER_TICK_RATE: f64 = 60.0;
pub const SERVER_TICK_DT: f64 = 1.0 / SERVER_TICK_RATE;

/// Maximum accumulated time before we drop ticks to prevent spiral of death.
/// At 250ms, the server will process at most 15 ticks per loop iteration
/// before sleeping, preventing unbounded catch-up.
pub const MAX_TICK_ACCUMULATION: f64 = 0.25;

/// Threshold for logging a tick overrun warning.
/// If a single tick takes longer than this, something is wrong.
pub const TICK_OVERRUN_THRESHOLD: f64 = SERVER_TICK_DT * 1.5; // 25ms
```

### Tick Loop Implementation

```rust
use std::time::{Duration, Instant};
use tokio::sync::watch;

pub struct ServerTickLoop {
    tick_count: u64,
    accumulator: f64,
    previous_time: Instant,
    total_sim_time: f64,
    overrun_count: u64,
}

impl ServerTickLoop {
    pub fn new() -> Self {
        Self {
            tick_count: 0,
            accumulator: 0.0,
            previous_time: Instant::now(),
            total_sim_time: 0.0,
            overrun_count: 0,
        }
    }

    pub fn tick_count(&self) -> u64 {
        self.tick_count
    }

    pub fn total_sim_time(&self) -> f64 {
        self.total_sim_time
    }

    pub fn overrun_count(&self) -> u64 {
        self.overrun_count
    }
}

pub async fn run(
    mut world: bevy_ecs::world::World,
    mut schedule: bevy_ecs::schedule::Schedule,
    listener: tokio::net::TcpListener,
    config: super::config::ServerConfig,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let mut tick_loop = ServerTickLoop::new();
    let mut admin_rx = super::cli_admin::setup_admin_channel();

    tracing::info!(
        tick_rate = SERVER_TICK_RATE,
        tick_dt_ms = SERVER_TICK_DT * 1000.0,
        "Server tick loop starting"
    );

    loop {
        // Check for shutdown signal
        if *shutdown_rx.borrow() {
            tracing::info!(
                total_ticks = tick_loop.tick_count,
                overruns = tick_loop.overrun_count,
                "Tick loop received shutdown signal"
            );
            break;
        }

        // Calculate elapsed time since last iteration
        let now = Instant::now();
        let mut elapsed = now.duration_since(tick_loop.previous_time).as_secs_f64();
        tick_loop.previous_time = now;

        // Clamp to prevent spiral of death
        if elapsed > MAX_TICK_ACCUMULATION {
            tracing::warn!(
                elapsed_ms = elapsed * 1000.0,
                max_ms = MAX_TICK_ACCUMULATION * 1000.0,
                "Tick accumulation clamped (server falling behind)"
            );
            elapsed = MAX_TICK_ACCUMULATION;
        }

        tick_loop.accumulator += elapsed;

        // Process as many fixed-rate ticks as the accumulator allows
        while tick_loop.accumulator >= SERVER_TICK_DT {
            let tick_start = Instant::now();

            // === TICK STAGES ===

            // Stage 1: Process admin commands (non-blocking)
            while let Ok(cmd) = admin_rx.try_recv() {
                super::cli_admin::process_admin_command(
                    cmd, &mut world, &shutdown_rx
                ).await;
            }

            // Stage 2: Accept new TCP connections (non-blocking poll)
            accept_pending_connections(&listener, &mut world).await;

            // Stage 3: Read inbound network messages
            // Drains all buffered messages from connected clients into
            // the ECS world as events for systems to process.
            drain_network_messages(&mut world);

            // Stage 4: Run game logic (ECS schedule)
            // This runs the same shared systems as the client's FixedUpdate:
            //   - Input processing (from network messages, not OS events)
            //   - Game logic (crafting, inventory, interactions)
            //   - Physics step (Rapier world.step())
            //   - Terrain generation (for chunks near players)
            //   - Entity replication (mark dirty components)
            schedule.run(&mut world);

            // Stage 5: Send outbound state updates
            // Serializes dirty entity state and sends to each client
            // based on interest management (only entities near the player).
            send_state_updates(&mut world).await;

            // === END TICK STAGES ===

            tick_loop.accumulator -= SERVER_TICK_DT;
            tick_loop.total_sim_time += SERVER_TICK_DT;
            tick_loop.tick_count += 1;

            // Check for tick overrun
            let tick_duration = tick_start.elapsed().as_secs_f64();
            if tick_duration > TICK_OVERRUN_THRESHOLD {
                tick_loop.overrun_count += 1;
                tracing::warn!(
                    tick = tick_loop.tick_count,
                    duration_ms = tick_duration * 1000.0,
                    budget_ms = SERVER_TICK_DT * 1000.0,
                    total_overruns = tick_loop.overrun_count,
                    "Tick overrun: processing exceeded budget"
                );
            }
        }

        // Sleep for the remaining time until the next tick is due.
        // This prevents the server from busy-looping at 100% CPU.
        let sleep_duration = SERVER_TICK_DT - tick_loop.accumulator;
        if sleep_duration > 0.0 {
            tokio::time::sleep(Duration::from_secs_f64(sleep_duration)).await;
        }
    }

    tracing::info!("Server tick loop exited");
}

/// Non-blocking accept of any pending TCP connections.
async fn accept_pending_connections(
    listener: &tokio::net::TcpListener,
    world: &mut bevy_ecs::world::World,
) {
    // Use try_accept or a short timeout to avoid blocking the tick loop.
    // tokio's TcpListener with poll_accept pattern.
    loop {
        match listener.try_accept() {
            Ok((stream, peer_addr)) => {
                stream.set_nodelay(true).ok();
                tracing::info!("Accepted connection from {peer_addr}");
                // Register the new connection in the ECS world
                // (handled by nebula-net connection manager)
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                break; // No more pending connections
            }
            Err(e) => {
                tracing::error!("Accept error: {e}");
                break;
            }
        }
    }
}

fn drain_network_messages(world: &mut bevy_ecs::world::World) {
    // Read all buffered messages from each connection's receive buffer
    // and insert them as ECS events for game systems to process.
    // Implementation details in nebula-net and nebula-multiplayer.
}

async fn send_state_updates(world: &mut bevy_ecs::world::World) {
    // Iterate over entities with dirty replication components,
    // serialize their state, and write to each connection's send buffer
    // based on interest management.
    // Implementation details in nebula-multiplayer.
}
```

### Tick Stages Order

Each tick processes stages in a strict order to ensure consistency:

```
1. Admin commands     — Process operator commands (kick, save, shutdown)
2. Accept connections — Register new TCP connections in the ECS
3. Read network       — Drain inbound messages into ECS events
4. Game logic         — ECS schedule: input → logic → physics → terrain → replication
5. Send updates       — Serialize and transmit state to clients
```

This ordering ensures that:
- Admin commands take effect before the next tick's logic runs.
- Inbound messages are available to game systems within the same tick they arrive.
- Outbound updates reflect the result of this tick's simulation, not stale data from the previous tick.
- New connections are registered before their first messages are processed.

### Timing Diagram

```
|--- tick 0 ---|--- tick 1 ---|--- tick 2 ---|--- sleep ---|
    16.67ms        16.67ms        16.67ms       remainder

<--- elapsed = 54ms (example) --->
accumulator = 54ms → 3 ticks + 3.99ms remainder
sleep for 16.67ms - 3.99ms = 12.68ms
```

### Comparison with Client Loop

| Aspect | Client (story 06) | Server (this story) |
|--------|-------------------|---------------------|
| Tick rate | 60 Hz | 60 Hz (identical) |
| Fixed timestep | `FIXED_DT = 1/60` | `SERVER_TICK_DT = 1/60` |
| Variable-rate step | Render with interpolation alpha | None (no rendering) |
| Pacing mechanism | VSync / `request_redraw` | `tokio::time::sleep` |
| Spiral-of-death cap | `MAX_FRAME_TIME = 0.25` | `MAX_TICK_ACCUMULATION = 0.25` |
| Accumulator pattern | Identical | Identical |

## Outcome

A `tick_loop.rs` module in `crates/nebula-server/src/` that runs the server simulation at exactly 60 ticks per second using a fixed-timestep accumulator. Each tick processes admin commands, accepts connections, reads network messages, runs the ECS schedule (shared game logic and physics), and sends state updates. Tick overruns are detected and logged. The accumulator is clamped to prevent spiral of death. The loop sleeps between ticks to avoid burning CPU. The loop exits cleanly on shutdown signal. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

The server simulation runs at a fixed 60 Hz tick rate. The CLI displays tick rate and count. Performance remains stable under load with multiple connected clients.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `tokio` | `1.49` (features: `time`, `sync`, `net`) | Async sleep for tick pacing, watch channel for shutdown, TCP accept |
| `bevy_ecs` | `0.18` | ECS `World` and `Schedule` for running game systems |
| `tracing` | `0.1` | Structured logging for tick timing, overruns, and lifecycle events |

## Unit Tests

- **`test_tick_rate_is_60_per_second`** — Assert `SERVER_TICK_RATE == 60.0` and `SERVER_TICK_DT` is approximately `1.0 / 60.0` within `f64::EPSILON * 10.0`. Assert `SERVER_TICK_DT * 1000.0` is approximately `16.667` milliseconds.

- **`test_tick_loop_processes_all_stages`** — Create a `World` with a counter resource and a schedule containing a system that increments it. Manually set the accumulator to `3.0 * SERVER_TICK_DT`. Run the tick processing loop (not the full async `run`, but the inner accumulator-draining while-loop). Assert the counter equals 3 and the accumulator is less than `SERVER_TICK_DT`.

- **`test_overrun_is_detected`** — Create a `ServerTickLoop`, simulate a tick that takes 25ms (greater than `TICK_OVERRUN_THRESHOLD`). Assert `overrun_count` is incremented to 1. Simulate a tick that takes 10ms (under threshold). Assert `overrun_count` remains 1.

- **`test_accumulator_clamped_on_spike`** — Create a `ServerTickLoop` and manually add 1.0 second of elapsed time to the accumulator logic. Assert the accumulator is clamped to `MAX_TICK_ACCUMULATION` (0.25 seconds). Assert the number of ticks processed is at most `ceil(0.25 / SERVER_TICK_DT)` = 15, not 60.

- **`test_accumulated_time_consumed_correctly`** — Set the accumulator to exactly `2.5 * SERVER_TICK_DT`. Process ticks. Assert exactly 2 ticks run. Assert the remaining accumulator is approximately `0.5 * SERVER_TICK_DT`.

- **`test_loop_shuts_down_on_stop_signal`** — Create a `watch::channel`, set the value to `true`, and run the tick loop. Assert it exits within 100ms without panic. Verify `tick_count` is 0 (it checked shutdown before processing any ticks).

- **`test_sleep_duration_calculation`** — After processing ticks with 5ms of accumulator remaining, assert the calculated sleep duration is approximately `SERVER_TICK_DT - 0.005` = `~11.67ms`. After processing with 0ms remaining, assert sleep duration is approximately `SERVER_TICK_DT` = `~16.67ms`.

- **`test_tick_count_advances`** — Run the accumulator loop with accumulator set to `10.0 * SERVER_TICK_DT`. Assert `tick_count` advances by exactly 10. Assert `total_sim_time` is approximately `10.0 * SERVER_TICK_DT`.

- **`test_zero_elapsed_produces_no_ticks`** — Set elapsed time to 0. Process the accumulator. Assert no ticks are processed and the accumulator remains 0.

- **`test_stage_ordering_is_deterministic`** — Create a `World` with a `Vec<String>` resource. Register systems in stages that push their stage name to the vector: `"admin"`, `"network_read"`, `"game_logic"`, `"network_send"`. Run one tick. Assert the vector contains the stages in the correct order.
