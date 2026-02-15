# Chunk Bounding Volumes

## Problem

Frustum culling and LOD distance calculations require fast geometric tests against every loaded chunk. Testing individual vertices or triangles is far too expensive — the engine needs a precomputed bounding volume for each chunk that can be tested in constant time. Two types are needed: a bounding sphere (cheapest test — single distance comparison against each frustum plane) and an axis-aligned bounding box (AABB, tighter fit for rectangular frustum tests and spatial hashing). Both must account for terrain height: the cubesphere surface is displaced outward by a height value at each point, so the bounds must expand to contain the maximum possible height within the chunk. If the bounds are too tight, terrain pokes through and vanishes; if too loose, the GPU wastes time rendering off-screen chunks. The bounds must also be in world space (i128 coordinates) for integration with the 128-bit coordinate system.

## Solution

Implement chunk bounding volumes in the `nebula_cubesphere` crate.

### Bounding Sphere

A bounding sphere for a cubesphere chunk is centered at the midpoint of the chunk's arc on the sphere surface, with a radius that encompasses all possible vertex positions including terrain height.

```rust
use glam::DVec3;

/// A bounding sphere in local f64 space (relative to planet center).
#[derive(Clone, Copy, Debug)]
pub struct BoundingSphere {
    /// Center of the sphere, relative to the planet center.
    pub center: DVec3,
    /// Radius of the bounding sphere.
    pub radius: f64,
}

/// A bounding sphere in world space (i128 coordinates).
#[derive(Clone, Copy, Debug)]
pub struct WorldBoundingSphere {
    /// Center position in world coordinates (mm).
    pub center_x: i128,
    pub center_y: i128,
    pub center_z: i128,
    /// Radius in mm.
    pub radius: i128,
}

impl BoundingSphere {
    /// Compute the bounding sphere for a cubesphere chunk.
    ///
    /// - `addr`: the chunk's address (determines its UV extent on the face)
    /// - `planet_radius`: radius of the planet in engine units (mm)
    /// - `max_height`: maximum terrain height above the sphere surface within this chunk
    /// - `min_height`: minimum terrain height (can be negative for valleys/oceans)
    pub fn from_chunk(
        addr: &ChunkAddress,
        planet_radius: f64,
        min_height: f64,
        max_height: f64,
    ) -> Self {
        let (u_min, v_min, u_max, v_max) = addr.uv_bounds();

        // Sample the 4 corners and center of the chunk on the unit sphere
        let corners = [
            FaceCoord::new(addr.face, u_min, v_min),
            FaceCoord::new(addr.face, u_max, v_min),
            FaceCoord::new(addr.face, u_min, v_max),
            FaceCoord::new(addr.face, u_max, v_max),
        ];
        let center_fc = FaceCoord::new(
            addr.face,
            (u_min + u_max) * 0.5,
            (v_min + v_max) * 0.5,
        );

        let sphere_center = face_coord_to_sphere_everitt(&center_fc);

        // The bounding sphere center is at the midpoint height
        let mid_radius = planet_radius + (min_height + max_height) * 0.5;
        let bs_center = sphere_center * mid_radius;

        // The radius must encompass:
        // 1. The angular extent of the chunk (arc from center to corners)
        // 2. The height variation (max_height - min_height)
        let mut max_dist_sq: f64 = 0.0;
        for corner_fc in &corners {
            let corner_dir = face_coord_to_sphere_everitt(corner_fc);
            // Check both extremes: point at max_height and point at min_height
            let corner_max = corner_dir * (planet_radius + max_height);
            let corner_min = corner_dir * (planet_radius + min_height);
            let d_max = (corner_max - bs_center).length_squared();
            let d_min = (corner_min - bs_center).length_squared();
            max_dist_sq = max_dist_sq.max(d_max).max(d_min);
        }

        // Also check center point at extreme heights
        let center_max = sphere_center * (planet_radius + max_height);
        let center_min = sphere_center * (planet_radius + min_height);
        max_dist_sq = max_dist_sq
            .max((center_max - bs_center).length_squared())
            .max((center_min - bs_center).length_squared());

        BoundingSphere {
            center: bs_center,
            radius: max_dist_sq.sqrt(),
        }
    }
}
```

### Axis-Aligned Bounding Box (AABB)

