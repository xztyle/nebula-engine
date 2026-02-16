//! Cross-face neighbor finding for chunks at cube-face edges.
//!
//! When a chunk sits at the boundary of a cube face, its neighbor in one
//! direction lies on a different face. This module encodes the cube's face
//! adjacency topology and provides coordinate transforms across seams.

use crate::neighbor::FaceDirection;
use crate::{ChunkAddress, CubeFace, FaceCoord};

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
#[must_use]
pub fn face_adjacency(face: CubeFace, edge: FaceDirection) -> FaceEdgeAdjacency {
    use CubeFace::*;
    use FaceDirection::*;

    match (face, edge) {
        // PosX face edges
        (PosX, North) => FaceEdgeAdjacency {
            neighbor_face: PosY,
            neighbor_edge: East,
            flipped: false,
        },
        (PosX, South) => FaceEdgeAdjacency {
            neighbor_face: NegY,
            neighbor_edge: East,
            flipped: true,
        },
        (PosX, East) => FaceEdgeAdjacency {
            neighbor_face: PosZ,
            neighbor_edge: West,
            flipped: false,
        },
        (PosX, West) => FaceEdgeAdjacency {
            neighbor_face: NegZ,
            neighbor_edge: East,
            flipped: false,
        },

        // NegX face edges
        (NegX, North) => FaceEdgeAdjacency {
            neighbor_face: PosY,
            neighbor_edge: West,
            flipped: true,
        },
        (NegX, South) => FaceEdgeAdjacency {
            neighbor_face: NegY,
            neighbor_edge: West,
            flipped: false,
        },
        (NegX, East) => FaceEdgeAdjacency {
            neighbor_face: NegZ,
            neighbor_edge: West,
            flipped: false,
        },
        (NegX, West) => FaceEdgeAdjacency {
            neighbor_face: PosZ,
            neighbor_edge: East,
            flipped: false,
        },

        // PosY face edges
        (PosY, North) => FaceEdgeAdjacency {
            neighbor_face: NegZ,
            neighbor_edge: North,
            flipped: true,
        },
        (PosY, South) => FaceEdgeAdjacency {
            neighbor_face: PosZ,
            neighbor_edge: North,
            flipped: false,
        },
        (PosY, East) => FaceEdgeAdjacency {
            neighbor_face: PosX,
            neighbor_edge: North,
            flipped: false,
        },
        (PosY, West) => FaceEdgeAdjacency {
            neighbor_face: NegX,
            neighbor_edge: North,
            flipped: true,
        },

        // NegY face edges
        (NegY, North) => FaceEdgeAdjacency {
            neighbor_face: PosZ,
            neighbor_edge: South,
            flipped: false,
        },
        (NegY, South) => FaceEdgeAdjacency {
            neighbor_face: NegZ,
            neighbor_edge: South,
            flipped: true,
        },
        (NegY, East) => FaceEdgeAdjacency {
            neighbor_face: PosX,
            neighbor_edge: South,
            flipped: true,
        },
        (NegY, West) => FaceEdgeAdjacency {
            neighbor_face: NegX,
            neighbor_edge: South,
            flipped: false,
        },

        // PosZ face edges
        (PosZ, North) => FaceEdgeAdjacency {
            neighbor_face: PosY,
            neighbor_edge: South,
            flipped: false,
        },
        (PosZ, South) => FaceEdgeAdjacency {
            neighbor_face: NegY,
            neighbor_edge: North,
            flipped: false,
        },
        (PosZ, East) => FaceEdgeAdjacency {
            neighbor_face: NegX,
            neighbor_edge: West,
            flipped: false,
        },
        (PosZ, West) => FaceEdgeAdjacency {
            neighbor_face: PosX,
            neighbor_edge: East,
            flipped: false,
        },

        // NegZ face edges
        (NegZ, North) => FaceEdgeAdjacency {
            neighbor_face: PosY,
            neighbor_edge: North,
            flipped: true,
        },
        (NegZ, South) => FaceEdgeAdjacency {
            neighbor_face: NegY,
            neighbor_edge: South,
            flipped: true,
        },
        (NegZ, East) => FaceEdgeAdjacency {
            neighbor_face: PosX,
            neighbor_edge: West,
            flipped: false,
        },
        (NegZ, West) => FaceEdgeAdjacency {
            neighbor_face: NegX,
            neighbor_edge: East,
            flipped: false,
        },
    }
}

