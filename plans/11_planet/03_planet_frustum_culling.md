# Planet Frustum Culling

## Problem

Rendering a planet is expensive. A single planet might have thousands of active chunks, and a solar system might contain multiple planets, moons, and asteroids. The engine must avoid wasting any GPU time on geometry the player cannot see. Story 02 introduced face-level culling, but this story formalizes a proper two-level culling pipeline that works at both planetary and chunk granularity. Level 1 (coarse) tests the entire planet's bounding sphere against the 128-bit frustum from Epic 03 -- if the planet is entirely behind the camera or outside the field of view, every chunk on it is skipped in one test. Level 2 (fine) tests individual chunk bounding volumes against the camera-relative f32 frustum for precise per-chunk culling. Without this two-level approach, the engine either wastes time testing thousands of chunks on invisible planets (no coarse pass) or renders chunks that are off-screen (no fine pass).

## Solution

### Level 1: Coarse Planet Culling (i128)

Each planet has a world-space position (`WorldPosition`, i128) and a radius. Its bounding volume is a sphere. The coarse culling test checks this sphere against the `Frustum128`:

```rust
use nebula_coords::{Frustum128, Intersection, WorldPosition, AABB128};

/// A planet's bounding volume for coarse frustum culling.
#[derive(Debug, Clone)]
pub struct PlanetBounds {
    /// Center of the planet in world space (i128 coordinates).
    pub center: WorldPosition,
    /// Planet radius in millimeters (i128).
    pub radius: i128,
}

impl PlanetBounds {
    /// Convert the bounding sphere to an AABB for frustum testing.
    /// The Frustum128 tests AABBs, not spheres, so we use the
    /// circumscribing AABB of the sphere.
    pub fn to_aabb(&self) -> AABB128 {
        AABB128 {
            min: WorldPosition {
                x: self.center.x - self.radius,
                y: self.center.y - self.radius,
                z: self.center.z - self.radius,
            },
            max: WorldPosition {
                x: self.center.x + self.radius,
                y: self.center.y + self.radius,
                z: self.center.z + self.radius,
            },
        }
    }

    /// Test this planet against the i128 frustum.
    pub fn test_frustum(&self, frustum: &Frustum128) -> Intersection {
        frustum.contains_aabb(&self.to_aabb())
    }
}
```

If `test_frustum` returns `Intersection::Outside`, the planet is entirely invisible and the engine skips all chunk processing for it. If it returns `Inside` or `Intersecting`, the planet proceeds to level 2 culling.

### Level 2: Fine Chunk Culling (f32)

For planets that pass the coarse test, each active chunk's bounding volume is tested against the camera-relative f32 frustum. Chunk bounds are computed in local space (relative to the camera's coordinate origin) to maximize f32 precision:

```rust
use glam::{Vec3, Mat4};

/// Camera-relative frustum in f32 for fine-grained chunk culling.
pub struct LocalFrustum {
    /// Six plane normals (pointing inward) and distances.
    pub planes: [(Vec3, f32); 6],
}

impl LocalFrustum {
    /// Extract frustum planes from a view-projection matrix.
    pub fn from_view_proj(vp: &Mat4) -> Self {
        // Gribb/Hartmann method: extract planes from the rows of VP.
        let row0 = vp.row(0);
        let row1 = vp.row(1);
        let row2 = vp.row(2);
        let row3 = vp.row(3);

        let mut planes = [
            extract_plane(row3 + row0), // left
            extract_plane(row3 - row0), // right
            extract_plane(row3 + row1), // bottom
            extract_plane(row3 - row1), // top
            extract_plane(row3 + row2), // near (reverse-Z: near is w+z)
            extract_plane(row3 - row2), // far
        ];

        // Normalize each plane.
        for plane in &mut planes {
            let len = plane.0.length();
            if len > 1e-8 {
                plane.0 /= len;
                plane.1 /= len;
            }
        }

        Self { planes }
    }

    /// Test an AABB (center + half_extents) against the frustum.
    pub fn test_aabb(&self, center: Vec3, half_extents: Vec3) -> Intersection {
        let mut all_inside = true;

        for &(normal, distance) in &self.planes {
            // Effective radius: projection of half_extents onto the plane normal.
            let effective_radius =
                half_extents.x * normal.x.abs()
                + half_extents.y * normal.y.abs()
                + half_extents.z * normal.z.abs();

            let signed_dist = normal.dot(center) + distance;

            if signed_dist < -effective_radius {
                return Intersection::Outside;
            }
            if signed_dist < effective_radius {
                all_inside = false;
            }
        }

        if all_inside {
            Intersection::Inside
        } else {
            Intersection::Intersecting
        }
    }
}
```

