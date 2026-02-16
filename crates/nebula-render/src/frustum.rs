//! Local-space frustum culling using f32 AABB tests against view-projection planes.
//!
//! This is the second stage of the culling pipeline: after i128 coarse culling
//! (which rejects objects too far to convert to f32), this module tests AABBs
//! in local f32 space against the camera's view frustum extracted from the
//! view-projection matrix.

use glam::{Mat4, Vec3, Vec4};

/// Plane indices into the frustum planes array.
const LEFT: usize = 0;
const RIGHT: usize = 1;
const BOTTOM: usize = 2;
const TOP: usize = 3;
const NEAR: usize = 4;
const FAR: usize = 5;

/// An axis-aligned bounding box in local f32 space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    /// Minimum corner of the bounding box.
    pub min: Vec3,
    /// Maximum corner of the bounding box.
    pub max: Vec3,
}

impl Aabb {
    /// Create a new AABB from min and max corners.
    pub fn new(min: Vec3, max: Vec3) -> Self {
        Self { min, max }
    }

    /// Returns the center point of the AABB.
    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    /// Returns the half-extents (half-size along each axis).
    pub fn extents(&self) -> Vec3 {
        (self.max - self.min) * 0.5
    }
}

/// A view frustum defined by six inward-pointing planes extracted from
/// the view-projection matrix.
#[derive(Clone, Debug)]
pub struct Frustum {
    /// Six planes: left, right, bottom, top, near, far.
    /// Each `Vec4(a, b, c, d)` where `(a,b,c)` is the normalized inward
    /// normal and `d` is the signed distance term.
    planes: [Vec4; 6],
}

impl Frustum {
    /// Extract frustum planes from a combined view-projection matrix
    /// using the Griggs-Hartmann method.
    ///
    /// Works with both perspective and orthographic projections,
    /// including reverse-Z.
    pub fn from_view_projection(vp: &Mat4) -> Self {
        let rows = [vp.row(0), vp.row(1), vp.row(2), vp.row(3)];

        let mut planes = [Vec4::ZERO; 6];
        planes[LEFT] = rows[3] + rows[0];
        planes[RIGHT] = rows[3] - rows[0];
        planes[BOTTOM] = rows[3] + rows[1];
        planes[TOP] = rows[3] - rows[1];
        // With reverse-Z (near→z=1, far→z=0), the standard Griggs-Hartmann
        // row3±row2 encodes the near clip plane but not the far clip plane
        // correctly. Use row2 directly for the geometric far plane and
        // row3+row2 for the geometric near plane.
        planes[NEAR] = rows[3] + rows[2];
        planes[FAR] = rows[2];

        // Normalize each plane so that (a,b,c) is a unit vector.
        for plane in &mut planes {
            let len = plane.truncate().length();
            if len > 0.0 {
                *plane /= len;
            }
        }

        Self { planes }
    }

    /// Test whether an AABB is at least partially inside the frustum.
    ///
    /// Uses the p-vertex (positive vertex) method: for each plane, find
    /// the corner of the AABB furthest along the plane normal. If that
    /// corner is behind the plane, the entire AABB is outside.
    ///
    /// This is conservative — it may return `true` for some AABBs that
    /// are fully outside (false positives near frustum corners), but
    /// never returns `false` for visible objects.
    pub fn is_visible(&self, aabb: &Aabb) -> bool {
        for plane in &self.planes {
            let normal = plane.truncate();
            let d = plane.w;

            // Positive vertex: the corner furthest along the plane normal.
            let p = Vec3::new(
                if normal.x >= 0.0 {
                    aabb.max.x
                } else {
                    aabb.min.x
                },
                if normal.y >= 0.0 {
                    aabb.max.y
                } else {
                    aabb.min.y
                },
                if normal.z >= 0.0 {
                    aabb.max.z
                } else {
                    aabb.min.z
                },
            );

            if normal.dot(p) + d < 0.0 {
                return false;
            }
        }
        true
    }
}

/// Convenience wrapper for per-frame frustum culling.
///
/// Constructed once per frame from the camera's view-projection matrix,
/// then used to test each chunk/object before issuing draw calls.
pub struct FrustumCuller {
    frustum: Frustum,
}

