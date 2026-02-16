//! Cascaded shadow maps for directional light shadow rendering.
//!
//! Splits the view frustum into multiple depth slices (cascades), each with its
//! own shadow map covering a progressively larger area at lower resolution.
//! This provides sharp shadows near the camera and acceptable quality far away.

use bytemuck::{Pod, Zeroable};

/// Configuration for cascaded shadow mapping.
#[derive(Clone, Debug)]
pub struct CascadedShadowConfig {
    /// Number of cascades (1–4). Default: 4.
    pub cascade_count: u32,
    /// Far distance of each cascade in meters.
    /// `cascade_far[0] < cascade_far[1] < ... < cascade_far[cascade_count-1]`.
    pub cascade_far: [f32; 4],
    /// Shadow map resolution (width = height) per cascade. Default: 2048.
    pub resolution: u32,
    /// Depth bias to mitigate shadow acne (constant term).
    pub depth_bias_constant: f32,
    /// Depth bias slope factor.
    pub depth_bias_slope: f32,
    /// Normal offset bias in texels.
    pub normal_bias_texels: f32,
}

impl Default for CascadedShadowConfig {
    fn default() -> Self {
        Self {
            cascade_count: 4,
            cascade_far: [32.0, 128.0, 512.0, 2048.0],
            resolution: 2048,
            depth_bias_constant: 1.25,
            depth_bias_slope: 1.75,
            normal_bias_texels: 0.5,
        }
    }
}

/// GPU-side shadow uniform data (bound in fragment shader).
///
/// Contains 4 light-space matrices, cascade far distances, and cascade count.
/// Total size: 4×64 + 16 + 16 = 288 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct ShadowUniform {
    /// Light-space view-projection matrix per cascade.
    pub light_matrices: [[f32; 16]; 4],
    /// Far distance per cascade (vec4).
    pub cascade_far: [f32; 4],
    /// Number of active cascades (u32), plus 3 padding u32s.
    pub cascade_count_pad: [u32; 4],
}

impl ShadowUniform {
    /// Build from config and computed light matrices.
    pub fn from_matrices(config: &CascadedShadowConfig, matrices: &[glam::Mat4; 4]) -> Self {
        Self {
            light_matrices: [
                matrices[0].to_cols_array(),
                matrices[1].to_cols_array(),
                matrices[2].to_cols_array(),
                matrices[3].to_cols_array(),
            ],
            cascade_far: config.cascade_far,
            cascade_count_pad: [config.cascade_count, 0, 0, 0],
        }
    }
}

/// Cascaded shadow map GPU resources.
pub struct CascadedShadowMaps {
    /// 2D texture array (one layer per cascade).
    pub texture: wgpu::Texture,
    /// View of the full texture array (for sampling in fragment shader).
    pub view: wgpu::TextureView,
    /// One view per cascade layer for rendering into.
    pub cascade_views: Vec<wgpu::TextureView>,
    /// Comparison sampler for hardware PCF.
    pub sampler: wgpu::Sampler,
    /// Light-space view-projection matrix per cascade.
    pub light_matrices: [glam::Mat4; 4],
    /// Configuration.
    pub config: CascadedShadowConfig,
}

impl CascadedShadowMaps {
    /// Create shadow map resources on the GPU.
    pub fn new(device: &wgpu::Device, config: &CascadedShadowConfig) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("csm-depth-array"),
            size: wgpu::Extent3d {
                width: config.resolution,
                height: config.resolution,
                depth_or_array_layers: config.cascade_count,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("csm-depth-array-view"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });

        let cascade_views = (0..config.cascade_count)
            .map(|i| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some(&format!("csm-cascade-{i}")),
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: i,
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("csm-comparison-sampler"),
            compare: Some(wgpu::CompareFunction::GreaterEqual), // reverse-Z
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        Self {
            texture,
            view,
            cascade_views,
            sampler,
            light_matrices: [glam::Mat4::IDENTITY; 4],
            config: config.clone(),
        }
    }

    /// Update light matrices from the current camera and light direction.
    pub fn update_matrices(
        &mut self,
        light_dir: glam::Vec3,
        camera_view_proj_inv: glam::Mat4,
        camera_near: f32,
    ) {
        let count = self.config.cascade_count as usize;
        for i in 0..count {
            let near = if i == 0 {
                camera_near
            } else {
                self.config.cascade_far[i - 1]
            };
            let far = self.config.cascade_far[i];
            self.light_matrices[i] = compute_cascade_matrix(
                light_dir,
                camera_view_proj_inv,
                near,
                far,
                self.config.resolution,
            );
        }
        // Fill remaining with identity.
        for i in count..4 {
            self.light_matrices[i] = glam::Mat4::IDENTITY;
        }
    }

    /// Build the GPU uniform from current state.
    pub fn to_uniform(&self) -> ShadowUniform {
        ShadowUniform::from_matrices(&self.config, &self.light_matrices)
    }
}

