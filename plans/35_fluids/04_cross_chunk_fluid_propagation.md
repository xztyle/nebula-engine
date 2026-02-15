# Cross-Chunk Fluid Propagation

## Problem

Chunks in the voxel engine are independent 32x32x32 blocks of data. Fluid simulation operates on individual cells and their neighbors, but when a fluid cell sits at the edge of a chunk, its neighbor lives in a different chunk's memory. Without explicit cross-chunk propagation, fluid stops at chunk boundaries as if hitting an invisible wall. A river flowing across a terrain must seamlessly cross dozens of chunk boundaries. The cross-chunk system must handle several difficult cases: the neighbor chunk may not be loaded (the player hasn't explored there yet), both chunks may be updating simultaneously (race conditions), and a cascade of cross-chunk updates (water flowing downhill across 50 chunks in one tick) could spike CPU usage and freeze the game. The system must propagate fluid across boundaries correctly, buffer updates for unloaded chunks, and rate-limit cascading propagation.

## Solution

### Cross-Chunk Fluid Update Queue

When the cellular automaton (story 02) modifies a fluid cell at the edge of a chunk, it does not directly write to the neighbor chunk. Instead, it enqueues a `CrossChunkFluidUpdate`:

```rust
/// A pending fluid update that targets a cell in a neighboring chunk.
#[derive(Clone, Debug)]
pub struct CrossChunkFluidUpdate {
    /// The chunk that should receive this update.
    pub target_chunk: ChunkAddress,
    /// The local coordinates within the target chunk (0..CHUNK_SIZE for each axis).
    pub local_x: u8,
    pub local_y: u8,
    pub local_z: u8,
    /// The fluid type being propagated.
    pub fluid_type: FluidTypeId,
    /// The level to set (or the amount to add, depending on merge strategy).
    pub level_delta: i8,
    /// Source chunk that generated this update (for cycle detection).
    pub source_chunk: ChunkAddress,
    /// Tick number when this update was generated.
    pub tick: u64,
}
```

### Propagation Queue Resource

A central queue holds all pending cross-chunk updates:

```rust
pub struct FluidPropagationQueue {
    /// Pending updates grouped by target chunk.
    pending: HashMap<ChunkAddress, Vec<CrossChunkFluidUpdate>>,
    /// Updates for chunks that are not currently loaded.
    buffered: HashMap<ChunkAddress, Vec<CrossChunkFluidUpdate>>,
    /// Maximum cross-chunk updates to process per tick (rate limit).
    max_updates_per_tick: usize,
    /// Maximum cross-chunk updates to process per chunk per tick.
    max_updates_per_chunk_per_tick: usize,
}
```

### Enqueueing Updates

When a fluid cell at the boundary of chunk A flows into a cell that belongs to chunk B, the automaton calls:

```rust
impl FluidPropagationQueue {
    pub fn enqueue(
        &mut self,
        update: CrossChunkFluidUpdate,
        loaded_chunks: &HashSet<ChunkAddress>,
    ) {
        if loaded_chunks.contains(&update.target_chunk) {
            self.pending
                .entry(update.target_chunk)
                .or_default()
                .push(update);
        } else {
            // Target chunk isn't loaded — buffer for later
            self.buffered
                .entry(update.target_chunk)
                .or_default()
                .push(update);
        }
    }
}
```

### Applying Pending Updates

A system runs after the main fluid simulation, in the same `FixedUpdate` tick:

```rust
fn apply_cross_chunk_fluid_system(
    mut queue: ResMut<FluidPropagationQueue>,
    mut chunks: Query<(&ChunkAddress, &mut ChunkFluidData)>,
    mut simulation: ResMut<FluidSimulation>,
) {
    let mut total_applied = 0;

    // Process pending updates, respecting rate limits
    let chunk_addrs: Vec<_> = queue.pending.keys().copied().collect();
    for addr in chunk_addrs {
        if total_applied >= queue.max_updates_per_tick {
            break; // Global budget exhausted — defer remaining to next tick
        }

        if let Some(updates) = queue.pending.remove(&addr) {
            let chunk_budget = queue.max_updates_per_chunk_per_tick;
            let (apply_now, defer) = if updates.len() > chunk_budget {
                let (a, d) = updates.split_at(chunk_budget);
                (a.to_vec(), Some(d.to_vec()))
            } else {
                (updates, None)
            };

            for update in &apply_now {
                apply_fluid_update(&mut chunks, &addr, update);
                total_applied += 1;
            }

            // Mark this chunk as active so the automaton runs on it next tick
            simulation.activate_chunk(addr);

            // Re-queue deferred updates
            if let Some(deferred) = defer {
                queue.pending.insert(addr, deferred);
            }
        }
    }
}
```

### Buffered Updates for Unloaded Chunks

When a chunk loads (detected via a `ChunkLoadedEvent` from Epic 06), the system drains any buffered updates for that chunk:

```rust
fn flush_buffered_fluid_on_chunk_load(
    mut queue: ResMut<FluidPropagationQueue>,
    mut events: EventReader<ChunkLoadedEvent>,
    mut chunks: Query<(&ChunkAddress, &mut ChunkFluidData)>,
    mut simulation: ResMut<FluidSimulation>,
) {
    for event in events.read() {
        if let Some(buffered) = queue.buffered.remove(&event.address) {
            for update in &buffered {
                apply_fluid_update(&mut chunks, &event.address, update);
            }
            simulation.activate_chunk(event.address);
        }
    }
}
```

Buffered updates have a TTL: if a chunk hasn't loaded within 300 seconds (game time), its buffered updates are discarded to prevent unbounded memory growth:

```rust
impl FluidPropagationQueue {
    pub fn expire_old_buffers(&mut self, current_tick: u64, ticks_per_second: u64) {
        let max_age = 300 * ticks_per_second;
        self.buffered.retain(|_, updates| {
            updates.retain(|u| current_tick - u.tick < max_age);
            !updates.is_empty()
        });
    }
}
```

### Rate Limiting to Prevent Cascading Updates

The core defense against cascade storms is the per-tick budget. Default configuration:

- `max_updates_per_tick`: 1024 (global, across all chunks)
- `max_updates_per_chunk_per_tick`: 64 (per target chunk)

When a waterfall drops through 100 vertical chunks, each chunk generates boundary updates for the chunk below. Without rate limiting, all 100 chunks would propagate in a single tick. With the budget, only 1024 total updates are applied per tick. The remaining updates stay in the `pending` queue and are processed over subsequent ticks, creating a natural propagation wavefront that spreads over time rather than computing instantly.

### Bidirectional Flow

When chunk A pushes fluid into chunk B, chunk B may push fluid back (e.g., when equalizing levels). This is handled naturally: chunk B's automaton runs, and if it generates a boundary update for chunk A, that update is enqueued just like any other. The `source_chunk` field prevents infinite ping-pong within a single tick by skipping updates that would immediately reverse a just-applied update from the same tick.

### All 6 Faces

Cross-chunk propagation must work in all 6 directions (+X, -X, +Y, -Y, +Z, -Z). The boundary detection logic checks all 6 faces of the chunk:

```rust
/// Check if a local coordinate is at a chunk boundary, and if so,
/// return the neighbor chunk address and the mirrored local coordinate.
pub fn boundary_neighbor(
    chunk_addr: &ChunkAddress,
    local: [u8; 3],
    direction: usize, // index into NEIGHBOR_OFFSETS
) -> Option<(ChunkAddress, [u8; 3])> {
    let offset = NEIGHBOR_OFFSETS[direction];
    let new_local = [
        local[0] as i32 + offset[0],
        local[1] as i32 + offset[1],
        local[2] as i32 + offset[2],
    ];

    // If new_local is outside [0, CHUNK_SIZE), we've crossed a boundary
    if new_local.iter().any(|&c| c < 0 || c >= CHUNK_SIZE as i32) {
        let neighbor_addr = chunk_addr.neighbor_in_direction(direction)?;
        let mirrored = [
            new_local[0].rem_euclid(CHUNK_SIZE as i32) as u8,
            new_local[1].rem_euclid(CHUNK_SIZE as i32) as u8,
            new_local[2].rem_euclid(CHUNK_SIZE as i32) as u8,
        ];
        Some((neighbor_addr, mirrored))
    } else {
        None
    }
}
```

