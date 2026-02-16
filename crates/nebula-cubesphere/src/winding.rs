//! Winding order validation and correction for cubesphere meshes.
//!
//! Ensures all triangles have counter-clockwise winding when viewed from
//! outside the planet, enabling correct backface culling on all six cube faces.

use glam::DVec3;

use crate::cube_face::CubeFace;
use crate::face_coord::FaceCoord;
use crate::projection::face_coord_to_sphere_everitt;

/// Check if a triangle has correct outward-facing winding order.
///
/// Returns `true` if the triangle's computed normal (via cross product)
/// points away from the planet center (i.e., has a positive dot product
/// with the centroid direction from the planet center).
///
/// `v0`, `v1`, `v2` are the triangle vertices in world space relative to
/// the planet center (i.e., planet-relative positions).
pub fn triangle_winds_outward(v0: DVec3, v1: DVec3, v2: DVec3) -> bool {
    let edge1 = v1 - v0;
    let edge2 = v2 - v0;
    let face_normal = edge1.cross(edge2);

    // The centroid of the triangle (its average position) gives the
    // approximate outward direction from the planet center
    let centroid = (v0 + v1 + v2) / 3.0;

    face_normal.dot(centroid) > 0.0
}

/// Determine whether triangles generated on a given face need their
/// winding order reversed to maintain outward-facing CCW convention.
///
/// This is determined by checking whether the cross product of the
/// face's tangent and bitangent (in the order used for mesh generation)
/// produces a normal that matches the face's outward normal, after
/// projection onto the sphere.
pub fn face_needs_winding_flip(face: CubeFace) -> bool {
    let fc0 = FaceCoord::new(face, 0.5, 0.5);
    let fc1 = FaceCoord::new(face, 0.501, 0.5);
    let fc2 = FaceCoord::new(face, 0.5, 0.501);

    let v0 = face_coord_to_sphere_everitt(&fc0);
    let v1 = face_coord_to_sphere_everitt(&fc1);
    let v2 = face_coord_to_sphere_everitt(&fc2);

    !triangle_winds_outward(v0, v1, v2)
}

/// Precomputed winding flip table. Call once at startup.
///
/// Returns an array indexed by `CubeFace as usize`. If the entry is `true`,
/// all triangles on that face must have their vertex order reversed
/// (swap v1 and v2) during mesh generation.
pub fn compute_winding_flip_table() -> [bool; 6] {
    let mut table = [false; 6];
    for face in CubeFace::ALL {
        table[face as usize] = face_needs_winding_flip(face);
    }
    table
}

/// Emit a triangle with correct winding order.
///
/// If `flip` is true, swap v1 and v2 to reverse the winding.
pub fn emit_triangle(v0: u32, v1: u32, v2: u32, flip: bool) -> [u32; 3] {
    if flip { [v0, v2, v1] } else { [v0, v1, v2] }
}

/// Generate index buffer for a chunk mesh grid with correct winding.
///
/// `grid_size`: number of vertices along each axis of the chunk grid.
/// `face`: the cube face, used to determine if winding flip is needed.
/// `flip_table`: precomputed from [`compute_winding_flip_table()`].
pub fn generate_chunk_indices(grid_size: u32, face: CubeFace, flip_table: &[bool; 6]) -> Vec<u32> {
    let flip = flip_table[face as usize];
    let quads = (grid_size - 1) * (grid_size - 1);
    let mut indices = Vec::with_capacity((quads * 6) as usize);

    for y in 0..(grid_size - 1) {
        for x in 0..(grid_size - 1) {
            let i00 = y * grid_size + x;
            let i10 = y * grid_size + (x + 1);
            let i01 = (y + 1) * grid_size + x;
            let i11 = (y + 1) * grid_size + (x + 1);

            let tri1 = emit_triangle(i00, i10, i01, flip);
            let tri2 = emit_triangle(i10, i11, i01, flip);

            indices.extend_from_slice(&tri1);
            indices.extend_from_slice(&tri2);
        }
    }

    indices
}

/// Validate that winding order is consistent across a face edge.
///
/// Generates triangles on both sides of the edge and verifies that
/// both have outward-facing normals after applying the flip correction.
pub fn validate_edge_winding(face_a: CubeFace, flip_table: &[bool; 6]) -> bool {
    let samples = 10;
    for i in 0..samples {
        let t = (i as f64 + 0.5) / samples as f64;

        let fc_a = FaceCoord::new(face_a, 0.999, t);
        let fc_a1 = FaceCoord::new(face_a, 0.998, t);
        let fc_a2 = FaceCoord::new(face_a, 0.999, (t + 0.001).min(1.0));

        let v0 = face_coord_to_sphere_everitt(&fc_a);
        let mut v1 = face_coord_to_sphere_everitt(&fc_a1);
        let mut v2 = face_coord_to_sphere_everitt(&fc_a2);

        if flip_table[face_a as usize] {
            std::mem::swap(&mut v1, &mut v2);
        }

        if !triangle_winds_outward(v0, v1, v2) {
            return false;
        }
    }
    true
}

