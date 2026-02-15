# Server Monitoring

## Problem

A dedicated game server is a long-running process that must be observable. Operators need to know the server's health, performance, and resource usage — not just at the moment they type `status` in the admin console, but continuously over hours and days. Without monitoring:

- **Silent performance degradation** — The tick rate quietly drops from 60 to 45 due to a terrain generation bottleneck. No one notices until players complain about lag, and by then the root cause is impossible to identify because no metrics were recorded.
- **Memory leaks go undetected** — A chunk that is never unloaded slowly accumulates. Memory usage climbs from 2 GB to 16 GB over 24 hours. Without tracking, the server is killed by the OOM reaper at 3 AM with no diagnostic data.
- **Capacity planning is guesswork** — Without historical data on CPU usage per system, memory per player, and bandwidth per connection, the operator cannot answer "How many players can this server handle?" or "Do I need to upgrade before the weekend event?"
- **Disconnect spikes are invisible** — If 15 players disconnect within 10 seconds, something is wrong (network issue, game-breaking bug, server crash). Without monitoring, the operator only sees "14 players online" instead of "15 players just disconnected simultaneously."
- **No integration with external tools** — Production servers use Prometheus, Grafana, Datadog, or similar tools to aggregate metrics across multiple servers. Without an HTTP metrics endpoint, the game server is a black box to the operations team.

The monitoring system records metrics internally, logs them periodically, and optionally exposes them via a lightweight HTTP endpoint.

## Solution

### Metrics Registry

```rust
use std::sync::atomic::{AtomicU64, AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug)]
pub struct ServerMetrics {
    /// Server start time for uptime calculation
    start_time: Instant,

    /// Current simulation tick count
    pub tick_count: AtomicU64,

    /// Measured tick rate (ticks per second, averaged over the last second)
    /// Stored as f64 bits in a u64 for atomic access.
    tick_rate_bits: AtomicU64,

    /// Number of currently connected players
    pub player_count: AtomicU64,

    /// Peak player count since server start
    pub peak_player_count: AtomicU64,

    /// Number of loaded chunks
    pub chunk_count: AtomicU64,

    /// Approximate memory usage in bytes (from allocator stats or /proc/self)
    pub memory_bytes: AtomicU64,

    /// Total bytes received across all connections since server start
    pub bytes_received: AtomicU64,

    /// Total bytes sent across all connections since server start
    pub bytes_sent: AtomicU64,

    /// Number of player disconnects in the last monitoring interval
    pub recent_disconnects: AtomicU64,

    /// Number of tick overruns in the last monitoring interval
    pub recent_overruns: AtomicU64,

    /// Per-system execution times (updated each tick, in microseconds)
    /// Stored as a map behind a RwLock for system-name lookups.
    pub system_times: tokio::sync::RwLock<std::collections::HashMap<String, u64>>,
}

impl ServerMetrics {
    pub fn new() -> Self {
        Self {
            start_time: Instant::now(),
            tick_count: AtomicU64::new(0),
            tick_rate_bits: AtomicU64::new(f64::to_bits(0.0)),
            player_count: AtomicU64::new(0),
            peak_player_count: AtomicU64::new(0),
            chunk_count: AtomicU64::new(0),
            memory_bytes: AtomicU64::new(0),
            bytes_received: AtomicU64::new(0),
            bytes_sent: AtomicU64::new(0),
            recent_disconnects: AtomicU64::new(0),
            recent_overruns: AtomicU64::new(0),
            system_times: tokio::sync::RwLock::new(std::collections::HashMap::new()),
        }
    }

    pub fn uptime(&self) -> std::time::Duration {
        self.start_time.elapsed()
    }

    pub fn tick_rate(&self) -> f64 {
        f64::from_bits(self.tick_rate_bits.load(Ordering::Relaxed))
    }

    pub fn set_tick_rate(&self, rate: f64) {
        self.tick_rate_bits.store(f64::to_bits(rate), Ordering::Relaxed);
    }

    pub fn update_player_count(&self, count: u64) {
        self.player_count.store(count, Ordering::Relaxed);
        let current_peak = self.peak_player_count.load(Ordering::Relaxed);
        if count > current_peak {
            self.peak_player_count.store(count, Ordering::Relaxed);
        }
    }
}
```