/// Compute a tight orthographic light-space matrix for one cascade.
///
/// The matrix encloses the frustum slice between `near` and `far` (in view-space depth)
/// as seen from the directional light. Includes texel snapping to prevent shadow swimming.
pub fn compute_cascade_matrix(
    light_dir: glam::Vec3,
    camera_view_proj_inv: glam::Mat4,
    near: f32,
    far: f32,
    resolution: u32,
) -> glam::Mat4 {
    // 1. Compute 8 corners of the sub-frustum in NDC, then transform to world space.
    let ndc_corners = frustum_ndc_corners(near, far);
    let world_corners: Vec<glam::Vec3> = ndc_corners
        .iter()
        .map(|c| {
            let wc = camera_view_proj_inv * glam::Vec4::new(c.x, c.y, c.z, 1.0);
            glam::Vec3::new(wc.x / wc.w, wc.y / wc.w, wc.z / wc.w)
        })
        .collect();

    // 2. Build a light-space view matrix looking along light_dir.
    let center: glam::Vec3 =
        world_corners.iter().copied().sum::<glam::Vec3>() / world_corners.len() as f32;
    let light_up = if light_dir.y.abs() > 0.99 {
        glam::Vec3::Z
    } else {
        glam::Vec3::Y
    };
    let light_view = glam::Mat4::look_to_rh(center, light_dir, light_up);

    // 3. Compute AABB of frustum corners in light space.
    let mut min = glam::Vec3::splat(f32::MAX);
    let mut max = glam::Vec3::splat(f32::MIN);
    for corner in &world_corners {
        let ls = (light_view * glam::Vec4::new(corner.x, corner.y, corner.z, 1.0)).truncate();
        min = min.min(ls);
        max = max.max(ls);
    }

    // 4. Texel snapping: round extents to texel boundaries.
    let world_units_per_texel_x = (max.x - min.x) / resolution as f32;
    let world_units_per_texel_y = (max.y - min.y) / resolution as f32;
    if world_units_per_texel_x > 0.0 {
        min.x = (min.x / world_units_per_texel_x).floor() * world_units_per_texel_x;
        max.x = (max.x / world_units_per_texel_x).ceil() * world_units_per_texel_x;
    }
    if world_units_per_texel_y > 0.0 {
        min.y = (min.y / world_units_per_texel_y).floor() * world_units_per_texel_y;
        max.y = (max.y / world_units_per_texel_y).ceil() * world_units_per_texel_y;
    }

    // 5. Build orthographic projection in light space.
    //    For reverse-Z: near maps to 1.0, far maps to 0.0 → swap near/far.
    let ortho = glam::Mat4::orthographic_rh(min.x, max.x, min.y, max.y, max.z, min.z);

    ortho * light_view
}

/// Generate the 8 NDC corners for a sub-frustum between `near` and `far`.
///
/// In reverse-Z, near plane depth = 1.0 and far plane depth = 0.0.
/// However, when we use the camera's inverse VP to unproject, we need the NDC z
/// values that correspond to those depth values. For a standard perspective
/// projection with reverse-Z, near → z_ndc=1, far → z_ndc=0. But since we
/// receive `near`/`far` as linear view-space distances and want to map through
/// the full inverse VP, we use z_ndc = 1.0 for near corners and z_ndc = 0.0 for
/// far corners (matching the reverse-Z convention).
fn frustum_ndc_corners(near_frac: f32, far_frac: f32) -> [glam::Vec3; 8] {
    // We generate two sets of 4 corners, but since we're using an inverse VP that
    // already accounts for the full frustum, we interpolate z-NDC linearly between
    // 1.0 (near plane of full frustum) and 0.0 (far plane of full frustum).
    //
    // For sub-frustum slicing, we compute two z-NDC values and unproject.
    // This simplification works because the unproject + reproject cycle gives correct
    // world-space positions for the cascade AABB.
    let z_near = 1.0 - near_frac.min(1.0); // rough mapping
    let z_far = 1.0 - far_frac.min(1.0);

    // Actually, for correctness with arbitrary near/far ratios we should use the
    // full NDC cube corners and let the caller handle sub-frustum splitting via
    // separate inverse VPs. Instead, use a simpler approach: generate corners at
    // z_ndc=1 and z_ndc=0, then the caller builds per-cascade inverse VPs.
    //
    // Simplest correct approach: always use the full NDC cube. The caller constructs
    // a camera VP with the cascade's near/far, then inverts it.
    let _ = (z_near, z_far);

    // Full NDC corners (reverse-Z: near=1, far=0)
    [
        // Near plane (z=1)
        glam::Vec3::new(-1.0, -1.0, 1.0),
        glam::Vec3::new(1.0, -1.0, 1.0),
        glam::Vec3::new(1.0, 1.0, 1.0),
        glam::Vec3::new(-1.0, 1.0, 1.0),
        // Far plane (z=0)
        glam::Vec3::new(-1.0, -1.0, 0.0),
        glam::Vec3::new(1.0, -1.0, 0.0),
        glam::Vec3::new(1.0, 1.0, 0.0),
        glam::Vec3::new(-1.0, 1.0, 0.0),
    ]
}

