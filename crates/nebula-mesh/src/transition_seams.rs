//! LOD transition seam elimination: edge vertex constraining and skirt geometry.
//!
//! When adjacent chunks have different LOD levels, their boundary vertices don't
//! align, producing visible cracks (T-junctions). This module provides:
//!
//! 1. **Edge vertex constraining** — snaps higher-LOD boundary vertices to
//!    interpolated positions on the lower-LOD grid, eliminating T-junctions.
//! 2. **Skirt geometry** — adds short face strips along chunk boundaries that
//!    extend inward (toward planet center) to hide any remaining sub-pixel gaps.

use crate::face_direction::FaceDirection;
use crate::packed::{ChunkVertex, PackedChunkMesh};

/// Describes the LOD relationship between a chunk and its neighbor on one face.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NeighborLodRelation {
    /// Same LOD level — no constraining needed.
    Same,
    /// This chunk has higher resolution than the neighbor.
    /// The edge vertices of this chunk must be constrained downward.
    HigherThanNeighbor {
        /// The LOD difference (always positive). Typically 1.
        lod_diff: u8,
    },
    /// This chunk has lower resolution than the neighbor.
    /// The neighbor will constrain its vertices; this chunk does nothing special.
    LowerThanNeighbor,
}

/// Describes the LOD context for meshing a chunk, including neighbor relationships.
#[derive(Clone, Debug)]
pub struct ChunkLodContext {
    /// LOD level of this chunk.
    pub lod: u8,
    /// Neighbor LOD relationship for each of the 6 faces.
    /// Indexed by [`FaceDirection`] discriminant: +X, -X, +Y, -Y, +Z, -Z.
    pub neighbors: [NeighborLodRelation; 6],
}

impl ChunkLodContext {
    /// Create a context where all neighbors share the same LOD level.
    pub fn uniform(lod: u8) -> Self {
        Self {
            lod,
            neighbors: [NeighborLodRelation::Same; 6],
        }
    }

    /// Create a context from a center LOD and an array of neighbor LOD levels.
    /// `None` means the neighbor is not loaded (treated as same LOD).
    pub fn from_neighbor_lods(center_lod: u8, neighbor_lods: [Option<u8>; 6]) -> Self {
        let mut neighbors = [NeighborLodRelation::Same; 6];
        for (i, opt) in neighbor_lods.iter().enumerate() {
            if let Some(n_lod) = opt {
                neighbors[i] = if *n_lod > center_lod {
                    NeighborLodRelation::HigherThanNeighbor {
                        lod_diff: n_lod - center_lod,
                    }
                } else if *n_lod < center_lod {
                    NeighborLodRelation::LowerThanNeighbor
                } else {
                    NeighborLodRelation::Same
                };
            }
        }
        Self {
            lod: center_lod,
            neighbors,
        }
    }

    /// Check if a specific face needs edge constraining (neighbor is coarser).
    pub fn needs_constraining(&self, direction: FaceDirection) -> Option<u8> {
        match self.neighbors[direction as usize] {
            NeighborLodRelation::HigherThanNeighbor { lod_diff } => Some(lod_diff),
            _ => None,
        }
    }

    /// Check if a specific face has any LOD difference (needs skirt).
    pub fn has_lod_difference(&self, direction: FaceDirection) -> bool {
        self.neighbors[direction as usize] != NeighborLodRelation::Same
    }
}

/// Returns the two tangential axis indices for a face direction.
fn tangential_axes(direction: FaceDirection) -> (usize, usize) {
    let (_layer, u, v) = direction.sweep_axes();
    (u, v)
}

/// Returns `true` if the vertex sits on the boundary face for the given direction.
fn is_on_boundary(vertex: &ChunkVertex, direction: FaceDirection, chunk_size: usize) -> bool {
    let (layer_axis, _, _) = direction.sweep_axes();
    let boundary = match direction {
        FaceDirection::PosX | FaceDirection::PosY | FaceDirection::PosZ => chunk_size as u8,
        FaceDirection::NegX | FaceDirection::NegY | FaceDirection::NegZ => 0,
    };
    vertex.position[layer_axis] == boundary
}

