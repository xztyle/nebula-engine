//! LOD transition stitching: eliminates cracks at boundaries between chunks
//! at different levels of detail.
//!
//! When adjacent chunks have different LOD levels, their meshes have different
//! vertex densities along the shared edge. This module provides two approaches
//! to fix the resulting T-junctions:
//!
//! 1. **Vertex snapping** — high-LOD edge vertices are snapped to the low-LOD grid.
//! 2. **Transition strips** — explicit bridging geometry connects the two grids.

use crate::face_direction::FaceDirection;
use crate::packed::{ChunkVertex, PackedChunkMesh};

/// Information about the LOD relationship between a chunk and its 6 face neighbors.
#[derive(Clone, Debug)]
pub struct LodContext {
    /// The LOD level of the central chunk being meshed.
    pub center_lod: u8,
    /// The LOD level of each face neighbor (if loaded).
    /// `None` means the neighbor is not loaded (treat as same LOD).
    /// Indexed by [`FaceDirection`] discriminant.
    pub neighbor_lods: [Option<u8>; 6],
}

impl LodContext {
    /// Create a context where all neighbors share the same LOD level.
    pub fn uniform(lod: u8) -> Self {
        Self {
            center_lod: lod,
            neighbor_lods: [Some(lod); 6],
        }
    }

    /// Check if a specific face has a LOD transition.
    pub fn has_transition(&self, direction: FaceDirection) -> bool {
        if let Some(neighbor_lod) = self.neighbor_lods[direction as usize] {
            neighbor_lod != self.center_lod
        } else {
            false
        }
    }

    /// Get the LOD difference for a face. Positive means neighbor is coarser.
    pub fn lod_difference(&self, direction: FaceDirection) -> i8 {
        if let Some(neighbor_lod) = self.neighbor_lods[direction as usize] {
            neighbor_lod as i8 - self.center_lod as i8
        } else {
            0
        }
    }
}

/// Returns `(u_axis_index, v_axis_index)` — the two tangential axis indices
/// for the given face direction. Each value is 0=X, 1=Y, 2=Z.
fn tangential_axes(direction: FaceDirection) -> (usize, usize) {
    let (_layer, u, v) = direction.sweep_axes();
    (u, v)
}

/// Returns `true` if the vertex sits on the boundary face for the given direction.
fn is_on_face_boundary(vertex: &ChunkVertex, direction: FaceDirection, chunk_size: usize) -> bool {
    let (layer_axis, _, _) = direction.sweep_axes();
    let boundary = match direction {
        FaceDirection::PosX | FaceDirection::PosY | FaceDirection::PosZ => chunk_size as u8,
        FaceDirection::NegX | FaceDirection::NegY | FaceDirection::NegZ => 0,
    };
    vertex.position[layer_axis] == boundary
}

/// Snap high-LOD edge vertices to align with the low-LOD grid.
///
/// For a LOD 0 chunk adjacent to a LOD 1 chunk along a face:
/// vertices on the boundary that don't align with the coarser grid
/// are snapped to the nearest grid position, eliminating T-junctions.
///
/// `lod_difference` is the unsigned LOD step count (e.g. 1 for LOD 0→1).
pub fn snap_edge_vertices(
    mesh: &mut PackedChunkMesh,
    direction: FaceDirection,
    lod_difference: u8,
    chunk_size: usize,
) {
    if lod_difference == 0 {
        return;
    }
    let step = 1usize << lod_difference;
    let (u_idx, v_idx) = tangential_axes(direction);

    for vertex in &mut mesh.vertices {
        if !is_on_face_boundary(vertex, direction, chunk_size) {
            continue;
        }

        let u = vertex.position[u_idx] as usize;
        let v = vertex.position[v_idx] as usize;

        // Round to nearest multiple of step (ties go down via integer division).
        let snapped_u = ((u + step / 2) / step) * step;
        let snapped_v = ((v + step / 2) / step) * step;

        // Clamp to chunk bounds.
        vertex.position[u_idx] = snapped_u.min(chunk_size) as u8;
        vertex.position[v_idx] = snapped_v.min(chunk_size) as u8;
    }
}

