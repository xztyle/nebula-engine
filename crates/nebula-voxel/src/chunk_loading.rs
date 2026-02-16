//! Chunk loading/unloading system with hysteresis and per-tick budgeting.
//!
//! Manages dynamic loading of chunks around a camera position, using a priority
//! queue (nearest-first) and configurable load/unload radii with a hysteresis
//! band to prevent thrashing.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use rustc_hash::FxHashSet;

use crate::chunk_api::{Chunk, SAVE_DIRTY};
use crate::chunk_manager::{ChunkAddress, ChunkManager};

/// Configuration for the chunk loading/unloading system.
#[derive(Clone, Debug)]
pub struct ChunkLoadConfig {
    /// Chunks within this radius (in chunk units) around the camera are loaded.
    pub load_radius: u32,
    /// Chunks beyond this radius are unloaded.
    /// Must be > `load_radius` to create a hysteresis band.
    pub unload_radius: u32,
    /// Maximum number of chunk load operations per tick.
    pub loads_per_tick: u32,
    /// Maximum number of chunk unload operations per tick.
    pub unloads_per_tick: u32,
}

impl Default for ChunkLoadConfig {
    fn default() -> Self {
        Self {
            load_radius: 8,
            unload_radius: 10,
            loads_per_tick: 4,
            unloads_per_tick: 8,
        }
    }
}

/// Priority queue for chunks awaiting loading, ordered by distance to camera.
///
/// Uses a min-heap so that the nearest chunks are loaded first.
#[derive(Debug)]
pub struct ChunkLoadQueue {
    /// Min-heap: `(distance_squared, ChunkAddress)`.
    queue: BinaryHeap<Reverse<(u64, ChunkAddress)>>,
    /// Addresses already in the queue (dedup guard).
    pending: FxHashSet<ChunkAddress>,
}

impl ChunkLoadQueue {
    /// Creates an empty load queue.
    pub fn new() -> Self {
        Self {
            queue: BinaryHeap::new(),
            pending: FxHashSet::default(),
        }
    }

    /// Enqueues a chunk address with its squared distance to the camera.
    ///
    /// Duplicate addresses are silently ignored.
    pub fn enqueue(&mut self, addr: ChunkAddress, dist_sq: u64) {
        if self.pending.insert(addr) {
            self.queue.push(Reverse((dist_sq, addr)));
        }
    }

    /// Dequeues the nearest chunk. Returns `None` if the queue is empty.
    pub fn dequeue(&mut self) -> Option<(u64, ChunkAddress)> {
        while let Some(Reverse((dist_sq, addr))) = self.queue.pop() {
            if self.pending.remove(&addr) {
                return Some((dist_sq, addr));
            }
            // Entry was removed externally (e.g. already loaded); skip it.
        }
        None
    }

    /// Returns true if the queue contains no pending addresses.
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Number of pending addresses in the queue.
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    /// Clears the queue entirely.
    pub fn clear(&mut self) {
        self.queue.clear();
        self.pending.clear();
    }
}

impl Default for ChunkLoadQueue {
    fn default() -> Self {
        Self::new()
    }
}

/// Squared Euclidean distance between two chunk addresses (ignoring face).
pub fn chunk_distance_sq(a: &ChunkAddress, b: &ChunkAddress) -> u64 {
    let dx = (a.x - b.x) as i128;
    let dy = (a.y - b.y) as i128;
    let dz = (a.z - b.z) as i128;
    (dx * dx + dy * dy + dz * dz) as u64
}

/// Result of a single loading tick.
#[derive(Debug, Default)]
pub struct ChunkLoadTickResult {
    /// Number of chunks loaded this tick.
    pub loaded: u32,
    /// Number of chunks unloaded this tick.
    pub unloaded: u32,
    /// Addresses of chunks that were unloaded and had unsaved modifications.
    pub dirty_unloaded: Vec<ChunkAddress>,
}

/// The chunk loading/unloading controller.
///
/// Call [`ChunkLoader::tick`] each frame, providing the camera's chunk address
/// and a mutable reference to the [`ChunkManager`].
#[derive(Debug)]
pub struct ChunkLoader {
    /// Configuration for radii and budgets.
    config: ChunkLoadConfig,
    /// Priority queue of chunks to load.
    load_queue: ChunkLoadQueue,
}