/// Constrain edge vertices of a high-res chunk to match a low-res neighbor.
///
/// For each boundary vertex on the specified `edge`:
/// 1. Every `2^lod_diff` vertices in the high-res grid maps to one vertex
///    in the low-res grid.
/// 2. Intermediate vertices are snapped to the interpolated position between
///    the two nearest low-res-aligned vertices.
/// 3. This eliminates T-junctions by ensuring the high-res edge matches
///    the low-res edge's tessellation.
///
/// `chunk_size` is the number of voxels per axis (e.g. 32).
pub fn constrain_edge_vertices(
    mesh: &mut PackedChunkMesh,
    edge: FaceDirection,
    lod_diff: u8,
    chunk_size: usize,
) {
    if lod_diff == 0 {
        return;
    }
    let step = 1usize << lod_diff;
    let (u_idx, v_idx) = tangential_axes(edge);

    for vertex in &mut mesh.vertices {
        if !is_on_boundary(vertex, edge, chunk_size) {
            continue;
        }

        let u = vertex.position[u_idx] as usize;
        let v = vertex.position[v_idx] as usize;

        // Snap to nearest lower-LOD grid point via linear interpolation.
        // For integer coordinates this is equivalent to rounding to nearest step.
        let snapped_u = ((u + step / 2) / step) * step;
        let snapped_v = ((v + step / 2) / step) * step;

        vertex.position[u_idx] = snapped_u.min(chunk_size) as u8;
        vertex.position[v_idx] = snapped_v.min(chunk_size) as u8;
    }
}

/// Generate skirt geometry along a chunk boundary edge.
///
/// The skirt extends `skirt_depth` voxel units below the surface (toward
/// the planet center). These thin strips are invisible under normal viewing
/// but cover any remaining hairline cracks from floating-point imprecision
/// or cubesphere curvature.
///
/// Returns the number of triangles added.
pub fn generate_skirt(
    mesh: &mut PackedChunkMesh,
    edge: FaceDirection,
    skirt_depth: u8,
    chunk_size: usize,
) -> usize {
    if skirt_depth == 0 {
        return 0;
    }

    let (layer_axis, u_axis, v_axis) = edge.sweep_axes();

    let boundary = match edge {
        FaceDirection::PosX | FaceDirection::PosY | FaceDirection::PosZ => chunk_size as u8,
        FaceDirection::NegX | FaceDirection::NegY | FaceDirection::NegZ => 0,
    };

    // Compute inward offset: move boundary toward center of chunk.
    let inward_offset: i8 = match edge {
        FaceDirection::PosX | FaceDirection::PosY | FaceDirection::PosZ => -(skirt_depth as i8),
        FaceDirection::NegX | FaceDirection::NegY | FaceDirection::NegZ => skirt_depth as i8,
    };

    let lowered_boundary =
        (boundary as i16 + inward_offset as i16).clamp(0, chunk_size as i16) as u8;

    let mut triangles_added = 0;

    // Walk along u-axis, emitting quads for each edge segment.
    for u_pos in 0..chunk_size as u8 {
        // Top-surface vertices at (u_pos, 0) and (u_pos+1, 0) along the boundary.
        let mut v0 = [0u8; 3];
        v0[layer_axis] = boundary;
        v0[u_axis] = u_pos;
        v0[v_axis] = 0;

        let mut v1 = [0u8; 3];
        v1[layer_axis] = boundary;
        v1[u_axis] = u_pos + 1;
        v1[v_axis] = 0;

        // Lowered vertices (skirt bottom).
        let mut v0_low = v0;
        v0_low[layer_axis] = lowered_boundary;

        let mut v1_low = v1;
        v1_low[layer_axis] = lowered_boundary;

        let cv0 = ChunkVertex::new(v0, edge, 0, 0, [u_pos, 0]);
        let cv1 = ChunkVertex::new(v1, edge, 0, 0, [u_pos + 1, 0]);
        let cv0_low = ChunkVertex::new(v0_low, edge, 0, 0, [u_pos, 0]);
        let cv1_low = ChunkVertex::new(v1_low, edge, 0, 0, [u_pos + 1, 0]);

        let base = mesh.vertices.len() as u32;
        mesh.vertices
            .extend_from_slice(&[cv0, cv1, cv0_low, cv1_low]);

        // Two triangles forming a quad.
        mesh.indices.extend_from_slice(&[base, base + 1, base + 2]);
        mesh.indices
            .extend_from_slice(&[base + 1, base + 3, base + 2]);
        triangles_added += 2;
    }

    // Walk along v-axis too, for full coverage.
    for v_pos in 0..chunk_size as u8 {
        let mut v0 = [0u8; 3];
        v0[layer_axis] = boundary;
        v0[u_axis] = 0;
        v0[v_axis] = v_pos;

        let mut v1 = [0u8; 3];
        v1[layer_axis] = boundary;
        v1[u_axis] = 0;
        v1[v_axis] = v_pos + 1;

        let mut v0_low = v0;
        v0_low[layer_axis] = lowered_boundary;

        let mut v1_low = v1;
        v1_low[layer_axis] = lowered_boundary;

        let cv0 = ChunkVertex::new(v0, edge, 0, 0, [0, v_pos]);
        let cv1 = ChunkVertex::new(v1, edge, 0, 0, [0, v_pos + 1]);
        let cv0_low = ChunkVertex::new(v0_low, edge, 0, 0, [0, v_pos]);
        let cv1_low = ChunkVertex::new(v1_low, edge, 0, 0, [0, v_pos + 1]);

        let base = mesh.vertices.len() as u32;
        mesh.vertices
            .extend_from_slice(&[cv0, cv1, cv0_low, cv1_low]);

        mesh.indices.extend_from_slice(&[base, base + 1, base + 2]);
        mesh.indices
            .extend_from_slice(&[base + 1, base + 3, base + 2]);
        triangles_added += 2;
    }

    triangles_added
}

