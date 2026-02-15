# Collider Lifecycle

## Problem

The Rapier physics world and the Bevy ECS world are two separate state stores that must remain perfectly synchronized. When a voxel chunk loads, a corresponding Rapier collider must be created. When a chunk unloads, that collider must be removed. When voxels within a chunk change (mining, building), the collider must be rebuilt. When an entity with physics spawns (an NPC, a dropped item, a projectile), a rigid body and collider must be created in Rapier. When that entity despawns, the body and collider must be removed. Any desynchronization leads to ghost collisions (colliders without entities), missing collisions (entities without colliders), or memory leaks (orphaned Rapier handles). The engine needs a centralized, event-driven lifecycle manager that ties Rapier state to ECS state using change detection and events, ensuring that no Rapier handle exists without a corresponding entity and no physics-eligible entity exists without its Rapier handles.

## Solution

### Handle Components

Rapier handles are stored as ECS components on their owning entities:

```rust
#[derive(Component)]
pub struct RigidBodyHandle(pub rapier3d::prelude::RigidBodyHandle);

#[derive(Component)]
pub struct ColliderHandle(pub rapier3d::prelude::ColliderHandle);

/// For chunks: the chunk coordinate this collider represents.
#[derive(Component)]
pub struct ChunkCollider {
    pub coord: ChunkCoord,
    pub handle: rapier3d::prelude::ColliderHandle,
}
```

By attaching handles as components, the ECS owns the lifecycle — when the entity is despawned, the system can detect the removal and clean up Rapier state.

### Chunk Collider Lifecycle

Chunk collider creation and destruction is driven by events from the chunk system:

```rust
/// Fired when a chunk finishes loading and its voxel data is ready.
pub struct ChunkLoadedEvent {
    pub coord: ChunkCoord,
    pub entity: Entity,
}

/// Fired when a chunk is being unloaded.
pub struct ChunkUnloadedEvent {
    pub coord: ChunkCoord,
    pub entity: Entity,
}

/// Fired when one or more voxels in a chunk change.
pub struct VoxelChangedEvent {
    pub chunk_coord: ChunkCoord,
}
```

**On chunk load:**

```rust
fn on_chunk_loaded_system(
    mut commands: Commands,
    mut physics: ResMut<PhysicsWorld>,
    mut events: EventReader<ChunkLoadedEvent>,
    chunks: Res<ChunkManager>,
    origin: Res<PhysicsOrigin>,
    island: Res<PhysicsIsland>,
) {
    for event in events.read() {
        // Only create colliders for chunks within the physics island.
        let chunk_world_pos = chunk_coord_to_world_pos(&event.coord);
        if !island.contains(&chunk_world_pos) {
            continue;
        }

        if let Some(chunk) = chunks.get(&event.coord) {
            let local_pos = world_to_local(&chunk_world_pos, &origin.world_origin);
            if let Some(shape) = chunk_to_voxel_collider(chunk, 1.0) {
                let collider = ColliderBuilder::new(shape)
                    .translation(vector![local_pos.x, local_pos.y, local_pos.z])
                    .friction(0.7)
                    .restitution(0.0)
                    .build();
                let handle = physics.collider_set.insert(collider);
                commands.entity(event.entity).insert(ChunkCollider {
                    coord: event.coord,
                    handle,
                });
            }
        }
    }
}
```

**On chunk unload:**

```rust
fn on_chunk_unloaded_system(
    mut commands: Commands,
    mut physics: ResMut<PhysicsWorld>,
    mut events: EventReader<ChunkUnloadedEvent>,
    query: Query<&ChunkCollider>,
) {
    for event in events.read() {
        if let Ok(chunk_collider) = query.get(event.entity) {
            physics.collider_set.remove(
                chunk_collider.handle,
                &mut physics.island_manager,
                &mut physics.rigid_body_set,
                true,
            );
            commands.entity(event.entity).remove::<ChunkCollider>();
        }
    }
}
```

**On voxel change:**

```rust
fn on_voxel_changed_system(
    mut physics: ResMut<PhysicsWorld>,
    mut events: EventReader<VoxelChangedEvent>,
    chunks: Res<ChunkManager>,
    origin: Res<PhysicsOrigin>,
    mut chunk_query: Query<(Entity, &mut ChunkCollider)>,
) {
    // Deduplicate multiple changes to the same chunk in one tick.
    let mut dirty: HashSet<ChunkCoord> = HashSet::new();
    for event in events.read() {
        dirty.insert(event.chunk_coord);
    }

    for coord in dirty {
        // Find the chunk entity with this coordinate.
        for (entity, mut chunk_collider) in chunk_query.iter_mut() {
            if chunk_collider.coord != coord {
                continue;
            }

            // Remove old collider.
            physics.collider_set.remove(
                chunk_collider.handle,
                &mut physics.island_manager,
                &mut physics.rigid_body_set,
                true,
            );

            // Rebuild from updated voxel data.
            if let Some(chunk) = chunks.get(&coord) {
                let local_pos = world_to_local(
                    &chunk_coord_to_world_pos(&coord),
                    &origin.world_origin,
                );
                if let Some(shape) = chunk_to_voxel_collider(chunk, 1.0) {
                    let collider = ColliderBuilder::new(shape)
                        .translation(vector![local_pos.x, local_pos.y, local_pos.z])
                        .friction(0.7)
                        .build();
                    chunk_collider.handle = physics.collider_set.insert(collider);
                }
            }
            break;
        }
    }
}
```

