# Neighbor Dependency Resolution

## Problem

Meshing a chunk requires voxel data from all six face-adjacent neighbors. Neighbor data is needed to determine face visibility at chunk boundaries (story 01 of Epic 07) and to compute ambient occlusion that spans chunk edges. If a chunk's neighbor has not yet been generated, meshing that chunk would produce incorrect geometry: either invisible walls at the boundary (if missing neighbors default to air) or visible faces that should be culled (if missing neighbors default to solid). The engine must wait until all six face-adjacent neighbors are in the `Generated` (or later) state before a chunk can transition to `Meshing`. This introduces a dependency graph that must be carefully managed to avoid deadlocks — if chunk A waits for chunk B and chunk B waits for chunk A, neither will ever mesh.

## Solution

Implement a neighbor dependency tracker in the `nebula_chunk` crate that monitors neighbor readiness and signals when a chunk is unblocked for meshing.

### Neighbor Map

```rust
use crate::coords::ChunkAddress;

/// The six face-adjacent directions in 3D space.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FaceNeighbor {
    PosX,
    NegX,
    PosY,
    NegY,
    PosZ,
    NegZ,
}

impl FaceNeighbor {
    pub const ALL: [FaceNeighbor; 6] = [
        Self::PosX, Self::NegX,
        Self::PosY, Self::NegY,
        Self::PosZ, Self::NegZ,
    ];

    /// Compute the neighbor's chunk address given a center chunk address.
    pub fn offset_from(&self, center: &ChunkAddress) -> ChunkAddress {
        match self {
            Self::PosX => center.offset(1, 0, 0),
            Self::NegX => center.offset(-1, 0, 0),
            Self::PosY => center.offset(0, 1, 0),
            Self::NegY => center.offset(0, -1, 0),
            Self::PosZ => center.offset(0, 0, 1),
            Self::NegZ => center.offset(0, 0, -1),
        }
    }
}
```

### Dependency Tracker

```rust
use bevy_ecs::prelude::*;
use std::collections::{HashMap, HashSet};

/// Tracks which chunks are waiting for neighbors before meshing.
#[derive(Resource, Default)]
pub struct NeighborDependencyTracker {
    /// For each chunk address, the set of neighbor directions still missing.
    waiting: HashMap<ChunkAddress, HashSet<FaceNeighbor>>,
    /// Reverse index: for each chunk address, which other chunks are waiting
    /// on it to become Generated.
    depended_on_by: HashMap<ChunkAddress, HashSet<ChunkAddress>>,
}

impl NeighborDependencyTracker {
    /// Register a chunk as waiting for its neighbors. Only neighbors that
    /// are not yet Generated are added to the waiting set.
    pub fn register(
        &mut self,
        chunk_addr: ChunkAddress,
        chunk_states: &HashMap<ChunkAddress, ChunkState>,
    ) {
        let mut missing = HashSet::new();

        for dir in FaceNeighbor::ALL {
            let neighbor_addr = dir.offset_from(&chunk_addr);

            match chunk_states.get(&neighbor_addr) {
                Some(state) if Self::is_ready(*state) => {
                    // Neighbor is already ready, no dependency
                }
                Some(_) => {
                    // Neighbor exists but is not ready yet
                    missing.insert(dir);
                    self.depended_on_by
                        .entry(neighbor_addr)
                        .or_default()
                        .insert(chunk_addr);
                }
                None => {
                    // Neighbor doesn't exist (edge of planet, unloaded, etc.)
                    // Treat as ready — edge chunks mesh with air boundaries.
                }
            }
        }

        if missing.is_empty() {
            // Already unblocked, don't add to waiting map
            return;
        }

        self.waiting.insert(chunk_addr, missing);
    }

    /// Notify the tracker that a chunk has reached the Generated state.
    /// Returns the set of chunks that are now fully unblocked for meshing.
    pub fn notify_generated(&mut self, generated_addr: ChunkAddress) -> Vec<ChunkAddress> {
        let mut unblocked = Vec::new();

        if let Some(dependents) = self.depended_on_by.remove(&generated_addr) {
            for dependent_addr in dependents {
                if let Some(missing) = self.waiting.get_mut(&dependent_addr) {
                    // Find and remove the direction that corresponds to generated_addr
                    missing.retain(|dir| dir.offset_from(&dependent_addr) != generated_addr);

                    if missing.is_empty() {
                        self.waiting.remove(&dependent_addr);
                        unblocked.push(dependent_addr);
                    }
                }
            }
        }

        unblocked
    }

    /// Check if a chunk is currently waiting for any neighbors.
    pub fn is_waiting(&self, chunk_addr: &ChunkAddress) -> bool {
        self.waiting.contains_key(chunk_addr)
    }

    /// Get the number of missing neighbors for a chunk, or 0 if not waiting.
    pub fn missing_count(&self, chunk_addr: &ChunkAddress) -> usize {
        self.waiting
            .get(chunk_addr)
            .map_or(0, |set| set.len())
    }

    fn is_ready(state: ChunkState) -> bool {
        matches!(
            state,
            ChunkState::Generated
                | ChunkState::Meshing
                | ChunkState::Meshed
                | ChunkState::Active
        )
    }
}
```

