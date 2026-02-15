# Single Face Rendering

## Problem

All the foundational systems -- cubesphere geometry (Epic 05), voxel storage (Epic 06), greedy meshing (Epic 07), terrain generation (Epic 09), and the unlit render pipeline (Epic 04) -- have been built and tested in isolation. But none of them have been composed into a visible planetary surface. The engine cannot yet show a single square meter of cubesphere terrain on screen. Without this integration, there is no proof that the chunk loading pipeline, cubesphere vertex displacement, and GPU submission work together end-to-end. This story produces the engine's first visual proof of cubesphere terrain: one cube face, loaded and meshed, displaced onto a sphere, and rendered with the unlit pipeline. The camera sits above the face center looking straight down, providing a clear view of the voxel terrain warped onto the sphere surface.

## Solution

### Face Selection and Chunk Loading

Choose a single cube face (e.g., `CubeFace::PosY`, the "top" face) and load all chunks within a configurable radius around the face center. The chunk manager (from Epic 06, story 04) already supports loading chunks by `ChunkAddress`, and the terrain generator (Epic 09) fills them with voxel data. This story wires those systems together for one face:

```rust
use nebula_cubesphere::{CubeFace, ChunkAddress, FaceCoord};
use nebula_voxel::ChunkManager;
use nebula_terrain::TerrainGenerator;

pub struct SingleFaceLoader {
    pub face: CubeFace,
    pub load_radius: u32,
    pub chunk_manager: ChunkManager,
    pub terrain_gen: TerrainGenerator,
}

impl SingleFaceLoader {
    /// Load chunks for a single face within `load_radius` chunks of the face center.
    /// Returns the number of chunks loaded.
    pub fn load_face_chunks(&mut self) -> u32 {
        let mut loaded = 0;
        let center = self.face_center_chunk();
        let r = self.load_radius as i32;

        for cu in -r..=r {
            for cv in -r..=r {
                let addr = ChunkAddress {
                    face: self.face,
                    u: center.u + cu,
                    v: center.v + cv,
                    lod: 0,
                };
                if self.chunk_manager.is_loaded(&addr) {
                    continue;
                }
                let chunk_data = self.terrain_gen.generate_chunk(&addr);
                self.chunk_manager.insert(addr, chunk_data);
                loaded += 1;
            }
        }
        loaded
    }

    fn face_center_chunk(&self) -> ChunkAddress {
        ChunkAddress {
            face: self.face,
            u: 0,
            v: 0,
            lod: 0,
        }
    }
}
```

### Meshing with Cubesphere Displacement

Once chunks are loaded, each chunk is meshed using the greedy mesher (Epic 07). The resulting vertices are in flat chunk-local coordinates. Before uploading to the GPU, every vertex position is displaced onto the sphere surface:

```rust
use nebula_cubesphere::face_coord_to_sphere_everitt;
use nebula_mesh::MeshData;

/// Displace flat chunk-local vertices onto the cubesphere surface.
///
/// For each vertex:
/// 1. Convert its chunk-local (x, z) position to a FaceCoord (u, v) on the cube face.
/// 2. Project the FaceCoord to a unit sphere point using the Everitt mapping.
/// 3. Scale the sphere point by (planet_radius + vertex_y * height_scale) to get
///    the final 3D position. The vertex's original Y becomes radial displacement.
pub fn displace_mesh_to_sphere(
    mesh: &mut MeshData,
    face: CubeFace,
    chunk_addr: &ChunkAddress,
    planet_radius: f64,
    chunk_size: f64,
) {
    for vertex in &mut mesh.vertices {
        // Convert chunk-local position to face-space UV in [0, 1].
        let face_u = (chunk_addr.u as f64 * chunk_size + vertex.position[0] as f64)
            / face_total_size();
        let face_v = (chunk_addr.v as f64 * chunk_size + vertex.position[2] as f64)
            / face_total_size();

        let fc = FaceCoord::new(face, face_u.clamp(0.0, 1.0), face_v.clamp(0.0, 1.0));
        let sphere_dir = face_coord_to_sphere_everitt(&fc);

        // The vertex's Y coordinate (height) becomes radial displacement.
        let radius = planet_radius + vertex.position[1] as f64;
        let world_pos = sphere_dir * radius;

        vertex.position = [world_pos.x as f32, world_pos.y as f32, world_pos.z as f32];
    }
}
```

