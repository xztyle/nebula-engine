# Chunk Loading/Unloading

## Problem

The world is far too large to keep in memory all at once — a planet with a 6,400 km radius at 1-meter voxel resolution contains roughly 10^18 surface voxels. The engine must dynamically load chunks around the player's current position and unload distant chunks to reclaim memory. Naive implementations suffer from two key issues: **thrashing** (a chunk at the exact boundary of the load radius is repeatedly loaded and unloaded as the player oscillates near the edge) and **stuttering** (loading too many chunks in a single frame causes a frame-time spike). The loading system must address both problems while ensuring the player always has a fully loaded neighborhood of chunks for seamless gameplay.

## Solution

Implement a chunk loading/unloading system in `nebula-voxel` driven by the camera (player) position, with hysteresis and per-frame budgeting.

### Radius Configuration

```rust
pub struct ChunkLoadConfig {
    /// Chunks within this radius (in chunk units) around the camera are loaded.
    pub load_radius: u32,
    /// Chunks beyond this radius are unloaded.
    /// Must be > load_radius to create a hysteresis band.
    pub unload_radius: u32,
    /// Maximum number of chunk load operations per tick.
    pub loads_per_tick: u32,
    /// Maximum number of chunk unload operations per tick.
    pub unloads_per_tick: u32,
}
```

Example configuration: `load_radius: 8`, `unload_radius: 10`, `loads_per_tick: 4`, `unloads_per_tick: 8`. The 2-chunk hysteresis band means a chunk at distance 9 is neither loaded nor unloaded — it stays in whatever state it was already in.

### Priority Queue

Chunks that need to be loaded are inserted into a priority queue (min-heap) sorted by squared distance to the camera's chunk address. Nearest chunks are loaded first because they are most likely to be visible and interactable.

```rust
pub struct ChunkLoadQueue {
    /// Min-heap: (distance_squared, ChunkAddress)
    queue: BinaryHeap<Reverse<(u64, ChunkAddress)>>,
    /// Set of addresses already in the queue (to avoid duplicates).
    pending: HashSet<ChunkAddress>,
}
```

### System Flow

Each tick, the `chunk_loading_system` runs:

1. **Determine camera chunk address**: Convert the camera's world position (128-bit coordinates) to a `ChunkAddress` by dividing by chunk size (32).

2. **Scan for needed chunks**: Iterate over all chunk addresses within `load_radius` of the camera. For each address not currently loaded and not already in the load queue, add it to the priority queue.

3. **Process load queue**: Dequeue up to `loads_per_tick` entries from the priority queue. For each, initiate chunk loading (terrain generation or disk read). When the chunk data is ready (possibly async), insert it into the `ChunkManager`.

4. **Scan for unload candidates**: Iterate over all loaded chunks. For each chunk whose distance to the camera exceeds `unload_radius`, mark it as an unload candidate.

5. **Process unloads**: Unload up to `unloads_per_tick` candidates per tick. Before unloading, check `SAVE_DIRTY` — if the chunk has unsaved modifications, serialize it to disk first.

### Hysteresis

The gap between `load_radius` and `unload_radius` prevents thrashing:

- A chunk at distance 8 is loaded (within `load_radius`).
- The player moves away slightly — the chunk is now at distance 9, within the hysteresis band (> load_radius but < unload_radius). It stays loaded.
- The player moves further — the chunk is now at distance 11 (> unload_radius). It is unloaded.
- The player moves back — the chunk returns to distance 8 and is loaded again.

Without hysteresis, the chunk at distance 8 would toggle loaded/unloaded every frame as floating-point camera movement oscillates it across the boundary.

### Spherical vs. Cubic Radius

The radius check uses squared Euclidean distance (no square root needed), producing a spherical load region. This is more visually uniform than a cubic region (which loads unnecessary corner chunks at 1.73x the desired radius). For cubesphere planets, the distance calculation accounts for the curvature of the planet surface at the chunk scale, but at typical load radii (8-16 chunks = 256-512 voxels) the surface is effectively flat.

## Outcome

A chunk loading/unloading system that keeps chunks loaded in a sphere around the player, uses hysteresis to prevent thrashing at boundaries, and budgets load/unload operations per frame to avoid stuttering. Integrated as a Bevy ECS system running in the `PreUpdate` schedule.

## Demo Integration

**Demo crate:** `nebula-demo`

The camera orbit drives chunk lifecycle. Chunks beyond a load radius are unloaded. The title shows the loaded count fluctuating: `Loaded: 23... 25... 24...`.

## Crates & Dependencies

- **`bevy_ecs`** `0.15` — System scheduling, resource access (workspace dependency)
- **`rustc-hash`** `2.1` — `FxHashSet` for the pending-load set

## Unit Tests

- **`test_chunks_within_radius_marked_for_load`** — Set camera at chunk `(0, 0, 0)` with `load_radius: 2`. Assert that all chunks within distance 2 (a sphere of ~33 chunks) are added to the load queue.
- **`test_chunks_beyond_unload_radius_marked_for_unload`** — Load chunks at distances 1 through 12. Set `unload_radius: 10`. Run the unload scan and assert chunks at distances 11 and 12 are marked for unloading, while chunks at distances 1-10 are retained.
- **`test_hysteresis_prevents_thrashing`** — Load a chunk at distance 8 (`load_radius: 8`, `unload_radius: 10`). Move camera so the chunk is at distance 9. Run the system and assert the chunk is still loaded (within hysteresis band). Move camera so the chunk is at distance 11. Run the system and assert the chunk is marked for unloading.
- **`test_priority_queue_orders_by_distance`** — Enqueue chunks at distances 5, 2, 8, 1, 3. Dequeue all and assert they come out in order: 1, 2, 3, 5, 8.
- **`test_budget_limits_loads_per_frame`** — Set `loads_per_tick: 3`. Enqueue 10 chunks. Run one tick and assert exactly 3 chunks were loaded. Run another tick and assert 3 more were loaded (6 total).
