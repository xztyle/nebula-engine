# LOD Priority Queue

## Problem

The engine must generate terrain data and mesh geometry for hundreds of chunks every frame as the camera moves. Not all chunk operations are equally urgent — a chunk directly in front of the camera at close range is far more important than a distant chunk behind the camera. Without prioritization, the engine might spend its per-frame budget generating distant low-priority chunks while the player stares at missing terrain in front of them. The chunk generation pipeline needs a priority queue that orders work items by visual importance so that the most impactful chunks are processed first, ensuring the player always sees complete terrain in their immediate vicinity even if distant terrain is still loading.

## Solution

Implement an `LodPriorityQueue` in the `nebula_lod` crate that orders chunk generation and meshing tasks by a composite priority score. The queue supports insertion, removal, priority updates, and efficient extraction of the highest-priority item.

### Priority Scoring

```rust
/// Factors that determine a chunk's generation priority.
#[derive(Clone, Debug)]
pub struct ChunkPriorityFactors {
    /// Euclidean distance from the camera to the chunk center.
    pub distance: f64,
    /// LOD level of the chunk (0 = highest detail, higher = coarser).
    pub lod: u8,
    /// Whether the chunk is inside the camera's view frustum.
    pub in_frustum: bool,
    /// Dot product between the camera's forward vector and the direction to the chunk.
    /// Ranges from -1.0 (directly behind) to 1.0 (directly ahead).
    pub direction_dot: f32,
}

/// Compute a priority score from the given factors.
/// Higher scores mean higher priority (processed first).
pub fn compute_priority(factors: &ChunkPriorityFactors) -> f64 {
    let mut score = 0.0;

    // Distance: closer chunks have exponentially higher priority.
    // Use inverse square so nearby chunks dominate.
    let distance_clamped = factors.distance.max(1.0);
    score += 10_000.0 / (distance_clamped * distance_clamped);

    // LOD level: lower LOD numbers (higher detail) get a bonus.
    // LOD 0 gets +100, LOD 1 gets +50, LOD 2 gets +25, etc.
    score += 100.0 / (1 << factors.lod) as f64;

    // Frustum visibility: in-frustum chunks get a 10x multiplier.
    if factors.in_frustum {
        score *= 10.0;
    }

    // Direction of movement: chunks ahead of the camera get a bonus.
    // direction_dot of 1.0 (directly ahead) adds +50, 0.0 adds 0, -1.0 subtracts 50.
    score += factors.direction_dot as f64 * 50.0;

    score
}
```

### Queue Data Structure

The priority queue is implemented as a binary heap (via `BinaryHeap`) with a `HashMap` side-table for O(1) address lookups to support priority updates and duplicate prevention.

```rust
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

/// A chunk address uniquely identifying a chunk in the world.
/// Combines the cube face, quadtree path, and LOD level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ChunkAddress {
    pub face: CubeFace,
    pub quadtree_path: u64,
    pub lod: u8,
}

/// An entry in the priority queue.
#[derive(Clone, Debug)]
struct PriorityEntry {
    address: ChunkAddress,
    priority: f64,
    /// Generation counter to handle stale entries after priority updates.
    generation: u64,
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
        self.priority.partial_cmp(&other.priority)
            .unwrap_or(Ordering::Equal)
    }
}

/// Priority queue for chunk generation and meshing tasks.
pub struct LodPriorityQueue {
    heap: BinaryHeap<PriorityEntry>,
    /// Maps chunk addresses to their current generation counter.
    /// Used to invalidate stale entries after priority updates.
    generations: HashMap<ChunkAddress, u64>,
    /// Monotonically increasing generation counter.
    next_generation: u64,
}

impl LodPriorityQueue {
    pub fn new() -> Self {
        Self {
            heap: BinaryHeap::new(),
            generations: HashMap::new(),
            next_generation: 0,
        }
    }

    /// Insert or update a chunk's priority.
    /// If the chunk is already in the queue, its priority is updated.
    pub fn push(&mut self, address: ChunkAddress, priority: f64) {
        let gen = self.next_generation;
        self.next_generation += 1;
        self.generations.insert(address, gen);
        self.heap.push(PriorityEntry {
            address,
            priority,
            generation: gen,
        });
    }

    /// Remove and return the highest-priority chunk address.
    /// Returns `None` if the queue is empty.
    pub fn pop(&mut self) -> Option<ChunkAddress> {
        while let Some(entry) = self.heap.pop() {
            // Check if this entry is still current (not invalidated by an update)
            if let Some(&current_gen) = self.generations.get(&entry.address) {
                if current_gen == entry.generation {
                    self.generations.remove(&entry.address);
                    return Some(entry.address);
                }
            }
            // Stale entry — skip it
        }
        None
    }

    /// Number of valid entries in the queue.
    pub fn len(&self) -> usize {
        self.generations.len()
    }

    /// Whether the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.generations.is_empty()
    }

    /// Clear all entries from the queue.
    pub fn clear(&mut self) {
        self.heap.clear();
        self.generations.clear();
    }
}
```

### Per-Frame Update