### Entity Rigid Body Lifecycle

For non-chunk entities (NPCs, items, projectiles), rigid bodies and colliders are managed through component insertion/removal detection:

```rust
/// Marker component requesting physics body creation.
#[derive(Component)]
pub struct PhysicsBody {
    pub body_type: PhysicsBodyType,
    pub shape: PhysicsShape,
    pub mass: f32,
    pub friction: f32,
    pub restitution: f32,
}

pub enum PhysicsBodyType {
    Dynamic,
    Static,
    KinematicPositionBased,
    KinematicVelocityBased,
}

pub enum PhysicsShape {
    Capsule { half_height: f32, radius: f32 },
    Cuboid { half_extents: glam::Vec3 },
    Sphere { radius: f32 },
    ConvexHull { points: Vec<glam::Vec3> },
}
```

**On entity spawn with PhysicsBody (Added detection):**

```rust
fn spawn_physics_bodies_system(
    mut commands: Commands,
    mut physics: ResMut<PhysicsWorld>,
    origin: Res<PhysicsOrigin>,
    query: Query<(Entity, &WorldPos, &PhysicsBody), Added<PhysicsBody>>,
) {
    for (entity, world_pos, body_def) in query.iter() {
        let local_pos = world_to_local(world_pos, &origin.world_origin);

        let body = match body_def.body_type {
            PhysicsBodyType::Dynamic => RigidBodyBuilder::dynamic(),
            PhysicsBodyType::Static => RigidBodyBuilder::fixed(),
            PhysicsBodyType::KinematicPositionBased =>
                RigidBodyBuilder::kinematic_position_based(),
            PhysicsBodyType::KinematicVelocityBased =>
                RigidBodyBuilder::kinematic_velocity_based(),
        }
        .translation(vector![local_pos.x, local_pos.y, local_pos.z])
        .build();

        let body_handle = physics.rigid_body_set.insert(body);

        let shape = match &body_def.shape {
            PhysicsShape::Capsule { half_height, radius } =>
                SharedShape::capsule_y(*half_height, *radius),
            PhysicsShape::Cuboid { half_extents } =>
                SharedShape::cuboid(half_extents.x, half_extents.y, half_extents.z),
            PhysicsShape::Sphere { radius } =>
                SharedShape::ball(*radius),
            PhysicsShape::ConvexHull { points } => {
                let pts: Vec<_> = points.iter()
                    .map(|p| rapier3d::prelude::Point::new(p.x, p.y, p.z))
                    .collect();
                SharedShape::convex_hull(&pts).unwrap_or(SharedShape::ball(0.5))
            }
        };

        let collider = ColliderBuilder::new(shape)
            .friction(body_def.friction)
            .restitution(body_def.restitution)
            .build();
        let collider_handle = physics.collider_set.insert_with_parent(
            collider, body_handle, &mut physics.rigid_body_set,
        );

        commands.entity(entity).insert((
            RigidBodyHandle(body_handle),
            ColliderHandle(collider_handle),
        ));
    }
}
```

**On entity despawn (RemovedComponents detection):**

```rust
fn despawn_physics_bodies_system(
    mut physics: ResMut<PhysicsWorld>,
    mut removals: RemovedComponents<RigidBodyHandle>,
    // Cache of handle values, populated when handles are inserted.
    mut handle_cache: ResMut<DespawnedHandleCache>,
) {
    for entity in removals.read() {
        if let Some(body_handle) = handle_cache.remove(&entity) {
            physics.rigid_body_set.remove(
                body_handle,
                &mut physics.island_manager,
                &mut physics.collider_set,
                &mut physics.impulse_joint_set,
                &mut physics.multibody_joint_set,
                true, // Remove attached colliders automatically.
            );
        }
    }
}
```

The `DespawnedHandleCache` is a `HashMap<Entity, rapier3d::prelude::RigidBodyHandle>` that mirrors the `RigidBodyHandle` component. It is needed because by the time `RemovedComponents` fires, the component data is already gone from the entity.

### Orphan Detection (Debug Only)

In debug builds, a validation system runs periodically to detect orphaned Rapier handles:

