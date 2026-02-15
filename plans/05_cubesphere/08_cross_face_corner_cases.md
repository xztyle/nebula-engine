# Cross-Face Corner Cases

## Problem

A cube has 8 corners where 3 faces meet. A chunk located at a corner of a cube face needs diagonal neighbors that belong to two other faces — neither of which shares an edge with the chunk in the relevant direction. This is more complex than cross-edge neighbor finding (story 07) because the shared point is zero-dimensional (a single vertex), not one-dimensional (an edge). Additionally, when chunks at different LODs meet at a corner, T-junctions can form — two edges meet one edge at a non-vertex point, causing cracks in the mesh unless explicitly handled. The engine must define a corner adjacency table, ensure consistent vertex placement at corners, and handle LOD transitions that occur at these triple-face meeting points.

## Solution

Implement cross-face corner handling in the `nebula_cubesphere` crate.

### Cube Corner Definition

```rust
/// A corner of the cube where 3 faces meet.
///
/// Each corner is identified by the sign of its position on each axis.
/// For example, `PosXPosYPosZ` is the corner at (+1, +1, +1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CubeCorner {
    PosXPosYPosZ,
    PosXPosYNegZ,
    PosXNegYPosZ,
    PosXNegYNegZ,
    NegXPosYPosZ,
    NegXPosYNegZ,
    NegXNegYPosZ,
    NegXNegYNegZ,
}

impl CubeCorner {
    pub const ALL: [CubeCorner; 8] = [
        CubeCorner::PosXPosYPosZ,
        CubeCorner::PosXPosYNegZ,
        CubeCorner::PosXNegYPosZ,
        CubeCorner::PosXNegYNegZ,
        CubeCorner::NegXPosYPosZ,
        CubeCorner::NegXPosYNegZ,
        CubeCorner::NegXNegYPosZ,
        CubeCorner::NegXNegYNegZ,
    ];

    /// The 3 faces that meet at this corner.
    pub fn faces(self) -> [CubeFace; 3] {
        match self {
            CubeCorner::PosXPosYPosZ => [CubeFace::PosX, CubeFace::PosY, CubeFace::PosZ],
            CubeCorner::PosXPosYNegZ => [CubeFace::PosX, CubeFace::PosY, CubeFace::NegZ],
            CubeCorner::PosXNegYPosZ => [CubeFace::PosX, CubeFace::NegY, CubeFace::PosZ],
            CubeCorner::PosXNegYNegZ => [CubeFace::PosX, CubeFace::NegY, CubeFace::NegZ],
            CubeCorner::NegXPosYPosZ => [CubeFace::NegX, CubeFace::PosY, CubeFace::PosZ],
            CubeCorner::NegXPosYNegZ => [CubeFace::NegX, CubeFace::PosY, CubeFace::NegZ],
            CubeCorner::NegXNegYPosZ => [CubeFace::NegX, CubeFace::NegY, CubeFace::PosZ],
            CubeCorner::NegXNegYNegZ => [CubeFace::NegX, CubeFace::NegY, CubeFace::NegZ],
        }
    }

    /// The 3D position of this corner on the unit cube (each component is +/-1).
    pub fn position(self) -> glam::DVec3 {
        match self {
            CubeCorner::PosXPosYPosZ => glam::DVec3::new( 1.0,  1.0,  1.0),
            CubeCorner::PosXPosYNegZ => glam::DVec3::new( 1.0,  1.0, -1.0),
            CubeCorner::PosXNegYPosZ => glam::DVec3::new( 1.0, -1.0,  1.0),
            CubeCorner::PosXNegYNegZ => glam::DVec3::new( 1.0, -1.0, -1.0),
            CubeCorner::NegXPosYPosZ => glam::DVec3::new(-1.0,  1.0,  1.0),
            CubeCorner::NegXPosYNegZ => glam::DVec3::new(-1.0,  1.0, -1.0),
            CubeCorner::NegXNegYPosZ => glam::DVec3::new(-1.0, -1.0,  1.0),
            CubeCorner::NegXNegYNegZ => glam::DVec3::new(-1.0, -1.0, -1.0),
        }
    }
}
```

