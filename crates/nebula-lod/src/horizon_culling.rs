//! Horizon culling for spherical planets.
//!
//! Eliminates chunks that are geometrically below the horizon, exploiting
//! the known spherical geometry of the planet. At surface level this can
//! cull ~50% of all chunks.

use glam::DVec3;

/// Horizon culling state computed from camera position and planet geometry.
#[derive(Clone, Debug)]
pub struct HorizonCuller {
    /// Camera position in world space (as f64 for precision).
    camera_pos: DVec3,
    /// Planet center in world space.
    planet_center: DVec3,
    /// Planet radius.
    radius: f64,
    /// Distance from camera to planet center.
    camera_distance: f64,
    /// The cosine of the horizon angle (angle from camera-to-center axis
    /// to the tangent line). Used for fast cone tests.
    cos_horizon: f64,
    /// The horizon plane distance from camera along the camera-to-center axis.
    /// Points beyond this distance along the axis are below the horizon.
    horizon_plane_dist: f64,
}

impl HorizonCuller {
    /// Create a new horizon culler from camera and planet parameters.
    pub fn new(camera_pos: DVec3, planet_center: DVec3, radius: f64) -> Self {
        let to_center = planet_center - camera_pos;
        let camera_distance = to_center.length();

        // cos(horizon_angle) = r / d
        let cos_horizon = if camera_distance > radius {
            radius / camera_distance
        } else {
            // Camera is inside the planet — everything is visible.
            0.0
        };

        // The horizon plane is at distance d * cos^2(horizon_angle) from camera
        let horizon_plane_dist = camera_distance * cos_horizon * cos_horizon;

        Self {
            camera_pos,
            planet_center,
            radius,
            camera_distance,
            cos_horizon,
            horizon_plane_dist,
        }
    }

    /// Test whether a chunk's bounding sphere is above the horizon (visible).
    ///
    /// Returns `true` if the chunk might be visible, `false` if it is
    /// definitely below the horizon and can be culled.
    ///
    /// Uses the dot-product test: a surface point P is above the horizon iff
    /// the angle between (P - center) and (camera - center) is less than
    /// the horizon angle. For bounding spheres we apply a conservative
    /// angular margin.
    pub fn is_above_horizon(&self, chunk_center: DVec3, chunk_radius: f64) -> bool {
        if self.camera_distance <= self.radius {
            return true;
        }

        let center_to_chunk = chunk_center - self.planet_center;
        let chunk_dist = center_to_chunk.length();
        if chunk_dist < 1e-10 {
            // Chunk at planet center — always visible (degenerate)
            return true;
        }

        let center_to_camera = self.camera_pos - self.planet_center;

        // cos(angle) between center→chunk and center→camera
        let cos_angle = center_to_chunk.dot(center_to_camera) / (chunk_dist * self.camera_distance);

        // The horizon angle from the center's perspective: the half-angle of the
        // visible cap as seen from the center is (π/2 - horizon_angle_from_camera).
        // A point on the surface at angle α from the sub-camera direction is visible
        // if α < arccos(r/d) measured from the center? No.
        //
        // Correct test: a point P at distance ~r from center is above the horizon if
        //   dot(P - C, E - C) / (|P-C| * |E-C|) >= r / d
        // i.e., cos_angle >= cos_horizon = r / d
        //
        // For a bounding sphere, apply margin: sin(margin) ≈ chunk_radius / chunk_dist
        let cos_margin = if chunk_dist > chunk_radius {
            let sin_margin = chunk_radius / chunk_dist;
            (1.0 - sin_margin * sin_margin).max(0.0).sqrt()
        } else {
            0.0 // chunk_radius >= chunk_dist → always visible
        };

        // Visible if cos_angle >= cos_horizon * cos_margin - sin_horizon * sin_margin
        // = cos(horizon + margin), i.e., angle < horizon + margin
        // Using cos(a+b) = cos(a)*cos(b) - sin(a)*sin(b)
        let sin_horizon = (1.0 - self.cos_horizon * self.cos_horizon).max(0.0).sqrt();
        let sin_margin = if chunk_dist > chunk_radius {
            chunk_radius / chunk_dist
        } else {
            1.0
        };
        let cos_threshold = self.cos_horizon * cos_margin - sin_horizon * sin_margin;

        cos_angle >= cos_threshold
    }

