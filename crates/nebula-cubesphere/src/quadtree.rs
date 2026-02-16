//! Per-face quadtree for adaptive LOD subdivision.

use crate::{ChunkAddress, CubeFace};

/// A node in the per-face quadtree.
///
/// Each node represents a rectangular region of a cube face. It is either
/// a leaf (rendered as a single chunk) or an interior node with exactly
/// 4 children.
#[derive(Debug, Clone)]
pub enum QuadNode {
    /// A leaf node — this region is rendered as a single chunk.
    Leaf {
        /// The chunk address for this leaf region.
        address: ChunkAddress,
    },
    /// An interior node — subdivided into 4 children.
    /// Children are ordered: \[bottom-left, bottom-right, top-left, top-right\]
    /// i.e., \[(x, y), (x+1, y), (x, y+1), (x+1, y+1)\] at the child LOD.
    Branch {
        /// The chunk address for this branch region.
        address: ChunkAddress,
        /// The four child nodes.
        children: Box<[QuadNode; 4]>,
    },
}

impl QuadNode {
    /// Subdivide this leaf into 4 children at the next finer LOD.
    ///
    /// # Panics
    ///
    /// Panics if this node is already a branch or at LOD 0.
    pub fn subdivide(&mut self) {
        let address = match self {
            QuadNode::Leaf { address } => *address,
            QuadNode::Branch { .. } => panic!("Cannot subdivide a branch node"),
        };
        let children = address.children().expect("Cannot subdivide at LOD 0");

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
    /// Returns the `ChunkAddress` of that leaf.
    #[must_use]
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
    #[must_use]
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
    #[must_use]
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
    #[must_use]
    pub fn is_leaf(&self) -> bool {
        matches!(self, QuadNode::Leaf { .. })
    }

    /// Returns the address of this node (leaf or branch).
    #[must_use]
    pub fn address(&self) -> ChunkAddress {
        match self {
            QuadNode::Leaf { address } | QuadNode::Branch { address, .. } => *address,
        }
    }
}

/// The quadtree for one cube face.
///
/// The root node covers the entire face (LOD = `MAX_LOD`, x=0, y=0).
#[derive(Debug, Clone)]
pub struct FaceQuadtree {
    /// Which cube face this quadtree covers.
    pub face: CubeFace,
    /// The root node of the quadtree.
    pub root: QuadNode,
}

impl FaceQuadtree {
    /// Create a new quadtree with a single leaf covering the entire face.
    #[must_use]
    pub fn new(face: CubeFace) -> Self {
        Self {
            face,
            root: QuadNode::Leaf {
                address: ChunkAddress::new(face, ChunkAddress::MAX_LOD, 0, 0),
            },
        }
    }
}

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

        let leaf = tree.root.find_leaf(0.1, 0.1);
        let (u_min, v_min, u_max, v_max) = leaf.uv_bounds();
        assert!(u_min <= 0.1 && 0.1 < u_max);
        assert!(v_min <= 0.1 && 0.1 < v_max);

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
        tree.root.subdivide();

        if let QuadNode::Branch { children, .. } = &mut tree.root {
            children[0].subdivide();
        }

        let leaves_max_minus_1 = tree.root.leaves_at_lod(ChunkAddress::MAX_LOD - 1);
        assert_eq!(leaves_max_minus_1.len(), 3);

        let leaves_max_minus_2 = tree.root.leaves_at_lod(ChunkAddress::MAX_LOD - 2);
        assert_eq!(leaves_max_minus_2.len(), 4);
    }

    #[test]
    fn test_deep_subdivision() {
        let mut tree = FaceQuadtree::new(CubeFace::PosY);
        tree.root.subdivide();
        if let QuadNode::Branch { children, .. } = &mut tree.root {
            children[0].subdivide();
            if let QuadNode::Branch { children: gc, .. } = &mut children[0] {
                gc[0].subdivide();
            }
        }

        let all_leaves = tree.root.all_leaves();
        assert_eq!(all_leaves.len(), 3 + 3 + 4);
    }

    #[test]
    fn test_find_leaf_in_unsubdivided_tree() {
        let tree = FaceQuadtree::new(CubeFace::NegY);
        let leaf = tree.root.find_leaf(0.5, 0.5);
        assert_eq!(leaf, tree.root.address());
    }
}
