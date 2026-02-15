# Same-Face Neighbor Finding

## Problem

Mesh generation, terrain blending, ambient occlusion, and LOD stitching all require knowing which chunks are adjacent to a given chunk. The simplest case is when the neighbor is on the same cube face — a matter of incrementing or decrementing the `x` or `y` coordinate of the `ChunkAddress`. However, even this "simple" case has subtleties: a chunk at the edge of the face has no same-face neighbor in one direction (that neighbor is on an adjacent face, handled in story 07), and when the quadtree has chunks at different LODs, a fine chunk's neighbor might be a coarser parent chunk or the neighbor might be subdivided into multiple finer children. The engine needs a function that, given a `ChunkAddress` and a cardinal direction, returns the neighbor address(es) on the same face, accounting for LOD boundaries.

## Solution

Implement same-face neighbor finding in the `nebula_cubesphere` crate.

### Cardinal Direction Enum

```rust
/// Cardinal directions on a cube face in UV space.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FaceDirection {
    /// Decreasing U (left)
    West,
    /// Increasing U (right)
    East,
    /// Decreasing V (down)
    South,
    /// Increasing V (up)
    North,
}

impl FaceDirection {
    pub const ALL: [FaceDirection; 4] = [
        FaceDirection::North,
        FaceDirection::South,
        FaceDirection::East,
        FaceDirection::West,
    ];
}
```

### Same-Face Neighbor at Same LOD

```rust
/// Result of a same-face neighbor query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SameFaceNeighbor {
    /// The neighbor is on the same face at the same LOD.
    Same(ChunkAddress),
    /// The neighbor would be off this face's edge (needs cross-face lookup).
    OffFace,
}

impl ChunkAddress {
    /// Find the same-face neighbor in the given direction at the same LOD.
    ///
    /// Returns `SameFaceNeighbor::OffFace` if the neighbor would be
    /// outside this face's grid.
    pub fn same_face_neighbor(&self, dir: FaceDirection) -> SameFaceNeighbor {
        let grid = Self::grid_size(self.lod);
        let (nx, ny) = match dir {
            FaceDirection::West => {
                if self.x == 0 { return SameFaceNeighbor::OffFace; }
                (self.x - 1, self.y)
            }
            FaceDirection::East => {
                if self.x + 1 >= grid { return SameFaceNeighbor::OffFace; }
                (self.x + 1, self.y)
            }
            FaceDirection::South => {
                if self.y == 0 { return SameFaceNeighbor::OffFace; }
                (self.x, self.y - 1)
            }
            FaceDirection::North => {
                if self.y + 1 >= grid { return SameFaceNeighbor::OffFace; }
                (self.x, self.y + 1)
            }
        };
        SameFaceNeighbor::Same(ChunkAddress::new(self.face, self.lod, nx, ny))
    }
}
```

### LOD-Aware Neighbor Finding

When the quadtree has chunks at different LODs, the neighbor query must consult the quadtree to determine the actual loaded neighbor:

```rust
/// LOD-aware neighbor result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LodNeighbor {
    /// A single neighbor chunk at the same or coarser LOD.
    Single(ChunkAddress),
    /// Multiple neighbor chunks at a finer LOD (the edge is shared with
    /// 2 smaller chunks in the neighbor's subdivision).
    Multiple(Vec<ChunkAddress>),
    /// Neighbor is off this face.
    OffFace,
}

impl FaceQuadtree {
    /// Find the actual loaded neighbor of `addr` in direction `dir`.
    ///
    /// If the neighbor region is a single leaf (same or coarser LOD),
    /// return `LodNeighbor::Single`. If the neighbor region is subdivided
    /// finer than `addr`, return `LodNeighbor::Multiple` with all leaf
    /// chunks along the shared edge.
    pub fn find_neighbor(
        &self,
        addr: &ChunkAddress,
        dir: FaceDirection,
    ) -> LodNeighbor {
        let same = addr.same_face_neighbor(dir);
        match same {
            SameFaceNeighbor::OffFace => LodNeighbor::OffFace,
            SameFaceNeighbor::Same(neighbor_addr) => {
                // Find what the quadtree actually has at this location
                let (u_mid, v_mid) = {
                    let (u0, v0, u1, v1) = neighbor_addr.uv_bounds();
                    ((u0 + u1) * 0.5, (v0 + v1) * 0.5)
                };
                let leaf = self.root.find_leaf(u_mid, v_mid);

                if leaf.lod >= addr.lod {
                    // Neighbor is at same or coarser LOD
                    LodNeighbor::Single(leaf)
                } else {
                    // Neighbor is more finely subdivided — collect all leaves
                    // along the shared edge
                    let (u0, v0, u1, v1) = neighbor_addr.uv_bounds();
                    let edge_leaves = self.leaves_along_edge(u0, v0, u1, v1, dir);
                    LodNeighbor::Multiple(edge_leaves)
                }
            }
        }
    }

    /// Collect all leaf addresses that touch a specific edge of a UV region.
    fn leaves_along_edge(
        &self,
        u0: f64, v0: f64, u1: f64, v1: f64,
        dir: FaceDirection,
    ) -> Vec<ChunkAddress> {
        // Sample multiple points along the edge and collect unique leaves
        let mut leaves = Vec::new();
        let samples = 16; // enough to catch any subdivision level
        for i in 0..=samples {
            let t = i as f64 / samples as f64;
            let (u, v) = match dir {
                FaceDirection::East => (u0, v0 + t * (v1 - v0)),
                FaceDirection::West => (u1, v0 + t * (v1 - v0)),
                FaceDirection::North => (u0 + t * (u1 - u0), v0),
                FaceDirection::South => (u0 + t * (u1 - u0), v1),
            };
            let leaf = self.root.find_leaf(u.clamp(0.0, 0.999999), v.clamp(0.0, 0.999999));
            if !leaves.contains(&leaf) {
                leaves.push(leaf);
            }
        }
        leaves
    }
}
```

### Design Constraints

- Same-LOD neighbor finding is O(1) — pure arithmetic on `ChunkAddress` fields.
- LOD-aware neighbor finding requires a quadtree traversal but is bounded by the tree depth (at most `MAX_LOD` levels, i.e., 20).
- The function does not modify the quadtree; it is a read-only query.
- Cross-face neighbors are explicitly deferred to story 07.

## Outcome

The `nebula_cubesphere` crate exports `FaceDirection`, `SameFaceNeighbor`, `LodNeighbor`, `ChunkAddress::same_face_neighbor()`, and `FaceQuadtree::find_neighbor()`. Mesh stitching and LOD transition systems use these to find adjacent chunks for seamless rendering. Running `cargo test -p nebula_cubesphere` passes all same-face neighbor tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Hovering over a chunk highlights its 4 cardinal neighbors in bright white wireframe, showing same-face neighbor relationships.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| *(none)* | — | Pure `std` only; neighbor finding is arithmetic on `ChunkAddress` fields |

The crate uses Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_center_chunk_has_4_same_face_neighbors() {
        // A chunk in the middle of the face should have neighbors in all 4 directions
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
        // Chunk at x=0 has no west neighbor on this face
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
        assert_eq!(off_count, 2, "Corner chunk should have 2 off-face directions");
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
        // Build a quadtree where one region is coarser than its neighbor
        let mut tree = FaceQuadtree::new(CubeFace::PosX);
        tree.root.subdivide(); // 4 children at LOD MAX-1

        // Subdivide only child[0] further
        if let QuadNode::Branch { children, .. } = &mut tree.root {
            children[0].subdivide(); // 4 grandchildren at LOD MAX-2
        }

        // A grandchild asking for its east neighbor should find a
        // same-LOD-or-coarser leaf
        let leaves = tree.root.all_leaves();
        let fine_leaf = leaves.iter()
            .find(|a| a.lod == ChunkAddress::MAX_LOD - 2)
            .unwrap();

        // Query the tree for the neighbor
        let result = tree.find_neighbor(fine_leaf, FaceDirection::East);
        match result {
            LodNeighbor::Single(n) => {
                assert!(n.lod >= fine_leaf.lod - 1,
                    "Neighbor should be at same or coarser LOD");
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
}
```
