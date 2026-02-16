//! Collider lifecycle management: synchronizes Rapier physics state with ECS entities.
//!
//! Provides event-driven functions for chunk collider creation/destruction/rebuild
//! and ECS systems for entity rigid body spawn/despawn.

use std::collections::HashSet;

use bevy_ecs::prelude::*;
use rustc_hash::FxHashMap;

use rapier3d::prelude::*;

use nebula_voxel::ChunkAddress;

use crate::PhysicsWorld;

// ---------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------

/// Wraps a Rapier collider handle as an ECS component.
#[derive(Component)]
pub struct ColliderHandle(pub rapier3d::prelude::ColliderHandle);

/// Associates a chunk entity with its Rapier collider.
#[derive(Component)]
pub struct ChunkCollider {
    /// The chunk address this collider represents.
    pub coord: ChunkAddress,
    /// The Rapier collider handle for this chunk.
    pub handle: rapier3d::prelude::ColliderHandle,
}

/// Marker + definition component requesting physics body creation.
///
/// When added to an entity that also has an [`crate::physics_island::IslandWorldPos`],
/// the [`spawn_physics_bodies`] system creates a Rapier rigid body and collider.
#[derive(Component)]
pub struct PhysicsBody {
    /// The type of rigid body to create.
    pub body_type: PhysicsBodyType,
    /// The collision shape.
    pub shape: PhysicsShape,
    /// Mass in kilograms.
    pub mass: f32,
    /// Surface friction coefficient.
    pub friction: f32,
    /// Restitution (bounciness).
    pub restitution: f32,
}

/// Rigid body type selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhysicsBodyType {
    /// Fully simulated body affected by forces and gravity.
    Dynamic,
    /// Immovable body (infinite mass).
    Static,
    /// Kinematic body controlled by position.
    KinematicPositionBased,
    /// Kinematic body controlled by velocity.
    KinematicVelocityBased,
}

/// Collision shape definition.
#[derive(Debug, Clone)]
pub enum PhysicsShape {
    /// Capsule aligned along the Y axis.
    Capsule {
        /// Half the height of the cylindrical part.
        half_height: f32,
        /// Radius of the hemispheres.
        radius: f32,
    },
    /// Axis-aligned box.
    Cuboid {
        /// Half-extents along each axis.
        half_extents: glam::Vec3,
    },
    /// Sphere.
    Sphere {
        /// Radius.
        radius: f32,
    },
    /// Convex hull from a point cloud.
    ConvexHull {
        /// Points defining the hull.
        points: Vec<glam::Vec3>,
    },
}

// ---------------------------------------------------------------------------
// Resources
// ---------------------------------------------------------------------------

/// Caches `RigidBodyHandle` values for entities so they can be cleaned up
/// after the component is removed (at which point component data is gone).
#[derive(Resource, Default)]
pub struct DespawnedHandleCache {
    map: FxHashMap<Entity, rapier3d::prelude::RigidBodyHandle>,
}

impl DespawnedHandleCache {
    /// Creates a new empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts a handle for the given entity.
    pub fn insert(&mut self, entity: Entity, handle: rapier3d::prelude::RigidBodyHandle) {
        self.map.insert(entity, handle);
    }

    /// Removes and returns the handle for the given entity.
    pub fn remove(&mut self, entity: &Entity) -> Option<rapier3d::prelude::RigidBodyHandle> {
        self.map.remove(entity)
    }

