//! Spatial hash map for efficient entity lookup by location.
//!
//! This module provides a spatial data structure that buckets entities by their sector coordinate,
//! allowing for O(1) sector-based queries and efficient radius queries.

use std::collections::HashMap;

use nebula_math::{WorldPosition, distance_squared};

use crate::{SectorCoord, SectorKey};

/// Entity ID type for spatial hash map entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EntityId(pub u64);

impl EntityId {
    /// Create a new entity ID from a u64.
    pub fn new(id: u64) -> Self {
        Self(id)
    }

    /// Get the inner u64 value.
    pub fn value(&self) -> u64 {
        self.0
    }
}

/// Trait that items in the spatial hash must implement.
pub trait SpatialEntity {
    /// Return the unique ID of this entity.
    fn entity_id(&self) -> EntityId;

    /// Return the entity's current world position.
    fn world_position(&self) -> &WorldPosition;
}

/// Trait for spatial entities that support position updates.
pub trait SpatialEntityMut: SpatialEntity {
    /// Update the entity's world position.
    fn set_world_position(&mut self, position: WorldPosition);
}

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

impl<T> Default for SpatialHashMap<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> SpatialHashMap<T> {
    /// Create a new empty spatial hash map.
    pub fn new() -> Self {
        Self {
            buckets: HashMap::new(),
            index: HashMap::new(),
            count: 0,
        }
    }

    /// Return the total number of entities in the spatial hash.
    pub fn count(&self) -> usize {
        self.count
    }