```rust
#[cfg(debug_assertions)]
fn orphan_detection_system(
    physics: Res<PhysicsWorld>,
    body_query: Query<&RigidBodyHandle>,
    collider_query: Query<&ColliderHandle>,
    chunk_query: Query<&ChunkCollider>,
) {
    let ecs_body_handles: HashSet<_> = body_query.iter()
        .map(|h| h.0)
        .collect();
    let ecs_collider_handles: HashSet<_> = collider_query.iter()
        .map(|h| h.0)
        .chain(chunk_query.iter().map(|c| c.handle))
        .collect();

    for (handle, _) in physics.rigid_body_set.iter() {
        if !ecs_body_handles.contains(&handle) {
            warn!("Orphaned rigid body detected: {:?}", handle);
        }
    }

    for (handle, collider) in physics.collider_set.iter() {
        if collider.parent().is_none() && !ecs_collider_handles.contains(&handle) {
            warn!("Orphaned collider detected: {:?}", handle);
        }
    }
}
```

### System Ordering

All lifecycle systems run in `FixedUpdate`, before the physics step:

1. `on_chunk_loaded_system`
2. `on_chunk_unloaded_system`
3. `on_voxel_changed_system`
4. `spawn_physics_bodies_system`
5. `despawn_physics_bodies_system`
6. `physics_island_update_system` (story 02)
7. `recenter_physics_origin` (story 03)
8. `bridge_write_to_rapier` (story 03)
9. `physics_step_system` (story 01)
10. `bridge_read_from_rapier` (story 03)

## Outcome

Rapier rigid bodies and colliders are created, updated, and destroyed in lockstep with ECS entities and chunk lifecycle events. No orphaned handles exist. No physics-eligible entity lacks its handles. Voxel changes immediately rebuild the affected chunk's collider. `cargo test -p nebula-physics` passes all lifecycle synchronization tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Colliders are created when chunks load and destroyed when chunks unload. Modifying a voxel regenerates the affected chunk's collider. No stale or phantom collisions.

## Crates & Dependencies

- `rapier3d = "0.32"` — Rigid body and collider CRUD operations, handle types, sparse voxel collider construction
- `parry3d = "0.26"` — `SharedShape` construction for various collider geometries
- `bevy_ecs = "0.18"` — `Added<T>` and `RemovedComponents<T>` change detection, `Commands` for inserting/removing components, event readers, schedule ordering
- `glam = "0.32"` — Vector math for position conversions and shape definitions
- `nebula-voxel` (internal) — `ChunkManager`, `ChunkCoord`, chunk events, voxel-to-collider conversion
- `nebula-coords` (internal) — `WorldPos`, coordinate conversions
- `nebula-physics` (internal, self) — `PhysicsWorld`, `PhysicsIsland`, `PhysicsOrigin`, bridge functions

## Unit Tests

- **`test_chunk_load_creates_collider`** — Fire a `ChunkLoadedEvent` for a chunk containing solid voxels within the physics island. Run the `on_chunk_loaded_system`. Assert the chunk entity now has a `ChunkCollider` component. Assert the collider handle is valid in `physics.collider_set`. Assert the collider's position matches the chunk's local-space position.

- **`test_chunk_unload_removes_collider`** — Load a chunk and create its collider. Fire a `ChunkUnloadedEvent`. Run the `on_chunk_unloaded_system`. Assert the `ChunkCollider` component is removed from the entity. Assert the collider handle is no longer present in `physics.collider_set`.

- **`test_voxel_change_triggers_collider_rebuild`** — Load a chunk, create its collider, record the `ColliderHandle` value. Fire a `VoxelChangedEvent` for that chunk (after modifying voxel data). Run `on_voxel_changed_system`. Assert the old handle is no longer valid. Assert the entity has a new `ChunkCollider` with a different handle. Assert the new collider exists in `physics.collider_set`.

- **`test_entity_spawn_creates_body`** — Spawn an entity with `WorldPos`, `PhysicsBody { body_type: Dynamic, shape: Sphere { radius: 0.5 }, .. }`. Run `spawn_physics_bodies_system`. Assert the entity now has `RigidBodyHandle` and `ColliderHandle` components. Assert the body exists in `physics.rigid_body_set` with the correct position. Assert the collider exists and is attached to the body.

- **`test_entity_despawn_removes_body`** — Spawn an entity with physics, create its body. Despawn the entity. Run `despawn_physics_bodies_system`. Assert the rigid body handle is no longer valid in `physics.rigid_body_set`. Assert the associated collider is also removed.

- **`test_no_orphaned_colliders`** — Spawn 10 entities with physics. Despawn 5 of them. Run lifecycle systems. Assert `physics.rigid_body_set.len() == 5`. Assert `physics.collider_set.len()` equals the number of remaining entity colliders plus any chunk colliders. Run the orphan detection system and assert no warnings are emitted.

- **`test_multiple_voxel_changes_single_rebuild`** — Fire three `VoxelChangedEvent` events for the same chunk coordinate in one tick. Run `on_voxel_changed_system`. Assert the collider was rebuilt only once (the system deduplicates). This can be verified by checking that `collider_set.len()` equals the expected count (no duplicates) and that only one insertion occurred.
