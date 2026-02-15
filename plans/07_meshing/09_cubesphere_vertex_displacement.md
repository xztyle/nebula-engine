# Cubesphere Vertex Displacement

## Problem

The greedy meshing pipeline (stories 01-05) produces flat chunk geometry in a local Cartesian coordinate system — positions are integers in `[0, 32]` along each axis. But Nebula Engine's planets are cubespheres: each chunk sits on a face of a cube that is projected onto a sphere. The flat chunk mesh must be curved to conform to the planet's spherical surface. Without this displacement, chunks rendered at their local positions would form a flat cube rather than a sphere, and the terrain would look like a Minecraft-style flat world rather than a planetary body.

The displacement must account for both the cubesphere projection (mapping cube coordinates to sphere surface points) and the terrain height (how far above or below the base sphere radius each voxel sits). Additionally, vertices at chunk boundaries must align exactly with the corresponding vertices of adjacent chunks to prevent visible seams.

## Solution

Implement cubesphere vertex displacement in the `nebula_meshing` crate as a post-processing pass that transforms flat chunk mesh vertices into their correct positions on the planet's curved surface.

### Coordinate Pipeline

The displacement follows this chain:

1. **Chunk-local position** `(u8, u8, u8)` in `[0, 32]` range.
2. **Cube face UV** `(f64, f64)` in `[0, 1]` range on the chunk's cube face.
3. **Full cube face UV** accounting for the chunk's position within the face (a face may contain many chunks in a grid).
4. **Unit cube point** `(f64, f64, f64)` on the surface of a unit cube.
5. **Unit sphere point** via cube-to-sphere projection (normalize the cube point).
6. **Planet surface point** by scaling the unit sphere direction by `radius + height`.
7. **Camera-relative position** `(f32, f32, f32)` for rendering.

```rust
/// Displace a flat chunk mesh's vertices to conform to the cubesphere surface.
pub fn displace_to_cubesphere(
    mesh: &mut ChunkMesh,
    chunk_pos: &ChunkPosition,
    planet: &PlanetParams,
) {
    let face = chunk_pos.cube_face();
    let chunk_origin_uv = chunk_pos.face_uv_origin(); // (f64, f64) on [0, 1]
    let chunk_uv_size = chunk_pos.face_uv_size();     // how much UV one chunk spans
    let chunk_size = 32.0f64;

    for vertex in &mut mesh.vertices {
        // Step 1: chunk-local to face UV
        let local_u = vertex.position[0] as f64 / chunk_size;
        let local_v = vertex.position[2] as f64 / chunk_size;
        let face_u = chunk_origin_uv.0 + local_u * chunk_uv_size;
        let face_v = chunk_origin_uv.1 + local_v * chunk_uv_size;

        // Step 2: face UV to unit cube point
        let cube_point = face.uv_to_cube_point(face_u, face_v);

        // Step 3: unit cube point to unit sphere point (normalize)
        let sphere_dir = cube_point.normalize();

        // Step 4: apply terrain height
        // The Y component of the local position encodes the radial (height) axis
        let height = vertex.position[1] as f64 * planet.voxel_size;
        let world_pos = sphere_dir * (planet.radius + height);

        // Store displaced position (will be converted to camera-relative f32 at render time)
        vertex.displaced_position = [world_pos.x, world_pos.y, world_pos.z];
    }
}
```

### Cube-to-Sphere Projection

The projection uses the standard normalization approach: a point `(x, y, z)` on the surface of a unit cube (where one coordinate is +/-1) is projected to the sphere by normalizing the vector. This produces a unit sphere point along the same direction.

```rust
impl CubeFace {
    /// Convert a UV coordinate on this face to a point on the unit cube surface.
    pub fn uv_to_cube_point(self, u: f64, v: f64) -> DVec3 {
        // Map u, v from [0,1] to [-1, 1]
        let s = u * 2.0 - 1.0;
        let t = v * 2.0 - 1.0;

        let tangent = self.tangent();
        let bitangent = self.bitangent();
        let normal = self.normal();

        // Point on the cube face at distance 1 from origin along the normal
        normal + tangent * s + bitangent * t
    }
}
```

### Height Mapping

In the flat chunk, the Y axis represents the radial (height) direction. A voxel at `y=0` is at the planet's surface, and voxels at higher Y values are above the surface. After displacement, the Y axis maps to the radial direction of the sphere at that point. The `planet.voxel_size` parameter converts voxel units to world-space meters.

### Boundary Alignment

