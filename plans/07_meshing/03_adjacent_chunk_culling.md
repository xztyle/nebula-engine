# Adjacent Chunk Culling

## Problem

Voxel meshing operates on individual chunks, but voxels at chunk boundaries have neighbors that live in adjacent chunks. A stone voxel at position `(31, y, z)` in one chunk has its +X neighbor at position `(0, y, z)` in the neighboring chunk. Without cross-chunk access, the mesher must either conservatively assume all boundary neighbors are air (producing false visible faces that waste geometry) or assume they are solid (producing holes in the terrain). Neither is acceptable.

Beyond face culling, ambient occlusion (story 04) needs to inspect the 3 corner-adjacent voxels for each vertex. Some of these voxels may lie in edge-adjacent chunks (12 neighbors sharing an edge with the current chunk) or even corner-adjacent chunks (8 neighbors sharing only a corner). A complete neighborhood for AO requires access to voxels in up to 26 surrounding chunks.

Repeatedly querying the world's chunk storage during meshing is expensive due to lock contention and cache misses. The mesher needs a self-contained snapshot of the local neighborhood that can be passed to a worker thread without holding any locks.

## Solution

Define a `ChunkNeighborhood` type in the `nebula_meshing` crate that aggregates voxel access for a central chunk and its surrounding neighbors into a single, self-contained data structure.

### Data Structure

```rust
/// Provides voxel access beyond the boundaries of a single chunk.
/// Contains the central chunk's data plus cached slices of up to 26
/// neighboring chunks.
pub struct ChunkNeighborhood {
    /// The central chunk's full voxel data.
    center: ChunkVoxelData,
    /// Face-adjacent chunks (6). Only the boundary slice facing the
    /// center chunk is stored, not the full chunk.
    face_neighbors: [Option<ChunkBoundarySlice>; 6],
    /// Edge-adjacent chunks (12). Only the boundary edge column is stored.
    edge_neighbors: [Option<ChunkBoundaryEdge>; 12],
    /// Corner-adjacent chunks (8). Only the single corner voxel is stored.
    corner_neighbors: [Option<VoxelType>; 8],
    /// Chunk size (typically 32).
    size: usize,
}
```

### Boundary Data

Rather than copying entire adjacent chunks (32x32x32 = 32K voxels each), only the relevant boundary data is extracted:

- **Face neighbor**: A 2D slice of `size x size` voxels from the face touching the center chunk. For the +X face neighbor, this is the slice at `x=0` of the neighbor (which borders `x=31` of the center). Cost: 1,024 voxels per face = 6,144 total.
- **Edge neighbor**: A 1D column of `size` voxels along the shared edge. Cost: 32 voxels per edge = 384 total.
- **Corner neighbor**: A single voxel at the shared corner. Cost: 8 voxels total.

Total neighbor data: ~6,536 voxels, versus 32,768 per full chunk copy. This is 20% of a single chunk rather than 26x a full chunk.

### Voxel Lookup

```rust
impl ChunkNeighborhood {
    /// Get the voxel at a position relative to the center chunk.
    /// Coordinates may be in the range [-1, size] to access one voxel
    /// beyond the chunk boundary in any direction.
    pub fn get(&self, x: i32, y: i32, z: i32) -> VoxelType {
        if x >= 0 && x < self.size as i32
            && y >= 0 && y < self.size as i32
            && z >= 0 && z < self.size as i32
        {
            return self.center.get(x as usize, y as usize, z as usize);
        }

        // Determine which neighbor region this falls in and look up
        self.lookup_neighbor(x, y, z)
            .unwrap_or(VoxelType::AIR) // missing neighbor = air
    }
}
```

The `lookup_neighbor` method classifies the out-of-bounds coordinate into one of 26 neighbor categories based on how many axes are out of range: 1 axis out = face neighbor (6), 2 axes out = edge neighbor (12), 3 axes out = corner neighbor (8).

### Construction

```rust
impl ChunkNeighborhood {
    /// Build a neighborhood from the world's chunk storage.
    /// This copies the necessary boundary data and produces an owned,
    /// self-contained snapshot.
    pub fn build(
        center_pos: ChunkPosition,
        world: &ChunkStorage,
    ) -> Self {
        let center = world.get_chunk_data(center_pos).clone();
        let size = center.size();

        let mut neighborhood = Self {
            center,
            face_neighbors: Default::default(),
            edge_neighbors: Default::default(),
            corner_neighbors: Default::default(),
            size,
        };

        // Extract face neighbor boundary slices
        for (i, offset) in FACE_OFFSETS.iter().enumerate() {
            if let Some(neighbor_chunk) = world.get_chunk_data(center_pos + *offset) {
                neighborhood.face_neighbors[i] =
                    Some(extract_boundary_slice(neighbor_chunk, FACE_DIRECTIONS[i].opposite(), size));
            }
        }

        // Extract edge and corner data similarly...
        neighborhood
    }
}
```

### Caching

Once built, a `ChunkNeighborhood` is an owned value with no references into the world. It can be sent to a worker thread for meshing. The world can continue mutating while meshing runs. If a neighbor chunk changes after the snapshot was taken, the resulting mesh may be slightly stale — this is resolved by the cache invalidation system (story 08) which re-meshes affected chunks.

## Outcome

