//! GPU texture management: creation, caching, mipmap generation, and bind groups.
//!
//! Provides [`TextureManager`] which handles the full lifecycle of GPU textures.
//! Downstream systems call `create_texture()` once and receive an
//! [`Arc<ManagedTexture>`] with a ready-to-bind [`wgpu::BindGroup`].

use std::collections::HashMap;
use std::sync::Arc;

/// A GPU texture with its view, bind group, and metadata.
pub struct ManagedTexture {
    /// The underlying GPU texture.
    pub texture: wgpu::Texture,
    /// Default view into the texture.
    pub view: wgpu::TextureView,
    /// Pre-built bind group for immediate use in draw calls.
    pub bind_group: wgpu::BindGroup,
    /// Width and height in texels.
    pub dimensions: (u32, u32),
    /// Pixel format.
    pub format: wgpu::TextureFormat,
    /// Number of mip levels (1 if mipmaps were not generated).
    pub mip_level_count: u32,
}

/// Per-layer data for texture array creation.
pub struct TextureLayerData<'a> {
    /// Raw pixel bytes for this layer.
    pub data: &'a [u8],
    /// Debug label for this layer.
    pub label: &'a str,
}

/// Errors that can occur during texture creation.
#[derive(Debug, thiserror::Error)]
pub enum TextureError {
    /// Pixel data length doesn't match the expected size for the given dimensions and format.
    #[error(
        "texture data size ({actual}) does not match expected ({expected}) for {width}x{height} {format:?}"
    )]
    DataSizeMismatch {
        actual: usize,
        expected: usize,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    },

    /// Width or height is zero.
    #[error("texture dimensions must be non-zero, got {width}x{height}")]
    ZeroDimensions { width: u32, height: u32 },

    /// Texture array layers have inconsistent data sizes.
    #[error("texture array layers have inconsistent dimensions")]
    InconsistentLayerDimensions,
}

/// Calculates the number of mip levels for the given dimensions.
pub fn mip_level_count(width: u32, height: u32) -> u32 {
    (width.max(height) as f32).log2().floor() as u32 + 1
}

/// Centralized GPU texture manager with caching, mipmap generation, and bind groups.
pub struct TextureManager {
    textures: HashMap<String, Arc<ManagedTexture>>,
    sampler_linear: wgpu::Sampler,
    sampler_nearest: wgpu::Sampler,
    bind_group_layout: wgpu::BindGroupLayout,
    blit_shader: wgpu::ShaderModule,
    blit_pipeline_layout: wgpu::PipelineLayout,
    blit_bind_group_layout: wgpu::BindGroupLayout,
    blit_sampler: wgpu::Sampler,
}

/// WGSL shader for mipmap generation via fullscreen blit.
const BLIT_SHADER_SOURCE: &str = r#"
@group(0) @binding(0) var src_texture: texture_2d<f32>;
@group(0) @binding(1) var src_sampler: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> VertexOutput {
    // Full-screen triangle
    let uv = vec2<f32>(f32((idx << 1u) & 2u), f32(idx & 2u));
    var out: VertexOutput;
    out.position = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(uv.x, 1.0 - uv.y);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(src_texture, src_sampler, in.uv);
}
"#;

