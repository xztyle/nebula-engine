//! Cubesphere vertex displacement: transforms flat chunk mesh vertices onto a planet's
//! curved surface by projecting through the cube-to-sphere mapping.

use glam::DVec3;
use nebula_cubesphere::{ChunkAddress, FaceCoord, face_coord_to_cube_point};

/// Parameters describing a planet's geometry for vertex displacement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlanetParams {
    /// Radius of the planet's base sphere in world-space units (meters).
    pub radius: f64,
    /// Size of one voxel in world-space units (meters per voxel).
    pub voxel_size: f64,
}

impl PlanetParams {
    /// Create new planet parameters.
    ///
    /// # Panics
    ///
    /// Panics if `radius` or `voxel_size` are not positive and finite.
    pub fn new(radius: f64, voxel_size: f64) -> Self {
        assert!(
            radius > 0.0 && radius.is_finite(),
            "radius must be positive and finite, got {radius}"
        );
        assert!(
            voxel_size > 0.0 && voxel_size.is_finite(),
            "voxel_size must be positive and finite, got {voxel_size}"
        );
        Self { radius, voxel_size }
    }
}

/// A buffer of displaced `[f32; 3]` positions, stored parallel to the packed vertex buffer.
///
/// Each entry corresponds 1:1 with a vertex in the [`super::PackedChunkMesh`]. The vertex
/// shader reads position from this buffer and other attributes from the packed buffer.
pub struct DisplacementBuffer {
    /// Displaced world-space positions, one per vertex.
    pub positions: Vec<[f32; 3]>,
}

impl DisplacementBuffer {
    /// Create an empty displacement buffer.
    pub fn new() -> Self {
        Self {
            positions: Vec::new(),
        }
    }

    /// Create a buffer with the given capacity.
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            positions: Vec::with_capacity(cap),
        }
    }

    /// Returns the buffer contents as a byte slice for GPU upload.
    pub fn as_bytes(&self) -> &[u8] {
        bytemuck::cast_slice(&self.positions)
    }

    /// Returns the number of displaced positions.
    pub fn len(&self) -> usize {
        self.positions.len()
    }

    /// Returns `true` if the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }

    /// Size in bytes.
    pub fn byte_size(&self) -> usize {
        self.positions.len() * std::mem::size_of::<[f32; 3]>()
    }
}

impl Default for DisplacementBuffer {
    fn default() -> Self {
        Self::new()
    }
}

/// Displace a packed chunk mesh's vertices onto the cubesphere surface.
///
/// Each vertex's chunk-local `(x, y, z)` position is mapped through the cubesphere
/// projection. The X and Z axes map to the face's UV tangent/bitangent directions,
/// while the Y axis maps to the radial (height) direction.
///
/// The chunk's position on the cube face is derived from `chunk_addr`, which provides
/// the UV bounds via [`ChunkAddress::uv_bounds`].
///
/// Returns a [`DisplacementBuffer`] with one `[f32; 3]` per vertex.
///
/// # Coordinate pipeline
///
/// 1. Chunk-local `(u8, u8, u8)` in `[0, 32]`
/// 2. Face UV `(f64, f64)` in `[0, 1]` via chunk's UV bounds
/// 3. Unit cube point via [`face_coord_to_cube_point`]
/// 4. Unit sphere point via normalization
/// 5. Planet surface point at `radius + height`
pub fn displace_to_cubesphere(
    mesh: &super::PackedChunkMesh,
    chunk_addr: &ChunkAddress,
    planet: &PlanetParams,
) -> DisplacementBuffer {
    let (u_min, v_min, u_max, v_max) = chunk_addr.uv_bounds();
    let u_size = u_max - u_min;
    let v_size = v_max - v_min;
    let chunk_size = 32.0_f64;
    let face = chunk_addr.face;

    let mut buffer = DisplacementBuffer::with_capacity(mesh.vertices.len());

    for vertex in &mesh.vertices {
        // Step 1: chunk-local to face UV
        // X maps to U (tangent direction), Z maps to V (bitangent direction)
        let local_u = vertex.position[0] as f64 / chunk_size;
        let local_v = vertex.position[2] as f64 / chunk_size;
        let face_u = u_min + local_u * u_size;
        let face_v = v_min + local_v * v_size;

        // Step 2: face UV to unit cube point
        let fc = FaceCoord::new(face, face_u, face_v);
        let cube_point = face_coord_to_cube_point(&fc);

        // Step 3: normalize to unit sphere
        let sphere_dir = cube_point.normalize();

        // Step 4: apply terrain height (Y axis = radial)
        let height = vertex.position[1] as f64 * planet.voxel_size;
        let world_pos = sphere_dir * (planet.radius + height);

        buffer
            .positions
            .push([world_pos.x as f32, world_pos.y as f32, world_pos.z as f32]);
    }

    buffer
}

