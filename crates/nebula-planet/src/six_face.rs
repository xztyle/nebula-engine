//! Six-face planet rendering with face-level and chunk-level frustum culling.
//!
//! Composes six [`SingleFaceLoader`]s (one per [`CubeFace`]) and provides
//! face-level culling against a view frustum so that faces behind the camera
//! are skipped entirely.

use glam::{Mat4, Vec3};
use nebula_cubesphere::CubeFace;
use nebula_render::{Aabb, Frustum, VertexPositionColor};
use tracing::info;

use crate::single_face::{SingleFaceLoader, build_face_render_data};

/// State for one cube face: loader, visibility, and cached render data.
pub struct FaceState {
    /// Which cube face.
    pub face: CubeFace,
    /// Whether the face passed frustum culling this frame.
    pub visible: bool,
    /// The loader that generates terrain and meshes for this face.
    pub loader: SingleFaceLoader,
    /// Cached GPU-ready vertex data (built once at init).
    pub vertices: Vec<VertexPositionColor>,
    /// Cached GPU-ready index data.
    pub indices: Vec<u32>,
}

/// Manages all six cube faces of a planet with frustum culling.
pub struct PlanetFaces {
    /// Per-face state, one for each of the 6 cube faces.
    pub face_states: [FaceState; 6],
    /// Planet radius in meters.
    pub planet_radius: f64,
    /// Voxel size in meters.
    pub voxel_size: f64,
}

impl PlanetFaces {
    /// Create a new planet with all six faces loaded and meshed.
    ///
    /// Uses the same demo parameters as [`SingleFaceLoader::new_demo`].
    pub fn new_demo(load_radius: u32, seed: u64) -> Self {
        let face_states = CubeFace::ALL.map(|face| {
            let loader = SingleFaceLoader::new_demo(face, load_radius, seed);
            let chunks = loader.load_and_mesh();
            let render_data =
                build_face_render_data(&chunks, loader.planet_radius, loader.voxel_size);

            FaceState {
                face,
                visible: true,
                loader,
                vertices: render_data.vertices,
                indices: render_data.indices,
            }
        });

        let planet_radius = face_states[0].loader.planet_radius;
        let voxel_size = face_states[0].loader.voxel_size;

        let total_verts: usize = face_states.iter().map(|f| f.vertices.len()).sum();
        let total_tris: usize = face_states.iter().map(|f| f.indices.len()).sum::<usize>() / 3;
        info!(
            "PlanetFaces initialized: {} total vertices, {} triangles across 6 faces",
            total_verts, total_tris
        );

        Self {
            face_states,
            planet_radius,
            voxel_size,
        }
    }

    /// Perform face-level frustum culling.
    ///
    /// Computes an AABB for each face's hemisphere cap and tests it against
    /// the given frustum. Faces that are entirely outside the frustum are
    /// marked invisible. Returns the number of visible faces.
    pub fn cull_faces(&mut self, frustum: &Frustum) -> u32 {
        let mut visible_count = 0;
        for face_state in &mut self.face_states {
            let aabb = compute_face_aabb(face_state.face, self.planet_radius as f32);
            face_state.visible = frustum.is_visible(&aabb);
            if face_state.visible {
                visible_count += 1;
            }
        }
        visible_count
    }

    /// Build combined render data from all currently visible faces.
    ///
    /// Returns `(vertices, indices)` ready for GPU upload.
    pub fn visible_render_data(&self) -> (Vec<VertexPositionColor>, Vec<u32>) {
        let mut all_vertices = Vec::new();
        let mut all_indices = Vec::new();

        for face_state in &self.face_states {
            if !face_state.visible || face_state.vertices.is_empty() {
                continue;
            }
            let base = all_vertices.len() as u32;
            all_vertices.extend_from_slice(&face_state.vertices);
            all_indices.extend(face_state.indices.iter().map(|&i| i + base));
        }

        (all_vertices, all_indices)
    }

    /// Count how many faces are currently marked visible.
    pub fn visible_face_count(&self) -> u32 {
        self.face_states.iter().filter(|f| f.visible).count() as u32
    }

    /// Total vertex count across all faces (regardless of visibility).
    pub fn total_vertex_count(&self) -> usize {
        self.face_states.iter().map(|f| f.vertices.len()).sum()
    }
}

/// Compute an f32 AABB that encloses the hemisphere cap of the given cube face.
///
/// The AABB is sized to contain all vertices displaced onto the sphere for
/// this face. It extends ±radius in the two axes perpendicular to the face
/// normal, and from 0 to +radius along the face normal.
fn compute_face_aabb(face: CubeFace, planet_radius: f32) -> Aabb {
    let normal = face.normal();
    let r = planet_radius;
    let margin = r * 0.1; // terrain height margin

    let n = Vec3::new(normal.x as f32, normal.y as f32, normal.z as f32);

    // The face hemisphere extends from 0 to r+margin along the normal,
    // and from -(r*0.71) to +(r*0.71) perpendicular (sphere inscribed in cube:
    // at 45 degrees the sphere surface is at r/sqrt(2) ≈ 0.707r).
    let perp_extent = r * 0.75; // slightly over 1/sqrt(2) for margin

    // Along the normal: from 0 to r+margin
    // Perpendicular: ±perp_extent in both tangent and bitangent
    let tangent = face.tangent();
    let bitangent = face.bitangent();
    let t = Vec3::new(tangent.x as f32, tangent.y as f32, tangent.z as f32);
    let b = Vec3::new(bitangent.x as f32, bitangent.y as f32, bitangent.z as f32);
    let perp = t.abs() * perp_extent + b.abs() * perp_extent;

    // min corner: along normal from 0, perpendicular negative
    // max corner: along normal to r+margin, perpendicular positive
    // Sign of normal matters: if n is negative, min is at n*(r+margin), max at 0
    let n_min = n.min(Vec3::ZERO) * (r + margin);
    let n_max = n.max(Vec3::ZERO) * (r + margin);

    Aabb::new(n_min - perp, n_max + perp)
}

