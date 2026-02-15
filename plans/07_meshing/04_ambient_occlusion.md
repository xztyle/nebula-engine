# Ambient Occlusion

## Problem

Flat-shaded voxel meshes look artificial because adjacent faces that meet at concave corners are lit identically. In reality, corners and crevices receive less ambient light because nearby geometry occludes incoming light rays. Without per-vertex ambient occlusion, the visual difference between a flat wall and an interior corner is invisible — the mesh looks like untextured colored blocks rather than a coherent landscape. The classic Minecraft-style voxel AO algorithm is cheap, requires no ray tracing, and produces convincing results by checking only the immediate voxel neighbors at each vertex.

## Solution

Implement per-vertex voxel ambient occlusion in the `nebula_meshing` crate, following the algorithm described by Mikola Lysenko. For each vertex of a visible face, examine the 3 adjacent voxels that form the corner to compute an occlusion value.

### Per-Vertex AO Calculation

Each face of a voxel has 4 vertices. For each vertex, identify the 3 neighboring voxels that could occlude it:

- **side1**: The voxel adjacent along one edge of the face.
- **side2**: The voxel adjacent along the other edge.
- **corner**: The voxel diagonally adjacent at the vertex corner.

```rust
/// Compute the ambient occlusion value for a single vertex.
/// Returns a value in 0..=3 where:
///   0 = fully exposed (brightest)
///   1 = one neighbor solid
///   2 = two neighbors solid
///   3 = fully occluded (darkest)
pub fn vertex_ao(side1: bool, side2: bool, corner: bool) -> u8 {
    if side1 && side2 {
        // Both sides are solid — the corner is irrelevant because
        // the vertex is fully occluded.
        3
    } else {
        (side1 as u8) + (side2 as u8) + (corner as u8)
    }
}
```

The key insight is that when both side voxels are solid, the AO value is 3 regardless of the corner voxel. This avoids the visual artifact where a corner voxel "pokes through" two solid walls.

### Brightness Mapping

AO values map to brightness multipliers applied to the vertex color or passed to the shader:

| AO Value | Brightness | Visual |
|----------|------------|--------|
| 0 | 1.00 | Fully lit |
| 1 | 0.75 | Slight shadow |
| 2 | 0.50 | Medium shadow |
| 3 | 0.25 | Deep shadow |

The brightness is stored as a `u8` AO level (0-3) in the vertex data. The shader converts it: `brightness = 1.0 - 0.25 * ao_level`.

### Quad Diagonal Flipping

When the four AO values of a quad are anisotropic (the two diagonals have different AO sums), the quad's triangulation diagonal must be chosen to avoid an interpolation artifact. Consider a quad with AO values at corners `[a, b, c, d]`:

```rust
/// Determine whether to flip the quad diagonal based on AO values.
/// Returns true if the diagonal should be flipped (a-c vs b-d split).
pub fn should_flip_ao_diagonal(ao: [u8; 4]) -> bool {
    // Compare the two possible diagonals.
    // Default split: triangles (0,1,2) and (0,2,3).
    // Flipped split: triangles (1,2,3) and (0,1,3).
    // Choose the split where the diagonal connects the two vertices
    // with more similar AO values, producing smoother interpolation.
    ao[0] + ao[2] > ao[1] + ao[3]
}
```

If the default diagonal connects two vertices with very different AO values, the interpolated shading creates a visible seam. Flipping the diagonal aligns it with the more uniform pair, hiding the seam.

### Integration with Greedy Meshing

AO values constrain greedy meshing: two adjacent faces can only merge if their shared vertices have identical AO values. If a flat surface has AO=0 everywhere except at one edge (where a wall meets the floor), the merge must split at that AO boundary. This reduces merging efficiency slightly but preserves visual correctness.

```rust
pub fn compute_face_ao(
    neighborhood: &ChunkNeighborhood,
    registry: &VoxelTypeRegistry,
    face_pos: (usize, usize, usize),
    direction: FaceDirection,
) -> [u8; 4] {
    let mut ao = [0u8; 4];
    let offsets = direction.vertex_ao_offsets(); // 4 vertices x 3 neighbors each

    for (i, vertex_offsets) in offsets.iter().enumerate() {
        let side1 = registry.is_solid(neighborhood.get(
            face_pos.0 as i32 + vertex_offsets.side1.0,
            face_pos.1 as i32 + vertex_offsets.side1.1,
            face_pos.2 as i32 + vertex_offsets.side1.2,
        ));
        let side2 = registry.is_solid(neighborhood.get(
            face_pos.0 as i32 + vertex_offsets.side2.0,
            face_pos.1 as i32 + vertex_offsets.side2.1,
            face_pos.2 as i32 + vertex_offsets.side2.2,
        ));
        let corner = registry.is_solid(neighborhood.get(
            face_pos.0 as i32 + vertex_offsets.corner.0,
            face_pos.1 as i32 + vertex_offsets.corner.1,
            face_pos.2 as i32 + vertex_offsets.corner.2,
        ));

        ao[i] = vertex_ao(side1, side2, corner);
    }

    ao
}
```

### Neighbor Access

AO computation requires voxels beyond the chunk boundary (up to 1 voxel in any direction), which is why story 03's `ChunkNeighborhood` provides access to face, edge, and corner neighbors. The `get()` call with coordinates like `(-1, -1, z)` seamlessly resolves to edge neighbor data.

