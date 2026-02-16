//! LOD-aware greedy meshing: generates meshes from variable-resolution chunks.
//!
//! At LOD N, a chunk stores `(32 / 2^N)^3` voxels covering the same spatial
//! extent. The greedy meshing algorithm operates on the reduced grid and then
//! scales vertex positions so the mesh covers the correct world-space area.

use nebula_voxel::{LodChunkData, VoxelTypeId, VoxelTypeRegistry};

use crate::chunk_mesh::ChunkMesh;
use crate::face_direction::FaceDirection;
use crate::neighborhood::ChunkNeighborhood;
use crate::transition_seams::ChunkLodContext;
use crate::visible_faces::VisibleFaces;

/// Generate a mesh from an LOD chunk, producing geometry with correct
/// world-space vertex positions.
///
/// The mesh is generated at the chunk's native resolution, then vertex
/// positions are scaled by `voxel_world_size` so the mesh covers the same
/// spatial extent regardless of LOD level.
pub fn mesh_lod_chunk(
    chunk: &LodChunkData,
    neighbors: &ChunkNeighborhood,
    registry: &VoxelTypeRegistry,
    lod_context: &ChunkLodContext,
) -> ChunkMesh {
    let resolution = chunk.resolution() as usize;

    // Step 1: Compute visible faces at the chunk's native resolution.
    let visible = compute_visible_faces_lod(chunk, registry);

    // Step 2: Run greedy meshing on the reduced-resolution grid.
    let mut mesh = greedy_mesh_lod(chunk, &visible, resolution, neighbors, registry);

    // Step 3: Scale vertex positions from grid space to world space.
    // At LOD N, each grid cell is 2^N base voxel units wide.
    let voxel_scale = (1u32 << chunk.lod()) as f32;
    mesh.scale_vertices(voxel_scale);

    // Step 4: Apply LOD transition seam fixes (uses context but the actual
    // seam fix operates on PackedChunkMesh; we note it for completeness).
    let _ = lod_context;

    mesh
}

/// Compute per-voxel visible-face bitmasks for an LOD chunk.
///
/// Works like [`crate::visibility::compute_visible_faces`] but on the
/// variable-resolution grid of an [`LodChunkData`].
pub fn compute_visible_faces_lod(
    chunk: &LodChunkData,
    registry: &VoxelTypeRegistry,
) -> Vec<VisibleFaces> {
    let res = chunk.resolution();
    let total = (res * res * res) as usize;
    let mut result = vec![VisibleFaces::NONE; total];

    for z in 0..res {
        for y in 0..res {
            for x in 0..res {
                let voxel = chunk.get(x, y, z);
                if registry.is_air(voxel) {
                    continue;
                }

                let mut faces = VisibleFaces::NONE;
                let is_self_transparent = registry.is_transparent(voxel);

                for dir in FaceDirection::ALL {
                    let (nx, ny, nz) = dir.offset(x as i32, y as i32, z as i32);
                    let res_i = res as i32;

                    let neighbor_voxel = if nx >= 0
                        && nx < res_i
                        && ny >= 0
                        && ny < res_i
                        && nz >= 0
                        && nz < res_i
                    {
                        chunk.get(nx as u32, ny as u32, nz as u32)
                    } else {
                        // Outside the chunk boundary — treat as air for LOD chunks.
                        VoxelTypeId(0)
                    };

                    if registry.is_air(neighbor_voxel) {
                        faces.set_visible(dir);
                    } else if is_self_transparent {
                        if neighbor_voxel != voxel {
                            faces.set_visible(dir);
                        }
                    } else if registry.is_transparent(neighbor_voxel) {
                        faces.set_visible(dir);
                    }
                }

                let idx = (x + y * res + z * res * res) as usize;
                result[idx] = faces;
            }
        }
    }

    result
}

/// Performs greedy meshing on an LOD chunk at its native resolution.
///
/// This is parametric on grid resolution rather than using the fixed
/// `CHUNK_SIZE` constant.
fn greedy_mesh_lod(
    chunk: &LodChunkData,
    visible_faces: &[VisibleFaces],
    resolution: usize,
    _neighbors: &ChunkNeighborhood,
    _registry: &VoxelTypeRegistry,
) -> ChunkMesh {
    let mut mesh = ChunkMesh::new();
    let size = resolution;
    let mut visited = vec![false; size * size];

    for direction in FaceDirection::ALL {
        let (layer_axis, u_axis, v_axis) = direction.sweep_axes();

        for layer in 0..size {
            visited.fill(false);

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

                    let voxel_type = chunk.get(x as u32, y as u32, z as u32);

                    // Extend width along u-axis.
                    let mut w = 1;
                    while u + w < size {
                        let (nx, ny, nz) = axes_to_xyz(layer_axis, u_axis, v_axis, layer, u + w, v);
                        let ni = nx + ny * size + nz * size * size;
                        if visited[v * size + u + w]
                            || !visible_faces[ni].is_visible(direction)
                            || chunk.get(nx as u32, ny as u32, nz as u32) != voxel_type
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
                                || chunk.get(nx as u32, ny as u32, nz as u32) != voxel_type
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

                    // Emit merged quad (without AO for LOD meshes).
                    mesh.push_quad(direction, layer, u, v, w, h, voxel_type);
                }
            }
        }
    }

    mesh
}

