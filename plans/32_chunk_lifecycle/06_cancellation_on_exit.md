# Cancellation on Exit

## Problem

When a player moves quickly (sprinting, teleporting, vehicle travel), chunks that were just scheduled for generation or meshing may exit the load radius before the background tasks complete. Without cancellation, the engine wastes CPU time generating and meshing chunks that will be immediately discarded. Worse, dozens of stale tasks clog the worker pool, delaying generation of chunks the player actually needs. In extreme cases (teleportation), the entire task queue becomes stale. The engine must cancel in-flight tasks when their target chunks are no longer needed, freeing worker threads for higher-priority work.

## Solution

Integrate cooperative cancellation tokens from Epic 31 (threading) into the chunk lifecycle pipeline. Each generation or meshing task carries a cancellation token that can be triggered from the main thread when a chunk exits the load radius.

### Cancellation Token

The token is a lightweight, thread-safe flag. This design comes from Epic 31 but is used here specifically for chunk tasks:

```rust
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// A cooperative cancellation token shared between the main thread
/// and a background worker.
#[derive(Clone, Debug)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Signal cancellation. The worker will observe this on its next check.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    /// Check if cancellation has been requested. Called periodically by workers.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}
```

### Integration with Chunk Tasks

Each in-flight chunk task is tracked with its cancellation token:

```rust
use bevy_ecs::prelude::*;
use std::collections::HashMap;
use crate::coords::ChunkAddress;

/// Tracks all in-flight chunk generation and meshing tasks.
#[derive(Resource, Default)]
pub struct InFlightChunkTasks {
    generation: HashMap<ChunkAddress, CancellationToken>,
    meshing: HashMap<ChunkAddress, CancellationToken>,
    /// Diagnostic counters
    completed_count: u64,
    cancelled_count: u64,
}

impl InFlightChunkTasks {
    /// Register a new generation task and return the token for the worker.
    pub fn start_generation(&mut self, addr: ChunkAddress) -> CancellationToken {
        let token = CancellationToken::new();
        self.generation.insert(addr, token.clone());
        token
    }

    /// Register a new meshing task and return the token for the worker.
    pub fn start_meshing(&mut self, addr: ChunkAddress) -> CancellationToken {
        let token = CancellationToken::new();
        self.meshing.insert(addr, token.clone());
        token
    }

    /// Cancel all tasks for a chunk (both generation and meshing).
    pub fn cancel(&mut self, addr: &ChunkAddress) {
        if let Some(token) = self.generation.remove(addr) {
            token.cancel();
            self.cancelled_count += 1;
        }
        if let Some(token) = self.meshing.remove(addr) {
            token.cancel();
            self.cancelled_count += 1;
        }
    }

    /// Mark a task as completed (called when a result is received).
    pub fn complete_generation(&mut self, addr: &ChunkAddress) {
        if self.generation.remove(addr).is_some() {
            self.completed_count += 1;
        }
    }

    pub fn complete_meshing(&mut self, addr: &ChunkAddress) {
        if self.meshing.remove(addr).is_some() {
            self.completed_count += 1;
        }
    }

    /// Cancel all in-flight tasks (e.g., on teleport or world switch).
    pub fn cancel_all(&mut self) {
        for (_, token) in self.generation.drain() {
            token.cancel();
            self.cancelled_count += 1;
        }
        for (_, token) in self.meshing.drain() {
            token.cancel();
            self.cancelled_count += 1;
        }
    }

    pub fn has_in_flight(&self, addr: &ChunkAddress) -> bool {
        self.generation.contains_key(addr) || self.meshing.contains_key(addr)
    }

    pub fn completed_count(&self) -> u64 {
        self.completed_count
    }

    pub fn cancelled_count(&self) -> u64 {
        self.cancelled_count
    }

    pub fn in_flight_count(&self) -> usize {
        self.generation.len() + self.meshing.len()
    }
}
```

### Cooperative Cancellation in Workers

Generation tasks check the cancellation token at natural checkpoints — between voxel column generations, between noise octaves, or between chunk sections:

```rust
pub fn generate_chunk(
    addr: ChunkAddress,
    seed: u64,
    token: &CancellationToken,
) -> Result<ChunkVoxelData, ChunkTaskError> {
    let mut data = ChunkVoxelData::new_empty(CHUNK_SIZE);

    for z in 0..CHUNK_SIZE {
        // Check cancellation once per z-slice (32 checks per chunk)
        if token.is_cancelled() {
            return Err(ChunkTaskError::Cancelled);
        }

        for y in 0..CHUNK_SIZE {
            for x in 0..CHUNK_SIZE {
                let voxel = terrain_noise(addr, x, y, z, seed);
                data.set(x, y, z, voxel);
            }
        }
    }

    Ok(data)
}

#[derive(Debug)]
pub enum ChunkTaskError {
    Cancelled,
    GenerationFailed(String),
}
```

### Cancellation on Unload

When the chunk evaluation system (story 02) determines a chunk should be unloaded, it cancels any in-flight tasks before transitioning the chunk state:

```rust
fn cancel_tasks_for_unloading_chunks(
    mut in_flight: ResMut<InFlightChunkTasks>,
    unloading: Query<&ChunkAddress, Added<ScheduleForUnload>>,
) {
    for addr in &unloading {
        in_flight.cancel(addr);
    }
}
```

### Re-entering the Load Radius

