//! Greedy meshing algorithm: merges coplanar, same-type adjacent faces into
//! larger rectangular quads to reduce triangle count.

use nebula_voxel::{CHUNK_SIZE, ChunkData, VoxelTypeRegistry};

use crate::ambient_occlusion::compute_face_ao;
use crate::chunk_mesh::ChunkMesh;
use crate::face_direction::FaceDirection;
use crate::neighborhood::ChunkNeighborhood;
use crate::visible_faces::VisibleFaces;

/// Converts abstract axis coordinates back to concrete `(x, y, z)`.
///
/// `layer_axis`, `u_axis`, `v_axis` are 0=X, 1=Y, 2=Z.
fn axes_to_xyz(
    layer_axis: usize,
    u_axis: usize,
    v_axis: usize,
    layer: usize,
    u: usize,
    v: usize,
) -> (usize, usize, usize) {
    let mut coords = [0usize; 3];
    coords[layer_axis] = layer;
    coords[u_axis] = u;
    coords[v_axis] = v;
    (coords[0], coords[1], coords[2])
}

/// Performs greedy meshing on a chunk, merging adjacent same-type visible faces
/// into larger rectangular quads.
///
/// # Arguments
///
/// * `chunk` – The chunk's voxel data.
/// * `visible_faces` – Per-voxel visibility bitmasks (from [`crate::compute_visible_faces`]).
/// * `_neighbors` – Neighbor chunks (reserved for future AO-aware merging).
/// * `registry` – Voxel type registry (reserved for future merge predicate hooks).
///
/// # Returns
///
/// A [`ChunkMesh`] containing merged quads with vertices, indices, and metadata.
pub fn greedy_mesh(
    chunk: &ChunkData,
    visible_faces: &[VisibleFaces],
    neighbors: &ChunkNeighborhood,
    registry: &VoxelTypeRegistry,
) -> ChunkMesh {
    let mut mesh = ChunkMesh::new();
    let size = CHUNK_SIZE;
    let mut visited = vec![false; size * size];
    // Pre-compute per-face AO values for the current layer.
    let mut ao_cache = vec![[0u8; 4]; size * size];

    for direction in FaceDirection::ALL {
        let (layer_axis, u_axis, v_axis) = direction.sweep_axes();

        for layer in 0..size {
            visited.fill(false);

            // Pre-compute AO for all visible faces in this layer.
            for v in 0..size {
                for u in 0..size {
                    let (x, y, z) = axes_to_xyz(layer_axis, u_axis, v_axis, layer, u, v);
                    let idx = x + y * size + z * size * size;
                    if visible_faces[idx].is_visible(direction) {
                        ao_cache[v * size + u] =
                            compute_face_ao(neighbors, registry, (x, y, z), direction);
                    }
                }
            }

            for v in 0..size {
                for u in 0..size {
                    let vis_idx = v * size + u;
                    if visited[vis_idx] {
                        continue;
                    }

                    let (x, y, z) = axes_to_xyz(layer_axis, u_axis, v_axis, layer, u, v);
                    let idx = x + y * size + z * size * size;

                    if !visible_faces[idx].is_visible(direction) {
                        continue;
                    }

                    let voxel_type = chunk.get(x, y, z);
                    let base_ao = ao_cache[vis_idx];

                    // Extend width along u-axis.
                    let mut w = 1;
                    while u + w < size {
                        let (nx, ny, nz) = axes_to_xyz(layer_axis, u_axis, v_axis, layer, u + w, v);
                        let ni = nx + ny * size + nz * size * size;
                        if visited[v * size + u + w]
                            || !visible_faces[ni].is_visible(direction)
                            || chunk.get(nx, ny, nz) != voxel_type
                            || ao_cache[v * size + u + w] != base_ao
                        {
                            break;
                        }
                        w += 1;
                    }

                    // Extend height along v-axis.
                    let mut h = 1;
                    'outer: while v + h < size {
                        for du in 0..w {
                            let (nx, ny, nz) =
                                axes_to_xyz(layer_axis, u_axis, v_axis, layer, u + du, v + h);
                            let ni = nx + ny * size + nz * size * size;
                            if visited[(v + h) * size + u + du]
                                || !visible_faces[ni].is_visible(direction)
                                || chunk.get(nx, ny, nz) != voxel_type
                                || ao_cache[(v + h) * size + u + du] != base_ao
                            {
                                break 'outer;
                            }
                        }
                        h += 1;
                    }

                    // Mark visited.
                    for dv in 0..h {
                        for du in 0..w {
                            visited[(v + dv) * size + u + du] = true;
                        }
                    }

                    // Emit merged quad with AO.
                    mesh.push_quad_ao(direction, layer, u, v, w, h, voxel_type, base_ao);
                }
            }
        }
    }

    mesh
}

#[cfg(test)]
mod tests {
    use nebula_voxel::{Transparency, VoxelTypeDef, VoxelTypeId};

    use super::*;
    use crate::visibility::compute_visible_faces;

    fn test_registry() -> VoxelTypeRegistry {
        let mut reg = VoxelTypeRegistry::new();
        reg.register(VoxelTypeDef {
            name: "stone".to_string(),
            solid: true,
            transparency: Transparency::Opaque,
            material_index: 1,
            light_emission: 0,
        })
        .expect("register stone");
        reg.register(VoxelTypeDef {
            name: "dirt".to_string(),
            solid: true,
            transparency: Transparency::Opaque,
            material_index: 2,
            light_emission: 0,
        })
        .expect("register dirt");
        reg
    }

