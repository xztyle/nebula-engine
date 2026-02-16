//! Point light: localized light source with inverse-square attenuation.
//!
//! Provides [`PointLight`] (CPU), [`PointLightGpu`] (GPU), and [`PointLightManager`]
//! which culls, sorts, and uploads up to 256 visible lights per frame.

use bytemuck::{Pod, Zeroable};

/// CPU-side point light descriptor.
#[derive(Clone, Debug)]
pub struct PointLight {
    /// Position in world-local coordinates (relative to the current origin chunk).
    pub position: glam::Vec3,
    /// Linear RGB color.
    pub color: glam::Vec3,
    /// Luminous intensity. Higher values push the light further.
    pub intensity: f32,
    /// Maximum radius of effect. Beyond this distance, contribution is zero.
    pub radius: f32,
}

/// Compute attenuation at a given distance from a point light.
///
/// Uses inverse-square falloff with a smooth windowing function that reaches
/// exactly zero at the cutoff radius. Returns a value in `[0.0, 1.0]`.
pub fn attenuation(distance: f32, radius: f32) -> f32 {
    if distance >= radius {
        return 0.0;
    }
    // Inverse-square falloff (+1.0 prevents singularity at d=0).
    let inv_sq = 1.0 / (distance * distance + 1.0);
    // Smooth windowing: (1 - (d/r)^2)^2 ensures C1 continuity at the boundary.
    let ratio = distance / radius;
    let t = (1.0 - ratio * ratio).max(0.0);
    let window = t * t;
    inv_sq * window
}

/// Per-light GPU data, 48 bytes, std430-compatible.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct PointLightGpu {
    /// xyz = position (view space), w = radius.
    pub position_radius: [f32; 4],
    /// xyz = color (linear RGB), w = intensity.
    pub color_intensity: [f32; 4],
    /// Padding to maintain 48-byte stride.
    pub _padding: [f32; 4],
}

/// Header at the start of the light storage buffer.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct PointLightHeader {
    /// Number of active point lights in the buffer.
    pub count: u32,
    /// Padding to align to 16 bytes.
    pub _pad: [u32; 3],
}

/// Manages point lights: culling, sorting, and GPU upload.
pub struct PointLightManager {
    /// All registered point lights in the scene.
    all_lights: Vec<PointLight>,
    /// Scratch buffer reused each frame for culled/sorted lights.
    visible_scratch: Vec<PointLightGpu>,
}

impl PointLightManager {
    /// Maximum number of point lights sent to the GPU per frame.
    pub const MAX_VISIBLE_LIGHTS: usize = 256;

    /// Size in bytes of the GPU storage buffer (header + max lights).
    pub const BUFFER_SIZE: u64 = std::mem::size_of::<PointLightHeader>() as u64
        + (Self::MAX_VISIBLE_LIGHTS as u64 * std::mem::size_of::<PointLightGpu>() as u64);

    /// Create a new empty manager.
    pub fn new() -> Self {
        Self {
            all_lights: Vec::new(),
            visible_scratch: Vec::with_capacity(Self::MAX_VISIBLE_LIGHTS),
        }
    }

    /// Add a point light to the scene.
    pub fn add(&mut self, light: PointLight) {
        self.all_lights.push(light);
    }

    /// Remove all point lights.
    pub fn clear(&mut self) {
        self.all_lights.clear();
    }

    /// Number of registered lights (before culling).
    pub fn len(&self) -> usize {
        self.all_lights.len()
    }

    /// Returns `true` if no lights are registered.
    pub fn is_empty(&self) -> bool {
        self.all_lights.is_empty()
    }

    /// Cull and sort lights, returning the visible list (no GPU upload).
    ///
    /// Used for testing and for the actual upload path.
    pub fn cull_to_list(&self, camera_pos: glam::Vec3, frustum: &Frustum) -> Vec<PointLightGpu> {
        let mut result: Vec<(f32, PointLightGpu)> = Vec::new();

        for light in &self.all_lights {
            let to_light = light.position - camera_pos;
            let dist = to_light.length();

            // Distance cull: if the light's sphere doesn't reach the camera area.
            if dist > light.radius + 1000.0 {
                continue;
            }

            // Frustum cull: test light's bounding sphere.
            if !frustum.is_sphere_visible(light.position, light.radius) {
                continue;
            }

            result.push((
                dist,
                PointLightGpu {
                    position_radius: [
                        light.position.x,
                        light.position.y,
                        light.position.z,
                        light.radius,
                    ],
                    color_intensity: [light.color.x, light.color.y, light.color.z, light.intensity],
                    _padding: [0.0; 4],
                },
            ));
        }

        // Sort by distance (closest first).
        result.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

        // Truncate to max.
        result.truncate(Self::MAX_VISIBLE_LIGHTS);

        result.into_iter().map(|(_, gpu)| gpu).collect()
    }

    /// Cull, sort, and upload visible lights to the GPU storage buffer.
    pub fn cull_and_upload(
        &mut self,
        camera_pos: glam::Vec3,
        frustum: &Frustum,
        queue: &wgpu::Queue,
        buffer: &wgpu::Buffer,
    ) {
        self.visible_scratch = self.cull_to_list(camera_pos, frustum);

        let header = PointLightHeader {
            count: self.visible_scratch.len() as u32,
            _pad: [0; 3],
        };

        queue.write_buffer(buffer, 0, bytemuck::cast_slice(&[header]));
        if !self.visible_scratch.is_empty() {
            queue.write_buffer(
                buffer,
                std::mem::size_of::<PointLightHeader>() as u64,
                bytemuck::cast_slice(&self.visible_scratch),
            );
        }
    }

