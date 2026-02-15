# Cross-Face Neighbor Finding

## Problem

When a chunk sits at the edge of a cube face, its neighbor in one direction lies on a different face of the cube. Unlike same-face neighbors (story 06), cross-face neighbors require knowing which face is adjacent, which edge connects them, and how the UV coordinate systems transform across the seam. A cube has 12 edges, each shared by 2 faces, and the UV axes may be rotated or flipped relative to each other. If the adjacency table or UV transform is wrong for even one of the 24 face-edge combinations (6 faces times 4 edges), there will be visible seams, mismatched terrain, or cracks in the mesh at that boundary. This is the trickiest piece of cubesphere geometry and must be precisely defined.

## Solution

Implement cross-face neighbor finding in the `nebula_cubesphere` crate.

### Face Adjacency Table

For each face and each edge direction (N/S/E/W), define which face is adjacent and how the UV coordinates transform:

```rust
/// Describes the relationship between a face edge and its adjacent face.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FaceEdgeAdjacency {
    /// The adjacent face.
    pub neighbor_face: CubeFace,
    /// Which edge of the neighbor face this connects to.
    pub neighbor_edge: FaceDirection,
    /// Whether the coordinate along the shared edge is flipped.
    /// If true, U=0 on this edge maps to the far end of the neighbor edge.
    pub flipped: bool,
}

/// The full adjacency table: for each face and each edge, which face is adjacent
/// and how the coordinates transform.
///
/// This table encodes the topology of a cube. Each of the 6 faces has 4 edges,
/// giving 24 entries. Each edge is shared by exactly 2 faces, so there are 12
/// unique edge connections.
pub fn face_adjacency(face: CubeFace, edge: FaceDirection) -> FaceEdgeAdjacency {
    use CubeFace::*;
    use FaceDirection::*;

    match (face, edge) {
        // PosX face edges
        (PosX, North) => FaceEdgeAdjacency { neighbor_face: PosY, neighbor_edge: East, flipped: false },
        (PosX, South) => FaceEdgeAdjacency { neighbor_face: NegY, neighbor_edge: East, flipped: true },
        (PosX, East)  => FaceEdgeAdjacency { neighbor_face: PosZ, neighbor_edge: West, flipped: false },  // Note: depends on tangent/bitangent convention
        (PosX, West)  => FaceEdgeAdjacency { neighbor_face: NegZ, neighbor_edge: East, flipped: false },

        // NegX face edges
        (NegX, North) => FaceEdgeAdjacency { neighbor_face: PosY, neighbor_edge: West, flipped: true },
        (NegX, South) => FaceEdgeAdjacency { neighbor_face: NegY, neighbor_edge: West, flipped: false },
        (NegX, East)  => FaceEdgeAdjacency { neighbor_face: NegZ, neighbor_edge: West, flipped: false },
        (NegX, West)  => FaceEdgeAdjacency { neighbor_face: PosZ, neighbor_edge: East, flipped: false },

        // PosY face edges
        (PosY, North) => FaceEdgeAdjacency { neighbor_face: NegZ, neighbor_edge: North, flipped: true },
        (PosY, South) => FaceEdgeAdjacency { neighbor_face: PosZ, neighbor_edge: North, flipped: false },
        (PosY, East)  => FaceEdgeAdjacency { neighbor_face: PosX, neighbor_edge: North, flipped: false },
        (PosY, West)  => FaceEdgeAdjacency { neighbor_face: NegX, neighbor_edge: North, flipped: true },

        // NegY face edges
        (NegY, North) => FaceEdgeAdjacency { neighbor_face: PosZ, neighbor_edge: South, flipped: false },
        (NegY, South) => FaceEdgeAdjacency { neighbor_face: NegZ, neighbor_edge: South, flipped: true },
        (NegY, East)  => FaceEdgeAdjacency { neighbor_face: PosX, neighbor_edge: South, flipped: true },
        (NegY, West)  => FaceEdgeAdjacency { neighbor_face: NegX, neighbor_edge: South, flipped: false },

        // PosZ face edges
        (PosZ, North) => FaceEdgeAdjacency { neighbor_face: PosY, neighbor_edge: South, flipped: false },
        (PosZ, South) => FaceEdgeAdjacency { neighbor_face: NegY, neighbor_edge: North, flipped: false },
        (PosZ, East)  => FaceEdgeAdjacency { neighbor_face: NegX, neighbor_edge: West, flipped: false },
        (PosZ, West)  => FaceEdgeAdjacency { neighbor_face: PosX, neighbor_edge: East, flipped: false },

        // NegZ face edges
        (NegZ, North) => FaceEdgeAdjacency { neighbor_face: PosY, neighbor_edge: North, flipped: true },
        (NegZ, South) => FaceEdgeAdjacency { neighbor_face: NegY, neighbor_edge: South, flipped: true },
        (NegZ, East)  => FaceEdgeAdjacency { neighbor_face: PosX, neighbor_edge: West, flipped: false },
        (NegZ, West)  => FaceEdgeAdjacency { neighbor_face: NegX, neighbor_edge: East, flipped: false },
    }
}
```

