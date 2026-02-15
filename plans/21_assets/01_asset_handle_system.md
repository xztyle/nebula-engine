# Asset Handle System

## Problem

Every subsystem in the Nebula Engine needs to reference assets — textures, meshes, sounds, materials, shaders — but the assets themselves may not be loaded yet when the reference is created. A naive approach of passing around `Arc<Texture>` or raw pointers forces callers to block until the asset is ready, tightly couples systems to asset lifetimes, and makes it impossible to serialize references (you cannot write a pointer to disk). The engine needs a lightweight, type-safe, serializable token that represents "this asset, whether or not it has arrived yet." The token must distinguish between asset types at compile time (a `Handle<Texture>` must not be silently confused with a `Handle<Mesh>`), support cheap copying so systems can freely pass references around, and expose the current load state so rendering code can substitute a placeholder until the real asset arrives.

Additionally, there must be a central store that maps handles to their underlying data. Without this, every subsystem would need its own lookup table, leading to fragmented ownership and no single place to query "is this asset loaded?" or "how much memory are loaded assets consuming?"

## Solution

### Handle Type

A `Handle<T>` is a thin wrapper around a `u64` identifier plus a `PhantomData<T>` marker for type safety. The `u64` is assigned by a monotonically increasing atomic counter, guaranteeing uniqueness within a single engine session. Handles are `Copy`, `Clone`, `Eq`, `Hash`, and `Debug`, making them suitable as HashMap keys, ECS component fields, and log output.

```rust
use std::marker::PhantomData;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_HANDLE_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Eq, PartialEq, Hash)]
pub struct Handle<T> {
    id: u64,
    _marker: PhantomData<T>,
}

impl<T> Handle<T> {
    /// Create a new handle with a globally unique ID.
    pub fn new() -> Self {
        let id = NEXT_HANDLE_ID.fetch_add(1, Ordering::Relaxed);
        Self {
            id,
            _marker: PhantomData,
        }
    }

    /// Return the raw numeric identifier.
    pub fn id(&self) -> u64 {
        self.id
    }
}

impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            _marker: PhantomData,
        }
    }
}

impl<T> Copy for Handle<T> {}
```

Handles start at ID 1. ID 0 is reserved as a sentinel for "no handle" in contexts where `Option<Handle<T>>` would add unwanted overhead (e.g., tightly packed component arrays).

### Asset State

Every tracked asset has an associated state that progresses through a linear lifecycle:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetState {
    /// Handle has been created but no load request has been issued.
    Pending,
    /// A background task is actively loading the asset from disk or network.
    Loading,
    /// The asset data is available in memory and ready for use.
    Loaded,
    /// The load failed. The String contains a human-readable error message.
    Failed(String),
}
```

The state transitions are strictly `Pending -> Loading -> Loaded` or `Pending -> Loading -> Failed`. A `Loaded` asset can transition back to `Loading` during hot-reload (covered in a later story). A `Failed` asset can be retried by issuing a new load request, which resets it to `Loading`.

### AssetStore

The `AssetStore<T>` is a generic, type-erased container that maps `Handle<T>` to a combination of the asset data and its current state:

```rust
use std::collections::HashMap;

struct AssetEntry<T> {
    state: AssetState,
    data: Option<T>,
}

pub struct AssetStore<T> {
    entries: HashMap<u64, AssetEntry<T>>,
}

impl<T> AssetStore<T> {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Register a handle as pending. Called when a handle is created
    /// before any load request is submitted.
    pub fn insert_pending(&mut self, handle: Handle<T>) {
        self.entries.insert(
            handle.id(),
            AssetEntry {
                state: AssetState::Pending,
                data: None,
            },
        );
    }

    /// Transition a handle to the Loading state.
    pub fn set_loading(&mut self, handle: Handle<T>) {
        if let Some(entry) = self.entries.get_mut(&handle.id()) {
            entry.state = AssetState::Loading;
        }
    }

    /// Store the loaded asset data and transition to Loaded.
    pub fn set_loaded(&mut self, handle: Handle<T>, data: T) {
        if let Some(entry) = self.entries.get_mut(&handle.id()) {
            entry.state = AssetState::Loaded;
            entry.data = Some(data);
        }
    }

    /// Mark the asset as failed with an error message.
    pub fn set_failed(&mut self, handle: Handle<T>, error: String) {
        if let Some(entry) = self.entries.get_mut(&handle.id()) {
            entry.state = AssetState::Failed(error);
            entry.data = None;
        }
    }

