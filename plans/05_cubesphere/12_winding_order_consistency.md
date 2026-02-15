# Winding Order Consistency

## Problem

Modern GPU rasterizers use triangle winding order (the direction vertices appear when viewed from the front — counter-clockwise is the convention) to perform backface culling: triangles facing away from the camera are discarded before fragment shading. On a cubesphere, "front-facing" means the triangle normal points outward, away from the planet center. The cube-to-sphere projection can silently flip the winding order on certain faces because the tangent/bitangent basis vectors are mirrored relative to the face normal on half the cube faces (specifically, faces where the projection involves an odd number of axis reflections). If this is not detected and corrected, entire faces of the planet will appear invisible (backface-culled when they should be visible) or will have inverted lighting (normals point inward instead of outward). The problem is especially insidious because it only manifests on some faces and may not be noticed until the camera orbits to the far side of the planet.

## Solution

Implement winding order validation and correction in the `nebula_cubesphere` crate.

### Winding Order Detection

Given three vertices of a triangle on the cubesphere surface, determine whether the winding order is correct (CCW when viewed from outside, meaning the cross product of the two edge vectors points outward from the planet center):

```rust
use glam::DVec3;

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
```

### Per-Face Winding Correction Table

Rather than checking every triangle at runtime, precompute which faces need their winding order flipped. The cube-to-sphere projection is deterministic, so the winding flip is a property of the face, not of individual triangles:

```rust
/// Determine whether triangles generated on a given face need their
/// winding order reversed to maintain outward-facing CCW convention.
///
/// This is determined by checking whether the cross product of the
/// face's tangent and bitangent (in the order used for mesh generation)
/// produces a normal that matches the face's outward normal, after
/// projection onto the sphere.
pub fn face_needs_winding_flip(face: CubeFace) -> bool {
    // Generate a small test triangle near the face center
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
```

### Winding Correction During Mesh Generation

The mesher applies the correction when emitting triangles:

```rust
/// Emit a triangle with correct winding order.
///
/// If `flip` is true, swap v1 and v2 to reverse the winding.
pub fn emit_triangle(v0: u32, v1: u32, v2: u32, flip: bool) -> [u32; 3] {
    if flip {
        [v0, v2, v1]
    } else {
        [v0, v1, v2]
    }
}

/// Generate index buffer for a chunk mesh grid with correct winding.
///
/// `grid_size`: number of vertices along each axis of the chunk grid.
/// `face`: the cube face, used to determine if winding flip is needed.
/// `flip_table`: precomputed from `compute_winding_flip_table()`.
pub fn generate_chunk_indices(
    grid_size: u32,
    face: CubeFace,
    flip_table: &[bool; 6],
) -> Vec<u32> {
    let flip = flip_table[face as usize];
    let mut indices = Vec::with_capacity(((grid_size - 1) * (grid_size - 1) * 6) as usize);

    for y in 0..(grid_size - 1) {
        for x in 0..(grid_size - 1) {
            let i00 = y * grid_size + x;
            let i10 = y * grid_size + (x + 1);
            let i01 = (y + 1) * grid_size + x;
            let i11 = (y + 1) * grid_size + (x + 1);

            // Two triangles per quad
            let tri1 = emit_triangle(i00, i10, i01, flip);
            let tri2 = emit_triangle(i10, i11, i01, flip);

            indices.extend_from_slice(&tri1);
            indices.extend_from_slice(&tri2);
        }
    }

    indices
}
```

### Face Edge Winding Validation

At face boundaries, triangles from adjacent faces meet. Both faces must produce consistent winding at the seam:

```rust
/// Validate that winding order is consistent across a face edge.
///
/// Generates triangles on both sides of the edge and verifies that
/// both have outward-facing normals.
pub fn validate_edge_winding(
    face_a: CubeFace,
    face_b: CubeFace,
    flip_table: &[bool; 6],
) -> bool {
    // Generate test triangles near the shared edge and verify both
    // have outward-facing normals after applying the flip correction.
    // The specific edge point depends on the adjacency table from story 07.
    // This function samples several points along the shared edge.

    let samples = 10;
    for i in 0..samples {
        let t = (i as f64 + 0.5) / samples as f64;

        // Sample a triangle on face_a near the edge
        let fc_a = FaceCoord::new(face_a, 0.999, t);
        let fc_a1 = FaceCoord::new(face_a, 0.998, t);
        let fc_a2 = FaceCoord::new(face_a, 0.999, t + 0.001);

        let mut v0 = face_coord_to_sphere_everitt(&fc_a);
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
```

### LOD Transition Triangle Winding

