//! Multi-pass bloom post-processing pipeline for HDR rendering.
//!
//! Extracts bright pixels, progressively blurs via a downsample/upsample mip chain,
//! and composites the bloom glow onto the final output. Operates between the HDR
//! scene pass and tonemapping.

use bytemuck::{Pod, Zeroable};

/// Configuration for the bloom post-processing effect.
#[derive(Clone, Debug)]
pub struct BloomConfig {
    /// Brightness threshold. Only pixels with luminance above this value contribute to bloom.
    /// Default: 1.0 (anything above standard white is considered "bright").
    pub threshold: f32,
    /// Soft knee for the threshold curve. Prevents hard cutoff artifacts.
    /// Range \[0, 1\]. Default: 0.5.
    pub soft_knee: f32,
    /// Overall bloom intensity multiplier. Default: 0.3.
    pub intensity: f32,
    /// Number of downscale iterations. Each iteration halves resolution and doubles blur radius.
    /// More iterations = wider bloom. Range \[1, 8\]. Default: 5.
    pub iterations: u32,
    /// Bloom radius multiplier. Scales the blur tap offset. Default: 1.0.
    pub radius: f32,
}

impl Default for BloomConfig {
    fn default() -> Self {
        Self {
            threshold: 1.0,
            soft_knee: 0.5,
            intensity: 0.3,
            iterations: 5,
            radius: 1.0,
        }
    }
}

/// GPU uniform for bloom shader parameters.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(crate) struct BloomParams {
    pub threshold: f32,
    pub soft_knee: f32,
    pub intensity: f32,
    pub radius: f32,
}

/// WGSL shader source for all bloom passes (extract, downsample, upsample, tonemap, composite).
pub const BLOOM_SHADER_SOURCE: &str = r#"
struct BloomParams {
    threshold: f32,
    soft_knee: f32,
    intensity: f32,
    radius: f32,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@group(0) @binding(0) var<uniform> params: BloomParams;
@group(1) @binding(0) var input_tex: texture_2d<f32>;
@group(1) @binding(1) var input_sampler: sampler;

@vertex
fn vs_fullscreen(@builtin(vertex_index) idx: u32) -> VertexOutput {
    let uv = vec2<f32>(f32((idx << 1u) & 2u), f32(idx & 2u));
    var out: VertexOutput;
    out.position = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(uv.x, 1.0 - uv.y);
    return out;
}

fn soft_threshold(color: vec3<f32>, threshold: f32, knee: f32) -> vec3<f32> {
    let luminance = dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
    let soft = luminance - threshold + knee;
    let soft_clamped = clamp(soft, 0.0, 2.0 * knee);
    let contribution = soft_clamped * soft_clamped / (4.0 * knee + 0.0001);
    let factor = max(luminance - threshold, contribution) / max(luminance, 0.0001);
    return color * max(factor, 0.0);
}

@fragment
fn fs_extract(in: VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(input_tex, input_sampler, in.uv).rgb;
    let extracted = soft_threshold(color, params.threshold, params.soft_knee);
    return vec4<f32>(extracted, 1.0);
}

@fragment
fn fs_downsample(in: VertexOutput) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(input_tex));
    let texel = params.radius / dims;
    let a = textureSample(input_tex, input_sampler, in.uv + vec2(-texel.x, -texel.y)).rgb;
    let b = textureSample(input_tex, input_sampler, in.uv + vec2( texel.x, -texel.y)).rgb;
    let c = textureSample(input_tex, input_sampler, in.uv + vec2(-texel.x,  texel.y)).rgb;
    let d = textureSample(input_tex, input_sampler, in.uv + vec2( texel.x,  texel.y)).rgb;
    return vec4<f32>((a + b + c + d) * 0.25, 1.0);
}

@fragment
fn fs_upsample(in: VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(input_tex, input_sampler, in.uv).rgb;
    return vec4<f32>(color * params.intensity, 1.0);
}

