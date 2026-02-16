//! Single cube-face terrain loading, meshing, and displacement.
//!
//! Composes the cubesphere geometry, voxel storage, greedy meshing, terrain
//! generation, and cubesphere displacement into a complete pipeline that
//! produces GPU-ready vertex data for one cube face.

use glam::{Mat4, Vec3};
use nebula_cubesphere::{
    ChunkAddress as CsChunkAddress, CubeFace, FaceCoord, face_coord_to_cube_point,
    face_coord_to_sphere_everitt,
};
use nebula_mesh::visibility::compute_visible_faces;
use nebula_mesh::{ChunkMesh, ChunkNeighborhood, greedy_mesh};
use nebula_render::VertexPositionColor;
use nebula_terrain::{HeightmapParams, TerrainHeightConfig, TerrainHeightSampler};
use nebula_voxel::{ChunkData, Transparency, VoxelTypeDef, VoxelTypeId, VoxelTypeRegistry};
use tracing::info;

/// A meshed chunk with its cubesphere address, ready for displacement.
pub struct FaceChunkMesh {
    /// The cubesphere chunk address.
    pub address: CsChunkAddress,
    /// The greedy-meshed chunk data.
    pub mesh: ChunkMesh,
}

/// Configuration and state for loading a single cube face of terrain.
pub struct SingleFaceLoader {
    /// Which cube face to load.
    pub face: CubeFace,
    /// How many chunks outward from the face center to load.
    pub load_radius: u32,
    /// LOD level for the chunks (determines grid size).
    pub lod: u8,
    /// Terrain height sampler for generating voxel data.
    pub terrain: TerrainHeightSampler,
    /// Voxel type registry with terrain materials.
    pub registry: VoxelTypeRegistry,
    /// Planet radius in meters.
    pub planet_radius: f64,
    /// Voxel size in meters.
    pub voxel_size: f64,
}

impl SingleFaceLoader {
    /// Create a loader with sensible defaults for a small demo planet.
    ///
    /// Uses a small planet radius (1000m) so curvature is visible.
    pub fn new_demo(face: CubeFace, load_radius: u32, seed: u64) -> Self {
        let planet_radius = 200.0;
        let voxel_size = 1.0;
        let terrain = TerrainHeightSampler::new(
            HeightmapParams {
                seed,
                octaves: 4,
                amplitude: 8.0,
                base_frequency: 0.05,
                ..Default::default()
            },
            TerrainHeightConfig {
                min_height: -4.0,
                max_height: 12.0,
                sea_level: 0.0,
                planet_radius,
            },
        );

        let mut registry = VoxelTypeRegistry::new();
        registry
            .register(VoxelTypeDef {
                name: "stone".into(),
                solid: true,
                transparency: Transparency::Opaque,
                material_index: 1,
                light_emission: 0,
            })
            .expect("register stone");
        registry
            .register(VoxelTypeDef {
                name: "dirt".into(),
                solid: true,
                transparency: Transparency::Opaque,
                material_index: 2,
                light_emission: 0,
            })
            .expect("register dirt");
        registry
            .register(VoxelTypeDef {
                name: "grass".into(),
                solid: true,
                transparency: Transparency::Opaque,
                material_index: 3,
                light_emission: 0,
            })
            .expect("register grass");

        Self {
            face,
            load_radius,
            lod: 17, // grid_size = 2^20 >> 17 = 8 chunks per axis — large chunks for visibility
            terrain,
            registry,
            planet_radius,
            voxel_size,
        }
    }

    /// Load and mesh all chunks within the load radius.
    ///
    /// Returns a list of meshed chunks ready for displacement.
    pub fn load_and_mesh(&self) -> Vec<FaceChunkMesh> {
        let grid_size = CsChunkAddress::grid_size(self.lod);
        let center_x = grid_size / 2;
        let center_y = grid_size / 2;
        let r = self.load_radius as i64;

        let mut results = Vec::new();

        for dx in -r..=r {
            for dy in -r..=r {
                let cx = center_x as i64 + dx;
                let cy = center_y as i64 + dy;
                if cx < 0 || cy < 0 || cx >= grid_size as i64 || cy >= grid_size as i64 {
                    continue;
                }

                let addr = CsChunkAddress::new(self.face, self.lod, cx as u32, cy as u32);
                let chunk_data = self.generate_chunk(&addr);
                let neighbors = ChunkNeighborhood::all_air();
                let visible = compute_visible_faces(&chunk_data, &neighbors, &self.registry);
                let mesh = greedy_mesh(&chunk_data, &visible, &neighbors, &self.registry);

                if !mesh.vertices.is_empty() {
                    results.push(FaceChunkMesh {
                        address: addr,
                        mesh,
                    });
                }
            }
        }

        info!(
            "Loaded {} non-empty chunks for face {:?} (radius {})",
            results.len(),
            self.face,
            self.load_radius
        );
        results
    }