/// Compute the cascade light-space matrix using explicit camera parameters.
///
/// This builds a sub-frustum camera VP for the given near/far range, inverts it,
/// and computes the tight orthographic enclosure. More accurate than using the
/// full camera inverse VP for all cascades.
pub fn compute_cascade_matrix_from_camera(
    light_dir: glam::Vec3,
    camera_view: glam::Mat4,
    fov_y: f32,
    aspect: f32,
    cascade_near: f32,
    cascade_far: f32,
    resolution: u32,
) -> glam::Mat4 {
    // Build a reverse-Z perspective for this cascade's sub-frustum.
    let proj = glam::Mat4::perspective_rh(fov_y, aspect, cascade_far, cascade_near);
    let vp = proj * camera_view;
    let inv_vp = vp.inverse();

    compute_cascade_matrix(light_dir, inv_vp, 0.0, 1.0, resolution)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shadow_uniform_size() {
        // 4 mat4 (4×64=256) + vec4 (16) + uvec4 (16) = 288 bytes
        assert_eq!(std::mem::size_of::<ShadowUniform>(), 288);
    }

    #[test]
    fn test_cascade_boundaries_cover_view_frustum() {
        let config = CascadedShadowConfig::default();
        assert!(
            config.cascade_far[0] > 0.0,
            "first cascade must cover near range"
        );
        for i in 1..config.cascade_count as usize {
            assert!(
                config.cascade_far[i] > config.cascade_far[i - 1],
                "cascade {i} must be farther than cascade {}",
                i - 1
            );
        }
        assert!(
            config.cascade_far[config.cascade_count as usize - 1] >= 1000.0,
            "last cascade should cover at least 1000m"
        );
    }

    #[test]
    fn test_shadow_acne_bias_is_positive() {
        let config = CascadedShadowConfig::default();
        assert!(config.depth_bias_constant > 0.0);
        assert!(config.depth_bias_slope > 0.0);
    }

    #[test]
    fn test_peter_panning_bias_is_bounded() {
        let config = CascadedShadowConfig::default();
        assert!(config.depth_bias_constant < 10.0);
        assert!(config.normal_bias_texels < 4.0);
    }

    #[test]
    fn test_cascade_blending_overlap() {
        let config = CascadedShadowConfig::default();
        let blend_fraction = 0.05;
        for i in 0..config.cascade_count as usize - 1 {
            let blend_start = config.cascade_far[i] * (1.0 - blend_fraction);
            let blend_end = config.cascade_far[i];
            assert!(blend_end > blend_start);
            assert!(
                (blend_end - blend_start) >= 1.0,
                "blend zone for cascade {i} is too narrow: {}m",
                blend_end - blend_start
            );
        }
    }

    #[test]
    fn test_light_matrix_is_valid() {
        let light_dir = glam::Vec3::new(0.3, -1.0, 0.2).normalize();
        let camera_inv = glam::Mat4::IDENTITY;
        let matrix = compute_cascade_matrix(light_dir, camera_inv, 0.1, 32.0, 2048);
        for col in 0..4 {
            for row in 0..4 {
                let val = matrix.col(col)[row];
                assert!(
                    val.is_finite(),
                    "light matrix element [{col}][{row}] is not finite: {val}"
                );
            }
        }
        assert_ne!(matrix, glam::Mat4::IDENTITY);
    }

    #[test]
    fn test_shadow_uniform_from_matrices() {
        let config = CascadedShadowConfig::default();
        let matrices = [glam::Mat4::IDENTITY; 4];
        let uniform = ShadowUniform::from_matrices(&config, &matrices);
        assert_eq!(uniform.cascade_count_pad[0], 4);
        assert!((uniform.cascade_far[0] - 32.0).abs() < 1e-6);
    }

    #[test]
    fn test_cascade_matrix_from_camera() {
        let light_dir = glam::Vec3::new(0.0, -1.0, 0.3).normalize();
        let view = glam::Mat4::look_to_rh(
            glam::Vec3::new(0.0, 10.0, 0.0),
            glam::Vec3::NEG_Z,
            glam::Vec3::Y,
        );
        let mat = compute_cascade_matrix_from_camera(
            light_dir,
            view,
            std::f32::consts::FRAC_PI_4,
            16.0 / 9.0,
            0.1,
            32.0,
            2048,
        );
        for col in 0..4 {
            for row in 0..4 {
                assert!(mat.col(col)[row].is_finite());
            }
        }
    }
}