The `nebula_meshing` crate exports `ChunkNeighborhood` with `build()` and `get()`. Meshing functions receive a `ChunkNeighborhood` instead of a raw chunk reference, giving them seamless access to boundary voxels without runtime chunk lookups. The neighborhood is a self-contained, owned snapshot suitable for sending to worker threads. Running `cargo test -p nebula_meshing` passes all neighborhood tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Faces at chunk boundaries are correctly culled when the neighboring chunk's voxel is solid. No seams or z-fighting visible at chunk borders.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `nebula_voxel` | workspace | `ChunkVoxelData`, `VoxelType`, chunk storage access |
| `nebula_cubesphere` | workspace | `ChunkPosition` and chunk coordinate arithmetic |

No external crates required. The neighborhood is built from array copies and coordinate arithmetic. The crate uses Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn make_chunk(size: usize, fill: VoxelType) -> ChunkVoxelData {
        ChunkVoxelData::new_filled(size, fill)
    }

    /// An interior voxel (well within chunk bounds) should be served directly
    /// from the center chunk without touching neighbor data.
    #[test]
    fn test_interior_voxel_does_not_need_neighbors() {
        let center = make_chunk(32, VoxelType::STONE);
        let neighborhood = ChunkNeighborhood::from_center_only(center);

        let voxel = neighborhood.get(16, 16, 16);
        assert_eq!(voxel, VoxelType::STONE);
    }

    /// A boundary voxel query (x = -1) should return the correct voxel
    /// from the -X face neighbor's boundary slice.
    #[test]
    fn test_boundary_voxel_queries_correct_neighbor() {
        let center = make_chunk(32, VoxelType::AIR);
        let mut neg_x_neighbor = make_chunk(32, VoxelType::AIR);
        neg_x_neighbor.set(31, 10, 10, VoxelType::STONE);

        let mut neighborhood = ChunkNeighborhood::from_center_only(center);
        neighborhood.set_face_neighbor(FaceDirection::NegX, &neg_x_neighbor);

        // x=-1 in center coords maps to x=31 in the -X neighbor
        let voxel = neighborhood.get(-1, 10, 10);
        assert_eq!(voxel, VoxelType::STONE);
    }

    /// When a face neighbor is not loaded (None), boundary queries should
    /// return AIR as the safe default.
    #[test]
    fn test_missing_neighbor_treats_boundary_as_air() {
        let center = make_chunk(32, VoxelType::STONE);
        let neighborhood = ChunkNeighborhood::from_center_only(center);

        // Query beyond +X boundary with no neighbor loaded
        let voxel = neighborhood.get(32, 10, 10);
        assert_eq!(voxel, VoxelType::AIR);
    }

    /// The neighborhood should correctly handle all 26 neighbor directions:
    /// 6 face, 12 edge, 8 corner.
    #[test]
    fn test_neighborhood_covers_all_26_directions() {
        let center = make_chunk(32, VoxelType::AIR);
        let mut neighborhood = ChunkNeighborhood::from_center_only(center);

        // Set up all 26 neighbors with stone
        let stone_chunk = make_chunk(32, VoxelType::STONE);
        for dir in FaceDirection::ALL {
            neighborhood.set_face_neighbor(dir, &stone_chunk);
        }
        for edge in EdgeDirection::ALL {
            neighborhood.set_edge_neighbor(edge, &stone_chunk);
        }
        for corner in CornerDirection::ALL {
            neighborhood.set_corner_neighbor(corner, VoxelType::STONE);
        }

        // Face neighbors: 6 positions just outside each face
        assert_eq!(neighborhood.get(32, 16, 16), VoxelType::STONE);  // +X
        assert_eq!(neighborhood.get(-1, 16, 16), VoxelType::STONE);  // -X
        assert_eq!(neighborhood.get(16, 32, 16), VoxelType::STONE);  // +Y
        assert_eq!(neighborhood.get(16, -1, 16), VoxelType::STONE);  // -Y
        assert_eq!(neighborhood.get(16, 16, 32), VoxelType::STONE);  // +Z
        assert_eq!(neighborhood.get(16, 16, -1), VoxelType::STONE);  // -Z

        // Corner neighbors: all 8 corners
        assert_eq!(neighborhood.get(-1, -1, -1), VoxelType::STONE);
        assert_eq!(neighborhood.get(32, -1, -1), VoxelType::STONE);
        assert_eq!(neighborhood.get(-1, 32, -1), VoxelType::STONE);
        assert_eq!(neighborhood.get(32, 32, -1), VoxelType::STONE);
        assert_eq!(neighborhood.get(-1, -1, 32), VoxelType::STONE);
        assert_eq!(neighborhood.get(32, -1, 32), VoxelType::STONE);
        assert_eq!(neighborhood.get(-1, 32, 32), VoxelType::STONE);
        assert_eq!(neighborhood.get(32, 32, 32), VoxelType::STONE);
    }

    /// Verify that the boundary slice extraction only copies the relevant
    /// face of the neighbor, not the entire chunk.
    #[test]
    fn test_boundary_slice_is_minimal() {
        let neighbor = make_chunk(32, VoxelType::STONE);
        let slice = extract_boundary_slice(&neighbor, FaceDirection::PosX, 32);

        // The slice should contain exactly 32*32 = 1024 voxels
        assert_eq!(slice.len(), 32 * 32);
    }
}
```