impl TextureManager {
    /// Create a new texture manager with shared samplers and bind group layout.
    pub fn new(device: &wgpu::Device) -> Self {
        let sampler_linear = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sampler-linear"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        let sampler_nearest = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sampler-nearest"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("texture-bind-group-layout"),
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

        let blit_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("blit-bind-group-layout"),
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

        let blit_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("blit-shader"),
            source: wgpu::ShaderSource::Wgsl(BLIT_SHADER_SOURCE.into()),
        });

        let blit_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("blit-pipeline-layout"),
            bind_group_layouts: &[&blit_bind_group_layout],
            immediate_size: 0,
        });

        let blit_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("blit-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        Self {
            textures: HashMap::new(),
            sampler_linear,
            sampler_nearest,
            bind_group_layout,
            blit_shader,
            blit_pipeline_layout,
            blit_bind_group_layout,
            blit_sampler,
        }
    }

    /// Create a 2D texture from raw pixel data.
    #[allow(clippy::too_many_arguments)]
    pub fn create_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        name: &str,
        data: &[u8],
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        generate_mipmaps: bool,
    ) -> Result<Arc<ManagedTexture>, TextureError> {
        // Check cache first
        if let Some(existing) = self.textures.get(name) {
            return Ok(Arc::clone(existing));
        }

        validate_dimensions(width, height)?;
        validate_data_size(data, width, height, format)?;

        let mip_levels = if generate_mipmaps {
            mip_level_count(width, height)
        } else {
            1
        };

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(name),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: mip_levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row(width, format)),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        if generate_mipmaps && mip_levels > 1 {
            self.generate_mipmaps(device, queue, &texture, format, mip_levels);
        }

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(&format!("{name}-bind-group")),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler_linear),
                },
            ],
        });

        let managed = Arc::new(ManagedTexture {
            texture,
            view,
            bind_group,
            dimensions: (width, height),
            format,
            mip_level_count: mip_levels,
        });

        self.textures.insert(name.to_string(), Arc::clone(&managed));
        log::info!("Created texture '{name}' ({width}x{height}, {mip_levels} mips)");
        Ok(managed)
    }

    /// Create a 2D texture array (for voxel block faces, terrain layers, etc.).
    #[allow(clippy::too_many_arguments)]
    pub fn create_texture_array(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        name: &str,
        layers: &[TextureLayerData],
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        generate_mipmaps: bool,
    ) -> Result<Arc<ManagedTexture>, TextureError> {
        if let Some(existing) = self.textures.get(name) {
            return Ok(Arc::clone(existing));
        }

        validate_dimensions(width, height)?;

        let expected_size = expected_byte_size(width, height, format);
        for layer in layers {
            if layer.data.len() != expected_size {
                return Err(TextureError::InconsistentLayerDimensions);
            }
        }

        let layer_count = layers.len() as u32;
        let mip_levels = if generate_mipmaps {
            mip_level_count(width, height)
        } else {
            1
        };

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(name),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: layer_count,
            },
            mip_level_count: mip_levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });

        let bpr = bytes_per_row(width, format);
        for (i, layer) in layers.iter().enumerate() {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: i as u32,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                layer.data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bpr),
                    rows_per_image: Some(height),
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        }

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(&format!("{name}-bind-group")),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler_linear),
                },
            ],
        });

        let managed = Arc::new(ManagedTexture {
            texture,
            view,
            bind_group,
            dimensions: (width, height),
            format,
            mip_level_count: mip_levels,
        });

        self.textures.insert(name.to_string(), Arc::clone(&managed));
        log::info!(
            "Created texture array '{name}' ({width}x{height}, {layer_count} layers, {mip_levels} mips)"
        );
        Ok(managed)
    }

    /// Get a previously created texture by name.
    pub fn get(&self, name: &str) -> Option<Arc<ManagedTexture>> {
        self.textures.get(name).cloned()
    }

    /// Remove a texture from the cache. Returns `true` if it existed.
    pub fn remove(&mut self, name: &str) -> bool {
        self.textures.remove(name).is_some()
    }

    /// The shared bind group layout for texture + sampler pairs.
    pub fn bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.bind_group_layout
    }

    /// The nearest-neighbor sampler (pixel art / voxel faces).
    pub fn sampler_nearest(&self) -> &wgpu::Sampler {
        &self.sampler_nearest
    }

    /// The linear filtering sampler.
    pub fn sampler_linear(&self) -> &wgpu::Sampler {
        &self.sampler_linear
    }

    /// Generate mipmaps for a texture using render passes.
    fn generate_mipmaps(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
        format: wgpu::TextureFormat,
        mip_count: u32,
    ) {
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("mipmap-pipeline"),
            layout: Some(&self.blit_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &self.blit_shader,
                entry_point: Some("vs_main"),
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
                module: &self.blit_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview_mask: None,
            cache: None,
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("mipmap-encoder"),
        });

        for level in 1..mip_count {
            let src_view = texture.create_view(&wgpu::TextureViewDescriptor {
                base_mip_level: level - 1,
                mip_level_count: Some(1),
                ..Default::default()
            });

            let dst_view = texture.create_view(&wgpu::TextureViewDescriptor {
                base_mip_level: level,
                mip_level_count: Some(1),
                ..Default::default()
            });

            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("mipmap-bind-group"),
                layout: &self.blit_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&src_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.blit_sampler),
                    },
                ],
            });

            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("mipmap-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &dst_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });

            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        queue.submit(std::iter::once(encoder.finish()));
    }
}

/// Calculate the expected byte size for a texture.
fn expected_byte_size(width: u32, height: u32, format: wgpu::TextureFormat) -> usize {
    let bpp = format.block_copy_size(None).unwrap_or(4) as usize;
    width as usize * height as usize * bpp
}

/// Calculate bytes per row for a texture.
fn bytes_per_row(width: u32, format: wgpu::TextureFormat) -> u32 {
    let bpp = format.block_copy_size(None).unwrap_or(4);
    width * bpp
}

/// Validate that dimensions are non-zero.
fn validate_dimensions(width: u32, height: u32) -> Result<(), TextureError> {
    if width == 0 || height == 0 {
        return Err(TextureError::ZeroDimensions { width, height });
    }
    Ok(())
}

/// Validate that data size matches expected size.
fn validate_data_size(
    data: &[u8],
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
) -> Result<(), TextureError> {
    let expected = expected_byte_size(width, height, format);
    if data.len() != expected {
        return Err(TextureError::DataSizeMismatch {
            actual: data.len(),
            expected,
            width,
            height,
            format,
        });
    }
    Ok(())
}