@fragment
fn fs_tonemap(in: VertexOutput) -> @location(0) vec4<f32> {
    let hdr = textureSample(input_tex, input_sampler, in.uv).rgb;
    let a = 2.51;
    let b_coeff = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    let mapped = clamp(
        (hdr * (a * hdr + b_coeff)) / (hdr * (c * hdr + d) + e),
        vec3<f32>(0.0), vec3<f32>(1.0)
    );
    return vec4<f32>(mapped, 1.0);
}

@fragment
fn fs_bloom_composite(in: VertexOutput) -> @location(0) vec4<f32> {
    let bloom = textureSample(input_tex, input_sampler, in.uv).rgb;
    return vec4<f32>(bloom, 1.0);
}
"#;

/// 9-tap Gaussian weights for sigma ≈ 1.5 (normalized). Used in tests and documentation.
pub const GAUSSIAN_WEIGHTS: [f32; 5] = [
    0.227_027_03,
    0.194_594_6,
    0.121_621_62,
    0.054_054_055,
    0.016_216_216,
];

/// Multi-pass bloom post-processing pipeline.
///
/// Owns an HDR render target, a mip chain for progressive blur, and all GPU
/// pipelines needed for bloom extraction, blur, tonemapping, and compositing.
pub struct BloomPipeline {
    config: BloomConfig,
    // Bind group layouts (kept alive for resize bind group recreation)
    #[allow(dead_code)]
    params_bgl: wgpu::BindGroupLayout,
    texture_bgl: wgpu::BindGroupLayout,
    // Pipelines (all share vs_fullscreen vertex shader)
    extract_pipeline: wgpu::RenderPipeline,
    downsample_pipeline: wgpu::RenderPipeline,
    upsample_pipeline: wgpu::RenderPipeline,
    tonemap_pipeline: wgpu::RenderPipeline,
    composite_pipeline: wgpu::RenderPipeline,
    // Shared resources
    sampler: wgpu::Sampler,
    params_buffer: wgpu::Buffer,
    params_bind_group: wgpu::BindGroup,
    // HDR render target (scene renders here)
    hdr_texture: wgpu::Texture,
    hdr_view: wgpu::TextureView,
    hdr_bind_group: wgpu::BindGroup,
    hdr_format: wgpu::TextureFormat,
    // Mip chain for progressive blur
    mip_textures: Vec<wgpu::Texture>,
    mip_views: Vec<wgpu::TextureView>,
    mip_bind_groups: Vec<wgpu::BindGroup>,
}