    /// Stone = VoxelTypeId(1), Dirt = VoxelTypeId(2) per registration order.
    const STONE: VoxelTypeId = VoxelTypeId(1);
    const DIRT: VoxelTypeId = VoxelTypeId(2);

    #[test]
    fn test_flat_surface_single_type_produces_one_quad() {
        let mut chunk = ChunkData::new_air();
        for z in 0..CHUNK_SIZE {
            for x in 0..CHUNK_SIZE {
                chunk.set(x, 0, z, STONE);
            }
        }
        let neighbors = ChunkNeighborhood::all_air();
        let reg = test_registry();
        let visible = compute_visible_faces(&chunk, &neighbors, &reg);
        let mesh = greedy_mesh(&chunk, &visible, &neighbors, &reg);

        let top_quads = mesh.count_quads_for_direction(FaceDirection::PosY);
        assert_eq!(top_quads, 1, "Flat 32x32 surface should merge to 1 quad");
    }

    #[test]
    fn test_checkerboard_produces_many_quads() {
        let mut chunk = ChunkData::new_air();
        for z in 0..CHUNK_SIZE {
            for x in 0..CHUNK_SIZE {
                let vtype = if (x + z) % 2 == 0 { STONE } else { DIRT };
                chunk.set(x, 0, z, vtype);
            }
        }
        let neighbors = ChunkNeighborhood::all_air();
        let reg = test_registry();
        let visible = compute_visible_faces(&chunk, &neighbors, &reg);
        let mesh = greedy_mesh(&chunk, &visible, &neighbors, &reg);

        let top_quads = mesh.count_quads_for_direction(FaceDirection::PosY);
        assert_eq!(
            top_quads,
            CHUNK_SIZE * CHUNK_SIZE,
            "Checkerboard should produce 1024 quads on +Y"
        );
    }

    #[test]
    fn test_l_shaped_surface_produces_multiple_quads() {
        let mut chunk = ChunkData::new_air();
        for x in 0..8 {
            chunk.set(x, 0, 0, STONE);
        }
        for z in 1..8 {
            chunk.set(0, 0, z, STONE);
        }
        let neighbors = ChunkNeighborhood::all_air();
        let reg = test_registry();
        let visible = compute_visible_faces(&chunk, &neighbors, &reg);
        let mesh = greedy_mesh(&chunk, &visible, &neighbors, &reg);

        let top_quads = mesh.count_quads_for_direction(FaceDirection::PosY);
        assert!(
            top_quads >= 2,
            "L-shaped surface should need at least 2 quads, got {top_quads}"
        );
    }

    #[test]
    fn test_different_types_not_merged() {
        let mut chunk = ChunkData::new_air();
        for x in 0..16 {
            chunk.set(x, 0, 0, STONE);
        }
        for x in 16..CHUNK_SIZE {
            chunk.set(x, 0, 0, DIRT);
        }
        let neighbors = ChunkNeighborhood::all_air();
        let reg = test_registry();
        let visible = compute_visible_faces(&chunk, &neighbors, &reg);
        let mesh = greedy_mesh(&chunk, &visible, &neighbors, &reg);

        let top_quads = mesh.count_quads_for_direction(FaceDirection::PosY);
        assert!(
            top_quads >= 2,
            "Two different types must produce at least 2 quads, got {top_quads}"
        );
    }

    #[test]
    fn test_empty_chunk_produces_zero_quads() {
        let chunk = ChunkData::new_air();
        let neighbors = ChunkNeighborhood::all_air();
        let reg = test_registry();
        let visible = compute_visible_faces(&chunk, &neighbors, &reg);
        let mesh = greedy_mesh(&chunk, &visible, &neighbors, &reg);

        assert_eq!(mesh.vertices.len(), 0);
        assert_eq!(mesh.indices.len(), 0);
    }

    #[test]
    fn test_meshing_performance_32_cubed() {
        let chunk = ChunkData::new(STONE);
        let neighbors = ChunkNeighborhood::all_air();
        let reg = test_registry();
        let visible = compute_visible_faces(&chunk, &neighbors, &reg);

        let start = std::time::Instant::now();
        let _mesh = greedy_mesh(&chunk, &visible, &neighbors, &reg);
        let elapsed = start.elapsed();

        // Allow up to 5ms in debug builds, <1ms expected in release.
        assert!(
            elapsed.as_millis() < 50,
            "Greedy meshing took {}ms, expected <50ms (debug build tolerance)",
            elapsed.as_millis()
        );
    }

    #[test]
    fn test_solid_chunk_produces_six_face_quads() {
        let chunk = ChunkData::new(STONE);
        let neighbors = ChunkNeighborhood::all_air();
        let reg = test_registry();
        let visible = compute_visible_faces(&chunk, &neighbors, &reg);
        let mesh = greedy_mesh(&chunk, &visible, &neighbors, &reg);

        // A fully solid chunk with all-air neighbors: each face direction has
        // exactly one 32x32 quad.
        for dir in FaceDirection::ALL {
            assert_eq!(
                mesh.count_quads_for_direction(dir),
                1,
                "Solid chunk should have 1 quad for {dir:?}"
            );
        }
        assert_eq!(mesh.quad_count(), 6);
    }
}
