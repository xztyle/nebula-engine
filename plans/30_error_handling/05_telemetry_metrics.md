# Telemetry Metrics

## Problem

Structured logging (story 04) captures individual events, but it does not provide the continuous, aggregate view that engine developers need for performance tuning and monitoring. Knowing that "frame 12847 took 22ms" is useful, but what is really needed is:

- **Is the frame time trending upward?** -- A histogram of the last 60 seconds of frame times reveals spikes, trends, and distribution (are most frames 8ms with occasional 30ms spikes, or is the engine consistently at 16ms?).
- **Which system is the bottleneck?** -- Per-system duration metrics (physics took 2.1ms, meshing took 5.3ms, rendering took 8.0ms) identify where optimization effort should be focused.
- **How fast are chunks loading?** -- The chunk load rate (chunks/second) and the chunk queue depth indicate whether terrain generation is keeping up with the player's movement speed.
- **Is memory growing unboundedly?** -- Tracking memory usage over time reveals leaks that would be invisible from a single snapshot.
- **How much bandwidth is the network using?** -- Bytes sent/received per second determines whether the multiplayer protocol is efficient enough for the target connection speed.

Without a metrics system, developers resort to ad-hoc `println!` timing, manually computing averages in their head, and being unable to correlate performance regressions with specific changes. A structured telemetry system stores time-series data in memory, exposes it through the debug UI, and optionally exports it for offline analysis.

## Solution

### Metric Types

Define three fundamental metric types that cover all engine telemetry needs:

```rust
/// A single scalar value that can go up or down (e.g., entity count, memory usage).
pub struct Gauge {
    name: &'static str,
    current: f64,
    history: RingBuffer<TimestampedValue>,
}

/// A monotonically increasing counter (e.g., total frames rendered, total bytes sent).
/// The rate of change is computed from the history.
pub struct Counter {
    name: &'static str,
    total: u64,
    history: RingBuffer<TimestampedValue>,
}

/// A distribution of values over a time window (e.g., frame time distribution).
pub struct Histogram {
    name: &'static str,
    buckets: Vec<HistogramBucket>,
    current_window: Vec<f64>,
    history: RingBuffer<HistogramSnapshot>,
}

#[derive(Clone, Copy)]
struct TimestampedValue {
    timestamp_secs: f64,
    value: f64,
}

#[derive(Clone)]
struct HistogramSnapshot {
    timestamp_secs: f64,
    min: f64,
    max: f64,
    mean: f64,
    p50: f64,
    p95: f64,
    p99: f64,
    count: usize,
}

struct HistogramBucket {
    le: f64,   // upper bound (less-than-or-equal)
    count: u64,
}
```

### Ring Buffer

A fixed-size ring buffer that stores the last N entries, overwriting the oldest when full. Used for all metric history storage:

```rust
pub struct RingBuffer<T> {
    data: Vec<T>,
    capacity: usize,
    head: usize,  // Next write position
    len: usize,   // Current number of valid entries
}

impl<T: Clone + Default> RingBuffer<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            data: vec![T::default(); capacity],
            capacity,
            head: 0,
            len: 0,
        }
    }

    pub fn push(&mut self, value: T) {
        self.data[self.head] = value;
        self.head = (self.head + 1) % self.capacity;
        if self.len < self.capacity {
            self.len += 1;
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        let start = if self.len < self.capacity {
            0
        } else {
            self.head
        };
        (0..self.len).map(move |i| {
            let idx = (start + i) % self.capacity;
            &self.data[idx]
        })
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn is_full(&self) -> bool {
        self.len == self.capacity
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Get the most recent entry.
    pub fn latest(&self) -> Option<&T> {
        if self.len == 0 {
            return None;
        }
        let idx = if self.head == 0 { self.capacity - 1 } else { self.head - 1 };
        Some(&self.data[idx])
    }
}
```

### MetricsRegistry

A central registry that owns all metrics and provides recording and querying APIs:

```rust
use std::collections::HashMap;
use std::time::Instant;

pub struct MetricsRegistry {
    gauges: HashMap<&'static str, Gauge>,
    counters: HashMap<&'static str, Counter>,
    histograms: HashMap<&'static str, Histogram>,
    start_time: Instant,
    snapshot_interval_secs: f64,
    last_snapshot: f64,
}

impl MetricsRegistry {
    pub fn new() -> Self {
        Self {
            gauges: HashMap::new(),
            counters: HashMap::new(),
            histograms: HashMap::new(),
            start_time: Instant::now(),
            snapshot_interval_secs: 1.0,  // 1-second granularity
            last_snapshot: 0.0,
        }
    }

    /// Record a gauge value (replaces the current value).
    pub fn set_gauge(&mut self, name: &'static str, value: f64) {
        let now = self.elapsed_secs();
        let gauge = self.gauges.entry(name).or_insert_with(|| Gauge {
            name,
            current: 0.0,
            history: RingBuffer::new(60), // 60 seconds of history
        });
        gauge.current = value;
    }

    /// Increment a counter by the given amount.
    pub fn increment_counter(&mut self, name: &'static str, amount: u64) {
        let counter = self.counters.entry(name).or_insert_with(|| Counter {
            name,
            total: 0,
            history: RingBuffer::new(60),
        });
        counter.total += amount;
    }

    /// Record a histogram observation (e.g., a single frame time).
    pub fn observe_histogram(&mut self, name: &'static str, value: f64) {
        let histogram = self.histograms.entry(name).or_insert_with(|| Histogram {
            name,
            buckets: default_histogram_buckets(),
            current_window: Vec::new(),
            history: RingBuffer::new(60),
        });
        histogram.current_window.push(value);

        // Update bucket counts
        for bucket in &mut histogram.buckets {
            if value <= bucket.le {
                bucket.count += 1;
            }
        }
    }

    /// Called once per second (or at the configured interval) to snapshot
    /// current values into the history ring buffers.
    pub fn snapshot(&mut self) {
        let now = self.elapsed_secs();

        // Snapshot gauges
        for gauge in self.gauges.values_mut() {
            gauge.history.push(TimestampedValue {
                timestamp_secs: now,
                value: gauge.current,
            });
        }

        // Snapshot counters
        for counter in self.counters.values_mut() {
            counter.history.push(TimestampedValue {
                timestamp_secs: now,
                value: counter.total as f64,
            });
        }

        // Snapshot histograms (compute percentiles from current window)
        for histogram in self.histograms.values_mut() {
            if !histogram.current_window.is_empty() {
                let mut sorted = histogram.current_window.clone();
                sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());

                let snapshot = HistogramSnapshot {
                    timestamp_secs: now,
                    min: sorted[0],
                    max: *sorted.last().unwrap(),
                    mean: sorted.iter().sum::<f64>() / sorted.len() as f64,
                    p50: percentile(&sorted, 50.0),
                    p95: percentile(&sorted, 95.0),
                    p99: percentile(&sorted, 99.0),
                    count: sorted.len(),
                };
                histogram.history.push(snapshot);
                histogram.current_window.clear();
            }
        }

        self.last_snapshot = now;
    }

    /// Query gauge history within a time range.
    pub fn query_gauge(&self, name: &str, from_secs: f64, to_secs: f64) -> Vec<TimestampedValue> {
        self.gauges.get(name)
            .map(|g| {
                g.history.iter()
                    .filter(|v| v.timestamp_secs >= from_secs && v.timestamp_secs <= to_secs)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    fn elapsed_secs(&self) -> f64 {
        self.start_time.elapsed().as_secs_f64()
    }
}

fn default_histogram_buckets() -> Vec<HistogramBucket> {
    // Frame-time-oriented buckets (in milliseconds)
    [1.0, 2.0, 4.0, 8.0, 16.0, 33.0, 50.0, 100.0, 250.0, 500.0, 1000.0]
        .iter()
        .map(|&le| HistogramBucket { le, count: 0 })
        .collect()
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((p / 100.0) * (sorted.len() - 1) as f64).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}
```