Vertices at chunk boundaries must land at exactly the same sphere position as the corresponding vertices of the adjacent chunk. This is guaranteed by the deterministic nature of the projection: both chunks compute `face_u` and `face_v` using the same formula, and the chunk's UV origin and size are derived purely from its `ChunkPosition`. As long as the floating-point computation is identical (same operations in the same order), boundary vertices match exactly.

To ensure this, the UV calculation uses `f64` precision throughout. The final position is converted to `f32` only at the very end, and both sides of a boundary compute the same `f64` intermediate before truncating to `f32`.

### Vertex Format Extension

The displaced position cannot fit in the packed `u8` position field of `ChunkVertex` (story 05). Two approaches:

1. **Separate displacement buffer**: store displaced `[f32; 3]` positions in a parallel buffer alongside the packed vertex buffer. The vertex shader reads position from the displacement buffer and other attributes from the packed buffer.
2. **Extended vertex format**: add `displaced_position: [f32; 3]` to the vertex struct, increasing size from 12 to 24 bytes. This is simpler but uses more bandwidth.

The implementation uses approach 1 (separate buffer) to keep the packed format from story 05 intact for non-planetary meshes. Planetary chunks bind both buffers.

## Outcome

The `nebula_meshing` crate exports `displace_to_cubesphere()` which transforms flat chunk mesh vertices onto the cubesphere surface. After displacement, each vertex sits at its correct position on the planet at the appropriate altitude. Adjacent chunk boundaries align seamlessly. The displaced positions are stored in a separate buffer uploaded alongside the packed vertex buffer. Running `cargo test -p nebula_meshing` passes all displacement tests.

## Demo Integration

**Demo crate:** `nebula-demo`

The flat terrain slab is displaced onto the cubesphere surface. The terrain now visibly curves — chunks at the edge of the loaded area show the planet's curvature. This is the first time terrain looks like it belongs on a sphere.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `glam` | `0.29` | `DVec3` for high-precision sphere projection math |
| `nebula_cubesphere` | workspace | `CubeFace`, `FaceCoord`, projection utilities |
| `nebula_meshing` | workspace | `ChunkMesh`, `ChunkVertex` from prior stories |