/// Apply full LOD transition seam elimination to a mesh.
///
/// For each face with a coarser neighbor:
/// 1. Constrains edge vertices to eliminate T-junctions.
/// 2. Generates skirt geometry to cover remaining gaps.
///
/// Returns the total number of skirt triangles added.
pub fn apply_seam_fix(
    mesh: &mut PackedChunkMesh,
    context: &ChunkLodContext,
    chunk_size: usize,
    skirt_depth: u8,
) -> usize {
    let mut total_skirt_tris = 0;

    for dir in FaceDirection::ALL {
        // Step 1: constrain edge vertices where neighbor is coarser.
        if let Some(lod_diff) = context.needs_constraining(dir) {
            constrain_edge_vertices(mesh, dir, lod_diff, chunk_size);
        }

        // Step 2: add skirt geometry for any LOD difference.
        if context.has_lod_difference(dir) {
            total_skirt_tris += generate_skirt(mesh, dir, skirt_depth, chunk_size);
        }
    }

    total_skirt_tris
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two adjacent chunks at the same LOD level should need no constraining.
    #[test]
    fn test_same_lod_neighbors_no_seam() {
        let ctx = ChunkLodContext::uniform(0);
        for dir in FaceDirection::ALL {
            assert_eq!(ctx.needs_constraining(dir), None);
            assert!(!ctx.has_lod_difference(dir));
        }
    }

    /// A LOD 0 chunk next to a LOD 1 chunk should constrain boundary vertices.
    #[test]
    fn test_lod_difference_of_1_handled() {
        let chunk_size = 32usize;
        let step = 2usize; // 2^1

        let mut mesh = PackedChunkMesh::new();
        // Place vertices along the +X boundary at every integer position.
        for y in 0..=chunk_size as u8 {
            for z in 0..=chunk_size as u8 {
                mesh.vertices.push(ChunkVertex::new(
                    [chunk_size as u8, y, z],
                    FaceDirection::PosX,
                    0,
                    1,
                    [y, z],
                ));
            }
        }

        constrain_edge_vertices(&mut mesh, FaceDirection::PosX, 1, chunk_size);

        // After constraining, all boundary vertex coords should align to step.
        for vertex in &mesh.vertices {
            let y = vertex.position[1] as usize;
            let z = vertex.position[2] as usize;
            assert_eq!(y % step, 0, "Y={y} not aligned to step={step}");
            assert_eq!(z % step, 0, "Z={z} not aligned to step={step}");
        }
    }

    /// Skirt geometry should add vertices and triangles.
    #[test]
    fn test_skirt_geometry_covers_gaps() {
        let mut mesh = PackedChunkMesh::new();
        let vert_count_before = mesh.vertices.len();

        let tris = generate_skirt(&mut mesh, FaceDirection::PosX, 2, 32);

        assert!(mesh.vertices.len() > vert_count_before);
        assert!(tris > 0, "Skirt should add triangles");
        // All indices should be valid.
        let vc = mesh.vertices.len() as u32;
        for &idx in &mesh.indices {
            assert!(idx < vc, "Invalid index {idx} (vertex count {vc})");
        }
    }

    /// `apply_seam_fix` should constrain and add skirts for transitioning faces.
    #[test]
    fn test_apply_seam_fix_integration() {
        let chunk_size = 32usize;
        let mut mesh = PackedChunkMesh::new();

        // Add vertices on +X boundary.
        for y in 0..=chunk_size as u8 {
            mesh.vertices.push(ChunkVertex::new(
                [chunk_size as u8, y, 0],
                FaceDirection::PosX,
                0,
                1,
                [y, 0],
            ));
        }

        let ctx = ChunkLodContext::from_neighbor_lods(0, [Some(1), None, None, None, None, None]);
        let skirt_tris = apply_seam_fix(&mut mesh, &ctx, chunk_size, 2);

        // Should have added skirt geometry for +X face.
        assert!(skirt_tris > 0, "Expected skirt triangles on +X face");

        // Boundary vertices should be constrained.
        let step = 2usize;
        for v in &mesh.vertices[..chunk_size + 1] {
            if v.position[0] == chunk_size as u8 {
                let y = v.position[1] as usize;
                assert_eq!(y % step, 0, "Y={y} not aligned after seam fix");
            }
        }
    }

    /// `ChunkLodContext::from_neighbor_lods` correctly classifies relations.
    #[test]
    fn test_chunk_lod_context_classification() {
        let ctx = ChunkLodContext::from_neighbor_lods(
            1,
            [Some(0), Some(1), Some(2), None, Some(3), Some(1)],
        );

        // +X: neighbor LOD 0 < center LOD 1 → LowerThanNeighbor
        assert_eq!(ctx.neighbors[0], NeighborLodRelation::LowerThanNeighbor);
        // -X: same
        assert_eq!(ctx.neighbors[1], NeighborLodRelation::Same);
        // +Y: neighbor LOD 2 > center LOD 1 → HigherThanNeighbor { lod_diff: 1 }
        assert_eq!(
            ctx.neighbors[2],
            NeighborLodRelation::HigherThanNeighbor { lod_diff: 1 }
        );
        // -Y: None → Same
        assert_eq!(ctx.neighbors[3], NeighborLodRelation::Same);
        // +Z: neighbor LOD 3 > center LOD 1 → HigherThanNeighbor { lod_diff: 2 }
        assert_eq!(
            ctx.neighbors[4],
            NeighborLodRelation::HigherThanNeighbor { lod_diff: 2 }
        );
        // -Z: same
        assert_eq!(ctx.neighbors[5], NeighborLodRelation::Same);
    }

    /// Skirt with zero depth should add nothing.
    #[test]
    fn test_skirt_zero_depth_noop() {
        let mut mesh = PackedChunkMesh::new();
        let tris = generate_skirt(&mut mesh, FaceDirection::PosX, 0, 32);
        assert_eq!(tris, 0);
        assert!(mesh.vertices.is_empty());
    }

    /// Visual continuity: constrained vertices should be smooth (no big jumps).
    #[test]
    fn test_visual_continuity_across_boundary() {
        let chunk_size = 32usize;
        let mut mesh = PackedChunkMesh::new();

        for y in 0..=chunk_size as u8 {
            mesh.vertices.push(ChunkVertex::new(
                [chunk_size as u8, y, 16],
                FaceDirection::PosX,
                0,
                1,
                [y, 16],
            ));
        }

        constrain_edge_vertices(&mut mesh, FaceDirection::PosX, 1, chunk_size);

        // Adjacent constrained vertices should differ by at most `step`.
        for pair in mesh.vertices.windows(2) {
            let dy = (pair[0].position[1] as i16 - pair[1].position[1] as i16).unsigned_abs();
            assert!(dy <= 2, "Adjacent vertices differ by {dy}, expected <= 2");
        }
    }
}