### Tick Rate Measurement

The tick rate is measured by counting ticks over a rolling 1-second window, not by inverting the duration of a single tick (which is noisy):

```rust
pub struct TickRateMeasurer {
    window_start: Instant,
    ticks_in_window: u64,
}

impl TickRateMeasurer {
    pub fn new() -> Self {
        Self {
            window_start: Instant::now(),
            ticks_in_window: 0,
        }
    }

    /// Call this once per tick. Returns Some(measured_rate) when the
    /// 1-second window rolls over.
    pub fn record_tick(&mut self) -> Option<f64> {
        self.ticks_in_window += 1;

        let elapsed = self.window_start.elapsed().as_secs_f64();
        if elapsed >= 1.0 {
            let rate = self.ticks_in_window as f64 / elapsed;
            self.ticks_in_window = 0;
            self.window_start = Instant::now();
            Some(rate)
        } else {
            None
        }
    }
}
```

### Memory Usage (Linux)

On Linux, memory usage is read from `/proc/self/status`:

```rust
#[cfg(target_os = "linux")]
pub fn get_memory_usage_bytes() -> u64 {
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        for line in status.lines() {
            if line.starts_with("VmRSS:") {
                // Format: "VmRSS:    12345 kB"
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    if let Ok(kb) = parts[1].parse::<u64>() {
                        return kb * 1024; // Convert kB to bytes
                    }
                }
            }
        }
    }
    0
}

#[cfg(not(target_os = "linux"))]
pub fn get_memory_usage_bytes() -> u64 {
    // On non-Linux platforms, return 0 (monitoring is best-effort)
    0
}
```

### Periodic Logging

A monitoring task runs on a tokio interval timer and logs metrics every 30 seconds using structured tracing fields:

```rust
use std::sync::Arc;
use tokio::sync::watch;

pub async fn run_monitoring_loop(
    metrics: Arc<ServerMetrics>,
    log_interval_secs: u64,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let mut interval = tokio::time::interval(
        std::time::Duration::from_secs(log_interval_secs)
    );

    loop {
        tokio::select! {
            _ = interval.tick() => {
                // Update memory usage
                metrics.memory_bytes.store(
                    get_memory_usage_bytes(),
                    std::sync::atomic::Ordering::Relaxed,
                );

                let uptime = metrics.uptime();
                let tick_rate = metrics.tick_rate();
                let players = metrics.player_count.load(Ordering::Relaxed);
                let peak = metrics.peak_player_count.load(Ordering::Relaxed);
                let chunks = metrics.chunk_count.load(Ordering::Relaxed);
                let memory_mb = metrics.memory_bytes.load(Ordering::Relaxed) as f64
                    / (1024.0 * 1024.0);
                let bytes_rx = metrics.bytes_received.load(Ordering::Relaxed);
                let bytes_tx = metrics.bytes_sent.load(Ordering::Relaxed);
                let disconnects = metrics.recent_disconnects.swap(0, Ordering::Relaxed);
                let overruns = metrics.recent_overruns.swap(0, Ordering::Relaxed);

                tracing::info!(
                    uptime_secs = uptime.as_secs(),
                    tick_rate = format!("{tick_rate:.1}"),
                    players,
                    peak_players = peak,
                    chunks,
                    memory_mb = format!("{memory_mb:.1}"),
                    bandwidth_rx_kb = bytes_rx / 1024,
                    bandwidth_tx_kb = bytes_tx / 1024,
                    disconnects,
                    overruns,
                    "Server metrics"
                );

                // Check alert conditions
                check_alerts(&metrics, tick_rate, memory_mb, disconnects);
            }
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
        }
    }

    tracing::debug!("Monitoring loop exited");
}
```

### Alert Conditions