/// Create an orbiting camera matrix that views the full planet.
///
/// The camera orbits at the given altitude above the planet surface,
/// with `orbit_angle` controlling the horizontal position and `tilt`
/// controlling the elevation angle above the equator.
pub fn create_orbit_camera(
    planet_radius: f32,
    altitude: f32,
    orbit_angle: f64,
    tilt: f32,
    aspect_ratio: f32,
) -> Mat4 {
    let dist = planet_radius + altitude;
    let eye = Vec3::new(
        (orbit_angle.sin() as f32) * dist * tilt.cos(),
        dist * tilt.sin(),
        (orbit_angle.cos() as f32) * dist * tilt.cos(),
    );
    let target = Vec3::ZERO;
    let up = Vec3::Y;

    let view = Mat4::look_at_rh(eye, target, up);
    let near = altitude * 0.01;
    let far = dist * 4.0;
    let proj = Mat4::perspective_rh(70.0_f32.to_radians(), aspect_ratio, near, far);
    proj * view
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_six_faces_created() {
        let planet = PlanetFaces::new_demo(1, 42);
        assert_eq!(planet.face_states.len(), 6);
        for (i, fs) in planet.face_states.iter().enumerate() {
            assert_eq!(fs.face, CubeFace::ALL[i]);
        }
    }

    #[test]
    fn test_all_faces_have_vertices() {
        let planet = PlanetFaces::new_demo(1, 42);
        for fs in &planet.face_states {
            assert!(
                !fs.vertices.is_empty(),
                "Face {:?} should have vertices",
                fs.face
            );
        }
    }

    #[test]
    fn test_face_culling_reduces_visible_faces() {
        let mut planet = PlanetFaces::new_demo(1, 42);
        let r = planet.planet_radius as f32;

        // Camera above planet looking UPWARD (away from planet).
        // Nothing should be visible because the planet is behind us.
        let eye = Vec3::new(0.0, r + 50.0, 0.0);
        let target = Vec3::new(0.0, r + 150.0, 0.0); // looking up
        let view = Mat4::look_at_rh(eye, target, Vec3::Z);
        // Reverse-Z for compatibility with engine's Frustum
        let proj = Mat4::perspective_rh(60.0_f32.to_radians(), 1.0, 10000.0, 0.1);
        let vp = proj * view;
        let frustum = Frustum::from_view_projection(&vp);

        let visible = planet.cull_faces(&frustum);

        // Looking away from planet, all faces should be behind the camera
        assert!(
            visible == 0,
            "Expected 0 visible faces when looking away from planet, got {visible}"
        );

        // Now look toward the planet -- some faces should be visible
        let target_down = Vec3::ZERO;
        let view_down = Mat4::look_at_rh(eye, target_down, Vec3::Z);
        let vp_down = proj * view_down;
        let frustum_down = Frustum::from_view_projection(&vp_down);

        let visible_down = planet.cull_faces(&frustum_down);
        assert!(
            visible_down > 0,
            "Expected some visible faces when looking at planet"
        );
    }

    #[test]
    fn test_visible_render_data_combines_faces() {
        let planet = PlanetFaces::new_demo(1, 42);
        let (verts, indices) = planet.visible_render_data();
        assert!(!verts.is_empty());
        assert!(!indices.is_empty());
        for &idx in &indices {
            assert!(
                (idx as usize) < verts.len(),
                "Index {idx} out of bounds (len={})",
                verts.len()
            );
        }
    }

    #[test]
    fn test_vertices_on_sphere_surface() {
        let planet = PlanetFaces::new_demo(1, 42);
        let r = planet.planet_radius as f32;
        for fs in &planet.face_states {
            for v in &fs.vertices {
                let pos = Vec3::from(v.position);
                let dist = pos.length();
                assert!(
                    (dist - r).abs() < 50.0,
                    "Vertex at dist {dist} too far from radius {r} on face {:?}",
                    fs.face
                );
            }
        }
    }

    #[test]
    fn test_orbit_camera_produces_valid_matrix() {
        let vp = create_orbit_camera(200.0, 100.0, 1.0, 0.5, 16.0 / 9.0);
        for col in 0..4 {
            for row in 0..4 {
                assert!(vp.col(col)[row].is_finite());
            }
        }
    }

    #[test]
    fn test_face_aabb_contains_face_center() {
        for face in CubeFace::ALL {
            let aabb = compute_face_aabb(face, 200.0);
            let normal = face.normal();
            let center = Vec3::new(
                normal.x as f32 * 200.0,
                normal.y as f32 * 200.0,
                normal.z as f32 * 200.0,
            );
            assert!(
                center.x >= aabb.min.x
                    && center.x <= aabb.max.x
                    && center.y >= aabb.min.y
                    && center.y <= aabb.max.y
                    && center.z >= aabb.min.z
                    && center.z <= aabb.max.z,
                "AABB for {:?} should contain face center {center:?}",
                face
            );
        }
    }
}
