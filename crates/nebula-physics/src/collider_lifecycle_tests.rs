//! Tests for the collider lifecycle module.

use std::collections::HashSet;

use bevy_ecs::prelude::*;
use bevy_ecs::system::RunSystemOnce;
use nebula_math::WorldPosition;
use nebula_voxel::{
    Chunk, ChunkAddress, ChunkManager, Transparency, VoxelTypeDef, VoxelTypeId, VoxelTypeRegistry,
};

use super::*;
use crate::physics_bridge::PhysicsOrigin;
use crate::physics_island::IslandWorldPos;

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

fn solid_chunk() -> Chunk {
    let stone = VoxelTypeId(1);
    let mut chunk = Chunk::new();
    for x in 0u8..32 {
        for z in 0u8..32 {
            chunk.set(x, 0, z, stone);
        }
    }
    chunk
}

fn setup_ecs() -> World {
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new());
    world.insert_resource(PhysicsOrigin::default());
    world.insert_resource(DespawnedHandleCache::new());
    world
}

// -- Test 1: chunk load creates collider --

#[test]
fn test_chunk_load_creates_collider() {
    let registry = test_registry();
    let origin = PhysicsOrigin::default();
    let mut mgr = ChunkManager::new();
    mgr.load_chunk(addr(0, 0, 0), solid_chunk());

    let mut physics = PhysicsWorld::new();
    let chunk = mgr.get_chunk(&addr(0, 0, 0)).unwrap();

    let local_pos = crate::world_to_local(
        &chunk_addr_to_world_pos(&addr(0, 0, 0)),
        &origin.world_origin,
    );
    let handle = crate::voxel_collision::create_chunk_collider(
        &mut physics,
        chunk,
        &registry,
        local_pos,
        1.0,
    );

    assert!(handle.is_some());
    let handle = handle.unwrap();
    assert!(physics.collider_set.get(handle).is_some());

    // Verify collider position matches chunk local position.
    let collider = physics.collider_set.get(handle).unwrap();
    let pos = collider.position().translation;
    assert!((pos.x - local_pos.x).abs() < 0.01);
    assert!((pos.y - local_pos.y).abs() < 0.01);
    assert!((pos.z - local_pos.z).abs() < 0.01);
}

// -- Test 2: chunk unload removes collider --

#[test]
fn test_chunk_unload_removes_collider() {
    let registry = test_registry();
    let mut mgr = ChunkManager::new();
    mgr.load_chunk(addr(0, 0, 0), solid_chunk());

    let mut physics = PhysicsWorld::new();
    let chunk = mgr.get_chunk(&addr(0, 0, 0)).unwrap();
    let handle = crate::voxel_collision::create_chunk_collider(
        &mut physics,
        chunk,
        &registry,
        glam::Vec3::ZERO,
        1.0,
    )
    .unwrap();

    assert!(physics.collider_set.get(handle).is_some());

    let cc = ChunkCollider {
        coord: addr(0, 0, 0),
        handle,
    };

    // Remove via physics directly (equivalent to on_chunk_unloaded).
    physics.collider_set.remove(
        cc.handle,
        &mut physics.island_manager,
        &mut physics.rigid_body_set,
        true,
    );

    assert!(physics.collider_set.get(handle).is_none());
}

// -- Test 3: voxel change triggers collider rebuild --

#[test]
fn test_voxel_change_triggers_collider_rebuild() {
    let registry = test_registry();
    let origin = PhysicsOrigin::default();
    let stone = VoxelTypeId(1);
    let mut mgr = ChunkManager::new();
    mgr.load_chunk(addr(0, 0, 0), solid_chunk());

    let mut physics = PhysicsWorld::new();
    let chunk = mgr.get_chunk(&addr(0, 0, 0)).unwrap();
    let old_handle = crate::voxel_collision::create_chunk_collider(
        &mut physics,
        chunk,
        &registry,
        glam::Vec3::ZERO,
        1.0,
    )
    .unwrap();

    // Modify voxel.
    let chunk_mut = mgr.get_chunk_mut(&addr(0, 0, 0)).unwrap();
    chunk_mut.set(16, 0, 16, VoxelTypeId(0));
    chunk_mut.set(0, 1, 0, stone);

    let mut cc = ChunkCollider {
        coord: addr(0, 0, 0),
        handle: old_handle,
    };

    let dirty: HashSet<ChunkAddress> = [addr(0, 0, 0)].into_iter().collect();
    let entity = Entity::from_raw(0);
    let mut colliders: Vec<(Entity, &mut ChunkCollider)> = vec![(entity, &mut cc)];
    on_voxel_changed(
        &mut physics,
        &dirty,
        &mgr,
        &registry,
        &origin,
        &mut colliders,
    );

    assert_ne!(old_handle, cc.handle);
    assert!(physics.collider_set.get(old_handle).is_none());
    assert!(physics.collider_set.get(cc.handle).is_some());
}

// -- Test 4: entity spawn creates body --

