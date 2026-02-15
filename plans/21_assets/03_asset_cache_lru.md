# Asset Cache with LRU Eviction

## Problem

A cubesphere-voxel planet generates enormous numbers of unique assets — each chunk face may reference distinct textures, each biome has its own material set, and the player can teleport across vast 128-bit coordinate spaces where entirely different asset sets are needed. Loading everything into memory simultaneously is infeasible. The engine needs a cache that holds recently-used assets in memory, automatically evicts the least-recently-used ones when a configurable memory budget is exceeded, and transparently re-loads evicted assets when they are needed again.

Without eviction, GPU and CPU memory grow without bound until the system OOMs. Without a "pinning" mechanism, the cache might evict an asset that a currently-visible mesh is actively referencing, causing a visible pop or crash. Without transparent re-loading, eviction would force callers to add complex "is this still valid?" checks around every asset access.

## Solution

### LRU Cache Structure

The cache wraps a doubly-linked list for O(1) LRU ordering combined with a HashMap for O(1) key lookup. Each entry tracks its memory footprint so the cache can enforce a byte-level budget:

```rust
use std::collections::HashMap;

struct CacheEntry<T> {
    handle: Handle<T>,
    data: T,
    size_bytes: usize,
    pinned: bool,
}

pub struct AssetCache<T> {
    /// Map from handle ID to index in the order list.
    index: HashMap<u64, usize>,
    /// Ordered list: most-recently-used at the back, LRU at the front.
    order: Vec<CacheEntry<T>>,
    /// Current total memory usage in bytes.
    current_bytes: usize,
    /// Maximum allowed memory usage in bytes.
    max_bytes: usize,
}

impl<T> AssetCache<T> {
    pub fn new(max_bytes: usize) -> Self {
        Self {
            index: HashMap::new(),
            order: Vec::new(),
            current_bytes: 0,
            max_bytes,
        }
    }

    /// Insert or update an asset in the cache.
    /// `size_bytes` is the estimated memory footprint of this asset.
    pub fn insert(
        &mut self,
        handle: Handle<T>,
        data: T,
        size_bytes: usize,
    ) {
        // If the handle already exists, remove the old entry first
        if let Some(&idx) = self.index.get(&handle.id()) {
            self.current_bytes -= self.order[idx].size_bytes;
            self.remove_at_index(idx);
        }

        // Evict LRU entries until there is room
        self.evict_until_fits(size_bytes);

        let entry = CacheEntry {
            handle,
            data,
            size_bytes,
            pinned: false,
        };

        self.current_bytes += size_bytes;
        let idx = self.order.len();
        self.order.push(entry);
        self.index.insert(handle.id(), idx);
    }

    /// Access an asset, marking it as most-recently-used.
    /// Returns None if the asset is not in the cache.
    pub fn get(&mut self, handle: Handle<T>) -> Option<&T> {
        if let Some(&idx) = self.index.get(&handle.id()) {
            self.move_to_back(idx);
            // After move_to_back, the entry is at the end
            Some(&self.order.last().unwrap().data)
        } else {
            None
        }
    }

    /// Pin an asset so it cannot be evicted.
    /// Used when a handle is actively referenced by visible geometry.
    pub fn pin(&mut self, handle: Handle<T>) {
        if let Some(&idx) = self.index.get(&handle.id()) {
            self.order[idx].pinned = true;
        }
    }

    /// Unpin an asset, making it eligible for eviction again.
    pub fn unpin(&mut self, handle: Handle<T>) {
        if let Some(&idx) = self.index.get(&handle.id()) {
            self.order[idx].pinned = false;
        }
    }

    /// Returns true if the handle is in the cache.
    pub fn contains(&self, handle: Handle<T>) -> bool {
        self.index.contains_key(&handle.id())
    }

    /// Current memory usage in bytes.
    pub fn current_bytes(&self) -> usize {
        self.current_bytes
    }

    /// Maximum memory budget in bytes.
    pub fn max_bytes(&self) -> usize {
        self.max_bytes
    }

    /// Number of cached entries.
    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// Evict unpinned LRU entries until `needed_bytes` can fit
    /// within the budget.
    fn evict_until_fits(&mut self, needed_bytes: usize) {
        while self.current_bytes + needed_bytes > self.max_bytes {
            if !self.evict_one() {
                // All remaining entries are pinned — cannot free more.
                log::warn!(
                    "Asset cache over budget ({} + {} > {}), \
                     but all entries are pinned",
                    self.current_bytes,
                    needed_bytes,
                    self.max_bytes,
                );
                break;
            }
        }
    }

    /// Evict the least-recently-used unpinned entry.
    /// Returns true if an entry was evicted, false if all are pinned.
    fn evict_one(&mut self) -> bool {
        // Scan from the front (LRU end) for the first unpinned entry
        for i in 0..self.order.len() {
            if !self.order[i].pinned {
                self.current_bytes -= self.order[i].size_bytes;
                let handle_id = self.order[i].handle.id();
                self.remove_at_index(i);
                self.index.remove(&handle_id);
                log::debug!("Evicted asset with handle ID {}", handle_id);
                return true;
            }
        }
        false
    }

    fn remove_at_index(&mut self, idx: usize) {
        self.order.remove(idx);
        // Rebuild index for entries that shifted
        for (i, entry) in self.order.iter().enumerate() {
            self.index.insert(entry.handle.id(), i);
        }
    }

    fn move_to_back(&mut self, idx: usize) {
        let entry = self.order.remove(idx);
        self.order.push(entry);
        // Rebuild affected indices
        for (i, entry) in self.order.iter().enumerate() {
            self.index.insert(entry.handle.id(), i);
        }
    }
}
```

