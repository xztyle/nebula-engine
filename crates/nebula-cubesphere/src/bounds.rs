//! Chunk bounding volumes for frustum culling and spatial queries.

use glam::DVec3;

use crate::{ChunkAddress, FaceCoord, face_coord_to_sphere_everitt};

/// A bounding sphere in local f64 space (relative to planet center).
#[derive(Clone, Copy, Debug)]
pub struct BoundingSphere {
    /// Center of the sphere, relative to the planet center.
    pub center: DVec3,
    /// Radius of the bounding sphere.
    pub radius: f64,
}

/// A bounding sphere in world space (i128 coordinates, mm).
#[derive(Clone, Copy, Debug)]
pub struct WorldBoundingSphere {
    /// Center X position in world coordinates (mm).
    pub center_x: i128,
    /// Center Y position in world coordinates (mm).
    pub center_y: i128,
    /// Center Z position in world coordinates (mm).
    pub center_z: i128,
    /// Radius in mm.
    pub radius: i128,
}

/// An axis-aligned bounding box in local f64 space.
#[derive(Clone, Copy, Debug)]
pub struct ChunkAABB {
    /// Minimum corner of the AABB.
    pub min: DVec3,
    /// Maximum corner of the AABB.
    pub max: DVec3,
}

/// An AABB in world space (i128 coordinates, mm).
#[derive(Clone, Copy, Debug)]
pub struct WorldAABB {
    /// Minimum X in world coordinates (mm).
    pub min_x: i128,
    /// Minimum Y in world coordinates (mm).
    pub min_y: i128,
    /// Minimum Z in world coordinates (mm).
    pub min_z: i128,
    /// Maximum X in world coordinates (mm).
    pub max_x: i128,
    /// Maximum Y in world coordinates (mm).
    pub max_y: i128,
    /// Maximum Z in world coordinates (mm).
    pub max_z: i128,
}

impl BoundingSphere {
    /// Compute the bounding sphere for a cubesphere chunk.
    ///
    /// - `addr`: the chunk's address (determines its UV extent on the face)
    /// - `planet_radius`: radius of the planet in engine units (mm)
    /// - `min_height`: minimum terrain height (can be negative for valleys/oceans)
    /// - `max_height`: maximum terrain height above the sphere surface within this chunk
    pub fn from_chunk(
        addr: &ChunkAddress,
        planet_radius: f64,
        min_height: f64,
        max_height: f64,
    ) -> Self {
        let (u_min, v_min, u_max, v_max) = addr.uv_bounds();

        let corners = [
            FaceCoord::new(addr.face, u_min, v_min),
            FaceCoord::new(addr.face, u_max, v_min),
            FaceCoord::new(addr.face, u_min, v_max),
            FaceCoord::new(addr.face, u_max, v_max),
        ];
        let center_fc = FaceCoord::new(addr.face, (u_min + u_max) * 0.5, (v_min + v_max) * 0.5);

        let sphere_center = face_coord_to_sphere_everitt(&center_fc);

        // Center at midpoint height
        let mid_radius = planet_radius + (min_height + max_height) * 0.5;
        let bs_center = sphere_center * mid_radius;

        // Radius must encompass angular extent and height variation
        let mut max_dist_sq: f64 = 0.0;
        for corner_fc in &corners {
            let corner_dir = face_coord_to_sphere_everitt(corner_fc);
            for &h in &[min_height, max_height] {
                let pos = corner_dir * (planet_radius + h);
                let d = (pos - bs_center).length_squared();
                max_dist_sq = max_dist_sq.max(d);
            }
        }

        // Also check center point at extreme heights
        let center_max = sphere_center * (planet_radius + max_height);
        let center_min = sphere_center * (planet_radius + min_height);
        max_dist_sq = max_dist_sq
            .max((center_max - bs_center).length_squared())
            .max((center_min - bs_center).length_squared());

        Self {
            center: bs_center,
            radius: max_dist_sq.sqrt(),
        }
    }
}

impl WorldBoundingSphere {
    /// Convert a local bounding sphere to world space by adding the planet center.
    pub fn from_local(
        local: &BoundingSphere,
        planet_center_x: i128,
        planet_center_y: i128,
        planet_center_z: i128,
    ) -> Self {
        Self {
            center_x: planet_center_x + local.center.x as i128,
            center_y: planet_center_y + local.center.y as i128,
            center_z: planet_center_z + local.center.z as i128,
            radius: local.radius.ceil() as i128,
        }
    }
}

impl ChunkAABB {
    /// Compute the AABB for a cubesphere chunk.
    ///
    /// Samples a grid of points on the chunk surface at both min and max
    /// terrain heights to find the tightest enclosing AABB.
    pub fn from_chunk(
        addr: &ChunkAddress,
        planet_radius: f64,
        min_height: f64,
        max_height: f64,
    ) -> Self {
        let (u_min, v_min, u_max, v_max) = addr.uv_bounds();

        let mut aabb_min = DVec3::splat(f64::MAX);
        let mut aabb_max = DVec3::splat(f64::MIN);

        let samples = 8u32;
        for ui in 0..=samples {
            for vi in 0..=samples {
                let u = u_min + (u_max - u_min) * (f64::from(ui) / f64::from(samples));
                let v = v_min + (v_max - v_min) * (f64::from(vi) / f64::from(samples));
                let fc = FaceCoord::new(addr.face, u, v);
                let dir = face_coord_to_sphere_everitt(&fc);

                for &h in &[min_height, max_height] {
                    let pos = dir * (planet_radius + h);
                    aabb_min = aabb_min.min(pos);
                    aabb_max = aabb_max.max(pos);
                }
            }
        }

        Self {
            min: aabb_min,
            max: aabb_max,
        }
    }