/// Generate a transition mesh strip along a LOD boundary.
///
/// The strip connects the high-LOD edge vertices to the low-LOD edge vertices
/// using triangle fans that smoothly bridge the two grids.
///
/// `center_lod` — LOD of the chunk being meshed (higher detail).
/// `neighbor_lod` — LOD of the adjacent chunk (coarser). Must be > `center_lod`.
/// `chunk_size` — number of voxels along each chunk axis.
///
/// Returns a [`PackedChunkMesh`] containing the transition triangles.
pub fn generate_transition_strip(
    direction: FaceDirection,
    center_lod: u8,
    neighbor_lod: u8,
    chunk_size: usize,
) -> PackedChunkMesh {
    let mut strip = PackedChunkMesh::new();

    if neighbor_lod <= center_lod {
        return strip;
    }

    let lod_diff = neighbor_lod - center_lod;
    let low_step = 1usize << lod_diff;
    let (layer_axis, u_axis, v_axis) = direction.sweep_axes();

    let boundary = match direction {
        FaceDirection::PosX | FaceDirection::PosY | FaceDirection::PosZ => chunk_size as u8,
        FaceDirection::NegX | FaceDirection::NegY | FaceDirection::NegZ => 0,
    };

    // Walk along u and v axes of the boundary face.
    // For each low-LOD cell, emit a fan of triangles bridging high-LOD vertices.
    let mut low_u = 0usize;
    while low_u < chunk_size {
        let next_low_u = (low_u + low_step).min(chunk_size);
        let mut low_v = 0usize;
        while low_v < chunk_size {
            let next_low_v = (low_v + low_step).min(chunk_size);

            // Four corners of the low-LOD cell on the boundary.
            let make_pos = |u: usize, v: usize| -> [u8; 3] {
                let mut pos = [0u8; 3];
                pos[layer_axis] = boundary;
                pos[u_axis] = u as u8;
                pos[v_axis] = v as u8;
                pos
            };

            // Emit triangles along the bottom edge (v = low_v, u varies).
            // Fan from (low_u, low_v) to each high-LOD vertex.
            let anchor = make_pos(low_u, low_v);
            let anchor_v = ChunkVertex::new(anchor, direction, 0, 0, [0, 0]);

            // Bottom edge: (low_u..next_low_u, low_v)
            for high_u in low_u..next_low_u {
                let v0 = ChunkVertex::new(make_pos(high_u, low_v), direction, 0, 0, [0, 0]);
                let v1 = ChunkVertex::new(make_pos(high_u + 1, low_v), direction, 0, 0, [0, 0]);
                // Skip degenerate: if anchor == v0, emit with next vertex
                if anchor != make_pos(high_u, low_v) {
                    let base = strip.vertices.len() as u32;
                    strip.vertices.push(anchor_v);
                    strip.vertices.push(v0);
                    strip.vertices.push(v1);
                    strip.indices.extend_from_slice(&[base, base + 1, base + 2]);
                }
            }

            // Right edge: (next_low_u, low_v..next_low_v)
            for high_v in low_v..next_low_v {
                let v0 = ChunkVertex::new(make_pos(next_low_u, high_v), direction, 0, 0, [0, 0]);
                let v1 =
                    ChunkVertex::new(make_pos(next_low_u, high_v + 1), direction, 0, 0, [0, 0]);
                let base = strip.vertices.len() as u32;
                strip.vertices.push(anchor_v);
                strip.vertices.push(v0);
                strip.vertices.push(v1);
                strip.indices.extend_from_slice(&[base, base + 1, base + 2]);
            }

            // Top edge: (next_low_u..low_u, next_low_v) — reverse direction
            let top_anchor_pos = make_pos(next_low_u, next_low_v);
            let top_anchor = ChunkVertex::new(top_anchor_pos, direction, 0, 0, [0, 0]);
            for high_u in (low_u..next_low_u).rev() {
                let v0 =
                    ChunkVertex::new(make_pos(high_u + 1, next_low_v), direction, 0, 0, [0, 0]);
                let v1 = ChunkVertex::new(make_pos(high_u, next_low_v), direction, 0, 0, [0, 0]);
                if top_anchor_pos != make_pos(high_u + 1, next_low_v) {
                    let base = strip.vertices.len() as u32;
                    strip.vertices.push(top_anchor);
                    strip.vertices.push(v0);
                    strip.vertices.push(v1);
                    strip.indices.extend_from_slice(&[base, base + 1, base + 2]);
                }
            }

            // Left edge: (low_u, next_low_v..low_v) — reverse direction
            for high_v in (low_v..next_low_v).rev() {
                let v0 = ChunkVertex::new(make_pos(low_u, high_v + 1), direction, 0, 0, [0, 0]);
                let v1 = ChunkVertex::new(make_pos(low_u, high_v), direction, 0, 0, [0, 0]);
                let base = strip.vertices.len() as u32;
                strip.vertices.push(top_anchor);
                strip.vertices.push(v0);
                strip.vertices.push(v1);
                strip.indices.extend_from_slice(&[base, base + 1, base + 2]);
            }

            low_v = next_low_v;
        }
        low_u = next_low_u;
    }

    strip
}