The exact entries in this table depend on the tangent/bitangent convention established in story 01. The table above is consistent with the basis vectors defined there. The `flipped` flag indicates whether traversal along the shared edge runs in the same direction on both faces or is reversed.

### Cross-Face Neighbor Address Computation

```rust
impl ChunkAddress {
    /// Find the neighbor across a face boundary.
    ///
    /// `self` must be at an edge of its face in direction `dir`.
    /// Returns the ChunkAddress on the adjacent face.
    pub fn cross_face_neighbor(&self, dir: FaceDirection) -> ChunkAddress {
        let adj = face_adjacency(self.face, dir);
        let grid = Self::grid_size(self.lod);

        // Determine the coordinate along the shared edge on the source face
        let edge_coord = match dir {
            FaceDirection::North | FaceDirection::South => self.x,
            FaceDirection::East | FaceDirection::West => self.y,
        };

        // Apply flip if needed
        let neighbor_edge_coord = if adj.flipped {
            grid - 1 - edge_coord
        } else {
            edge_coord
        };

        // Map onto the neighbor face's coordinate system
        let (nx, ny) = match adj.neighbor_edge {
            FaceDirection::North => (neighbor_edge_coord, grid - 1),
            FaceDirection::South => (neighbor_edge_coord, 0),
            FaceDirection::East => (grid - 1, neighbor_edge_coord),
            FaceDirection::West => (0, neighbor_edge_coord),
        };

        ChunkAddress::new(adj.neighbor_face, self.lod, nx, ny)
    }
}
```

### UV Transform for Seam Stitching

For mesh stitching at face boundaries, we also need to transform a (u, v) coordinate from one face's edge to the corresponding (u, v) on the neighbor face:

```rust
/// Transform a UV coordinate from the edge of one face to the corresponding
/// UV on the adjacent face.
pub fn transform_uv_across_edge(
    face: CubeFace,
    dir: FaceDirection,
    u: f64,
    v: f64,
) -> FaceCoord {
    let adj = face_adjacency(face, dir);

    // Extract the edge-parallel coordinate
    let edge_t = match dir {
        FaceDirection::North | FaceDirection::South => u,
        FaceDirection::East | FaceDirection::West => v,
    };

    let neighbor_t = if adj.flipped { 1.0 - edge_t } else { edge_t };

    // Map onto the neighbor face's UV system
    let (nu, nv) = match adj.neighbor_edge {
        FaceDirection::North => (neighbor_t, 1.0),
        FaceDirection::South => (neighbor_t, 0.0),
        FaceDirection::East => (1.0, neighbor_t),
        FaceDirection::West => (0.0, neighbor_t),
    };

    FaceCoord::new(adj.neighbor_face, nu, nv)
}
```

### Design Constraints

- The adjacency table is a pure function (no mutable state, no allocation). It will be called frequently during chunk loading and mesh stitching.
- The table must be validated at compile time or by exhaustive unit tests: every face-edge pair must map to a valid adjacent face, and the mapping must be symmetric (if face A's north edge connects to face B's east edge, then face B's east edge must connect to face A's north edge).
- The `flipped` flag handles the cases where the UV coordinate along the shared edge runs in opposite directions on the two faces.

## Outcome

The `nebula_cubesphere` crate exports `FaceEdgeAdjacency`, `face_adjacency()`, `ChunkAddress::cross_face_neighbor()`, and `transform_uv_across_edge()`. Any system that needs to find neighbors across face boundaries — chunk loading, mesh stitching, terrain blending — uses these functions. Running `cargo test -p nebula_cubesphere` passes all cross-face adjacency tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Chunks at face edges now correctly identify neighbors on adjacent faces. Hovering over an edge chunk highlights neighbors spanning two differently-colored faces.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| *(none)* | — | Pure `std` only; adjacency is a lookup table with arithmetic |

