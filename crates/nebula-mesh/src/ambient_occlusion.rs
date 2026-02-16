//! Per-vertex voxel ambient occlusion following the Mikola Lysenko algorithm.
//!
//! Each visible face has 4 vertices; for each vertex, three neighboring voxels
//! (two sides and one corner) are checked to compute an occlusion value 0–3.

use crate::face_direction::FaceDirection;
use crate::neighborhood::ChunkNeighborhood;
use nebula_voxel::VoxelTypeRegistry;

/// Neighbor offsets for a single vertex's AO calculation.
#[derive(Clone, Copy, Debug)]
pub struct VertexAoOffsets {
    /// Offset to the first side neighbor (relative to the voxel position).
    pub side1: (i32, i32, i32),
    /// Offset to the second side neighbor.
    pub side2: (i32, i32, i32),
    /// Offset to the diagonal corner neighbor.
    pub corner: (i32, i32, i32),
}

/// Compute the ambient occlusion value for a single vertex.
///
/// Returns a value in `0..=3` where:
/// - `0` = fully exposed (brightest)
/// - `1` = one neighbor solid
/// - `2` = two neighbors solid
/// - `3` = fully occluded (darkest)
pub fn vertex_ao(side1: bool, side2: bool, corner: bool) -> u8 {
    if side1 && side2 {
        3
    } else {
        (side1 as u8) + (side2 as u8) + (corner as u8)
    }
}

/// Determine whether to flip the quad diagonal based on AO values.
///
/// Returns `true` if the diagonal should be flipped to produce smoother
/// interpolation across the quad.
pub fn should_flip_ao_diagonal(ao: [u8; 4]) -> bool {
    ao[0] + ao[2] > ao[1] + ao[3]
}