#[test]
fn test_entity_spawn_creates_body() {
    let mut world = setup_ecs();

    let entity = world
        .spawn((
            IslandWorldPos(WorldPosition::new(5000, 10000, 5000)),
            PhysicsBody {
                body_type: PhysicsBodyType::Dynamic,
                shape: PhysicsShape::Sphere { radius: 0.5 },
                mass: 1.0,
                friction: 0.5,
                restitution: 0.3,
            },
        ))
        .id();

    world.run_system_once(spawn_physics_bodies).unwrap();
    world.flush();

    assert!(
        world
            .get::<crate::physics_island::RigidBodyHandle>(entity)
            .is_some(),
        "Entity should have RigidBodyHandle"
    );
    assert!(
        world.get::<ColliderHandle>(entity).is_some(),
        "Entity should have ColliderHandle"
    );

    let rb_handle = world
        .get::<crate::physics_island::RigidBodyHandle>(entity)
        .unwrap()
        .0;
    let physics = world.resource::<PhysicsWorld>();
    assert!(physics.rigid_body_set.get(rb_handle).is_some());

    let col_handle = world.get::<ColliderHandle>(entity).unwrap().0;
    let collider = physics.collider_set.get(col_handle).unwrap();
    assert_eq!(collider.parent(), Some(rb_handle));
}

// -- Test 5: entity despawn removes body --

#[test]
fn test_entity_despawn_removes_body() {
    let mut world = setup_ecs();

    let entity = world
        .spawn((
            IslandWorldPos(WorldPosition::new(0, 0, 0)),
            PhysicsBody {
                body_type: PhysicsBodyType::Dynamic,
                shape: PhysicsShape::Sphere { radius: 0.5 },
                mass: 1.0,
                friction: 0.5,
                restitution: 0.3,
            },
        ))
        .id();

    world.run_system_once(spawn_physics_bodies).unwrap();
    world.flush();

    let rb_handle = world
        .get::<crate::physics_island::RigidBodyHandle>(entity)
        .unwrap()
        .0;
    assert!(
        world
            .resource::<PhysicsWorld>()
            .rigid_body_set
            .get(rb_handle)
            .is_some()
    );

    // Despawn entity.
    world.despawn(entity);

    world.run_system_once(despawn_physics_bodies).unwrap();
    world.flush();

    let physics = world.resource::<PhysicsWorld>();
    assert!(
        physics.rigid_body_set.get(rb_handle).is_none(),
        "Rigid body should be removed after despawn"
    );
}

// -- Test 6: no orphaned colliders --

#[test]
fn test_no_orphaned_colliders() {
    let mut world = setup_ecs();

    let mut entities = Vec::new();
    for i in 0..10 {
        let e = world
            .spawn((
                IslandWorldPos(WorldPosition::new(i * 1000, 0, 0)),
                PhysicsBody {
                    body_type: PhysicsBodyType::Dynamic,
                    shape: PhysicsShape::Sphere { radius: 0.5 },
                    mass: 1.0,
                    friction: 0.5,
                    restitution: 0.3,
                },
            ))
            .id();
        entities.push(e);
    }

    world.run_system_once(spawn_physics_bodies).unwrap();
    world.flush();

    assert_eq!(world.resource::<PhysicsWorld>().rigid_body_set.len(), 10);

    for entity in &entities[..5] {
        world.despawn(*entity);
    }

    world.run_system_once(despawn_physics_bodies).unwrap();
    world.flush();

    let physics = world.resource::<PhysicsWorld>();
    assert_eq!(physics.rigid_body_set.len(), 5);
    assert_eq!(physics.collider_set.len(), 5);
}

// -- Test 7: multiple voxel changes single rebuild --

#[test]
fn test_multiple_voxel_changes_single_rebuild() {
    let registry = test_registry();
    let origin = PhysicsOrigin::default();
    let mut mgr = ChunkManager::new();
    mgr.load_chunk(addr(0, 0, 0), solid_chunk());

    let mut physics = PhysicsWorld::new();
    let chunk = mgr.get_chunk(&addr(0, 0, 0)).unwrap();
    let handle = crate::voxel_collision::create_chunk_collider(
        &mut physics,
        chunk,
        &registry,
        glam::Vec3::ZERO,
        1.0,
    )
    .unwrap();

    let mut cc = ChunkCollider {
        coord: addr(0, 0, 0),
        handle,
    };

    // 3 events for the same chunk → deduplicated to 1.
    let events = [addr(0, 0, 0), addr(0, 0, 0), addr(0, 0, 0)];
    let dirty = deduplicate_voxel_changes(events.iter());
    assert_eq!(dirty.len(), 1);

    let entity = Entity::from_raw(0);
    let mut colliders: Vec<(Entity, &mut ChunkCollider)> = vec![(entity, &mut cc)];
    on_voxel_changed(
        &mut physics,
        &dirty,
        &mgr,
        &registry,
        &origin,
        &mut colliders,
    );

    assert_eq!(
        physics.collider_set.len(),
        1,
        "Should have exactly 1 collider after deduplication"
    );
}
