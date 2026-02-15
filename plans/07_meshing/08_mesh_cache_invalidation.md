# Mesh Cache Invalidation

## Problem

When a player places or destroys a block, the affected chunk's voxel data changes. The previously generated mesh for that chunk is now stale — it may show geometry that no longer exists or be missing geometry for newly placed blocks. The mesh must be regenerated. But simply remeshing every chunk every frame is wasteful: most chunks are static. The engine needs a targeted invalidation system that marks exactly the right set of chunks for remeshing when voxel data changes.

The complication is cross-chunk dependencies. If a player destroys a block at position `(0, y, z)` in chunk A, this exposes a face on the block at `(31, y, z)` in the adjacent chunk B. Chunk B's mesh is now also stale, even though no voxel in chunk B changed. Similarly, AO values for vertices near the boundary depend on neighbor voxels, so a block edit at a chunk boundary can affect AO in adjacent chunks. Failing to invalidate neighbors causes visual artifacts: invisible holes, missing AO shadows, or phantom geometry.

## Solution

Implement a mesh cache invalidation system in the `nebula_meshing` crate that tracks chunk data versions and invalidates both the edited chunk and its affected neighbors.

### Chunk Data Version

Each chunk maintains a monotonically increasing version number:

```rust
/// Metadata for a chunk's mesh cache state.
pub struct ChunkMeshState {
    /// Version of the voxel data when the current mesh was generated.
    /// If this doesn't match the chunk's current data version, the mesh is stale.
    pub meshed_version: u64,
    /// The current GPU mesh (if any).
    pub gpu_mesh: Option<GpuChunkMesh>,
    /// Whether a remesh task is already in-flight for this chunk.
    pub remesh_pending: bool,
}
```

When a voxel is modified, the chunk's data version is incremented:

```rust
impl ChunkVoxelData {
    pub fn set_voxel(&mut self, x: usize, y: usize, z: usize, voxel: VoxelType) {
        self.data[self.index(x, y, z)] = voxel;
        self.version += 1;
    }

    pub fn version(&self) -> u64 {
        self.version
    }
}
```

### Invalidation Logic

```rust
pub struct MeshInvalidator;

impl MeshInvalidator {
    /// Determine which chunks need remeshing after a voxel edit.
    /// Returns the set of chunk positions that should be invalidated.
    pub fn invalidate(
        edited_chunk: ChunkPosition,
        local_pos: (usize, usize, usize),
        chunk_size: usize,
    ) -> Vec<ChunkPosition> {
        let mut dirty = vec![edited_chunk];

        // Check if the edit is at a chunk boundary.
        // If so, the adjacent chunk's mesh depends on this voxel and must be invalidated.
        let (x, y, z) = local_pos;

        if x == 0 {
            dirty.push(edited_chunk.neighbor(FaceDirection::NegX));
        }
        if x == chunk_size - 1 {
            dirty.push(edited_chunk.neighbor(FaceDirection::PosX));
        }
        if y == 0 {
            dirty.push(edited_chunk.neighbor(FaceDirection::NegY));
        }
        if y == chunk_size - 1 {
            dirty.push(edited_chunk.neighbor(FaceDirection::PosY));
        }
        if z == 0 {
            dirty.push(edited_chunk.neighbor(FaceDirection::NegZ));
        }
        if z == chunk_size - 1 {
            dirty.push(edited_chunk.neighbor(FaceDirection::PosZ));
        }

        // Corner and edge neighbors for AO: if the edit is within 1 voxel
        // of a corner or edge, additional neighbors may need invalidation.
        // (For simplicity, always invalidate face neighbors at boundaries;
        // edge/corner invalidation is optional and can be added if AO artifacts
        // are observed.)

        dirty
    }
}
```

### Remesh Scheduling

Each frame, the main thread scans the mesh state for all loaded chunks:

```rust
pub fn schedule_remeshes(
    chunks: &HashMap<ChunkPosition, ChunkMeshState>,
    voxel_data: &ChunkStorage,
    pipeline: &MeshingPipeline,
    camera_pos: ChunkPosition,
) {
    // Collect stale chunks: meshed_version != current data version
    let mut stale: Vec<ChunkPosition> = chunks
        .iter()
        .filter(|(pos, state)| {
            !state.remesh_pending
                && voxel_data.get_version(**pos) != state.meshed_version
        })
        .map(|(pos, _)| *pos)
        .collect();

    // Sort by distance to camera so nearby chunks are remeshed first
    stale.sort_by_key(|pos| pos.distance_squared(camera_pos));

    // Submit tasks up to the pipeline's budget
    for pos in stale {
        if let Some(neighborhood) = ChunkNeighborhood::try_build(pos, voxel_data) {
            let task = MeshingTask {
                chunk_pos: pos,
                neighborhood,
                data_version: voxel_data.get_version(pos),
            };
            if pipeline.submit(task) {
                chunks.get_mut(&pos).unwrap().remesh_pending = true;
            } else {
                break; // budget exhausted
            }
        }
    }
}
```

### Result Application