## Outcome

The `nebula-fluid` crate exports `CrossChunkFluidUpdate`, `FluidPropagationQueue`, `apply_cross_chunk_fluid_system`, and `flush_buffered_fluid_on_chunk_load`. Fluid flows seamlessly across chunk boundaries. Updates for unloaded chunks are buffered and applied on load. Rate limiting prevents cascade spikes. Running `cargo test -p nebula-fluid` passes all cross-chunk propagation tests. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

Water flows across chunk boundaries without interruption. A river can span many chunks seamlessly. The fluid simulation coordinates across boundaries.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.18` | ECS systems, resources, event readers, and `FixedUpdate` schedule |
| `hashbrown` | `0.15` | Fast `HashMap` for pending/buffered update queues grouped by chunk |
| `smallvec` | `1.15` | Stack-allocated update batches for per-chunk processing |

Depends on Epic 05 (chunk neighbor system including cross-face), Epic 06 (chunk loading events), and stories 01-03 of this epic.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn make_chunk_addr(face: CubeFace, x: u32, y: u32, z: u32) -> ChunkAddress {
        ChunkAddress::new(face, 10, x, y)
    }

    fn make_update(target: ChunkAddress, level: i8, tick: u64) -> CrossChunkFluidUpdate {
        CrossChunkFluidUpdate {
            target_chunk: target,
            local_x: 0,
            local_y: 0,
            local_z: 0,
            fluid_type: FluidTypeId(0),
            level_delta: level,
            source_chunk: ChunkAddress::new(CubeFace::PosX, 10, 0, 0),
            tick,
        }
    }

    #[test]
    fn test_fluid_crosses_chunk_boundary() {
        let mut queue = FluidPropagationQueue::new(1024, 64);
        let target = make_chunk_addr(CubeFace::PosX, 1, 0, 0);
        let loaded = HashSet::from([target]);

        let update = make_update(target, 7, 0);
        queue.enqueue(update, &loaded);

        assert!(queue.pending.contains_key(&target));
        assert_eq!(queue.pending[&target].len(), 1);
        assert_eq!(queue.pending[&target][0].level_delta, 7);
    }

    #[test]
    fn test_buffered_update_applies_when_neighbor_loads() {
        let mut queue = FluidPropagationQueue::new(1024, 64);
        let target = make_chunk_addr(CubeFace::PosX, 5, 5, 0);
        let loaded = HashSet::new(); // target is NOT loaded

        let update = make_update(target, 4, 0);
        queue.enqueue(update, &loaded);

        // Should be buffered, not pending
        assert!(!queue.pending.contains_key(&target));
        assert!(queue.buffered.contains_key(&target));
        assert_eq!(queue.buffered[&target].len(), 1);

        // Simulate chunk loading: move buffered -> pending
        let drained = queue.buffered.remove(&target).unwrap();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].level_delta, 4);
    }

    #[test]
    fn test_rate_limiting_prevents_cascade() {
        let max_per_tick = 10;
        let mut queue = FluidPropagationQueue::new(max_per_tick, 64);
        let target = make_chunk_addr(CubeFace::PosY, 0, 0, 0);
        let loaded = HashSet::from([target]);

        // Enqueue 50 updates — far more than the per-tick budget
        for i in 0..50 {
            let update = make_update(target, 1, 0);
            queue.enqueue(update, &loaded);
        }

        assert_eq!(queue.pending[&target].len(), 50);

        // Simulate applying with budget
        let applied = queue.pending[&target]
            .drain(..max_per_tick.min(queue.pending[&target].len()))
            .collect::<Vec<_>>();
        assert_eq!(applied.len(), max_per_tick);

        // 40 updates remain deferred
        assert_eq!(queue.pending[&target].len(), 40);
    }

    #[test]
    fn test_bidirectional_flow_across_boundary() {
        let mut queue = FluidPropagationQueue::new(1024, 64);
        let chunk_a = make_chunk_addr(CubeFace::PosX, 0, 0, 0);
        let chunk_b = make_chunk_addr(CubeFace::PosX, 1, 0, 0);
        let loaded = HashSet::from([chunk_a, chunk_b]);

        // A -> B flow
        let update_ab = CrossChunkFluidUpdate {
            target_chunk: chunk_b,
            local_x: 0, local_y: 15, local_z: 15,
            fluid_type: FluidTypeId(0),
            level_delta: 3,
            source_chunk: chunk_a,
            tick: 0,
        };
        queue.enqueue(update_ab, &loaded);

        // B -> A flow (equalization)
        let update_ba = CrossChunkFluidUpdate {
            target_chunk: chunk_a,
            local_x: 31, local_y: 15, local_z: 15,
            fluid_type: FluidTypeId(0),
            level_delta: 1,
            source_chunk: chunk_b,
            tick: 0,
        };
        queue.enqueue(update_ba, &loaded);

        assert!(queue.pending.contains_key(&chunk_a), "A should have pending updates");
        assert!(queue.pending.contains_key(&chunk_b), "B should have pending updates");
    }

    #[test]
    fn test_all_6_faces_propagate_correctly() {
        // Verify that boundary_neighbor returns valid results for all 6 directions
        let chunk = make_chunk_addr(CubeFace::PosX, 10, 10, 0);

        // Test each face of the chunk
        let edge_positions: [(usize, [u8; 3]); 6] = [
            (0, [31, 15, 15]), // +X boundary
            (1, [0, 15, 15]),  // -X boundary
            (2, [15, 31, 15]), // +Y boundary
            (3, [15, 0, 15]),  // -Y boundary
            (4, [15, 15, 31]), // +Z boundary
            (5, [15, 15, 0]),  // -Z boundary
        ];

        for (direction, local) in &edge_positions {
            let result = boundary_neighbor(&chunk, *local, *direction);
            assert!(
                result.is_some(),
                "Boundary neighbor should exist for direction {direction} at edge {local:?}"
            );
            let (neighbor_addr, mirrored) = result.unwrap();
            // Mirrored coordinate should be on the opposite edge
            for &c in &mirrored {
                assert!(
                    c < CHUNK_SIZE as u8,
                    "Mirrored coordinate {c} out of range for direction {direction}"
                );
            }
        }
    }

    #[test]
    fn test_buffered_updates_expire_after_timeout() {
        let mut queue = FluidPropagationQueue::new(1024, 64);
        let target = make_chunk_addr(CubeFace::NegZ, 0, 0, 0);
        let loaded = HashSet::new();

        // Enqueue an update at tick 0
        queue.enqueue(make_update(target, 5, 0), &loaded);
        assert_eq!(queue.buffered.len(), 1);

        // Expire with current_tick far in the future (300s * 60 tps = 18000 ticks)
        queue.expire_old_buffers(20_000, 60);
        assert!(
            queue.buffered.is_empty(),
            "Old buffered updates should be expired"
        );
    }

    #[test]
    fn test_interior_cell_has_no_boundary_neighbor() {
        let chunk = make_chunk_addr(CubeFace::PosX, 5, 5, 0);
        let interior = [15u8, 15, 15];

        for direction in 0..6 {
            let result = boundary_neighbor(&chunk, interior, direction);
            assert!(
                result.is_none(),
                "Interior cell at {interior:?} should not have a boundary neighbor in direction {direction}"
            );
        }
    }
}
```