/// Create a test GPU device and queue. Returns `None` if no GPU is available.
#[cfg(test)]
pub(crate) fn create_test_device_queue() -> Option<(wgpu::Device, wgpu::Queue)> {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .ok()?;

        adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: None,
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::default(),
                experimental_features: Default::default(),
                ..Default::default()
            })
            .await
            .ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_texture_with_valid_dimensions() {
        let Some((device, queue)) = create_test_device_queue() else {
            return;
        };
        let mut manager = TextureManager::new(&device);

        let data = vec![255u8; 64]; // 4x4 RGBA8
        let result = manager.create_texture(
            &device,
            &queue,
            "test-4x4",
            &data,
            4,
            4,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            false,
        );
        assert!(result.is_ok());
        let tex = result.unwrap();
        assert_eq!(tex.dimensions, (4, 4));
    }

    #[test]
    fn test_mipmap_level_count_calculation() {
        assert_eq!(mip_level_count(1, 1), 1);
        assert_eq!(mip_level_count(2, 2), 2);
        assert_eq!(mip_level_count(4, 4), 3);
        assert_eq!(mip_level_count(256, 256), 9);
        assert_eq!(mip_level_count(512, 256), 10);
        assert_eq!(mip_level_count(1024, 1024), 11);
    }

    #[test]
    fn test_bind_group_creation_succeeds() {
        let Some((device, queue)) = create_test_device_queue() else {
            return;
        };
        let mut manager = TextureManager::new(&device);

        let data = vec![128u8; 16]; // 2x2 RGBA8
        let tex = manager
            .create_texture(
                &device,
                &queue,
                "test-bind",
                &data,
                2,
                2,
                wgpu::TextureFormat::Rgba8UnormSrgb,
                false,
            )
            .unwrap();

        let _bg = &tex.bind_group;
    }

    #[test]
    fn test_texture_cache_deduplicates() {
        let Some((device, queue)) = create_test_device_queue() else {
            return;
        };
        let mut manager = TextureManager::new(&device);

        let data = vec![255u8; 16]; // 2x2 RGBA8
        let tex1 = manager
            .create_texture(
                &device,
                &queue,
                "shared",
                &data,
                2,
                2,
                wgpu::TextureFormat::Rgba8UnormSrgb,
                false,
            )
            .unwrap();

        let tex2 = manager
            .create_texture(
                &device,
                &queue,
                "shared",
                &data,
                2,
                2,
                wgpu::TextureFormat::Rgba8UnormSrgb,
                false,
            )
            .unwrap();

        assert!(Arc::ptr_eq(&tex1, &tex2));
    }

    #[test]
    fn test_rgba8_format_handling() {
        let Some((device, queue)) = create_test_device_queue() else {
            return;
        };
        let mut manager = TextureManager::new(&device);

        let data = vec![0u8; 256]; // 8x8 RGBA8
        let tex = manager
            .create_texture(
                &device,
                &queue,
                "rgba8-test",
                &data,
                8,
                8,
                wgpu::TextureFormat::Rgba8UnormSrgb,
                false,
            )
            .unwrap();

        assert_eq!(tex.format, wgpu::TextureFormat::Rgba8UnormSrgb);
    }

    #[test]
    fn test_zero_dimensions_returns_error() {
        let Some((device, queue)) = create_test_device_queue() else {
            return;
        };
        let mut manager = TextureManager::new(&device);

        let result = manager.create_texture(
            &device,
            &queue,
            "zero",
            &[],
            0,
            0,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            false,
        );
        assert!(matches!(result, Err(TextureError::ZeroDimensions { .. })));
    }

    #[test]
    fn test_data_size_mismatch_returns_error() {
        let Some((device, queue)) = create_test_device_queue() else {
            return;
        };
        let mut manager = TextureManager::new(&device);

        let data = vec![0u8; 32]; // 4x4 expects 64
        let result = manager.create_texture(
            &device,
            &queue,
            "mismatch",
            &data,
            4,
            4,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            false,
        );
        assert!(matches!(result, Err(TextureError::DataSizeMismatch { .. })));
    }

    #[test]
    fn test_mipmap_generation_sets_correct_mip_count() {
        let Some((device, queue)) = create_test_device_queue() else {
            return;
        };
        let mut manager = TextureManager::new(&device);

        let data = vec![255u8; 256 * 256 * 4];
        let tex = manager
            .create_texture(
                &device,
                &queue,
                "mipmapped",
                &data,
                256,
                256,
                wgpu::TextureFormat::Rgba8UnormSrgb,
                true,
            )
            .unwrap();

        assert_eq!(tex.mip_level_count, 9);
    }

    #[test]
    fn test_remove_texture_from_cache() {
        let Some((device, queue)) = create_test_device_queue() else {
            return;
        };
        let mut manager = TextureManager::new(&device);

        let data = vec![0u8; 16];
        manager
            .create_texture(
                &device,
                &queue,
                "removable",
                &data,
                2,
                2,
                wgpu::TextureFormat::Rgba8UnormSrgb,
                false,
            )
            .unwrap();

        assert!(manager.get("removable").is_some());
        assert!(manager.remove("removable"));
        assert!(manager.get("removable").is_none());
    }
}