    /// Returns true if this AABB contains the given point.
    pub fn contains(&self, point: DVec3) -> bool {
        point.x >= self.min.x
            && point.x <= self.max.x
            && point.y >= self.min.y
            && point.y <= self.max.y
            && point.z >= self.min.z
            && point.z <= self.max.z
    }
}

impl WorldAABB {
    /// Convert a local AABB to world space by adding the planet center.
    pub fn from_local(
        local: &ChunkAABB,
        planet_center_x: i128,
        planet_center_y: i128,
        planet_center_z: i128,
    ) -> Self {
        Self {
            min_x: planet_center_x + local.min.x.floor() as i128,
            min_y: planet_center_y + local.min.y.floor() as i128,
            min_z: planet_center_z + local.min.z.floor() as i128,
            max_x: planet_center_x + local.max.x.ceil() as i128,
            max_y: planet_center_y + local.max.y.ceil() as i128,
            max_z: planet_center_z + local.max.z.ceil() as i128,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CubeFace;

    const PLANET_RADIUS: f64 = 6_371_000_000.0; // Earth-like, in mm

    #[test]
    fn test_bounding_sphere_contains_all_chunk_vertices() {
        let addr = ChunkAddress::new(CubeFace::PosX, 10, 50, 50);
        let bs = BoundingSphere::from_chunk(&addr, PLANET_RADIUS, 0.0, 10_000.0);

        let (u_min, v_min, u_max, v_max) = addr.uv_bounds();
        let samples = 4u32;
        for ui in 0..=samples {
            for vi in 0..=samples {
                let u = u_min + (u_max - u_min) * (f64::from(ui) / f64::from(samples));
                let v = v_min + (v_max - v_min) * (f64::from(vi) / f64::from(samples));
                let fc = FaceCoord::new(addr.face, u, v);
                let dir = face_coord_to_sphere_everitt(&fc);

                for &h in &[0.0, 5_000.0, 10_000.0] {
                    let pos = dir * (PLANET_RADIUS + h);
                    let dist = (pos - bs.center).length();
                    assert!(
                        dist <= bs.radius + 1.0,
                        "Vertex at ({u}, {v}, h={h}) is outside bounding sphere: \
                         dist={dist}, radius={}",
                        bs.radius
                    );
                }
            }
        }
    }

    #[test]
    fn test_aabb_encloses_bounding_sphere() {
        let addr = ChunkAddress::new(CubeFace::NegY, 8, 20, 30);
        let bs = BoundingSphere::from_chunk(&addr, PLANET_RADIUS, -1_000.0, 5_000.0);
        let aabb = ChunkAABB::from_chunk(&addr, PLANET_RADIUS, -1_000.0, 5_000.0);

        assert!(
            aabb.contains(bs.center),
            "AABB does not contain bounding sphere center"
        );

        let aabb_half = (aabb.max - aabb.min) * 0.5;
        let aabb_half_diag = aabb_half.length();
        assert!(
            aabb_half_diag >= bs.radius * 0.5,
            "AABB seems too small relative to bounding sphere"
        );
    }

    #[test]
    fn test_zero_height_chunk_bounds_match_sphere_surface() {
        let addr = ChunkAddress::new(CubeFace::PosZ, 10, 100, 100);
        let bs = BoundingSphere::from_chunk(&addr, PLANET_RADIUS, 0.0, 0.0);

        let center_dist = bs.center.length();
        assert!(
            (center_dist - PLANET_RADIUS).abs() < PLANET_RADIUS * 0.01,
            "Zero-height bounding sphere center should be near planet surface: \
             dist={center_dist}, radius={PLANET_RADIUS}"
        );
    }

    #[test]
    fn test_height_offset_expands_bounds() {
        let addr = ChunkAddress::new(CubeFace::PosY, 10, 50, 50);
        let bs_flat = BoundingSphere::from_chunk(&addr, PLANET_RADIUS, 0.0, 0.0);
        let bs_tall = BoundingSphere::from_chunk(&addr, PLANET_RADIUS, 0.0, 100_000.0);

        assert!(
            bs_tall.radius > bs_flat.radius,
            "Taller terrain should produce a larger bounding sphere"
        );
    }

    #[test]
    fn test_negative_height_expands_bounds() {
        let addr = ChunkAddress::new(CubeFace::NegX, 10, 50, 50);
        let bs_flat = BoundingSphere::from_chunk(&addr, PLANET_RADIUS, 0.0, 0.0);
        let bs_deep = BoundingSphere::from_chunk(&addr, PLANET_RADIUS, -50_000.0, 0.0);

        assert!(
            bs_deep.radius > bs_flat.radius,
            "Negative height (valleys) should expand the bounding sphere"
        );
    }

    #[test]
    fn test_world_aabb_from_local() {
        let addr = ChunkAddress::new(CubeFace::PosX, 15, 10, 10);
        let local_aabb = ChunkAABB::from_chunk(&addr, PLANET_RADIUS, 0.0, 1_000.0);
        let world_aabb = WorldAABB::from_local(
            &local_aabb,
            1_000_000_000_000_i128,
            2_000_000_000_000_i128,
            3_000_000_000_000_i128,
        );
        assert!(world_aabb.min_x > 0);
        assert!(world_aabb.min_y > 0);
        assert!(world_aabb.min_z > 0);
        assert!(world_aabb.max_x > world_aabb.min_x);
        assert!(world_aabb.max_y > world_aabb.min_y);
        assert!(world_aabb.max_z > world_aabb.min_z);
    }
}
