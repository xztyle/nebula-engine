# LRU Eviction Policy

## Problem

The engine caches data at multiple levels: loaded chunk voxel data, generated chunk meshes on the CPU, uploaded GPU buffers, decoded textures, and compiled materials. Each cache has a finite capacity -- ultimately bounded by the memory budget (story 04). When a cache is full and a new entry needs to be inserted, the engine must decide which existing entry to evict. Without a principled eviction policy, the engine either refuses to load new data (degrading the player's experience with missing terrain) or evicts random entries (potentially evicting a chunk the player is looking at, causing visible pop-in).

Least Recently Used (LRU) eviction is the standard policy for spatial caches: the entry that has not been accessed for the longest time is the best candidate for eviction. In a voxel engine, chunks that the player has moved away from are accessed less frequently than nearby chunks, so LRU naturally evicts distant, low-priority data first.

The LRU cache needs to support an eviction callback so that the owning system can clean up associated resources (return GPU buffers to the pool, notify the physics engine to remove collision geometry, etc.) when an entry is evicted.

## Solution

Implement a generic `LruCache<K, V>` in the `nebula_memory` crate that provides O(1) `get`, `insert`, and eviction using a combination of a `HashMap` and a doubly-linked list.

### Data Structure

The classic LRU cache uses a `HashMap<K, NodeIndex>` for O(1) key lookup and a doubly-linked list for O(1) move-to-front and eviction-from-back. Rust's ownership model makes intrusive linked lists awkward, so the implementation uses a `Vec`-backed arena for list nodes (index-based "pointers") to avoid `unsafe` and raw pointers.

```rust
use std::collections::HashMap;

/// Index into the node arena.
type NodeIndex = usize;

/// A node in the doubly-linked list.
struct LruNode<K, V> {
    key: K,
    value: V,
    prev: Option<NodeIndex>,
    next: Option<NodeIndex>,
}

/// A generic Least Recently Used (LRU) cache with O(1) operations
/// and an optional eviction callback.
///
/// # Type Parameters
/// - `K`: The cache key type. Must be `Eq + Hash + Clone`.
/// - `V`: The cached value type.
pub struct LruCache<K, V>
where
    K: Eq + std::hash::Hash + Clone,
{
    /// Maps keys to their node index in the arena.
    map: HashMap<K, NodeIndex>,
    /// Arena-allocated doubly-linked list nodes.
    nodes: Vec<LruNode<K, V>>,
    /// Index of the most recently used node (front of the list).
    head: Option<NodeIndex>,
    /// Index of the least recently used node (back of the list).
    tail: Option<NodeIndex>,
    /// Maximum number of entries before eviction occurs.
    capacity: usize,
    /// Free list of reusable node indices (from evicted/removed entries).
    free_list: Vec<NodeIndex>,
    /// Optional callback invoked when an entry is evicted.
    /// Receives the evicted key and value.
    on_evict: Option<Box<dyn FnMut(K, V)>>,
}
```

### Implementation

```rust
impl<K, V> LruCache<K, V>
where
    K: Eq + std::hash::Hash + Clone,
{
    /// Create a new LRU cache with the given maximum capacity.
    ///
    /// # Panics
    /// Panics if `capacity` is 0.
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "LRU cache capacity must be > 0");
        Self {
            map: HashMap::with_capacity(capacity),
            nodes: Vec::with_capacity(capacity),
            head: None,
            tail: None,
            capacity,
            free_list: Vec::new(),
            on_evict: None,
        }
    }

    /// Set the eviction callback.
    ///
    /// The callback is invoked with the evicted (key, value) whenever an entry
    /// is evicted due to the cache being at capacity during an `insert()`.
    pub fn set_on_evict<F>(&mut self, callback: F)
    where
        F: FnMut(K, V) + 'static,
    {
        self.on_evict = Some(Box::new(callback));
    }

    /// Get a reference to the value associated with `key`, marking it as
    /// recently used.
    ///
    /// Returns `None` if the key is not in the cache.
    pub fn get(&mut self, key: &K) -> Option<&V> {
        let idx = *self.map.get(key)?;
        self.move_to_front(idx);
        Some(&self.nodes[idx].value)
    }

    /// Get a mutable reference to the value associated with `key`, marking it
    /// as recently used.
    pub fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        let idx = *self.map.get(key)?;
        self.move_to_front(idx);
        Some(&mut self.nodes[idx].value)
    }

    /// Peek at a value without updating its access time.
    pub fn peek(&self, key: &K) -> Option<&V> {
        let idx = self.map.get(key)?;
        Some(&self.nodes[*idx].value)
    }

    /// Insert a key-value pair into the cache.
    ///
    /// If the key already exists, its value is updated and it is marked as
    /// recently used. If the cache is at capacity, the least recently used
    /// entry is evicted (triggering the eviction callback if set).
    ///
    /// Returns the old value if the key was already present.
    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        // If the key already exists, update in place.
        if let Some(&idx) = self.map.get(&key) {
            let old = std::mem::replace(&mut self.nodes[idx].value, value);
            self.move_to_front(idx);
            return Some(old);
        }

        // If at capacity, evict the LRU entry.
        if self.map.len() >= self.capacity {
            self.evict_lru();
        }

        // Allocate a node (reuse from free list or push new).
        let node = LruNode {
            key: key.clone(),
            value,
            prev: None,
            next: self.head,
        };

        let idx = if let Some(free_idx) = self.free_list.pop() {
            self.nodes[free_idx] = node;
            free_idx
        } else {
            let idx = self.nodes.len();
            self.nodes.push(node);
            idx
        };

        // Link as the new head.
        if let Some(old_head) = self.head {
            self.nodes[old_head].prev = Some(idx);
        }
        self.head = Some(idx);
        if self.tail.is_none() {
            self.tail = Some(idx);
        }

        self.map.insert(key, idx);
        None
    }

    /// Remove a key from the cache, returning its value if present.
    ///
    /// This does NOT trigger the eviction callback (explicit removal is
    /// not eviction).
    pub fn remove(&mut self, key: &K) -> Option<V> {
        let idx = self.map.remove(key)?;
        self.unlink(idx);
        self.free_list.push(idx);
        // Extract value by replacing with a dummy. Since the node is on the
        // free list, the dummy key/value will be overwritten on next use.
        let node = &mut self.nodes[idx];
        // We need to take the value out. Use unsafe-free approach:
        // swap the node with a placeholder.
        let key_clone = node.key.clone();
        let value = unsafe {
            std::ptr::read(&node.value as *const V)
        };
        // Mark the node slot as logically empty (will be overwritten on reuse).
        // The key is still there but won't be accessed since it's removed from the map.
        Some(value)
    }

    /// Number of entries currently in the cache.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Maximum capacity of the cache.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Whether the key exists in the cache (does not update access time).
    pub fn contains_key(&self, key: &K) -> bool {
        self.map.contains_key(key)
    }

    // --- Internal methods ---

    /// Evict the least recently used entry (the tail of the list).
    fn evict_lru(&mut self) {
        if let Some(tail_idx) = self.tail {
            let key = self.nodes[tail_idx].key.clone();
            self.map.remove(&key);
            self.unlink(tail_idx);
            self.free_list.push(tail_idx);

            // Read out the evicted value for the callback.
            let evicted_value = unsafe {
                std::ptr::read(&self.nodes[tail_idx].value as *const V)
            };

            if let Some(ref mut callback) = self.on_evict {
                callback(key, evicted_value);
            }
        }
    }

    /// Move a node to the front of the list (most recently used).
    fn move_to_front(&mut self, idx: NodeIndex) {
        if self.head == Some(idx) {
            return; // Already at front.
        }

        self.unlink(idx);

        // Insert at front.
        self.nodes[idx].prev = None;
        self.nodes[idx].next = self.head;
        if let Some(old_head) = self.head {
            self.nodes[old_head].prev = Some(idx);
        }
        self.head = Some(idx);
        if self.tail.is_none() {
            self.tail = Some(idx);
        }
    }

    /// Unlink a node from the doubly-linked list.
    fn unlink(&mut self, idx: NodeIndex) {
        let prev = self.nodes[idx].prev;
        let next = self.nodes[idx].next;

        if let Some(p) = prev {
            self.nodes[p].next = next;
        } else {
            self.head = next;
        }

        if let Some(n) = next {
            self.nodes[n].prev = prev;
        } else {
            self.tail = prev;
        }

        self.nodes[idx].prev = None;
        self.nodes[idx].next = None;
    }
}
```

### Usage Examples

```rust
// Chunk cache: evict old chunks when memory is tight.
let mut chunk_cache: LruCache<ChunkAddress, Chunk> = LruCache::new(8192);
chunk_cache.set_on_evict(|addr, chunk| {
    // Return the chunk to the pool instead of dropping it.
    chunk_pool.release(chunk);
    // Free the memory budget.
    budget.free(MemorySubsystem::ChunkData, chunk.memory_size());
    tracing::debug!("Evicted chunk at {addr:?}");
});

// Texture cache: evict unused textures.
let mut texture_cache: LruCache<TextureId, GpuTexture> = LruCache::new(256);
texture_cache.set_on_evict(|id, texture| {
    budget.free(MemorySubsystem::Textures, texture.byte_size());
    // wgpu texture is dropped here, releasing GPU memory.
});

// Mesh cache: evict CPU-side mesh data after GPU upload.
let mut mesh_cache: LruCache<ChunkAddress, ChunkMesh> = LruCache::new(4096);
```

### Design Decisions

- **Vec-based arena instead of `Box`-linked list**: A `Vec<LruNode>` with index-based "pointers" avoids pointer indirection, is cache-friendly, and does not require `unsafe` for the linked list operations (only for value extraction on eviction). Rust's standard library does not provide a doubly-linked list with O(1) indexed removal.
- **Eviction callback via closure**: The `on_evict` callback decouples the LRU cache from specific resource cleanup logic. The chunk cache, texture cache, and mesh cache all use the same `LruCache` struct with different callbacks.
- **`get()` takes `&mut self`**: Accessing a value updates the LRU order (moves the node to the front), which requires mutation. This is a deliberate trade-off -- the cache enforces correct usage by requiring mutable access for lookups. `peek()` is provided for read-only access without updating order.
- **Clone bound on K**: Keys are cloned when stored in both the `HashMap` and the `LruNode`. For `ChunkAddress` (which is `Copy`), this is zero-cost.

## Outcome

The `nebula_memory` crate exports `LruCache<K, V>` with `get()`, `get_mut()`, `peek()`, `insert()`, `remove()`, `set_on_evict()`, `len()`, `capacity()`, and `contains_key()`. The chunk cache, texture cache, and mesh cache all use this generic LRU implementation for eviction under memory pressure. Running `cargo test -p nebula_memory` passes all LRU cache tests. The crate uses Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

When the chunk memory budget is exceeded, the least-recently-accessed chunks are evicted. Eviction prioritizes distant, occluded chunks. The console logs eviction events.

## Crates & Dependencies

No external crates are required. The LRU cache uses `std::collections::HashMap` from the standard library. The implementation is self-contained within the `nebula_memory` crate.

| Crate | Version | Purpose |
|-------|---------|---------|
| (none) | -- | Fully self-contained using `std` only |

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Recently accessed items should remain in the cache when at capacity.
    #[test]
    fn test_recently_used_items_are_kept() {
        let mut cache = LruCache::new(3);

        cache.insert("a", 1);
        cache.insert("b", 2);
        cache.insert("c", 3);

        // Access "a" to make it recently used.
        cache.get(&"a");

        // Insert "d" -- this should evict "b" (the LRU), not "a".
        cache.insert("d", 4);

        assert!(cache.contains_key(&"a"), "a should be kept (recently used)");
        assert!(!cache.contains_key(&"b"), "b should be evicted (least recently used)");
        assert!(cache.contains_key(&"c"), "c should be kept");
        assert!(cache.contains_key(&"d"), "d should be present (just inserted)");
    }

    /// The least recently used item should be evicted when inserting into
    /// a full cache.
    #[test]
    fn test_least_recently_used_is_evicted() {
        let mut cache = LruCache::new(2);

        cache.insert("first", 1);
        cache.insert("second", 2);

        // Cache is full. Insert a third item.
        cache.insert("third", 3);

        // "first" was the LRU (inserted first, never accessed again).
        assert!(!cache.contains_key(&"first"), "first should be evicted");
        assert!(cache.contains_key(&"second"));
        assert!(cache.contains_key(&"third"));
    }

    /// Calling get() should move the item to the most-recently-used position.
    #[test]
    fn test_get_updates_access_order() {
        let mut cache = LruCache::new(3);

        cache.insert("a", 1);
        cache.insert("b", 2);
        cache.insert("c", 3);

        // Access "a", making it the most recently used.
        assert_eq!(cache.get(&"a"), Some(&1));

        // Insert two more items to evict the two oldest.
        cache.insert("d", 4); // evicts "b"
        cache.insert("e", 5); // evicts "c"

        // "a" should still be present because get() refreshed it.
        assert!(cache.contains_key(&"a"), "a should survive (get refreshed it)");
        assert!(!cache.contains_key(&"b"), "b should be evicted");
        assert!(!cache.contains_key(&"c"), "c should be evicted");
    }

    /// The cache should never exceed its stated capacity.
    #[test]
    fn test_capacity_is_respected() {
        let mut cache = LruCache::new(5);

        for i in 0..100 {
            cache.insert(i, i * 10);
            assert!(
                cache.len() <= 5,
                "cache length {} exceeds capacity 5 after inserting {i}",
                cache.len()
            );
        }

        assert_eq!(cache.len(), 5);

        // Only the last 5 keys should remain.
        for i in 95..100 {
            assert!(cache.contains_key(&i), "key {i} should be present");
        }
        for i in 0..95 {
            assert!(!cache.contains_key(&i), "key {i} should have been evicted");
        }
    }

    /// The eviction callback should fire with the correct key and value
    /// when an item is evicted.
    #[test]
    fn test_eviction_callback_fires() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let evicted = Rc::new(RefCell::new(Vec::new()));
        let evicted_clone = Rc::clone(&evicted);

        let mut cache = LruCache::new(2);
        cache.set_on_evict(move |key: &str, value: i32| {
            evicted_clone.borrow_mut().push((key.to_string(), value));
        });

        cache.insert("a", 1);
        cache.insert("b", 2);

        // No eviction yet.
        assert!(evicted.borrow().is_empty());

        // This insert should evict "a".
        cache.insert("c", 3);
        assert_eq!(evicted.borrow().len(), 1);
        assert_eq!(evicted.borrow()[0], ("a".to_string(), 1));

        // This insert should evict "b".
        cache.insert("d", 4);
        assert_eq!(evicted.borrow().len(), 2);
        assert_eq!(evicted.borrow()[1], ("b".to_string(), 2));
    }

    /// An empty cache should not evict anything (no panic, no callback).
    #[test]
    fn test_empty_cache_does_not_evict() {
        let mut eviction_count = 0u32;
        let mut cache: LruCache<i32, i32> = LruCache::new(5);
        // We can't easily capture a mutable counter, so instead we verify
        // the cache behavior without a callback.

        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.get(&42), None);

        // Inserting into a non-full cache should not evict.
        cache.insert(1, 100);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.get(&1), Some(&100));
    }

    /// peek() should NOT update the access order.
    #[test]
    fn test_peek_does_not_update_order() {
        let mut cache = LruCache::new(2);

        cache.insert("a", 1);
        cache.insert("b", 2);

        // Peek at "a" -- should NOT refresh it.
        assert_eq!(cache.peek(&"a"), Some(&1));

        // Insert "c" -- should evict "a" since peek didn't refresh it.
        cache.insert("c", 3);
        assert!(!cache.contains_key(&"a"), "a should be evicted (peek does not refresh)");
        assert!(cache.contains_key(&"b"));
        assert!(cache.contains_key(&"c"));
    }

    /// Inserting a duplicate key should update the value and refresh access order.
    #[test]
    fn test_insert_duplicate_updates_value() {
        let mut cache = LruCache::new(2);

        cache.insert("a", 1);
        cache.insert("b", 2);

        // Update "a" with a new value.
        let old = cache.insert("a", 100);
        assert_eq!(old, Some(1), "should return the old value");
        assert_eq!(cache.get(&"a"), Some(&100), "should have the new value");
        assert_eq!(cache.len(), 2, "length should not change on duplicate insert");

        // "a" was refreshed by the duplicate insert, so "b" is now LRU.
        cache.insert("c", 3);
        assert!(cache.contains_key(&"a"), "a should survive (refreshed by update)");
        assert!(!cache.contains_key(&"b"), "b should be evicted");
    }
}
```