### Memory Estimation

Each asset type provides its memory footprint via a trait:

```rust
pub trait AssetSize {
    /// Estimated memory usage of this asset in bytes.
    fn size_bytes(&self) -> usize;
}

impl AssetSize for CpuTexture {
    fn size_bytes(&self) -> usize {
        self.width as usize * self.height as usize * self.bytes_per_pixel as usize
    }
}

impl AssetSize for CpuMesh {
    fn size_bytes(&self) -> usize {
        self.vertex_count * std::mem::size_of::<Vertex>()
            + self.index_count * std::mem::size_of::<u32>()
    }
}
```

### GPU Memory Eviction

When a cached asset with an associated GPU resource is evicted, the corresponding wgpu buffer or texture must also be freed. The cache emits eviction events that a GPU cleanup system processes:

```rust
pub struct EvictionEvent<T> {
    pub handle: Handle<T>,
}

impl<T> AssetCache<T> {
    /// Drain eviction events. Call once per frame.
    pub fn drain_evictions(&mut self) -> Vec<EvictionEvent<T>> {
        std::mem::take(&mut self.pending_evictions)
    }
}
```

The GPU cleanup system receives these events and calls `wgpu::Buffer::destroy()` or drops the `wgpu::Texture` for the evicted assets.

### Transparent Re-Loading

When a system tries to access an evicted asset via its `Handle<T>`, the `AssetStore<T>.get(handle)` returns `None` (the data was removed). The system then checks `AssetStore<T>.state(handle)` — if the state is `Loaded` but data is `None`, this indicates eviction. The system resubmits a load request using the original path (stored in a separate `AssetPathMap` resource). The handle transitions back to `Loading` and the asset will arrive again from disk.

```rust
pub struct AssetPathMap {
    paths: HashMap<u64, PathBuf>,
}

impl AssetPathMap {
    pub fn new() -> Self {
        Self { paths: HashMap::new() }
    }

    pub fn register<T>(&mut self, handle: Handle<T>, path: PathBuf) {
        self.paths.insert(handle.id(), path);
    }

    pub fn get_path<T>(&self, handle: Handle<T>) -> Option<&PathBuf> {
        self.paths.get(&handle.id())
    }
}
```

### Configuration

Cache limits are configurable per asset type via the engine configuration system:

```rust
pub struct CacheConfig {
    /// Maximum CPU-side texture cache in bytes. Default: 512 MB.
    pub texture_cache_bytes: usize,
    /// Maximum CPU-side mesh cache in bytes. Default: 256 MB.
    pub mesh_cache_bytes: usize,
    /// Maximum CPU-side sound cache in bytes. Default: 128 MB.
    pub sound_cache_bytes: usize,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            texture_cache_bytes: 512 * 1024 * 1024,
            mesh_cache_bytes: 256 * 1024 * 1024,
            sound_cache_bytes: 128 * 1024 * 1024,
        }
    }
}
```

## Outcome

An `AssetCache<T>` that stores loaded assets up to a configurable memory budget, evicts least-recently-used unpinned entries when the budget is exceeded, and integrates with the handle system so that eviction is transparent to callers. Pinned assets (those referenced by active handles in visible entities) are never evicted. Eviction events propagate to GPU cleanup systems. Re-loading an evicted asset uses the same handle, so no references need to be updated. The cache exposes byte-level memory tracking for profiling and debug overlays.

## Demo Integration

**Demo crate:** `nebula-demo`

