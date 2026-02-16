//! Voxel collision shapes: converts chunk voxel data into Rapier sparse voxel
//! colliders, keeps them in sync on voxel edits, and cleans up on chunk unload.
//!
//! Uses Rapier 0.32's native `SharedShape::voxels()` API which represents occupied
//! voxels as a sparse data structure — far cheaper than compound shapes or trimeshes.

use rustc_hash::{FxHashMap, FxHashSet};

use rapier3d::math::IVector;
use rapier3d::prelude::*;

use nebula_voxel::{ChunkAddress, ChunkManager, VoxelEventBuffer, VoxelTypeRegistry};

use crate::PhysicsWorld;

/// Maps chunk addresses to their active Rapier collider handles.
///
/// Provides O(1) lookup for collider update and removal.
#[derive(Default)]
pub struct ChunkColliderMap {
    map: FxHashMap<ChunkAddress, ColliderHandle>,
}

impl ChunkColliderMap {
    /// Creates a new empty collider map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts a mapping from chunk address to collider handle.
    pub fn insert(&mut self, addr: ChunkAddress, handle: ColliderHandle) {
        self.map.insert(addr, handle);
    }

    /// Removes and returns the collider handle for the given chunk address.
    pub fn remove(&mut self, addr: &ChunkAddress) -> Option<ColliderHandle> {
        self.map.remove(addr)
    }

    /// Returns the collider handle for the given chunk address, if any.
    pub fn get(&self, addr: &ChunkAddress) -> Option<&ColliderHandle> {
        self.map.get(addr)
    }

    /// Returns the number of tracked chunk colliders.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Returns `true` if no chunk colliders are tracked.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Returns `true` if the given chunk address has a collider.
    pub fn contains(&self, addr: &ChunkAddress) -> bool {
        self.map.contains_key(addr)
    }
}

/// Converts chunk voxel data into a Rapier sparse voxel collider shape.
///
/// Returns `None` if the chunk contains no solid voxels.
/// Uses `VoxelTypeRegistry::is_solid()` to determine occupancy.
pub fn chunk_to_voxel_collider(
    chunk: &nebula_voxel::Chunk,
    registry: &VoxelTypeRegistry,
    voxel_size: f32,
) -> Option<SharedShape> {
    let mut occupied = Vec::new();

    for z in 0u8..32 {
        for y in 0u8..32 {
            for x in 0u8..32 {
                let voxel_id = chunk.get(x, y, z);
                if registry.is_solid(voxel_id) {
                    occupied.push(IVector::new(x as i32, y as i32, z as i32));
                }
            }
        }
    }

    if occupied.is_empty() {
        return None;
    }

    let size = Vector::new(voxel_size, voxel_size, voxel_size);
    Some(SharedShape::voxels(size, &occupied))
}

/// Creates a static collider from chunk voxel data and inserts it into the physics world.
///
/// The collider is positioned at `chunk_local_pos` (meters, relative to physics origin).
/// Returns `None` if the chunk has no solid voxels.
pub fn create_chunk_collider(
    physics: &mut PhysicsWorld,
    chunk: &nebula_voxel::Chunk,
    registry: &VoxelTypeRegistry,
    chunk_local_pos: glam::Vec3,
    voxel_size: f32,
) -> Option<ColliderHandle> {
    let shape = chunk_to_voxel_collider(chunk, registry, voxel_size)?;

    let collider = ColliderBuilder::new(shape)
        .translation(Vector::new(
            chunk_local_pos.x,
            chunk_local_pos.y,
            chunk_local_pos.z,
        ))
        .friction(0.7)
        .restitution(0.0)
        .build();

    Some(physics.collider_set.insert(collider))
}

/// Rebuilds colliders for chunks that received voxel modification events.
///
/// Multiple voxel changes in the same chunk within one tick are deduplicated
/// so only one collider rebuild occurs per dirty chunk.
pub fn update_chunk_colliders(
    physics: &mut PhysicsWorld,
    events: &VoxelEventBuffer,
    chunks: &ChunkManager,
    registry: &VoxelTypeRegistry,
    collider_map: &mut ChunkColliderMap,
    chunk_local_pos_fn: impl Fn(&ChunkAddress) -> glam::Vec3,
    voxel_size: f32,
) {
    let mut dirty_chunks: FxHashSet<ChunkAddress> = FxHashSet::default();
    for event in events.read() {
        dirty_chunks.insert(event.chunk);
    }

    for coord in dirty_chunks {
        // Remove old collider.
        if let Some(old_handle) = collider_map.remove(&coord) {
            physics.collider_set.remove(
                old_handle,
                &mut physics.island_manager,
                &mut physics.rigid_body_set,
                true,
            );
        }

        // Rebuild from current chunk data.
        if let Some(chunk) = chunks.get_chunk(&coord) {
            let local_pos = chunk_local_pos_fn(&coord);
            if let Some(handle) =
                create_chunk_collider(physics, chunk, registry, local_pos, voxel_size)
            {
                collider_map.insert(coord, handle);
            }
        }
    }
}

