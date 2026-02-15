# Voxel Collision Shapes

## Problem

In a voxel-based game engine, the physical world is defined by millions of individual voxel blocks arranged in chunks. Players and entities must collide with solid terrain — walking on surfaces, bumping into walls, falling through caves. Traditional mesh-based collision is prohibitively expensive at this scale: a 32x32x32 chunk could contain up to 32,768 individual cube colliders if done naively. Even greedy-meshed triangle colliders carry overhead in Rapier's collision pipeline. Fortunately, Rapier 0.32 introduced **sparse voxel colliders** — the first physics engine with explicit, native support for voxel collision geometry. This feature represents voxel volumes as a sparse data structure rather than decomposing them into individual convex shapes or triangle meshes, dramatically reducing memory usage and broadphase overhead. The engine must generate these colliders from chunk data, keep them in sync as voxels change (mining, building), and remove them when chunks unload.

## Solution

### Sparse Voxel Collider Generation

When a chunk loads within the physics island radius, its voxel data is converted into a Rapier sparse voxel collider. Rapier 0.32's `SharedShape::voxels()` API accepts a 3D grid of occupancy data:

```rust
use rapier3d::prelude::*;
use nebula_voxel::Chunk;

pub fn chunk_to_voxel_collider(chunk: &Chunk, voxel_size: f32) -> Option<SharedShape> {
    let dim = chunk.size(); // e.g., 32

    // Build the occupancy grid. Rapier expects a flat array indexed [x][y][z].
    let mut filled = Vec::with_capacity(dim * dim * dim);
    let mut any_solid = false;

    for x in 0..dim {
        for y in 0..dim {
            for z in 0..dim {
                let solid = chunk.get(x, y, z).is_solid();
                filled.push(solid);
                any_solid |= solid;
            }
        }
    }

    if !any_solid {
        return None; // Entirely empty chunk — no collider needed.
    }

    Some(SharedShape::voxels(
        voxel_size,
        [dim as u32, dim as u32, dim as u32],
        filled,
    ))
}
```

The sparse voxel collider internally stores only the occupied cells, so a chunk with 10% solid voxels uses roughly 10% of the memory compared to a dense grid. Rapier handles the broadphase spatial lookup internally using an optimized spatial hash over the occupied cells.

### Collider Attachment

Each chunk collider is attached as a **static** body (zero mass, immovable) positioned at the chunk's local-space origin:

```rust
pub fn create_chunk_collider(
    physics: &mut PhysicsWorld,
    chunk: &Chunk,
    chunk_local_pos: glam::Vec3,
    voxel_size: f32,
) -> Option<ColliderHandle> {
    let shape = chunk_to_voxel_collider(chunk, voxel_size)?;

    let collider = ColliderBuilder::new(shape)
        .translation(vector![
            chunk_local_pos.x,
            chunk_local_pos.y,
            chunk_local_pos.z
        ])
        .friction(0.7)
        .restitution(0.0)
        .build();

    Some(physics.collider_set.insert(collider))
}
```

The chunk's local position is computed by the i128-to-f32 bridge (story 03), converting the chunk's `WorldPos` corner to an offset from the physics origin.

### Collider Update on Voxel Change

When a player mines or places a block, the affected chunk's collider must be rebuilt. Rather than modifying the voxel collider in-place (which Rapier does not support for sparse voxels), the engine removes the old collider and inserts a new one:

```rust
fn update_chunk_collider_system(
    mut physics: ResMut<PhysicsWorld>,
    mut events: EventReader<VoxelChangedEvent>,
    chunks: Res<ChunkManager>,
    mut collider_map: ResMut<ChunkColliderMap>,
    origin: Res<PhysicsOrigin>,
) {
    // Deduplicate: multiple voxel changes in the same chunk in one tick
    // only need one rebuild.
    let mut dirty_chunks: HashSet<ChunkCoord> = HashSet::new();
    for event in events.read() {
        dirty_chunks.insert(event.chunk_coord);
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
        if let Some(chunk) = chunks.get(&coord) {
            let local_pos = chunk_world_to_local(&coord, &origin);
            if let Some(handle) = create_chunk_collider(
                &mut physics, chunk, local_pos, 1.0,
            ) {
                collider_map.insert(coord, handle);
            }
        }
    }
}
```

### Collider Removal on Chunk Unload

