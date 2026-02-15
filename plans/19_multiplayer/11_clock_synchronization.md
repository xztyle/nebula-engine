# Clock Synchronization

## Problem

In a server-authoritative multiplayer engine, the server and all clients must agree on a shared notion of time (game ticks). Without clock synchronization, client-side prediction (Story 04) cannot tag inputs with correct tick numbers, server reconciliation (Story 05) cannot match predictions to authoritative states, and entity interpolation across clients will be jittery. The client needs to estimate the server's current tick despite variable network latency, and run slightly ahead so that inputs arrive at the server in time for the tick they target.

## Solution

### Tick Model

Both client and server run at a fixed tick rate of 60 ticks per second. The server's tick counter is the authoritative reference. The client maintains its own tick counter that attempts to stay synchronized with the server's.

```rust
pub const TICK_RATE: u32 = 60;
pub const TICK_DURATION: Duration = Duration::from_nanos(1_000_000_000 / TICK_RATE as u64);

#[derive(Component)]
pub struct TickCounter {
    pub tick: u64,
    pub is_server: bool,
}
```

### RTT Measurement

The client periodically sends ping messages and measures the round-trip time from the server's pong response. RTT is computed using an exponentially weighted moving average (EWMA) for stability:

```rust
#[derive(Serialize, Deserialize)]
pub struct Ping {
    pub client_send_time: Instant, // local monotonic clock
    pub sequence: u32,
}

#[derive(Serialize, Deserialize)]
pub struct Pong {
    pub sequence: u32,
    pub server_tick: u64,
}

pub struct RttEstimator {
    pub samples: VecDeque<Duration>,
    pub max_samples: usize,       // default: 16
    pub ewma_rtt: Duration,
    pub alpha: f64,               // EWMA smoothing factor, default: 0.125
}

impl RttEstimator {
    pub fn record_sample(&mut self, rtt: Duration) {
        self.samples.push_back(rtt);
        if self.samples.len() > self.max_samples {
            self.samples.pop_front();
        }

        let rtt_secs = rtt.as_secs_f64();
        let ewma_secs = self.ewma_rtt.as_secs_f64();
        let new_ewma = self.alpha * rtt_secs + (1.0 - self.alpha) * ewma_secs;
        self.ewma_rtt = Duration::from_secs_f64(new_ewma);
    }

    pub fn median_rtt(&self) -> Duration {
        let mut sorted: Vec<_> = self.samples.iter().copied().collect();
        sorted.sort();
        sorted.get(sorted.len() / 2).copied().unwrap_or(Duration::ZERO)
    }
}
```

### Tick Offset Estimation

Using the RTT and the server tick from the pong, the client estimates its tick offset relative to the server:

```rust
pub struct ClockSync {
    pub rtt: RttEstimator,
    pub tick_offset: i64,       // client_tick - server_tick
    pub target_lead: f64,       // ticks to lead by (≈ RTT/2 in ticks)
    pub converged: bool,
    pub min_samples_for_convergence: usize, // default: 8
}

impl ClockSync {
    pub fn on_pong_received(
        &mut self,
        pong: &Pong,
        local_receive_time: Instant,
        local_send_time: Instant,
        client_tick_at_send: u64,
    ) {
        let rtt = local_receive_time.duration_since(local_send_time);
        self.rtt.record_sample(rtt);

        // Estimate server tick at the time of receive
        let half_rtt_ticks = (rtt.as_secs_f64() * TICK_RATE as f64) / 2.0;
        let estimated_server_tick = pong.server_tick as f64 + half_rtt_ticks;

        // Client should lead by half RTT so inputs arrive on time
        self.target_lead = half_rtt_ticks;

        // Adjust tick offset
        let current_client_tick = client_tick_at_send as f64
            + (local_receive_time.duration_since(local_send_time).as_secs_f64()
                * TICK_RATE as f64);
        self.tick_offset = (current_client_tick - estimated_server_tick) as i64;

        if self.rtt.samples.len() >= self.min_samples_for_convergence {
            self.converged = true;
        }
    }

    pub fn adjusted_client_tick(&self, raw_tick: u64) -> u64 {
        // Ensure client runs target_lead ticks ahead of server
        (raw_tick as i64 + self.tick_offset) as u64
    }
}
```