    /// Query the current state of a handle.
    pub fn state(&self, handle: Handle<T>) -> Option<&AssetState> {
        self.entries.get(&handle.id()).map(|e| &e.state)
    }

    /// Get an immutable reference to the loaded asset data.
    /// Returns None if the asset is not in the Loaded state.
    pub fn get(&self, handle: Handle<T>) -> Option<&T> {
        self.entries
            .get(&handle.id())
            .and_then(|e| e.data.as_ref())
    }

    /// Get a mutable reference to the loaded asset data.
    pub fn get_mut(&mut self, handle: Handle<T>) -> Option<&mut T> {
        self.entries
            .get_mut(&handle.id())
            .and_then(|e| e.data.as_mut())
    }

    /// Remove a handle and its data from the store entirely.
    /// Returns the data if it was loaded.
    pub fn remove(&mut self, handle: Handle<T>) -> Option<T> {
        self.entries
            .remove(&handle.id())
            .and_then(|e| e.data)
    }

    /// Returns true if the handle exists in the store (any state).
    pub fn contains(&self, handle: Handle<T>) -> bool {
        self.entries.contains_key(&handle.id())
    }

    /// Number of tracked assets (all states).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate over all handles that are in the Loaded state.
    pub fn loaded_handles(&self) -> impl Iterator<Item = Handle<T>> + '_ {
        self.entries
            .iter()
            .filter(|(_, e)| e.state == AssetState::Loaded)
            .map(|(&id, _)| Handle {
                id,
                _marker: PhantomData,
            })
    }
}
```

### Type Safety Across Subsystems

Each subsystem maintains its own `AssetStore` for its asset type. The ECS world holds these stores as resources:

```rust
use bevy_ecs::prelude::*;

#[derive(Resource)]
pub struct TextureAssets(pub AssetStore<GpuTexture>);

#[derive(Resource)]
pub struct MeshAssets(pub AssetStore<GpuMesh>);

#[derive(Resource)]
pub struct SoundAssets(pub AssetStore<SoundBuffer>);
```

A component can store a `Handle<GpuTexture>` and the rendering system looks it up in `Res<TextureAssets>`. The compiler prevents accidentally looking up a `Handle<GpuTexture>` in the `MeshAssets` store because the generic parameter does not match.

### Handle Allocation Pattern

The recommended pattern for creating a handle and initiating a load:

```rust
/// Allocate a handle, register it as pending, then kick off a load.
pub fn request_load<T>(
    store: &mut AssetStore<T>,
    // ... loader details
) -> Handle<T> {
    let handle = Handle::<T>::new();
    store.insert_pending(handle);
    // Submit load task (covered in async loading story)
    handle
}
```

Callers receive the handle immediately and can embed it in components or data structures. They poll `store.state(handle)` or simply call `store.get(handle)` each frame — returning `None` until the data arrives.

## Outcome

A `Handle<T>` type and `AssetStore<T>` container that together form the foundation of the asset system. Every loaded asset in the engine is referenced exclusively through handles. The handle is 8 bytes, `Copy`, type-safe, and usable as a HashMap key or ECS component field. The `AssetStore<T>` tracks state transitions from `Pending` through `Loading` to `Loaded` or `Failed`, provides O(1) lookups by handle ID, and exposes iteration over loaded assets. Downstream stories (async loading, caching, hot-reload) build on this foundation.

## Demo Integration

**Demo crate:** `nebula-demo`

Assets are referenced by typed handles with automatic reference counting. Dropping all handles to an asset allows it to be unloaded.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.15` | `Resource` derive for inserting `AssetStore` wrappers into the ECS world |
| `serde` | `1.0` | Optional `Serialize`/`Deserialize` on `Handle<T>` for scene saving |
| `log` | `0.4` | Logging state transitions for debugging |