### Deadlock Avoidance

The dependency system cannot deadlock because generation and meshing are separate phases:

1. **Generation has no dependencies.** Any chunk can be generated independently — it only needs the terrain seed and its coordinates. Chunks are generated in parallel without waiting on each other.
2. **Meshing depends on generation, not on other meshes.** A chunk waits only for its neighbors to be *generated*, not *meshed*. Since generation is dependency-free, all chunks in a region will eventually reach `Generated`.
3. **Circular dependencies cannot form.** If chunk A is waiting for chunk B to generate, chunk B is not waiting for chunk A to generate — it is generating independently on a worker thread.

This two-phase design (generate all, then mesh all) breaks any possible dependency cycle.

### Edge Chunk Handling

Chunks at the edge of the cubesphere face or at the planet boundary may have fewer than six neighbors. For these chunks, the missing neighbor directions are treated as "ready" with air data. This is handled in `register()`: if a neighbor address is not found in `chunk_states`, it is not added to the missing set. The boundary face will be visible, which is the correct behavior (the player sees the surface of the planet).

### Bevy ECS Integration

```rust
fn on_chunk_generated(
    mut tracker: ResMut<NeighborDependencyTracker>,
    mut events: EventReader<ChunkGeneratedEvent>,
    mut commands: Commands,
) {
    for event in events.read() {
        let unblocked = tracker.notify_generated(event.address);
        for addr in unblocked {
            // Transition these chunks to Meshing
            commands.entity(addr.entity()).insert(ReadyToMesh);
        }
    }
}
```

## Outcome

The `nebula_chunk` crate exports `FaceNeighbor`, `NeighborDependencyTracker`, and the associated ECS systems. Chunks are never meshed before all their face-adjacent neighbors are generated. When a chunk finishes generating, the tracker notifies all waiting chunks and automatically unblocks those that are now ready. Edge chunks at planet boundaries mesh correctly with air boundaries. Running `cargo test -p nebula_chunk` passes all neighbor dependency tests.

## Demo Integration

**Demo crate:** `nebula-demo`