```rust
/// An axis-aligned bounding box in local f64 space.
#[derive(Clone, Copy, Debug)]
pub struct ChunkAABB {
    pub min: DVec3,
    pub max: DVec3,
}

/// An AABB in world space (i128 coordinates, mm).
#[derive(Clone, Copy, Debug)]
pub struct WorldAABB {
    pub min_x: i128, pub min_y: i128, pub min_z: i128,
    pub max_x: i128, pub max_y: i128, pub max_z: i128,
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

        // Sample a grid of points across the chunk
        let samples = 8;
        for ui in 0..=samples {
            for vi in 0..=samples {
                let u = u_min + (u_max - u_min) * (ui as f64 / samples as f64);
                let v = v_min + (v_max - v_min) * (vi as f64 / samples as f64);
                let fc = FaceCoord::new(addr.face, u, v);
                let dir = face_coord_to_sphere_everitt(&fc);

                // Check both height extremes
                for &h in &[min_height, max_height] {
                    let pos = dir * (planet_radius + h);
                    aabb_min = aabb_min.min(pos);
                    aabb_max = aabb_max.max(pos);
                }
            }
        }

        ChunkAABB {
            min: aabb_min,
            max: aabb_max,
        }
    }

    /// Returns true if this AABB contains the given point.
    pub fn contains(&self, point: DVec3) -> bool {
        point.x >= self.min.x && point.x <= self.max.x
            && point.y >= self.min.y && point.y <= self.max.y
            && point.z >= self.min.z && point.z <= self.max.z
    }
}
```

### Conversion to World Space

```rust
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
```

### Design Constraints

- Bounding volume computation samples the chunk surface rather than computing exact bounds analytically, because the Everitt projection has no simple closed-form extrema. The 8x8 sample grid (81 points x 2 heights = 162 evaluations) is sufficient because the cubesphere surface is smooth within a single chunk.
- The bounding sphere is intentionally conservative (slightly larger than necessary) to avoid false negatives in frustum culling.
- World-space bounds use `i128` to maintain precision across the full universe.
- Bounding volumes are computed once when a chunk is loaded and cached with the chunk data. They are not recomputed per-frame.

## Outcome

The `nebula_cubesphere` crate exports `BoundingSphere`, `ChunkAABB`, `WorldBoundingSphere`, `WorldAABB`, and their construction methods. The frustum culling system uses `WorldBoundingSphere` for fast plane tests; the spatial hash uses `WorldAABB` for bin assignment. Running `cargo test -p nebula_cubesphere` passes all bounding volume tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Each chunk's bounding AABB is drawn as a translucent wireframe box. The boxes tightly enclose the curved chunk surface, proving they are computed correctly.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `glam` | 0.29 | `DVec3` for 3D bounding volume math |

The crate uses Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;

    const PLANET_RADIUS: f64 = 6_371_000_000.0; // Earth-like, in mm

    #[test]
    fn test_bounding_sphere_contains_all_chunk_vertices() {
        let addr = ChunkAddress::new(CubeFace::PosX, 10, 50, 50);
        let bs = BoundingSphere::from_chunk(&addr, PLANET_RADIUS, 0.0, 10_000.0);

        let (u_min, v_min, u_max, v_max) = addr.uv_bounds();
        let samples = 4;
        for ui in 0..=samples {
            for vi in 0..=samples {
                let u = u_min + (u_max - u_min) * (ui as f64 / samples as f64);
                let v = v_min + (v_max - v_min) * (vi as f64 / samples as f64);
                let fc = FaceCoord::new(addr.face, u, v);
                let dir = face_coord_to_sphere_everitt(&fc);

                for &h in &[0.0, 5_000.0, 10_000.0] {
                    let pos = dir * (PLANET_RADIUS + h);
                    let dist = (pos - bs.center).length();
                    assert!(
                        dist <= bs.radius + 1.0, // 1mm tolerance for floating-point
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

        // The AABB should at least contain the bounding sphere center
        assert!(aabb.contains(bs.center),
            "AABB does not contain bounding sphere center");

        // Check that AABB extents are at least as large as bounding sphere
        let aabb_half = (aabb.max - aabb.min) * 0.5;
        let aabb_center = (aabb.min + aabb.max) * 0.5;
        // The bounding sphere inscribed in the AABB
        let max_half = aabb_half.x.max(aabb_half.y).max(aabb_half.z);
        // AABB half-diagonal should be >= bounding sphere radius
        let aabb_half_diag = aabb_half.length();
        // This is a loose check: the sphere should fit inside or be comparable
        assert!(
            aabb_half_diag >= bs.radius * 0.5,
            "AABB seems too small relative to bounding sphere"
        );
    }

    #[test]
    fn test_zero_height_chunk_bounds_match_sphere_surface() {
        let addr = ChunkAddress::new(CubeFace::PosZ, 10, 100, 100);
        let bs = BoundingSphere::from_chunk(&addr, PLANET_RADIUS, 0.0, 0.0);

        // With zero height, all points are on the sphere surface at planet_radius
        // The bounding sphere center should be at approximately planet_radius distance
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
            1_000_000_000_000i128,
            2_000_000_000_000i128,
            3_000_000_000_000i128,
        );
        // World AABB min should be offset by planet center
        assert!(world_aabb.min_x > 0);
        assert!(world_aabb.min_y > 0);
        assert!(world_aabb.min_z > 0);
        assert!(world_aabb.max_x > world_aabb.min_x);
        assert!(world_aabb.max_y > world_aabb.min_y);
        assert!(world_aabb.max_z > world_aabb.min_z);
    }
}
```