### Corner of a Chunk

Determine which cube corner (if any) a chunk sits at:

```rust
/// Which UV corner of the face a chunk occupies, if any.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaceCorner {
    /// (u=0, v=0) — bottom-left
    BottomLeft,
    /// (u=1, v=0) — bottom-right
    BottomRight,
    /// (u=0, v=1) — top-left
    TopLeft,
    /// (u=1, v=1) — top-right
    TopRight,
}

impl ChunkAddress {
    /// If this chunk is at a corner of its face, return the face corner
    /// and the corresponding cube corner. Returns None if not at any corner.
    pub fn cube_corner(&self) -> Option<(FaceCorner, CubeCorner)> {
        let grid = Self::grid_size(self.lod);
        let at_left = self.x == 0;
        let at_right = self.x == grid - 1;
        let at_bottom = self.y == 0;
        let at_top = self.y == grid - 1;

        let face_corner = match (at_left || at_right, at_bottom || at_top) {
            (true, true) => {
                match (at_left, at_bottom) {
                    (true, true) => FaceCorner::BottomLeft,
                    (false, true) => FaceCorner::BottomRight,
                    (true, false) => FaceCorner::TopLeft,
                    (false, false) => FaceCorner::TopRight,
                }
            }
            _ => return None,
        };

        let cube_corner = face_corner_to_cube_corner(self.face, face_corner);
        Some((face_corner, cube_corner))
    }
}

/// Map a face + face corner to the corresponding cube corner.
fn face_corner_to_cube_corner(face: CubeFace, corner: FaceCorner) -> CubeCorner {
    // The mapping depends on the tangent/bitangent convention from story 01.
    // For each face, the 4 UV corners map to specific cube corners.
    // This function encodes that mapping via the basis vectors.
    let t = face.tangent();
    let b = face.bitangent();
    let n = face.normal();

    let u_sign = match corner {
        FaceCorner::BottomLeft | FaceCorner::TopLeft => -1.0,
        FaceCorner::BottomRight | FaceCorner::TopRight => 1.0,
    };
    let v_sign = match corner {
        FaceCorner::BottomLeft | FaceCorner::BottomRight => -1.0,
        FaceCorner::TopLeft | FaceCorner::TopRight => 1.0,
    };

    let pos = n + u_sign * t + v_sign * b;

    // Determine cube corner from the signs of the components
    match (pos.x >= 0.0, pos.y >= 0.0, pos.z >= 0.0) {
        (true, true, true) => CubeCorner::PosXPosYPosZ,
        (true, true, false) => CubeCorner::PosXPosYNegZ,
        (true, false, true) => CubeCorner::PosXNegYPosZ,
        (true, false, false) => CubeCorner::PosXNegYNegZ,
        (false, true, true) => CubeCorner::NegXPosYPosZ,
        (false, true, false) => CubeCorner::NegXPosYNegZ,
        (false, false, true) => CubeCorner::NegXNegYPosZ,
        (false, false, false) => CubeCorner::NegXNegYNegZ,
    }
}
```

### Diagonal Corner Neighbors