    /// Returns the number of cached handles.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Returns `true` if the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Chunk lifecycle functions
// ---------------------------------------------------------------------------

/// Creates a Rapier collider for a loaded chunk and inserts [`ChunkCollider`] on
/// the chunk entity via `commands`.
///
/// Delegates to [`crate::voxel_collision::create_chunk_collider`] for shape
/// construction. Returns the collider handle if one was created.
pub fn on_chunk_loaded(
    commands: &mut Commands,
    physics: &mut PhysicsWorld,
    chunk: &nebula_voxel::Chunk,
    registry: &nebula_voxel::VoxelTypeRegistry,
    origin: &crate::PhysicsOrigin,
    coord: ChunkAddress,
    entity: Entity,
) -> Option<rapier3d::prelude::ColliderHandle> {
    let local_pos = crate::world_to_local(&chunk_addr_to_world_pos(&coord), &origin.world_origin);
    let handle =
        crate::voxel_collision::create_chunk_collider(physics, chunk, registry, local_pos, 1.0)?;
    commands
        .entity(entity)
        .insert(ChunkCollider { coord, handle });
    Some(handle)
}

/// Removes a chunk's Rapier collider and strips the [`ChunkCollider`] component.
pub fn on_chunk_unloaded(
    commands: &mut Commands,
    physics: &mut PhysicsWorld,
    chunk_collider: &ChunkCollider,
    entity: Entity,
) {
    physics.collider_set.remove(
        chunk_collider.handle,
        &mut physics.island_manager,
        &mut physics.rigid_body_set,
        true,
    );
    commands.entity(entity).remove::<ChunkCollider>();
}

/// Rebuilds colliders for all chunks in `dirty_coords`.
///
/// Deduplication is the caller's responsibility (use a [`HashSet`]).
/// This function removes the old collider, rebuilds from current voxel data,
/// and updates the [`ChunkCollider`] component in-place.
pub fn on_voxel_changed(
    physics: &mut PhysicsWorld,
    dirty_coords: &HashSet<ChunkAddress>,
    chunks: &nebula_voxel::ChunkManager,
    registry: &nebula_voxel::VoxelTypeRegistry,
    origin: &crate::PhysicsOrigin,
    chunk_colliders: &mut [(Entity, &mut ChunkCollider)],
) {
    for coord in dirty_coords {
        for (_entity, cc) in chunk_colliders.iter_mut() {
            if cc.coord != *coord {
                continue;
            }

            // Remove old collider.
            physics.collider_set.remove(
                cc.handle,
                &mut physics.island_manager,
                &mut physics.rigid_body_set,
                true,
            );

            // Rebuild from updated voxel data.
            if let Some(chunk) = chunks.get_chunk(coord) {
                let local_pos =
                    crate::world_to_local(&chunk_addr_to_world_pos(coord), &origin.world_origin);
                if let Some(shape) =
                    crate::voxel_collision::chunk_to_voxel_collider(chunk, registry, 1.0)
                {
                    let collider = ColliderBuilder::new(shape)
                        .translation(Vector::new(local_pos.x, local_pos.y, local_pos.z))
                        .friction(0.7)
                        .restitution(0.0)
                        .build();
                    cc.handle = physics.collider_set.insert(collider);
                }
            }
            break;
        }
    }
}

/// Collects dirty chunk coordinates from voxel change events, deduplicating.
pub fn deduplicate_voxel_changes<'a>(
    events: impl Iterator<Item = &'a ChunkAddress>,
) -> HashSet<ChunkAddress> {
    events.copied().collect()
}

// ---------------------------------------------------------------------------
// Entity lifecycle systems (Bevy ECS)
// ---------------------------------------------------------------------------

/// Creates Rapier rigid bodies and colliders for entities that just gained a [`PhysicsBody`].
///
/// Also populates the [`DespawnedHandleCache`] so handles can be cleaned up on despawn.
pub fn spawn_physics_bodies(
    mut commands: Commands,
    mut physics: ResMut<PhysicsWorld>,
    origin: Res<crate::PhysicsOrigin>,
    mut handle_cache: ResMut<DespawnedHandleCache>,
    query: Query<
        (Entity, &crate::physics_island::IslandWorldPos, &PhysicsBody),
        Added<PhysicsBody>,
    >,
) {
    for (entity, world_pos, body_def) in query.iter() {
        let local_pos = crate::world_to_local(&world_pos.0, &origin.world_origin);

        let body = match body_def.body_type {
            PhysicsBodyType::Dynamic => RigidBodyBuilder::dynamic(),
            PhysicsBodyType::Static => RigidBodyBuilder::fixed(),
            PhysicsBodyType::KinematicPositionBased => RigidBodyBuilder::kinematic_position_based(),
            PhysicsBodyType::KinematicVelocityBased => RigidBodyBuilder::kinematic_velocity_based(),
        }
        .translation(Vector::new(local_pos.x, local_pos.y, local_pos.z))
        .build();

        let body_handle = physics.rigid_body_set.insert(body);

        let shape = build_shared_shape(&body_def.shape);

        let collider = ColliderBuilder::new(shape)
            .friction(body_def.friction)
            .restitution(body_def.restitution)
            .build();
        let phys = &mut *physics;
        let collider_handle =
            phys.collider_set
                .insert_with_parent(collider, body_handle, &mut phys.rigid_body_set);

        handle_cache.insert(entity, body_handle);

        commands.entity(entity).insert((
            crate::physics_island::RigidBodyHandle(body_handle),
            ColliderHandle(collider_handle),
        ));
    }
}

