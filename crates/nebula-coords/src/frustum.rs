//! Frustum culling in 128-bit world space for astronomical-scale objects.
//!
//! This module provides a frustum defined entirely in 128-bit integer space
//! for culling planets, stars, and other distant objects before they reach
//! the f32 rendering pipeline. It operates as the first stage in a two-stage
//! culling pipeline where objects billions of kilometers away can be culled
//! with perfect precision.

use crate::{SectorIndex, Vec3I64, WorldPosition};
use nebula_math::Aabb128;

/// Result of testing an object against a plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaneSide {
    Inside,
    Outside,
    OnPlane,
}

/// Result of testing an AABB against the frustum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intersection {
    /// The object is entirely inside the frustum.
    Inside,
    /// The object is entirely outside the frustum.
    Outside,
    /// The object straddles one or more frustum planes.
    Intersecting,
}

/// A plane in 128-bit integer space.
/// The plane equation is: normal.dot(point) + distance > 0 means "inside".
/// Normal components are stored as i64 to prevent overflow when dotted with i128 positions.
/// The dot product result fits in i128 because i64 * i128 can be computed
/// by widening to i128 before multiplication.
#[derive(Debug, Clone, Copy)]
pub struct Plane128 {
    pub normal: Vec3I64,
    pub distance: i128,
}

impl Plane128 {
    /// Create a new plane from a normal vector and distance.
    pub fn new(normal: Vec3I64, distance: i128) -> Self {
        Self { normal, distance }
    }

    /// Classify a point relative to this plane.
    /// Returns positive if inside, negative if outside, zero if on the plane.
    pub fn signed_distance(&self, point: &WorldPosition) -> i128 {
        let nx = self.normal.x as i128;
        let ny = self.normal.y as i128;
        let nz = self.normal.z as i128;
        nx.wrapping_mul(point.x)
            .wrapping_add(ny.wrapping_mul(point.y))
            .wrapping_add(nz.wrapping_mul(point.z))
            .wrapping_add(self.distance)
    }

    /// Returns true if the point is on the inside (positive) half-space.
    pub fn contains_point(&self, point: &WorldPosition) -> bool {
        self.signed_distance(point) >= 0
    }

    /// Coarse sector-level test. Classifies the center of a sector.
    pub fn sector_side(&self, sector: &SectorIndex) -> PlaneSide {
        // Sector center = sector_index * 2^32 + 2^31 (midpoint).
        let half_sector: i128 = 1_i128 << 31;
        let center = WorldPosition {
            x: (sector.x << 32) + half_sector,
            y: (sector.y << 32) + half_sector,
            z: (sector.z << 32) + half_sector,
        };
        let d = self.signed_distance(&center);
        if d > 0 {
            PlaneSide::Inside
        } else if d < 0 {
            PlaneSide::Outside
        } else {
            PlaneSide::OnPlane
        }
    }
}

/// A view frustum defined in 128-bit world space for coarse culling
/// of distant objects (planets, stars, moons).
#[derive(Debug, Clone)]
pub struct Frustum128 {
    /// The six frustum planes, ordered: near, far, left, right, top, bottom.
    /// Each plane's normal points inward (toward the interior of the frustum).
    pub planes: [Plane128; 6],
}

impl Frustum128 {
    /// Create a new frustum from six planes.
    pub fn new(planes: [Plane128; 6]) -> Self {
        Self { planes }
    }

    /// Test whether a point is inside all six frustum planes.
    pub fn contains_point(&self, point: &WorldPosition) -> bool {
        self.planes.iter().all(|plane| plane.contains_point(point))
    }