```rust
const TICK_RATE_ALERT_THRESHOLD: f64 = 50.0; // Alert if tick rate drops below 50
const MEMORY_ALERT_MB: f64 = 8192.0;          // Alert at 8 GB
const DISCONNECT_SPIKE_THRESHOLD: u64 = 5;    // Alert if 5+ disconnects in one interval

fn check_alerts(
    metrics: &ServerMetrics,
    tick_rate: f64,
    memory_mb: f64,
    disconnects: u64,
) {
    if tick_rate > 0.0 && tick_rate < TICK_RATE_ALERT_THRESHOLD {
        tracing::warn!(
            tick_rate = format!("{tick_rate:.1}"),
            threshold = TICK_RATE_ALERT_THRESHOLD,
            "ALERT: Tick rate below threshold"
        );
    }

    if memory_mb > MEMORY_ALERT_MB {
        tracing::warn!(
            memory_mb = format!("{memory_mb:.1}"),
            threshold = MEMORY_ALERT_MB,
            "ALERT: Memory usage above threshold"
        );
    }

    if disconnects >= DISCONNECT_SPIKE_THRESHOLD {
        tracing::warn!(
            disconnects,
            threshold = DISCONNECT_SPIKE_THRESHOLD,
            "ALERT: Disconnect spike detected"
        );
    }
}
```

### HTTP Metrics Endpoint (Optional)

An optional HTTP endpoint serves metrics as JSON for external monitoring tools. It uses a raw `tokio::net::TcpListener` with hand-written HTTP response formatting — no web framework dependency:

```rust
use tokio::io::AsyncWriteExt;

pub async fn run_metrics_http_server(
    metrics: Arc<ServerMetrics>,
    bind_addr: std::net::SocketAddr,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let listener = match tokio::net::TcpListener::bind(bind_addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("Failed to bind metrics HTTP server on {bind_addr}: {e}");
            return;
        }
    };

    tracing::info!("Metrics HTTP endpoint listening on {bind_addr}");

    loop {
        tokio::select! {
            result = listener.accept() => {
                if let Ok((mut stream, _)) = result {
                    let metrics = Arc::clone(&metrics);
                    tokio::spawn(async move {
                        // Read the HTTP request (we only care that it exists)
                        let mut buf = [0u8; 1024];
                        let _ = tokio::io::AsyncReadExt::read(&mut stream, &mut buf).await;

                        // Build JSON response
                        let json = build_metrics_json(&metrics).await;
                        let response = format!(
                            "HTTP/1.1 200 OK\r\n\
                             Content-Type: application/json\r\n\
                             Content-Length: {}\r\n\
                             Connection: close\r\n\
                             \r\n\
                             {}",
                            json.len(),
                            json,
                        );

                        let _ = stream.write_all(response.as_bytes()).await;
                    });
                }
            }
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
        }
    }
}

async fn build_metrics_json(metrics: &ServerMetrics) -> String {
    let system_times = metrics.system_times.read().await;
    let systems: Vec<String> = system_times
        .iter()
        .map(|(name, us)| format!("    \"{name}\": {us}"))
        .collect();

    format!(
        r#"{{
  "uptime_secs": {},
  "tick_rate": {:.1},
  "tick_count": {},
  "player_count": {},
  "peak_player_count": {},
  "chunk_count": {},
  "memory_bytes": {},
  "bytes_received": {},
  "bytes_sent": {},
  "system_times_us": {{
{}
  }}
}}"#,
        metrics.uptime().as_secs(),
        metrics.tick_rate(),
        metrics.tick_count.load(Ordering::Relaxed),
        metrics.player_count.load(Ordering::Relaxed),
        metrics.peak_player_count.load(Ordering::Relaxed),
        metrics.chunk_count.load(Ordering::Relaxed),
        metrics.memory_bytes.load(Ordering::Relaxed),
        metrics.bytes_received.load(Ordering::Relaxed),
        metrics.bytes_sent.load(Ordering::Relaxed),
        systems.join(",\n"),
    )
}
```

The endpoint listens on a separate port (default: 7778) from the game server (7777). A `curl http://localhost:7778/` returns the JSON metrics blob. This is intentionally minimal — a full Prometheus exporter can be added later as a subscriber layer.

### Integration with Tick Loop

The metrics object is shared via `Arc<ServerMetrics>` between the tick loop, the monitoring log task, and the HTTP endpoint:

```rust
// In server startup (story 01)
let metrics = Arc::new(ServerMetrics::new());

// Pass to tick loop
let tick_metrics = Arc::clone(&metrics);

// Spawn monitoring log task
let log_metrics = Arc::clone(&metrics);
tokio::spawn(run_monitoring_loop(log_metrics, 30, shutdown_rx.clone()));

// Optionally spawn HTTP metrics server
if config.metrics_port.is_some() {
    let http_metrics = Arc::clone(&metrics);
    let addr = format!("0.0.0.0:{}", config.metrics_port.unwrap())
        .parse()
        .unwrap();
    tokio::spawn(run_metrics_http_server(http_metrics, addr, shutdown_rx.clone()));
}
```

