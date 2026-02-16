//! Two-level frustum culling pipeline for planets.
//!
//! - **Level 1 (coarse):** Test the entire planet's bounding sphere (as an AABB)
//!   against the i128 [`Frustum128`] — skip all chunks if the planet is off-screen.
//! - **Level 2 (fine):** Test individual chunk bounding volumes against a
//!   camera-relative f32 [`LocalFrustum`] for per-chunk culling.

use glam::{Mat4, Vec3, Vec4};
use nebula_coords::{Frustum128, Intersection};
use nebula_math::{Aabb128, WorldPosition};

/// A planet's bounding volume for coarse frustum culling.
#[derive(Debug, Clone)]
pub struct PlanetBounds {
    /// Center of the planet in world space (i128 coordinates, millimeters).
    pub center: WorldPosition,
    /// Planet radius in millimeters (i128).
    pub radius: i128,
}

impl PlanetBounds {
    /// Convert the bounding sphere to an AABB for frustum testing.
    ///
    /// The [`Frustum128`] tests AABBs, not spheres, so we use the
    /// circumscribing AABB of the sphere.
    pub fn to_aabb(&self) -> Aabb128 {
        Aabb128::new(
            WorldPosition::new(
                self.center.x - self.radius,
                self.center.y - self.radius,
                self.center.z - self.radius,
            ),
            WorldPosition::new(
                self.center.x + self.radius,
                self.center.y + self.radius,
                self.center.z + self.radius,
            ),
        )
    }

    /// Test this planet against the i128 frustum.
    pub fn test_frustum(&self, frustum: &Frustum128) -> Intersection {
        frustum.contains_aabb(&self.to_aabb())
    }
}

/// Camera-relative frustum in f32 for fine-grained chunk culling.
///
/// Uses the Gribb/Hartmann method to extract six inward-pointing planes
/// from the view-projection matrix. Returns three-way [`Intersection`]
/// results (Inside/Outside/Intersecting) unlike [`nebula_render::Frustum`]
/// which only returns bool.
#[derive(Clone, Debug)]
pub struct LocalFrustum {
    /// Six plane normals (pointing inward) and distances as `Vec4(nx, ny, nz, d)`.
    planes: [Vec4; 6],
}

impl LocalFrustum {
    /// Extract frustum planes from a view-projection matrix.
    pub fn from_view_proj(vp: &Mat4) -> Self {
        let row0 = vp.row(0);
        let row1 = vp.row(1);
        let row2 = vp.row(2);
        let row3 = vp.row(3);

        let mut planes = [
            row3 + row0, // left
            row3 - row0, // right
            row3 + row1, // bottom
            row3 - row1, // top
            row3 + row2, // near
            row2,        // far (reverse-Z compatible)
        ];

        // Normalize each plane.
        for plane in &mut planes {
            let len = plane.truncate().length();
            if len > 1e-8 {
                *plane /= len;
            }
        }

        Self { planes }
    }

    /// Test an AABB (center + half_extents) against the frustum.
    pub fn test_aabb(&self, center: Vec3, half_extents: Vec3) -> Intersection {
        let mut all_inside = true;

        for plane in &self.planes {
            let normal = plane.truncate();
            let distance = plane.w;

            // Effective radius: projection of half_extents onto the plane normal.
            let effective_radius = half_extents.x * normal.x.abs()
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

/// Result of the two-level culling pipeline.
#[derive(Debug, Clone)]
pub struct CullResult {
    /// Whether the planet was entirely culled (level 1).
    pub planet_culled: bool,
    /// Number of chunks that were culled (level 2).
    pub chunks_culled: u32,
    /// Number of chunks that passed both levels.
    pub chunks_visible: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_coords::{Frustum128, Intersection};
    use nebula_math::WorldPosition;

    fn earth_like_planet() -> PlanetBounds {
        PlanetBounds {
            center: WorldPosition::new(0, 0, 0),
            radius: 6_371_000_000, // 6371 km in mm
        }
    }

    /// Build a Frustum128 looking from `position` in `forward` direction.
    fn build_test_frustum_128(
        position: WorldPosition,
        forward: nebula_coords::Vec3I64,
    ) -> Frustum128 {
        use nebula_coords::Vec3I64;

        // Derive right/up from forward
        let (right, up) = if forward.x.abs() > forward.z.abs() {
            (
                Vec3I64::new(-forward.y, forward.x, 0),
                Vec3I64::new(0, 0, 1_000_000),
            )
        } else {
            (Vec3I64::new(1_000_000, 0, 0), Vec3I64::new(0, 1_000_000, 0))
        };

        Frustum128::from_camera(
            &position,
            &forward,
            &right,
            &up,
            1_000,               // near: 1mm
            100_000_000_000_000, // far: 100 billion km
            (1, 1),              // 90° FOV
        )
    }

    #[test]
    fn test_planet_behind_camera_culled_entirely() {
        let frustum = build_test_frustum_128(
            WorldPosition::new(0, 0, 20_000_000_000_000),
            nebula_coords::Vec3I64::new(0, 0, 1_000_000), // looking +Z, away from origin
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
        let frustum = build_test_frustum_128(
            WorldPosition::new(0, 0, -20_000_000_000_000),
            nebula_coords::Vec3I64::new(0, 0, 1_000_000), // looking +Z, toward planet
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
            &(Mat4::perspective_rh(1.0, 1.0, 0.1, 10_000.0)
                * Mat4::look_at_rh(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y)),
        );

        // Chunk far to the right, outside the FOV.
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
            &(Mat4::perspective_rh(1.0, 1.0, 0.1, 10_000.0)
                * Mat4::look_at_rh(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y)),
        );

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
        let planet_radius = 1000.0_f32;
        let camera_pos = Vec3::new(0.0, planet_radius + 10.0, 0.0);
        let look_dir = Vec3::new(1.0, 0.0, 0.0).normalize();
        let vp = Mat4::perspective_rh(std::f32::consts::FRAC_PI_2, 1.0, 1.0, 5000.0)
            * Mat4::look_at_rh(camera_pos, camera_pos + look_dir, Vec3::Y);
        let frustum = LocalFrustum::from_view_proj(&vp);

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
            "Expected >50% culled, got {:.1}% ({visible}/{total_chunks} visible)",
            cull_ratio * 100.0
        );
    }

    #[test]
    fn test_planet_bounds_to_aabb() {
        let planet = earth_like_planet();
        let aabb = planet.to_aabb();
        assert_eq!(aabb.min.x, -6_371_000_000);
        assert_eq!(aabb.max.x, 6_371_000_000);
    }

    #[test]
    fn test_local_frustum_behind_camera() {
        let frustum = LocalFrustum::from_view_proj(
            &(Mat4::perspective_rh(1.0, 1.0, 0.1, 1000.0)
                * Mat4::look_at_rh(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y)),
        );
        // Chunk behind camera (positive Z in RH looking -Z)
        let result = frustum.test_aabb(Vec3::new(0.0, 0.0, 100.0), Vec3::splat(5.0));
        assert_eq!(result, Intersection::Outside);
    }
}
