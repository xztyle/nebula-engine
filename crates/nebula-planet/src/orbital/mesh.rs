//! Icosphere mesh generation for orbital planet rendering.

use glam::Vec3;

/// A mesh suitable for orbital-distance planet rendering.
pub struct OrbitalMesh {
    /// Vertex positions on the unit sphere.
    pub positions: Vec<Vec3>,
    /// Equirectangular UV coordinates per vertex.
    pub uvs: Vec<[f32; 2]>,
    /// Normal vectors (same as positions for a unit sphere).
    pub normals: Vec<Vec3>,
    /// Triangle indices.
    pub indices: Vec<u32>,
}

/// Generate an icosphere mesh with the given number of subdivisions.
///
/// Returns an [`OrbitalMesh`] with vertices on the unit sphere and
/// equirectangular UV coordinates. Subdivision 5 yields ~40k triangles,
/// subdivision 6 yields ~160k.
pub fn generate_orbital_sphere(subdivisions: u32) -> OrbitalMesh {
    // Start from an icosahedron
    let t = (1.0 + 5.0_f32.sqrt()) / 2.0;

    let mut positions: Vec<Vec3> = vec![
        Vec3::new(-1.0, t, 0.0),
        Vec3::new(1.0, t, 0.0),
        Vec3::new(-1.0, -t, 0.0),
        Vec3::new(1.0, -t, 0.0),
        Vec3::new(0.0, -1.0, t),
        Vec3::new(0.0, 1.0, t),
        Vec3::new(0.0, -1.0, -t),
        Vec3::new(0.0, 1.0, -t),
        Vec3::new(t, 0.0, -1.0),
        Vec3::new(t, 0.0, 1.0),
        Vec3::new(-t, 0.0, -1.0),
        Vec3::new(-t, 0.0, 1.0),
    ];

    // Normalize to unit sphere
    for p in &mut positions {
        *p = p.normalize();
    }

    let mut indices: Vec<u32> = vec![
        0, 11, 5, 0, 5, 1, 0, 1, 7, 0, 7, 10, 0, 10, 11, 1, 5, 9, 5, 11, 4, 11, 10, 2, 10, 7, 6, 7,
        1, 8, 3, 9, 4, 3, 4, 2, 3, 2, 6, 3, 6, 8, 3, 8, 9, 4, 9, 5, 2, 4, 11, 6, 2, 10, 8, 6, 7, 9,
        8, 1,
    ];

    // Subdivide
    for _ in 0..subdivisions {
        subdivide(&mut positions, &mut indices);
    }

    // Compute UVs and normals
    let normals: Vec<Vec3> = positions.clone();
    let uvs: Vec<[f32; 2]> = positions
        .iter()
        .map(|pos| {
            let u = 0.5 + pos.z.atan2(pos.x) / std::f32::consts::TAU;
            let v = 0.5 - pos.y.asin() / std::f32::consts::PI;
            [u, v]
        })
        .collect();

    OrbitalMesh {
        positions,
        uvs,
        normals,
        indices,
    }
}

/// Subdivide each triangle into 4 by splitting edges at midpoints.
fn subdivide(positions: &mut Vec<Vec3>, indices: &mut Vec<u32>) {
    use std::collections::HashMap;

    let mut midpoint_cache: HashMap<(u32, u32), u32> = HashMap::new();
    let mut new_indices = Vec::with_capacity(indices.len() * 4);

    let get_midpoint =
        |a: u32, b: u32, pos: &mut Vec<Vec3>, cache: &mut HashMap<(u32, u32), u32>| -> u32 {
            let key = if a < b { (a, b) } else { (b, a) };
            if let Some(&idx) = cache.get(&key) {
                return idx;
            }
            let mid = (pos[a as usize] + pos[b as usize]).normalize();
            let idx = pos.len() as u32;
            pos.push(mid);
            cache.insert(key, idx);
            idx
        };

    for tri in indices.chunks(3) {
        let (a, b, c) = (tri[0], tri[1], tri[2]);
        let ab = get_midpoint(a, b, positions, &mut midpoint_cache);
        let bc = get_midpoint(b, c, positions, &mut midpoint_cache);
        let ca = get_midpoint(c, a, positions, &mut midpoint_cache);

        new_indices.extend_from_slice(&[a, ab, ca]);
        new_indices.extend_from_slice(&[b, bc, ab]);
        new_indices.extend_from_slice(&[c, ca, bc]);
        new_indices.extend_from_slice(&[ab, bc, ca]);
    }

    *indices = new_indices;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_icosphere_vertices_on_unit_sphere() {
        let mesh = generate_orbital_sphere(5);
        for pos in &mesh.positions {
            let len = pos.length();
            assert!(
                (len - 1.0).abs() < 1e-5,
                "Orbital sphere vertex not on unit sphere: length = {len}"
            );
        }
    }

    #[test]
    fn test_icosphere_triangle_count() {
        let mesh = generate_orbital_sphere(5);
        let triangle_count = mesh.indices.len() / 3;
        assert!(
            triangle_count > 10_000,
            "Expected >10k triangles for subdivision 5, got {triangle_count}"
        );
    }

    #[test]
    fn test_icosphere_indices_valid() {
        let mesh = generate_orbital_sphere(3);
        let n = mesh.positions.len() as u32;
        for &idx in &mesh.indices {
            assert!(idx < n, "Index {idx} out of bounds (vertex count = {n})");
        }
    }

    #[test]
    fn test_uvs_in_range() {
        let mesh = generate_orbital_sphere(3);
        for uv in &mesh.uvs {
            assert!(uv[0] >= 0.0 && uv[0] <= 1.0, "U out of range: {}", uv[0]);
            assert!(uv[1] >= 0.0 && uv[1] <= 1.0, "V out of range: {}", uv[1]);
        }
    }

    #[test]
    fn test_normals_match_positions() {
        let mesh = generate_orbital_sphere(2);
        for (pos, norm) in mesh.positions.iter().zip(mesh.normals.iter()) {
            let diff = (*pos - *norm).length();
            assert!(diff < 1e-6, "Normal should equal position on unit sphere");
        }
    }
}