### Standard Engine Metrics

The following metrics are collected by default across the engine:

| Metric Name | Type | Source Crate | Description |
|-------------|------|-------------|-------------|
| `frame_time_ms` | Histogram | `nebula-app` | Time to produce each frame (ms) |
| `fps` | Gauge | `nebula-app` | Frames per second (computed) |
| `tick_time_ms` | Histogram | `nebula-app` | Time for each simulation tick (ms) |
| `physics_step_ms` | Histogram | `nebula-physics` | Physics world step duration (ms) |
| `mesh_generation_ms` | Histogram | `nebula-mesh` | Per-chunk mesh generation time (ms) |
| `chunks_loaded` | Counter | `nebula-voxel` | Total chunks loaded since start |
| `chunks_unloaded` | Counter | `nebula-voxel` | Total chunks unloaded since start |
| `active_chunks` | Gauge | `nebula-voxel` | Currently loaded chunks |
| `entity_count` | Gauge | `nebula-ecs` | Total active entities |
| `memory_used_mb` | Gauge | `nebula-app` | Process memory usage (MB) |
| `net_bytes_sent` | Counter | `nebula-net` | Total bytes sent |
| `net_bytes_received` | Counter | `nebula-net` | Total bytes received |
| `draw_calls` | Gauge | `nebula-render` | Draw calls per frame |
| `vertex_count` | Gauge | `nebula-render` | Vertices submitted per frame |

### Debug UI Panel

An egui panel displays metrics in real time. The panel shows:

- A frame time graph (last 60 seconds, plotted from the `frame_time_ms` histogram history).
- Per-system duration bars (physics, meshing, rendering, etc.).
- Chunk load/unload rates (computed from counter deltas between snapshots).
- Entity count and memory usage gauges.
- Network bandwidth (bytes/sec computed from counter rate of change).

```rust
pub fn draw_metrics_panel(ui: &mut egui::Ui, metrics: &MetricsRegistry) {
    ui.heading("Engine Metrics");

    // Frame time graph
    if let Some(histogram) = metrics.histograms.get("frame_time_ms") {
        let points: Vec<[f64; 2]> = histogram.history.iter()
            .map(|s| [s.timestamp_secs, s.mean])
            .collect();
        // Draw as egui plot...
    }

    // System durations
    ui.label(format!(
        "Physics: {:.1}ms | Mesh: {:.1}ms | Render: {:.1}ms",
        metrics.latest_histogram_mean("physics_step_ms"),
        metrics.latest_histogram_mean("mesh_generation_ms"),
        metrics.latest_histogram_mean("frame_time_ms"),
    ));

    // Entity count
    if let Some(gauge) = metrics.gauges.get("entity_count") {
        ui.label(format!("Entities: {}", gauge.current as u64));
    }
}
```

### File Export

Metrics can be exported to a CSV or JSON file for offline analysis:

```rust
pub fn export_metrics_csv(metrics: &MetricsRegistry, path: &Path) -> std::io::Result<()> {
    let mut file = std::fs::File::create(path)?;
    writeln!(file, "timestamp_secs,metric_name,value")?;

    for (name, gauge) in &metrics.gauges {
        for entry in gauge.history.iter() {
            writeln!(file, "{},{},{}", entry.timestamp_secs, name, entry.value)?;
        }
    }

    for (name, counter) in &metrics.counters {
        for entry in counter.history.iter() {
            writeln!(file, "{},{},{}", entry.timestamp_secs, name, entry.value)?;
        }
    }

    Ok(())
}
```

## Outcome

