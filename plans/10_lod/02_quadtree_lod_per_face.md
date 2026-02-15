# Quadtree LOD Per Face

## Problem

Each face of the cubesphere planet is subdivided into a quadtree of chunks. When rendering the planet, the engine must decide how deeply to subdivide each quadtree node — nodes near the camera need full subdivision to produce fine-grained chunks, while distant nodes should remain coarse to avoid generating millions of unnecessary chunks. The quadtree LOD decision must happen every frame as the camera moves, must be efficient enough to traverse thousands of nodes without stalling the frame, and must ensure that neighboring nodes never differ by more than one LOD level to avoid catastrophic seam artifacts at boundaries. Simply selecting LOD per-chunk (as in story 01) is not enough — the quadtree structure itself must adapt, with nodes splitting and merging dynamically as the camera moves across the planet surface or rises into orbit.

## Solution

Implement per-face quadtree LOD traversal in the `nebula_lod` crate. Each cube face maintains a quadtree where nodes can be in one of two states: **leaf** (rendered as a single chunk at that node's LOD level) or **split** (subdivided into four children). Every frame, the LOD system walks each face's quadtree and decides whether each node should be split or merged based on camera distance.

### Data Structures

```rust
/// A node in the per-face LOD quadtree.
#[derive(Debug)]
pub enum QuadNode {
    /// A leaf node representing a single renderable chunk at this quadtree depth.
    Leaf {
        /// The LOD level of this node (derived from quadtree depth).
        lod: u8,
        /// Bounding sphere center and radius for distance culling.
        bounding_sphere: BoundingSphere,
    },
    /// A split node whose four children are further subdivided.
    Split {
        children: Box<[QuadNode; 4]>,
        bounding_sphere: BoundingSphere,
    },
}

/// Which quadrant a child occupies within its parent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Quadrant {
    TopLeft = 0,
    TopRight = 1,
    BottomLeft = 2,
    BottomRight = 3,
}

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

/// Per-face quadtree LOD controller.
pub struct FaceQuadtreeLod {
    /// Root node of the quadtree for this face.
    root: QuadNode,
    /// Which cube face this quadtree represents.
    face: CubeFace,
    /// Maximum subdivision depth (LOD 0 = max depth).
    max_depth: u8,
    /// LOD thresholds used for split/merge decisions.
    thresholds: LodThresholds,
}
```

### Traversal Algorithm

Each frame, the LOD system calls `update()` on each face's quadtree. The update performs a recursive depth-first traversal:

```rust
impl FaceQuadtreeLod {
    /// Update the quadtree based on the current camera position.
    /// Returns the list of chunks (leaf nodes) that should be active after this update.
    pub fn update(&mut self, camera_pos: &WorldPosition) -> Vec<LodChunkDescriptor> {
        let mut active_chunks = Vec::new();
        self.update_node(&mut self.root, 0, camera_pos, &mut active_chunks);
        active_chunks
    }

    fn evaluate_node(
        &self,
        node: &QuadNode,
        depth: u8,
        camera_pos: &WorldPosition,
    ) -> LodAction {
        let bounding_sphere = node.bounding_sphere();
        let distance = bounding_sphere.distance_to(camera_pos);

        let desired_lod = self.thresholds.select_lod(distance);
        let node_lod = self.max_depth - depth;

        if node_lod > desired_lod && depth < self.max_depth {
            // Node is too coarse, should split for more detail
            LodAction::Split
        } else if node_lod < desired_lod {
            // Node is too detailed, should merge
            LodAction::Merge
        } else {
            LodAction::Keep
        }
    }
}
```

For each node:
1. Compute the distance from the camera to the node's bounding sphere.
2. Compare against LOD thresholds to determine the desired LOD level.
3. If the node's current LOD is coarser than desired and it is a leaf, **split** it into four children.
4. If the node's current LOD is finer than desired and all four children are leaves, **merge** them back into a single leaf.
5. If the node is already at the correct LOD, **keep** it.

### Neighbor Balance Constraint

After the split/merge pass, a second pass enforces the **max-1-LOD-difference** constraint between neighboring nodes. For every leaf node, check its four edge-adjacent neighbors (within the same face, or across face boundaries for edge nodes). If any neighbor is more than one LOD level coarser, force-split the coarser neighbor. This constraint pass iterates until stable (typically 1-2 iterations).

```rust
pub fn enforce_balance_constraint(&mut self) {
    let mut changed = true;
    while changed {
        changed = false;
        // Walk all leaves, check neighbors, force-split if LOD diff > 1
        changed |= self.balance_pass();
    }
}
```

### Visibility Integration

Before evaluating a node for split/merge, the traversal checks whether the node's bounding sphere is inside the camera's view frustum. Nodes entirely outside the frustum are not subdivided further — they remain as coarse leaves regardless of distance. This prevents the engine from wasting subdivision budget on terrain behind the camera.

## Outcome

The `nebula_lod` crate exports `FaceQuadtreeLod`, `QuadNode`, `LodAction`, and `LodChunkDescriptor`. Each frame, calling `update()` on each face's quadtree produces a list of active chunk descriptors that the chunk loading system uses to load, generate, and render the correct set of chunks. The quadtree guarantees that neighboring chunks differ by at most one LOD level. Running `cargo test -p nebula_lod` passes all quadtree LOD tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Each cube face subdivides into a quadtree. Chunks near the camera are small and detailed; distant chunks are large and coarse. The debug overlay shows the quadtree structure.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `nebula_math` | workspace | `WorldPosition`, `BoundingSphere`, distance calculations |
| `nebula_cubesphere` | workspace | `CubeFace` enum, face-local coordinate types |
| `nebula_lod` | workspace (self) | `LodThresholds` from story 01 |

No external crates required. The quadtree traversal is pure recursive logic. The crate uses Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_quadtree(max_depth: u8) -> FaceQuadtreeLod {
        FaceQuadtreeLod::new(
            CubeFace::PosY,
            max_depth,
            LodThresholds::default_planet(),
        )
    }

    /// Camera at the center of a face should subdivide nodes near it deeply.
    #[test]
    fn test_camera_at_face_center_subdivides_deeply() {
        let mut qt = make_test_quadtree(5);
        let camera = WorldPosition::new(0, 1_000_000, 0); // on surface of +Y face
        let chunks = qt.update(&camera);

        // Closest chunks should be at LOD 0 (max depth)
        let closest = chunks.iter().min_by(|a, b| {
            a.distance_to(&camera).partial_cmp(&b.distance_to(&camera)).unwrap()
        }).unwrap();
        assert_eq!(closest.lod, 0);
    }

    /// Camera very far away should keep the root node unsplit (single coarse chunk).
    #[test]
    fn test_camera_far_away_keeps_root_coarse() {
        let mut qt = make_test_quadtree(5);
        let camera = WorldPosition::new(0, 100_000_000, 0); // far in space
        let chunks = qt.update(&camera);

        // Should produce very few chunks (root-level leaves)
        assert!(chunks.len() <= 4, "expected at most 4 coarse chunks, got {}", chunks.len());
        for chunk in &chunks {
            assert!(chunk.lod >= 3, "expected coarse LOD, got {}", chunk.lod);
        }
    }

    /// Moving the camera toward a coarse node should trigger a split.
    #[test]
    fn test_moving_camera_triggers_split() {
        let mut qt = make_test_quadtree(5);

        // Start far away
        let far_camera = WorldPosition::new(0, 100_000_000, 0);
        let chunks_far = qt.update(&far_camera);
        let count_far = chunks_far.len();

        // Move close to surface
        let near_camera = WorldPosition::new(0, 1_000_000, 0);
        let chunks_near = qt.update(&near_camera);
        let count_near = chunks_near.len();

        assert!(count_near > count_far, "closer camera should produce more chunks");
    }

    /// Nodes entirely outside the view frustum should not be subdivided.
    #[test]
    fn test_only_visible_nodes_subdivided() {
        let mut qt = make_test_quadtree(5);
        let camera = WorldPosition::new(0, 1_000_000, 0);
        let frustum = ViewFrustum::looking_toward(camera, Vec3::new(1.0, 0.0, 0.0));

        let chunks = qt.update_with_frustum(&camera, &frustum);

        // Chunks behind the camera should not be subdivided deeply
        for chunk in &chunks {
            if !frustum.contains_sphere(&chunk.bounding_sphere) {
                assert!(chunk.lod >= 2, "off-screen chunk should be coarse");
            }
        }
    }

    /// Neighboring leaf nodes should never differ by more than 1 LOD level.
    #[test]
    fn test_quadtree_balance_max_one_lod_difference() {
        let mut qt = make_test_quadtree(5);
        let camera = WorldPosition::new(500_000, 1_000_000, 0);
        let chunks = qt.update(&camera);

        for chunk in &chunks {
            for neighbor in qt.leaf_neighbors(chunk) {
                let lod_diff = (chunk.lod as i8 - neighbor.lod as i8).abs();
                assert!(
                    lod_diff <= 1,
                    "LOD difference between neighbors must be <= 1, got {} (lods: {}, {})",
                    lod_diff, chunk.lod, neighbor.lod
                );
            }
        }
    }
}
```