    /// Generate voxel data for one chunk by sampling terrain height.
    fn generate_chunk(&self, addr: &CsChunkAddress) -> ChunkData {
        let (u_min, v_min, u_max, v_max) = addr.uv_bounds();
        let u_size = u_max - u_min;
        let v_size = v_max - v_min;
        let chunk_size = 32_usize;

        let stone = VoxelTypeId(1);
        let dirt = VoxelTypeId(2);
        let grass = VoxelTypeId(3);

        let mut chunk = ChunkData::new_air();

        for z in 0..chunk_size {
            for x in 0..chunk_size {
                // Map chunk-local (x, z) to face UV
                let face_u = u_min + (x as f64 + 0.5) / chunk_size as f64 * u_size;
                let face_v = v_min + (z as f64 + 0.5) / chunk_size as f64 * v_size;
                let fc = FaceCoord::new(self.face, face_u, face_v);
                let sphere_pt = face_coord_to_sphere_everitt(&fc);

                let height = self.terrain.sample_height(sphere_pt);
                // Map height to voxel Y. We center at y=16 as "sea level".
                let surface_y = (16.0 + height / self.voxel_size).round() as i32;
                let surface_y = surface_y.clamp(0, 31) as usize;

                for y in 0..chunk_size {
                    if y < surface_y.saturating_sub(3) {
                        chunk.set(x, y, z, stone);
                    } else if y < surface_y {
                        chunk.set(x, y, z, dirt);
                    } else if y == surface_y {
                        chunk.set(x, y, z, grass);
                    }
                    // else air
                }
            }
        }

        chunk
    }

    /// Total number of chunks that would be loaded.
    pub fn expected_chunk_count(&self) -> u32 {
        let side = 2 * self.load_radius + 1;
        side * side
    }
}

/// GPU-ready render data for a single face.
pub struct SingleFaceRenderData {
    /// Flat list of vertices with position and color, displaced onto the sphere.
    pub vertices: Vec<VertexPositionColor>,
    /// Index buffer (u32).
    pub indices: Vec<u32>,
}

/// Build GPU-ready render data from meshed chunks by applying cubesphere displacement.
///
/// Each vertex is displaced from flat chunk-local space onto the planet's curved
/// surface. Colors are assigned based on voxel material:
/// - stone (1) = gray
/// - dirt (2)  = brown
/// - grass (3) = green
pub fn build_face_render_data(
    chunks: &[FaceChunkMesh],
    planet_radius: f64,
    voxel_size: f64,
) -> SingleFaceRenderData {
    let mut all_vertices = Vec::new();
    let mut all_indices = Vec::new();

    for chunk in chunks {
        let base_vertex = all_vertices.len() as u32;
        let (u_min, v_min, u_max, v_max) = chunk.address.uv_bounds();
        let u_size = u_max - u_min;
        let v_size = v_max - v_min;
        let chunk_size = 32.0_f64;
        let face = chunk.address.face;

        for vertex in &chunk.mesh.vertices {
            // Chunk-local position to face UV
            let local_u = vertex.position[0] as f64 / chunk_size;
            let local_v = vertex.position[2] as f64 / chunk_size;
            let face_u = u_min + local_u * u_size;
            let face_v = v_min + local_v * v_size;

            let fc = FaceCoord::new(face, face_u.clamp(0.0, 1.0), face_v.clamp(0.0, 1.0));
            let cube_point = face_coord_to_cube_point(&fc);
            let sphere_dir = cube_point.normalize();

            // Y axis maps to radial displacement
            let height = (vertex.position[1] as f64 - 16.0) * voxel_size;
            let world_pos = sphere_dir * (planet_radius + height);

            // Color by material
            let color = material_color(vertex.voxel_type, vertex.ao);

            all_vertices.push(VertexPositionColor {
                position: [world_pos.x as f32, world_pos.y as f32, world_pos.z as f32],
                color,
            });
        }

        for &idx in &chunk.mesh.indices {
            all_indices.push(base_vertex + idx);
        }
    }

    info!(
        "Built face render data: {} vertices, {} indices ({} triangles)",
        all_vertices.len(),
        all_indices.len(),
        all_indices.len() / 3
    );

    SingleFaceRenderData {
        vertices: all_vertices,
        indices: all_indices,
    }
}