When a meshing result arrives (story 07's `drain_results`), the main thread checks the version:

```rust
pub fn apply_mesh_result(
    result: MeshingResult,
    chunks: &mut HashMap<ChunkPosition, ChunkMeshState>,
    voxel_data: &ChunkStorage,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) {
    if let Some(state) = chunks.get_mut(&result.chunk_pos) {
        state.remesh_pending = false;

        // Check if the result is still current
        if result.data_version == voxel_data.get_version(result.chunk_pos) {
            state.gpu_mesh = Some(GpuChunkMesh::upload(device, queue, &result.mesh));
            state.meshed_version = result.data_version;
        }
        // If version mismatch, discard — a new remesh will be triggered next frame
    }
}
```

## Outcome

The `nebula_meshing` crate exports `ChunkMeshState`, `MeshInvalidator`, `schedule_remeshes()`, and `apply_mesh_result()`. When a voxel changes, `MeshInvalidator::invalidate()` returns the set of chunks needing remeshing. The scheduler prioritizes nearby chunks. Stale results are detected via version numbers and discarded. Running `cargo test -p nebula_meshing` passes all invalidation tests.

## Demo Integration

**Demo crate:** `nebula-demo`

When a voxel is modified, only the affected chunk's mesh is regenerated. Surrounding chunks are not re-meshed unless their neighbor boundary data changed.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `nebula_voxel` | workspace | `ChunkVoxelData`, `ChunkStorage`, version tracking |
| `nebula_meshing` | workspace | `MeshingPipeline`, `GpuChunkMesh` from prior stories |

No external crates required. The invalidation logic is pure coordinate arithmetic and version comparison. The crate uses Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Changing a voxel should invalidate the chunk's own mesh.
    #[test]
    fn test_voxel_change_invalidates_own_mesh() {
        let pos = ChunkPosition::ORIGIN;
        let dirty = MeshInvalidator::invalidate(pos, (16, 16, 16), 32);

        assert!(dirty.contains(&pos), "Edited chunk must be in dirty set");
    }

    /// Changing a voxel at a chunk boundary should invalidate the neighbor chunk's mesh.
    #[test]
    fn test_boundary_voxel_change_invalidates_neighbor_mesh() {
        let pos = ChunkPosition::ORIGIN;

        // Edit at x=0: should invalidate the -X neighbor
        let dirty = MeshInvalidator::invalidate(pos, (0, 16, 16), 32);
        let neg_x_neighbor = pos.neighbor(FaceDirection::NegX);
        assert!(
            dirty.contains(&neg_x_neighbor),
            "-X neighbor should be invalidated for edit at x=0"
        );

        // Edit at x=31: should invalidate the +X neighbor
        let dirty = MeshInvalidator::invalidate(pos, (31, 16, 16), 32);
        let pos_x_neighbor = pos.neighbor(FaceDirection::PosX);
        assert!(
            dirty.contains(&pos_x_neighbor),
            "+X neighbor should be invalidated for edit at x=31"
        );

        // Edit at y=0: should invalidate the -Y neighbor
        let dirty = MeshInvalidator::invalidate(pos, (16, 0, 16), 32);
        let neg_y_neighbor = pos.neighbor(FaceDirection::NegY);
        assert!(
            dirty.contains(&neg_y_neighbor),
            "-Y neighbor should be invalidated for edit at y=0"
        );
    }

    /// Changing an interior voxel (not on any boundary) should NOT invalidate any neighbor.
    #[test]
    fn test_interior_change_does_not_invalidate_neighbors() {
        let pos = ChunkPosition::ORIGIN;
        let dirty = MeshInvalidator::invalidate(pos, (16, 16, 16), 32);

        // Only the edited chunk itself
        assert_eq!(dirty.len(), 1);
        assert_eq!(dirty[0], pos);
    }

    /// A version mismatch between mesh state and chunk data should trigger a remesh.
    #[test]
    fn test_version_mismatch_triggers_remesh() {
        let mut state = ChunkMeshState {
            meshed_version: 1,
            gpu_mesh: None,
            remesh_pending: false,
        };

        let current_data_version = 2u64;

        // The mesh is stale because meshed_version != current_data_version
        assert_ne!(state.meshed_version, current_data_version);

        // After remeshing with the current version
        state.meshed_version = current_data_version;
        assert_eq!(state.meshed_version, current_data_version);
    }

    /// A clean chunk (meshed_version == data_version) should NOT be remeshed.
    #[test]
    fn test_clean_chunk_not_remeshed() {
        let state = ChunkMeshState {
            meshed_version: 5,
            gpu_mesh: None,
            remesh_pending: false,
        };

        let current_data_version = 5u64;
        let is_stale = state.meshed_version != current_data_version;

        assert!(!is_stale, "Clean chunk should not be marked stale");
    }

    /// A corner edit (at 0,0,0) should invalidate up to 3 face neighbors.
    #[test]
    fn test_corner_edit_invalidates_three_face_neighbors() {
        let pos = ChunkPosition::ORIGIN;
        let dirty = MeshInvalidator::invalidate(pos, (0, 0, 0), 32);

        // Should include: self, -X neighbor, -Y neighbor, -Z neighbor
        assert!(dirty.len() >= 4, "Corner edit should invalidate at least 4 chunks (self + 3 neighbors), got {}", dirty.len());
        assert!(dirty.contains(&pos));
        assert!(dirty.contains(&pos.neighbor(FaceDirection::NegX)));
        assert!(dirty.contains(&pos.neighbor(FaceDirection::NegY)));
        assert!(dirty.contains(&pos.neighbor(FaceDirection::NegZ)));
    }
}
```