A chunk waits for all 6 face-neighbors to be generated before meshing (needed for correct face culling at boundaries). The state machine handles this automatically.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.18` | `Resource` derive, ECS systems, event handling |

The dependency tracker uses only `std::collections::HashMap` and `std::collections::HashSet`. No external graph or dependency resolution crate is needed. The crate uses Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn addr(x: i64, y: i64, z: i64) -> ChunkAddress {
        ChunkAddress::new(x as i128, y as i128, z as i128)
    }

    fn make_states(addrs: &[(ChunkAddress, ChunkState)]) -> HashMap<ChunkAddress, ChunkState> {
        addrs.iter().cloned().collect()
    }

    /// A chunk with all 6 neighbors already Generated should not be added to
    /// the waiting map — it is immediately meshable.
    #[test]
    fn test_chunk_with_all_neighbors_generated_can_be_meshed() {
        let center = addr(5, 5, 5);
        let states = make_states(&[
            (addr(6, 5, 5), ChunkState::Generated),
            (addr(4, 5, 5), ChunkState::Generated),
            (addr(5, 6, 5), ChunkState::Generated),
            (addr(5, 4, 5), ChunkState::Generated),
            (addr(5, 5, 6), ChunkState::Generated),
            (addr(5, 5, 4), ChunkState::Generated),
        ]);

        let mut tracker = NeighborDependencyTracker::default();
        tracker.register(center, &states);

        assert!(!tracker.is_waiting(&center));
        assert_eq!(tracker.missing_count(&center), 0);
    }

    /// A chunk with one missing neighbor should wait.
    #[test]
    fn test_chunk_with_missing_neighbor_waits() {
        let center = addr(5, 5, 5);
        let states = make_states(&[
            (addr(6, 5, 5), ChunkState::Generating), // not ready!
            (addr(4, 5, 5), ChunkState::Generated),
            (addr(5, 6, 5), ChunkState::Generated),
            (addr(5, 4, 5), ChunkState::Generated),
            (addr(5, 5, 6), ChunkState::Generated),
            (addr(5, 5, 4), ChunkState::Generated),
        ]);

        let mut tracker = NeighborDependencyTracker::default();
        tracker.register(center, &states);

        assert!(tracker.is_waiting(&center));
        assert_eq!(tracker.missing_count(&center), 1);
    }

    /// When a neighbor finishes generating, the waiting chunk is unblocked.
    #[test]
    fn test_neighbor_generation_unblocks_meshing() {
        let center = addr(5, 5, 5);
        let missing_neighbor = addr(6, 5, 5);

        let states = make_states(&[
            (missing_neighbor, ChunkState::Generating),
            (addr(4, 5, 5), ChunkState::Generated),
            (addr(5, 6, 5), ChunkState::Generated),
            (addr(5, 4, 5), ChunkState::Generated),
            (addr(5, 5, 6), ChunkState::Generated),
            (addr(5, 5, 4), ChunkState::Generated),
        ]);

        let mut tracker = NeighborDependencyTracker::default();
        tracker.register(center, &states);
        assert!(tracker.is_waiting(&center));

        // The missing neighbor finishes generating
        let unblocked = tracker.notify_generated(missing_neighbor);

        assert_eq!(unblocked, vec![center]);
        assert!(!tracker.is_waiting(&center));
    }

    /// Two adjacent chunks waiting on each other's generation should not
    /// deadlock — both generate independently and then both unblock.
    #[test]
    fn test_no_deadlock_with_circular_neighbors() {
        let chunk_a = addr(5, 5, 5);
        let chunk_b = addr(6, 5, 5); // +X neighbor of A

        // Both are still Generating; each is the other's neighbor.
        // Other neighbors are all Generated for simplicity.
        let mut states = HashMap::new();
        for dir in FaceNeighbor::ALL {
            let n = dir.offset_from(&chunk_a);
            if n != chunk_b {
                states.insert(n, ChunkState::Generated);
            }
        }
        for dir in FaceNeighbor::ALL {
            let n = dir.offset_from(&chunk_b);
            if n != chunk_a {
                states.insert(n, ChunkState::Generated);
            }
        }
        states.insert(chunk_a, ChunkState::Generating);
        states.insert(chunk_b, ChunkState::Generating);

        let mut tracker = NeighborDependencyTracker::default();
        tracker.register(chunk_a, &states);
        tracker.register(chunk_b, &states);

        assert!(tracker.is_waiting(&chunk_a));
        assert!(tracker.is_waiting(&chunk_b));

        // A finishes generating
        let unblocked_a = tracker.notify_generated(chunk_a);
        assert_eq!(unblocked_a, vec![chunk_b], "B should be unblocked");

        // B finishes generating
        let unblocked_b = tracker.notify_generated(chunk_b);
        assert_eq!(unblocked_b, vec![chunk_a], "A should be unblocked");

        // Neither is waiting anymore — no deadlock
        assert!(!tracker.is_waiting(&chunk_a));
        assert!(!tracker.is_waiting(&chunk_b));
    }

    /// An edge chunk (at planet boundary) with no outer neighbor should
    /// treat that missing neighbor as ready and not block on it.
    #[test]
    fn test_edge_chunks_handle_missing_outer_neighbor() {
        let edge_chunk = addr(0, 5, 5);

        // Only 5 neighbors exist; the -X neighbor at (-1,5,5) is not in states
        // (beyond the planet boundary).
        let states = make_states(&[
            (addr(1, 5, 5), ChunkState::Generated),
            (addr(0, 6, 5), ChunkState::Generated),
            (addr(0, 4, 5), ChunkState::Generated),
            (addr(0, 5, 6), ChunkState::Generated),
            (addr(0, 5, 4), ChunkState::Generated),
            // addr(-1, 5, 5) intentionally absent
        ]);

        let mut tracker = NeighborDependencyTracker::default();
        tracker.register(edge_chunk, &states);

        // Should NOT be waiting — missing neighbor treated as air/ready
        assert!(!tracker.is_waiting(&edge_chunk));
    }

    /// Multiple chunks waiting on the same neighbor are all unblocked
    /// when that neighbor generates.
    #[test]
    fn test_multiple_chunks_unblocked_by_single_neighbor() {
        let shared_neighbor = addr(5, 5, 5);
        let waiter_a = addr(6, 5, 5); // shared_neighbor is its -X
        let waiter_b = addr(4, 5, 5); // shared_neighbor is its +X

        // Set up so that both waiters are only missing the shared neighbor
        let mut states = HashMap::new();
        states.insert(shared_neighbor, ChunkState::Generating);

        // Fill in all other neighbors as Generated
        for dir in FaceNeighbor::ALL {
            let n = dir.offset_from(&waiter_a);
            if n != shared_neighbor {
                states.insert(n, ChunkState::Generated);
            }
            let n = dir.offset_from(&waiter_b);
            if n != shared_neighbor {
                states.insert(n, ChunkState::Generated);
            }
        }

        let mut tracker = NeighborDependencyTracker::default();
        tracker.register(waiter_a, &states);
        tracker.register(waiter_b, &states);

        let mut unblocked = tracker.notify_generated(shared_neighbor);
        unblocked.sort();

        let mut expected = vec![waiter_a, waiter_b];
        expected.sort();
        assert_eq!(unblocked, expected);
    }
}
```