    /// Return the straight-line distance from camera to the horizon tangent point.
    pub fn horizon_distance(&self) -> f64 {
        if self.camera_distance <= self.radius {
            return 0.0;
        }
        (self.camera_distance * self.camera_distance - self.radius * self.radius).sqrt()
    }

    /// Return the camera's altitude above the planet surface.
    pub fn camera_altitude(&self) -> f64 {
        (self.camera_distance - self.radius).max(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn earth_culler(altitude: f64) -> HorizonCuller {
        let radius = 6_400_000.0;
        let camera_pos = DVec3::new(0.0, radius + altitude, 0.0);
        let planet_center = DVec3::ZERO;
        HorizonCuller::new(camera_pos, planet_center, radius)
    }

    /// A chunk directly below the camera (between camera and planet center)
    /// should be visible.
    #[test]
    fn test_chunk_directly_below_is_visible() {
        let culler = earth_culler(1000.0);
        let chunk_center = DVec3::new(0.0, 6_400_000.0, 0.0);
        assert!(culler.is_above_horizon(chunk_center, 100.0));
    }

    /// A chunk on the exact opposite side of the planet should be culled.
    #[test]
    fn test_chunk_on_far_side_is_culled() {
        let culler = earth_culler(1000.0);
        let chunk_center = DVec3::new(0.0, -6_400_000.0, 0.0);
        assert!(
            !culler.is_above_horizon(chunk_center, 100.0),
            "chunk on far side of planet should be culled"
        );
    }

    /// Horizon distance should increase with camera altitude.
    #[test]
    fn test_horizon_distance_increases_with_altitude() {
        let culler_low = earth_culler(100.0);
        let culler_high = earth_culler(10_000.0);

        assert!(
            culler_high.horizon_distance() > culler_low.horizon_distance(),
            "higher altitude ({}) should see farther horizon than lower ({})",
            culler_high.horizon_distance(),
            culler_low.horizon_distance()
        );
    }

    /// Camera very far from the planet should see almost the entire visible hemisphere.
    #[test]
    fn test_camera_in_space_sees_hemisphere() {
        let radius = 6_400_000.0;
        let culler = earth_culler(radius * 100.0);

        // Near-side pole: directly below camera — visible
        let near = DVec3::new(0.0, radius, 0.0);
        assert!(culler.is_above_horizon(near, 100.0));

        // A point at ~45° from the sub-camera pole — clearly visible
        let mid_lat = DVec3::new(
            radius * 0.707,
            radius * 0.707,
            0.0,
        );
        assert!(culler.is_above_horizon(mid_lat, 100.0));

        // Antipodal point — culled
        let far = DVec3::new(0.0, -radius, 0.0);
        assert!(!culler.is_above_horizon(far, 100.0));
    }

    /// Chunks near the horizon should be handled conservatively.
    #[test]
    fn test_horizon_edge_chunk_is_visible() {
        let radius = 6_400_000.0;
        let culler = earth_culler(1000.0);

        let d = radius + 1000.0;
        let tangent_height = (radius * radius) / d;
        let tangent_perp_sq: f64 = radius * radius - tangent_height * tangent_height;
        let tangent_perp = tangent_perp_sq.max(0.0).sqrt();
        let tangent_point = DVec3::new(0.0, tangent_height, tangent_perp);

        assert!(
            culler.is_above_horizon(tangent_point, 500.0),
            "chunk straddling the horizon should be visible with sufficient bounding radius"
        );
    }
}