When a chunk leaves the physics island (either because the player moved away or the chunk was explicitly unloaded), its collider is removed:

```rust
fn remove_unloaded_chunk_colliders_system(
    mut physics: ResMut<PhysicsWorld>,
    mut events: EventReader<ChunkUnloadedEvent>,
    mut collider_map: ResMut<ChunkColliderMap>,
) {
    for event in events.read() {
        if let Some(handle) = collider_map.remove(&event.coord) {
            physics.collider_set.remove(
                handle,
                &mut physics.island_manager,
                &mut physics.rigid_body_set,
                true,
            );
        }
    }
}
```

### ChunkColliderMap

A resource mapping chunk coordinates to their active Rapier collider handles:

```rust
#[derive(Resource, Default)]
pub struct ChunkColliderMap {
    map: HashMap<ChunkCoord, ColliderHandle>,
}
```

This allows O(1) lookup when a chunk needs its collider updated or removed.

### Performance Considerations

- Sparse voxel colliders avoid per-voxel broadphase entries, keeping Rapier's internal structures compact.
- Collider rebuilds are batched per tick — multiple voxel changes in the same chunk produce only one rebuild.
- Only chunks within the physics island radius (~512m) have active colliders, limiting the total to approximately `(512*2/32)^3 ≈ 32,768` chunks worst case, though in practice far fewer are solid enough to need colliders.

## Outcome

Every loaded chunk near the player has a Rapier sparse voxel collider that accurately represents its solid blocks. Players and entities collide with terrain naturally. Voxel modifications (mining, building) immediately update the collision geometry. Chunk unloads cleanly remove colliders. `cargo test -p nebula-physics` passes all voxel collision tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Terrain chunks generate collision meshes from their voxel data. The player cannot walk through solid terrain. The physics debug overlay shows collider wireframes matching terrain.

## Crates & Dependencies

- `rapier3d = "0.32"` — Sparse voxel collider support via `SharedShape::voxels()`, the first physics engine with native voxel collision
- `parry3d = "0.26"` — Underlying collision geometry library used by Rapier; provides `SharedShape`
- `bevy_ecs = "0.18"` — ECS framework for systems, events, resources, and queries
- `glam = "0.32"` — Vector math for chunk position calculations
- `nebula-voxel` (internal) — `Chunk` data structure, `ChunkCoord`, `VoxelChangedEvent`, `ChunkManager`
- `nebula-coords` (internal) — Coordinate conversions for chunk positions

## Unit Tests

- **`test_solid_voxel_blocks_movement`** — Create a chunk with a solid floor (y=0 layer filled). Generate its collider, insert into the physics world. Place a dynamic sphere body at `(16, 5, 16)` above the floor. Step physics 120 times. Assert the sphere's y-position is approximately 0.5 (resting on the floor, accounting for sphere radius). The sphere should not fall through.

- **`test_air_voxel_allows_passage`** — Create a chunk that is entirely air except for a floor at y=0 with a 1-block hole at `(16, 0, 16)`. Place a small dynamic sphere directly above the hole. Step physics. Assert the sphere falls through the hole (y-position decreases below 0). Air voxels must not generate collision geometry.

- **`test_chunk_collider_updates_on_voxel_change`** — Create a chunk with a solid floor. Generate collider. Place a sphere resting on the floor at `(16, 1, 16)`. Remove the floor voxel at `(16, 0, 16)` and trigger a `VoxelChangedEvent`. Run the update system to rebuild the collider. Step physics. Assert the sphere falls through the now-missing voxel.

- **`test_collider_removed_on_chunk_unload`** — Create a chunk, generate its collider, record the `ColliderHandle`. Fire a `ChunkUnloadedEvent`. Run the removal system. Assert the handle is no longer present in `physics.collider_set` and the `ChunkColliderMap` no longer contains the chunk coordinate.

- **`test_collider_shape_matches_chunk_geometry`** — Create a chunk with a known pattern (e.g., a 4x4x4 solid cube in the corner). Generate the collider. Use Rapier's `QueryPipeline::cast_ray` to cast rays at known solid and empty positions. Assert rays hit where voxels are solid and miss where voxels are air. Verifies geometric fidelity of the generated collider.

- **`test_empty_chunk_produces_no_collider`** — Create an entirely empty (air) chunk. Call `chunk_to_voxel_collider`. Assert it returns `None`. No collider should be inserted into the physics world for empty chunks.
