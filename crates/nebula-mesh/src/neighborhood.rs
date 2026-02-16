//! Cross-chunk neighbor access for face, edge, and corner voxel lookups.
//!
//! [`ChunkNeighborhood`] aggregates a center chunk and boundary data from up
//! to 26 surrounding chunks into a self-contained, owned snapshot suitable
//! for sending to worker threads without holding any locks.

use nebula_voxel::{CHUNK_SIZE, ChunkData, VoxelTypeId};

use crate::face_direction::{CornerDirection, EdgeDirection, FaceDirection};

// ---------------------------------------------------------------------------
// Boundary data types
// ---------------------------------------------------------------------------

/// A 2D slice of `CHUNK_SIZE × CHUNK_SIZE` voxels from a face neighbor.
///
/// Only the single layer touching the center chunk is stored, not the full
/// 32³ chunk. Cost: 1,024 voxels per face.
#[derive(Clone, Debug)]
pub struct ChunkBoundarySlice {
    /// Voxels stored in row-major order (u varies fastest).
    data: Vec<VoxelTypeId>,
    /// Side length (typically `CHUNK_SIZE`).
    size: usize,
}

impl ChunkBoundarySlice {
    /// Returns the number of voxels in the slice.
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Returns `true` if the slice contains no voxels.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Looks up a voxel by 2D coordinates within the slice.
    pub fn get(&self, u: usize, v: usize) -> VoxelTypeId {
        self.data[v * self.size + u]
    }
}

/// A 1D column of `CHUNK_SIZE` voxels from an edge neighbor.
///
/// Only the single column along the shared edge is stored. Cost: 32 voxels
/// per edge.
#[derive(Clone, Debug)]
pub struct ChunkBoundaryEdge {
    /// Voxels along the shared edge.
    data: Vec<VoxelTypeId>,
}

impl ChunkBoundaryEdge {
    /// Returns the number of voxels in the edge.
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Returns `true` if the edge contains no voxels.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Looks up a voxel by index along the edge.
    pub fn get(&self, index: usize) -> VoxelTypeId {
        self.data[index]
    }
}

// ---------------------------------------------------------------------------
// Extraction helpers
// ---------------------------------------------------------------------------

/// Extracts a 2D boundary slice from a chunk for the given face direction.
///
/// `direction` indicates which face of `chunk` is the boundary. For example,
/// `FaceDirection::PosX` extracts the slice at `x = CHUNK_SIZE - 1`.
pub fn extract_boundary_slice(
    chunk: &ChunkData,
    direction: FaceDirection,
    size: usize,
) -> ChunkBoundarySlice {
    let mut data = Vec::with_capacity(size * size);

    match direction {
        FaceDirection::PosX => {
            for v in 0..size {
                for u in 0..size {
                    data.push(chunk.get(size - 1, u, v));
                }
            }
        }
        FaceDirection::NegX => {
            for v in 0..size {
                for u in 0..size {
                    data.push(chunk.get(0, u, v));
                }
            }
        }
        FaceDirection::PosY => {
            for v in 0..size {
                for u in 0..size {
                    data.push(chunk.get(u, size - 1, v));
                }
            }
        }
        FaceDirection::NegY => {
            for v in 0..size {
                for u in 0..size {
                    data.push(chunk.get(u, 0, v));
                }
            }
        }
        FaceDirection::PosZ => {
            for v in 0..size {
                for u in 0..size {
                    data.push(chunk.get(u, v, size - 1));
                }
            }
        }
        FaceDirection::NegZ => {
            for v in 0..size {
                for u in 0..size {
                    data.push(chunk.get(u, v, 0));
                }
            }
        }
    }

    ChunkBoundarySlice { data, size }
}

