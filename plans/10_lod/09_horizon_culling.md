# Horizon Culling

## Problem

On a spherical planet, approximately half of all terrain is hidden below the horizon at any given time. When the camera is standing on the surface, the entire far hemisphere plus most of the near hemisphere beyond the tangent line is occluded by the planet's curvature. Standard frustum culling eliminates chunks outside the camera's field of view, but it cannot account for planetary curvature — chunks that are technically within the frustum but below the horizon would still be loaded and rendered uselessly. For a planet with millions of potential chunks, this wastes up to 50% of the LOD budget on invisible terrain. Horizon culling exploits the known spherical geometry of the planet to eliminate chunks that are geometrically impossible to see, providing a massive reduction in chunk count that compounds with frustum culling and LOD.

## Solution

Implement horizon culling in the `nebula_lod` crate by computing the horizon plane (or horizon cone) based on the camera's position relative to the planet center, and testing each chunk's bounding volume against it.

### Horizon Geometry

For a camera at distance `d` from the planet center (where `d > r`, the planet radius), the horizon forms a circle on the planet surface. The horizon plane passes through the tangent point and is perpendicular to the vector from the planet center to the camera.

```rust
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
        // This is the angle from the camera-to-center direction to the tangent line.
        let cos_horizon = if camera_distance > radius {
            radius / camera_distance
        } else {
            // Camera is inside the planet (shouldn't happen, but handle gracefully).
            // Everything is visible.
            0.0
        };

        // The horizon plane is at distance d * cos^2(horizon_angle) from the camera
        // along the camera-to-center axis.
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
    pub fn is_above_horizon(&self, chunk_center: DVec3, chunk_radius: f64) -> bool {
        if self.camera_distance <= self.radius {
            // Camera is at or below the surface; cannot cull by horizon.
            return true;
        }

        let to_chunk = chunk_center - self.camera_pos;
        let to_center_dir = (self.planet_center - self.camera_pos).normalize();

        // Project chunk center onto the camera-to-planet-center axis
        let proj = to_chunk.dot(to_center_dir);

        if proj < 0.0 {
            // Chunk is behind the camera (toward space) — always above horizon
            return true;
        }

        // Perpendicular distance from chunk center to the camera-to-center axis
        let perp_sq = to_chunk.length_squared() - proj * proj;
        let perp = perp_sq.max(0.0).sqrt();

        // At projection distance `proj` along the axis, the horizon cone
        // has a perpendicular radius determined by the horizon angle.
        // The cone test: chunk is visible if it's within the visible cone
        // plus the chunk's bounding radius.
        let sin_horizon = (1.0 - self.cos_horizon * self.cos_horizon).max(0.0).sqrt();
        let cone_radius_at_proj = proj * sin_horizon / self.cos_horizon;

        // If the chunk (including its bounding radius) is above the cone surface,
        // it's visible.
        if proj > self.horizon_plane_dist {
            // Beyond the horizon plane — need to be above the cone
            perp - chunk_radius < cone_radius_at_proj
        } else {
            // Before the horizon plane — visible
            true
        }
    }

    /// Return the distance from the camera to the horizon on the planet surface.
    /// This is the straight-line distance to the tangent point.
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
```

### Integration with Quadtree LOD

Horizon culling is applied during the quadtree traversal (story 02). Before evaluating a node for split/merge, the traversal checks the horizon culler:

```rust
fn should_process_node(
    node: &QuadNode,
    culler: &HorizonCuller,
    frustum: &ViewFrustum,
) -> bool {
    let sphere = node.bounding_sphere();

    // First: horizon culling (cheapest test)
    if !culler.is_above_horizon(sphere.center, sphere.radius) {
        return false;
    }

    // Second: frustum culling
    if !frustum.intersects_sphere(&sphere) {
        return false;
    }

    true
}
```

### Altitude-Dependent Effectiveness

The effectiveness of horizon culling depends on camera altitude:

| Camera Position | Approximate Culling | Chunks Saved |
|----------------|-------------------|--------------|
| On surface (0 m) | ~50% of planet | Maximum savings |
| Low altitude (1 km) | ~49% | Still very effective |
| High orbit (1000 km) | ~30% | Moderate savings |
| Very far (10x radius) | ~5% | Minimal savings |

At surface level, horizon culling is the single most impactful optimization — it eliminates half the planet from consideration before LOD and frustum culling even begin.

## Outcome

The `nebula_lod` crate exports `HorizonCuller` with `new()`, `is_above_horizon()`, `horizon_distance()`, and `camera_altitude()`. The quadtree LOD traversal creates a `HorizonCuller` each frame and uses it to skip nodes below the horizon. Running `cargo test -p nebula_lod` passes all horizon culling tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Chunks beyond the planet's horizon are culled. The console logs `Horizon culled: 42% of chunks`. Draw calls drop significantly when the camera is near the surface.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `nebula_math` | workspace | `WorldPosition`, `DVec3` (f64 vector type) |
| `glam` | `0.29` | `DVec3` for double-precision vector math |

The crate uses Rust edition 2024.

## Unit Tests

```rust
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
        let culler = earth_culler(1000.0); // 1 km altitude
        let chunk_center = DVec3::new(0.0, 6_400_000.0, 0.0); // on surface below camera
        assert!(culler.is_above_horizon(chunk_center, 100.0));
    }

    /// A chunk on the exact opposite side of the planet should be culled.
    #[test]
    fn test_chunk_on_far_side_is_culled() {
        let culler = earth_culler(1000.0);
        let chunk_center = DVec3::new(0.0, -6_400_000.0, 0.0); // antipodal point
        assert!(
            !culler.is_above_horizon(chunk_center, 100.0),
            "chunk on far side of planet should be culled"
        );
    }

    /// Horizon distance should increase with camera altitude.
    #[test]
    fn test_horizon_distance_increases_with_altitude() {
        let culler_low = earth_culler(100.0);     // 100 m
        let culler_high = earth_culler(10_000.0);  // 10 km

        assert!(
            culler_high.horizon_distance() > culler_low.horizon_distance(),
            "higher altitude ({}) should see farther horizon than lower ({})",
            culler_high.horizon_distance(),
            culler_low.horizon_distance()
        );
    }

    /// Camera very far from the planet (in deep space) should see almost
    /// the entire visible hemisphere.
    #[test]
    fn test_camera_in_space_sees_hemisphere() {
        let radius = 6_400_000.0;
        let culler = earth_culler(radius * 100.0); // 100x radius away

        // A chunk on the near side should be visible
        let near = DVec3::new(0.0, radius, 0.0);
        assert!(culler.is_above_horizon(near, 100.0));

        // A chunk at the equator (90 degrees from sub-camera point) should
        // still be visible from this far away
        let equator = DVec3::new(radius, 0.0, 0.0);
        assert!(culler.is_above_horizon(equator, 100.0));

        // But the antipodal point should still be culled
        let far = DVec3::new(0.0, -radius, 0.0);
        assert!(!culler.is_above_horizon(far, 100.0));
    }

    /// Chunks near the horizon should be handled correctly:
    /// a chunk whose bounding sphere straddles the horizon should be
    /// considered visible (conservative).
    #[test]
    fn test_horizon_edge_chunk_is_visible() {
        let radius = 6_400_000.0;
        let culler = earth_culler(1000.0);

        // A point exactly at the horizon tangent should be on the edge.
        // Compute the tangent point on the surface.
        let d = radius + 1000.0;
        let horizon_dist = culler.horizon_distance();

        // Tangent point in world space (approximate, on the +Z side)
        let tangent_height = (radius * radius) / d;
        let tangent_perp = (radius * radius - tangent_height * tangent_height).max(0.0).sqrt();
        let tangent_point = DVec3::new(0.0, tangent_height, tangent_perp);

        // With a large bounding radius, this chunk should be visible
        assert!(
            culler.is_above_horizon(tangent_point, 500.0),
            "chunk straddling the horizon should be visible with sufficient bounding radius"
        );
    }
}
```
