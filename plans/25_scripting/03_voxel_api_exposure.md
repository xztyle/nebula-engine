# Voxel API Exposure

## Problem

Scripted gameplay on cubesphere-voxel planets needs to read and modify terrain. Scripts that define magic abilities, environmental interactions, or procedural structures must be able to query voxel types, place and remove blocks, and cast rays against the voxel world. Unrestricted voxel access from scripts could corrupt chunk data, cause cascading remesh operations that stall the renderer, or allow scripts to modify terrain far outside their authority. A bounded, validated, deferred voxel API is required.

## Solution

Register Rhai functions that expose voxel operations through the same `ScriptContext` pattern used for ECS queries. All coordinates in the script API are relative to the context entity's position, preserving floating-point precision on 128-bit coordinate planets.

### Registered Functions

```rust
// Read the voxel type at a position relative to the context entity
fn get_voxel(ctx: &ScriptContext, x: f64, y: f64, z: f64) -> Result<ScriptVoxelTypeId, Box<EvalAltResult>>

// Queue a voxel modification (validated, deferred)
fn set_voxel(ctx: &ScriptContext, x: f64, y: f64, z: f64, voxel_type: ScriptVoxelTypeId) -> Result<(), Box<EvalAltResult>>

// Cast a ray from origin in direction, return first hit
fn raycast(ctx: &ScriptContext, origin: ScriptVec3, direction: ScriptVec3, max_dist: f64) -> Result<Dynamic, Box<EvalAltResult>>

// Get metadata about a chunk at a given address
fn get_chunk_data(ctx: &ScriptContext, chunk_x: i64, chunk_y: i64, chunk_z: i64) -> Result<Dynamic, Box<EvalAltResult>>
```

### Coordinate Translation

Script-local coordinates are translated to 128-bit absolute coordinates by adding the context entity's position. The translation happens inside each API function:

```rust
let abs_pos = ctx.entity_origin_128 + DVec3_128::from_f64(x, y, z);
let chunk_addr = ChunkAddress::from_absolute(abs_pos);
let local_offset = abs_pos.chunk_local_offset();
```

This means a script attached to an entity at position `(5_000_000, 0, 5_000_000)` can use `get_voxel(1.0, 0.0, 0.0)` to query the voxel one meter to its right without worrying about absolute coordinates.

### Validation Rules

Before any voxel modification is accepted into the command buffer:

1. **Range check**: The target position must be within a configurable radius of the context entity (default: 64 blocks). Out-of-range writes return `Err`.
2. **Permission check**: The script's permission flags are consulted. Scripts may be tagged `read_only`, `local_write` (within range), or `world_write` (admin/creative mode).
3. **Rate limit**: Each script may queue at most 256 voxel modifications per frame to prevent bulk-grief or accidental terrain destruction.
4. **Type validation**: The `VoxelTypeId` must exist in the voxel registry. Unknown types are rejected.

### HitResult Type

The `raycast` function returns a Rhai object-map with the following fields:

```rhai
// hit.position  -> Vec3 (hit point in script-local coords)
// hit.normal    -> Vec3 (surface normal)
// hit.voxel     -> VoxelTypeId (type of the voxel that was hit)
// hit.distance  -> f64 (distance from origin)
// hit.hit       -> bool (false if nothing was hit within max_dist)
```

### Deferred Voxel Commands

```rust
pub enum VoxelCommand {
    SetVoxel {
        position: DVec3_128,
        voxel_type: VoxelTypeId,
        source_entity: EntityId,
    },
}
```

Voxel commands are collected alongside ECS commands and applied in a dedicated `apply_voxel_commands` system that runs after script execution. This system batches modifications by chunk, marks affected chunks for remeshing, and emits `VoxelChanged` events for the event hook system.

### ChunkInfo Type

The `get_chunk_data` function returns a Rhai object-map:

```rhai
// chunk.loaded    -> bool (is the chunk currently in memory)
// chunk.solid     -> i64 (count of solid voxels)
// chunk.empty     -> i64 (count of air voxels)
// chunk.modified  -> bool (has the chunk been modified from its generated state)
```

## Outcome

Four voxel-interop functions registered on the Rhai engine, with coordinate translation preserving 128-bit precision, validation rules preventing abuse, and a deferred command pipeline that batches voxel modifications by chunk for efficient remeshing.

## Demo Integration

**Demo crate:** `nebula-demo`

Scripts can call `set_voxel(x,y,z,"stone")` and `get_voxel(x,y,z)` to read and modify voxels from Rhai.

## Crates & Dependencies

| Crate | Version | Purpose |
|---|---|---|
| `rhai` | `1.23` | Script function registration and Dynamic type |
| `glam` | `0.29` | Vector math for ray direction normalization |

## Unit Tests

```rust
#[test]
fn test_get_voxel_returns_correct_type() {
    let ctx = setup_voxel_test_context();
    // Place a stone voxel at (1, 0, 0) relative to context entity
    place_voxel_in_world(&ctx, 1.0, 0.0, 0.0, VoxelTypeId::STONE);
    let result = scripted_get_voxel(&ctx, 1.0, 0.0, 0.0).unwrap();
    assert_eq!(result.0, VoxelTypeId::STONE.0);
}

#[test]
fn test_set_voxel_modifies_world() {
    let ctx = setup_voxel_test_context();
    scripted_set_voxel(&ctx, 2.0, 0.0, 0.0, ScriptVoxelTypeId(VoxelTypeId::DIRT.0)).unwrap();
    let commands = ctx.voxel_commands.borrow();
    assert_eq!(commands.len(), 1);
    assert!(matches!(commands[0], VoxelCommand::SetVoxel { .. }));
}

#[test]
fn test_raycast_finds_solid_voxel() {
    let ctx = setup_voxel_test_context();
    place_voxel_in_world(&ctx, 5.0, 0.0, 0.0, VoxelTypeId::STONE);
    let origin = ScriptVec3 { x: 0.0, y: 0.0, z: 0.0 };
    let direction = ScriptVec3 { x: 1.0, y: 0.0, z: 0.0 };
    let hit = scripted_raycast(&ctx, origin, direction, 10.0).unwrap();
    let hit_map = hit.cast::<rhai::Map>();
    assert_eq!(hit_map["hit"].as_bool().unwrap(), true);
    assert!((hit_map["distance"].as_float().unwrap() - 5.0).abs() < 0.5);
}

#[test]
fn test_out_of_range_access_returns_error() {
    let ctx = setup_voxel_test_context(); // default range limit = 64
    let result = scripted_set_voxel(&ctx, 200.0, 0.0, 0.0, ScriptVoxelTypeId(1));
    assert!(result.is_err());
    let err_msg = format!("{}", result.unwrap_err());
    assert!(err_msg.contains("range"));
}

#[test]
fn test_modifications_are_deferred() {
    let ctx = setup_voxel_test_context();
    scripted_set_voxel(&ctx, 1.0, 0.0, 0.0, ScriptVoxelTypeId(VoxelTypeId::DIRT.0)).unwrap();
    // The world snapshot should still show the original voxel
    let current = scripted_get_voxel(&ctx, 1.0, 0.0, 0.0).unwrap();
    assert_eq!(current.0, VoxelTypeId::AIR.0); // not yet applied
    // But the command buffer should have the pending change
    assert_eq!(ctx.voxel_commands.borrow().len(), 1);
}
```
