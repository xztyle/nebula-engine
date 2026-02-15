# Quadtree Subdivision

## Problem

Each cube face must be subdivided adaptively based on the camera's distance: nearby regions need high-detail (low LOD number) chunks, while distant regions should use coarser chunks. A flat grid would waste memory and GPU time on distant terrain that the player cannot see in detail. The natural structure for adaptive 2D subdivision is a quadtree, where each node can be recursively split into four children or merged back into a single leaf. The quadtree must support efficient queries ("which leaf contains this UV point?"), enumeration ("give me all leaves at a specific LOD"), and dynamic restructuring as the camera moves.

## Solution

Implement the per-face quadtree in the `nebula_cubesphere` crate.

### QuadNode

```rust
/// A node in the per-face quadtree.
///
/// Each node represents a rectangular region of a cube face. It is either
/// a leaf (rendered as a single chunk) or an interior node with exactly
/// 4 children.
pub enum QuadNode {
    /// A leaf node — this region is rendered as a single chunk.
    Leaf {
        address: ChunkAddress,
    },
    /// An interior node — subdivided into 4 children.
    /// Children are ordered: [bottom-left, bottom-right, top-left, top-right]
    /// i.e., [(x, y), (x+1, y), (x, y+1), (x+1, y+1)] at the child LOD.
    Branch {
        address: ChunkAddress,
        children: Box<[QuadNode; 4]>,
    },
}
```

### FaceQuadtree

```rust
/// The quadtree for one cube face.
///
/// The root node covers the entire face (LOD = MAX_LOD, x=0, y=0).
pub struct FaceQuadtree {
    pub face: CubeFace,
    pub root: QuadNode,
}

impl FaceQuadtree {
    /// Create a new quadtree with a single leaf covering the entire face.
    pub fn new(face: CubeFace) -> Self {
        Self {
            face,
            root: QuadNode::Leaf {
                address: ChunkAddress::new(face, ChunkAddress::MAX_LOD, 0, 0),
            },
        }
    }
}
```

### Core Operations

```rust
impl QuadNode {
    /// Subdivide this leaf into 4 children at the next finer LOD.
    /// Panics if this node is already a branch or at LOD 0.
    pub fn subdivide(&mut self) {
        let address = match self {
            QuadNode::Leaf { address } => *address,
            QuadNode::Branch { .. } => panic!("Cannot subdivide a branch node"),
        };
        let children = address.children()
            .expect("Cannot subdivide at LOD 0");

        *self = QuadNode::Branch {
            address,
            children: Box::new(children.map(|addr| QuadNode::Leaf { address: addr })),
        };
    }

    /// Merge this branch back into a single leaf, discarding all children.
    /// No-op if already a leaf.
    pub fn merge(&mut self) {
        if let QuadNode::Branch { address, .. } = self {
            *self = QuadNode::Leaf { address: *address };
        }
    }

    /// Find the leaf node that contains the given (u, v) point.
    /// Returns the ChunkAddress of that leaf.
    pub fn find_leaf(&self, u: f64, v: f64) -> ChunkAddress {
        match self {
            QuadNode::Leaf { address } => *address,
            QuadNode::Branch { address, children } => {
                let (u_min, v_min, u_max, v_max) = address.uv_bounds();
                let u_mid = (u_min + u_max) * 0.5;
                let v_mid = (v_min + v_max) * 0.5;

                let idx = match (u >= u_mid, v >= v_mid) {
                    (false, false) => 0, // bottom-left
                    (true, false) => 1,  // bottom-right
                    (false, true) => 2,  // top-left
                    (true, true) => 3,   // top-right
                };
                children[idx].find_leaf(u, v)
            }
        }
    }

    /// Collect all leaf addresses at a specific LOD level.
    pub fn leaves_at_lod(&self, target_lod: u8) -> Vec<ChunkAddress> {
        let mut result = Vec::new();
        self.collect_leaves_at_lod(target_lod, &mut result);
        result
    }

    fn collect_leaves_at_lod(&self, target_lod: u8, result: &mut Vec<ChunkAddress>) {
        match self {
            QuadNode::Leaf { address } => {
                if address.lod == target_lod {
                    result.push(*address);
                }
            }
            QuadNode::Branch { children, .. } => {
                for child in children.iter() {
                    child.collect_leaves_at_lod(target_lod, result);
                }
            }
        }
    }

    /// Collect all current leaf addresses regardless of LOD.
    pub fn all_leaves(&self) -> Vec<ChunkAddress> {
        let mut result = Vec::new();
        self.collect_all_leaves(&mut result);
        result
    }

    fn collect_all_leaves(&self, result: &mut Vec<ChunkAddress>) {
        match self {
            QuadNode::Leaf { address } => result.push(*address),
            QuadNode::Branch { children, .. } => {
                for child in children.iter() {
                    child.collect_all_leaves(result);
                }
            }
        }
    }

    /// Returns true if this node is a leaf.
    pub fn is_leaf(&self) -> bool {
        matches!(self, QuadNode::Leaf { .. })
    }

    /// Returns the address of this node (leaf or branch).
    pub fn address(&self) -> ChunkAddress {
        match self {
            QuadNode::Leaf { address } | QuadNode::Branch { address, .. } => *address,
        }
    }
}
```