The crate uses Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;

    fn test_planet() -> PlanetParams {
        PlanetParams {
            radius: 1000.0,  // 1000 meter radius
            voxel_size: 1.0, // 1 meter per voxel
        }
    }

    /// A displaced vertex should lie on or very near the sphere surface
    /// (within voxel_size tolerance for non-zero height).
    #[test]
    fn test_displaced_vertex_is_on_sphere_surface() {
        let planet = test_planet();
        let chunk_pos = ChunkPosition::new_on_face(CubeFace::PosX, 0, 0, 0);

        let mut mesh = ChunkMesh::new();
        // A vertex at height 0 (y=0) should be at exactly the planet radius
        mesh.push_quad(
            [
                ChunkVertex::new([16, 0, 16], FaceDirection::PosY, 0, 1, [0, 0]),
                ChunkVertex::new([17, 0, 16], FaceDirection::PosY, 0, 1, [1, 0]),
                ChunkVertex::new([17, 0, 17], FaceDirection::PosY, 0, 1, [1, 1]),
                ChunkVertex::new([16, 0, 17], FaceDirection::PosY, 0, 1, [0, 1]),
            ],
            false,
        );

        displace_to_cubesphere(&mut mesh, &chunk_pos, &planet);

        for vertex in &mesh.vertices {
            let pos = DVec3::new(
                vertex.displaced_position[0] as f64,
                vertex.displaced_position[1] as f64,
                vertex.displaced_position[2] as f64,
            );
            let distance = pos.length();
            assert!(
                (distance - planet.radius).abs() < 0.01,
                "Vertex at height 0 should be at radius {}, got {distance}",
                planet.radius
            );
        }
    }

    /// A vertex at the center of a chunk on the +X face should map to the correct
    /// sphere point (predominantly in the +X direction).
    #[test]
    fn test_vertex_at_chunk_center_maps_to_correct_sphere_point() {
        let planet = test_planet();
        let chunk_pos = ChunkPosition::new_on_face(CubeFace::PosX, 0, 0, 0);

        let mut mesh = ChunkMesh::new();
        mesh.push_quad(
            [
                ChunkVertex::new([16, 0, 16], FaceDirection::PosY, 0, 1, [0, 0]),
                ChunkVertex::new([17, 0, 16], FaceDirection::PosY, 0, 1, [1, 0]),
                ChunkVertex::new([17, 0, 17], FaceDirection::PosY, 0, 1, [1, 1]),
                ChunkVertex::new([16, 0, 17], FaceDirection::PosY, 0, 1, [0, 1]),
            ],
            false,
        );

        displace_to_cubesphere(&mut mesh, &chunk_pos, &planet);

        // For a chunk on the +X face, the displaced position should have
        // a significant +X component
        let pos = DVec3::new(
            mesh.vertices[0].displaced_position[0] as f64,
            mesh.vertices[0].displaced_position[1] as f64,
            mesh.vertices[0].displaced_position[2] as f64,
        );
        let dir = pos.normalize();
        assert!(
            dir.x > 0.0,
            "Vertex on +X face should have positive X direction, got {dir:?}"
        );
    }

    /// Vertices at a shared chunk boundary should have identical displaced positions
    /// when computed from either chunk.
    #[test]
    fn test_adjacent_chunk_vertices_align_at_boundaries() {
        let planet = test_planet();
        let chunk_a = ChunkPosition::new_on_face(CubeFace::PosX, 0, 0, 0);
        let chunk_b = ChunkPosition::new_on_face(CubeFace::PosX, 1, 0, 0);

        // Chunk A: vertex at its +U boundary (x=32)
        let mut mesh_a = ChunkMesh::new();
        mesh_a.push_quad(
            [
                ChunkVertex::new([32, 0, 16], FaceDirection::PosY, 0, 1, [0, 0]),
                ChunkVertex::new([32, 0, 16], FaceDirection::PosY, 0, 1, [0, 0]),
                ChunkVertex::new([32, 0, 16], FaceDirection::PosY, 0, 1, [0, 0]),
                ChunkVertex::new([32, 0, 16], FaceDirection::PosY, 0, 1, [0, 0]),
            ],
            false,
        );

        // Chunk B: vertex at its -U boundary (x=0), which is the same world position
        let mut mesh_b = ChunkMesh::new();
        mesh_b.push_quad(
            [
                ChunkVertex::new([0, 0, 16], FaceDirection::PosY, 0, 1, [0, 0]),
                ChunkVertex::new([0, 0, 16], FaceDirection::PosY, 0, 1, [0, 0]),
                ChunkVertex::new([0, 0, 16], FaceDirection::PosY, 0, 1, [0, 0]),
                ChunkVertex::new([0, 0, 16], FaceDirection::PosY, 0, 1, [0, 0]),
            ],
            false,
        );

        displace_to_cubesphere(&mut mesh_a, &chunk_a, &planet);
        displace_to_cubesphere(&mut mesh_b, &chunk_b, &planet);

        let pos_a = mesh_a.vertices[0].displaced_position;
        let pos_b = mesh_b.vertices[0].displaced_position;

        let diff = (
            (pos_a[0] - pos_b[0]).abs(),
            (pos_a[1] - pos_b[1]).abs(),
            (pos_a[2] - pos_b[2]).abs(),
        );
        let max_diff = diff.0.max(diff.1).max(diff.2);
        assert!(
            max_diff < 1e-4,
            "Boundary vertices should align, max difference: {max_diff}"
        );
    }

    /// Displacement magnitude should equal planet radius + height.
    #[test]
    fn test_displacement_magnitude_equals_radius_plus_height() {
        let planet = test_planet();
        let chunk_pos = ChunkPosition::new_on_face(CubeFace::PosZ, 0, 0, 0);

        let mut mesh = ChunkMesh::new();
        let height_voxels = 10u8;
        mesh.push_quad(
            [
                ChunkVertex::new([16, height_voxels, 16], FaceDirection::PosY, 0, 1, [0, 0]),
                ChunkVertex::new([17, height_voxels, 16], FaceDirection::PosY, 0, 1, [1, 0]),
                ChunkVertex::new([17, height_voxels, 17], FaceDirection::PosY, 0, 1, [1, 1]),
                ChunkVertex::new([16, height_voxels, 17], FaceDirection::PosY, 0, 1, [0, 1]),
            ],
            false,
        );

        displace_to_cubesphere(&mut mesh, &chunk_pos, &planet);

        let expected_distance = planet.radius + (height_voxels as f64 * planet.voxel_size);
        for vertex in &mesh.vertices {
            let pos = DVec3::new(
                vertex.displaced_position[0] as f64,
                vertex.displaced_position[1] as f64,
                vertex.displaced_position[2] as f64,
            );
            let distance = pos.length();
            assert!(
                (distance - expected_distance).abs() < 0.1,
                "Expected distance {expected_distance}, got {distance}"
            );
        }
    }
}
```
