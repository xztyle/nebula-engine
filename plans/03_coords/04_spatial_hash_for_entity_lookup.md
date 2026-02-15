# Spatial Hash for Entity Lookup

## Problem

The engine must efficiently answer the question "which entities are near this position?" for a variety of systems: networking (replicate only entities within the player's interest radius), physics (broad-phase collision candidates), gameplay (proximity triggers, aggro ranges), and rendering (hybrid culling). A naive O(n) scan of all entities is unacceptable when the universe contains millions of entities spread across billions of kilometers. The sector system from Story 02 already partitions space into ~4,295 km cubes, making it a natural granularity for spatial bucketing. What is needed is a hash map keyed by sector index that provides O(1) access to entities within a sector and efficient radius queries that check only adjacent sectors.

## Solution

### Data Structure: SpatialHashMap<T>

```rust
use std::collections::HashMap;

/// A spatial hash map that buckets items by their sector coordinate.
/// `T` must carry enough information to identify and locate itself.
pub struct SpatialHashMap<T> {
    /// The primary storage: sector key -> list of items in that sector.
    buckets: HashMap<SectorKey, Vec<T>>,

    /// Reverse index: item ID -> sector key, for O(1) removal and updates.
    index: HashMap<EntityId, SectorKey>,

    /// Total number of items across all buckets.
    count: usize,
}
```

The `EntityId` is a lightweight `Copy` identifier (e.g., a `u64` or ECS entity index) that each item `T` must provide via a trait:

```rust
/// Trait that items in the spatial hash must implement.
pub trait SpatialEntity {
    /// Return the unique ID of this entity.
    fn entity_id(&self) -> EntityId;

    /// Return the entity's current world position.
    fn world_position(&self) -> &WorldPosition;
}
```

### Methods

**Insert:**

```rust
impl<T: SpatialEntity> SpatialHashMap<T> {
    /// Insert an entity into the spatial hash.
    /// If the entity already exists (by ID), it is first removed from its old bucket.
    pub fn insert(&mut self, entity: T) {
        let key = SectorKey::from_world(entity.world_position());
        let id = entity.entity_id();

        // Remove from old bucket if present.
        if let Some(old_key) = self.index.remove(&id) {
            if let Some(bucket) = self.buckets.get_mut(&old_key) {
                bucket.retain(|e| e.entity_id() != id);
                if bucket.is_empty() {
                    self.buckets.remove(&old_key);
                }
            }
        } else {
            self.count += 1;
        }

        self.buckets.entry(key).or_default().push(entity);
        self.index.insert(id, key);
    }
}
```

**Query by Sector:**

```rust
impl<T: SpatialEntity> SpatialHashMap<T> {
    /// Return all entities in the given sector. Returns an empty slice if
    /// no entities are present.
    pub fn query_sector(&self, key: &SectorKey) -> &[T] {
        self.buckets.get(key).map(|v| v.as_slice()).unwrap_or(&[])
    }
}
```

**Query by Radius:**

```rust
impl<T: SpatialEntity> SpatialHashMap<T> {
    /// Return all entities within `radius` millimeters of `center`.
    /// This checks the center's sector plus all neighboring sectors that
    /// the radius could overlap.
    pub fn query_radius(&self, center: &WorldPosition, radius: i128) -> Vec<&T> {
        let sector_size: i128 = 1_i128 << 32;
        let center_coord = SectorCoord::from_world(center);

        // How many sectors the radius spans in each direction.
        let sector_reach = (radius / sector_size) + 1;

        let mut results = Vec::new();

        for dx in -sector_reach..=sector_reach {
            for dy in -sector_reach..=sector_reach {
                for dz in -sector_reach..=sector_reach {
                    let neighbor = SectorKey(SectorIndex {
                        x: center_coord.sector.x + dx,
                        y: center_coord.sector.y + dy,
                        z: center_coord.sector.z + dz,
                    });

                    if let Some(bucket) = self.buckets.get(&neighbor) {
                        for entity in bucket {
                            let pos = entity.world_position();
                            let dist_sq = distance_squared_128(center, pos);
                            if dist_sq <= radius.checked_mul(radius).unwrap_or(i128::MAX) {
                                results.push(entity);
                            }
                        }
                    }
                }
            }
        }

        results
    }
}
```

The `distance_squared_128` function computes `(ax-bx)^2 + (ay-by)^2 + (az-bz)^2` using `i128` arithmetic, being careful to avoid overflow by checking intermediate products or using widening multiplication if needed.

**Remove:**

```rust
impl<T: SpatialEntity> SpatialHashMap<T> {
    /// Remove an entity by its ID. Returns `true` if the entity was found and removed.
    pub fn remove(&mut self, id: EntityId) -> bool {
        if let Some(key) = self.index.remove(&id) {
            if let Some(bucket) = self.buckets.get_mut(&key) {
                bucket.retain(|e| e.entity_id() != id);
                if bucket.is_empty() {
                    self.buckets.remove(&key);
                }
            }
            self.count -= 1;
            true
        } else {
            false
        }
    }
}
```

**Update Position:**

```rust
impl<T: SpatialEntity> SpatialHashMap<T> {
    /// Update an entity's position. If the entity has moved to a new sector,
    /// it is migrated to the correct bucket. If it remains in the same sector,
    /// only the entity's internal position is updated.
    pub fn update_position(&mut self, id: EntityId, new_pos: WorldPosition)
    where
        T: SpatialEntityMut,
    {
        let new_key = SectorKey::from_world(&new_pos);

        if let Some(old_key) = self.index.get(&id).copied() {
            if old_key == new_key {
                // Same sector: update position in place.
                if let Some(bucket) = self.buckets.get_mut(&old_key) {
                    if let Some(entity) = bucket.iter_mut().find(|e| e.entity_id() == id) {
                        entity.set_world_position(new_pos);
                    }
                }
            } else {
                // Different sector: remove from old, insert into new.
                let mut entity = None;
                if let Some(bucket) = self.buckets.get_mut(&old_key) {
                    if let Some(idx) = bucket.iter().position(|e| e.entity_id() == id) {
                        let mut e = bucket.swap_remove(idx);
                        e.set_world_position(new_pos);
                        entity = Some(e);
                    }
                    if bucket.is_empty() {
                        self.buckets.remove(&old_key);
                    }
                }
                if let Some(e) = entity {
                    self.buckets.entry(new_key).or_default().push(e);
                    self.index.insert(id, new_key);
                }
            }
        }
    }
}
```

### Performance Characteristics

| Operation | Time Complexity | Notes |
|---|---|---|
| `insert` | O(1) amortized | HashMap insert + Vec push |
| `query_sector` | O(1) | Single HashMap lookup |
| `query_radius` | O(k * m) | k = sectors checked, m = avg entities per sector |
| `remove` | O(m) | Linear scan within the bucket (swap_remove) |
| `update_position` (same sector) | O(m) | Find + in-place update |
| `update_position` (cross-sector) | O(m) | Remove from old + insert into new |

For typical entity densities (hundreds per sector, not millions), the linear scans within buckets are fast. If profiling reveals hot spots, the per-bucket `Vec<T>` can be replaced with a `SlotMap` for O(1) removal by index.

### Thread Safety

The `SpatialHashMap` is not `Sync` by default. For parallel ECS systems, it should be wrapped in a `RwLock` or partitioned into per-thread shards that are merged at synchronization points. The ECS schedule should ensure that mutation (insert/remove/update) runs in exclusive systems while queries run in parallel read-only systems.

## Outcome

The `nebula-coords` crate exports `SpatialHashMap<T>`, the `SpatialEntity` trait, and the `SpatialEntityMut` trait. The data structure supports insert, query-by-sector, query-by-radius, remove, and update-position. All operations are O(1) in the common case (sector lookup) and O(k*m) for radius queries where k is the number of neighboring sectors checked. The implementation compiles and passes all tests. A demo can insert 10,000 entities at random positions, query a radius around an arbitrary point, and verify that all returned entities are within the radius.

## Demo Integration

**Demo crate:** `nebula-demo`

The demo inserts 1000 simulated entity positions into a spatial hash and queries "how many are within 100m?" The count is shown in the title: `Nearby: 42 entities`.

## Crates & Dependencies

- **`nebula-math`** (workspace) — `IVec3_128`, `WorldPosition`, distance functions
- **`nebula-coords`** (internal, same crate) — `SectorCoord`, `SectorKey`, `SectorIndex`
- **`hashbrown`** 0.15 — Optional: drop-in replacement for `std::collections::HashMap` with better performance; can be enabled via feature flag
- No other external dependencies

## Unit Tests

- **`test_insert_and_retrieve`** — Insert an entity at position `(1000, 2000, 3000)`. Query the sector containing that position. Assert the returned slice contains exactly one entity with the correct ID and position.

- **`test_query_empty_sector_returns_empty`** — Create a new `SpatialHashMap`. Query a sector that has never had anything inserted. Assert the returned slice is empty (length 0).

- **`test_query_radius_finds_adjacent_sector_entities`** — Insert entity A at the last position in sector (0,0,0) (offset `x = 2^32 - 1`) and entity B at the first position in sector (1,0,0) (offset `x = 0`). These entities are 1 mm apart but in different sectors. Query with center at entity A's position and radius = 100 mm. Assert both entities are returned.

- **`test_query_radius_excludes_distant_entities`** — Insert entity A at position `(0, 0, 0)` and entity B at position `(10_000_000_000, 0, 0)` (10 billion mm = 10,000 km away). Query with center at A and radius = 1,000,000 (1 km). Assert only entity A is returned.

- **`test_update_position_same_sector`** — Insert an entity at position `(100, 100, 100)`. Update its position to `(200, 200, 200)` (same sector). Query the sector and assert the entity's position has been updated. Assert the total count is still 1.

- **`test_update_position_cross_sector`** — Insert an entity at position `(100, 0, 0)` in sector (0,0,0). Update its position to `((1_i128 << 32) + 100, 0, 0)` in sector (1,0,0). Query sector (0,0,0) and assert it is empty. Query sector (1,0,0) and assert the entity is present with the new position.

- **`test_remove_entity`** — Insert two entities in the same sector. Remove one by ID. Query the sector and assert only the other entity remains. Assert the total count is 1.

- **`test_remove_nonexistent_returns_false`** — Create a new `SpatialHashMap`. Call `remove` with an arbitrary `EntityId`. Assert the return value is `false`.

- **`test_remove_cleans_up_empty_bucket`** — Insert one entity into a sector. Remove it. Assert that `query_sector` returns an empty slice and that the internal bucket map does not contain a key for that sector (verified by asserting `count == 0` and `buckets.len() == 0` via a test-only accessor or by querying).

- **`test_insert_duplicate_replaces`** — Insert an entity with ID 42 at position A. Insert another entity with the same ID 42 at position B. Assert the total count is 1 and querying by sector B's key returns the entity at position B.