The console shows cache hit/miss rates. Rarely-used assets are evicted from the LRU cache when memory pressure rises.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.15` | `Resource` derive for `AssetCache<T>` wrappers and system scheduling |
| `log` | `0.4` | Logging eviction events and cache pressure warnings |

No additional crates. The LRU data structure is implemented in-engine using `std::collections::HashMap` and `Vec`. Rust edition 2024. A production optimization would replace the `Vec`-based LRU with an intrusive doubly-linked list for true O(1) eviction, but the `Vec` approach is correct and sufficient for the initial implementation.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    struct TestAsset {
        name: String,
    }

    #[test]
    fn test_cache_stores_loaded_assets() {
        let mut cache = AssetCache::<TestAsset>::new(1024);
        let h = Handle::new();
        let asset = TestAsset { name: "texture.png".into() };

        cache.insert(h, asset.clone(), 100);

        assert!(cache.contains(h));
        assert_eq!(cache.get(h), Some(&asset));
        assert_eq!(cache.current_bytes(), 100);
    }

    #[test]
    fn test_lru_eviction_removes_oldest_unused() {
        // Cache with budget for exactly 2 entries of 100 bytes each
        let mut cache = AssetCache::<TestAsset>::new(200);

        let h1 = Handle::new();
        let h2 = Handle::new();
        let h3 = Handle::new();

        cache.insert(h1, TestAsset { name: "first".into() }, 100);
        cache.insert(h2, TestAsset { name: "second".into() }, 100);

        // Cache is full (200/200). Inserting h3 should evict h1 (LRU).
        cache.insert(h3, TestAsset { name: "third".into() }, 100);

        assert!(!cache.contains(h1), "h1 should have been evicted");
        assert!(cache.contains(h2));
        assert!(cache.contains(h3));
        assert_eq!(cache.current_bytes(), 200);
    }

    #[test]
    fn test_access_updates_lru_order() {
        let mut cache = AssetCache::<TestAsset>::new(200);

        let h1 = Handle::new();
        let h2 = Handle::new();
        let h3 = Handle::new();

        cache.insert(h1, TestAsset { name: "first".into() }, 100);
        cache.insert(h2, TestAsset { name: "second".into() }, 100);

        // Access h1, making h2 the LRU
        let _ = cache.get(h1);

        // Inserting h3 should now evict h2 (the real LRU)
        cache.insert(h3, TestAsset { name: "third".into() }, 100);

        assert!(cache.contains(h1), "h1 was accessed recently, should survive");
        assert!(!cache.contains(h2), "h2 should have been evicted");
        assert!(cache.contains(h3));
    }

    #[test]
    fn test_pinned_assets_not_evicted() {
        let mut cache = AssetCache::<TestAsset>::new(200);

        let h1 = Handle::new();
        let h2 = Handle::new();
        let h3 = Handle::new();

        cache.insert(h1, TestAsset { name: "pinned".into() }, 100);
        cache.pin(h1);
        cache.insert(h2, TestAsset { name: "second".into() }, 100);

        // Cache is full. h1 is LRU but pinned. Inserting h3 should evict h2.
        cache.insert(h3, TestAsset { name: "third".into() }, 100);

        assert!(cache.contains(h1), "Pinned h1 must not be evicted");
        assert!(!cache.contains(h2), "Unpinned h2 should be evicted");
        assert!(cache.contains(h3));
    }

    #[test]
    fn test_unpin_makes_asset_evictable() {
        let mut cache = AssetCache::<TestAsset>::new(200);

        let h1 = Handle::new();
        let h2 = Handle::new();
        let h3 = Handle::new();

        cache.insert(h1, TestAsset { name: "was_pinned".into() }, 100);
        cache.pin(h1);
        cache.insert(h2, TestAsset { name: "second".into() }, 100);

        // Unpin h1
        cache.unpin(h1);

        // Now h1 is the LRU and unpinned — should be evicted
        cache.insert(h3, TestAsset { name: "third".into() }, 100);

        assert!(!cache.contains(h1), "Unpinned h1 should now be evictable");
        assert!(cache.contains(h2));
        assert!(cache.contains(h3));
    }

    #[test]
    fn test_reload_after_eviction_works() {
        let mut cache = AssetCache::<TestAsset>::new(200);

        let h1 = Handle::new();
        let h2 = Handle::new();
        let h3 = Handle::new();

        cache.insert(h1, TestAsset { name: "evictable".into() }, 100);
        cache.insert(h2, TestAsset { name: "second".into() }, 100);

        // Evict h1 by inserting h3
        cache.insert(h3, TestAsset { name: "third".into() }, 100);
        assert!(!cache.contains(h1));

        // Simulate re-load: evict h2 by re-inserting h1
        cache.insert(h1, TestAsset { name: "reloaded".into() }, 100);

        assert!(cache.contains(h1));
        assert_eq!(
            cache.get(h1),
            Some(&TestAsset { name: "reloaded".into() })
        );
    }

    #[test]
    fn test_cache_size_limit_is_respected() {
        let mut cache = AssetCache::<TestAsset>::new(500);

        for i in 0..20 {
            let h = Handle::new();
            cache.insert(h, TestAsset { name: format!("asset_{i}") }, 100);
        }

        // With a 500 byte budget and 100 bytes per entry,
        // at most 5 entries can be cached.
        assert!(
            cache.current_bytes() <= 500,
            "Cache should not exceed budget: {} > 500",
            cache.current_bytes()
        );
        assert!(
            cache.len() <= 5,
            "Cache should have at most 5 entries, got {}",
            cache.len()
        );
    }

    #[test]
    fn test_empty_cache() {
        let cache = AssetCache::<TestAsset>::new(1024);
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.current_bytes(), 0);
    }

    #[test]
    fn test_asset_path_map_registration() {
        let mut paths = AssetPathMap::new();
        let h = Handle::<TestAsset>::new();
        paths.register(h, PathBuf::from("textures/stone.png"));

        assert_eq!(
            paths.get_path(h),
            Some(&PathBuf::from("textures/stone.png"))
        );
    }
}
```