/// Extracts a 1D boundary edge from a chunk for the given edge direction.
///
/// The edge is the column of voxels at the intersection of two faces.
fn extract_boundary_edge(chunk: &ChunkData, edge: EdgeDirection, size: usize) -> ChunkBoundaryEdge {
    let s = size - 1;
    let data: Vec<VoxelTypeId> = (0..size)
        .map(|i| match edge {
            EdgeDirection::PosXPosY => chunk.get(s, s, i),
            EdgeDirection::PosXNegY => chunk.get(s, 0, i),
            EdgeDirection::PosXPosZ => chunk.get(s, i, s),
            EdgeDirection::PosXNegZ => chunk.get(s, i, 0),
            EdgeDirection::NegXPosY => chunk.get(0, s, i),
            EdgeDirection::NegXNegY => chunk.get(0, 0, i),
            EdgeDirection::NegXPosZ => chunk.get(0, i, s),
            EdgeDirection::NegXNegZ => chunk.get(0, i, 0),
            EdgeDirection::PosYPosZ => chunk.get(i, s, s),
            EdgeDirection::PosYNegZ => chunk.get(i, s, 0),
            EdgeDirection::NegYPosZ => chunk.get(i, 0, s),
            EdgeDirection::NegYNegZ => chunk.get(i, 0, 0),
        })
        .collect();
    ChunkBoundaryEdge { data }
}

/// Extracts a single corner voxel from a chunk.
fn extract_corner_voxel(chunk: &ChunkData, corner: CornerDirection, size: usize) -> VoxelTypeId {
    let s = size - 1;
    match corner {
        CornerDirection::NegXNegYNegZ => chunk.get(0, 0, 0),
        CornerDirection::PosXNegYNegZ => chunk.get(s, 0, 0),
        CornerDirection::NegXPosYNegZ => chunk.get(0, s, 0),
        CornerDirection::PosXPosYNegZ => chunk.get(s, s, 0),
        CornerDirection::NegXNegYPosZ => chunk.get(0, 0, s),
        CornerDirection::PosXNegYPosZ => chunk.get(s, 0, s),
        CornerDirection::NegXPosYPosZ => chunk.get(0, s, s),
        CornerDirection::PosXPosYPosZ => chunk.get(s, s, s),
    }
}

// ---------------------------------------------------------------------------
// ChunkNeighborhood
// ---------------------------------------------------------------------------

/// Provides voxel access beyond the boundaries of a single chunk.
///
/// Contains the central chunk's full voxel data plus cached boundary data
/// from up to 26 neighboring chunks:
/// - 6 face neighbors (2D slice of `size × size` voxels each)
/// - 12 edge neighbors (1D column of `size` voxels each)
/// - 8 corner neighbors (single voxel each)
///
/// Total neighbor data: ~6,536 voxels vs 26 × 32,768 for full copies.
pub struct ChunkNeighborhood {
    /// The central chunk's full voxel data (if available).
    center: Option<ChunkData>,
    /// Face-adjacent boundary slices, indexed by [`FaceDirection`].
    face_neighbors: [Option<ChunkBoundarySlice>; 6],
    /// Edge-adjacent boundary edges, indexed by [`EdgeDirection`].
    edge_neighbors: [Option<ChunkBoundaryEdge>; 12],
    /// Corner-adjacent single voxels, indexed by [`CornerDirection`].
    corner_neighbors: [Option<VoxelTypeId>; 8],
    /// Chunk size (typically 32).
    size: usize,
}

impl ChunkNeighborhood {
    /// Creates a neighborhood with no center chunk and no neighbors (all air).
    pub fn all_air() -> Self {
        Self {
            center: None,
            face_neighbors: Default::default(),
            edge_neighbors: Default::default(),
            corner_neighbors: Default::default(),
            size: CHUNK_SIZE,
        }
    }

    /// Creates a neighborhood from a center chunk with no neighbors loaded.
    pub fn from_center_only(center: ChunkData) -> Self {
        Self {
            center: Some(center),
            face_neighbors: Default::default(),
            edge_neighbors: Default::default(),
            corner_neighbors: Default::default(),
            size: CHUNK_SIZE,
        }
    }

