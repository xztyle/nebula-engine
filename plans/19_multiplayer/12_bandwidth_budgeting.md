# Bandwidth Budgeting

## Problem

A multiplayer server sends many types of data to each client: entity replication updates, voxel chunk streams, chat messages, clock sync pings, and more. Without bandwidth control, a client exploring a new area could receive a flood of chunk data that saturates their connection, causing packet buffering, increased latency, and potential TCP congestion collapse. The server must enforce a per-client bandwidth budget, prioritize critical messages, and adapt to clients with varying connection quality.

## Solution

### Per-Client Bandwidth Budget

Each connected client has a configurable bandwidth budget — the maximum number of bytes the server will send to that client per tick. The default is 1 Mbps, which at 60 Hz equates to approximately 2,083 bytes per tick.

```rust
pub struct BandwidthConfig {
    pub max_bytes_per_second: usize,  // default: 125_000 (1 Mbps)
    pub tick_rate: u32,                // 60
}

impl BandwidthConfig {
    pub fn bytes_per_tick(&self) -> usize {
        self.max_bytes_per_second / self.tick_rate as usize
    }
}

pub struct ClientBandwidthTracker {
    pub client_id: ClientId,
    pub config: BandwidthConfig,
    pub bytes_sent_this_tick: usize,
    pub bytes_sent_history: VecDeque<usize>, // per-tick history for averaging
    pub max_history: usize,                  // default: 600 (10 seconds)
}

impl ClientBandwidthTracker {
    pub fn remaining_budget(&self) -> usize {
        self.config.bytes_per_tick().saturating_sub(self.bytes_sent_this_tick)
    }

    pub fn consume(&mut self, bytes: usize) {
        self.bytes_sent_this_tick += bytes;
    }

    pub fn end_tick(&mut self) {
        self.bytes_sent_history.push_back(self.bytes_sent_this_tick);
        if self.bytes_sent_history.len() > self.max_history {
            self.bytes_sent_history.pop_front();
        }
        self.bytes_sent_this_tick = 0;
    }

    pub fn average_usage(&self) -> f64 {
        if self.bytes_sent_history.is_empty() {
            return 0.0;
        }
        let sum: usize = self.bytes_sent_history.iter().sum();
        sum as f64 / self.bytes_sent_history.len() as f64
    }
}
```

### Message Priority Levels

Messages are assigned priority levels. Higher-priority messages are sent first; lower-priority messages are deferred when budget is exceeded.

| Priority | Message Type                  | Description                              |
| -------- | ----------------------------- | ---------------------------------------- |
| 0 (max)  | Player state updates          | The client's own authoritative position   |
| 1        | Nearby entity updates         | Entities within the interest area         |
| 2        | Voxel edit events             | Real-time block changes                   |
| 3        | Chunk data                    | Streamed terrain data                     |
| 4        | Chat messages                 | Text communication                        |
| 5 (min)  | Non-critical metadata         | Stats, debug info                         |

```rust
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MessagePriority {
    PlayerState = 0,
    NearbyEntities = 1,
    VoxelEdits = 2,
    ChunkData = 3,
    Chat = 4,
    Metadata = 5,
}

pub struct PrioritizedMessage {
    pub priority: MessagePriority,
    pub data: Vec<u8>,
    pub size: usize,
}
```

### Tick Send Loop

Each tick, the server's send system processes the outgoing message queue for each client in priority order:

```rust
pub fn send_tick_messages(
    tracker: &mut ClientBandwidthTracker,
    queue: &mut Vec<PrioritizedMessage>,
    tcp_sender: &mut TcpSender,
) -> Vec<PrioritizedMessage> {
    // Sort by priority (ascending = highest priority first)
    queue.sort_by_key(|m| m.priority);

    let mut deferred = Vec::new();

    for message in queue.drain(..) {
        if tracker.remaining_budget() >= message.size {
            tracker.consume(message.size);
            tcp_sender.send(&message.data);
        } else {
            deferred.push(message);
        }
    }

    tracker.end_tick();
    deferred // Return deferred messages for next tick
}
```

Messages that are deferred are placed back into the queue for the next tick. If a deferred message ages beyond a maximum deferral time (e.g., 500 ms), it is escalated in priority or dropped (depending on type — chunk data can be deferred indefinitely; entity updates older than 500 ms are dropped as stale).