    /// Number of lights visible after the last cull.
    pub fn visible_count(&self) -> usize {
        self.visible_scratch.len()
    }
}

impl Default for PointLightManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Minimal frustum interface for point light culling.
///
/// This wraps the frustum planes to provide sphere visibility testing.
/// For production use, this delegates to `nebula_render::Frustum`.
pub struct Frustum {
    planes: [glam::Vec4; 6],
}

impl Frustum {
    /// Create a frustum that accepts everything (for testing).
    pub fn infinite() -> Self {
        Self {
            planes: [glam::Vec4::ZERO; 6],
        }
    }

    /// Create from six inward-pointing planes (same format as `nebula_render::Frustum`).
    pub fn from_planes(planes: [glam::Vec4; 6]) -> Self {
        Self { planes }
    }

    /// Test whether a sphere is at least partially inside the frustum.
    pub fn is_sphere_visible(&self, center: glam::Vec3, radius: f32) -> bool {
        for plane in &self.planes {
            let normal = plane.truncate();
            let d = plane.w;
            // If the normal is zero (infinite frustum), skip.
            if normal.length_squared() < 1e-10 {
                continue;
            }
            let signed_dist = normal.dot(center) + d;
            if signed_dist < -radius {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_light_at_distance_zero_has_full_intensity() {
        let atten = attenuation(0.0, 10.0);
        assert!(
            (atten - 1.0).abs() < 1e-6,
            "attenuation at d=0 should be 1.0, got {atten}"
        );
    }

    #[test]
    fn test_light_beyond_radius_has_zero_contribution() {
        let atten_at = attenuation(10.0, 10.0);
        let atten_beyond = attenuation(15.0, 10.0);
        assert_eq!(atten_at, 0.0, "attenuation at exactly radius should be 0.0");
        assert_eq!(atten_beyond, 0.0, "attenuation beyond radius should be 0.0");
    }

    #[test]
    fn test_attenuation_follows_inverse_square() {
        let radius = 100.0;
        let a1 = attenuation(5.0, radius);
        let a2 = attenuation(10.0, radius);
        let ratio = a1 / a2;
        assert!(
            ratio > 3.0 && ratio < 5.0,
            "doubling distance should roughly quarter intensity, got ratio {ratio}"
        );
    }

    #[test]
    fn test_light_culling_reduces_active_count() {
        let mut manager = PointLightManager::new();
        for i in 0..5 {
            manager.add(PointLight {
                position: glam::Vec3::new(i as f32, 0.0, 0.0),
                color: glam::Vec3::ONE,
                intensity: 1.0,
                radius: 50.0,
            });
        }
        for i in 0..5 {
            manager.add(PointLight {
                position: glam::Vec3::new(10_000.0 + i as f32, 0.0, 0.0),
                color: glam::Vec3::ONE,
                intensity: 1.0,
                radius: 5.0,
            });
        }
        let visible = manager.cull_to_list(glam::Vec3::ZERO, &Frustum::infinite());
        assert_eq!(visible.len(), 5, "far lights should be culled");
    }

    #[test]
    fn test_max_256_lights_enforced() {
        let mut manager = PointLightManager::new();
        for i in 0..400 {
            manager.add(PointLight {
                position: glam::Vec3::new(i as f32 * 0.1, 0.0, 0.0),
                color: glam::Vec3::ONE,
                intensity: 1.0,
                radius: 1000.0,
            });
        }
        let visible = manager.cull_to_list(glam::Vec3::ZERO, &Frustum::infinite());
        assert!(
            visible.len() <= PointLightManager::MAX_VISIBLE_LIGHTS,
            "visible lights must not exceed {}, got {}",
            PointLightManager::MAX_VISIBLE_LIGHTS,
            visible.len()
        );
    }

    #[test]
    fn test_gpu_struct_size() {
        assert_eq!(std::mem::size_of::<PointLightGpu>(), 48);
        assert_eq!(std::mem::size_of::<PointLightHeader>(), 16);
    }

    #[test]
    fn test_frustum_sphere_culling() {
        // A plane facing +X at x=10: normal=(1,0,0), d=-10
        let planes = [
            glam::Vec4::new(1.0, 0.0, 0.0, -10.0),
            glam::Vec4::ZERO,
            glam::Vec4::ZERO,
            glam::Vec4::ZERO,
            glam::Vec4::ZERO,
            glam::Vec4::ZERO,
        ];
        let frustum = Frustum::from_planes(planes);

        // Sphere at x=5, radius=3 → signed_dist = 5-10 = -5, -5 < -3 → culled
        assert!(!frustum.is_sphere_visible(glam::Vec3::new(5.0, 0.0, 0.0), 3.0));

        // Sphere at x=8, radius=3 → signed_dist = 8-10 = -2, -2 > -3 → visible
        assert!(frustum.is_sphere_visible(glam::Vec3::new(8.0, 0.0, 0.0), 3.0));
    }

    #[test]
    fn test_manager_add_clear() {
        let mut manager = PointLightManager::new();
        assert!(manager.is_empty());
        manager.add(PointLight {
            position: glam::Vec3::ZERO,
            color: glam::Vec3::ONE,
            intensity: 1.0,
            radius: 10.0,
        });
        assert_eq!(manager.len(), 1);
        manager.clear();
        assert!(manager.is_empty());
    }
}
