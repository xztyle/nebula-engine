# Voxel Modification Events

## Problem

When a voxel changes — whether from player interaction, terrain generation, physics destruction, or scripted modification — multiple independent systems must react. The meshing system needs to rebuild the affected chunk's mesh (and possibly neighboring chunks' meshes if the voxel is on a boundary). The lighting system needs to repropagate light through the changed region. The physics system needs to rebuild the chunk's collision shape. The networking system needs to replicate the change to other clients. These systems should not poll every chunk every frame for changes (wasteful) and should not be tightly coupled to the code that performs the modification (inflexible). An event-driven architecture decouples the modification source from the responding systems and allows batching multiple changes within a single frame for efficient processing.

## Solution

Implement a voxel modification event system in `nebula-voxel` using Bevy's `Events<T>` mechanism.

### Event Definition

```rust
/// Emitted when one or more voxels in a chunk are modified.
#[derive(Clone, Debug)]
pub struct VoxelModifiedEvent {
    /// The chunk containing the modified voxel.
    pub chunk: ChunkAddress,
    /// Local position within the chunk (each component in [0, 32)).
    pub local_pos: (u8, u8, u8),
    /// The voxel type that was at this position before the modification.
    pub old_type: VoxelTypeId,
    /// The voxel type now at this position after the modification.
    pub new_type: VoxelTypeId,
}
```

### Event Emission

The `Chunk::set()` method is extended to emit events. Since `Chunk` itself does not have access to Bevy's event writer, the event emission happens at the system level. A wrapper function or system parameter handles this:

```rust
/// System-level wrapper for voxel modification that emits events.
pub fn set_voxel(
    chunk_manager: &mut ChunkManager,
    addr: &ChunkAddress,
    x: u8, y: u8, z: u8,
    new_type: VoxelTypeId,
    events: &mut EventWriter<VoxelModifiedEvent>,
) {
    if let Some(chunk) = chunk_manager.get_chunk_mut(addr) {
        let old_type = chunk.get(x, y, z);

        // Skip if the voxel is already the requested type.
        if old_type == new_type {
            return;
        }

        chunk.set(x, y, z, new_type);

        events.send(VoxelModifiedEvent {
            chunk: *addr,
            local_pos: (x, y, z),
            old_type,
            new_type,
        });
    }
}
```

### No-Event on Same-Type Set

If `set()` is called with the same type that already occupies the voxel, no event is emitted and no modification occurs. This prevents downstream systems from doing unnecessary work when gameplay code redundantly sets a voxel to its current type (common in brush tools that paint over an area).

### Batch Events

Multiple voxel modifications in a single frame (e.g., an explosion destroying a 5x5x5 region = 125 voxels) produce 125 individual `VoxelModifiedEvent` entries. Bevy's `Events<T>` system stores them in a ring buffer that is double-buffered per frame. Consuming systems read all events from the previous frame using `EventReader<VoxelModifiedEvent>`.

For bulk operations, an additional batch event can reduce per-event overhead:

```rust
/// Emitted for bulk modifications (e.g., explosions, terrain generation).
#[derive(Clone, Debug)]
pub struct VoxelBatchModifiedEvent {
    /// The chunk that was modified.
    pub chunk: ChunkAddress,
    /// Number of voxels changed in this batch.
    pub count: u32,
}
```

Systems that only need to know "this chunk changed" (like meshing) can listen to `VoxelBatchModifiedEvent` instead of processing individual events.

### Subscriber Systems

Systems register as event readers in their Bevy system signature:

```rust
fn meshing_response_system(
    mut events: EventReader<VoxelModifiedEvent>,
    mut mesh_queue: ResMut<MeshRebuildQueue>,
) {
    for event in events.read() {
        mesh_queue.enqueue(event.chunk);
        // Also enqueue neighbors if the voxel is on a chunk boundary
        if event.local_pos.0 == 0 { mesh_queue.enqueue(neighbor(event.chunk, -X)); }
        if event.local_pos.0 == 31 { mesh_queue.enqueue(neighbor(event.chunk, +X)); }
        // ... y, z boundaries similarly
    }
}
```

### Event Lifecycle

Bevy's event system automatically handles lifecycle:
- Events are available for 2 frames (current + 1 for late readers).
- After that, they are dropped.
- No manual clearing is needed.

## Outcome

A `VoxelModifiedEvent` struct and event emission pipeline that notifies downstream systems (meshing, lighting, physics, networking) when voxels change. Events contain the chunk address, local position, old type, and new type. Same-type modifications are suppressed. Batch events provide coarse-grained notification for systems that do not need per-voxel detail.

## Demo Integration

**Demo crate:** `nebula-demo`

Modifying a voxel emits an event. The demo subscribes and logs: `VoxelModified { chunk: (0,3), pos: (5,17,8), old: Air, new: Stone }`.

## Crates & Dependencies

- **`bevy_ecs`** `0.15` — `Events<T>`, `EventWriter`, `EventReader` (workspace dependency)

## Unit Tests

- **`test_set_voxel_emits_event`** — Set a voxel from Air to Stone. Assert exactly one `VoxelModifiedEvent` was emitted with the correct `chunk`, `local_pos`, `old_type == VoxelTypeId(0)`, and `new_type == stone_id`.
- **`test_batch_set_emits_batch`** — Set 10 voxels in the same chunk in one frame. Assert 10 `VoxelModifiedEvent` entries are emitted. Optionally assert one `VoxelBatchModifiedEvent` with `count == 10`.
- **`test_event_contains_correct_old_new_types`** — Set position `(5,5,5)` to Stone, then in the next operation set it to Dirt. Assert the second event has `old_type == stone_id` and `new_type == dirt_id`.
- **`test_no_event_on_set_to_same_type`** — Set a voxel to Stone, then set it to Stone again. Assert only one event was emitted (the first set), not two.
- **`test_events_cleared_after_frame`** — Emit events in frame N. Advance to frame N+2. Assert the event reader yields zero events (Bevy's double-buffering has cleared them).
