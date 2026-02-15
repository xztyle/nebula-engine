# LOD Memory Budget

## Problem

Without a memory budget, the LOD system will continue loading chunks until system memory is exhausted. On a planetary-scale cubesphere world, the number of visible chunks grows quadratically with view distance — doubling the view distance quadruples the number of chunks. A player in orbit viewing an entire hemisphere could theoretically need millions of (coarse) chunks loaded simultaneously. Even with LOD reducing per-chunk data sizes, the aggregate memory consumption can easily exceed available RAM or VRAM. The engine needs a hard memory budget that caps total chunk data and mesh memory usage, automatically evicting the least important chunks when the budget is exceeded. This budget must be fast to track (no per-voxel accounting), configurable for different hardware profiles, and must evict chunks in priority order (farthest, coarsest, out-of-frustum chunks go first).

## Solution

Implement a `MemoryBudget` system in the `nebula_lod` crate that tracks approximate memory usage of loaded chunks and their GPU meshes, and triggers eviction when usage exceeds configurable limits.

### Memory Tracking

```rust
/// Approximate memory usage of a loaded chunk.
#[derive(Clone, Copy, Debug)]
pub struct ChunkMemoryUsage {
    /// Bytes used by voxel data (palette + bit-packed array).
    pub voxel_bytes: usize,
    /// Bytes used by the GPU mesh (vertex buffer + index buffer).
    pub mesh_bytes: usize,
}

impl ChunkMemoryUsage {
    /// Estimate memory for a chunk at the given LOD level.
    /// This is an approximation based on resolution and typical palette sizes.
    pub fn estimate(lod: u8, triangle_count: u32) -> Self {
        let resolution = 32u32 >> lod;
        let voxel_count = (resolution * resolution * resolution) as usize;

        // Estimate voxel storage: assume 4-bit palette (typical surface chunk)
        let voxel_bytes = voxel_count / 2 + 64; // +64 for palette overhead

        // Mesh: each triangle = 3 vertices * 32 bytes (pos + normal + uv + ao)
        // With index buffer: ~20 bytes per vertex + 4 bytes per index
        let mesh_bytes = triangle_count as usize * 3 * 20 + triangle_count as usize * 3 * 4;

        Self {
            voxel_bytes,
            mesh_bytes,
        }
    }

    pub fn total(&self) -> usize {
        self.voxel_bytes + self.mesh_bytes
    }
}
```

### Budget Configuration

```rust
/// Memory budget configuration.
#[derive(Clone, Debug)]
pub struct MemoryBudgetConfig {
    /// Maximum bytes for chunk voxel data. Default: 2 GB.
    pub voxel_budget: usize,
    /// Maximum bytes for chunk mesh data. Default: 1 GB.
    pub mesh_budget: usize,
}

impl Default for MemoryBudgetConfig {
    fn default() -> Self {
        Self {
            voxel_budget: 2 * 1024 * 1024 * 1024, // 2 GB
            mesh_budget: 1 * 1024 * 1024 * 1024,   // 1 GB
        }
    }
}

impl MemoryBudgetConfig {
    /// Create a budget for low-memory systems (e.g., integrated GPU).
    pub fn low() -> Self {
        Self {
            voxel_budget: 512 * 1024 * 1024,  // 512 MB
            mesh_budget: 256 * 1024 * 1024,    // 256 MB
        }
    }

    /// Create a budget for high-end systems.
    pub fn high() -> Self {
        Self {
            voxel_budget: 4 * 1024 * 1024 * 1024, // 4 GB
            mesh_budget: 2 * 1024 * 1024 * 1024,   // 2 GB
        }
    }
}
```

### Budget Tracker

