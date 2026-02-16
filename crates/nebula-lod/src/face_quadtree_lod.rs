//! Per-face quadtree LOD controller that splits/merges nodes based on camera distance.

use glam::DVec3;
use nebula_cubesphere::{BoundingSphere, ChunkAddress, CubeFace, FaceQuadtree, QuadNode};
use nebula_math::WorldPosition;

use crate::LodThresholds;

/// Result of evaluating a node during quadtree traversal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LodAction {
    /// Keep the node as-is.
    Keep,
    /// Split the node into four children (increase detail).
    Split,
    /// Merge the node's children back into a single leaf (decrease detail).
    Merge,
}

/// Describes an active (leaf) chunk produced by the quadtree LOD update.
#[derive(Clone, Debug)]
pub struct LodChunkDescriptor {
    /// The chunk address on the cubesphere.
    pub address: ChunkAddress,
    /// LOD level (0 = finest detail).
    pub lod: u8,
    /// Bounding sphere for this chunk.
    pub bounding_sphere: BoundingSphere,
    /// Distance from camera to chunk center in mm.
    pub distance: f64,
}

/// Per-face quadtree LOD controller.
///
/// Manages a quadtree for one cube face, splitting and merging nodes
/// based on camera distance each frame.
pub struct FaceQuadtreeLod {
    /// The underlying quadtree.
    tree: FaceQuadtree,
    /// Maximum subdivision depth (LOD 0 = max depth, MAX_LOD = coarsest).
    max_depth: u8,
    /// LOD thresholds for split/merge decisions.
    thresholds: LodThresholds,
    /// Planet radius in mm (for bounding sphere computation).
    planet_radius: f64,
}

impl FaceQuadtreeLod {
    /// Create a new per-face quadtree LOD controller.
    ///
    /// - `face`: which cube face this quadtree covers
    /// - `max_depth`: maximum subdivision depth (typically 5-8 for planets)
    /// - `thresholds`: distance thresholds for LOD selection
    /// - `planet_radius`: planet radius in mm
    pub fn new(
        face: CubeFace,
        max_depth: u8,
        thresholds: LodThresholds,
        planet_radius: f64,
    ) -> Self {
        Self {
            tree: FaceQuadtree::new(face),
            max_depth,
            thresholds,
            planet_radius,
        }
    }

    /// Which face this quadtree covers.
    pub fn face(&self) -> CubeFace {
        self.tree.face
    }

    /// Access the underlying quadtree (read-only).
    pub fn tree(&self) -> &FaceQuadtree {
        &self.tree
    }

    /// Update the quadtree based on the camera position.
    ///
    /// Returns a list of active chunk descriptors (leaf nodes) after
    /// splitting/merging. The camera position is in world coordinates (mm).
    pub fn update(&mut self, camera_pos: &WorldPosition) -> Vec<LodChunkDescriptor> {
        let cam_dvec3 = DVec3::new(
            camera_pos.x as f64,
            camera_pos.y as f64,
            camera_pos.z as f64,
        );

        // Phase 1: recursive split/merge
        let max_depth = self.max_depth;
        let root_lod = self.tree.root.address().lod;
        Self::update_node(
            &mut self.tree.root,
            &cam_dvec3,
            &self.thresholds,
            self.planet_radius,
            max_depth,
            root_lod,
        );

        // Phase 2: balance constraint (max 1 LOD diff between neighbors)
        self.enforce_balance(&cam_dvec3);

        // Phase 3: collect active leaves
        let mut chunks = Vec::new();
        Self::collect_leaves(&self.tree.root, &cam_dvec3, self.planet_radius, &mut chunks);
        chunks
    }

    /// Recursively decide whether to split or merge each node.
    ///
    /// The quadtree maps `max_depth` levels of subdivision onto the threshold LOD range.
    /// - Depth 0 (root) → quadtree LOD = `max_depth` (coarsest)
    /// - Depth `max_depth` → quadtree LOD = 0 (finest)
    fn update_node(
        node: &mut QuadNode,
        cam: &DVec3,
        thresholds: &LodThresholds,
        planet_radius: f64,
        max_depth: u8,
        root_lod: u8,
    ) {
        let addr = node.address();
        let bs = BoundingSphere::from_chunk(&addr, planet_radius, 0.0, 0.0);
        let distance = (bs.center - *cam).length();

        // Convert distance to threshold units (thresholds are in meters, distance in mm)
        let distance_meters = distance / 1000.0;

        // Desired LOD from thresholds (0 = finest, max_lod = coarsest)
        let desired_lod = select_lod(thresholds, distance_meters);

        // Map node address LOD to quadtree-relative LOD:
        // depth_from_root = root_lod - addr.lod
        // quadtree_lod = max_depth - depth_from_root
        let depth_from_root = root_lod.saturating_sub(addr.lod);
        let quadtree_lod = max_depth.saturating_sub(depth_from_root);

        match node {
            QuadNode::Leaf { .. } => {
                // Split if node is too coarse (quadtree_lod > desired_lod)
                // and we haven't reached max depth
                if quadtree_lod > desired_lod && depth_from_root < max_depth && addr.lod > 0 {
                    node.subdivide();
                    // Recurse into new children
                    if let QuadNode::Branch { children, .. } = node {
                        for child in children.iter_mut() {
                            Self::update_node(
                                child,
                                cam,
                                thresholds,
                                planet_radius,
                                max_depth,
                                root_lod,
                            );
                        }
                    }
                }
            }
            QuadNode::Branch { children, .. } => {
                // Merge if node is already fine enough (quadtree_lod <= desired_lod)
                if quadtree_lod <= desired_lod {
                    node.merge();
                } else {
                    // Recurse into children
                    for child in children.iter_mut() {
                        Self::update_node(
                            child,
                            cam,
                            thresholds,
                            planet_radius,
                            max_depth,
                            root_lod,
                        );
                    }
                }
            }
        }
    }

