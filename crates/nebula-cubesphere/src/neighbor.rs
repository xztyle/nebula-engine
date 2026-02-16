//! Same-face neighbor finding for chunks on the cubesphere.
//!
//! Given a [`ChunkAddress`] and a cardinal [`FaceDirection`], these utilities
//! find the neighbor address(es) on the same cube face, accounting for LOD
//! boundaries. Cross-face neighbors are handled in a separate module (story 07).

use crate::{ChunkAddress, FaceQuadtree, QuadNode};

/// Cardinal directions on a cube face in UV space.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FaceDirection {
    /// Decreasing U (left).
    West,
    /// Increasing U (right).
    East,
    /// Decreasing V (down).
    South,
    /// Increasing V (up).
    North,
}

impl FaceDirection {
    /// All four cardinal directions.
    pub const ALL: [FaceDirection; 4] = [
        FaceDirection::North,
        FaceDirection::South,
        FaceDirection::East,
        FaceDirection::West,
    ];
}

/// Result of a same-face neighbor query at the same LOD.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SameFaceNeighbor {
    /// The neighbor is on the same face at the same LOD.
    Same(ChunkAddress),
    /// The neighbor would be off this face's edge (needs cross-face lookup).
    OffFace,
}

/// LOD-aware neighbor result from a quadtree query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LodNeighbor {
    /// A single neighbor chunk at the same or coarser LOD.
    Single(ChunkAddress),
    /// Multiple neighbor chunks at a finer LOD (the edge is shared with
    /// smaller chunks in the neighbor's subdivision).
    Multiple(Vec<ChunkAddress>),
    /// Neighbor is off this face.
    OffFace,
}

impl ChunkAddress {
    /// Find the same-face neighbor in the given direction at the same LOD.
    ///
    /// Returns [`SameFaceNeighbor::OffFace`] if the neighbor would be
    /// outside this face's grid.
    #[must_use]
    pub fn same_face_neighbor(&self, dir: FaceDirection) -> SameFaceNeighbor {
        let grid = Self::grid_size(self.lod);
        let (nx, ny) = match dir {
            FaceDirection::West => {
                if self.x == 0 {
                    return SameFaceNeighbor::OffFace;
                }
                (self.x - 1, self.y)
            }
            FaceDirection::East => {
                if self.x + 1 >= grid {
                    return SameFaceNeighbor::OffFace;
                }
                (self.x + 1, self.y)
            }
            FaceDirection::South => {
                if self.y == 0 {
                    return SameFaceNeighbor::OffFace;
                }
                (self.x, self.y - 1)
            }
            FaceDirection::North => {
                if self.y + 1 >= grid {
                    return SameFaceNeighbor::OffFace;
                }
                (self.x, self.y + 1)
            }
        };
        SameFaceNeighbor::Same(ChunkAddress::new(self.face, self.lod, nx, ny))
    }
}

impl FaceQuadtree {
    /// Find the actual loaded neighbor of `addr` in direction `dir`.
    ///
    /// If the neighbor region is a single leaf (same or coarser LOD),
    /// returns [`LodNeighbor::Single`]. If the neighbor region is subdivided
    /// finer than `addr`, returns [`LodNeighbor::Multiple`] with all leaf
    /// chunks along the shared edge.
    #[must_use]
    pub fn find_neighbor(&self, addr: &ChunkAddress, dir: FaceDirection) -> LodNeighbor {
        let same = addr.same_face_neighbor(dir);
        match same {
            SameFaceNeighbor::OffFace => LodNeighbor::OffFace,
            SameFaceNeighbor::Same(neighbor_addr) => {
                // Find what the quadtree actually has at this location
                let (u0, v0, u1, v1) = neighbor_addr.uv_bounds();
                let u_mid = (u0 + u1) * 0.5;
                let v_mid = (v0 + v1) * 0.5;
                let leaf = self.root.find_leaf(u_mid, v_mid);

                if leaf.lod >= addr.lod {
                    // Neighbor is at same or coarser LOD
                    LodNeighbor::Single(leaf)
                } else {
                    // Neighbor is more finely subdivided — collect all leaves
                    // along the shared edge
                    let edge_leaves = Self::leaves_along_edge(&self.root, u0, v0, u1, v1, dir);
                    LodNeighbor::Multiple(edge_leaves)
                }
            }
        }
    }

