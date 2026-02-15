# Network Stats Overlay

## Problem

Multiplayer networking is inherently unreliable: packets are lost, delayed, reordered, or arrive in bursts. When multiplayer gameplay feels "off" — rubber-banding, delayed actions, entity teleporting, desyncs — the root cause could be anywhere in the networking stack: high latency, packet loss, bandwidth saturation, server overload, or client-side prediction divergence. Without real-time network statistics visible during gameplay:

- **Players cannot distinguish client-side bugs from network issues** — A "laggy" experience might be the server running slowly (low tick rate), the network dropping packets (packet loss), or the client's prediction code diverging (client-side bug). Each has a different fix.
- **Bandwidth saturation is invisible** — If the server is sending more data than the player's connection can handle, packets queue up and latency spikes. Seeing the bytes-per-second in real time reveals when the connection is saturated.
- **Intermittent issues are undiagnosable** — A 2-second latency spike that happens every 30 seconds is impossible to catch without a rolling graph. By the time the developer notices the stutter, the moment has passed.
- **Server performance problems are projected onto the client** — If the server tick rate drops from 60 to 20, the client experiences it as "lag," but the actual problem is server-side. Displaying the server tick rate separates server performance from network quality.

## Solution

### Network Statistics Resource

A `NetworkStats` resource is updated by the networking layer each frame:

```rust
use std::collections::VecDeque;
use std::time::{Duration, Instant};

pub struct NetworkStats {
    /// Round-trip time (ping) in milliseconds
    pub rtt_ms: f64,
    /// Rolling history of RTT values for the graph
    pub rtt_history: VecDeque<(Instant, f64)>,
    /// Upload bandwidth in bytes per second
    pub bandwidth_up_bps: u64,
    /// Download bandwidth in bytes per second
    pub bandwidth_down_bps: u64,
    /// Packet loss percentage (0.0 - 100.0), rolling window
    pub packet_loss_percent: f64,
    /// Packets sent/received counters for loss calculation
    packets_sent: u64,
    packets_acked: u64,
    /// Server's simulation tick rate (reported by server)
    pub server_tick_rate: f32,
    /// Client's simulation tick rate
    pub client_tick_rate: f32,
    /// Number of entities currently being replicated from server
    pub replicated_entity_count: u32,
    /// Whether the overlay is visible (only in multiplayer)
    pub visible: bool,
    /// Connection quality assessment
    pub quality: ConnectionQuality,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ConnectionQuality {
    Excellent, // RTT < 50ms, loss < 1%
    Good,      // RTT < 100ms, loss < 3%
    Fair,      // RTT < 200ms, loss < 5%
    Poor,      // RTT < 500ms, loss < 10%
    Critical,  // RTT >= 500ms or loss >= 10%
}
```

### RTT Measurement

Ping is measured using the engine's heartbeat system. The client sends a timestamped heartbeat packet every 500ms. When the server echoes it back, the client computes the round-trip time:

```rust
fn update_rtt(&mut self, send_time: Instant) {
    let rtt = send_time.elapsed();
    self.rtt_ms = rtt.as_secs_f64() * 1000.0;
    self.rtt_history.push_back((Instant::now(), self.rtt_ms));

    // Keep only the last 60 seconds of history
    let cutoff = Instant::now() - Duration::from_secs(60);
    while self.rtt_history.front().is_some_and(|(t, _)| *t < cutoff) {
        self.rtt_history.pop_front();
    }
}
```

### Bandwidth Tracking

Bandwidth is measured by counting bytes sent and received within a sliding 1-second window:

```rust
pub struct BandwidthTracker {
    upload_bytes: VecDeque<(Instant, usize)>,
    download_bytes: VecDeque<(Instant, usize)>,
}

impl BandwidthTracker {
    pub fn record_upload(&mut self, bytes: usize) {
        self.upload_bytes.push_back((Instant::now(), bytes));
    }

    pub fn record_download(&mut self, bytes: usize) {
        self.download_bytes.push_back((Instant::now(), bytes));
    }

    pub fn upload_bps(&mut self) -> u64 {
        let cutoff = Instant::now() - Duration::from_secs(1);
        self.upload_bytes.retain(|(t, _)| *t >= cutoff);
        self.upload_bytes.iter().map(|(_, b)| *b as u64).sum()
    }

    pub fn download_bps(&mut self) -> u64 {
        let cutoff = Instant::now() - Duration::from_secs(1);
        self.download_bytes.retain(|(t, _)| *t >= cutoff);
        self.download_bytes.iter().map(|(_, b)| *b as u64).sum()
    }
}
```

### Packet Loss Calculation