No additional dependencies. The handle counter uses `std::sync::atomic`. The store uses `std::collections::HashMap`. Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Dummy asset type for testing.
    #[derive(Debug, Clone, PartialEq)]
    struct TestAsset {
        name: String,
    }

    #[test]
    fn test_handle_creation_returns_unique_ids() {
        let h1 = Handle::<TestAsset>::new();
        let h2 = Handle::<TestAsset>::new();
        let h3 = Handle::<TestAsset>::new();
        assert_ne!(h1.id(), h2.id());
        assert_ne!(h2.id(), h3.id());
        assert_ne!(h1.id(), h3.id());
    }

    #[test]
    fn test_handle_id_is_nonzero() {
        let h = Handle::<TestAsset>::new();
        assert!(h.id() > 0, "Handle IDs should start at 1, got {}", h.id());
    }

    #[test]
    fn test_handle_is_copy_and_clone() {
        let h1 = Handle::<TestAsset>::new();
        let h2 = h1; // Copy
        let h3 = h1.clone(); // Clone
        assert_eq!(h1.id(), h2.id());
        assert_eq!(h1.id(), h3.id());
    }

    #[test]
    fn test_handle_equality_by_id() {
        let h1 = Handle::<TestAsset>::new();
        let h2 = h1;
        assert_eq!(h1, h2);

        let h3 = Handle::<TestAsset>::new();
        assert_ne!(h1, h3);
    }

    #[test]
    fn test_handle_usable_as_hash_key() {
        use std::collections::HashMap;
        let h = Handle::<TestAsset>::new();
        let mut map = HashMap::new();
        map.insert(h, "value");
        assert_eq!(map.get(&h), Some(&"value"));
    }

    #[test]
    fn test_pending_handle_transitions_to_loaded() {
        let mut store = AssetStore::<TestAsset>::new();
        let h = Handle::new();

        store.insert_pending(h);
        assert_eq!(store.state(h), Some(&AssetState::Pending));
        assert!(store.get(h).is_none());

        store.set_loading(h);
        assert_eq!(store.state(h), Some(&AssetState::Loading));
        assert!(store.get(h).is_none());

        let asset = TestAsset { name: "texture.png".into() };
        store.set_loaded(h, asset.clone());
        assert_eq!(store.state(h), Some(&AssetState::Loaded));
        assert_eq!(store.get(h), Some(&asset));
    }

    #[test]
    fn test_failed_handle_is_queryable() {
        let mut store = AssetStore::<TestAsset>::new();
        let h = Handle::new();

        store.insert_pending(h);
        store.set_loading(h);
        store.set_failed(h, "file not found".into());

        assert_eq!(
            store.state(h),
            Some(&AssetState::Failed("file not found".into()))
        );
        assert!(store.get(h).is_none(), "Failed asset should have no data");
    }

    #[test]
    fn test_store_remove_returns_data() {
        let mut store = AssetStore::<TestAsset>::new();
        let h = Handle::new();
        let asset = TestAsset { name: "mesh.glb".into() };

        store.insert_pending(h);
        store.set_loaded(h, asset.clone());

        let removed = store.remove(h);
        assert_eq!(removed, Some(asset));
        assert!(!store.contains(h));
    }

    #[test]
    fn test_store_len_and_empty() {
        let mut store = AssetStore::<TestAsset>::new();
        assert!(store.is_empty());
        assert_eq!(store.len(), 0);

        let h1 = Handle::new();
        let h2 = Handle::new();
        store.insert_pending(h1);
        store.insert_pending(h2);

        assert_eq!(store.len(), 2);
        assert!(!store.is_empty());
    }

    #[test]
    fn test_loaded_handles_iterator() {
        let mut store = AssetStore::<TestAsset>::new();
        let h1 = Handle::new();
        let h2 = Handle::new();
        let h3 = Handle::new();

        store.insert_pending(h1);
        store.insert_pending(h2);
        store.insert_pending(h3);

        store.set_loaded(h1, TestAsset { name: "a".into() });
        // h2 stays pending
        store.set_loaded(h3, TestAsset { name: "c".into() });

        let loaded: Vec<u64> = store.loaded_handles().map(|h| h.id()).collect();
        assert_eq!(loaded.len(), 2);
        assert!(loaded.contains(&h1.id()));
        assert!(loaded.contains(&h3.id()));
        assert!(!loaded.contains(&h2.id()));
    }

    #[test]
    fn test_get_nonexistent_handle_returns_none() {
        let store = AssetStore::<TestAsset>::new();
        let h = Handle::new(); // never inserted
        assert!(store.state(h).is_none());
        assert!(store.get(h).is_none());
    }

    #[test]
    fn test_get_mut_modifies_loaded_data() {
        let mut store = AssetStore::<TestAsset>::new();
        let h = Handle::new();
        store.insert_pending(h);
        store.set_loaded(h, TestAsset { name: "original".into() });

        if let Some(data) = store.get_mut(h) {
            data.name = "modified".into();
        }

        assert_eq!(store.get(h).unwrap().name, "modified");
    }
}
```