### NTP-Like Algorithm

The synchronization algorithm takes multiple samples (default: 8 minimum) before considering the clock "converged." This filters out outlier samples caused by network jitter. The algorithm:

1. Send pings every 500 ms during initial sync (first 8 samples).
2. After convergence, reduce ping frequency to every 5 seconds for maintenance.
3. Discard outlier samples (those > 2x median RTT).
4. Use the median RTT for tick offset computation (more robust than mean).

### Client Tick Adjustment

If the client tick drifts too far from the target (more than 2 ticks off), the client adjusts:

- **Small drift (< 2 ticks)**: Speed up or slow down the tick rate slightly (e.g., 61 Hz or 59 Hz for a few ticks) until aligned.
- **Large drift (>= 2 ticks)**: Hard reset the tick counter to the target value.

```rust
pub fn compute_tick_adjustment(offset_error: f64) -> TickAdjustment {
    if offset_error.abs() < 2.0 {
        if offset_error > 0.1 {
            TickAdjustment::SlowDown // run slightly slower to let server catch up
        } else if offset_error < -0.1 {
            TickAdjustment::SpeedUp  // run slightly faster to catch up with server
        } else {
            TickAdjustment::None
        }
    } else {
        TickAdjustment::HardReset
    }
}
```

### Monotonic Ticks

Tick numbers are guaranteed to be monotonically increasing on both client and server. The adjustment logic never decreases the tick counter; it only slows down or speeds up progression.

## Outcome

- `nebula_multiplayer::clock` module containing `ClockSync`, `RttEstimator`, `Ping`, `Pong`, `TickCounter`, `TickAdjustment`, and related constants.
- NTP-like clock synchronization with multiple samples and jitter filtering.
- Client tick leads server by approximately RTT/2 for timely input delivery.
- Smooth tick rate adjustment for small drifts, hard reset for large drifts.
- Fixed 60 Hz tick rate on both client and server.

## Demo Integration

**Demo crate:** `nebula-demo`

Server and clients share a synchronized tick counter. The console shows `Server tick: 124,500 | Local tick: 124,498 | Offset: -2`. Tick drift is corrected gradually.

## Crates & Dependencies

| Crate       | Version | Purpose                                        |
| ----------- | ------- | ---------------------------------------------- |
| `tokio`     | 1.49    | Async TCP for ping/pong messages, timers        |
| `serde`     | 1.0     | Serialization of Ping/Pong messages             |
| `postcard`  | 1.1     | Binary encoding of sync messages                |
| `bevy_ecs`  | 0.18    | ECS resource for tick counter and clock state   |

## Unit Tests

### `test_tick_offset_converges_after_multiple_samples`
Simulate 16 pong responses with a consistent 50 ms RTT. Assert that `ClockSync.converged` is true after 8 samples and the `tick_offset` stabilizes to within 0.5 ticks of the expected value.

### `test_rtt_measurement_is_accurate`
Send a ping, simulate a 40 ms delay, receive a pong. Assert the `RttEstimator` records a sample of approximately 40 ms (within 1 ms tolerance).

### `test_client_tick_leads_server_by_half_rtt`
Synchronize with a server at RTT = 60 ms. Assert the client's target lead is approximately 1.8 ticks (60ms / 2 = 30ms = 1.8 ticks at 60 Hz) and the adjusted client tick reflects this lead.

### `test_clock_sync_handles_jitter`
Provide 16 RTT samples: 14 around 50 ms, 2 outliers at 200 ms. Assert the median RTT is approximately 50 ms and the outliers do not significantly affect the tick offset (within 0.5 ticks of the non-jittered expectation).

### `test_tick_numbers_are_monotonic`
Run the client tick system for 1000 ticks with clock adjustments occurring at ticks 100, 300, and 500. Assert the tick counter never decreases between consecutive ticks.