/// Compute the displaced position for a single vertex.
///
/// Useful for point queries without building a full displacement buffer.
pub fn displace_vertex(
    position: [u8; 3],
    chunk_addr: &ChunkAddress,
    planet: &PlanetParams,
) -> DVec3 {
    let (u_min, v_min, u_max, v_max) = chunk_addr.uv_bounds();
    let u_size = u_max - u_min;
    let v_size = v_max - v_min;
    let chunk_size = 32.0_f64;

    let local_u = position[0] as f64 / chunk_size;
    let local_v = position[2] as f64 / chunk_size;
    let face_u = u_min + local_u * u_size;
    let face_v = v_min + local_v * v_size;

    let fc = FaceCoord::new(chunk_addr.face, face_u, face_v);
    let cube_point = face_coord_to_cube_point(&fc);
    let sphere_dir = cube_point.normalize();

    let height = position[1] as f64 * planet.voxel_size;
    sphere_dir * (planet.radius + height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FaceDirection;
    use crate::packed::{ChunkVertex, PackedChunkMesh};
    use nebula_cubesphere::CubeFace;

    fn test_planet() -> PlanetParams {
        PlanetParams::new(1000.0, 1.0)
    }

    /// A displaced vertex at height 0 should lie on the sphere surface.
    #[test]
    fn test_displaced_vertex_is_on_sphere_surface() {
        let planet = test_planet();
        let chunk_addr = ChunkAddress::new(CubeFace::PosX, 10, 0, 0);

        let mut mesh = PackedChunkMesh::new();
        mesh.push_quad(
            [
                ChunkVertex::new([16, 0, 16], FaceDirection::PosY, 0, 1, [0, 0]),
                ChunkVertex::new([17, 0, 16], FaceDirection::PosY, 0, 1, [1, 0]),
                ChunkVertex::new([17, 0, 17], FaceDirection::PosY, 0, 1, [1, 1]),
                ChunkVertex::new([16, 0, 17], FaceDirection::PosY, 0, 1, [0, 1]),
            ],
            false,
        );

        let buf = displace_to_cubesphere(&mesh, &chunk_addr, &planet);
        assert_eq!(buf.len(), 4);

        for pos in &buf.positions {
            let p = DVec3::new(pos[0] as f64, pos[1] as f64, pos[2] as f64);
            let distance = p.length();
            assert!(
                (distance - planet.radius).abs() < 0.01,
                "Vertex at height 0 should be at radius {}, got {distance}",
                planet.radius
            );
        }
    }

    /// A vertex on the +X face should have a positive X direction.
    #[test]
    fn test_vertex_on_pos_x_face_has_positive_x() {
        let planet = test_planet();
        let chunk_addr = ChunkAddress::new(CubeFace::PosX, 10, 0, 0);

        let mut mesh = PackedChunkMesh::new();
        mesh.push_quad(
            [
                ChunkVertex::new([16, 0, 16], FaceDirection::PosY, 0, 1, [0, 0]),
                ChunkVertex::new([17, 0, 16], FaceDirection::PosY, 0, 1, [1, 0]),
                ChunkVertex::new([17, 0, 17], FaceDirection::PosY, 0, 1, [1, 1]),
                ChunkVertex::new([16, 0, 17], FaceDirection::PosY, 0, 1, [0, 1]),
            ],
            false,
        );

        let buf = displace_to_cubesphere(&mesh, &chunk_addr, &planet);
        let pos = DVec3::new(
            buf.positions[0][0] as f64,
            buf.positions[0][1] as f64,
            buf.positions[0][2] as f64,
        );
        let dir = pos.normalize();
        assert!(
            dir.x > 0.0,
            "Vertex on +X face should have positive X direction, got {dir:?}"
        );
    }

    /// Vertices at a shared chunk boundary should have identical displaced positions.
    #[test]
    fn test_adjacent_chunk_vertices_align_at_boundaries() {
        let planet = test_planet();
        let chunk_a = ChunkAddress::new(CubeFace::PosX, 10, 0, 0);
        let chunk_b = ChunkAddress::new(CubeFace::PosX, 10, 1, 0);

        // Chunk A: vertex at its +U boundary (x=32)
        let mut mesh_a = PackedChunkMesh::new();
        mesh_a.push_quad(
            [
                ChunkVertex::new([32, 0, 16], FaceDirection::PosY, 0, 1, [0, 0]),
                ChunkVertex::new([32, 0, 16], FaceDirection::PosY, 0, 1, [0, 0]),
                ChunkVertex::new([32, 0, 16], FaceDirection::PosY, 0, 1, [0, 0]),
                ChunkVertex::new([32, 0, 16], FaceDirection::PosY, 0, 1, [0, 0]),
            ],
            false,
        );

        // Chunk B: vertex at its -U boundary (x=0)
        let mut mesh_b = PackedChunkMesh::new();
        mesh_b.push_quad(
            [
                ChunkVertex::new([0, 0, 16], FaceDirection::PosY, 0, 1, [0, 0]),
                ChunkVertex::new([0, 0, 16], FaceDirection::PosY, 0, 1, [0, 0]),
                ChunkVertex::new([0, 0, 16], FaceDirection::PosY, 0, 1, [0, 0]),
                ChunkVertex::new([0, 0, 16], FaceDirection::PosY, 0, 1, [0, 0]),
            ],
            false,
        );

        let buf_a = displace_to_cubesphere(&mesh_a, &chunk_a, &planet);
        let buf_b = displace_to_cubesphere(&mesh_b, &chunk_b, &planet);

        let pos_a = buf_a.positions[0];
        let pos_b = buf_b.positions[0];

        let max_diff = (pos_a[0] - pos_b[0])
            .abs()
            .max((pos_a[1] - pos_b[1]).abs())
            .max((pos_a[2] - pos_b[2]).abs());
        assert!(
            max_diff < 1e-4,
            "Boundary vertices should align, max difference: {max_diff}"
        );
    }

    /// Displacement magnitude should equal planet radius + height.
    #[test]
    fn test_displacement_magnitude_equals_radius_plus_height() {
        let planet = test_planet();
        let chunk_addr = ChunkAddress::new(CubeFace::PosZ, 10, 0, 0);
        let height_voxels = 10u8;

        let mut mesh = PackedChunkMesh::new();
        mesh.push_quad(
            [
                ChunkVertex::new([16, height_voxels, 16], FaceDirection::PosY, 0, 1, [0, 0]),
                ChunkVertex::new([17, height_voxels, 16], FaceDirection::PosY, 0, 1, [1, 0]),
                ChunkVertex::new([17, height_voxels, 17], FaceDirection::PosY, 0, 1, [1, 1]),
                ChunkVertex::new([16, height_voxels, 17], FaceDirection::PosY, 0, 1, [0, 1]),
            ],
            false,
        );

        let buf = displace_to_cubesphere(&mesh, &chunk_addr, &planet);
        let expected = planet.radius + (height_voxels as f64 * planet.voxel_size);

        for pos in &buf.positions {
            let p = DVec3::new(pos[0] as f64, pos[1] as f64, pos[2] as f64);
            let distance = p.length();
            assert!(
                (distance - expected).abs() < 0.1,
                "Expected distance {expected}, got {distance}"
            );
        }
    }

    /// Empty mesh produces empty displacement buffer.
    #[test]
    fn test_empty_mesh_produces_empty_buffer() {
        let planet = test_planet();
        let chunk_addr = ChunkAddress::new(CubeFace::PosX, 10, 0, 0);
        let mesh = PackedChunkMesh::new();
        let buf = displace_to_cubesphere(&mesh, &chunk_addr, &planet);
        assert!(buf.is_empty());
    }

    /// `displace_vertex` matches the buffer version.
    #[test]
    fn test_displace_vertex_matches_buffer() {
        let planet = test_planet();
        let chunk_addr = ChunkAddress::new(CubeFace::NegY, 10, 5, 5);
        let pos = [16, 8, 16];

        let single = displace_vertex(pos, &chunk_addr, &planet);

        let mut mesh = PackedChunkMesh::new();
        mesh.push_quad(
            [
                ChunkVertex::new(pos, FaceDirection::PosY, 0, 1, [0, 0]),
                ChunkVertex::new(pos, FaceDirection::PosY, 0, 1, [0, 0]),
                ChunkVertex::new(pos, FaceDirection::PosY, 0, 1, [0, 0]),
                ChunkVertex::new(pos, FaceDirection::PosY, 0, 1, [0, 0]),
            ],
            false,
        );
        let buf = displace_to_cubesphere(&mesh, &chunk_addr, &planet);
        let buffered = DVec3::new(
            buf.positions[0][0] as f64,
            buf.positions[0][1] as f64,
            buf.positions[0][2] as f64,
        );

        assert!(
            (single - buffered).length() < 0.01,
            "Single vertex and buffer should match: {single:?} vs {buffered:?}"
        );
    }

    /// Displacement buffer bytes are correct size.
    #[test]
    fn test_displacement_buffer_bytes() {
        let mut buf = DisplacementBuffer::new();
        buf.positions.push([1.0, 2.0, 3.0]);
        buf.positions.push([4.0, 5.0, 6.0]);
        assert_eq!(buf.byte_size(), 2 * 12);
        assert_eq!(buf.as_bytes().len(), 24);
    }

    #[test]
    #[should_panic(expected = "radius must be positive")]
    fn test_planet_params_zero_radius_panics() {
        PlanetParams::new(0.0, 1.0);
    }

    #[test]
    #[should_panic(expected = "voxel_size must be positive")]
    fn test_planet_params_zero_voxel_size_panics() {
        PlanetParams::new(1000.0, 0.0);
    }

    /// All six faces produce vertices in the correct hemisphere.
    #[test]
    fn test_all_faces_displace_to_correct_hemisphere() {
        let planet = test_planet();

        for face in CubeFace::ALL {
            let chunk_addr = ChunkAddress::new(face, 10, 0, 0);
            let pos = displace_vertex([16, 0, 16], &chunk_addr, &planet);
            let dir = pos.normalize();
            let normal = face.normal();

            // The displaced vertex should be in the hemisphere of this face's normal
            assert!(
                dir.dot(normal) > 0.0,
                "Vertex on {face:?} should be in correct hemisphere, got dot={:.3}",
                dir.dot(normal)
            );
        }
    }
}