If a chunk exits and then re-enters the load radius (player turns around), the chunk is re-scheduled for generation from scratch. The previous task was cancelled and its partial results were discarded. The state machine returns the chunk to `Unloaded`, and the loading system can schedule it again.

## Outcome

The `nebula_chunk` crate exports `CancellationToken`, `InFlightChunkTasks`, and `ChunkTaskError`. Every in-flight generation and meshing task carries a cancellation token. When a chunk exits the load radius, its task is cancelled cooperatively. Cancelled tasks free their resources promptly. Diagnostic counters track completed vs cancelled task counts. Running `cargo test -p nebula_chunk` passes all cancellation tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Moving away from a chunk that is mid-generation cancels its generation task. The chunk returns to Unloaded state. No computation is wasted on irrelevant chunks.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.18` | `Resource` derive, ECS systems and queries |
| `tokio` | `1.49` | Async task spawning for I/O-bound tasks (save/load); generation tasks use `std::thread` via Epic 31 thread pool |

The cancellation token itself uses only `std::sync::atomic` and `std::sync::Arc`. The crate uses Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn addr(x: i64, y: i64, z: i64) -> ChunkAddress {
        ChunkAddress::new(x as i128, y as i128, z as i128)
    }

    /// Cancelling a chunk should signal the cancellation token.
    #[test]
    fn test_moving_away_cancels_in_flight_generation() {
        let mut tasks = InFlightChunkTasks::default();
        let chunk = addr(10, 0, 0);

        let token = tasks.start_generation(chunk);
        assert!(!token.is_cancelled());

        // Chunk exits load radius
        tasks.cancel(&chunk);

        assert!(token.is_cancelled(), "token should be cancelled after chunk exits");
        assert!(!tasks.has_in_flight(&chunk));
    }

    /// Cancelled tasks should free their resources (removed from tracking).
    #[test]
    fn test_cancellation_frees_resources() {
        let mut tasks = InFlightChunkTasks::default();

        for i in 0..10 {
            tasks.start_generation(addr(i, 0, 0));
        }
        assert_eq!(tasks.in_flight_count(), 10);

        // Cancel all
        tasks.cancel_all();
        assert_eq!(tasks.in_flight_count(), 0);
    }

    /// The cancelled count diagnostic should be tracked correctly.
    #[test]
    fn test_cancelled_count_is_tracked() {
        let mut tasks = InFlightChunkTasks::default();
        let a = addr(1, 0, 0);
        let b = addr(2, 0, 0);
        let c = addr(3, 0, 0);

        tasks.start_generation(a);
        tasks.start_generation(b);
        tasks.start_meshing(c);

        assert_eq!(tasks.cancelled_count(), 0);

        tasks.cancel(&a);
        assert_eq!(tasks.cancelled_count(), 1);

        tasks.cancel(&b);
        assert_eq!(tasks.cancelled_count(), 2);

        // c has a meshing task
        tasks.cancel(&c);
        assert_eq!(tasks.cancelled_count(), 3);

        assert_eq!(tasks.completed_count(), 0);
    }

    /// A chunk that re-enters the load radius should be re-schedulable.
    #[test]
    fn test_re_entering_load_radius_reschedules() {
        let mut tasks = InFlightChunkTasks::default();
        let chunk = addr(5, 0, 0);

        // First pass: schedule and cancel
        let token1 = tasks.start_generation(chunk);
        tasks.cancel(&chunk);
        assert!(token1.is_cancelled());
        assert!(!tasks.has_in_flight(&chunk));

        // Second pass: re-schedule
        let token2 = tasks.start_generation(chunk);
        assert!(!token2.is_cancelled(), "new token should not be cancelled");
        assert!(tasks.has_in_flight(&chunk));
    }

    /// Cancellation should not corrupt the state of other tracked tasks.
    #[test]
    fn test_cancellation_does_not_corrupt_state() {
        let mut tasks = InFlightChunkTasks::default();
        let chunk_a = addr(1, 0, 0);
        let chunk_b = addr(2, 0, 0);

        let token_a = tasks.start_generation(chunk_a);
        let token_b = tasks.start_generation(chunk_b);

        // Cancel only A
        tasks.cancel(&chunk_a);

        assert!(token_a.is_cancelled());
        assert!(!token_b.is_cancelled(), "B's token should be unaffected");
        assert!(tasks.has_in_flight(&chunk_b));
        assert!(!tasks.has_in_flight(&chunk_a));
    }

    /// Completing a task should increment the completed counter.
    #[test]
    fn test_completed_count_tracked_separately() {
        let mut tasks = InFlightChunkTasks::default();
        let chunk = addr(1, 0, 0);

        tasks.start_generation(chunk);
        tasks.complete_generation(&chunk);

        assert_eq!(tasks.completed_count(), 1);
        assert_eq!(tasks.cancelled_count(), 0);
        assert!(!tasks.has_in_flight(&chunk));
    }

    /// The cooperative cancellation check in a generation function should
    /// return an error when the token is cancelled.
    #[test]
    fn test_cooperative_cancellation_returns_error() {
        let token = CancellationToken::new();
        token.cancel();
        assert!(token.is_cancelled());

        // Simulating what the worker would do
        let result = if token.is_cancelled() {
            Err(ChunkTaskError::Cancelled)
        } else {
            Ok(())
        };

        assert!(matches!(result, Err(ChunkTaskError::Cancelled)));
    }
}
```
