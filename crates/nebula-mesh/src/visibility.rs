//! Visible face detection: determines which voxel faces are exposed to air or
//! transparent neighbors and need geometry.

use nebula_voxel::{CHUNK_SIZE, ChunkData, VoxelTypeRegistry};

use crate::face_direction::FaceDirection;
use crate::neighborhood::ChunkNeighborhood;
use crate::visible_faces::VisibleFaces;

/// Computes per-voxel visible-face bitmasks for a chunk.
///
/// For each solid voxel, checks all six neighbors. A face is visible when its
/// neighbor is air or transparent (as determined by the registry). Two
/// transparent voxels of the **same** type hide their shared face to avoid
/// z-fighting; different transparent types both show the shared face.
///
/// Returns a flat `Vec` of length `CHUNK_SIZE³`, indexed the same way as
/// [`ChunkData`] (x varies fastest: `x + y * SIZE + z * SIZE * SIZE`).
pub fn compute_visible_faces(
    chunk: &ChunkData,
    neighbors: &ChunkNeighborhood,
    registry: &VoxelTypeRegistry,
) -> Vec<VisibleFaces> {
    let size = CHUNK_SIZE;
    let total = size * size * size;
    let mut result = vec![VisibleFaces::NONE; total];

    for z in 0..size {
        for y in 0..size {
            for x in 0..size {
                let voxel = chunk.get(x, y, z);
                if registry.is_air(voxel) {
                    continue;
                }

                let mut faces = VisibleFaces::NONE;
                let is_self_transparent = registry.is_transparent(voxel);

                for dir in FaceDirection::ALL {
                    let (nx, ny, nz) = dir.offset(x as i32, y as i32, z as i32);
                    let size_i = size as i32;

                    let neighbor_voxel = if nx >= 0
                        && nx < size_i
                        && ny >= 0
                        && ny < size_i
                        && nz >= 0
                        && nz < size_i
                    {
                        chunk.get(nx as usize, ny as usize, nz as usize)
                    } else {
                        neighbors.get(nx, ny, nz)
                    };

                    if registry.is_air(neighbor_voxel) {
                        // Air always exposes the face.
                        faces.set_visible(dir);
                    } else if is_self_transparent {
                        // Transparent self: hide face only if neighbor is same type.
                        if neighbor_voxel != voxel {
                            faces.set_visible(dir);
                        }
                    } else if registry.is_transparent(neighbor_voxel) {
                        // Opaque self next to transparent neighbor: face is visible.
                        faces.set_visible(dir);
                    }
                    // else: opaque self next to opaque neighbor → face hidden.
                }

                let idx = x + y * size + z * size * size;
                result[idx] = faces;
            }
        }
    }

    result
}

/// Counts the total number of visible faces in a visibility array.
pub fn count_visible_faces(faces: &[VisibleFaces]) -> u32 {
    faces.iter().map(|vf| vf.count()).sum()
}