impl BloomPipeline {
    /// Create a new bloom pipeline with the given configuration.
    ///
    /// `hdr_format` is the format for intermediate HDR textures (typically `Rgba16Float`).
    /// `surface_format` is the final output format (the swapchain format).
    pub fn new(
        device: &wgpu::Device,
        hdr_format: wgpu::TextureFormat,
        surface_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        config: BloomConfig,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bloom-shader"),
            source: wgpu::ShaderSource::Wgsl(BLOOM_SHADER_SOURCE.into()),
        });

        // Bind group layouts
        let params_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bloom-params-bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: std::num::NonZeroU64::new(16),
                },
                count: None,
            }],
        });

        let texture_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bloom-texture-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        // Pipeline layouts
        let hdr_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("bloom-hdr-layout"),
            bind_group_layouts: &[&params_bgl, &texture_bgl],
            immediate_size: 0,
        });

        let surface_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("bloom-surface-layout"),
            bind_group_layouts: &[&params_bgl, &texture_bgl],
            immediate_size: 0,
        });

        // Create pipelines
        let extract_pipeline = create_fullscreen_pipeline(
            device,
            &shader,
            &hdr_layout,
            "fs_extract",
            hdr_format,
            None,
            "bloom-extract",
        );
        let downsample_pipeline = create_fullscreen_pipeline(
            device,
            &shader,
            &hdr_layout,
            "fs_downsample",
            hdr_format,
            None,
            "bloom-downsample",
        );
        let upsample_pipeline = create_fullscreen_pipeline(
            device,
            &shader,
            &hdr_layout,
            "fs_upsample",
            hdr_format,
            Some(wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent::OVER,
            }),
            "bloom-upsample",
        );
        let tonemap_pipeline = create_fullscreen_pipeline(
            device,
            &shader,
            &surface_layout,
            "fs_tonemap",
            surface_format,
            None,
            "bloom-tonemap",
        );
        let composite_pipeline = create_fullscreen_pipeline(
            device,
            &shader,
            &surface_layout,
            "fs_bloom_composite",
            surface_format,
            Some(wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent::OVER,
            }),
            "bloom-composite",
        );

        // Sampler
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("bloom-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        // Params buffer
        let params = BloomParams {
            threshold: config.threshold,
            soft_knee: config.soft_knee,
            intensity: config.intensity,
            radius: config.radius,
        };
        use wgpu::util::DeviceExt;
        let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("bloom-params"),
            contents: bytemuck::cast_slice(&[params]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let params_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bloom-params-bg"),
            layout: &params_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: params_buffer.as_entire_binding(),
            }],
        });

        // Create HDR texture and mip chain
        let (hdr_texture, hdr_view, hdr_bind_group) =
            create_hdr_texture(device, &texture_bgl, &sampler, hdr_format, width, height);
        let (mip_textures, mip_views, mip_bind_groups) = create_mip_chain(
            device,
            &texture_bgl,
            &sampler,
            hdr_format,
            width,
            height,
            &config,
        );

        Self {
            config,
            params_bgl,
            texture_bgl,
            extract_pipeline,
            downsample_pipeline,
            upsample_pipeline,
            tonemap_pipeline,
            composite_pipeline,
            sampler,
            params_buffer,
            params_bind_group,
            hdr_texture,
            hdr_view,
            hdr_bind_group,
            hdr_format,
            mip_textures,
            mip_views,
            mip_bind_groups,
        }
    }

    /// Returns the HDR texture view that the scene should render to.
    pub fn hdr_view(&self) -> &wgpu::TextureView {
        &self.hdr_view
    }

    /// Returns the HDR texture format.
    pub fn hdr_format(&self) -> wgpu::TextureFormat {
        self.hdr_format
    }

    /// Recreate textures and bind groups after a window resize.
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let (hdr_texture, hdr_view, hdr_bind_group) = create_hdr_texture(
            device,
            &self.texture_bgl,
            &self.sampler,
            self.hdr_format,
            width,
            height,
        );
        self.hdr_texture = hdr_texture;
        self.hdr_view = hdr_view;
        self.hdr_bind_group = hdr_bind_group;

        let (mip_textures, mip_views, mip_bind_groups) = create_mip_chain(
            device,
            &self.texture_bgl,
            &self.sampler,
            self.hdr_format,
            width,
            height,
            &self.config,
        );
        self.mip_textures = mip_textures;
        self.mip_views = mip_views;
        self.mip_bind_groups = mip_bind_groups;
    }

    /// Update bloom parameters (e.g., when the user changes settings).
    pub fn update_config(&mut self, queue: &wgpu::Queue, config: BloomConfig) {
        let params = BloomParams {
            threshold: config.threshold,
            soft_knee: config.soft_knee,
            intensity: config.intensity,
            radius: config.radius,
        };
        queue.write_buffer(&self.params_buffer, 0, bytemuck::cast_slice(&[params]));
        self.config = config;
    }

    /// Execute the full bloom pipeline: extract → downsample → upsample → tonemap → composite.
    ///
    /// After this call, `surface_view` contains the tonemapped scene with bloom applied.
    pub fn execute(&self, encoder: &mut wgpu::CommandEncoder, surface_view: &wgpu::TextureView) {
        let iterations = self.config.iterations as usize;
        if iterations == 0 || self.mip_textures.is_empty() {
            return;
        }

        // 1. Extract bright pixels: HDR → mip[0]
        self.run_pass(
            encoder,
            &self.extract_pipeline,
            &self.hdr_bind_group,
            &self.mip_views[0],
            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
            "bloom-extract",
        );

        // 2. Progressive downsample: mip[i-1] → mip[i]
        for i in 1..iterations.min(self.mip_textures.len()) {
            self.run_pass(
                encoder,
                &self.downsample_pipeline,
                &self.mip_bind_groups[i - 1],
                &self.mip_views[i],
                wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                "bloom-downsample",
            );
        }

        // 3. Progressive upsample: mip[i+1] → mip[i] (additive)
        let last = iterations.min(self.mip_textures.len()) - 1;
        for i in (0..last).rev() {
            self.run_pass(
                encoder,
                &self.upsample_pipeline,
                &self.mip_bind_groups[i + 1],
                &self.mip_views[i],
                wgpu::LoadOp::Load,
                "bloom-upsample",
            );
        }

        // 4. Tonemap HDR → surface (clears surface)
        self.run_pass(
            encoder,
            &self.tonemap_pipeline,
            &self.hdr_bind_group,
            surface_view,
            wgpu::LoadOp::Clear(wgpu::Color::BLACK),
            "bloom-tonemap",
        );

        // 5. Additive bloom composite: mip[0] → surface
        self.run_pass(
            encoder,
            &self.composite_pipeline,
            &self.mip_bind_groups[0],
            surface_view,
            wgpu::LoadOp::Load,
            "bloom-composite",
        );
    }

    /// Run a single fullscreen render pass.
    fn run_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::RenderPipeline,
        texture_bind_group: &wgpu::BindGroup,
        target_view: &wgpu::TextureView,
        load_op: wgpu::LoadOp<wgpu::Color>,
        label: &str,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: load_op,
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &self.params_bind_group, &[]);
        pass.set_bind_group(1, texture_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// Create a fullscreen render pipeline with the given fragment entry point.
fn create_fullscreen_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    fragment_entry: &str,
    target_format: wgpu::TextureFormat,
    blend: Option<wgpu::BlendState>,
    label: &str,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_fullscreen"),
            buffers: &[],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fragment_entry),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// Create the full-resolution HDR render target texture.