/// Returns the 4 sets of AO neighbor offsets for vertices of a face.
///
/// Each face has 4 vertices; each vertex has 3 neighbor offsets (side1, side2, corner).
/// The offsets are relative to the voxel's own position in chunk-local coords.
///
/// Vertex order matches `push_quad`: corners are enumerated as
/// `(u, v)`, `(u+1, v)`, `(u+1, v+1)`, `(u, v+1)` in the face's UV space.
pub fn face_ao_offsets(direction: FaceDirection) -> [VertexAoOffsets; 4] {
    // For each face direction, the normal axis is fixed (e.g. PosY means
    // we're on top of the voxel). The 4 vertex corners sample neighbors
    // along the two tangent axes plus the normal.
    //
    // Convention: for PosY face (normal = +Y), the face sits at y+1.
    // Vertex (u,v) = (x,z) in chunk coords. Neighbors to check are at y+1
    // (one step in normal direction from the voxel).
    match direction {
        FaceDirection::PosY => [
            // vertex 0: (x, y+1, z) — bottom-left of face
            VertexAoOffsets {
                side1: (-1, 1, 0),
                side2: (0, 1, -1),
                corner: (-1, 1, -1),
            },
            // vertex 1: (x+1, y+1, z)
            VertexAoOffsets {
                side1: (1, 1, 0),
                side2: (0, 1, -1),
                corner: (1, 1, -1),
            },
            // vertex 2: (x+1, y+1, z+1)
            VertexAoOffsets {
                side1: (1, 1, 0),
                side2: (0, 1, 1),
                corner: (1, 1, 1),
            },
            // vertex 3: (x, y+1, z+1)
            VertexAoOffsets {
                side1: (-1, 1, 0),
                side2: (0, 1, 1),
                corner: (-1, 1, 1),
            },
        ],
        FaceDirection::NegY => [
            // Face at y=0 side, normal = -Y. Neighbors at y-1.
            VertexAoOffsets {
                side1: (-1, -1, 0),
                side2: (0, -1, -1),
                corner: (-1, -1, -1),
            },
            VertexAoOffsets {
                side1: (1, -1, 0),
                side2: (0, -1, -1),
                corner: (1, -1, -1),
            },
            VertexAoOffsets {
                side1: (1, -1, 0),
                side2: (0, -1, 1),
                corner: (1, -1, 1),
            },
            VertexAoOffsets {
                side1: (-1, -1, 0),
                side2: (0, -1, 1),
                corner: (-1, -1, 1),
            },
        ],
        FaceDirection::PosX => [
            // Face at x+1 side, normal = +X. Tangent axes: (Z, Y).
            VertexAoOffsets {
                side1: (1, 0, -1),
                side2: (1, -1, 0),
                corner: (1, -1, -1),
            },
            VertexAoOffsets {
                side1: (1, 0, 1),
                side2: (1, -1, 0),
                corner: (1, -1, 1),
            },
            VertexAoOffsets {
                side1: (1, 0, 1),
                side2: (1, 1, 0),
                corner: (1, 1, 1),
            },
            VertexAoOffsets {
                side1: (1, 0, -1),
                side2: (1, 1, 0),
                corner: (1, 1, -1),
            },
        ],
        FaceDirection::NegX => [
            VertexAoOffsets {
                side1: (-1, 0, -1),
                side2: (-1, -1, 0),
                corner: (-1, -1, -1),
            },
            VertexAoOffsets {
                side1: (-1, 0, 1),
                side2: (-1, -1, 0),
                corner: (-1, -1, 1),
            },
            VertexAoOffsets {
                side1: (-1, 0, 1),
                side2: (-1, 1, 0),
                corner: (-1, 1, 1),
            },
            VertexAoOffsets {
                side1: (-1, 0, -1),
                side2: (-1, 1, 0),
                corner: (-1, 1, -1),
            },
        ],
        FaceDirection::PosZ => [
            // Face at z+1, normal = +Z. Tangent axes: (X, Y).
            VertexAoOffsets {
                side1: (-1, 0, 1),
                side2: (0, -1, 1),
                corner: (-1, -1, 1),
            },
            VertexAoOffsets {
                side1: (1, 0, 1),
                side2: (0, -1, 1),
                corner: (1, -1, 1),
            },
            VertexAoOffsets {
                side1: (1, 0, 1),
                side2: (0, 1, 1),
                corner: (1, 1, 1),
            },
            VertexAoOffsets {
                side1: (-1, 0, 1),
                side2: (0, 1, 1),
                corner: (-1, 1, 1),
            },
        ],
        FaceDirection::NegZ => [
            VertexAoOffsets {
                side1: (-1, 0, -1),
                side2: (0, -1, -1),
                corner: (-1, -1, -1),
            },
            VertexAoOffsets {
                side1: (1, 0, -1),
                side2: (0, -1, -1),
                corner: (1, -1, -1),
            },
            VertexAoOffsets {
                side1: (1, 0, -1),
                side2: (0, 1, -1),
                corner: (1, 1, -1),
            },
            VertexAoOffsets {
                side1: (-1, 0, -1),
                side2: (0, 1, -1),
                corner: (-1, 1, -1),
            },
        ],
    }
}