    /// Creates a neighborhood with only the −X face neighbor; all others air.
    pub fn with_neg_x(chunk: ChunkData) -> Self {
        let mut n = Self::all_air();
        let slice = extract_boundary_slice(&chunk, FaceDirection::PosX, CHUNK_SIZE);
        n.face_neighbors[FaceDirection::NegX.index()] = Some(slice);
        n
    }

    /// Sets a face neighbor by direction index (0–5), extracting the boundary
    /// slice from the provided full chunk data.
    ///
    /// This is the legacy API; prefer [`Self::set_face_neighbor`] for clarity.
    pub fn set(&mut self, direction_index: usize, chunk: ChunkData) {
        if direction_index < 6 {
            let dir = FaceDirection::ALL[direction_index];
            self.set_face_neighbor(dir, &chunk);
        }
    }

    /// Sets a face neighbor, extracting only the boundary slice that touches
    /// the center chunk.
    pub fn set_face_neighbor(&mut self, direction: FaceDirection, neighbor_chunk: &ChunkData) {
        // The face of the neighbor chunk that touches us is the opposite face.
        let extract_face = direction.opposite();
        let slice = extract_boundary_slice(neighbor_chunk, extract_face, self.size);
        self.face_neighbors[direction.index()] = Some(slice);
    }

    /// Sets an edge neighbor, extracting only the boundary edge column.
    pub fn set_edge_neighbor(&mut self, edge: EdgeDirection, neighbor_chunk: &ChunkData) {
        // Extract the corner edge of the neighbor that touches us (opposite corner).
        let opposite_edge = opposite_edge(edge);
        let edge_data = extract_boundary_edge(neighbor_chunk, opposite_edge, self.size);
        self.edge_neighbors[edge.index()] = Some(edge_data);
    }

    /// Sets a corner neighbor to a single voxel value.
    pub fn set_corner_neighbor(&mut self, corner: CornerDirection, voxel: VoxelTypeId) {
        self.corner_neighbors[corner.index()] = Some(voxel);
    }

    /// Sets a corner neighbor by extracting the relevant corner voxel from a chunk.
    pub fn set_corner_neighbor_from_chunk(
        &mut self,
        corner: CornerDirection,
        neighbor_chunk: &ChunkData,
    ) {
        let opposite = opposite_corner(corner);
        let voxel = extract_corner_voxel(neighbor_chunk, opposite, self.size);
        self.corner_neighbors[corner.index()] = Some(voxel);
    }

    /// Gets a voxel at coordinates relative to the center chunk.
    ///
    /// Coordinates in `[0, size)` are served from the center chunk.
    /// Coordinates in `[-1, size]` (one voxel beyond each boundary) are
    /// served from the appropriate neighbor. Missing neighbors return air.
    pub fn get(&self, x: i32, y: i32, z: i32) -> VoxelTypeId {
        let s = self.size as i32;

        let x_out = if x < 0 {
            -1i8
        } else if x >= s {
            1
        } else {
            0
        };
        let y_out = if y < 0 {
            -1i8
        } else if y >= s {
            1
        } else {
            0
        };
        let z_out = if z < 0 {
            -1i8
        } else if z >= s {
            1
        } else {
            0
        };

        let out_count = (x_out != 0) as u8 + (y_out != 0) as u8 + (z_out != 0) as u8;

        match out_count {
            0 => self.get_center(x as usize, y as usize, z as usize),
            1 => self.lookup_face(x, y, z, x_out, y_out, z_out),
            2 => self.lookup_edge(x_out, y_out, z_out, x, y, z),
            3 => self.lookup_corner(x_out, y_out, z_out),
            _ => VoxelTypeId(0),
        }
    }

    /// Returns a reference to the center chunk data, if available.
    pub fn center(&self) -> Option<&ChunkData> {
        self.center.as_ref()
    }

    // -- private helpers --

    fn get_center(&self, x: usize, y: usize, z: usize) -> VoxelTypeId {
        match &self.center {
            Some(c) => c.get(x, y, z),
            None => VoxelTypeId(0),
        }
    }