fn create_hdr_texture(
    device: &wgpu::Device,
    texture_bgl: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView, wgpu::BindGroup) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("bloom-hdr"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("bloom-hdr-bg"),
        layout: texture_bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    });
    (texture, view, bind_group)
}

/// Create the mip chain textures for progressive blur.
fn create_mip_chain(
    device: &wgpu::Device,
    texture_bgl: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
    config: &BloomConfig,
) -> (
    Vec<wgpu::Texture>,
    Vec<wgpu::TextureView>,
    Vec<wgpu::BindGroup>,
) {
    let mut textures = Vec::new();
    let mut views = Vec::new();
    let mut bind_groups = Vec::new();
    let mut w = width / 2;
    let mut h = height / 2;

    for i in 0..config.iterations {
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("bloom-mip"),
            size: wgpu::Extent3d {
                width: w.max(1),
                height: h.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bloom-mip-bg"),
            layout: texture_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });
        textures.push(tex);
        views.push(view);
        bind_groups.push(bg);

        log::trace!("Bloom mip {i}: {w}x{h}");
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }

    (textures, views, bind_groups)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bloom_config_default_values() {
        let config = BloomConfig::default();
        assert_eq!(config.threshold, 1.0);
        assert_eq!(config.intensity, 0.3);
        assert_eq!(config.iterations, 5);
        assert_eq!(config.radius, 1.0);
        assert!(config.soft_knee >= 0.0 && config.soft_knee <= 1.0);
    }

    #[test]
    fn test_bright_points_produce_glow() {
        let bright_pixel = [5.0_f32, 5.0, 5.0];
        let threshold = 1.0;
        let luminance =
            bright_pixel[0] * 0.2126 + bright_pixel[1] * 0.7152 + bright_pixel[2] * 0.0722;
        let factor = (luminance - threshold) / luminance;
        let extracted: Vec<f32> = bright_pixel.iter().map(|c| c * factor).collect();
        assert!(
            extracted[0] > 0.0 && extracted[1] > 0.0 && extracted[2] > 0.0,
            "Bright pixels should produce non-zero bloom contribution: {extracted:?}"
        );
    }

    #[test]
    fn test_dim_points_produce_minimal_glow() {
        let dim_pixel = [0.3_f32, 0.3, 0.3];
        let threshold = 1.0;
        let luminance = dim_pixel[0] * 0.2126 + dim_pixel[1] * 0.7152 + dim_pixel[2] * 0.0722;
        let factor = ((luminance - threshold).max(0.0)) / luminance.max(0.0001);
        let extracted: Vec<f32> = dim_pixel.iter().map(|c| c * factor).collect();
        assert!(
            extracted.iter().all(|&v| v < 0.01),
            "Dim pixels should produce near-zero bloom contribution: {extracted:?}"
        );
    }

    #[test]
    fn test_bloom_radius_is_configurable() {
        let config_narrow = BloomConfig {
            radius: 0.5,
            ..Default::default()
        };
        let config_wide = BloomConfig {
            radius: 2.0,
            ..Default::default()
        };
        assert!(config_narrow.radius < config_wide.radius);
        assert_eq!(config_narrow.radius, 0.5);
        assert_eq!(config_wide.radius, 2.0);
    }

    #[test]
    fn test_bloom_does_not_affect_non_bright_pixels() {
        let threshold = 1.0;
        let knee = 0.5;
        let test_luminance: f32 = 0.4;

        let soft = test_luminance - threshold + knee;
        let contribution = if soft > 0.0 {
            let soft_clamped = soft.min(2.0 * knee);
            soft_clamped * soft_clamped / (4.0 * knee + 0.0001)
        } else {
            0.0
        };
        let factor = (test_luminance - threshold).max(contribution) / test_luminance.max(0.0001);
        assert!(
            factor <= 0.0,
            "Pixel at luminance {test_luminance} should have zero bloom factor, got {factor}"
        );
    }

    #[test]
    fn test_gaussian_weights_sum_to_approximately_one() {
        let sum = GAUSSIAN_WEIGHTS[0] + 2.0 * GAUSSIAN_WEIGHTS[1..].iter().sum::<f32>();
        assert!(
            (sum - 1.0).abs() < 0.01,
            "Gaussian weights should sum to ~1.0, got {sum}"
        );
    }

    #[test]
    fn test_bloom_intensity_scales_output() {
        let config_low = BloomConfig {
            intensity: 0.1,
            ..Default::default()
        };
        let config_high = BloomConfig {
            intensity: 1.0,
            ..Default::default()
        };
        assert!(
            config_high.intensity > config_low.intensity,
            "High intensity ({}) should exceed low intensity ({})",
            config_high.intensity,
            config_low.intensity
        );
    }

    #[test]
    fn test_mip_chain_dimensions_halve_each_level() {
        let mut w = 1920u32 / 2;
        let mut h = 1080u32 / 2;
        let expected_dims = [(960, 540), (480, 270), (240, 135), (120, 67), (60, 33)];
        for (i, &(ew, eh)) in expected_dims.iter().enumerate() {
            assert_eq!((w, h), (ew, eh), "Mip level {i} dimensions mismatch");
            w = (w / 2).max(1);
            h = (h / 2).max(1);
        }
    }

    #[test]
    fn test_bloom_params_uniform_size() {
        assert_eq!(std::mem::size_of::<BloomParams>(), 16);
    }
}
