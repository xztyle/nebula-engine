//! Priority queue for ordering chunk generation/meshing tasks by visual importance.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

use nebula_cubesphere::ChunkAddress;

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
#[must_use]
pub fn compute_priority(factors: &ChunkPriorityFactors) -> f64 {
    let mut score = 0.0;

    // Distance: closer chunks have exponentially higher priority.
    let distance_clamped = factors.distance.max(1.0);
    score += 10_000.0 / (distance_clamped * distance_clamped);

    // LOD level: lower LOD numbers (higher detail) get a bonus.
    score += 100.0 / (1_u32 << factors.lod) as f64;

    // Frustum visibility: in-frustum chunks get a 10x multiplier.
    if factors.in_frustum {
        score *= 10.0;
    }

    // Direction: chunks ahead of the camera get a bonus.
    score += factors.direction_dot as f64 * 50.0;

    score
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
        self.priority
            .partial_cmp(&other.priority)
            .unwrap_or(Ordering::Equal)
    }
}

/// Priority queue for chunk generation and meshing tasks.
///
/// Orders work items by visual importance so the most impactful chunks
/// are processed first each frame. Supports insertion, priority updates,
/// and efficient extraction of the highest-priority item.
pub struct LodPriorityQueue {
    heap: BinaryHeap<PriorityEntry>,
    /// Maps chunk addresses to their current generation counter.
    generations: HashMap<ChunkAddress, u64>,
    /// Monotonically increasing generation counter.
    next_generation: u64,
}

impl Default for LodPriorityQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl LodPriorityQueue {
    /// Create a new empty priority queue.
    #[must_use]
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
        let generation = self.next_generation;
        self.next_generation += 1;
        self.generations.insert(address, generation);
        self.heap.push(PriorityEntry {
            address,
            priority,
            generation,
        });
    }

    /// Remove and return the highest-priority chunk address.
    /// Returns `None` if the queue is empty.
    pub fn pop(&mut self) -> Option<ChunkAddress> {
        while let Some(entry) = self.heap.pop() {
            if let Some(&current_gen) = self.generations.get(&entry.address)
                && current_gen == entry.generation
            {
                self.generations.remove(&entry.address);
                return Some(entry.address);
            }
            // Stale entry — skip it
        }
        None
    }

    /// Number of valid entries in the queue.
    #[must_use]
    pub fn len(&self) -> usize {
        self.generations.len()
    }

    /// Whether the queue is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.generations.is_empty()
    }

    /// Clear all entries from the queue.
    pub fn clear(&mut self) {
        self.heap.clear();
        self.generations.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_cubesphere::CubeFace;

    fn make_address(face: u8, lod: u8, x: u32, y: u32) -> ChunkAddress {
        let f = CubeFace::ALL[face as usize % 6];
        let grid = ChunkAddress::grid_size(lod);
        ChunkAddress::new(f, lod, x.min(grid - 1), y.min(grid - 1))
    }

    /// A closer chunk should have higher priority than a farther chunk.
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
        assert!(
            close > far,
            "close priority ({close}) should exceed far ({far})"
        );
    }

    /// In-frustum chunks should have higher priority than out-of-frustum chunks.
    #[test]
    fn test_in_frustum_beats_out_of_frustum() {
        let in_f = compute_priority(&ChunkPriorityFactors {
            distance: 500.0,
            lod: 0,
            in_frustum: true,
            direction_dot: 0.0,
        });
        let out_f = compute_priority(&ChunkPriorityFactors {
            distance: 500.0,
            lod: 0,
            in_frustum: false,
            direction_dot: 0.0,
        });
        assert!(
            in_f > out_f,
            "in-frustum ({in_f}) should beat out-of-frustum ({out_f})"
        );
    }

    /// The queue should return the highest-priority chunk first.
    #[test]
    fn test_queue_returns_highest_priority_first() {
        let mut queue = LodPriorityQueue::new();
        let low = make_address(0, 3, 1, 0);
        let mid = make_address(0, 1, 2, 0);
        let high = make_address(0, 1, 3, 0);
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
        let addr_a = make_address(0, 10, 1, 0);
        let addr_b = make_address(0, 10, 2, 0);
        queue.push(addr_a, 100.0);
        queue.push(addr_b, 50.0);
        // Camera moves, now B is higher priority
        queue.push(addr_a, 30.0);
        queue.push(addr_b, 90.0);
        assert_eq!(queue.pop(), Some(addr_b));
        assert_eq!(queue.pop(), Some(addr_a));
    }

    /// Queue length should reflect the number of valid entries.
    #[test]
    fn test_queue_length() {
        let mut queue = LodPriorityQueue::new();
        assert_eq!(queue.len(), 0);
        queue.push(make_address(0, 10, 1, 0), 10.0);
        assert_eq!(queue.len(), 1);
        queue.push(make_address(0, 10, 2, 0), 20.0);
        assert_eq!(queue.len(), 2);
        queue.pop();
        assert_eq!(queue.len(), 1);
    }
}