/// Apply LOD stitching to a mesh based on the given [`LodContext`].
///
/// For each face with a positive LOD difference (neighbor is coarser),
/// snaps boundary vertices to the coarser grid.
pub fn apply_lod_stitching(mesh: &mut PackedChunkMesh, context: &LodContext, chunk_size: usize) {
    for dir in FaceDirection::ALL {
        let diff = context.lod_difference(dir);
        if diff > 0 {
            snap_edge_vertices(mesh, dir, diff as u8, chunk_size);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// When two adjacent chunks have the same LOD level, no stitching is needed.
    #[test]
    fn test_same_lod_neighbors_no_stitching() {
        let ctx = LodContext {
            center_lod: 0,
            neighbor_lods: [Some(0); 6],
        };

        for dir in FaceDirection::ALL {
            assert!(
                !ctx.has_transition(dir),
                "Same-LOD neighbors should not have a transition on {dir:?}"
            );
        }
    }

    /// LOD N next to LOD N+1 should produce transition geometry.
    #[test]
    fn test_lod_transition_produces_geometry() {
        let strip = generate_transition_strip(FaceDirection::PosX, 0, 1, 32);

        assert!(
            !strip.is_empty(),
            "LOD 0 -> LOD 1 transition should produce transition geometry"
        );
        assert!(
            strip.triangle_count() > 0,
            "Transition strip should have triangles"
        );
    }

    /// After snapping, all edge vertices on the high-LOD side must align
    /// with the low-LOD grid positions.
    #[test]
    fn test_no_cracks_after_stitching() {
        let chunk_size = 32;
        let lod_diff = 1u8;
        let step = 1usize << lod_diff; // 2

        let mut mesh = PackedChunkMesh::new();
        for z in 0..=chunk_size as u8 {
            for y in 0..=chunk_size as u8 {
                mesh.vertices.push(ChunkVertex::new(
                    [chunk_size as u8, y, z],
                    FaceDirection::PosX,
                    0,
                    1,
                    [y, z],
                ));
            }
        }

        snap_edge_vertices(&mut mesh, FaceDirection::PosX, lod_diff, chunk_size);

        for vertex in &mesh.vertices {
            let y = vertex.position[1] as usize;
            let z = vertex.position[2] as usize;
            assert_eq!(
                y % step,
                0,
                "Y position {y} is not aligned to LOD grid (step={step})"
            );
            assert_eq!(
                z % step,
                0,
                "Z position {z} is not aligned to LOD grid (step={step})"
            );
        }
    }

    /// Transition mesh should be watertight: all indices valid, no degenerate triangles.
    #[test]
    fn test_transition_mesh_is_watertight() {
        let strip = generate_transition_strip(FaceDirection::PosX, 0, 1, 32);

        let vertex_count = strip.vertices.len() as u32;
        for &idx in &strip.indices {
            assert!(
                idx < vertex_count,
                "Invalid index {idx} in transition strip (vertex count: {vertex_count})"
            );
        }

        for tri in strip.indices.chunks(3) {
            let v0 = strip.vertices[tri[0] as usize].position;
            let v1 = strip.vertices[tri[1] as usize].position;
            let v2 = strip.vertices[tri[2] as usize].position;
            let degenerate = v0 == v1 || v1 == v2 || v0 == v2;
            assert!(
                !degenerate,
                "Transition strip should not contain degenerate triangles"
            );
        }
    }

    /// `LodContext` correctly reports transitions and differences.
    #[test]
    fn test_lod_context_reports_transitions() {
        let ctx = LodContext {
            center_lod: 0,
            neighbor_lods: [Some(0), Some(1), Some(0), Some(2), None, Some(0)],
        };

        assert!(!ctx.has_transition(FaceDirection::PosX)); // same LOD
        assert!(ctx.has_transition(FaceDirection::NegX)); // LOD 0 vs 1
        assert!(!ctx.has_transition(FaceDirection::PosY)); // same LOD
        assert!(ctx.has_transition(FaceDirection::NegY)); // LOD 0 vs 2
        assert!(!ctx.has_transition(FaceDirection::PosZ)); // None = no transition
        assert!(!ctx.has_transition(FaceDirection::NegZ)); // same LOD

        assert_eq!(ctx.lod_difference(FaceDirection::NegX), 1);
        assert_eq!(ctx.lod_difference(FaceDirection::NegY), 2);
    }

    /// `apply_lod_stitching` snaps vertices on all transitioning faces.
    #[test]
    fn test_apply_lod_stitching_multi_face() {
        let chunk_size = 32usize;
        let mut mesh = PackedChunkMesh::new();

        // Add vertices on +X boundary
        for i in 0..=chunk_size as u8 {
            mesh.vertices.push(ChunkVertex::new(
                [chunk_size as u8, i, 0],
                FaceDirection::PosX,
                0,
                1,
                [i, 0],
            ));
        }

        let ctx = LodContext {
            center_lod: 0,
            neighbor_lods: [Some(1), Some(0), Some(0), Some(0), Some(0), Some(0)],
        };

        apply_lod_stitching(&mut mesh, &ctx, chunk_size);

        let step = 2usize;
        for vertex in &mesh.vertices {
            if vertex.position[0] == chunk_size as u8 {
                let y = vertex.position[1] as usize;
                assert_eq!(y % step, 0, "Y={y} not aligned after stitching");
            }
        }
    }

    /// Uniform LOD context has no transitions.
    #[test]
    fn test_uniform_lod_context() {
        let ctx = LodContext::uniform(2);
        for dir in FaceDirection::ALL {
            assert!(!ctx.has_transition(dir));
            assert_eq!(ctx.lod_difference(dir), 0);
        }
    }

    /// Transition strip for LOD 0 → LOD 2 (step=4) produces geometry.
    #[test]
    fn test_transition_strip_lod_difference_2() {
        let strip = generate_transition_strip(FaceDirection::NegY, 0, 2, 32);
        assert!(!strip.is_empty());
        assert!(strip.triangle_count() > 0);

        let vertex_count = strip.vertices.len() as u32;
        for &idx in &strip.indices {
            assert!(idx < vertex_count);
        }
    }
}