### Design Constraints

- The quadtree is per-face. A full planet has 6 `FaceQuadtree` instances.
- Children are heap-allocated (`Box<[QuadNode; 4]>`) to keep the enum size manageable. A leaf is just a `ChunkAddress` (small).
- `subdivide` and `merge` mutate in place to avoid allocating new trees on every frame.
- The quadtree does not own chunk data (meshes, voxels). It is purely a spatial index that tells the chunk loader which addresses to load.
- Maximum depth is `MAX_LOD` levels (20), which means at most ~4^20 leaves if fully subdivided — but in practice the LOD system limits this to a few thousand active leaves per face.

## Outcome

The `nebula_cubesphere` crate exports `QuadNode`, `FaceQuadtree`, and the associated methods for subdivision, merging, leaf finding, and enumeration. The LOD system uses these to decide which chunks to load and unload as the camera moves. Running `cargo test -p nebula_cubesphere` passes all quadtree tests.

## Demo Integration

**Demo crate:** `nebula-demo`

The chunk grid is now hierarchical. Chunks near the camera subdivide into 4 children, and those children subdivide further — coarse far away, fine up close.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| *(none)* | — | Pure `std` only; quadtree is built with standard Rust enums and `Box` |

The crate uses Rust edition 2024. No external dependencies beyond the workspace crates (`nebula_cubesphere` itself for `ChunkAddress` and `CubeFace`).

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_root_covers_entire_face() {
        let tree = FaceQuadtree::new(CubeFace::PosX);
        let addr = tree.root.address();
        let (u_min, v_min, u_max, v_max) = addr.uv_bounds();
        assert!((u_min - 0.0).abs() < 1e-12);
        assert!((v_min - 0.0).abs() < 1e-12);
        assert!((u_max - 1.0).abs() < 1e-12);
        assert!((v_max - 1.0).abs() < 1e-12);
    }

    #[test]
    fn test_subdivide_produces_4_children() {
        let mut tree = FaceQuadtree::new(CubeFace::PosY);
        tree.root.subdivide();
        match &tree.root {
            QuadNode::Branch { children, .. } => {
                assert_eq!(children.len(), 4);
                for child in children.iter() {
                    assert!(child.is_leaf());
                }
            }
            _ => panic!("Root should be a branch after subdivide"),
        }
    }

    #[test]
    fn test_children_cover_parent_area_exactly() {
        let mut tree = FaceQuadtree::new(CubeFace::NegZ);
        let parent_bounds = tree.root.address().uv_bounds();
        tree.root.subdivide();

        let leaves = tree.root.all_leaves();
        assert_eq!(leaves.len(), 4);

        let mut u_min = f64::MAX;
        let mut v_min = f64::MAX;
        let mut u_max = f64::MIN;
        let mut v_max = f64::MIN;
        for leaf in &leaves {
            let (u0, v0, u1, v1) = leaf.uv_bounds();
            u_min = u_min.min(u0);
            v_min = v_min.min(v0);
            u_max = u_max.max(u1);
            v_max = v_max.max(v1);
        }
        assert!((u_min - parent_bounds.0).abs() < 1e-12);
        assert!((v_min - parent_bounds.1).abs() < 1e-12);
        assert!((u_max - parent_bounds.2).abs() < 1e-12);
        assert!((v_max - parent_bounds.3).abs() < 1e-12);
    }

    #[test]
    fn test_find_leaf_returns_correct_address() {
        let mut tree = FaceQuadtree::new(CubeFace::PosZ);
        tree.root.subdivide();

        // Query a point in the bottom-left quadrant
        let leaf = tree.root.find_leaf(0.1, 0.1);
        let (u_min, v_min, u_max, v_max) = leaf.uv_bounds();
        assert!(u_min <= 0.1 && 0.1 < u_max);
        assert!(v_min <= 0.1 && 0.1 < v_max);

        // Query a point in the top-right quadrant
        let leaf = tree.root.find_leaf(0.9, 0.9);
        let (u_min, v_min, u_max, v_max) = leaf.uv_bounds();
        assert!(u_min <= 0.9 && 0.9 < u_max);
        assert!(v_min <= 0.9 && 0.9 < v_max);
    }

    #[test]
    fn test_merge_reverses_subdivide() {
        let mut tree = FaceQuadtree::new(CubeFace::NegX);
        let original_addr = tree.root.address();
        assert!(tree.root.is_leaf());

        tree.root.subdivide();
        assert!(!tree.root.is_leaf());

        tree.root.merge();
        assert!(tree.root.is_leaf());
        assert_eq!(tree.root.address(), original_addr);
    }

    #[test]
    fn test_leaves_at_lod() {
        let mut tree = FaceQuadtree::new(CubeFace::PosX);
        tree.root.subdivide(); // root (LOD MAX) -> 4 children (LOD MAX-1)

        // Subdivide the first child further
        if let QuadNode::Branch { children, .. } = &mut tree.root {
            children[0].subdivide(); // -> 4 grandchildren (LOD MAX-2)
        }

        let leaves_max_minus_1 = tree.root.leaves_at_lod(ChunkAddress::MAX_LOD - 1);
        assert_eq!(leaves_max_minus_1.len(), 3); // 3 unsplit children

        let leaves_max_minus_2 = tree.root.leaves_at_lod(ChunkAddress::MAX_LOD - 2);
        assert_eq!(leaves_max_minus_2.len(), 4); // 4 grandchildren from split child
    }

    #[test]
    fn test_deep_subdivision() {
        let mut tree = FaceQuadtree::new(CubeFace::PosY);
        // Subdivide 3 levels deep, always following the first child
        tree.root.subdivide();
        if let QuadNode::Branch { children, .. } = &mut tree.root {
            children[0].subdivide();
            if let QuadNode::Branch { children: gc, .. } = &mut children[0] {
                gc[0].subdivide();
            }
        }

        let all_leaves = tree.root.all_leaves();
        // 3 leaves from first split + 3 from second + 4 from third = 10
        assert_eq!(all_leaves.len(), 3 + 3 + 4);
    }

    #[test]
    fn test_find_leaf_in_unsubdivided_tree() {
        let tree = FaceQuadtree::new(CubeFace::NegY);
        let leaf = tree.root.find_leaf(0.5, 0.5);
        assert_eq!(leaf, tree.root.address());
    }
}
```