Packet loss is computed over a rolling window of the last 100 packets. Each packet has a sequence number. The client tracks which sequence numbers were acknowledged. Missing acknowledgments within the window are counted as lost:

```rust
fn compute_packet_loss(&self) -> f64 {
    if self.packets_sent == 0 {
        return 0.0;
    }
    let lost = self.packets_sent.saturating_sub(self.packets_acked);
    (lost as f64 / self.packets_sent as f64) * 100.0
}
```

### Connection Quality Assessment

The `ConnectionQuality` enum is computed from RTT and packet loss:

```rust
fn assess_quality(rtt_ms: f64, loss_percent: f64) -> ConnectionQuality {
    if rtt_ms >= 500.0 || loss_percent >= 10.0 {
        ConnectionQuality::Critical
    } else if rtt_ms >= 200.0 || loss_percent >= 5.0 {
        ConnectionQuality::Poor
    } else if rtt_ms >= 100.0 || loss_percent >= 3.0 {
        ConnectionQuality::Fair
    } else if rtt_ms >= 50.0 || loss_percent >= 1.0 {
        ConnectionQuality::Good
    } else {
        ConnectionQuality::Excellent
    }
}
```

### Overlay Rendering

The overlay is positioned in the top-right corner and includes both text statistics and a rolling RTT graph:

```rust
fn draw_network_overlay(
    mut egui_ctx: ResMut<EguiContext>,
    stats: Res<NetworkStats>,
    is_multiplayer: Res<MultiplayerState>,
) {
    // Only show in multiplayer mode
    if !is_multiplayer.connected || !stats.visible {
        return;
    }

    let quality_color = match stats.quality {
        ConnectionQuality::Excellent => egui::Color32::from_rgb(80, 220, 80),
        ConnectionQuality::Good => egui::Color32::from_rgb(150, 220, 80),
        ConnectionQuality::Fair => egui::Color32::from_rgb(220, 200, 50),
        ConnectionQuality::Poor => egui::Color32::from_rgb(220, 130, 50),
        ConnectionQuality::Critical => egui::Color32::from_rgb(220, 50, 50),
    };

    egui::Area::new(egui::Id::new("net_overlay"))
        .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-8.0, 8.0))
        .show(egui_ctx.get_mut(), |ui| {
            egui::Frame::NONE
                .fill(egui::Color32::from_black_alpha(180))
                .corner_radius(4.0)
                .inner_margin(egui::Margin::same(6))
                .show(ui, |ui| {
                    ui.style_mut().override_text_style = Some(egui::TextStyle::Monospace);

                    ui.colored_label(quality_color, format!(
                        "Ping: {:.0} ms ({:?})", stats.rtt_ms, stats.quality
                    ));

                    // Bandwidth with human-readable formatting
                    ui.label(format!(
                        "Up: {}  Down: {}",
                        format_bytes(stats.bandwidth_up_bps),
                        format_bytes(stats.bandwidth_down_bps),
                    ));

                    ui.label(format!("Loss: {:.1}%", stats.packet_loss_percent));

                    ui.label(format!(
                        "Ticks: Server {:.0}/s  Client {:.0}/s",
                        stats.server_tick_rate, stats.client_tick_rate
                    ));

                    ui.label(format!("Entities: {}", stats.replicated_entity_count));

                    ui.separator();

                    // RTT graph (last 30 seconds)
                    draw_rtt_graph(ui, &stats.rtt_history);

                    // Warning indicators
                    if stats.quality == ConnectionQuality::Poor
                        || stats.quality == ConnectionQuality::Critical
                    {
                        ui.separator();
                        ui.colored_label(
                            egui::Color32::from_rgb(220, 50, 50),
                            "WARNING: Poor connection detected",
                        );
                    }
                });
        });
}

fn draw_rtt_graph(ui: &mut egui::Ui, history: &VecDeque<(Instant, f64)>) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(200.0, 60.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);

    // Background
    painter.rect_filled(rect, 2.0, egui::Color32::from_black_alpha(100));

    if history.len() < 2 {
        return;
    }

    // Find max RTT for scaling
    let max_rtt = history.iter().map(|(_, v)| *v).fold(0.0_f64, f64::max).max(100.0);

    // Plot points as a line graph
    let now = Instant::now();
    let window = Duration::from_secs(30);
    let points: Vec<egui::Pos2> = history
        .iter()
        .filter(|(t, _)| now.duration_since(*t) < window)
        .map(|(t, rtt)| {
            let x_frac = 1.0 - now.duration_since(*t).as_secs_f32() / window.as_secs_f32();
            let y_frac = 1.0 - (*rtt as f32 / max_rtt as f32);
            egui::pos2(
                rect.left() + x_frac * rect.width(),
                rect.top() + y_frac * rect.height(),
            )
        })
        .collect();

    for pair in points.windows(2) {
        painter.line_segment(
            [pair[0], pair[1]],
            egui::Stroke::new(1.5, egui::Color32::from_rgb(80, 180, 220)),
        );
    }
}

fn format_bytes(bytes_per_sec: u64) -> String {
    if bytes_per_sec >= 1_000_000 {
        format!("{:.1} MB/s", bytes_per_sec as f64 / 1_000_000.0)
    } else if bytes_per_sec >= 1_000 {
        format!("{:.1} KB/s", bytes_per_sec as f64 / 1_000.0)
    } else {
        format!("{} B/s", bytes_per_sec)
    }
}
```