/// Compute the 4 AO values for a face at the given position.
///
/// Each vertex gets an occlusion value from 0 (fully lit) to 3 (fully shadowed).
pub fn compute_face_ao(
    neighborhood: &ChunkNeighborhood,
    registry: &VoxelTypeRegistry,
    face_pos: (usize, usize, usize),
    direction: FaceDirection,
) -> [u8; 4] {
    let offsets = face_ao_offsets(direction);
    let (x, y, z) = (face_pos.0 as i32, face_pos.1 as i32, face_pos.2 as i32);
    let mut ao = [0u8; 4];

    for (i, vo) in offsets.iter().enumerate() {
        let s1 =
            registry.is_solid(neighborhood.get(x + vo.side1.0, y + vo.side1.1, z + vo.side1.2));
        let s2 =
            registry.is_solid(neighborhood.get(x + vo.side2.0, y + vo.side2.1, z + vo.side2.2));
        let c =
            registry.is_solid(neighborhood.get(x + vo.corner.0, y + vo.corner.1, z + vo.corner.2));
        ao[i] = vertex_ao(s1, s2, c);
    }

    ao
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_voxel::{ChunkData, Transparency, VoxelTypeDef, VoxelTypeId};

    fn default_registry() -> VoxelTypeRegistry {
        let mut reg = VoxelTypeRegistry::new();
        reg.register(VoxelTypeDef {
            name: "stone".to_string(),
            solid: true,
            transparency: Transparency::Opaque,
            material_index: 1,
            light_emission: 0,
        })
        .expect("register stone");
        reg
    }

    const STONE: VoxelTypeId = VoxelTypeId(1);

    #[test]
    fn test_exposed_vertex_has_ao_zero() {
        assert_eq!(vertex_ao(false, false, false), 0);
    }

    #[test]
    fn test_corner_vertex_surrounded_by_three_solids_has_ao_three() {
        assert_eq!(vertex_ao(true, true, true), 3);
    }

    #[test]
    fn test_both_sides_solid_gives_ao_three_regardless_of_corner() {
        assert_eq!(vertex_ao(true, true, false), 3);
        assert_eq!(vertex_ao(true, true, true), 3);
    }

    #[test]
    fn test_one_side_solid_ao_one() {
        assert_eq!(vertex_ao(true, false, false), 1);
        assert_eq!(vertex_ao(false, true, false), 1);
    }

    #[test]
    fn test_one_side_and_corner_ao_two() {
        assert_eq!(vertex_ao(true, false, true), 2);
        assert_eq!(vertex_ao(false, true, true), 2);
    }

    #[test]
    fn test_corner_only_ao_one() {
        assert_eq!(vertex_ao(false, false, true), 1);
    }

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

    #[test]
    fn test_uniform_ao_no_flip() {
        assert!(!should_flip_ao_diagonal([0, 0, 0, 0]));
        assert!(!should_flip_ao_diagonal([2, 2, 2, 2]));
    }

    #[test]
    fn test_anisotropic_ao_triggers_flip() {
        assert!(should_flip_ao_diagonal([3, 0, 3, 0]));
        assert!(!should_flip_ao_diagonal([0, 3, 0, 3]));
    }

    #[test]
    fn test_face_ao_all_air_is_zero() {
        let neighborhood = ChunkNeighborhood::from_center_only(ChunkData::new_air());
        let registry = default_registry();
        let ao = compute_face_ao(&neighborhood, &registry, (5, 5, 5), FaceDirection::PosY);
        assert_eq!(ao, [0, 0, 0, 0]);
    }

    #[test]
    fn test_face_ao_wall_edge_higher_than_open() {
        let mut chunk = ChunkData::new_air();
        // Floor voxel at (5, 0, 5) and a wall voxel at (5, 1, 6).
        chunk.set(5, 0, 5, STONE);
        chunk.set(5, 1, 6, STONE);

        let neighborhood = ChunkNeighborhood::from_center_only(chunk);
        let registry = default_registry();
        let ao = compute_face_ao(&neighborhood, &registry, (5, 0, 5), FaceDirection::PosY);

        let max_ao = ao.iter().copied().max().unwrap_or(0);
        let min_ao = ao.iter().copied().min().unwrap_or(0);
        assert!(
            max_ao > min_ao,
            "Wall-adjacent vertices should have higher AO, got {ao:?}"
        );
    }

    #[test]
    fn test_face_ao_surrounded_corner() {
        // Place solid blocks around a voxel to create a fully occluded corner.
        let mut chunk = ChunkData::new_air();
        // Target voxel at (5, 5, 5), face PosY.
        chunk.set(5, 5, 5, STONE);
        // Place solids at all AO-relevant positions for PosY vertex 0:
        // side1=(-1,1,0) → (4,6,5), side2=(0,1,-1) → (5,6,4), corner=(-1,1,-1) → (4,6,4)
        chunk.set(4, 6, 5, STONE);
        chunk.set(5, 6, 4, STONE);
        chunk.set(4, 6, 4, STONE);

        let neighborhood = ChunkNeighborhood::from_center_only(chunk);
        let registry = default_registry();
        let ao = compute_face_ao(&neighborhood, &registry, (5, 5, 5), FaceDirection::PosY);

        assert_eq!(ao[0], 3, "Vertex 0 should be fully occluded, got {}", ao[0]);
    }
}