### Two-Level Pipeline Integration

The complete culling pipeline runs every frame:

```rust
/// Result of the two-level culling pipeline.
pub struct CullResult {
    /// Planets that were entirely culled (level 1).
    pub planets_culled: u32,
    /// Chunks that were culled (level 2).
    pub chunks_culled: u32,
    /// Chunks that passed both levels and should be rendered.
    pub visible_chunks: Vec<ChunkDrawCommand>,
}

pub fn cull_planet(
    planet: &PlanetState,
    frustum_128: &Frustum128,
    frustum_local: &LocalFrustum,
    camera_origin: &WorldPosition,
) -> CullResult {
    let mut result = CullResult {
        planets_culled: 0,
        chunks_culled: 0,
        visible_chunks: Vec::new(),
    };

    // Level 1: coarse planet test in i128 space.
    let planet_test = planet.bounds.test_frustum(frustum_128);
    if planet_test == Intersection::Outside {
        result.planets_culled = 1;
        return result;
    }

    // Level 2: fine per-chunk test in f32 local space.
    for chunk in &planet.active_chunks {
        // Convert chunk bounds from world space to camera-relative local space.
        let local_center = chunk.world_center.to_local_f32(camera_origin);
        let local_half_extents = chunk.half_extents_f32();

        let chunk_test = frustum_local.test_aabb(local_center, local_half_extents);
        if chunk_test == Intersection::Outside {
            result.chunks_culled += 1;
        } else {
            result.visible_chunks.push(chunk.draw_command());
        }
    }

    result
}
```

### Coordinate Conversion for Level 2

Chunk world positions are stored as `WorldPosition` (i128) but the f32 frustum operates in camera-relative coordinates. The conversion subtracts the camera's world position and casts to f32, which is safe because only nearby chunks (within f32 precision range) reach level 2:

```rust
impl WorldPosition {
    /// Convert to a camera-relative f32 position.
    /// Precision is only valid for positions within ~10 km of the camera.
    pub fn to_local_f32(&self, camera: &WorldPosition) -> Vec3 {
        Vec3::new(
            (self.x - camera.x) as f32,
            (self.y - camera.y) as f32,
            (self.z - camera.z) as f32,
        )
    }
}
```

## Outcome

A two-level frustum culling pipeline in the `nebula-planet` crate. Level 1 uses the `Frustum128` to discard entire planets in a single test. Level 2 uses a camera-relative `LocalFrustum` to discard individual chunks. The pipeline exports `cull_planet()` which returns a `CullResult` containing the list of visible chunks and culling statistics. In typical views (camera on a planet surface looking at the horizon), culling eliminates more than 50% of chunks.

## Demo Integration

**Demo crate:** `nebula-demo`

The console logs `Frustum culled: 52% of chunks (148 visible / 308 loaded)`. Draw calls drop significantly when looking away from the planet.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `glam` | `0.29` | f32 frustum planes, AABB math, matrix decomposition |