### Camera Placement

Position the camera above the center of the selected face, looking straight down at the surface. For `PosY`, the face normal is `(0, 1, 0)`, so the camera is placed along the positive Y axis at a height above the planet surface:

```rust
use glam::{Vec3, Mat4};

pub fn create_face_down_camera(
    face_normal: Vec3,
    planet_radius: f32,
    altitude: f32,
    aspect_ratio: f32,
) -> Mat4 {
    let eye = face_normal * (planet_radius + altitude);
    let target = face_normal * planet_radius;
    // Choose an up vector that is not parallel to the view direction.
    let up = if face_normal.y.abs() > 0.9 {
        Vec3::Z
    } else {
        Vec3::Y
    };
    let view = Mat4::look_at_rh(eye, target, up);
    let proj = Mat4::perspective_rh(
        60.0_f32.to_radians(),
        aspect_ratio,
        altitude * 0.01,  // near plane
        altitude * 10.0,  // far plane
    );
    proj * view
}
```

### Render Submission

Collect all displaced chunk meshes, upload vertex and index buffers to the GPU, build the camera uniform buffer, and issue draw calls through the existing unlit pipeline (Epic 04, story 05):

```rust
pub fn render_single_face(
    ctx: &RenderContext,
    pipeline: &UnlitPipeline,
    camera_bind_group: &wgpu::BindGroup,
    chunk_meshes: &[MeshBuffer],
) {
    let mut encoder = ctx.device.create_command_encoder(
        &wgpu::CommandEncoderDescriptor { label: Some("single-face-render") }
    );

    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("single-face-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &ctx.surface_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.05, g: 0.05, b: 0.1, a: 1.0 }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &ctx.depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.0), // reverse-Z
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });

        pass.set_pipeline(&pipeline.pipeline);
        pass.set_bind_group(0, camera_bind_group, &[]);

        for mesh in chunk_meshes {
            mesh.bind(&mut pass);
            mesh.draw(&mut pass);
        }
    }

    ctx.queue.submit(std::iter::once(encoder.finish()));
}
```

### Integration Order

1. Create a `SingleFaceLoader` targeting `CubeFace::PosY`.
2. Call `load_face_chunks()` to populate the chunk manager with terrain data.
3. Mesh each loaded chunk with the greedy mesher.
4. Displace each mesh onto the sphere surface with `displace_mesh_to_sphere()`.
5. Upload all meshes to GPU buffers.
6. Build the view-projection matrix with `create_face_down_camera()`.
7. Each frame, call `render_single_face()`.

## Outcome

Running the engine shows a single cube face of cubesphere terrain rendered with the unlit pipeline. The terrain appears as colored voxels displaced onto a curved surface (visible curvature at the edges of the loaded area). The camera looks down from above the face center. All chunks within the load radius are meshed and rendered. This is the first visual proof that the cubesphere-voxel pipeline works end-to-end.

## Demo Integration

**Demo crate:** `nebula-demo`

A single cube face of terrain renders with visible curvature. The camera orbits above the face center, showing colored voxel terrain on a curved surface.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `wgpu` | `24.0` | GPU rendering pipeline and draw calls |
| `glam` | `0.29` | Camera matrices, vector math |
| `bytemuck` | `1.21` | Vertex buffer serialization |
| `noise` | `0.9` | Terrain generation (via nebula-terrain) |