The crate uses Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_every_face_edge_connects_to_one_other_face() {
        for face in CubeFace::ALL {
            for dir in FaceDirection::ALL {
                let adj = face_adjacency(face, dir);
                assert_ne!(adj.neighbor_face, face,
                    "Face {face:?} edge {dir:?} should not connect to itself");
            }
        }
    }

    #[test]
    fn test_adjacency_is_symmetric() {
        for face in CubeFace::ALL {
            for dir in FaceDirection::ALL {
                let adj = face_adjacency(face, dir);
                let reverse = face_adjacency(adj.neighbor_face, adj.neighbor_edge);
                assert_eq!(reverse.neighbor_face, face,
                    "Adjacency not symmetric: {face:?}/{dir:?} -> {:?}/{:?} -> {:?}/{:?}",
                    adj.neighbor_face, adj.neighbor_edge,
                    reverse.neighbor_face, reverse.neighbor_edge
                );
            }
        }
    }

    #[test]
    fn test_all_24_edge_connections_defined() {
        // Simply calling face_adjacency for all 24 combinations should not panic
        let mut count = 0;
        for face in CubeFace::ALL {
            for dir in FaceDirection::ALL {
                let _adj = face_adjacency(face, dir);
                count += 1;
            }
        }
        assert_eq!(count, 24);
    }

    #[test]
    fn test_uv_transform_roundtrip() {
        for face in CubeFace::ALL {
            for dir in FaceDirection::ALL {
                // Pick a point on the edge
                let (u, v) = match dir {
                    FaceDirection::North => (0.3, 1.0),
                    FaceDirection::South => (0.7, 0.0),
                    FaceDirection::East => (1.0, 0.4),
                    FaceDirection::West => (0.0, 0.6),
                };

                let adj = face_adjacency(face, dir);
                let transformed = transform_uv_across_edge(face, dir, u, v);

                // Transform back from the neighbor's perspective
                let back = transform_uv_across_edge(
                    transformed.face,
                    adj.neighbor_edge,
                    transformed.u,
                    transformed.v,
                );

                assert_eq!(back.face, face,
                    "Roundtrip face mismatch for {face:?}/{dir:?}");
                // The edge coordinate should roundtrip
                let orig_t = match dir {
                    FaceDirection::North | FaceDirection::South => u,
                    FaceDirection::East | FaceDirection::West => v,
                };
                let back_t = match dir {
                    FaceDirection::North | FaceDirection::South => back.u,
                    FaceDirection::East | FaceDirection::West => back.v,
                };
                assert!(
                    (orig_t - back_t).abs() < 1e-10,
                    "UV roundtrip failed for {face:?}/{dir:?}: {orig_t} -> {back_t}"
                );
            }
        }
    }

    #[test]
    fn test_cross_face_neighbor_valid_address() {
        let grid = ChunkAddress::grid_size(10);
        // Chunk at east edge of PosX face
        let addr = ChunkAddress::new(CubeFace::PosX, 10, grid - 1, 50);
        // The east neighbor should not panic and should be on a different face
        // (only if East is actually off-face, which depends on the convention;
        //  for PosX, increasing U goes toward NegZ, so East at u=max is off-face)
        // First check if it's off-face via same_face_neighbor
        if matches!(addr.same_face_neighbor(FaceDirection::East), SameFaceNeighbor::OffFace) {
            let cross = addr.cross_face_neighbor(FaceDirection::East);
            assert_ne!(cross.face, CubeFace::PosX);
            let cross_grid = ChunkAddress::grid_size(cross.lod);
            assert!(cross.x < cross_grid);
            assert!(cross.y < cross_grid);
        }
    }

    #[test]
    fn test_each_face_has_4_distinct_neighbors() {
        for face in CubeFace::ALL {
            let mut neighbor_faces: Vec<CubeFace> = FaceDirection::ALL
                .iter()
                .map(|&dir| face_adjacency(face, dir).neighbor_face)
                .collect();
            neighbor_faces.sort();
            neighbor_faces.dedup();
            assert_eq!(neighbor_faces.len(), 4,
                "Face {face:?} should connect to 4 distinct faces, got {neighbor_faces:?}");
            // The opposite face should NOT be a neighbor
            assert!(!neighbor_faces.contains(&face.opposite()),
                "Face {face:?} should not be adjacent to its opposite {:?}",
                face.opposite());
        }
    }
}
```