Internal dependencies: `nebula-coords` (Frustum128, AABB128, WorldPosition), `nebula-cubesphere` (ChunkAddress), `nebula-math` (IVec3_128). Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nebula_coords::{Frustum128, Intersection, WorldPosition};

    fn earth_like_planet() -> PlanetBounds {
        PlanetBounds {
            center: WorldPosition { x: 0, y: 0, z: 0 },
            radius: 6_371_000_000, // 6371 km in mm
        }
    }

    #[test]
    fn test_planet_behind_camera_culled_entirely() {
        // Camera at (0, 0, 20_000_000_000) looking down +Z (away from origin).
        // The planet is at the origin, behind the camera.
        let frustum = build_frustum_128(
            WorldPosition { x: 0, y: 0, z: 20_000_000_000_000 },
            IVec3_64 { x: 0, y: 0, z: 1 },  // forward = +Z
            90,
            1_000,
            100_000_000_000_000,
        );
        let planet = earth_like_planet();
        let result = planet.test_frustum(&frustum);
        assert_eq!(
            result,
            Intersection::Outside,
            "Planet at origin should be culled when camera faces away from it"
        );
    }

    #[test]
    fn test_planet_in_view_passes_coarse_cull() {
        // Camera at (0, 0, -20_000_000_000) looking toward origin (+Z direction).
        let frustum = build_frustum_128(
            WorldPosition { x: 0, y: 0, z: -20_000_000_000_000 },
            IVec3_64 { x: 0, y: 0, z: 1 },  // forward = +Z, toward planet
            90,
            1_000,
            100_000_000_000_000,
        );
        let planet = earth_like_planet();
        let result = planet.test_frustum(&frustum);
        assert_ne!(
            result,
            Intersection::Outside,
            "Planet should be visible when camera looks directly at it"
        );
    }

    #[test]
    fn test_offscreen_chunks_not_rendered() {
        let frustum_local = LocalFrustum::from_view_proj(
            &Mat4::perspective_rh(1.0, 1.0, 0.1, 10_000.0)
                * Mat4::look_at_rh(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y),
        );

        // Chunk far to the right, outside the 57-degree half-FOV.
        let offscreen_center = Vec3::new(50_000.0, 0.0, -100.0);
        let half_extents = Vec3::splat(16.0);
        let result = frustum_local.test_aabb(offscreen_center, half_extents);
        assert_eq!(
            result,
            Intersection::Outside,
            "Chunk far to the right should be culled"
        );
    }

    #[test]
    fn test_onscreen_chunks_always_rendered() {
        let frustum_local = LocalFrustum::from_view_proj(
            &Mat4::perspective_rh(1.0, 1.0, 0.1, 10_000.0)
                * Mat4::look_at_rh(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y),
        );

        // Chunk directly ahead, well within the frustum.
        let onscreen_center = Vec3::new(0.0, 0.0, -500.0);
        let half_extents = Vec3::splat(16.0);
        let result = frustum_local.test_aabb(onscreen_center, half_extents);
        assert_ne!(
            result,
            Intersection::Outside,
            "Chunk directly ahead should NOT be culled"
        );
    }

    #[test]
    fn test_culling_reduces_draw_calls_by_50_percent() {
        // Simulate a surface view: camera on the planet surface looking at the horizon.
        // Only the forward hemisphere of chunks should survive culling.
        let planet_radius = 1000.0_f32;
        let camera_pos = Vec3::new(0.0, planet_radius + 10.0, 0.0);
        let look_dir = Vec3::new(1.0, 0.0, 0.0).normalize();
        let vp = Mat4::perspective_rh(std::f32::consts::FRAC_PI_2, 1.0, 1.0, 5000.0)
            * Mat4::look_at_rh(camera_pos, camera_pos + look_dir, Vec3::Y);
        let frustum = LocalFrustum::from_view_proj(&vp);

        // Scatter chunks uniformly around the planet surface.
        let total_chunks = 1000;
        let mut visible = 0;
        for i in 0..total_chunks {
            let theta = (i as f32 / total_chunks as f32) * std::f32::consts::TAU;
            let phi = ((i * 7 + 3) as f32 / total_chunks as f32) * std::f32::consts::PI;
            let pos = Vec3::new(
                planet_radius * phi.sin() * theta.cos(),
                planet_radius * phi.cos(),
                planet_radius * phi.sin() * theta.sin(),
            );
            let center = pos - camera_pos;
            let result = frustum.test_aabb(center, Vec3::splat(8.0));
            if result != Intersection::Outside {
                visible += 1;
            }
        }

        let cull_ratio = 1.0 - (visible as f32 / total_chunks as f32);
        assert!(
            cull_ratio > 0.5,
            "Expected >50% of chunks culled, but only {:.1}% were culled ({visible}/{total_chunks} visible)",
            cull_ratio * 100.0
        );
    }
}
```