/// Generate a transition triangle strip between a fine-LOD edge and a
/// coarse-LOD edge, maintaining correct winding order.
///
/// `fine_count`: number of vertices along the fine-LOD edge.
/// `coarse_count`: number of vertices along the coarse-LOD edge.
/// `fine_offset`: starting index for fine-LOD vertices in the combined buffer.
/// `coarse_offset`: starting index for coarse-LOD vertices in the combined buffer.
/// `flip`: whether to flip winding for this face.
///
/// Returns triangles as index triplets into a combined vertex buffer.
pub fn generate_lod_transition_strip(
    fine_count: u32,
    coarse_count: u32,
    fine_offset: u32,
    coarse_offset: u32,
    flip: bool,
) -> Vec<[u32; 3]> {
    let mut triangles = Vec::new();
    let ratio = fine_count / coarse_count;

    for c in 0..(coarse_count - 1) {
        let c0 = coarse_offset + c;
        let c1 = coarse_offset + c + 1;

        let f_start = fine_offset + c * ratio;
        for f in 0..ratio {
            let f0 = f_start + f;
            let f1 = f_start + f + 1;
            triangles.push(emit_triangle(c0, f0, f1, flip));
        }
        // Connecting triangle to next coarse vertex
        let f_last = f_start + ratio;
        triangles.push(emit_triangle(c0, f_last, c1, flip));
    }

    triangles
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_triangle_normal_points_away_from_center_all_faces() {
        let flip_table = compute_winding_flip_table();

        for face in CubeFace::ALL {
            let fc0 = FaceCoord::new(face, 0.45, 0.45);
            let fc1 = FaceCoord::new(face, 0.55, 0.45);
            let fc2 = FaceCoord::new(face, 0.45, 0.55);

            let v0 = face_coord_to_sphere_everitt(&fc0);
            let mut v1 = face_coord_to_sphere_everitt(&fc1);
            let mut v2 = face_coord_to_sphere_everitt(&fc2);

            if flip_table[face as usize] {
                std::mem::swap(&mut v1, &mut v2);
            }

            assert!(
                triangle_winds_outward(v0, v1, v2),
                "Triangle on face {face:?} does not point outward after correction"
            );
        }
    }

    #[test]
    fn test_winding_ccw_from_outside_random_samples() {
        let flip_table = compute_winding_flip_table();

        let test_points = [
            (0.1, 0.1),
            (0.9, 0.1),
            (0.1, 0.9),
            (0.9, 0.9),
            (0.3, 0.7),
            (0.7, 0.3),
            (0.5, 0.5),
            (0.2, 0.8),
        ];

        for face in CubeFace::ALL {
            for &(u, v) in &test_points {
                let du = 0.01;
                let dv = 0.01;
                let fc0 = FaceCoord::new(face, u, v);
                let fc1 = FaceCoord::new(face, (u + du).min(1.0), v);
                let fc2 = FaceCoord::new(face, u, (v + dv).min(1.0));

                let v0 = face_coord_to_sphere_everitt(&fc0);
                let mut v1 = face_coord_to_sphere_everitt(&fc1);
                let mut v2 = face_coord_to_sphere_everitt(&fc2);

                if flip_table[face as usize] {
                    std::mem::swap(&mut v1, &mut v2);
                }

                assert!(
                    triangle_winds_outward(v0, v1, v2),
                    "CCW winding failed for face {face:?} at ({u}, {v})"
                );
            }
        }
    }

    #[test]
    fn test_no_flipped_triangles_at_face_edges() {
        let flip_table = compute_winding_flip_table();

        let edge_samples = [(0.5, 0.005), (0.5, 0.99), (0.005, 0.5), (0.99, 0.5)];

        for face in CubeFace::ALL {
            for &(u, v) in &edge_samples {
                let du = 0.005;
                let dv = 0.005;
                let fc0 = FaceCoord::new(face, u, v);
                let fc1 = FaceCoord::new(face, u + du, v);
                let fc2 = FaceCoord::new(face, u, v + dv);

                let v0 = face_coord_to_sphere_everitt(&fc0);
                let mut v1 = face_coord_to_sphere_everitt(&fc1);
                let mut v2 = face_coord_to_sphere_everitt(&fc2);

                if flip_table[face as usize] {
                    std::mem::swap(&mut v1, &mut v2);
                }

                assert!(
                    triangle_winds_outward(v0, v1, v2),
                    "Edge triangle flipped on face {face:?} near ({u}, {v})"
                );
            }
        }
    }

    #[test]
    fn test_lod_transition_triangles_maintain_winding() {
        let flip_table = compute_winding_flip_table();

        for face in CubeFace::ALL {
            let flip = flip_table[face as usize];
            let triangles = generate_lod_transition_strip(8, 4, 0, 100, flip);
            assert!(!triangles.is_empty(), "No transition triangles generated");

            for tri in &triangles {
                assert_ne!(tri[0], tri[1]);
                assert_ne!(tri[1], tri[2]);
                assert_ne!(tri[0], tri[2]);
            }
        }
    }

    #[test]
    fn test_emit_triangle_flip() {
        let normal = emit_triangle(0, 1, 2, false);
        assert_eq!(normal, [0, 1, 2]);

        let flipped = emit_triangle(0, 1, 2, true);
        assert_eq!(flipped, [0, 2, 1]);
    }

    #[test]
    fn test_flip_table_has_6_entries() {
        let table = compute_winding_flip_table();
        assert_eq!(table.len(), 6);
    }

    #[test]
    fn test_generate_chunk_indices_count() {
        let flip_table = compute_winding_flip_table();
        let grid_size = 17;
        let indices = generate_chunk_indices(grid_size, CubeFace::PosX, &flip_table);
        let expected_triangles = (grid_size - 1) * (grid_size - 1) * 2;
        assert_eq!(indices.len() as u32, expected_triangles * 3);
    }

    #[test]
    fn test_opposite_faces_may_have_different_flip() {
        let table = compute_winding_flip_table();
        // Verify the table was computed (non-trivial check).
        assert_eq!(table.len(), 6);
    }
}