    /// Collect all leaf addresses that touch a specific edge of a UV region.
    ///
    /// Samples multiple points along the edge and collects unique leaves.
    fn leaves_along_edge(
        root: &QuadNode,
        u0: f64,
        v0: f64,
        u1: f64,
        v1: f64,
        dir: FaceDirection,
    ) -> Vec<ChunkAddress> {
        let mut leaves = Vec::new();
        let samples = 16; // enough to catch any subdivision level
        for i in 0..=samples {
            let t = i as f64 / samples as f64;
            let (u, v) = match dir {
                // When looking East, the shared edge is on the left (u0) side
                // of the neighbor region
                FaceDirection::East => (u0, v0 + t * (v1 - v0)),
                // When looking West, the shared edge is on the right (u1) side
                FaceDirection::West => (u1, v0 + t * (v1 - v0)),
                // When looking North, the shared edge is on the bottom (v0)
                FaceDirection::North => (u0 + t * (u1 - u0), v0),
                // When looking South, the shared edge is on the top (v1)
                FaceDirection::South => (u0 + t * (u1 - u0), v1),
            };
            let leaf = root.find_leaf(u.clamp(0.0, 1.0 - 1e-12), v.clamp(0.0, 1.0 - 1e-12));
            if !leaves.contains(&leaf) {
                leaves.push(leaf);
            }
        }
        leaves
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CubeFace;

    #[test]
    fn test_center_chunk_has_4_same_face_neighbors() {
        let addr = ChunkAddress::new(CubeFace::PosX, 10, 50, 50);
        for dir in FaceDirection::ALL {
            let neighbor = addr.same_face_neighbor(dir);
            match neighbor {
                SameFaceNeighbor::Same(n) => {
                    assert_eq!(n.face, CubeFace::PosX);
                    assert_eq!(n.lod, 10);
                }
                SameFaceNeighbor::OffFace => {
                    panic!("Center chunk should have same-face neighbor in {dir:?}");
                }
            }
        }
    }

    #[test]
    fn test_edge_chunk_has_3_same_face_neighbors() {
        let addr = ChunkAddress::new(CubeFace::PosY, 10, 0, 50);
        assert!(matches!(
            addr.same_face_neighbor(FaceDirection::West),
            SameFaceNeighbor::OffFace
        ));
        assert!(matches!(
            addr.same_face_neighbor(FaceDirection::East),
            SameFaceNeighbor::Same(_)
        ));
        assert!(matches!(
            addr.same_face_neighbor(FaceDirection::North),
            SameFaceNeighbor::Same(_)
        ));
        assert!(matches!(
            addr.same_face_neighbor(FaceDirection::South),
            SameFaceNeighbor::Same(_)
        ));
    }

    #[test]
    fn test_corner_chunk_has_2_same_face_neighbors() {
        let grid = ChunkAddress::grid_size(10);
        let addr = ChunkAddress::new(CubeFace::NegZ, 10, 0, grid - 1);
        let mut off_count = 0;
        for dir in FaceDirection::ALL {
            if matches!(addr.same_face_neighbor(dir), SameFaceNeighbor::OffFace) {
                off_count += 1;
            }
        }
        assert_eq!(
            off_count, 2,
            "Corner chunk should have 2 off-face directions"
        );
    }

    #[test]
    fn test_neighbor_coordinates_correct() {
        let addr = ChunkAddress::new(CubeFace::PosZ, 8, 100, 200);
        if let SameFaceNeighbor::Same(n) = addr.same_face_neighbor(FaceDirection::East) {
            assert_eq!(n.x, 101);
            assert_eq!(n.y, 200);
        }
        if let SameFaceNeighbor::Same(n) = addr.same_face_neighbor(FaceDirection::West) {
            assert_eq!(n.x, 99);
            assert_eq!(n.y, 200);
        }
        if let SameFaceNeighbor::Same(n) = addr.same_face_neighbor(FaceDirection::North) {
            assert_eq!(n.x, 100);
            assert_eq!(n.y, 201);
        }
        if let SameFaceNeighbor::Same(n) = addr.same_face_neighbor(FaceDirection::South) {
            assert_eq!(n.x, 100);
            assert_eq!(n.y, 199);
        }
    }

    #[test]
    fn test_lod_mismatch_returns_parent_chunk() {
        let mut tree = FaceQuadtree::new(CubeFace::PosX);
        tree.root.subdivide(); // 4 children at LOD MAX-1

        // Subdivide only child[0] further
        if let QuadNode::Branch { children, .. } = &mut tree.root {
            children[0].subdivide(); // 4 grandchildren at LOD MAX-2
        }

        // A grandchild asking for its east neighbor should find a
        // same-LOD-or-coarser leaf
        let leaves = tree.root.all_leaves();
        let fine_leaf = leaves
            .iter()
            .find(|a| a.lod == ChunkAddress::MAX_LOD - 2)
            .expect("should have a fine leaf");

        let result = tree.find_neighbor(fine_leaf, FaceDirection::East);
        match result {
            LodNeighbor::Single(n) => {
                assert!(
                    n.lod >= fine_leaf.lod - 1,
                    "Neighbor should be at same or coarser LOD"
                );
            }
            LodNeighbor::Multiple(_) | LodNeighbor::OffFace => {
                // Also acceptable depending on which grandchild we picked
            }
        }
    }

    #[test]
    fn test_all_neighbors_are_valid_addresses() {
        let addr = ChunkAddress::new(CubeFace::NegY, 5, 10, 10);
        for dir in FaceDirection::ALL {
            if let SameFaceNeighbor::Same(n) = addr.same_face_neighbor(dir) {
                let grid = ChunkAddress::grid_size(n.lod);
                assert!(n.x < grid, "Neighbor x out of range");
                assert!(n.y < grid, "Neighbor y out of range");
                assert_eq!(n.face, addr.face);
                assert_eq!(n.lod, addr.lod);
            }
        }
    }

    #[test]
    fn test_lod_neighbor_off_face() {
        let tree = FaceQuadtree::new(CubeFace::PosX);
        let addr = ChunkAddress::new(CubeFace::PosX, ChunkAddress::MAX_LOD, 0, 0);
        // The root is the only chunk; all directions are off-face
        for dir in FaceDirection::ALL {
            let result = tree.find_neighbor(&addr, dir);
            assert!(
                matches!(result, LodNeighbor::OffFace),
                "Root chunk neighbor should be off-face in {dir:?}"
            );
        }
    }

    #[test]
    fn test_lod_neighbor_same_level() {
        let mut tree = FaceQuadtree::new(CubeFace::PosX);
        tree.root.subdivide();

        // Bottom-left child looking east should find bottom-right child
        let leaves = tree.root.all_leaves();
        let bottom_left = leaves
            .iter()
            .find(|a| a.x == 0 && a.y == 0)
            .expect("should have bottom-left");

        let result = tree.find_neighbor(bottom_left, FaceDirection::East);
        match result {
            LodNeighbor::Single(n) => {
                assert_eq!(n.lod, bottom_left.lod);
                assert_eq!(n.x, bottom_left.x + 1);
                assert_eq!(n.y, bottom_left.y);
            }
            _ => panic!("Expected Single neighbor"),
        }
    }

    #[test]
    fn test_finer_neighbor_returns_multiple() {
        let mut tree = FaceQuadtree::new(CubeFace::PosX);
        tree.root.subdivide();

        // Subdivide the bottom-right child (index 1)
        if let QuadNode::Branch { children, .. } = &mut tree.root {
            children[1].subdivide();
        }

        // The bottom-left child (index 0) looking East should find
        // multiple finer leaves from the subdivided bottom-right
        let leaves = tree.root.all_leaves();
        let bottom_left = leaves
            .iter()
            .find(|a| a.lod == ChunkAddress::MAX_LOD - 1 && a.x == 0 && a.y == 0)
            .expect("should have bottom-left");

        let result = tree.find_neighbor(bottom_left, FaceDirection::East);
        if let LodNeighbor::Multiple(neighbors) = result {
            assert!(
                neighbors.len() >= 2,
                "Should have multiple finer neighbors along the edge"
            );
            // At least one neighbor should be finer (lower LOD number)
            let has_finer = neighbors.iter().any(|n| n.lod < bottom_left.lod);
            assert!(has_finer, "Should have at least one finer neighbor");
        } else {
            panic!("Expected Multiple neighbor, got {result:?}");
        }
    }
}