impl FrustumCuller {
    /// Create a new culler from the camera's view-projection matrix.
    pub fn new(view_projection: &Mat4) -> Self {
        Self {
            frustum: Frustum::from_view_projection(view_projection),
        }
    }

    /// Returns `true` if the AABB is at least partially inside the frustum.
    pub fn is_visible(&self, aabb: &Aabb) -> bool {
        self.frustum.is_visible(aabb)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Mat4, Vec3};

    fn default_camera_vp() -> Mat4 {
        let view = Mat4::look_to_rh(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y);
        let proj = Mat4::perspective_rh(
            std::f32::consts::FRAC_PI_4,
            16.0 / 9.0,
            1000.0, // reverse-Z: far as near param
            0.1,    // reverse-Z: near as far param
        );
        proj * view
    }

    #[test]
    fn test_object_at_origin_visible() {
        let culler = FrustumCuller::new(&default_camera_vp());
        let aabb = Aabb::new(Vec3::new(-1.0, -1.0, -5.0), Vec3::new(1.0, 1.0, -3.0));
        assert!(culler.is_visible(&aabb));
    }

    #[test]
    fn test_object_behind_camera_not_visible() {
        let culler = FrustumCuller::new(&default_camera_vp());
        let aabb = Aabb::new(Vec3::new(-1.0, -1.0, 5.0), Vec3::new(1.0, 1.0, 10.0));
        assert!(!culler.is_visible(&aabb));
    }

    #[test]
    fn test_object_far_to_the_side_not_visible() {
        let culler = FrustumCuller::new(&default_camera_vp());
        let aabb = Aabb::new(Vec3::new(1000.0, -1.0, -6.0), Vec3::new(1002.0, 1.0, -4.0));
        assert!(!culler.is_visible(&aabb));
    }

    #[test]
    fn test_object_partially_in_frustum_is_visible() {
        let culler = FrustumCuller::new(&default_camera_vp());
        let aabb = Aabb::new(Vec3::new(-100.0, -1.0, -10.0), Vec3::new(1.0, 1.0, -5.0));
        assert!(culler.is_visible(&aabb));
    }

    #[test]
    fn test_all_six_planes_tested() {
        let culler = FrustumCuller::new(&default_camera_vp());

        // Behind camera
        let behind = Aabb::new(Vec3::splat(10.0), Vec3::splat(20.0));
        assert!(!culler.is_visible(&behind));

        // Far left
        let left = Aabb::new(Vec3::new(-1000.0, 0.0, -5.0), Vec3::new(-999.0, 1.0, -4.0));
        assert!(!culler.is_visible(&left));

        // Far right
        let right = Aabb::new(Vec3::new(999.0, 0.0, -5.0), Vec3::new(1000.0, 1.0, -4.0));
        assert!(!culler.is_visible(&right));

        // Far above
        let above = Aabb::new(Vec3::new(0.0, 999.0, -5.0), Vec3::new(1.0, 1000.0, -4.0));
        assert!(!culler.is_visible(&above));

        // Far below
        let below = Aabb::new(Vec3::new(0.0, -1000.0, -5.0), Vec3::new(1.0, -999.0, -4.0));
        assert!(!culler.is_visible(&below));

        // Beyond far plane
        let beyond_far = Aabb::new(Vec3::new(0.0, 0.0, -2000.0), Vec3::new(1.0, 1.0, -1500.0));
        assert!(!culler.is_visible(&beyond_far));
    }

    #[test]
    fn test_aabb_center_and_extents() {
        let aabb = Aabb::new(Vec3::new(-2.0, -3.0, -4.0), Vec3::new(2.0, 3.0, 4.0));
        assert_eq!(aabb.center(), Vec3::ZERO);
        assert_eq!(aabb.extents(), Vec3::new(2.0, 3.0, 4.0));
    }

    #[test]
    fn test_frustum_has_six_planes() {
        let frustum = Frustum::from_view_projection(&default_camera_vp());
        assert_eq!(frustum.planes.len(), 6);
        for plane in &frustum.planes {
            let normal_len = plane.truncate().length();
            assert!(
                (normal_len - 1.0).abs() < 1e-4,
                "plane normal not normalized: {normal_len}"
            );
        }
    }
}