    /// Return true if the spatial hash contains no entities.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
}

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

    /// Return all entities in the given sector. Returns an empty slice if
    /// no entities are present.
    pub fn query_sector(&self, key: &SectorKey) -> &[T] {
        self.buckets.get(key).map(|v| v.as_slice()).unwrap_or(&[])
    }

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
                    let neighbor = SectorKey(crate::SectorIndex {
                        x: center_coord.sector.x + dx,
                        y: center_coord.sector.y + dy,
                        z: center_coord.sector.z + dz,
                    });

                    if let Some(bucket) = self.buckets.get(&neighbor) {
                        for entity in bucket {
                            let pos = entity.world_position();
                            let dist_sq = distance_squared(*center, *pos);
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

impl<T: SpatialEntityMut> SpatialHashMap<T> {
    /// Update an entity's position. If the entity has moved to a new sector,
    /// it is migrated to the correct bucket. If it remains in the same sector,
    /// only the entity's internal position is updated.
    pub fn update_position(&mut self, id: EntityId, new_pos: WorldPosition) {
        let new_key = SectorKey::from_world(&new_pos);

        if let Some(old_key) = self.index.get(&id).copied() {
            if old_key == new_key {
                // Same sector: update position in place.
                if let Some(bucket) = self.buckets.get_mut(&old_key)
                    && let Some(entity) = bucket.iter_mut().find(|e| e.entity_id() == id)
                {
                    entity.set_world_position(new_pos);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SectorIndex;

    // Test entity struct
    #[derive(Debug, Clone, PartialEq)]
    struct TestEntity {
        id: EntityId,
        position: WorldPosition,
    }

    impl TestEntity {
        fn new(id: u64, x: i128, y: i128, z: i128) -> Self {
            Self {
                id: EntityId::new(id),
                position: WorldPosition::new(x, y, z),
            }
        }
    }

    impl SpatialEntity for TestEntity {
        fn entity_id(&self) -> EntityId {
            self.id
        }

        fn world_position(&self) -> &WorldPosition {
            &self.position
        }
    }

    impl SpatialEntityMut for TestEntity {
        fn set_world_position(&mut self, position: WorldPosition) {
            self.position = position;
        }
    }

    #[test]
    fn test_insert_and_retrieve() {
        let mut spatial_hash = SpatialHashMap::new();
        let entity = TestEntity::new(1, 1000, 2000, 3000);
        let expected_sector = SectorKey::from_world(&entity.position);

        spatial_hash.insert(entity.clone());

        let results = spatial_hash.query_sector(&expected_sector);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entity_id(), EntityId::new(1));
        assert_eq!(
            results[0].world_position(),
            &WorldPosition::new(1000, 2000, 3000)
        );
    }

    #[test]
    fn test_query_empty_sector_returns_empty() {
        let spatial_hash: SpatialHashMap<TestEntity> = SpatialHashMap::new();
        let empty_sector = SectorKey(SectorIndex {
            x: 99,
            y: 99,
            z: 99,
        });

        let results = spatial_hash.query_sector(&empty_sector);
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_query_radius_finds_adjacent_sector_entities() {
        let mut spatial_hash = SpatialHashMap::new();

        // Entity A at the last position in sector (0,0,0)
        let entity_a = TestEntity::new(1, (1_i128 << 32) - 1, 0, 0);
        // Entity B at the first position in sector (1,0,0)
        let entity_b = TestEntity::new(2, 1_i128 << 32, 0, 0);

        spatial_hash.insert(entity_a.clone());
        spatial_hash.insert(entity_b.clone());

        // Query with center at entity A's position and radius = 100 mm
        let results = spatial_hash.query_radius(&entity_a.position, 100);
        assert_eq!(results.len(), 2);

        let mut found_ids: Vec<u64> = results.iter().map(|e| e.entity_id().value()).collect();
        found_ids.sort();
        assert_eq!(found_ids, vec![1, 2]);
    }

    #[test]
    fn test_query_radius_excludes_distant_entities() {
        let mut spatial_hash = SpatialHashMap::new();

        let entity_a = TestEntity::new(1, 0, 0, 0);
        let entity_b = TestEntity::new(2, 10_000_000_000, 0, 0); // 10 billion mm = 10,000 km away

        spatial_hash.insert(entity_a.clone());
        spatial_hash.insert(entity_b);

        // Query with center at A and radius = 1,000,000 mm (1 km)
        let results = spatial_hash.query_radius(&entity_a.position, 1_000_000);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entity_id(), EntityId::new(1));
    }

    #[test]
    fn test_update_position_same_sector() {
        let mut spatial_hash = SpatialHashMap::new();
        let entity = TestEntity::new(1, 100, 100, 100);
        let sector = SectorKey::from_world(&entity.position);

        spatial_hash.insert(entity);

        // Update position within same sector
        spatial_hash.update_position(EntityId::new(1), WorldPosition::new(200, 200, 200));

        let results = spatial_hash.query_sector(&sector);
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].world_position(),
            &WorldPosition::new(200, 200, 200)
        );
        assert_eq!(spatial_hash.count(), 1);
    }

    #[test]
    fn test_update_position_cross_sector() {
        let mut spatial_hash = SpatialHashMap::new();
        let entity = TestEntity::new(1, 100, 0, 0);
        let old_sector = SectorKey::from_world(&entity.position);

        spatial_hash.insert(entity);

        // Update position to different sector
        let new_pos = WorldPosition::new((1_i128 << 32) + 100, 0, 0);
        let new_sector = SectorKey::from_world(&new_pos);

        spatial_hash.update_position(EntityId::new(1), new_pos);

        // Old sector should be empty
        let old_results = spatial_hash.query_sector(&old_sector);
        assert_eq!(old_results.len(), 0);

        // New sector should contain the entity
        let new_results = spatial_hash.query_sector(&new_sector);
        assert_eq!(new_results.len(), 1);
        assert_eq!(new_results[0].world_position(), &new_pos);
    }

    #[test]
    fn test_remove_entity() {
        let mut spatial_hash = SpatialHashMap::new();
        let entity1 = TestEntity::new(1, 100, 100, 100);
        let entity2 = TestEntity::new(2, 100, 100, 100);
        let sector = SectorKey::from_world(&entity1.position);

        spatial_hash.insert(entity1);
        spatial_hash.insert(entity2);

        // Remove one entity
        let removed = spatial_hash.remove(EntityId::new(1));
        assert!(removed);

        // Check that only one entity remains
        let results = spatial_hash.query_sector(&sector);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entity_id(), EntityId::new(2));
        assert_eq!(spatial_hash.count(), 1);
    }

    #[test]
    fn test_remove_nonexistent_returns_false() {
        let mut spatial_hash: SpatialHashMap<TestEntity> = SpatialHashMap::new();
        let removed = spatial_hash.remove(EntityId::new(999));
        assert!(!removed);
    }

    #[test]
    fn test_remove_cleans_up_empty_bucket() {
        let mut spatial_hash = SpatialHashMap::new();
        let entity = TestEntity::new(1, 100, 100, 100);
        let sector = SectorKey::from_world(&entity.position);

        spatial_hash.insert(entity);
        spatial_hash.remove(EntityId::new(1));

        // Check that the sector is empty and internal state is cleaned up
        let results = spatial_hash.query_sector(&sector);
        assert_eq!(results.len(), 0);
        assert_eq!(spatial_hash.count(), 0);
        assert!(spatial_hash.is_empty());
    }

    #[test]
    fn test_insert_duplicate_replaces() {
        let mut spatial_hash = SpatialHashMap::new();
        let entity1 = TestEntity::new(42, 1000, 2000, 3000);
        let entity2 = TestEntity::new(42, 4000, 5000, 6000);

        spatial_hash.insert(entity1);
        spatial_hash.insert(entity2.clone());

        // Should have only one entity with ID 42
        assert_eq!(spatial_hash.count(), 1);

        // Should be at position B
        let sector_b = SectorKey::from_world(&entity2.position);
        let results = spatial_hash.query_sector(&sector_b);
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].world_position(),
            &WorldPosition::new(4000, 5000, 6000)
        );
    }

    #[test]
    fn test_entity_id_creation_and_access() {
        let id = EntityId::new(12345);
        assert_eq!(id.value(), 12345);

        let entity = TestEntity::new(999, 100, 200, 300);
        assert_eq!(entity.entity_id(), EntityId::new(999));
    }

    #[test]
    fn test_empty_spatial_hash() {
        let spatial_hash: SpatialHashMap<TestEntity> = SpatialHashMap::new();
        assert!(spatial_hash.is_empty());
        assert_eq!(spatial_hash.count(), 0);
    }

    #[test]
    fn test_spatial_hash_count_updates() {
        let mut spatial_hash = SpatialHashMap::new();
        assert_eq!(spatial_hash.count(), 0);

        let entity1 = TestEntity::new(1, 100, 200, 300);
        spatial_hash.insert(entity1);
        assert_eq!(spatial_hash.count(), 1);

        let entity2 = TestEntity::new(2, 400, 500, 600);
        spatial_hash.insert(entity2);
        assert_eq!(spatial_hash.count(), 2);

        spatial_hash.remove(EntityId::new(1));
        assert_eq!(spatial_hash.count(), 1);

        spatial_hash.remove(EntityId::new(2));
        assert_eq!(spatial_hash.count(), 0);
        assert!(spatial_hash.is_empty());
    }
}