```rust
/// Result of a corner neighbor query: the chunks on the other 2 faces
/// that share this cube corner.
#[derive(Clone, Debug)]
pub struct CornerNeighbors {
    /// The cube corner where the 3 faces meet.
    pub corner: CubeCorner,
    /// The chunk on the second face that touches this corner.
    pub neighbor_a: ChunkAddress,
    /// The chunk on the third face that touches this corner.
    pub neighbor_b: ChunkAddress,
}

impl ChunkAddress {
    /// Find the diagonal neighbors at a cube corner.
    ///
    /// Returns `None` if this chunk is not at a cube corner.
    /// Otherwise returns the two chunks on the other faces that
    /// share the corner vertex.
    pub fn corner_neighbors(&self) -> Option<CornerNeighbors> {
        let (face_corner, cube_corner) = self.cube_corner()?;
        let faces = cube_corner.faces();

        // Find the two other faces (not self.face)
        let other_faces: Vec<CubeFace> = faces.iter()
            .copied()
            .filter(|&f| f != self.face)
            .collect();

        assert_eq!(other_faces.len(), 2);

        let grid = Self::grid_size(self.lod);

        // For each other face, find the chunk at this cube corner
        let neighbor_a = corner_chunk_on_face(other_faces[0], cube_corner, self.lod);
        let neighbor_b = corner_chunk_on_face(other_faces[1], cube_corner, self.lod);

        Some(CornerNeighbors {
            corner: cube_corner,
            neighbor_a,
            neighbor_b,
        })
    }
}

/// Find the chunk address at a specific cube corner on a given face.
fn corner_chunk_on_face(face: CubeFace, corner: CubeCorner, lod: u8) -> ChunkAddress {
    let grid = ChunkAddress::grid_size(lod);
    // Determine which UV corner of this face corresponds to the cube corner
    let t = face.tangent();
    let b = face.bitangent();
    let n = face.normal();
    let corner_pos = corner.position();

    // The corner position projected onto this face gives the UV corner
    let s = (corner_pos - n).dot(t); // +1 or -1
    let r = (corner_pos - n).dot(b); // +1 or -1

    let x = if s > 0.0 { grid - 1 } else { 0 };
    let y = if r > 0.0 { grid - 1 } else { 0 };

    ChunkAddress::new(face, lod, x, y)
}
```

### T-Junction Prevention at Corners

When chunks at different LODs meet at a corner, T-junctions arise. The solution is to constrain the quadtree so that adjacent chunks (including diagonal/corner adjacencies) differ by at most 1 LOD level:

```rust
/// Check if a corner has a LOD violation (difference > 1 between any two
/// of the three chunks meeting at the corner).
pub fn corner_lod_valid(
    chunk: &ChunkAddress,
    neighbor_a: &ChunkAddress,
    neighbor_b: &ChunkAddress,
) -> bool {
    let lods = [chunk.lod, neighbor_a.lod, neighbor_b.lod];
    let max_lod = *lods.iter().max().unwrap();
    let min_lod = *lods.iter().min().unwrap();
    max_lod - min_lod <= 1
}
```

### Design Constraints

- Corner adjacency is a pure function with no allocation (aside from the small `CornerNeighbors` struct).
- The corner vertex on the sphere must be computed identically regardless of which face's projection is used, to avoid cracks. This is guaranteed by the fact that all three faces project the same cube corner point `(+/-1, +/-1, +/-1)` through the same projection function, yielding the same sphere point.
- LOD constraints at corners are enforced by the quadtree manager (story 05), not by this module. This module only provides the query and validation functions.

## Outcome

The `nebula_cubesphere` crate exports `CubeCorner`, `FaceCorner`, `CornerNeighbors`, `ChunkAddress::cube_corner()`, `ChunkAddress::corner_neighbors()`, `corner_chunk_on_face()`, and `corner_lod_valid()`. Mesh stitching and LOD management use these to prevent cracks at the 8 corners of the cubesphere. Running `cargo test -p nebula_cubesphere` passes all corner case tests.

## Demo Integration

**Demo crate:** `nebula-demo`

The three chunks that meet at a cube corner are highlighted simultaneously when hovering near a corner, proving the 3-face corner adjacency is solved.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `glam` | 0.29 | `DVec3` for corner positions and basis vector math |