/// Counts the total possible faces (6 per non-air voxel) in a chunk.
pub fn count_total_faces(chunk: &ChunkData, registry: &VoxelTypeRegistry) -> u32 {
    let size = CHUNK_SIZE;
    let mut count = 0u32;
    for z in 0..size {
        for y in 0..size {
            for x in 0..size {
                if !registry.is_air(chunk.get(x, y, z)) {
                    count += 6;
                }
            }
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use nebula_voxel::{Transparency, VoxelTypeDef, VoxelTypeId};

    use super::*;

    fn stone_def() -> VoxelTypeDef {
        VoxelTypeDef {
            name: "stone".to_string(),
            solid: true,
            transparency: Transparency::Opaque,
            material_index: 1,
            light_emission: 0,
        }
    }

    fn glass_def() -> VoxelTypeDef {
        VoxelTypeDef {
            name: "glass".to_string(),
            solid: true,
            transparency: Transparency::SemiTransparent,
            material_index: 2,
            light_emission: 0,
        }
    }

    fn water_def() -> VoxelTypeDef {
        VoxelTypeDef {
            name: "water".to_string(),
            solid: false,
            transparency: Transparency::SemiTransparent,
            material_index: 3,
            light_emission: 0,
        }
    }

    fn test_registry() -> (VoxelTypeRegistry, VoxelTypeId, VoxelTypeId, VoxelTypeId) {
        let mut reg = VoxelTypeRegistry::new();
        let stone = reg.register(stone_def()).expect("register stone");
        let glass = reg.register(glass_def()).expect("register glass");
        let water = reg.register(water_def()).expect("register water");
        (reg, stone, glass, water)
    }

    #[test]
    fn test_single_voxel_in_empty_chunk_has_six_visible_faces() {
        let (reg, stone, _, _) = test_registry();
        let mut chunk = ChunkData::new_air();
        chunk.set(16, 16, 16, stone);
        let neighbors = ChunkNeighborhood::all_air();

        let faces = compute_visible_faces(&chunk, &neighbors, &reg);
        let idx = 16 + 16 * CHUNK_SIZE + 16 * CHUNK_SIZE * CHUNK_SIZE;
        let vf = faces[idx];

        assert_eq!(vf.count(), 6);
        assert_eq!(vf, VisibleFaces::ALL);
    }

    #[test]
    fn test_two_adjacent_solid_voxels_share_hidden_face() {
        let (reg, stone, _, _) = test_registry();
        let mut chunk = ChunkData::new_air();
        chunk.set(10, 10, 10, stone);
        chunk.set(11, 10, 10, stone);
        let neighbors = ChunkNeighborhood::all_air();

        let faces = compute_visible_faces(&chunk, &neighbors, &reg);

        let idx_a = 10 + 10 * CHUNK_SIZE + 10 * CHUNK_SIZE * CHUNK_SIZE;
        let idx_b = 11 + 10 * CHUNK_SIZE + 10 * CHUNK_SIZE * CHUNK_SIZE;
        let vf_a = faces[idx_a];
        let vf_b = faces[idx_b];

        assert!(!vf_a.is_visible(FaceDirection::PosX));
        assert!(!vf_b.is_visible(FaceDirection::NegX));
        assert_eq!(vf_a.count(), 5);
        assert_eq!(vf_b.count(), 5);
    }

    #[test]
    fn test_transparent_voxel_does_not_hide_neighbor_faces() {
        let (reg, stone, glass, _) = test_registry();
        let mut chunk = ChunkData::new_air();
        chunk.set(10, 10, 10, stone);
        chunk.set(11, 10, 10, glass);
        let neighbors = ChunkNeighborhood::all_air();

        let faces = compute_visible_faces(&chunk, &neighbors, &reg);

        let idx = 10 + 10 * CHUNK_SIZE + 10 * CHUNK_SIZE * CHUNK_SIZE;
        let vf = faces[idx];
        assert!(vf.is_visible(FaceDirection::PosX));
        assert_eq!(vf.count(), 6);
    }

    #[test]
    fn test_same_transparent_type_hides_shared_face() {
        let (reg, _, glass, _) = test_registry();
        let mut chunk = ChunkData::new_air();
        chunk.set(10, 10, 10, glass);
        chunk.set(11, 10, 10, glass);
        let neighbors = ChunkNeighborhood::all_air();

        let faces = compute_visible_faces(&chunk, &neighbors, &reg);

        let idx_a = 10 + 10 * CHUNK_SIZE + 10 * CHUNK_SIZE * CHUNK_SIZE;
        let idx_b = 11 + 10 * CHUNK_SIZE + 10 * CHUNK_SIZE * CHUNK_SIZE;
        assert!(!faces[idx_a].is_visible(FaceDirection::PosX));
        assert!(!faces[idx_b].is_visible(FaceDirection::NegX));
    }

    #[test]
    fn test_different_transparent_types_show_shared_face() {
        let (reg, _, glass, water) = test_registry();
        let mut chunk = ChunkData::new_air();
        chunk.set(10, 10, 10, glass);
        chunk.set(11, 10, 10, water);
        let neighbors = ChunkNeighborhood::all_air();

        let faces = compute_visible_faces(&chunk, &neighbors, &reg);

        let idx_a = 10 + 10 * CHUNK_SIZE + 10 * CHUNK_SIZE * CHUNK_SIZE;
        let idx_b = 11 + 10 * CHUNK_SIZE + 10 * CHUNK_SIZE * CHUNK_SIZE;
        assert!(faces[idx_a].is_visible(FaceDirection::PosX));
        assert!(faces[idx_b].is_visible(FaceDirection::NegX));
    }

    #[test]
    fn test_boundary_voxel_queries_adjacent_chunk() {
        let (reg, stone, _, _) = test_registry();
        let mut chunk = ChunkData::new_air();
        chunk.set(0, 10, 10, stone);

        let mut neg_x_chunk = ChunkData::new_air();
        neg_x_chunk.set(31, 10, 10, stone);
        let neighbors = ChunkNeighborhood::with_neg_x(neg_x_chunk);

        let faces = compute_visible_faces(&chunk, &neighbors, &reg);
        let idx = 10 * CHUNK_SIZE + 10 * CHUNK_SIZE * CHUNK_SIZE;
        let vf = faces[idx];

        assert!(!vf.is_visible(FaceDirection::NegX));
        assert_eq!(vf.count(), 5);
    }

    #[test]
    fn test_empty_chunk_produces_zero_faces() {
        let (reg, _, _, _) = test_registry();
        let chunk = ChunkData::new_air();
        let neighbors = ChunkNeighborhood::all_air();

        let faces = compute_visible_faces(&chunk, &neighbors, &reg);

        for vf in &faces {
            assert_eq!(vf.count(), 0);
        }
    }

    #[test]
    fn test_air_voxel_has_no_visible_faces() {
        let (reg, _, _, _) = test_registry();
        let chunk = ChunkData::new_air();
        let neighbors = ChunkNeighborhood::all_air();

        let faces = compute_visible_faces(&chunk, &neighbors, &reg);
        assert!(faces.iter().all(|vf| *vf == VisibleFaces::NONE));
    }

    #[test]
    fn test_count_visible_faces_helper() {
        let (reg, stone, _, _) = test_registry();
        let mut chunk = ChunkData::new_air();
        chunk.set(16, 16, 16, stone);
        let neighbors = ChunkNeighborhood::all_air();

        let faces = compute_visible_faces(&chunk, &neighbors, &reg);
        assert_eq!(count_visible_faces(&faces), 6);
    }

    #[test]
    fn test_count_total_faces_helper() {
        let (reg, stone, _, _) = test_registry();
        let mut chunk = ChunkData::new_air();
        chunk.set(0, 0, 0, stone);
        chunk.set(1, 0, 0, stone);
        assert_eq!(count_total_faces(&chunk, &reg), 12);
    }
}