### Adaptive Rate Reduction

For clients with poor connections (detected via high RTT or frequent TCP backpressure), the server reduces the entity update rate. Instead of sending updates every tick, it sends every 2nd or 3rd tick for non-critical entities:

```rust
pub struct AdaptiveRate {
    pub entity_update_interval: u32, // default: 1 (every tick), max: 4
    pub rtt_threshold_ms: u64,       // above this, start reducing (default: 150ms)
}

impl AdaptiveRate {
    pub fn adjust(&mut self, rtt_ms: u64) {
        if rtt_ms > self.rtt_threshold_ms * 2 {
            self.entity_update_interval = 4;
        } else if rtt_ms > self.rtt_threshold_ms {
            self.entity_update_interval = 2;
        } else {
            self.entity_update_interval = 1;
        }
    }

    pub fn should_send_entity_update(&self, tick: u64) -> bool {
        tick % self.entity_update_interval as u64 == 0
    }
}
```

The player's own state (priority 0) is never reduced — it always sends every tick.

### Bandwidth Monitoring

The server tracks per-client bandwidth usage over time for monitoring and debugging. This data is exposed to the debug overlay (Epic 28) and server admin tools:

```rust
pub struct BandwidthStats {
    pub client_id: ClientId,
    pub current_bps: usize,
    pub peak_bps: usize,
    pub average_bps: f64,
    pub messages_deferred_this_tick: usize,
    pub adaptive_interval: u32,
}
```

## Outcome

- `nebula_multiplayer::bandwidth` module containing `BandwidthConfig`, `ClientBandwidthTracker`, `MessagePriority`, `PrioritizedMessage`, `AdaptiveRate`, `BandwidthStats`, and `send_tick_messages`.
- Per-client bandwidth enforcement with configurable limits.
- Priority-based message scheduling ensuring critical data is always delivered.
- Deferral of low-priority messages when budget is exceeded.
- Adaptive entity update rate reduction for high-latency clients.
- Bandwidth statistics for monitoring and debugging.

## Demo Integration

**Demo crate:** `nebula-demo`

The server caps outbound bandwidth per client. If too many entities need replication, lower-priority updates are deferred. The title shows `Budget: 64 KB/s, used: 41 KB/s`.

## Crates & Dependencies

| Crate       | Version | Purpose                                         |
| ----------- | ------- | ----------------------------------------------- |
| `tokio`     | 1.49    | Async TCP send, connection quality monitoring    |
| `serde`     | 1.0     | Serialization of prioritized messages            |
| `postcard`  | 1.1     | Binary encoding of outgoing messages             |
| `bevy_ecs`  | 0.18    | ECS resource for per-client bandwidth state      |

## Unit Tests

### `test_bandwidth_stays_within_budget`
Set budget to 10,000 bytes/tick. Queue 20 messages of 1,000 bytes each. Run `send_tick_messages`. Assert total bytes sent is exactly 10,000 (10 messages) and 10 messages are deferred.

### `test_high_priority_messages_always_sent`
Set budget to 5,000 bytes. Queue one `PlayerState` message (1,000 bytes, priority 0) and five `ChunkData` messages (1,000 bytes each, priority 3). Run `send_tick_messages`. Assert the `PlayerState` message is sent. Assert exactly 4 `ChunkData` messages are sent (filling remaining budget). Assert 1 `ChunkData` message is deferred.

### `test_low_priority_deferred_when_budget_exceeded`
Set budget to 3,000 bytes. Queue three `NearbyEntities` messages (1,000 bytes each, priority 1) and two `Chat` messages (500 bytes each, priority 4). Run `send_tick_messages`. Assert all three entity messages are sent (3,000 bytes, budget exhausted). Assert both chat messages are deferred.

### `test_per_client_tracking_is_accurate`
Send varying amounts of data across 10 ticks. Assert `bytes_sent_history` contains exactly 10 entries matching the actual bytes sent each tick. Assert `average_usage()` equals the arithmetic mean of the 10 entries.

### `test_adaptive_rate_reduction_works`
Set RTT threshold to 150 ms. Record RTT of 100 ms. Assert `entity_update_interval` is 1. Record RTT of 200 ms. Assert interval increases to 2. Record RTT of 350 ms. Assert interval increases to 4. Record RTT of 80 ms. Assert interval returns to 1.