    /// Test an AABB against the frustum using the "p-vertex / n-vertex" method.
    /// For each plane, find the vertex of the AABB most in the direction of the
    /// plane normal (p-vertex) and the vertex most against it (n-vertex).
    /// - If the n-vertex is inside, the AABB is fully inside that plane.
    /// - If the p-vertex is outside, the AABB is fully outside that plane (and
    ///   therefore outside the frustum).
    /// - Otherwise, the AABB intersects that plane.
    pub fn contains_aabb(&self, aabb: &Aabb128) -> Intersection {
        let mut all_inside = true;

        for plane in &self.planes {
            // p-vertex: for each axis, choose max if normal component > 0, else min.
            let px = if plane.normal.x >= 0 {
                aabb.max.x
            } else {
                aabb.min.x
            };
            let py = if plane.normal.y >= 0 {
                aabb.max.y
            } else {
                aabb.min.y
            };
            let pz = if plane.normal.z >= 0 {
                aabb.max.z
            } else {
                aabb.min.z
            };
            let p_vertex = WorldPosition {
                x: px,
                y: py,
                z: pz,
            };

            // n-vertex: opposite corners.
            let nx = if plane.normal.x >= 0 {
                aabb.min.x
            } else {
                aabb.max.x
            };
            let ny = if plane.normal.y >= 0 {
                aabb.min.y
            } else {
                aabb.max.y
            };
            let nz = if plane.normal.z >= 0 {
                aabb.min.z
            } else {
                aabb.max.z
            };
            let n_vertex = WorldPosition {
                x: nx,
                y: ny,
                z: nz,
            };

            if !plane.contains_point(&p_vertex) {
                // p-vertex is outside => entire AABB is outside this plane.
                return Intersection::Outside;
            }

            if !plane.contains_point(&n_vertex) {
                // n-vertex is outside => AABB straddles this plane.
                all_inside = false;
            }
        }

        if all_inside {
            Intersection::Inside
        } else {
            Intersection::Intersecting
        }
    }