    fn lookup_face(&self, x: i32, y: i32, z: i32, x_out: i8, y_out: i8, z_out: i8) -> VoxelTypeId {
        let s = self.size as i32;
        let (dir_index, u, v) = if x_out != 0 {
            let dir = if x_out < 0 { 1 } else { 0 }; // NegX=1, PosX=0
            (dir, y as usize, z as usize)
        } else if y_out != 0 {
            let dir = if y_out < 0 { 3 } else { 2 }; // NegY=3, PosY=2
            (dir, x as usize, z as usize)
        } else {
            let dir = if z_out < 0 { 5 } else { 4 }; // NegZ=5, PosZ=4
            (dir, x as usize, y as usize)
        };

        // Clamp the in-bounds coordinates
        let _ = s; // used above for out-of-bounds check
        match &self.face_neighbors[dir_index] {
            Some(slice) => slice.get(u, v),
            None => VoxelTypeId(0),
        }
    }

    fn lookup_edge(&self, x_out: i8, y_out: i8, z_out: i8, x: i32, y: i32, z: i32) -> VoxelTypeId {
        // Determine the in-bounds axis value and which edge
        let (edge_idx, in_val) = match (x_out != 0, y_out != 0, z_out != 0) {
            (true, true, false) => {
                let idx = match (x_out > 0, y_out > 0) {
                    (true, true) => 0,   // PosXPosY
                    (true, false) => 1,  // PosXNegY
                    (false, true) => 4,  // NegXPosY
                    (false, false) => 5, // NegXNegY
                };
                (idx, z as usize)
            }
            (true, false, true) => {
                let idx = match (x_out > 0, z_out > 0) {
                    (true, true) => 2,   // PosXPosZ
                    (true, false) => 3,  // PosXNegZ
                    (false, true) => 6,  // NegXPosZ
                    (false, false) => 7, // NegXNegZ
                };
                (idx, y as usize)
            }
            (false, true, true) => {
                let idx = match (y_out > 0, z_out > 0) {
                    (true, true) => 8,    // PosYPosZ
                    (true, false) => 9,   // PosYNegZ
                    (false, true) => 10,  // NegYPosZ
                    (false, false) => 11, // NegYNegZ
                };
                (idx, x as usize)
            }
            _ => return VoxelTypeId(0),
        };

        match &self.edge_neighbors[edge_idx] {
            Some(edge) => edge.get(in_val),
            None => VoxelTypeId(0),
        }
    }

    fn lookup_corner(&self, x_out: i8, y_out: i8, z_out: i8) -> VoxelTypeId {
        let idx = match (x_out > 0, y_out > 0, z_out > 0) {
            (false, false, false) => 0, // NegXNegYNegZ
            (true, false, false) => 1,  // PosXNegYNegZ
            (false, true, false) => 2,  // NegXPosYNegZ
            (true, true, false) => 3,   // PosXPosYNegZ
            (false, false, true) => 4,  // NegXNegYPosZ
            (true, false, true) => 5,   // PosXNegYPosZ
            (false, true, true) => 6,   // NegXPosYPosZ
            (true, true, true) => 7,    // PosXPosYPosZ
        };

        self.corner_neighbors[idx].unwrap_or(VoxelTypeId(0))
    }
}

/// Returns the opposite edge direction (the edge of the neighbor chunk that
/// touches the center chunk at the given edge).
fn opposite_edge(edge: EdgeDirection) -> EdgeDirection {
    match edge {
        EdgeDirection::PosXPosY => EdgeDirection::NegXNegY,
        EdgeDirection::PosXNegY => EdgeDirection::NegXPosY,
        EdgeDirection::PosXPosZ => EdgeDirection::NegXNegZ,
        EdgeDirection::PosXNegZ => EdgeDirection::NegXPosZ,
        EdgeDirection::NegXPosY => EdgeDirection::PosXNegY,
        EdgeDirection::NegXNegY => EdgeDirection::PosXPosY,
        EdgeDirection::NegXPosZ => EdgeDirection::PosXNegZ,
        EdgeDirection::NegXNegZ => EdgeDirection::PosXPosZ,
        EdgeDirection::PosYPosZ => EdgeDirection::NegYNegZ,
        EdgeDirection::PosYNegZ => EdgeDirection::NegYPosZ,
        EdgeDirection::NegYPosZ => EdgeDirection::PosYNegZ,
        EdgeDirection::NegYNegZ => EdgeDirection::PosYPosZ,
    }
}

