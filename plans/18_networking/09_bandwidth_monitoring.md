# Bandwidth Monitoring

## Problem

Without visibility into network usage, the engine cannot detect bandwidth problems, tune update rates, or give server administrators useful diagnostics. The Nebula Engine streams chunk data for cubesphere-voxel planets, broadcasts entity updates for all players in view, and sends heartbeats — all of which consume bandwidth. If a player is on a slow connection, the server needs to know so it can throttle updates. If the server is saturating its uplink, an administrator needs to see it before players start lagging. The engine must track bytes sent/received per second, messages per second, and per-message-type breakdowns. Both raw (pre-compression) and wire (post-compression) byte counts must be tracked to measure compression effectiveness (story 07). These stats are exposed to the debug overlay (Epic 28) and used internally for adaptive quality decisions.

## Solution

### Network stats resource

`NetworkStats` is a resource updated once per second. It stores the current and previous second's counters, allowing the game to read consistent per-second rates.

```rust
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Live counters incremented by the network tasks.
pub struct NetworkCounters {
    pub bytes_sent: AtomicU64,
    pub bytes_received: AtomicU64,
    pub bytes_sent_raw: AtomicU64,
    pub bytes_received_raw: AtomicU64,
    pub messages_sent: AtomicU64,
    pub messages_received: AtomicU64,
}

impl NetworkCounters {
    pub fn new() -> Self {
        Self {
            bytes_sent: AtomicU64::new(0),
            bytes_received: AtomicU64::new(0),
            bytes_sent_raw: AtomicU64::new(0),
            bytes_received_raw: AtomicU64::new(0),
            messages_sent: AtomicU64::new(0),
            messages_received: AtomicU64::new(0),
        }
    }

    pub fn record_send(&self, wire_bytes: u64, raw_bytes: u64) {
        self.bytes_sent.fetch_add(wire_bytes, Ordering::Relaxed);
        self.bytes_sent_raw.fetch_add(raw_bytes, Ordering::Relaxed);
        self.messages_sent.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_receive(&self, wire_bytes: u64, raw_bytes: u64) {
        self.bytes_received.fetch_add(wire_bytes, Ordering::Relaxed);
        self.bytes_received_raw.fetch_add(raw_bytes, Ordering::Relaxed);
        self.messages_received.fetch_add(1, Ordering::Relaxed);
    }

    /// Snapshot and reset all counters atomically (swap with 0).
    pub fn snapshot_and_reset(&self) -> StatsSnapshot {
        StatsSnapshot {
            bytes_sent: self.bytes_sent.swap(0, Ordering::Relaxed),
            bytes_received: self.bytes_received.swap(0, Ordering::Relaxed),
            bytes_sent_raw: self.bytes_sent_raw.swap(0, Ordering::Relaxed),
            bytes_received_raw: self.bytes_received_raw.swap(0, Ordering::Relaxed),
            messages_sent: self.messages_sent.swap(0, Ordering::Relaxed),
            messages_received: self.messages_received.swap(0, Ordering::Relaxed),
        }
    }
}
```

### Per-message-type counters

A separate structure tracks per-message-type counts and byte totals, keyed by `MessageTag` (story 05):

```rust
use std::sync::Mutex;

pub struct PerMessageCounters {
    inner: Mutex<HashMap<MessageTag, MessageTypeStats>>,
}

#[derive(Debug, Clone, Default)]
pub struct MessageTypeStats {
    pub count: u64,
    pub total_bytes: u64,
}

impl PerMessageCounters {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    pub fn record(&self, tag: MessageTag, bytes: u64) {
        let mut map = self.inner.lock().unwrap();
        let entry = map.entry(tag).or_default();
        entry.count += 1;
        entry.total_bytes += bytes;
    }

    /// Snapshot and reset all per-message counters.
    pub fn snapshot_and_reset(&self) -> HashMap<MessageTag, MessageTypeStats> {
        let mut map = self.inner.lock().unwrap();
        std::mem::take(&mut *map)
    }
}
```

### Stats snapshot

```rust
#[derive(Debug, Clone, Default)]
pub struct StatsSnapshot {
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub bytes_sent_raw: u64,
    pub bytes_received_raw: u64,
    pub messages_sent: u64,
    pub messages_received: u64,
}
```

### NetworkStats resource (ECS-facing)

```rust
/// ECS resource holding the latest per-second network statistics.
/// Updated once per second by the stats update system.
pub struct NetworkStats {
    /// Stats for the most recently completed second.
    pub current: StatsSnapshot,
    /// Per-message-type breakdown for the most recently completed second.
    pub per_message: HashMap<MessageTag, MessageTypeStats>,
    /// Bandwidth warning threshold in bytes/second. Default: 10 MB/s.
    pub warning_threshold: u64,
}

impl Default for NetworkStats {
    fn default() -> Self {
        Self {
            current: StatsSnapshot::default(),
            per_message: HashMap::new(),
            warning_threshold: 10 * 1024 * 1024,
        }
    }
}
```

### Stats update system

A system runs once per second (on a 1-second timer or via tick counting) to snapshot the live counters into the `NetworkStats` resource:

```rust
pub fn update_network_stats(
    counters: &NetworkCounters,
    per_msg_counters: &PerMessageCounters,
    stats: &mut NetworkStats,
) {
    stats.current = counters.snapshot_and_reset();
    stats.per_message = per_msg_counters.snapshot_and_reset();

    let total_bytes = stats.current.bytes_sent + stats.current.bytes_received;
    if total_bytes > stats.warning_threshold {
        tracing::warn!(
            "Bandwidth threshold exceeded: {} bytes/s (threshold: {} bytes/s)",
            total_bytes,
            stats.warning_threshold
        );
    }

    tracing::debug!(
        "Network: sent={} bytes ({} msgs), recv={} bytes ({} msgs), compression ratio={:.1}%",
        stats.current.bytes_sent,
        stats.current.messages_sent,
        stats.current.bytes_received,
        stats.current.messages_received,
        if stats.current.bytes_sent_raw > 0 {
            (1.0 - stats.current.bytes_sent as f64 / stats.current.bytes_sent_raw as f64) * 100.0
        } else {
            0.0
        }
    );
}
```

## Outcome

A `bandwidth.rs` module in `crates/nebula_net/src/` exporting `NetworkCounters`, `PerMessageCounters`, `StatsSnapshot`, `MessageTypeStats`, `NetworkStats`, and `update_network_stats`. Network tasks increment atomic counters on every send/receive. Once per second the counters are snapshotted into a `NetworkStats` resource for the debug overlay and adaptive quality systems. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

The title bar shows real-time bandwidth: `Up: 1.2 KB/s | Down: 4.8 KB/s`. Bandwidth is measured and displayed per second.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `tracing` | `0.1` | Logging for bandwidth warnings and debug stats |

No external crates beyond the standard library and `tracing` — counters use `std::sync::atomic` and `std::sync::Mutex`.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stats_track_bytes_sent() {
        let counters = NetworkCounters::new();
        counters.record_send(100, 120);
        counters.record_send(200, 250);

        let snapshot = counters.snapshot_and_reset();
        assert_eq!(snapshot.bytes_sent, 300);
        assert_eq!(snapshot.bytes_sent_raw, 370);
        assert_eq!(snapshot.messages_sent, 2);
    }

    #[test]
    fn test_stats_track_bytes_received() {
        let counters = NetworkCounters::new();
        counters.record_receive(500, 600);
        counters.record_receive(300, 400);

        let snapshot = counters.snapshot_and_reset();
        assert_eq!(snapshot.bytes_received, 800);
        assert_eq!(snapshot.bytes_received_raw, 1000);
        assert_eq!(snapshot.messages_received, 2);
    }

    #[test]
    fn test_stats_update_per_second() {
        let counters = NetworkCounters::new();
        counters.record_send(100, 100);
        counters.record_receive(200, 200);

        let per_msg = PerMessageCounters::new();
        let mut stats = NetworkStats::default();

        update_network_stats(&counters, &per_msg, &mut stats);

        assert_eq!(stats.current.bytes_sent, 100);
        assert_eq!(stats.current.bytes_received, 200);

        // After snapshot, counters should be reset
        let snapshot2 = counters.snapshot_and_reset();
        assert_eq!(snapshot2.bytes_sent, 0);
        assert_eq!(snapshot2.bytes_received, 0);
    }

    #[test]
    fn test_per_message_type_counts_are_correct() {
        let per_msg = PerMessageCounters::new();
        per_msg.record(MessageTag::Ping, 10);
        per_msg.record(MessageTag::Ping, 12);
        per_msg.record(MessageTag::ChunkData, 5000);

        let snapshot = per_msg.snapshot_and_reset();

        let ping_stats = snapshot.get(&MessageTag::Ping).unwrap();
        assert_eq!(ping_stats.count, 2);
        assert_eq!(ping_stats.total_bytes, 22);

        let chunk_stats = snapshot.get(&MessageTag::ChunkData).unwrap();
        assert_eq!(chunk_stats.count, 1);
        assert_eq!(chunk_stats.total_bytes, 5000);
    }

    #[test]
    fn test_warning_triggers_above_threshold() {
        let counters = NetworkCounters::new();
        // Exceed the 10 MB/s threshold
        counters.record_send(6 * 1024 * 1024, 6 * 1024 * 1024);
        counters.record_receive(6 * 1024 * 1024, 6 * 1024 * 1024);

        let per_msg = PerMessageCounters::new();
        let mut stats = NetworkStats::default();
        stats.warning_threshold = 10 * 1024 * 1024;

        update_network_stats(&counters, &per_msg, &mut stats);

        let total = stats.current.bytes_sent + stats.current.bytes_received;
        assert!(
            total > stats.warning_threshold,
            "Total {} should exceed threshold {}",
            total,
            stats.warning_threshold
        );
    }

    #[test]
    fn test_snapshot_resets_counters() {
        let counters = NetworkCounters::new();
        counters.record_send(100, 100);

        let snap1 = counters.snapshot_and_reset();
        assert_eq!(snap1.bytes_sent, 100);

        let snap2 = counters.snapshot_and_reset();
        assert_eq!(snap2.bytes_sent, 0, "Counters should be zero after reset");
    }

    #[test]
    fn test_per_message_snapshot_resets() {
        let per_msg = PerMessageCounters::new();
        per_msg.record(MessageTag::Ping, 10);

        let snap1 = per_msg.snapshot_and_reset();
        assert!(snap1.contains_key(&MessageTag::Ping));

        let snap2 = per_msg.snapshot_and_reset();
        assert!(snap2.is_empty(), "Per-message counters should be empty after reset");
    }

    #[test]
    fn test_compression_ratio_calculation() {
        let snapshot = StatsSnapshot {
            bytes_sent: 600,       // Wire bytes (compressed)
            bytes_sent_raw: 1000,  // Raw bytes (before compression)
            ..Default::default()
        };

        let ratio = 1.0 - (snapshot.bytes_sent as f64 / snapshot.bytes_sent_raw as f64);
        assert!((ratio - 0.4).abs() < 0.001, "Expected 40% compression ratio");
    }
}
```