## Outcome

The `nebula_meshing` crate exports `vertex_ao()`, `compute_face_ao()`, and `should_flip_ao_diagonal()`. Each visible face gets 4 AO values (one per vertex), stored in the vertex data as a `u8` in the range 0-3. The greedy mesher splits quads at AO boundaries. The shader reads the AO value and dims the vertex accordingly. Running `cargo test -p nebula_meshing` passes all AO tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Block edges and corners darken subtly. A staircase pattern shows gradient shadows where blocks meet. The terrain gains visual depth without any lighting system.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `nebula_voxel` | workspace | `VoxelTypeRegistry` for solidity checks |
| `nebula_meshing` | workspace | `ChunkNeighborhood`, `FaceDirection` from prior stories |

No external crates required. AO is pure integer arithmetic on voxel neighbor lookups. The crate uses Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// A fully exposed vertex (no solid neighbors) should have AO = 0 (brightest).
    #[test]
    fn test_exposed_vertex_has_ao_zero() {
        let ao = vertex_ao(false, false, false);
        assert_eq!(ao, 0);
    }

    /// A vertex surrounded by 3 solid neighbors should have AO = 3 (darkest).
    #[test]
    fn test_corner_vertex_surrounded_by_three_solids_has_ao_three() {
        let ao = vertex_ao(true, true, true);
        assert_eq!(ao, 3);
    }

    /// When both sides are solid, AO is 3 regardless of the corner.
    #[test]
    fn test_both_sides_solid_gives_ao_three_regardless_of_corner() {
        assert_eq!(vertex_ao(true, true, false), 3);
        assert_eq!(vertex_ao(true, true, true), 3);
    }

    /// One side solid, corner empty: AO = 1.
    #[test]
    fn test_one_side_solid_ao_one() {
        assert_eq!(vertex_ao(true, false, false), 1);
        assert_eq!(vertex_ao(false, true, false), 1);
    }

    /// One side + corner: AO = 2.
    #[test]
    fn test_one_side_and_corner_ao_two() {
        assert_eq!(vertex_ao(true, false, true), 2);
        assert_eq!(vertex_ao(false, true, true), 2);
    }

    /// Corner only: AO = 1.
    #[test]
    fn test_corner_only_ao_one() {
        assert_eq!(vertex_ao(false, false, true), 1);
    }

    /// AO values are symmetric — side1 and side2 are interchangeable.
    #[test]
    fn test_ao_values_are_symmetric() {
        for s1 in [false, true] {
            for s2 in [false, true] {
                for c in [false, true] {
                    assert_eq!(
                        vertex_ao(s1, s2, c),
                        vertex_ao(s2, s1, c),
                        "AO not symmetric for side1={s1}, side2={s2}, corner={c}"
                    );
                }
            }
        }
    }

    /// All AO values must be in the range [0, 3].
    #[test]
    fn test_ao_values_in_valid_range() {
        for s1 in [false, true] {
            for s2 in [false, true] {
                for c in [false, true] {
                    let ao = vertex_ao(s1, s2, c);
                    assert!(ao <= 3, "AO value {ao} out of range for ({s1}, {s2}, {c})");
                }
            }
        }
    }

    /// Quad flip detection: when AO is uniform, no flip needed.
    #[test]
    fn test_uniform_ao_no_flip() {
        assert!(!should_flip_ao_diagonal([0, 0, 0, 0]));
        assert!(!should_flip_ao_diagonal([2, 2, 2, 2]));
    }

    /// Quad flip detection: anisotropic AO triggers flip when a+c > b+d.
    #[test]
    fn test_anisotropic_ao_triggers_flip() {
        // Default diagonal connects vertices 0 and 2.
        // ao[0]+ao[2] = 3+3 = 6 > ao[1]+ao[3] = 0+0 = 0 → flip.
        assert!(should_flip_ao_diagonal([3, 0, 3, 0]));
        // ao[0]+ao[2] = 0+0 = 0 < ao[1]+ao[3] = 3+3 = 6 → no flip.
        assert!(!should_flip_ao_diagonal([0, 3, 0, 3]));
    }

    /// Full face AO computation on a floor next to a wall:
    /// vertices near the wall should have higher AO than those away from it.
    #[test]
    fn test_face_ao_wall_edge_higher_than_open() {
        let mut neighborhood = ChunkNeighborhood::from_center_only(
            ChunkVoxelData::new_filled(32, VoxelType::AIR),
        );
        // Place a floor voxel at (5, 0, 5) and a wall voxel at (5, 1, 6)
        neighborhood.center_mut().set(5, 0, 5, VoxelType::STONE);
        neighborhood.center_mut().set(5, 1, 6, VoxelType::STONE);

        let registry = default_registry();
        let ao = compute_face_ao(&neighborhood, &registry, (5, 0, 5), FaceDirection::PosY);

        // Vertices near z=6 (the wall) should have higher AO than vertices near z=5
        let max_ao = *ao.iter().max().unwrap();
        let min_ao = *ao.iter().min().unwrap();
        assert!(max_ao > min_ao, "Wall-adjacent vertices should have higher AO");
    }
}
```