/// Removes colliders for chunks that have been unloaded.
pub fn remove_chunk_colliders(
    physics: &mut PhysicsWorld,
    unloaded: &[ChunkAddress],
    collider_map: &mut ChunkColliderMap,
) {
    for addr in unloaded {
        if let Some(handle) = collider_map.remove(addr) {
            physics.collider_set.remove(
                handle,
                &mut physics.island_manager,
                &mut physics.rigid_body_set,
                true,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_voxel::{
        Chunk, ChunkAddress, ChunkManager, Transparency, VoxelEventBuffer, VoxelModifiedEvent,
        VoxelTypeDef, VoxelTypeId, VoxelTypeRegistry,
    };

    /// Creates a registry with Air (0) and Stone (1).
    fn test_registry() -> VoxelTypeRegistry {
        let mut reg = VoxelTypeRegistry::new();
        reg.register(VoxelTypeDef {
            name: "stone".into(),
            solid: true,
            transparency: Transparency::Opaque,
            material_index: 1,
            light_emission: 0,
        })
        .unwrap();
        reg
    }

    fn addr(x: i64, y: i64, z: i64) -> ChunkAddress {
        ChunkAddress::new(x, y, z, 0)
    }

    #[test]
    fn test_empty_chunk_produces_no_collider() {
        let reg = test_registry();
        let chunk = Chunk::new(); // all air
        let result = chunk_to_voxel_collider(&chunk, &reg, 1.0);
        assert!(result.is_none(), "Empty chunk should produce no collider");
    }

    #[test]
    fn test_solid_voxel_blocks_movement() {
        let reg = test_registry();
        let stone = VoxelTypeId(1);

        // Create chunk with solid floor at y=0.
        let mut chunk = Chunk::new();
        for x in 0u8..32 {
            for z in 0u8..32 {
                chunk.set(x, 0, z, stone);
            }
        }

        let mut physics = PhysicsWorld::new();
        let handle = create_chunk_collider(&mut physics, &chunk, &reg, glam::Vec3::ZERO, 1.0)
            .expect("solid chunk should produce collider");

        assert!(physics.collider_set.get(handle).is_some());

        // Place a dynamic sphere above the floor.
        let body = RigidBodyBuilder::dynamic()
            .translation(Vector::new(16.0, 5.0, 16.0))
            .build();
        let body_handle = physics.rigid_body_set.insert(body);
        let ball = ColliderBuilder::ball(0.5).build();
        physics
            .collider_set
            .insert_with_parent(ball, body_handle, &mut physics.rigid_body_set);

        // Step physics 120 times.
        for _ in 0..120 {
            physics.step();
        }

        let pos = physics.rigid_body_set[body_handle].translation();
        // Sphere should rest on the floor (y ≈ 1.0 + 0.5 = 1.5 for voxel top + radius).
        assert!(
            pos.y < 4.0,
            "Sphere should have fallen from 5.0, got y={}",
            pos.y
        );
        assert!(
            pos.y > 0.0,
            "Sphere should not have fallen through floor, got y={}",
            pos.y
        );
    }

    #[test]
    fn test_air_voxel_allows_passage() {
        let reg = test_registry();
        let stone = VoxelTypeId(1);

        // Floor at y=0 with a hole at (16, 0, 16).
        let mut chunk = Chunk::new();
        for x in 0u8..32 {
            for z in 0u8..32 {
                if x == 16 && z == 16 {
                    continue; // hole
                }
                chunk.set(x, 0, z, stone);
            }
        }

        let mut physics = PhysicsWorld::new();
        create_chunk_collider(&mut physics, &chunk, &reg, glam::Vec3::ZERO, 1.0)
            .expect("should produce collider");

        // Small sphere directly above the hole.
        let body = RigidBodyBuilder::dynamic()
            .translation(Vector::new(16.5, 2.0, 16.5))
            .build();
        let body_handle = physics.rigid_body_set.insert(body);
        let ball = ColliderBuilder::ball(0.3).build();
        physics
            .collider_set
            .insert_with_parent(ball, body_handle, &mut physics.rigid_body_set);

        for _ in 0..120 {
            physics.step();
        }

        let pos = physics.rigid_body_set[body_handle].translation();
        assert!(
            pos.y < 0.0,
            "Sphere should fall through hole, got y={}",
            pos.y
        );
    }

    #[test]
    fn test_chunk_collider_updates_on_voxel_change() {
        let reg = test_registry();
        let stone = VoxelTypeId(1);

        let mut mgr = ChunkManager::new();
        let a = addr(0, 0, 0);
        let mut chunk = Chunk::new();
        for x in 0u8..32 {
            for z in 0u8..32 {
                chunk.set(x, 0, z, stone);
            }
        }
        mgr.load_chunk(a, chunk);

        let mut physics = PhysicsWorld::new();
        let mut collider_map = ChunkColliderMap::new();

        // Create initial collider.
        let chunk_ref = mgr.get_chunk(&a).unwrap();
        let handle =
            create_chunk_collider(&mut physics, chunk_ref, &reg, glam::Vec3::ZERO, 1.0).unwrap();
        collider_map.insert(a, handle);

        // Place sphere on floor.
        let body = RigidBodyBuilder::dynamic()
            .translation(Vector::new(16.5, 2.0, 16.5))
            .build();
        let body_handle = physics.rigid_body_set.insert(body);
        let ball = ColliderBuilder::ball(0.3).build();
        physics
            .collider_set
            .insert_with_parent(ball, body_handle, &mut physics.rigid_body_set);

        // Remove floor voxel at (16, 0, 16) and fire event.
        let chunk_mut = mgr.get_chunk_mut(&a).unwrap();
        chunk_mut.set(16, 0, 16, VoxelTypeId(0));

        let mut events = VoxelEventBuffer::new();
        events.send(VoxelModifiedEvent {
            chunk: a,
            local_pos: (16, 0, 16),
            old_type: stone,
            new_type: VoxelTypeId(0),
        });

        // Run update.
        update_chunk_colliders(
            &mut physics,
            &events,
            &mgr,
            &reg,
            &mut collider_map,
            |_| glam::Vec3::ZERO,
            1.0,
        );

        // Step physics — sphere should fall through.
        for _ in 0..120 {
            physics.step();
        }

        let pos = physics.rigid_body_set[body_handle].translation();
        assert!(
            pos.y < 0.0,
            "Sphere should fall through removed voxel, got y={}",
            pos.y
        );
    }

    #[test]
    fn test_collider_removed_on_chunk_unload() {
        let reg = test_registry();
        let stone = VoxelTypeId(1);

        let mut chunk = Chunk::new();
        for x in 0u8..32 {
            for z in 0u8..32 {
                chunk.set(x, 0, z, stone);
            }
        }

        let mut physics = PhysicsWorld::new();
        let mut collider_map = ChunkColliderMap::new();
        let a = addr(0, 0, 0);

        let handle =
            create_chunk_collider(&mut physics, &chunk, &reg, glam::Vec3::ZERO, 1.0).unwrap();
        collider_map.insert(a, handle);

        assert!(physics.collider_set.get(handle).is_some());
        assert!(collider_map.contains(&a));

        // Unload.
        remove_chunk_colliders(&mut physics, &[a], &mut collider_map);

        assert!(physics.collider_set.get(handle).is_none());
        assert!(!collider_map.contains(&a));
    }

    #[test]
    fn test_collider_shape_matches_chunk_geometry() {
        let reg = test_registry();
        let stone = VoxelTypeId(1);

        // 4x4x4 solid cube in the corner.
        let mut chunk = Chunk::new();
        for x in 0u8..4 {
            for y in 0u8..4 {
                for z in 0u8..4 {
                    chunk.set(x, y, z, stone);
                }
            }
        }

        let mut physics = PhysicsWorld::new();
        let handle =
            create_chunk_collider(&mut physics, &chunk, &reg, glam::Vec3::ZERO, 1.0).unwrap();

        // Step physics once so the broad phase is up to date.
        physics.step();

        let dispatcher = rapier3d::parry::query::DefaultQueryDispatcher;
        let qp = physics.broad_phase.as_query_pipeline(
            &dispatcher,
            &physics.rigid_body_set,
            &physics.collider_set,
            QueryFilter::default(),
        );

        // Ray hitting solid area (center of 4x4x4 cube, from above).
        let ray_hit = Ray::new(Vector::new(2.0, 10.0, 2.0), Vector::new(0.0, -1.0, 0.0));
        let hit = qp.cast_ray(&ray_hit, 100.0, true);
        assert!(hit.is_some(), "Ray should hit solid voxels");

        // Ray hitting empty area (center of chunk, y=16, well above cube).
        let ray_miss = Ray::new(Vector::new(16.0, 10.0, 16.0), Vector::new(0.0, -1.0, 0.0));
        let miss = qp.cast_ray(&ray_miss, 100.0, true);
        assert!(miss.is_none(), "Ray should miss empty area");

        let _ = handle;
    }
}