/// Map voxel type and AO to an RGBA color.
fn material_color(voxel_type: VoxelTypeId, ao: u8) -> [f32; 4] {
    let ao_factor = 1.0 - (ao as f32 * 0.2); // 0..3 → 1.0..0.4
    let (r, g, b) = match voxel_type.0 {
        1 => (0.5, 0.5, 0.5),   // stone - gray
        2 => (0.55, 0.35, 0.2), // dirt - brown
        3 => (0.2, 0.7, 0.15),  // grass - green
        _ => (0.8, 0.8, 0.8),   // default - light gray
    };
    [r * ao_factor, g * ao_factor, b * ao_factor, 1.0]
}

/// Create a camera matrix looking down at the center of the given face.
///
/// Returns a view-projection matrix suitable for GPU upload.
pub fn create_face_camera(
    face: CubeFace,
    planet_radius: f32,
    altitude: f32,
    aspect_ratio: f32,
) -> Mat4 {
    let normal = face.normal().as_vec3();
    let eye = normal * (planet_radius + altitude);
    let target = normal * planet_radius;

    // Choose an up vector not parallel to the view direction
    let up = if normal.y.abs() > 0.9 {
        Vec3::Z
    } else {
        Vec3::Y
    };

    let view = Mat4::look_at_rh(eye, target, up);
    let proj = Mat4::perspective_rh(
        60.0_f32.to_radians(),
        aspect_ratio,
        altitude * 0.01,
        altitude * 10.0,
    );
    proj * view
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunks_load_for_single_face() {
        let loader = SingleFaceLoader::new_demo(CubeFace::PosY, 2, 42);
        let chunks = loader.load_and_mesh();
        // At least some chunks should have non-empty meshes
        assert!(
            !chunks.is_empty(),
            "Expected at least one non-empty meshed chunk"
        );
    }

    #[test]
    fn test_expected_chunk_count() {
        let loader = SingleFaceLoader::new_demo(CubeFace::PosY, 4, 42);
        assert_eq!(loader.expected_chunk_count(), 81);
    }

    #[test]
    fn test_render_data_has_vertices() {
        let loader = SingleFaceLoader::new_demo(CubeFace::PosY, 1, 42);
        let chunks = loader.load_and_mesh();
        let render_data = build_face_render_data(&chunks, loader.planet_radius, loader.voxel_size);
        assert!(
            !render_data.vertices.is_empty(),
            "Expected non-empty vertex data"
        );
        assert!(
            !render_data.indices.is_empty(),
            "Expected non-empty index data"
        );
    }

    #[test]
    fn test_vertices_displaced_onto_sphere() {
        let loader = SingleFaceLoader::new_demo(CubeFace::PosY, 1, 42);
        let radius = loader.planet_radius;
        let chunks = loader.load_and_mesh();
        let render_data = build_face_render_data(&chunks, radius, loader.voxel_size);

        for vertex in &render_data.vertices {
            let pos = Vec3::from(vertex.position);
            let distance = pos.length();
            // All vertices should be roughly at planet radius
            assert!(
                (distance - radius as f32).abs() < 50.0,
                "Vertex at distance {distance} is too far from radius {radius}"
            );
        }
    }

    #[test]
    fn test_face_camera_looks_at_face() {
        let vp = create_face_camera(CubeFace::PosY, 200.0, 100.0, 16.0 / 9.0);
        // The matrix should be finite
        for col in 0..4 {
            for row in 0..4 {
                assert!(
                    vp.col(col)[row].is_finite(),
                    "VP matrix has non-finite value at ({col}, {row})"
                );
            }
        }
    }

    #[test]
    fn test_pos_y_vertices_have_positive_y() {
        let loader = SingleFaceLoader::new_demo(CubeFace::PosY, 1, 42);
        let chunks = loader.load_and_mesh();
        let render_data = build_face_render_data(&chunks, loader.planet_radius, loader.voxel_size);

        for vertex in &render_data.vertices {
            assert!(
                vertex.position[1] > 0.0,
                "PosY face vertex should have positive Y, got {:?}",
                vertex.position
            );
        }
    }
}