The engine collects continuous telemetry metrics across all subsystems: frame time histograms, per-system durations, chunk load/unload rates, entity counts, memory usage, and network bandwidth. Metrics are stored in ring buffers holding the last 60 seconds at 1-second granularity. An egui debug panel displays metrics in real time with graphs and summary statistics. Metrics can be exported to CSV for offline analysis in tools like Python/matplotlib or spreadsheets. Developers can immediately see where frame time is being spent and identify performance regressions.

## Demo Integration

**Demo crate:** `nebula-demo`

A metrics overlay shows: errors per minute, recovery count, warning rate, and system disable count. The data is also written to a metrics file for offline analysis.

## Crates & Dependencies

- **`tracing = "0.1"`** -- Used indirectly; metric collection happens alongside tracing events but uses its own storage rather than the tracing subscriber. Metric recording points often coincide with `#[instrument]` spans.
- **`tracing-subscriber = "0.3"`** -- Not directly used by the metrics system, but the metrics system complements the structured logging from story 04.

No additional external crates are strictly required for the core metrics implementation. The ring buffer and metric types are pure Rust. The egui debug panel uses the existing `egui` dependency from `nebula-ui`/`nebula-debug`. The CSV export uses `std::io::Write`.

## Unit Tests

- **`test_gauge_records_value`** -- Create a `MetricsRegistry`. Call `set_gauge("entity_count", 100.0)`. Call `snapshot()`. Call `set_gauge("entity_count", 150.0)`. Call `snapshot()`. Query the gauge history and assert it contains two entries with values 100.0 and 150.0.

- **`test_counter_increments`** -- Create a `MetricsRegistry`. Call `increment_counter("chunks_loaded", 5)` three times. Assert the counter's total is 15. Call `snapshot()` and verify the history entry records 15.

- **`test_ring_buffer_wraps_correctly`** -- Create a `RingBuffer<i32>` with capacity 5. Push values 1 through 8. Assert `len()` is 5 (capacity). Assert `is_full()` is `true`. Iterate over the buffer and assert the values are `[4, 5, 6, 7, 8]` (the oldest 3 were overwritten). Assert `latest()` returns `Some(&8)`.

- **`test_ring_buffer_partial_fill`** -- Create a `RingBuffer<i32>` with capacity 10. Push values 1, 2, 3. Assert `len()` is 3. Assert `is_full()` is `false`. Iterate and assert values are `[1, 2, 3]`.

- **`test_histogram_tracks_distribution`** -- Create a `MetricsRegistry`. Observe the histogram "frame_time_ms" with values `[8.0, 9.0, 10.0, 16.0, 33.0, 8.5, 9.5]`. Call `snapshot()`. Retrieve the latest histogram snapshot. Assert `min` is 8.0, `max` is 33.0. Assert `mean` is approximately 13.4. Assert `p50` is approximately 9.5. Assert `count` is 7.

- **`test_metrics_queryable_by_time_range`** -- Create a `MetricsRegistry`. Record gauge values at different timestamps by calling `set_gauge` and `snapshot` multiple times with controlled timing. Query with `query_gauge("test", from, to)` for a subset of the time range. Assert only entries within the range are returned.

- **`test_export_produces_valid_csv`** -- Create a `MetricsRegistry` with a gauge and a counter, each with a few snapshots. Call `export_metrics_csv` to a temporary file. Read the file and parse it as CSV. Assert the header line is `"timestamp_secs,metric_name,value"`. Assert each data line has three comma-separated fields. Assert the metric names match what was recorded.

- **`test_histogram_buckets`** -- Create a histogram with default buckets. Observe values `[1.0, 5.0, 10.0, 20.0, 50.0]`. Verify bucket counts: the `le=1.0` bucket has count 1, the `le=8.0` bucket has count 2, the `le=16.0` bucket has count 3, the `le=33.0` bucket has count 4, the `le=50.0` bucket has count 5.

- **`test_snapshot_clears_histogram_window`** -- Observe 10 histogram values. Call `snapshot()`. Assert the current window is empty (length 0). Observe 5 more values. Call `snapshot()`. Assert the second snapshot's `count` is 5, not 15.
