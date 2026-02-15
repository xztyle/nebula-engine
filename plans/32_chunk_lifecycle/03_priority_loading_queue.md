# Priority Loading Queue

## Problem

When a player enters a new area or the game first loads, hundreds of chunks may simultaneously fall within the load radius. Loading them in arbitrary order (e.g., HashMap iteration order) produces a poor player experience: distant chunks behind the camera appear before nearby chunks directly in front of the player. The meshing and generation budgets (story 05) limit how many chunks can be processed per frame, so the order in which chunks are queued has a major impact on perceived load time. Chunks directly ahead of the camera and closest to the player must be prioritized so the visible world fills in naturally from near to far.

## Solution

Implement a priority loading queue in the `nebula_chunk` crate that orders chunks by a composite priority score, ensuring the most important chunks are generated and meshed first.

### Priority Score Computation

```rust
use crate::coords::ChunkAddress;

/// Weights for each priority factor. Tunable at runtime.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PriorityWeights {
    /// Weight for distance-based priority (higher = distance matters more).
    pub distance: f32,
    /// Weight for movement-direction bias (chunks ahead of the camera).
    pub direction: f32,
    /// Weight for LOD level (lower LOD = drawn at larger scale = more important).
    pub lod: f32,
    /// Weight for frustum visibility (in-frustum chunks get a boost).
    pub visibility: f32,
}

impl Default for PriorityWeights {
    fn default() -> Self {
        Self {
            distance: 1.0,
            direction: 0.5,
            lod: 0.3,
            visibility: 0.8,
        }
    }
}

/// Input data needed to compute a chunk's loading priority.
pub struct PriorityInput {
    /// Chebyshev distance from the camera in chunk units.
    pub distance: u32,
    /// Maximum possible load distance (used for normalization).
    pub max_distance: u32,
    /// Dot product of (chunk direction from camera) and (camera forward),
    /// ranging from -1.0 (directly behind) to 1.0 (directly ahead).
    pub direction_dot: f32,
    /// LOD level for this chunk (0 = highest detail, higher = coarser).
    pub lod_level: u8,
    /// Maximum LOD level in the system.
    pub max_lod: u8,
    /// Whether the chunk is within the camera's view frustum.
    pub in_frustum: bool,
}

/// Compute a priority score. Higher score = higher priority (loaded first).
pub fn compute_priority(input: &PriorityInput, weights: &PriorityWeights) -> f32 {
    // Distance: closer = higher priority. Normalize to [0, 1].
    let distance_score = 1.0 - (input.distance as f32 / input.max_distance.max(1) as f32).min(1.0);

    // Direction: chunks ahead of the camera get a boost.
    // Remap from [-1, 1] to [0, 1].
    let direction_score = (input.direction_dot + 1.0) * 0.5;

    // LOD: lower LOD level = more important (covers more visual area).
    let lod_score = 1.0 - (input.lod_level as f32 / input.max_lod.max(1) as f32);

    // Visibility: binary boost for chunks in the view frustum.
    let visibility_score = if input.in_frustum { 1.0 } else { 0.0 };

    distance_score * weights.distance
        + direction_score * weights.direction
        + lod_score * weights.lod
        + visibility_score * weights.visibility
}
```

### Priority Queue

```rust
use std::collections::BinaryHeap;
use std::cmp::Ordering;

/// An entry in the priority loading queue.
#[derive(Debug)]
struct PriorityEntry {
    pub address: ChunkAddress,
    pub priority: f32,
}

impl PartialEq for PriorityEntry {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority
    }
}

impl Eq for PriorityEntry {}

impl PartialOrd for PriorityEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PriorityEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        self.priority
            .partial_cmp(&other.priority)
            .unwrap_or(Ordering::Equal)
    }
}

/// A priority queue that orders chunk loading by importance.
pub struct ChunkLoadQueue {
    heap: BinaryHeap<PriorityEntry>,
    weights: PriorityWeights,
}

impl ChunkLoadQueue {
    pub fn new(weights: PriorityWeights) -> Self {
        Self {
            heap: BinaryHeap::new(),
            weights,
        }
    }

    /// Insert a chunk with a computed priority.
    pub fn enqueue(&mut self, address: ChunkAddress, input: &PriorityInput) {
        let priority = compute_priority(input, &self.weights);
        self.heap.push(PriorityEntry { address, priority });
    }

    /// Remove and return the highest-priority chunk address.
    pub fn dequeue(&mut self) -> Option<ChunkAddress> {
        self.heap.pop().map(|entry| entry.address)
    }

    /// Peek at the highest-priority entry without removing it.
    pub fn peek_priority(&self) -> Option<f32> {
        self.heap.peek().map(|entry| entry.priority)
    }

    /// Number of chunks waiting in the queue.
    pub fn len(&self) -> usize {
        self.heap.len()
    }

    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }

    /// Drain the queue completely and rebuild with new priorities.
    /// Called when the camera moves significantly (e.g., crosses a chunk boundary).
    pub fn rebuild(
        &mut self,
        chunks: impl Iterator<Item = (ChunkAddress, PriorityInput)>,
    ) {
        self.heap.clear();
        for (address, input) in chunks {
            self.enqueue(address, &input);
        }
    }
}
```

### Camera Movement Threshold

Re-prioritization is not done every frame. Instead, the system tracks the camera's chunk address. When the camera crosses a chunk boundary (its chunk address changes), a full re-prioritization pass runs. Minor sub-chunk movement within the same chunk does not trigger a rebuild.