    /// Build a frustum from camera parameters.
    ///
    /// - `position`: Camera world position (i128).
    /// - `forward`, `right`, `up`: Orientation vectors (i64, scaled to a
    ///   fixed magnitude like 1_000_000 to represent unit vectors).
    /// - `near`, `far`: Near and far plane distances in mm (i128).
    /// - `tan_half_fov_x`, `tan_half_fov_y`: Tangent of half the horizontal
    ///   and vertical field of view, encoded as `(numerator, denominator)` pairs
    ///   to avoid floating point.
    pub fn from_camera(
        position: &WorldPosition,
        forward: &Vec3I64,
        right: &Vec3I64,
        up: &Vec3I64,
        near: i128,
        far: i128,
        tan_half_fov: (i64, i64), // (numerator, denominator)
    ) -> Self {
        let (tan_num, tan_den) = tan_half_fov;

        // Near plane: normal = forward (points inward), plane at position + forward * near
        // For a point to be inside, it must be further than the near plane
        // Plane equation: forward.dot(point - near_point) >= 0
        // Rearranged: forward.dot(point) >= forward.dot(near_point)
        // So distance = -forward.dot(near_point)
        let near_normal = *forward;
        let near_point = WorldPosition {
            x: position.x + (forward.x as i128 * near) / 1_000_000,
            y: position.y + (forward.y as i128 * near) / 1_000_000,
            z: position.z + (forward.z as i128 * near) / 1_000_000,
        };
        let near_distance = -(near_normal.x as i128 * near_point.x
            + near_normal.y as i128 * near_point.y
            + near_normal.z as i128 * near_point.z);

        // Far plane: normal = -forward (points inward), plane at position + forward * far
        // For a point to be inside, it must be closer than the far plane
        // Plane equation: (-forward).dot(point - far_point) >= 0
        // Rearranged: -forward.dot(point) >= -forward.dot(far_point)
        // So: forward.dot(point) <= forward.dot(far_point)
        // Distance for (-forward) normal = -(-forward).dot(far_point) = forward.dot(far_point)
        let far_normal = Vec3I64::new(-forward.x, -forward.y, -forward.z);
        let far_point = WorldPosition {
            x: position.x + (forward.x as i128 * far) / 1_000_000,
            y: position.y + (forward.y as i128 * far) / 1_000_000,
            z: position.z + (forward.z as i128 * far) / 1_000_000,
        };
        let far_distance = forward.x as i128 * far_point.x
            + forward.y as i128 * far_point.y
            + forward.z as i128 * far_point.z;

        // Left plane: cross product of (forward + right * tan_half_fov) and up
        // Simplified: normal points right and slightly forward
        // For now, use a simplified approach assuming 90-degree FOV (tan = 1)
        let left_normal = Vec3I64::new(
            forward.x + (right.x * tan_num) / tan_den,
            forward.y + (right.y * tan_num) / tan_den,
            forward.z + (right.z * tan_num) / tan_den,
        );
        let left_distance = -(left_normal.x as i128 * position.x
            + left_normal.y as i128 * position.y
            + left_normal.z as i128 * position.z);

        // Right plane: cross product of (forward - right * tan_half_fov) and up
        let right_normal = Vec3I64::new(
            forward.x - (right.x * tan_num) / tan_den,
            forward.y - (right.y * tan_num) / tan_den,
            forward.z - (right.z * tan_num) / tan_den,
        );
        let right_distance = -(right_normal.x as i128 * position.x
            + right_normal.y as i128 * position.y
            + right_normal.z as i128 * position.z);

        // Top plane: similar to left/right but using up vector
        let top_normal = Vec3I64::new(
            forward.x - (up.x * tan_num) / tan_den,
            forward.y - (up.y * tan_num) / tan_den,
            forward.z - (up.z * tan_num) / tan_den,
        );
        let top_distance = -(top_normal.x as i128 * position.x
            + top_normal.y as i128 * position.y
            + top_normal.z as i128 * position.z);

        // Bottom plane
        let bottom_normal = Vec3I64::new(
            forward.x + (up.x * tan_num) / tan_den,
            forward.y + (up.y * tan_num) / tan_den,
            forward.z + (up.z * tan_num) / tan_den,
        );
        let bottom_distance = -(bottom_normal.x as i128 * position.x
            + bottom_normal.y as i128 * position.y
            + bottom_normal.z as i128 * position.z);

        Self::new([
            Plane128::new(near_normal, near_distance),
            Plane128::new(far_normal, far_distance),
            Plane128::new(left_normal, left_distance),
            Plane128::new(right_normal, right_distance),
            Plane128::new(top_normal, top_distance),
            Plane128::new(bottom_normal, bottom_distance),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Vec3I64;

    /// Helper function to create a simple frustum looking down +Z axis
    fn create_test_frustum() -> Frustum128 {
        let position = WorldPosition::new(0, 0, 0);
        let forward = Vec3I64::new(0, 0, 1_000_000); // Unit vector scaled to 1M
        let right = Vec3I64::new(1_000_000, 0, 0);
        let up = Vec3I64::new(0, 1_000_000, 0);
        let near = 1000; // 1mm near plane  
        let far = 1_000_000; // 1m far plane
        let tan_half_fov = (1, 1); // 45-degree half FOV (90 degrees total)

        Frustum128::from_camera(&position, &forward, &right, &up, near, far, tan_half_fov)
    }

    #[test]
    fn test_point_inside_frustum() {
        let frustum = create_test_frustum();
        let point = WorldPosition::new(0, 0, 500_000); // 500m ahead, on axis
        assert!(frustum.contains_point(&point));
    }

    #[test]
    fn test_point_behind_camera() {
        let frustum = create_test_frustum();
        let point = WorldPosition::new(0, 0, -100); // Behind the camera
        assert!(!frustum.contains_point(&point));
    }

    #[test]
    fn test_point_outside_left_plane() {
        let frustum = create_test_frustum();
        let point = WorldPosition::new(-1_000_000, 0, 100_000); // Far to the left, barely in front
        assert!(!frustum.contains_point(&point));
    }

    #[test]
    fn test_point_outside_right_plane() {
        let frustum = create_test_frustum();
        let point = WorldPosition::new(1_000_000, 0, 100_000); // Far to the right
        assert!(!frustum.contains_point(&point));
    }

    #[test]
    fn test_point_beyond_far_plane() {
        let frustum = create_test_frustum();
        let point = WorldPosition::new(0, 0, 2_000_000); // Beyond far plane
        assert!(!frustum.contains_point(&point));
    }

    #[test]
    fn test_aabb_fully_inside() {
        let frustum = create_test_frustum();
        let aabb = Aabb128::new(
            WorldPosition::new(-100, -100, 400_000),
            WorldPosition::new(100, 100, 600_000),
        );
        assert_eq!(frustum.contains_aabb(&aabb), Intersection::Inside);
    }

    #[test]
    fn test_aabb_fully_outside() {
        let frustum = create_test_frustum();
        let aabb = Aabb128::new(
            WorldPosition::new(-100, -100, -200),
            WorldPosition::new(100, 100, -100),
        );
        assert_eq!(frustum.contains_aabb(&aabb), Intersection::Outside);
    }

    #[test]
    fn test_aabb_intersecting() {
        let frustum = create_test_frustum();
        // AABB that straddles the near plane
        let aabb = Aabb128::new(
            WorldPosition::new(-100, -100, 500),
            WorldPosition::new(100, 100, 1500),
        );
        assert_eq!(frustum.contains_aabb(&aabb), Intersection::Intersecting);
    }

    #[test]
    fn test_degenerate_frustum_zero_volume() {
        let position = WorldPosition::new(0, 0, 0);
        let forward = Vec3I64::new(0, 0, 1_000_000);
        let right = Vec3I64::new(1_000_000, 0, 0);
        let up = Vec3I64::new(0, 1_000_000, 0);
        let near = 1000;
        let far = 1000; // Same as near - zero depth
        let tan_half_fov = (1, 1);

        let frustum =
            Frustum128::from_camera(&position, &forward, &right, &up, near, far, tan_half_fov);
        let point = WorldPosition::new(0, 0, 1001); // Slightly beyond the plane

        // With zero volume frustum, no point can satisfy both near and far planes
        // A point beyond the near plane should fail the far plane test
        assert!(!frustum.contains_point(&point));
    }

    #[test]
    fn test_large_distance_culling() {
        let position = WorldPosition::new(0, 0, 0);
        let forward = Vec3I64::new(0, 0, 1_000_000);
        let right = Vec3I64::new(1_000_000, 0, 0);
        let up = Vec3I64::new(0, 1_000_000, 0);
        let near = 1000;
        let far = 1_i128 << 60; // ~1.15 * 10^18 mm = ~1.15 billion km
        let tan_half_fov = (1, 1);

        let frustum =
            Frustum128::from_camera(&position, &forward, &right, &up, near, far, tan_half_fov);

        // Test a planet AABB centered at half the far distance with large radius
        let planet_center = WorldPosition::new(0, 0, 1_i128 << 59);
        let planet_radius = 1_i128 << 40; // Large planet
        let planet_aabb = Aabb128::new(
            WorldPosition::new(
                planet_center.x - planet_radius,
                planet_center.y - planet_radius,
                planet_center.z - planet_radius,
            ),
            WorldPosition::new(
                planet_center.x + planet_radius,
                planet_center.y + planet_radius,
                planet_center.z + planet_radius,
            ),
        );

        assert_eq!(frustum.contains_aabb(&planet_aabb), Intersection::Inside);

        // Test another AABB way off to the right
        let distant_aabb = Aabb128::new(
            WorldPosition::new(1_i128 << 62, 0, 0),
            WorldPosition::new((1_i128 << 62) + (1_i128 << 40), 1_i128 << 40, 1_i128 << 40),
        );
        assert_eq!(frustum.contains_aabb(&distant_aabb), Intersection::Outside);
    }

    #[test]
    fn test_plane_signed_distance() {
        let plane = Plane128::new(Vec3I64::new(1_000_000, 0, 0), 0); // X = 0 plane, normal pointing +X

        // Point on positive side
        let pos_point = WorldPosition::new(1000, 0, 0);
        assert!(plane.signed_distance(&pos_point) > 0);

        // Point on negative side
        let neg_point = WorldPosition::new(-1000, 0, 0);
        assert!(plane.signed_distance(&neg_point) < 0);

        // Point on plane
        let on_plane = WorldPosition::new(0, 0, 0);
        assert_eq!(plane.signed_distance(&on_plane), 0);
    }

    #[test]
    fn test_plane_contains_point() {
        let plane = Plane128::new(Vec3I64::new(0, 1_000_000, 0), 0); // Y = 0 plane, normal pointing +Y

        assert!(plane.contains_point(&WorldPosition::new(0, 1000, 0))); // Above plane
        assert!(!plane.contains_point(&WorldPosition::new(0, -1000, 0))); // Below plane
        assert!(plane.contains_point(&WorldPosition::new(0, 0, 0))); // On plane (>= 0)
    }

    #[test]
    fn test_sector_side_classification() {
        let plane = Plane128::new(Vec3I64::new(1_000_000, 0, 0), 0); // X = 0 plane

        // Sector with positive X should be inside
        let pos_sector = SectorIndex { x: 1, y: 0, z: 0 };
        assert_eq!(plane.sector_side(&pos_sector), PlaneSide::Inside);

        // Sector with negative X should be outside
        let neg_sector = SectorIndex { x: -1, y: 0, z: 0 };
        assert_eq!(plane.sector_side(&neg_sector), PlaneSide::Outside);

        // Sector at origin should be on the positive side (center is at 2^31)
        let origin_sector = SectorIndex { x: 0, y: 0, z: 0 };
        assert_eq!(plane.sector_side(&origin_sector), PlaneSide::Inside);
    }
}