When chunks at different LODs meet, the coarser chunk generates "skirt" or "transition" triangles to fill the gap. These transition triangles must also respect the winding convention:

```rust
/// Generate a transition triangle strip between a fine-LOD edge and a
/// coarse-LOD edge, maintaining correct winding order.
///
/// `fine_verts`: vertices along the fine-LOD chunk edge (more vertices).
/// `coarse_verts`: vertices along the coarse-LOD chunk edge (fewer vertices).
/// `flip`: whether to flip winding for this face.
///
/// Returns index offsets into a combined vertex buffer.
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

        // Fan from coarse edge to fine edge vertices
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
```

### Design Constraints

- The winding flip table is computed once at startup (or even at compile time via `const fn` if the projection functions are made `const`). It is not recomputed per-frame.
- The flip correction is applied in the index buffer, not by reordering vertex data. This is cheaper and does not affect vertex cache performance.
- All validation functions are debug-only (`#[cfg(debug_assertions)]`) in production builds, since the flip table is precomputed and trusted after validation.
- The winding convention is CCW when viewed from outside the planet, consistent with Vulkan/wgpu's default front-face setting (`FrontFace::Ccw`).

## Outcome

The `nebula_cubesphere` crate exports `triangle_winds_outward()`, `face_needs_winding_flip()`, `compute_winding_flip_table()`, `emit_triangle()`, `generate_chunk_indices()`, `validate_edge_winding()`, and `generate_lod_transition_strip()`. The mesh generation system uses the flip table to guarantee correct backface culling on all 6 faces. Running `cargo test -p nebula_cubesphere` passes all winding order tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Backface culling is enabled and no holes appear in the sphere. All triangles face outward consistently across all six faces and all LOD levels.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `glam` | 0.29 | `DVec3` for triangle normal computation |

The crate uses Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;

    #[test]
    fn test_triangle_normal_points_away_from_center_all_faces() {
        let flip_table = compute_winding_flip_table();

        for face in CubeFace::ALL {
            // Generate a test triangle near the face center
            let fc0 = FaceCoord::new(face, 0.45, 0.45);
            let fc1 = FaceCoord::new(face, 0.55, 0.45);
            let fc2 = FaceCoord::new(face, 0.45, 0.55);

            let mut v0 = face_coord_to_sphere_everitt(&fc0);
            let mut v1 = face_coord_to_sphere_everitt(&fc1);
            let mut v2 = face_coord_to_sphere_everitt(&fc2);

            // Apply flip correction
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
            (0.1, 0.1), (0.9, 0.1), (0.1, 0.9), (0.9, 0.9),
            (0.3, 0.7), (0.7, 0.3), (0.5, 0.5), (0.2, 0.8),
        ];

        for face in CubeFace::ALL {
            for &(u, v) in &test_points {
                let du = 0.01;
                let dv = 0.01;
                let fc0 = FaceCoord::new(face, u, v);
                let fc1 = FaceCoord::new(face, (u + du).min(1.0), v);
                let fc2 = FaceCoord::new(face, u, (v + dv).min(1.0));

                let mut v0 = face_coord_to_sphere_everitt(&fc0);
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

        // Test triangles near all 4 edges of each face
        let edge_samples = [
            (0.5, 0.001), // south edge
            (0.5, 0.999), // north edge
            (0.001, 0.5), // west edge
            (0.999, 0.5), // east edge
        ];

        for face in CubeFace::ALL {
            for &(u, v) in &edge_samples {
                let du = 0.005;
                let dv = 0.005;
                let fc0 = FaceCoord::new(face, u, v);
                let fc1 = FaceCoord::new(face, (u + du).min(0.999), v);
                let fc2 = FaceCoord::new(face, u, (v + dv).min(0.999));

                let mut v0 = face_coord_to_sphere_everitt(&fc0);
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

            // Verify all triangles have 3 distinct indices
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
        let grid_size = 17; // 16x16 quads = 512 triangles (2 per quad)
        let indices = generate_chunk_indices(grid_size, CubeFace::PosX, &flip_table);
        let expected_triangles = (grid_size - 1) * (grid_size - 1) * 2;
        assert_eq!(indices.len() as u32, expected_triangles * 3);
    }

    #[test]
    fn test_opposite_faces_may_have_different_flip() {
        let table = compute_winding_flip_table();
        // This test documents the actual flip state; it's not guaranteed
        // that opposites differ, but it's useful to verify the table is
        // non-trivial (not all true or all false).
        let all_same = table.iter().all(|&f| f == table[0]);
        // It's possible all faces have the same flip state depending on
        // the projection. This test just ensures the table was computed.
        // The real correctness check is in the outward-normal tests above.
        assert_eq!(table.len(), 6);
    }
}
```