```rust
use bevy_ecs::prelude::*;

#[derive(Resource)]
pub struct CameraChunkTracker {
    pub last_chunk: Option<ChunkAddress>,
}

fn reprioritize_on_camera_move(
    camera_query: Query<&ChunkAddress, With<Camera>>,
    mut tracker: ResMut<CameraChunkTracker>,
    mut load_queue: ResMut<ChunkLoadQueue>,
    pending_chunks: Query<(&ChunkAddress, &ChunkStateMachine), With<PendingLoad>>,
) {
    let Ok(camera_addr) = camera_query.single() else { return };

    let moved = tracker.last_chunk.map_or(true, |last| last != *camera_addr);
    if !moved {
        return;
    }
    tracker.last_chunk = Some(*camera_addr);

    // Rebuild priorities for all pending chunks
    let inputs = pending_chunks.iter().map(|(addr, _)| {
        let input = compute_priority_input(addr, camera_addr);
        (*addr, input)
    });
    load_queue.rebuild(inputs);
}
```

## Outcome

The `nebula_chunk` crate exports `ChunkLoadQueue`, `PriorityWeights`, `PriorityInput`, and `compute_priority()`. Chunks are loaded in perceptually optimal order: closest visible chunks in the camera's forward direction appear first. The queue is rebuilt when the camera crosses chunk boundaries. Running `cargo test -p nebula_chunk` passes all priority queue tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Chunks closest to the player and in the view frustum load first. Looking left while moving right loads left-facing chunks before right-facing ones.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.18` | ECS system, `Resource` derives, queries for camera tracking |
| `serde` | `1.0` | Serialize/deserialize `PriorityWeights` for settings |

The priority queue uses `std::collections::BinaryHeap` from the standard library. No external priority queue crate is needed. The crate uses Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn default_weights() -> PriorityWeights {
        PriorityWeights::default()
    }

    fn make_input(distance: u32, direction_dot: f32, in_frustum: bool) -> PriorityInput {
        PriorityInput {
            distance,
            max_distance: 16,
            direction_dot,
            lod_level: 0,
            max_lod: 4,
            in_frustum,
        }
    }

    /// The closest chunk should have a higher priority than a distant chunk.
    #[test]
    fn test_closest_chunk_loads_first() {
        let weights = default_weights();
        let close = make_input(2, 0.0, true);
        let far = make_input(14, 0.0, true);

        let close_priority = compute_priority(&close, &weights);
        let far_priority = compute_priority(&far, &weights);

        assert!(close_priority > far_priority,
            "close ({close_priority}) should be > far ({far_priority})");
    }

    /// A chunk ahead of the camera should be prioritized over one behind.
    #[test]
    fn test_chunk_ahead_of_movement_direction_is_prioritized() {
        let weights = default_weights();
        let ahead = make_input(8, 1.0, true);   // directly ahead
        let behind = make_input(8, -1.0, true);  // directly behind

        let ahead_priority = compute_priority(&ahead, &weights);
        let behind_priority = compute_priority(&behind, &weights);

        assert!(ahead_priority > behind_priority,
            "ahead ({ahead_priority}) should be > behind ({behind_priority})");
    }

    /// Visible (in-frustum) chunks should be prioritized over occluded ones.
    #[test]
    fn test_visible_chunks_prioritized_over_occluded() {
        let weights = default_weights();
        let visible = make_input(8, 0.0, true);
        let occluded = make_input(8, 0.0, false);

        let vis_priority = compute_priority(&visible, &weights);
        let occ_priority = compute_priority(&occluded, &weights);

        assert!(vis_priority > occ_priority,
            "visible ({vis_priority}) should be > occluded ({occ_priority})");
    }

    /// When the camera moves (rebuild called), priorities should update.
    #[test]
    fn test_priority_updates_on_camera_move() {
        let weights = default_weights();
        let mut queue = ChunkLoadQueue::new(weights.clone());

        let addr_a = ChunkAddress::new(0, 0, 5);
        let addr_b = ChunkAddress::new(0, 0, 10);

        // Initially, A is closer
        queue.enqueue(addr_a, &make_input(5, 0.5, true));
        queue.enqueue(addr_b, &make_input(10, 0.5, true));

        let first = queue.dequeue().unwrap();
        assert_eq!(first, addr_a, "A should be first (closer)");

        // Camera moves; now B is closer
        let mut queue2 = ChunkLoadQueue::new(weights);
        queue2.enqueue(addr_a, &make_input(12, -0.5, false));
        queue2.enqueue(addr_b, &make_input(3, 0.9, true));

        let first_after_move = queue2.dequeue().unwrap();
        assert_eq!(first_after_move, addr_b, "B should be first after camera move");
    }

    /// The queue should dequeue highest priority first.
    #[test]
    fn test_dequeue_order_is_highest_priority_first() {
        let weights = default_weights();
        let mut queue = ChunkLoadQueue::new(weights);

        let addr_near = ChunkAddress::new(1, 0, 0);
        let addr_mid = ChunkAddress::new(5, 0, 0);
        let addr_far = ChunkAddress::new(15, 0, 0);

        queue.enqueue(addr_far, &make_input(15, 0.0, true));
        queue.enqueue(addr_near, &make_input(1, 0.0, true));
        queue.enqueue(addr_mid, &make_input(5, 0.0, true));

        assert_eq!(queue.dequeue().unwrap(), addr_near);
        assert_eq!(queue.dequeue().unwrap(), addr_mid);
        assert_eq!(queue.dequeue().unwrap(), addr_far);
    }

    /// An empty queue returns None on dequeue.
    #[test]
    fn test_empty_queue_returns_none() {
        let queue = ChunkLoadQueue::new(default_weights());
        assert!(queue.is_empty());
        assert_eq!(queue.len(), 0);
    }
}
```