Each frame, after the quadtree LOD pass determines which chunks should be active, the priority queue is rebuilt with the current priorities:

```rust
pub fn rebuild_priorities(
    queue: &mut LodPriorityQueue,
    active_chunks: &[LodChunkDescriptor],
    loaded_chunks: &HashSet<ChunkAddress>,
    camera: &Camera,
) {
    queue.clear();

    for desc in active_chunks {
        if loaded_chunks.contains(&desc.address) {
            continue; // already loaded, no work needed
        }

        let factors = ChunkPriorityFactors {
            distance: desc.distance_to_camera(camera),
            lod: desc.lod,
            in_frustum: camera.frustum().contains_sphere(&desc.bounding_sphere),
            direction_dot: camera.forward().dot(
                (desc.center() - camera.position()).normalize()
            ),
        };

        queue.push(desc.address, compute_priority(&factors));
    }
}
```

## Outcome

The `nebula_lod` crate exports `LodPriorityQueue`, `ChunkAddress`, `ChunkPriorityFactors`, and `compute_priority()`. The chunk generation system pops items from this queue each frame (up to its per-frame budget) to determine which chunks to generate next. Running `cargo test -p nebula_lod` passes all priority queue tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Chunks closest to the camera and in the view frustum load first. Turning the camera causes the visible area to fill in before the periphery. The console logs the queue depth each frame.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `nebula_math` | workspace | Vector types for direction calculations |
| `nebula_cubesphere` | workspace | `CubeFace` enum |

No external crates required. The priority queue uses `std::collections::BinaryHeap` and `HashMap` from the standard library. The crate uses Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn make_address(face: u8, path: u64, lod: u8) -> ChunkAddress {
        ChunkAddress {
            face: CubeFace::from_index(face),
            quadtree_path: path,
            lod,
        }
    }

    /// A closer chunk should have higher priority than a farther chunk
    /// (all other factors equal).
    #[test]
    fn test_closer_chunk_has_higher_priority() {
        let close = compute_priority(&ChunkPriorityFactors {
            distance: 100.0,
            lod: 0,
            in_frustum: true,
            direction_dot: 0.0,
        });

        let far = compute_priority(&ChunkPriorityFactors {
            distance: 1000.0,
            lod: 0,
            in_frustum: true,
            direction_dot: 0.0,
        });

        assert!(close > far, "close priority ({close}) should exceed far ({far})");
    }

    /// In-frustum chunks should have higher priority than out-of-frustum chunks.
    #[test]
    fn test_in_frustum_beats_out_of_frustum() {
        let in_frustum = compute_priority(&ChunkPriorityFactors {
            distance: 500.0,
            lod: 0,
            in_frustum: true,
            direction_dot: 0.0,
        });

        let out_frustum = compute_priority(&ChunkPriorityFactors {
            distance: 500.0,
            lod: 0,
            in_frustum: false,
            direction_dot: 0.0,
        });

        assert!(
            in_frustum > out_frustum,
            "in-frustum ({in_frustum}) should beat out-of-frustum ({out_frustum})"
        );
    }

    /// The queue should return the highest-priority chunk first.
    #[test]
    fn test_queue_returns_highest_priority_first() {
        let mut queue = LodPriorityQueue::new();

        let low = make_address(0, 1, 3);
        let mid = make_address(0, 2, 1);
        let high = make_address(0, 3, 0);

        queue.push(low, 10.0);
        queue.push(mid, 50.0);
        queue.push(high, 100.0);

        assert_eq!(queue.pop(), Some(high));
        assert_eq!(queue.pop(), Some(mid));
        assert_eq!(queue.pop(), Some(low));
    }

    /// An empty queue should return None.
    #[test]
    fn test_empty_queue_returns_none() {
        let mut queue = LodPriorityQueue::new();
        assert_eq!(queue.pop(), None);
        assert!(queue.is_empty());
    }

    /// Updating a chunk's priority should cause the queue to return
    /// the updated priority on next pop.
    #[test]
    fn test_priority_updates_when_camera_moves() {
        let mut queue = LodPriorityQueue::new();

        let addr_a = make_address(0, 1, 0);
        let addr_b = make_address(0, 2, 0);

        // Initially A is higher priority
        queue.push(addr_a, 100.0);
        queue.push(addr_b, 50.0);

        // Camera moves, now B is higher priority
        queue.push(addr_a, 30.0);
        queue.push(addr_b, 90.0);

        // B should come out first now
        assert_eq!(queue.pop(), Some(addr_b));
        assert_eq!(queue.pop(), Some(addr_a));
    }

    /// Queue length should reflect the number of valid entries.
    #[test]
    fn test_queue_length() {
        let mut queue = LodPriorityQueue::new();
        assert_eq!(queue.len(), 0);

        queue.push(make_address(0, 1, 0), 10.0);
        assert_eq!(queue.len(), 1);

        queue.push(make_address(0, 2, 0), 20.0);
        assert_eq!(queue.len(), 2);

        queue.pop();
        assert_eq!(queue.len(), 1);
    }
}
```