/// Returns the opposite corner direction.
fn opposite_corner(corner: CornerDirection) -> CornerDirection {
    match corner {
        CornerDirection::NegXNegYNegZ => CornerDirection::PosXPosYPosZ,
        CornerDirection::PosXNegYNegZ => CornerDirection::NegXPosYPosZ,
        CornerDirection::NegXPosYNegZ => CornerDirection::PosXNegYPosZ,
        CornerDirection::PosXPosYNegZ => CornerDirection::NegXNegYPosZ,
        CornerDirection::NegXNegYPosZ => CornerDirection::PosXPosYNegZ,
        CornerDirection::PosXNegYPosZ => CornerDirection::NegXPosYNegZ,
        CornerDirection::NegXPosYPosZ => CornerDirection::PosXNegYNegZ,
        CornerDirection::PosXPosYPosZ => CornerDirection::NegXNegYNegZ,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_chunk(fill: VoxelTypeId) -> ChunkData {
        ChunkData::new(fill)
    }

    #[test]
    fn test_interior_voxel_does_not_need_neighbors() {
        let center = make_chunk(VoxelTypeId(1));
        let neighborhood = ChunkNeighborhood::from_center_only(center);
        assert_eq!(neighborhood.get(16, 16, 16), VoxelTypeId(1));
    }

    #[test]
    fn test_boundary_voxel_queries_correct_neighbor() {
        let center = ChunkData::new_air();
        let mut neg_x_neighbor = ChunkData::new_air();
        neg_x_neighbor.set(31, 10, 10, VoxelTypeId(1));

        let mut neighborhood = ChunkNeighborhood::from_center_only(center);
        neighborhood.set_face_neighbor(FaceDirection::NegX, &neg_x_neighbor);

        assert_eq!(neighborhood.get(-1, 10, 10), VoxelTypeId(1));
    }

    #[test]
    fn test_missing_neighbor_treats_boundary_as_air() {
        let center = make_chunk(VoxelTypeId(1));
        let neighborhood = ChunkNeighborhood::from_center_only(center);
        assert_eq!(neighborhood.get(32, 10, 10), VoxelTypeId(0));
    }

    #[test]
    fn test_neighborhood_covers_all_26_directions() {
        let center = ChunkData::new_air();
        let mut neighborhood = ChunkNeighborhood::from_center_only(center);

        let stone_chunk = make_chunk(VoxelTypeId(1));
        for dir in FaceDirection::ALL {
            neighborhood.set_face_neighbor(dir, &stone_chunk);
        }
        for edge in EdgeDirection::ALL {
            neighborhood.set_edge_neighbor(edge, &stone_chunk);
        }
        for corner in CornerDirection::ALL {
            neighborhood.set_corner_neighbor(corner, VoxelTypeId(1));
        }

        // Face neighbors
        assert_eq!(neighborhood.get(32, 16, 16), VoxelTypeId(1)); // +X
        assert_eq!(neighborhood.get(-1, 16, 16), VoxelTypeId(1)); // -X
        assert_eq!(neighborhood.get(16, 32, 16), VoxelTypeId(1)); // +Y
        assert_eq!(neighborhood.get(16, -1, 16), VoxelTypeId(1)); // -Y
        assert_eq!(neighborhood.get(16, 16, 32), VoxelTypeId(1)); // +Z
        assert_eq!(neighborhood.get(16, 16, -1), VoxelTypeId(1)); // -Z

        // Corner neighbors
        assert_eq!(neighborhood.get(-1, -1, -1), VoxelTypeId(1));
        assert_eq!(neighborhood.get(32, -1, -1), VoxelTypeId(1));
        assert_eq!(neighborhood.get(-1, 32, -1), VoxelTypeId(1));
        assert_eq!(neighborhood.get(32, 32, -1), VoxelTypeId(1));
        assert_eq!(neighborhood.get(-1, -1, 32), VoxelTypeId(1));
        assert_eq!(neighborhood.get(32, -1, 32), VoxelTypeId(1));
        assert_eq!(neighborhood.get(-1, 32, 32), VoxelTypeId(1));
        assert_eq!(neighborhood.get(32, 32, 32), VoxelTypeId(1));
    }

    #[test]
    fn test_boundary_slice_is_minimal() {
        let neighbor = make_chunk(VoxelTypeId(1));
        let slice = extract_boundary_slice(&neighbor, FaceDirection::PosX, 32);
        assert_eq!(slice.len(), 32 * 32);
    }

    #[test]
    fn test_all_air_returns_air() {
        let n = ChunkNeighborhood::all_air();
        assert_eq!(n.get(-1, 0, 0), VoxelTypeId(0));
        assert_eq!(n.get(32, 0, 0), VoxelTypeId(0));
        assert_eq!(n.get(0, -1, 0), VoxelTypeId(0));
    }

    #[test]
    fn test_legacy_with_neg_x() {
        let mut chunk = ChunkData::new_air();
        chunk.set(31, 10, 10, VoxelTypeId(5));
        let n = ChunkNeighborhood::with_neg_x(chunk);
        assert_eq!(n.get(-1, 10, 10), VoxelTypeId(5));
        assert_eq!(n.get(-1, 0, 0), VoxelTypeId(0));
    }

    #[test]
    fn test_legacy_set_pos_y_neighbor() {
        let mut n = ChunkNeighborhood::all_air();
        let mut chunk = ChunkData::new_air();
        chunk.set(5, 0, 5, VoxelTypeId(9));
        n.set(2, chunk); // +Y direction index
        assert_eq!(n.get(5, 32, 5), VoxelTypeId(9));
    }

    #[test]
    fn test_edge_neighbor_lookup() {
        let center = ChunkData::new_air();
        let mut neighborhood = ChunkNeighborhood::from_center_only(center);

        // Create an edge neighbor chunk with a specific voxel
        let mut edge_chunk = ChunkData::new_air();
        // PosXPosY edge: opposite is NegXNegY, which extracts at (0, 0, i)
        edge_chunk.set(0, 0, 15, VoxelTypeId(7));
        neighborhood.set_edge_neighbor(EdgeDirection::PosXPosY, &edge_chunk);

        assert_eq!(neighborhood.get(32, 32, 15), VoxelTypeId(7));
        assert_eq!(neighborhood.get(32, 32, 0), VoxelTypeId(0));
    }

    #[test]
    fn test_corner_neighbor_from_chunk() {
        let center = ChunkData::new_air();
        let mut neighborhood = ChunkNeighborhood::from_center_only(center);

        let mut corner_chunk = ChunkData::new_air();
        // PosXPosYPosZ corner: opposite is NegXNegYNegZ, extracts (0,0,0)
        corner_chunk.set(0, 0, 0, VoxelTypeId(42));
        neighborhood.set_corner_neighbor_from_chunk(CornerDirection::PosXPosYPosZ, &corner_chunk);

        assert_eq!(neighborhood.get(32, 32, 32), VoxelTypeId(42));
    }

    #[test]
    fn test_boundary_edge_is_minimal() {
        let chunk = make_chunk(VoxelTypeId(1));
        let edge = extract_boundary_edge(&chunk, EdgeDirection::PosXPosY, 32);
        assert_eq!(edge.len(), 32);
    }
}