```rust
use std::collections::HashMap;

/// Tracks memory usage across all loaded chunks and enforces budget limits.
pub struct MemoryBudgetTracker {
    config: MemoryBudgetConfig,
    /// Per-chunk memory usage.
    chunk_usage: HashMap<ChunkAddress, ChunkMemoryUsage>,
    /// Running total of voxel data bytes.
    total_voxel_bytes: usize,
    /// Running total of mesh data bytes.
    total_mesh_bytes: usize,
}

impl MemoryBudgetTracker {
    pub fn new(config: MemoryBudgetConfig) -> Self {
        Self {
            config,
            chunk_usage: HashMap::new(),
            total_voxel_bytes: 0,
            total_mesh_bytes: 0,
        }
    }

    /// Record that a chunk has been loaded with the given memory usage.
    pub fn on_chunk_loaded(&mut self, address: ChunkAddress, usage: ChunkMemoryUsage) {
        if let Some(old) = self.chunk_usage.insert(address, usage) {
            // Replacing an existing entry — subtract old usage first
            self.total_voxel_bytes -= old.voxel_bytes;
            self.total_mesh_bytes -= old.mesh_bytes;
        }
        self.total_voxel_bytes += usage.voxel_bytes;
        self.total_mesh_bytes += usage.mesh_bytes;
    }

    /// Record that a chunk has been unloaded.
    pub fn on_chunk_unloaded(&mut self, address: &ChunkAddress) {
        if let Some(usage) = self.chunk_usage.remove(address) {
            self.total_voxel_bytes -= usage.voxel_bytes;
            self.total_mesh_bytes -= usage.mesh_bytes;
        }
    }

    /// Check whether either budget is exceeded.
    pub fn is_over_budget(&self) -> bool {
        self.total_voxel_bytes > self.config.voxel_budget
            || self.total_mesh_bytes > self.config.mesh_budget
    }

    /// Return how many bytes over the voxel budget we are (0 if under budget).
    pub fn voxel_overage(&self) -> usize {
        self.total_voxel_bytes.saturating_sub(self.config.voxel_budget)
    }

    /// Return how many bytes over the mesh budget we are (0 if under budget).
    pub fn mesh_overage(&self) -> usize {
        self.total_mesh_bytes.saturating_sub(self.config.mesh_budget)
    }

    pub fn total_voxel_bytes(&self) -> usize {
        self.total_voxel_bytes
    }

    pub fn total_mesh_bytes(&self) -> usize {
        self.total_mesh_bytes
    }

    pub fn loaded_chunk_count(&self) -> usize {
        self.chunk_usage.len()
    }
}
```

### Eviction Strategy

When the budget is exceeded, the engine must evict chunks. Eviction candidates are sorted by priority (from the `LodPriorityQueue`) and the lowest-priority chunks are evicted first:

```rust
/// Determine which chunks to evict to bring memory usage within budget.
/// Returns chunk addresses in eviction order (lowest priority first).
pub fn select_evictions(
    tracker: &MemoryBudgetTracker,
    priorities: &HashMap<ChunkAddress, f64>,
) -> Vec<ChunkAddress> {
    if !tracker.is_over_budget() {
        return Vec::new();
    }

    // Sort loaded chunks by priority (ascending — lowest priority first)
    let mut candidates: Vec<_> = tracker.chunk_usage.keys()
        .map(|addr| {
            let priority = priorities.get(addr).copied().unwrap_or(0.0);
            (*addr, priority)
        })
        .collect();
    candidates.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut evictions = Vec::new();
    let mut freed_voxel = 0usize;
    let mut freed_mesh = 0usize;
    let voxel_target = tracker.voxel_overage();
    let mesh_target = tracker.mesh_overage();

    for (addr, _priority) in candidates {
        if freed_voxel >= voxel_target && freed_mesh >= mesh_target {
            break;
        }
        if let Some(usage) = tracker.chunk_usage.get(&addr) {
            freed_voxel += usage.voxel_bytes;
            freed_mesh += usage.mesh_bytes;
            evictions.push(addr);
        }
    }

    evictions
}
```

## Outcome

The `nebula_lod` crate exports `MemoryBudgetConfig`, `MemoryBudgetTracker`, `ChunkMemoryUsage`, and `select_evictions()`. The chunk lifecycle system calls `on_chunk_loaded()` and `on_chunk_unloaded()` as chunks flow in and out, and checks `is_over_budget()` each frame to trigger evictions. Running `cargo test -p nebula_lod` passes all memory budget tests.

## Demo Integration

**Demo crate:** `nebula-demo`

The console logs `Memory: 245 / 512 MB (chunks: 1024)`. When the budget is exceeded, distant low-priority chunks are evicted to free memory.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `nebula_lod` | workspace (self) | `ChunkAddress`, `LodPriorityQueue` from stories 06 |