impl ChunkLoader {
    /// Creates a new chunk loader with the given configuration.
    pub fn new(config: ChunkLoadConfig) -> Self {
        Self {
            config,
            load_queue: ChunkLoadQueue::new(),
        }
    }

    /// Returns a reference to the current configuration.
    pub fn config(&self) -> &ChunkLoadConfig {
        &self.config
    }

    /// Returns a reference to the load queue.
    pub fn load_queue(&self) -> &ChunkLoadQueue {
        &self.load_queue
    }

    /// Runs one tick of the chunk loading/unloading system.
    ///
    /// 1. Scans for chunks within `load_radius` that are not yet loaded.
    /// 2. Loads up to `loads_per_tick` from the priority queue.
    /// 3. Unloads up to `unloads_per_tick` chunks beyond `unload_radius`.
    pub fn tick(
        &mut self,
        camera_chunk: ChunkAddress,
        manager: &mut ChunkManager,
    ) -> ChunkLoadTickResult {
        let mut result = ChunkLoadTickResult::default();

        // --- Step 1: Scan for needed chunks ---
        let lr = self.config.load_radius as i64;
        let lr_sq = (self.config.load_radius as u64) * (self.config.load_radius as u64);

        for dx in -lr..=lr {
            for dy in -lr..=lr {
                for dz in -lr..=lr {
                    let dist_sq = (dx * dx + dy * dy + dz * dz) as u64;
                    if dist_sq > lr_sq {
                        continue;
                    }
                    let addr = ChunkAddress::new(
                        camera_chunk.x + dx,
                        camera_chunk.y + dy,
                        camera_chunk.z + dz,
                        camera_chunk.face,
                    );
                    if manager.get_chunk(&addr).is_none() {
                        self.load_queue.enqueue(addr, dist_sq);
                    }
                }
            }
        }

        // --- Step 2: Process load queue ---
        for _ in 0..self.config.loads_per_tick {
            let Some((_dist_sq, addr)) = self.load_queue.dequeue() else {
                break;
            };
            // Skip if already loaded (could happen from a previous tick).
            if manager.get_chunk(&addr).is_some() {
                continue;
            }
            // Create an empty chunk (terrain generation would go here).
            let chunk = Chunk::new();
            manager.load_chunk(addr, chunk);
            result.loaded += 1;
        }

        // --- Step 3: Scan and unload distant chunks ---
        let ur_sq = (self.config.unload_radius as u64) * (self.config.unload_radius as u64);
        let unload_candidates: Vec<ChunkAddress> = manager
            .loaded_addresses()
            .filter(|addr| chunk_distance_sq(addr, &camera_chunk) > ur_sq)
            .copied()
            .collect();

        let mut unloaded_count = 0u32;
        for addr in unload_candidates {
            if unloaded_count >= self.config.unloads_per_tick {
                break;
            }
            // Check if dirty before unloading.
            let is_dirty = manager
                .get_chunk(&addr)
                .is_some_and(|c| c.is_dirty(SAVE_DIRTY));
            if is_dirty {
                result.dirty_unloaded.push(addr);
            }
            manager.unload_chunk(addr);
            unloaded_count += 1;
        }
        result.unloaded = unloaded_count;

        result
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(x: i64, y: i64, z: i64) -> ChunkAddress {
        ChunkAddress::new(x, y, z, 0)
    }

    #[test]
    fn test_chunks_within_radius_marked_for_load() {
        let config = ChunkLoadConfig {
            load_radius: 2,
            unload_radius: 4,
            loads_per_tick: 1000,
            unloads_per_tick: 1000,
        };
        let mut loader = ChunkLoader::new(config);
        let mut manager = ChunkManager::new();
        let camera = addr(0, 0, 0);

        // Run tick to populate queue and load all within radius.
        loader.tick(camera, &mut manager);

        // Count chunks within radius 2 sphere: all (x,y,z) where x²+y²+z² <= 4
        let mut expected = 0;
        for dx in -2i64..=2 {
            for dy in -2i64..=2 {
                for dz in -2i64..=2 {
                    if dx * dx + dy * dy + dz * dz <= 4 {
                        expected += 1;
                    }
                }
            }
        }

        assert_eq!(manager.loaded_count(), expected);
        assert!(expected > 30, "sphere of r=2 should have ~33 chunks");
    }

    #[test]
    fn test_chunks_beyond_unload_radius_marked_for_unload() {
        let config = ChunkLoadConfig {
            load_radius: 8,
            unload_radius: 10,
            loads_per_tick: 1000,
            unloads_per_tick: 1000,
        };
        let mut loader = ChunkLoader::new(config);
        let mut manager = ChunkManager::new();
        let camera = addr(0, 0, 0);

        // Pre-load chunks at distances 1..=12 along x-axis.
        for d in 1..=12_i64 {
            manager.load_chunk(addr(d, 0, 0), Chunk::new());
        }
        assert_eq!(manager.loaded_count(), 12);

        // Tick: should unload chunks at distance 11 and 12 (>10).
        let result = loader.tick(camera, &mut manager);

        assert!(result.unloaded >= 2);
        assert!(manager.get_chunk(&addr(11, 0, 0)).is_none());
        assert!(manager.get_chunk(&addr(12, 0, 0)).is_none());

        // Chunks at distance 1-10 should still be loaded.
        for d in 1..=10_i64 {
            assert!(
                manager.get_chunk(&addr(d, 0, 0)).is_some(),
                "chunk at distance {} should be retained",
                d
            );
        }
    }

    #[test]
    fn test_hysteresis_prevents_thrashing() {
        let config = ChunkLoadConfig {
            load_radius: 8,
            unload_radius: 10,
            loads_per_tick: 1000,
            unloads_per_tick: 1000,
        };
        let mut loader = ChunkLoader::new(config);
        let mut manager = ChunkManager::new();

        // Load chunk at (8, 0, 0) — distance 8 from origin.
        manager.load_chunk(addr(8, 0, 0), Chunk::new());

        // Camera at (0,0,0): chunk is at distance 8, within load_radius.
        // Move camera so chunk is at distance 9 (in hysteresis band).
        // Camera at (-1, 0, 0) → chunk distance = 9.
        let camera = addr(-1, 0, 0);
        loader.tick(camera, &mut manager);

        // Chunk should still be loaded (9 < unload_radius 10).
        assert!(
            manager.get_chunk(&addr(8, 0, 0)).is_some(),
            "chunk in hysteresis band should stay loaded"
        );

        // Move camera further: camera at (-3, 0, 0) → chunk distance = 11 > unload_radius.
        let camera2 = addr(-3, 0, 0);
        loader.tick(camera2, &mut manager);

        assert!(
            manager.get_chunk(&addr(8, 0, 0)).is_none(),
            "chunk beyond unload_radius should be unloaded"
        );
    }

    #[test]
    fn test_priority_queue_orders_by_distance() {
        let mut queue = ChunkLoadQueue::new();

        queue.enqueue(addr(5, 0, 0), 25);
        queue.enqueue(addr(2, 0, 0), 4);
        queue.enqueue(addr(8, 0, 0), 64);
        queue.enqueue(addr(1, 0, 0), 1);
        queue.enqueue(addr(3, 0, 0), 9);

        let mut distances = Vec::new();
        while let Some((dist_sq, _addr)) = queue.dequeue() {
            distances.push(dist_sq);
        }

        assert_eq!(distances, vec![1, 4, 9, 25, 64]);
    }

    #[test]
    fn test_budget_limits_loads_per_frame() {
        let config = ChunkLoadConfig {
            load_radius: 100, // Large radius so many chunks are needed.
            unload_radius: 120,
            loads_per_tick: 3,
            unloads_per_tick: 8,
        };
        let mut loader = ChunkLoader::new(config);
        let mut manager = ChunkManager::new();
        let camera = addr(0, 0, 0);

        // First tick: should load at most 3 chunks.
        let r1 = loader.tick(camera, &mut manager);
        assert_eq!(r1.loaded, 3);
        assert_eq!(manager.loaded_count(), 3);

        // Second tick: should load 3 more.
        let r2 = loader.tick(camera, &mut manager);
        assert_eq!(r2.loaded, 3);
        assert_eq!(manager.loaded_count(), 6);
    }
}