## Outcome

A `monitoring.rs` module in `crates/nebula-server/src/` exporting `ServerMetrics`, `TickRateMeasurer`, `run_monitoring_loop`, and `run_metrics_http_server`. Metrics are recorded atomically during the tick loop and logged every 30 seconds with structured tracing fields. An optional HTTP endpoint on port 7778 serves metrics as JSON for external monitoring tools. Alerts are triggered when the tick rate drops below 50 tps, memory exceeds 8 GB, or 5+ players disconnect in a single monitoring interval. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

A monitoring endpoint exports metrics: tick rate, player count, memory usage, bandwidth, and chunk count. The format is compatible with Prometheus/Grafana monitoring stacks.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `tokio` | `1.49` (features: `time`, `sync`, `net`, `io-util`) | Interval timer for periodic logging, TCP listener for HTTP endpoint, async I/O for HTTP responses |
| `tracing` | `0.1` | Structured logging of metrics and alerts |
| `serde` | `1.0` (features: `derive`) | Potential future use for metrics serialization (currently hand-built JSON) |

No web framework is added. The HTTP endpoint uses raw `tokio::net::TcpListener` with hand-formatted HTTP/1.1 responses to avoid pulling in `hyper`, `axum`, `actix`, or any other web dependency.

## Unit Tests

- **`test_metrics_are_recorded`** — Create a `ServerMetrics` instance. Set `tick_count` to 100, `player_count` to 5, `chunk_count` to 1000. Read them back and assert the values match. This validates the atomic read/write path.

- **`test_tick_rate_measurer_after_one_second`** — Create a `TickRateMeasurer`. Call `record_tick()` 60 times with simulated 1-second elapsed time. Assert the returned tick rate is approximately 60.0 (within 5% tolerance).

- **`test_log_output_contains_metrics`** — Set up a tracing subscriber that writes to a `Vec<u8>` buffer. Run one iteration of the monitoring log. Assert the captured output contains the strings `"tick_rate"`, `"players"`, `"memory_mb"`, and `"Server metrics"`.

- **`test_http_endpoint_responds_with_json`** — Spawn `run_metrics_http_server` on an ephemeral port. Send an HTTP GET request using a raw `TcpStream`. Read the response. Assert the status line contains `"200 OK"`. Parse the body as JSON. Assert the JSON contains keys `"uptime_secs"`, `"tick_rate"`, `"player_count"`, `"chunk_count"`, `"memory_bytes"`.

- **`test_tick_rate_drop_triggers_alert`** — Create a `ServerMetrics` with tick rate set to 40.0 (below `TICK_RATE_ALERT_THRESHOLD` of 50.0). Set up a tracing subscriber that captures warnings. Call `check_alerts`. Assert the captured output contains `"ALERT: Tick rate below threshold"`.

- **`test_memory_threshold_triggers_alert`** — Call `check_alerts` with `memory_mb = 10000.0` (above `MEMORY_ALERT_MB` of 8192.0). Assert the warning log contains `"ALERT: Memory usage above threshold"`.

- **`test_disconnect_spike_triggers_alert`** — Call `check_alerts` with `disconnects = 10` (above `DISCONNECT_SPIKE_THRESHOLD` of 5). Assert the warning log contains `"ALERT: Disconnect spike detected"`.

- **`test_metrics_update_each_interval`** — Spawn the monitoring loop with a 1-second interval (for testing). Update `player_count` from 0 to 5 after 500ms. Wait for the next log interval. Assert the logged player count is 5, not 0.

- **`test_peak_player_count_tracks_maximum`** — Call `update_player_count(10)`, then `update_player_count(5)`, then `update_player_count(8)`. Assert `peak_player_count` is 10 (the historical maximum, not the current value).

- **`test_recent_disconnects_resets_after_read`** — Set `recent_disconnects` to 3. Read it with `swap(0, ...)` (as the monitoring loop does). Assert the returned value is 3. Read again. Assert the returned value is 0. This validates that disconnect counts reset each monitoring interval.