/// Transform a UV coordinate from the edge of one face to the corresponding
/// UV on the adjacent face.
///
/// The input `(u, v)` should lie on the edge indicated by `dir` (e.g., for
/// [`FaceDirection::North`], `v` should be 1.0). The returned [`FaceCoord`]
/// gives the corresponding point on the neighbor face.
#[must_use]
pub fn transform_uv_across_edge(face: CubeFace, dir: FaceDirection, u: f64, v: f64) -> FaceCoord {
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

impl ChunkAddress {
    /// Find the neighbor across a face boundary.
    ///
    /// `self` must be at an edge of its face in direction `dir` (i.e.,
    /// [`same_face_neighbor`](Self::same_face_neighbor) returns
    /// [`OffFace`](crate::SameFaceNeighbor::OffFace) for that direction).
    /// Returns the [`ChunkAddress`] on the adjacent face.
    #[must_use]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_every_face_edge_connects_to_one_other_face() {
        for face in CubeFace::ALL {
            for dir in FaceDirection::ALL {
                let adj = face_adjacency(face, dir);
                assert_ne!(
                    adj.neighbor_face, face,
                    "Face {face:?} edge {dir:?} should not connect to itself"
                );
            }
        }
    }

    #[test]
    fn test_adjacency_is_symmetric() {
        for face in CubeFace::ALL {
            for dir in FaceDirection::ALL {
                let adj = face_adjacency(face, dir);
                let reverse = face_adjacency(adj.neighbor_face, adj.neighbor_edge);
                assert_eq!(
                    reverse.neighbor_face,
                    face,
                    "Adjacency not symmetric: {face:?}/{dir:?} -> {:?}/{:?} -> {:?}/{:?}",
                    adj.neighbor_face,
                    adj.neighbor_edge,
                    reverse.neighbor_face,
                    reverse.neighbor_edge
                );
            }
        }
    }

    #[test]
    fn test_all_24_edge_connections_defined() {
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
                let (u, v) = match dir {
                    FaceDirection::North => (0.3, 1.0),
                    FaceDirection::South => (0.7, 0.0),
                    FaceDirection::East => (1.0, 0.4),
                    FaceDirection::West => (0.0, 0.6),
                };

                let adj = face_adjacency(face, dir);
                let transformed = transform_uv_across_edge(face, dir, u, v);

                let back = transform_uv_across_edge(
                    transformed.face,
                    adj.neighbor_edge,
                    transformed.u,
                    transformed.v,
                );

                assert_eq!(
                    back.face, face,
                    "Roundtrip face mismatch for {face:?}/{dir:?}"
                );

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
        use crate::SameFaceNeighbor;

        let grid = ChunkAddress::grid_size(10);
        let addr = ChunkAddress::new(CubeFace::PosX, 10, grid - 1, 50);

        if matches!(
            addr.same_face_neighbor(FaceDirection::East),
            SameFaceNeighbor::OffFace
        ) {
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
            assert_eq!(
                neighbor_faces.len(),
                4,
                "Face {face:?} should connect to 4 distinct faces, got {neighbor_faces:?}"
            );
            assert!(
                !neighbor_faces.contains(&face.opposite()),
                "Face {face:?} should not be adjacent to its opposite {:?}",
                face.opposite()
            );
        }
    }

    #[test]
    fn test_cross_face_neighbor_symmetry() {
        // If chunk A's cross-face neighbor in direction D is chunk B,
        // then B's cross-face neighbor back should land on A's face
        // at the same edge coordinate.
        let grid = ChunkAddress::grid_size(10);
        for face in CubeFace::ALL {
            for dir in FaceDirection::ALL {
                // Pick an edge chunk
                let (x, y) = match dir {
                    FaceDirection::North => (42, grid - 1),
                    FaceDirection::South => (42, 0),
                    FaceDirection::East => (grid - 1, 42),
                    FaceDirection::West => (0, 42),
                };
                let addr = ChunkAddress::new(face, 10, x, y);
                let adj = face_adjacency(face, dir);
                let cross = addr.cross_face_neighbor(dir);
                let back = cross.cross_face_neighbor(adj.neighbor_edge);
                assert_eq!(
                    back.face, face,
                    "Round-trip face mismatch for {face:?}/{dir:?}"
                );
                assert_eq!(back.x, addr.x, "Round-trip x mismatch for {face:?}/{dir:?}");
                assert_eq!(back.y, addr.y, "Round-trip y mismatch for {face:?}/{dir:?}");
            }
        }
    }

    #[test]
    fn test_flip_symmetry() {
        // If A→B is flipped, then B→A must also be flipped (double flip = identity)
        for face in CubeFace::ALL {
            for dir in FaceDirection::ALL {
                let adj = face_adjacency(face, dir);
                let reverse = face_adjacency(adj.neighbor_face, adj.neighbor_edge);
                assert_eq!(
                    adj.flipped, reverse.flipped,
                    "Flip asymmetry: {face:?}/{dir:?} flipped={}, reverse flipped={}",
                    adj.flipped, reverse.flipped
                );
            }
        }
    }
}