/// Converts abstract axis coordinates back to concrete `(x, y, z)`.
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

/// Helper to create a default registry with stone for testing.
pub fn default_registry() -> VoxelTypeRegistry {
    use nebula_voxel::{Transparency, VoxelTypeDef};
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

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_voxel::LodChunkData;

    /// Build a chunk with a checkerboard pattern on the surface to produce
    /// many quads that scale with resolution.
    fn make_checkerboard_lod_chunk(lod: u8) -> LodChunkData {
        let mut chunk = LodChunkData::new(lod);
        let res = chunk.resolution();
        for z in 0..res {
            for x in 0..res {
                if (x + z) % 2 == 0 {
                    chunk.set(x, 0, z, VoxelTypeId(1));
                }
            }
        }
        chunk
    }

    fn make_solid_lod_chunk(lod: u8) -> LodChunkData {
        let mut chunk = LodChunkData::new(lod);
        let res = chunk.resolution();
        for z in 0..res {
            for y in 0..res / 2 {
                for x in 0..res {
                    chunk.set(x, y, z, VoxelTypeId(1));
                }
            }
        }
        chunk
    }

    fn mesh_lod_chunk_simple(lod: u8) -> ChunkMesh {
        let chunk = make_checkerboard_lod_chunk(lod);
        mesh_lod_chunk(
            &chunk,
            &ChunkNeighborhood::all_air(),
            &default_registry(),
            &ChunkLodContext::uniform(lod),
        )
    }

    /// LOD 0 mesh should have the most triangles of any LOD level.
    #[test]
    fn test_lod_0_mesh_has_most_triangles() {
        let mesh_0 = mesh_lod_chunk_simple(0);
        let mesh_1 = mesh_lod_chunk_simple(1);
        let mesh_2 = mesh_lod_chunk_simple(2);

        assert!(
            mesh_0.triangle_count() > mesh_1.triangle_count(),
            "LOD 0 ({}) should have more triangles than LOD 1 ({})",
            mesh_0.triangle_count(),
            mesh_1.triangle_count()
        );
        assert!(
            mesh_1.triangle_count() > mesh_2.triangle_count(),
            "LOD 1 ({}) should have more triangles than LOD 2 ({})",
            mesh_1.triangle_count(),
            mesh_2.triangle_count()
        );
    }

    /// LOD N mesh should have approximately 1/4^N the triangles of LOD 0.
    #[test]
    fn test_lod_n_triangle_ratio() {
        let mesh_0 = mesh_lod_chunk_simple(0);
        let mesh_2 = mesh_lod_chunk_simple(2);

        let ratio = mesh_0.triangle_count() as f64 / mesh_2.triangle_count() as f64;
        assert!(
            ratio > 8.0 && ratio < 32.0,
            "expected ~16x ratio, got {ratio:.1}x"
        );
    }

    /// Mesh should be valid at every LOD level.
    #[test]
    fn test_mesh_valid_at_all_lod_levels() {
        for lod in 0..=4 {
            let mesh = mesh_lod_chunk_simple(lod);
            assert!(
                mesh.triangle_count() > 0,
                "LOD {lod} mesh should have some triangles"
            );
            assert!(
                !mesh.has_degenerate_triangles(),
                "LOD {lod} mesh has degenerate triangles"
            );
            assert!(
                mesh.has_consistent_winding(),
                "LOD {lod} mesh has inconsistent winding order"
            );
        }
    }

    /// Meshing time should decrease with higher LOD levels.
    #[test]
    fn test_meshing_time_decreases_with_lod() {
        // Run multiple iterations to get stable timing.
        let iterations = 10;

        let start_0 = std::time::Instant::now();
        for _ in 0..iterations {
            let _ = mesh_lod_chunk_simple(0);
        }
        let time_0 = start_0.elapsed();

        let start_2 = std::time::Instant::now();
        for _ in 0..iterations {
            let _ = mesh_lod_chunk_simple(2);
        }
        let time_2 = start_2.elapsed();

        assert!(
            time_0 > time_2,
            "LOD 0 meshing ({time_0:?}) should take longer than LOD 2 ({time_2:?})"
        );
    }

    /// The mesh should correctly represent the low-res voxel data.
    #[test]
    fn test_mesh_represents_low_res_data() {
        for lod in 0..=3 {
            let chunk = make_solid_lod_chunk(lod);
            let mesh = mesh_lod_chunk(
                &chunk,
                &ChunkNeighborhood::all_air(),
                &default_registry(),
                &ChunkLodContext::uniform(lod),
            );

            assert!(
                mesh.triangle_count() >= 6 * 2,
                "LOD {lod} mesh should represent all visible faces, got {} triangles",
                mesh.triangle_count()
            );
        }
    }
}