/// Removes Rapier rigid bodies (and their attached colliders) for despawned entities.
pub fn despawn_physics_bodies(
    mut physics: ResMut<PhysicsWorld>,
    mut removals: RemovedComponents<crate::physics_island::RigidBodyHandle>,
    mut handle_cache: ResMut<DespawnedHandleCache>,
) {
    for entity in removals.read() {
        if let Some(body_handle) = handle_cache.remove(&entity) {
            let phys = &mut *physics;
            phys.rigid_body_set.remove(
                body_handle,
                &mut phys.island_manager,
                &mut phys.collider_set,
                &mut phys.impulse_joint_set,
                &mut phys.multibody_joint_set,
                true,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Orphan detection (debug only)
// ---------------------------------------------------------------------------

/// Detects Rapier handles that have no corresponding ECS component.
///
/// Only compiled in debug builds. Logs warnings for orphaned handles.
#[cfg(debug_assertions)]
pub fn orphan_detection_system(
    physics: Res<PhysicsWorld>,
    body_query: Query<&crate::physics_island::RigidBodyHandle>,
    collider_query: Query<&ColliderHandle>,
    chunk_query: Query<&ChunkCollider>,
) {
    let ecs_body_handles: HashSet<_> = body_query.iter().map(|h| h.0).collect();
    let ecs_collider_handles: HashSet<_> = collider_query
        .iter()
        .map(|h| h.0)
        .chain(chunk_query.iter().map(|c| c.handle))
        .collect();

    for (handle, _) in physics.rigid_body_set.iter() {
        if !ecs_body_handles.contains(&handle) {
            tracing::warn!("Orphaned rigid body detected: {:?}", handle);
        }
    }

    for (handle, collider) in physics.collider_set.iter() {
        if collider.parent().is_none() && !ecs_collider_handles.contains(&handle) {
            tracing::warn!("Orphaned collider detected: {:?}", handle);
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Converts a [`ChunkAddress`] to a [`nebula_math::WorldPosition`] in millimeters.
///
/// Each chunk is 32 voxels across; each voxel is 1 meter = 1000 mm.
fn chunk_addr_to_world_pos(addr: &ChunkAddress) -> nebula_math::WorldPosition {
    let mm_per_chunk: i128 = 32 * 1000;
    nebula_math::WorldPosition::new(
        addr.x as i128 * mm_per_chunk,
        addr.y as i128 * mm_per_chunk,
        addr.z as i128 * mm_per_chunk,
    )
}

/// Builds a [`SharedShape`] from a [`PhysicsShape`] definition.
fn build_shared_shape(shape: &PhysicsShape) -> SharedShape {
    match shape {
        PhysicsShape::Capsule {
            half_height,
            radius,
        } => SharedShape::capsule_y(*half_height, *radius),
        PhysicsShape::Cuboid { half_extents } => {
            SharedShape::cuboid(half_extents.x, half_extents.y, half_extents.z)
        }
        PhysicsShape::Sphere { radius } => SharedShape::ball(*radius),
        PhysicsShape::ConvexHull { points } => {
            let pts: Vec<Vector> = points.iter().map(|p| Vector::new(p.x, p.y, p.z)).collect();
            SharedShape::convex_hull(&pts).unwrap_or_else(|| SharedShape::ball(0.5))
        }
    }
}

#[cfg(test)]
#[path = "collider_lifecycle_tests.rs"]
mod tests;