    /// Enforce the balance constraint: no two adjacent leaves differ by more than 1 LOD level.
    fn enforce_balance(&mut self, cam: &DVec3) {
        // Iterate until stable
        for _ in 0..self.max_depth {
            let leaves = self.tree.root.all_leaves();
            let mut changed = false;

            // For each leaf, check if any neighbor is >1 LOD coarser, and force-split it
            for leaf_addr in &leaves {
                let neighbors = self.same_face_neighbors(leaf_addr);
                for neighbor_addr in neighbors {
                    // If neighbor is >1 LOD coarser than this leaf, force-split
                    if neighbor_addr.lod > leaf_addr.lod + 1 {
                        // Find and split the neighbor node
                        if Self::force_split_at(
                            &mut self.tree.root,
                            &neighbor_addr,
                            cam,
                            &self.thresholds,
                            self.planet_radius,
                        ) {
                            changed = true;
                        }
                    }
                }
            }

            if !changed {
                break;
            }
        }
    }

    /// Find the leaf node containing the given address and split it if it matches.
    fn force_split_at(
        node: &mut QuadNode,
        target: &ChunkAddress,
        _cam: &DVec3,
        _thresholds: &LodThresholds,
        _planet_radius: f64,
    ) -> bool {
        let addr = node.address();
        if addr == *target {
            if node.is_leaf() && addr.lod > 0 {
                node.subdivide();
                return true;
            }
            return false;
        }

        // If this node's region could contain the target, recurse
        if let QuadNode::Branch { children, .. } = node {
            for child in children.iter_mut() {
                if Self::force_split_at(child, target, _cam, _thresholds, _planet_radius) {
                    return true;
                }
            }
        }
        false
    }

    /// Get same-face neighbor leaf addresses for a given chunk address.
    fn same_face_neighbors(&self, addr: &ChunkAddress) -> Vec<ChunkAddress> {
        let grid_size = ChunkAddress::grid_size(addr.lod);
        let mut neighbors = Vec::new();

        // Check 4 edge-adjacent neighbors
        let offsets: [(i64, i64); 4] = [(-1, 0), (1, 0), (0, -1), (0, 1)];
        for (dx, dy) in offsets {
            let nx = addr.x as i64 + dx;
            let ny = addr.y as i64 + dy;
            if nx >= 0 && nx < grid_size as i64 && ny >= 0 && ny < grid_size as i64 {
                // Find the actual leaf at this position
                let u = (nx as f64 + 0.5) / grid_size as f64;
                let v = (ny as f64 + 0.5) / grid_size as f64;
                let leaf = self.tree.root.find_leaf(u, v);
                if leaf != *addr {
                    neighbors.push(leaf);
                }
            }
        }

        neighbors
    }

    /// Collect all leaf nodes as `LodChunkDescriptor`s.
    fn collect_leaves(
        node: &QuadNode,
        cam: &DVec3,
        planet_radius: f64,
        out: &mut Vec<LodChunkDescriptor>,
    ) {
        match node {
            QuadNode::Leaf { address } => {
                let bs = BoundingSphere::from_chunk(address, planet_radius, 0.0, 0.0);
                let distance = (bs.center - *cam).length();
                out.push(LodChunkDescriptor {
                    address: *address,
                    lod: address.lod,
                    bounding_sphere: bs,
                    distance,
                });
            }
            QuadNode::Branch { children, .. } => {
                for child in children.iter() {
                    Self::collect_leaves(child, cam, planet_radius, out);
                }
            }
        }
    }

    /// Get all current leaf addresses for neighbor queries.
    pub fn leaf_neighbors(&self, desc: &LodChunkDescriptor) -> Vec<LodChunkDescriptor> {
        let cam = desc.bounding_sphere.center; // approximate
        let neighbor_addrs = self.same_face_neighbors(&desc.address);
        neighbor_addrs
            .iter()
            .map(|addr| {
                let bs = BoundingSphere::from_chunk(addr, self.planet_radius, 0.0, 0.0);
                let distance = (bs.center - cam).length();
                LodChunkDescriptor {
                    address: *addr,
                    lod: addr.lod,
                    bounding_sphere: bs,
                    distance,
                }
            })
            .collect()
    }

    /// Reset the quadtree to a single root leaf.
    pub fn reset(&mut self) {
        self.tree = FaceQuadtree::new(self.tree.face);
    }
}