Internal dependencies: `nebula-cubesphere`, `nebula-voxel`, `nebula-mesh`, `nebula-terrain`, `nebula-render`. Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nebula_cubesphere::CubeFace;

    #[test]
    fn test_chunks_load_for_single_face() {
        let mut loader = SingleFaceLoader {
            face: CubeFace::PosY,
            load_radius: 4,
            chunk_manager: ChunkManager::new(),
            terrain_gen: TerrainGenerator::with_seed(42),
        };
        let loaded = loader.load_face_chunks();
        // (2 * 4 + 1)^2 = 81 chunks in a 9x9 grid
        assert_eq!(loaded, 81, "Expected 81 chunks for radius 4, got {loaded}");
        assert_eq!(loader.chunk_manager.loaded_count(), 81);
    }

    #[test]
    fn test_meshes_generated_for_loaded_chunks() {
        let mut loader = SingleFaceLoader {
            face: CubeFace::PosY,
            load_radius: 2,
            chunk_manager: ChunkManager::new(),
            terrain_gen: TerrainGenerator::with_seed(42),
        };
        loader.load_face_chunks();

        let meshes: Vec<MeshData> = loader
            .chunk_manager
            .loaded_addresses()
            .iter()
            .map(|addr| mesh_chunk(&loader.chunk_manager, addr))
            .collect();

        // Every loaded chunk should produce a mesh (even if empty for air-only chunks).
        assert_eq!(meshes.len(), 25); // (2*2+1)^2 = 25
        // At least some meshes should have vertices (terrain is not all air).
        let non_empty = meshes.iter().filter(|m| !m.vertices.is_empty()).count();
        assert!(
            non_empty > 0,
            "Expected at least one non-empty mesh, got all empty"
        );
    }

    #[test]
    fn test_vertices_displaced_onto_sphere() {
        let planet_radius = 6_371_000.0; // Earth-like radius in meters
        let face = CubeFace::PosY;
        let addr = ChunkAddress { face, u: 0, v: 0, lod: 0 };

        let mut mesh = create_test_flat_mesh(); // Flat mesh at y=0
        displace_mesh_to_sphere(&mut mesh, face, &addr, planet_radius, 32.0);

        for vertex in &mesh.vertices {
            let pos = glam::Vec3::from(vertex.position);
            let distance_from_origin = pos.length();
            // All vertices should be approximately at planet_radius
            // (within the chunk's height range).
            assert!(
                (distance_from_origin - planet_radius as f32).abs() < 1000.0,
                "Vertex at distance {distance_from_origin} is too far from radius {planet_radius}"
            );
            // Vertices should be in the hemisphere of the face normal (PosY => y > 0).
            assert!(
                pos.y > 0.0,
                "PosY face vertex should have positive Y, got {pos:?}"
            );
        }
    }

    #[test]
    fn test_frame_renders_without_errors() {
        // Integration test: build the full pipeline and render one frame.
        let (ctx, pipeline) = create_test_render_context();
        let mut loader = SingleFaceLoader {
            face: CubeFace::PosY,
            load_radius: 2,
            chunk_manager: ChunkManager::new(),
            terrain_gen: TerrainGenerator::with_seed(42),
        };
        loader.load_face_chunks();

        let meshes = build_and_displace_meshes(&loader, 6_371_000.0);
        let gpu_meshes = upload_meshes(&ctx.device, &meshes);
        let camera_uniform = create_face_down_camera(
            Vec3::Y,
            6_371_000.0,
            10_000.0,
            16.0 / 9.0,
        );
        let camera_bg = create_camera_bind_group(&ctx.device, &pipeline, &camera_uniform);

        // Rendering should complete without panic or GPU validation error.
        render_single_face(&ctx, &pipeline, &camera_bg, &gpu_meshes);
        ctx.device.poll(wgpu::Maintain::Wait);
    }

    #[test]
    fn test_chunk_count_matches_expected_for_loaded_area() {
        for radius in [1, 3, 5, 8] {
            let mut loader = SingleFaceLoader {
                face: CubeFace::PosY,
                load_radius: radius,
                chunk_manager: ChunkManager::new(),
                terrain_gen: TerrainGenerator::with_seed(0),
            };
            let loaded = loader.load_face_chunks();
            let expected = ((2 * radius + 1) as u32).pow(2);
            assert_eq!(
                loaded, expected,
                "Radius {radius}: expected {expected} chunks, got {loaded}"
            );
        }
    }
}
```