The crate uses Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_each_corner_touches_exactly_3_faces() {
        for corner in CubeCorner::ALL {
            let faces = corner.faces();
            assert_eq!(faces.len(), 3, "Corner {corner:?} does not touch 3 faces");

            // All 3 faces should be distinct
            let face_set: HashSet<CubeFace> = faces.iter().copied().collect();
            assert_eq!(face_set.len(), 3, "Corner {corner:?} has duplicate faces");
        }
    }

    #[test]
    fn test_corner_neighbor_lookup_returns_chunks_from_2_other_faces() {
        let grid = ChunkAddress::grid_size(10);
        // Place a chunk at the bottom-left corner of PosX
        let addr = ChunkAddress::new(CubeFace::PosX, 10, 0, 0);
        if let Some(neighbors) = addr.corner_neighbors() {
            assert_ne!(neighbors.neighbor_a.face, CubeFace::PosX);
            assert_ne!(neighbors.neighbor_b.face, CubeFace::PosX);
            assert_ne!(neighbors.neighbor_a.face, neighbors.neighbor_b.face);
        } else {
            panic!("Chunk at corner should have corner neighbors");
        }
    }

    #[test]
    fn test_non_corner_chunk_returns_none() {
        // A chunk in the middle of a face
        let addr = ChunkAddress::new(CubeFace::PosY, 10, 50, 50);
        assert!(addr.cube_corner().is_none());
        assert!(addr.corner_neighbors().is_none());
    }

    #[test]
    fn test_lod_transitions_at_corners_valid() {
        // Same LOD: valid
        let a = ChunkAddress::new(CubeFace::PosX, 10, 0, 0);
        let b = ChunkAddress::new(CubeFace::PosY, 10, 0, 0);
        let c = ChunkAddress::new(CubeFace::PosZ, 10, 0, 0);
        assert!(corner_lod_valid(&a, &b, &c));

        // Difference of 1: valid
        let b2 = ChunkAddress::new(CubeFace::PosY, 11, 0, 0);
        assert!(corner_lod_valid(&a, &b2, &c));

        // Difference of 2: invalid
        let c2 = ChunkAddress::new(CubeFace::PosZ, 12, 0, 0);
        assert!(!corner_lod_valid(&a, &b, &c2));
    }

    #[test]
    fn test_all_8_corners_tested() {
        // For each corner, verify the faces are consistent with the corner position
        for corner in CubeCorner::ALL {
            let pos = corner.position();
            let faces = corner.faces();

            for face in &faces {
                // The corner position should be on the positive side of the face normal
                let n = face.normal();
                assert!(
                    pos.dot(n) > 0.0,
                    "Corner {corner:?} position {pos:?} should be on the positive \
                     side of face {face:?} with normal {n:?}"
                );
            }
        }
    }

    #[test]
    fn test_corner_positions_are_unit_cube_vertices() {
        for corner in CubeCorner::ALL {
            let pos = corner.position();
            assert_eq!(pos.x.abs(), 1.0);
            assert_eq!(pos.y.abs(), 1.0);
            assert_eq!(pos.z.abs(), 1.0);
        }
    }

    #[test]
    fn test_all_cube_corners_covered_by_faces() {
        // Each face should touch exactly 4 corners
        for face in CubeFace::ALL {
            let touching_corners: Vec<CubeCorner> = CubeCorner::ALL
                .iter()
                .filter(|corner| corner.faces().contains(&face))
                .copied()
                .collect();
            assert_eq!(touching_corners.len(), 4,
                "Face {face:?} should touch 4 corners, got {}",
                touching_corners.len());
        }
    }

    #[test]
    fn test_corner_chunk_on_face_is_at_corner() {
        for corner in CubeCorner::ALL {
            let faces = corner.faces();
            for &face in &faces {
                let addr = corner_chunk_on_face(face, corner, 10);
                let grid = ChunkAddress::grid_size(10);
                assert!(
                    (addr.x == 0 || addr.x == grid - 1)
                    && (addr.y == 0 || addr.y == grid - 1),
                    "Chunk for corner {corner:?} on face {face:?} should be at a face corner, \
                     got ({}, {})",
                    addr.x, addr.y
                );
            }
        }
    }
}
```