/// Select LOD level from thresholds (mirrors `LodSelector::select_lod`).
fn select_lod(thresholds: &LodThresholds, distance: f64) -> u8 {
    for (i, &threshold) in thresholds.thresholds().iter().enumerate() {
        if distance < threshold {
            return i as u8;
        }
    }
    thresholds.max_lod()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLANET_RADIUS: f64 = 6_371_000_000.0; // Earth-like, mm

    fn make_test_quadtree(max_depth: u8) -> FaceQuadtreeLod {
        FaceQuadtreeLod::new(
            CubeFace::PosY,
            max_depth,
            LodThresholds::default_planet(),
            PLANET_RADIUS,
        )
    }

    /// Camera at the center of a face should subdivide nodes near it deeply.
    #[test]
    fn test_camera_at_face_center_subdivides_deeply() {
        let mut qt = make_test_quadtree(5);
        // Camera on surface of +Y face (planet_radius mm up)
        let camera = WorldPosition::new(0, PLANET_RADIUS as i128, 0);
        let chunks = qt.update(&camera);

        // Should produce multiple chunks (root was split)
        assert!(
            chunks.len() > 1,
            "expected subdivision, got {} chunks",
            chunks.len()
        );

        // Closest chunk should have a low LOD (fine detail)
        let closest = chunks
            .iter()
            .min_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap())
            .unwrap();
        // The closest chunk should have been subdivided to a finer LOD than the root
        assert!(
            closest.lod < ChunkAddress::MAX_LOD,
            "closest chunk should be subdivided, got lod={}",
            closest.lod
        );
    }

    /// Camera very far away should keep the root node unsplit (single coarse chunk).
    #[test]
    fn test_camera_far_away_keeps_root_coarse() {
        let mut qt = make_test_quadtree(5);
        // Camera very far in space
        let camera = WorldPosition::new(0, 100_000_000_000_000, 0);
        let chunks = qt.update(&camera);

        // Should produce very few chunks (root-level leaf)
        assert!(
            chunks.len() <= 4,
            "expected at most 4 coarse chunks, got {}",
            chunks.len()
        );
    }

    /// Moving the camera toward a coarse node should trigger a split.
    #[test]
    fn test_moving_camera_triggers_split() {
        let mut qt = make_test_quadtree(5);

        // Start far away
        let far_camera = WorldPosition::new(0, 100_000_000_000_000, 0);
        let chunks_far = qt.update(&far_camera);
        let count_far = chunks_far.len();

        // Move close to surface
        let near_camera = WorldPosition::new(0, PLANET_RADIUS as i128, 0);
        let chunks_near = qt.update(&near_camera);
        let count_near = chunks_near.len();

        assert!(
            count_near > count_far,
            "closer camera should produce more chunks: near={count_near}, far={count_far}"
        );
    }

    /// Neighboring leaf nodes should never differ by more than 1 LOD level.
    #[test]
    fn test_quadtree_balance_max_one_lod_difference() {
        let mut qt = make_test_quadtree(5);
        // Camera offset on the surface to create LOD variation
        let camera = WorldPosition::new((PLANET_RADIUS * 0.1) as i128, PLANET_RADIUS as i128, 0);
        let chunks = qt.update(&camera);

        for chunk in &chunks {
            let neighbors = qt.leaf_neighbors(chunk);
            for neighbor in &neighbors {
                let lod_diff = (chunk.lod as i8 - neighbor.lod as i8).abs();
                assert!(
                    lod_diff <= 1,
                    "LOD difference between neighbors must be <= 1, got {} (lods: {}, {})",
                    lod_diff,
                    chunk.lod,
                    neighbor.lod
                );
            }
        }
    }

    /// The quadtree should produce valid chunk addresses.
    #[test]
    fn test_chunks_have_valid_addresses() {
        let mut qt = make_test_quadtree(4);
        let camera = WorldPosition::new(0, PLANET_RADIUS as i128, 0);
        let chunks = qt.update(&camera);

        for chunk in &chunks {
            assert_eq!(chunk.address.face, CubeFace::PosY);
            assert!(chunk.address.lod <= ChunkAddress::MAX_LOD);
            let grid = ChunkAddress::grid_size(chunk.address.lod);
            assert!(chunk.address.x < grid);
            assert!(chunk.address.y < grid);
        }
    }

    /// Reset should return to a single root leaf.
    #[test]
    fn test_reset() {
        let mut qt = make_test_quadtree(5);
        let camera = WorldPosition::new(0, PLANET_RADIUS as i128, 0);
        qt.update(&camera);
        qt.reset();
        let chunks = qt.update(&WorldPosition::new(0, 100_000_000_000_000, 0));
        assert_eq!(chunks.len(), 1);
    }

    /// LodAction enum should be constructable.
    #[test]
    fn test_lod_action_variants() {
        let keep = LodAction::Keep;
        let split = LodAction::Split;
        let merge = LodAction::Merge;
        assert_eq!(keep, LodAction::Keep);
        assert_eq!(split, LodAction::Split);
        assert_eq!(merge, LodAction::Merge);
    }
}
