# Voxel Edit Replication

## Problem

When a player places or removes a voxel block in a multiplayer world, the edit must be validated by the server, applied to the authoritative world state, and broadcast to all clients that have the affected chunk in their interest area. Without proper replication, clients would see inconsistent terrain — one player sees a block placed while another does not. The system must also handle concurrent edits from multiple players to the same chunk without conflicts or data corruption.

## Solution

### Edit Intent

The client sends an edit intent to the server, never directly modifying its own voxel data. The intent specifies the exact voxel to modify and the desired operation:

```rust
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum VoxelEditIntent {
    Place {
        chunk_id: ChunkId,
        local_pos: UVec3,
        material: VoxelMaterial,
        source_inventory_slot: u8,
    },
    Remove {
        chunk_id: ChunkId,
        local_pos: UVec3,
    },
}
```

### Server Validation

The server validates each edit intent against the authoritative world state before applying it:

| Check                       | Description                                                  |
| --------------------------- | ------------------------------------------------------------ |
| **Range check**             | Is the target voxel within the player's interaction radius (default: 6 m)? |
| **Chunk loaded**            | Is the target chunk currently loaded on the server?          |
| **Position valid**          | Is `local_pos` within the chunk bounds (0..CHUNK_SIZE)?      |
| **Inventory check (place)** | Does the player's inventory contain the specified material in the given slot? |
| **Not obstructed (place)**  | Is the target voxel currently empty (air)?                   |
| **Not empty (remove)**      | Is the target voxel currently non-air?                       |

```rust
pub fn validate_voxel_edit(
    world: &AuthoritativeWorld,
    player: Entity,
    edit: &VoxelEditIntent,
) -> Result<(), EditRejection> {
    // Range, chunk, bounds, inventory, obstruction checks
    // ...
}
```

If validation fails, the server sends an `EditRejected` message to the originating client with a reason code. The client reverts any optimistic local preview.

### Applying the Edit

Upon successful validation, the server applies the edit atomically:

1. Modify the voxel in the authoritative chunk data.
2. If placing: decrement the item count in the player's inventory.
3. Mark the chunk as dirty for persistence (Story 10).
4. Enqueue the edit for broadcast.

```rust
pub fn apply_voxel_edit(
    world: &mut AuthoritativeWorld,
    player: Entity,
    edit: &VoxelEditIntent,
) {
    match edit {
        VoxelEditIntent::Place { chunk_id, local_pos, material, source_inventory_slot } => {
            world.set_voxel(chunk_id, local_pos, *material);
            world.decrement_inventory(player, *source_inventory_slot);
        }
        VoxelEditIntent::Remove { chunk_id, local_pos } => {
            let removed_material = world.get_voxel(chunk_id, local_pos);
            world.set_voxel(chunk_id, local_pos, VoxelMaterial::Air);
            // Optionally add removed material to player inventory
        }
    }
}
```

### Broadcast

The server broadcasts a `VoxelEditEvent` to all clients whose interest area includes the affected chunk:

```rust
#[derive(Serialize, Deserialize)]
pub struct VoxelEditEvent {
    pub chunk_id: ChunkId,
    pub local_pos: UVec3,
    pub new_material: VoxelMaterial,
    pub editor_network_id: NetworkId,
    pub tick: u64,
}
```

Clients receiving this event update their local chunk cache and trigger a mesh rebuild for the affected chunk (and potentially adjacent chunks if the edit is on a boundary).

### Atomicity and Concurrency

Each voxel edit is atomic at the single-voxel level. The server processes edits sequentially within a tick, so two players editing the same voxel in the same tick are resolved by processing order (first-in wins). The second edit will either:

- Succeed if it is still valid after the first edit (e.g., removing a block that was just placed, if the second intent was "remove at position X").
- Fail validation (e.g., trying to place a block where one was just placed).

This avoids the need for complex conflict resolution or locking.

### Client-Side Preview

For responsiveness, the client may display an optimistic preview of the edit locally before server confirmation. If the server rejects the edit, the client reverts the preview. This is optional and purely visual.

## Outcome

- `nebula_multiplayer::voxel_edit` module containing `VoxelEditIntent`, `VoxelEditEvent`, `EditRejection`, `validate_voxel_edit`, and `apply_voxel_edit`.
- Server-validated voxel editing with inventory integration.
- Broadcast of edits to all interested clients.
- Atomic per-voxel operations with deterministic concurrency resolution.

## Demo Integration

**Demo crate:** `nebula-demo`

One player breaks a voxel. The edit is sent to the server, validated, applied, and broadcast to all nearby clients. Both players see the voxel disappear simultaneously.

## Crates & Dependencies

| Crate       | Version | Purpose                                        |
| ----------- | ------- | ---------------------------------------------- |
| `tokio`     | 1.49    | Async TCP for receiving intents and broadcasting |
| `serde`     | 1.0     | Serialization of edit intents and events        |
| `postcard`  | 1.1     | Binary wire format for edit messages            |
| `bevy_ecs`  | 0.18    | ECS access for world state and inventories      |

## Unit Tests

### `test_valid_edit_is_applied_and_broadcast`
Submit a `Place` intent for a valid empty voxel within range, with the material in inventory. Assert the server applies it (voxel changes to the specified material), inventory decrements, and a `VoxelEditEvent` is broadcast to interested clients.

### `test_invalid_edit_is_rejected`
Submit a `Place` intent for a voxel 100 m from the player (outside interaction radius). Assert the server returns `EditRejection::OutOfRange` and the voxel remains unchanged.

### `test_edit_reaches_all_interested_clients`
Connect three clients. Two have the affected chunk in their interest area, one does not. Apply a voxel edit. Assert the two interested clients receive the `VoxelEditEvent` and the third does not.

### `test_edit_modifies_correct_voxel`
Place material `Stone` at `local_pos (5, 10, 3)` in chunk `C`. Assert that the voxel at `(5, 10, 3)` in chunk `C` is now `Stone` and all other voxels in the chunk remain unchanged.

### `test_concurrent_edits_dont_conflict`
Two players submit `Place` intents for the same voxel in the same tick. Assert that exactly one succeeds (the first processed) and the other is rejected with `EditRejection::Obstructed`. The final voxel state matches the first edit's material.