### Visibility Logic

The overlay is automatically hidden in single-player mode. In multiplayer, it can be toggled with F8. The system checks `MultiplayerState::connected` before rendering.

## Outcome

In multiplayer sessions, a top-right overlay displays: ping with color-coded connection quality, upload/download bandwidth in human-readable units, packet loss percentage, server and client tick rates, and replicated entity count. A rolling line graph shows RTT over the last 30 seconds. Warning indicators appear when connection quality drops to Poor or Critical. The overlay is invisible in single-player mode and togglable with F8 in multiplayer. Implementation lives in `crates/nebula-debug/src/network_stats.rs` with data fed from `nebula-net` and `nebula-multiplayer`.

## Demo Integration

**Demo crate:** `nebula-demo`

A panel shows network diagnostics: ping, jitter, packet loss %, bytes sent/received per second, message counts by type, and replication entity count.

## Crates & Dependencies

- **`egui = "0.31"`** — Overlay rendering: text labels, RTT line graph with `painter.line_segment()`, color-coded quality indicators, and anchored positioning.
- **`tracing = "0.1"`** — Logging connection quality transitions (e.g., "Connection quality changed: Good -> Poor") and network warning events.

Internal crate dependencies (not external):
- `nebula-net` for raw bandwidth and packet counters.
- `nebula-multiplayer` for `MultiplayerState`, server tick rate, and entity replication count.

## Unit Tests

- **`test_ping_displays_rtt_value`** — Create a `NetworkStats` instance. Call `update_rtt` with a simulated 45ms round-trip (by using an `Instant` from 45ms ago). Assert `rtt_ms` is approximately `45.0` (within 5ms tolerance for timer precision). Assert `rtt_history` contains one entry.

- **`test_bandwidth_shows_bytes_per_sec`** — Create a `BandwidthTracker`. Call `record_upload(1000)` ten times within the same second. Assert `upload_bps()` returns `10000`. Wait 1.1 seconds (simulated by adjusting timestamps) and assert `upload_bps()` returns `0` (old data pruned).

- **`test_packet_loss_percentage_accurate`** — Set `packets_sent = 100` and `packets_acked = 95`. Assert `compute_packet_loss()` returns `5.0`. Set both to 0 and assert it returns `0.0` (no division by zero). Set `packets_sent = 100` and `packets_acked = 100` and assert `0.0`.

- **`test_graph_updates_over_time`** — Push 10 RTT values into `rtt_history` with timestamps spread over 5 seconds. Assert `rtt_history.len() == 10`. Push values with timestamps older than 60 seconds. Call `update_rtt` to trigger pruning and assert old entries are removed.

- **`test_overlay_hidden_in_single_player`** — Create a `MultiplayerState` with `connected: false`. Assert the rendering function returns early without drawing. Set `connected: true` and assert the function proceeds to draw.

- **`test_connection_quality_thresholds`** — Assert:
  - `assess_quality(30.0, 0.5)` returns `Excellent`.
  - `assess_quality(75.0, 0.5)` returns `Good`.
  - `assess_quality(150.0, 2.0)` returns `Fair`.
  - `assess_quality(300.0, 6.0)` returns `Poor`.
  - `assess_quality(600.0, 1.0)` returns `Critical`.
  - `assess_quality(50.0, 15.0)` returns `Critical` (loss alone triggers Critical).

- **`test_format_bytes_human_readable`** — Assert:
  - `format_bytes(500)` returns `"500 B/s"`.
  - `format_bytes(1500)` returns `"1.5 KB/s"`.
  - `format_bytes(2_500_000)` returns `"2.5 MB/s"`.

- **`test_rtt_history_prunes_old_entries`** — Insert 200 RTT entries with timestamps spanning 120 seconds. Call the pruning logic (60-second window). Assert the history contains only entries from the last 60 seconds and the count is less than 200.