No external crates required. Memory tracking uses only `std::collections::HashMap` and basic arithmetic. The crate uses Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn make_config(voxel_mb: usize, mesh_mb: usize) -> MemoryBudgetConfig {
        MemoryBudgetConfig {
            voxel_budget: voxel_mb * 1024 * 1024,
            mesh_budget: mesh_mb * 1024 * 1024,
        }
    }

    fn make_address(id: u64) -> ChunkAddress {
        ChunkAddress {
            face: CubeFace::PosY,
            quadtree_path: id,
            lod: 0,
        }
    }

    /// Loading a chunk should increase the tracked memory usage.
    #[test]
    fn test_memory_increases_on_load() {
        let mut tracker = MemoryBudgetTracker::new(MemoryBudgetConfig::default());
        assert_eq!(tracker.total_voxel_bytes(), 0);
        assert_eq!(tracker.total_mesh_bytes(), 0);

        let usage = ChunkMemoryUsage {
            voxel_bytes: 1024,
            mesh_bytes: 2048,
        };
        tracker.on_chunk_loaded(make_address(1), usage);

        assert_eq!(tracker.total_voxel_bytes(), 1024);
        assert_eq!(tracker.total_mesh_bytes(), 2048);
        assert_eq!(tracker.loaded_chunk_count(), 1);
    }

    /// Unloading a chunk should decrease the tracked memory usage.
    #[test]
    fn test_memory_decreases_on_unload() {
        let mut tracker = MemoryBudgetTracker::new(MemoryBudgetConfig::default());
        let addr = make_address(1);
        let usage = ChunkMemoryUsage {
            voxel_bytes: 1024,
            mesh_bytes: 2048,
        };

        tracker.on_chunk_loaded(addr, usage);
        tracker.on_chunk_unloaded(&addr);

        assert_eq!(tracker.total_voxel_bytes(), 0);
        assert_eq!(tracker.total_mesh_bytes(), 0);
        assert_eq!(tracker.loaded_chunk_count(), 0);
    }

    /// Exceeding the budget should be detected by is_over_budget().
    #[test]
    fn test_budget_exceeded_triggers_detection() {
        let mut tracker = MemoryBudgetTracker::new(make_config(1, 1)); // 1 MB each

        // Load chunks until over budget
        for i in 0..2000 {
            tracker.on_chunk_loaded(make_address(i), ChunkMemoryUsage {
                voxel_bytes: 1024,
                mesh_bytes: 512,
            });
        }

        // 2000 * 1024 = ~2 MB voxels, exceeding 1 MB budget
        assert!(tracker.is_over_budget());
    }

    /// Eviction should remove the lowest-priority chunks first.
    #[test]
    fn test_eviction_removes_lowest_priority_first() {
        let mut tracker = MemoryBudgetTracker::new(make_config(1, 1));
        let mut priorities = HashMap::new();

        // Load 3 chunks with different priorities
        let low = make_address(1);
        let mid = make_address(2);
        let high = make_address(3);

        for addr in [low, mid, high] {
            tracker.on_chunk_loaded(addr, ChunkMemoryUsage {
                voxel_bytes: 500 * 1024, // 500 KB each -> 1.5 MB total, over 1 MB budget
                mesh_bytes: 100 * 1024,
            });
        }

        priorities.insert(low, 10.0);
        priorities.insert(mid, 50.0);
        priorities.insert(high, 100.0);

        let evictions = select_evictions(&tracker, &priorities);

        // Lowest priority should be evicted first
        assert!(!evictions.is_empty());
        assert_eq!(evictions[0], low, "lowest priority chunk should be evicted first");
    }

    /// The budget should be configurable with custom values.
    #[test]
    fn test_budget_can_be_configured() {
        let config = make_config(4096, 2048); // 4 GB voxels, 2 GB meshes
        let tracker = MemoryBudgetTracker::new(config);

        assert!(!tracker.is_over_budget()); // empty tracker is never over budget

        // Verify the config values are stored correctly
        let config_low = MemoryBudgetConfig::low();
        assert_eq!(config_low.voxel_budget, 512 * 1024 * 1024);
        assert_eq!(config_low.mesh_budget, 256 * 1024 * 1024);

        let config_high = MemoryBudgetConfig::high();
        assert_eq!(config_high.voxel_budget, 4 * 1024 * 1024 * 1024);
        assert_eq!(config_high.mesh_budget, 2 * 1024 * 1024 * 1024);
    }
}
```
