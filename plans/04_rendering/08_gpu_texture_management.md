# GPU Texture Management

## Problem

Every material in the engine — terrain surfaces, voxel block faces, UI elements, particle sprites — needs GPU textures. Without centralized texture management, the engine will create duplicate textures for the same image, leak GPU memory when textures are no longer needed, forget to generate mipmaps (causing aliasing at distance), and scatter bind group creation code across every system. The voxel terrain alone may use hundreds of block face textures organized in texture arrays. A `TextureManager` must handle creation, caching, mipmap generation, and bind group setup so that downstream systems only deal with texture handles.

## Solution

### TextureManager

```rust
pub struct TextureManager {
    textures: HashMap<String, Arc<ManagedTexture>>,
    sampler_linear: wgpu::Sampler,
    sampler_nearest: wgpu::Sampler,
    bind_group_layout: wgpu::BindGroupLayout,
}

pub struct ManagedTexture {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub bind_group: wgpu::BindGroup,
    pub dimensions: (u32, u32),
    pub format: wgpu::TextureFormat,
    pub mip_level_count: u32,
}
```

The `TextureManager` owns two default samplers (linear filtering for most uses, nearest-neighbor for pixel art / voxel faces) and a shared bind group layout. Each `ManagedTexture` includes a pre-built `BindGroup` so that drawing code can simply bind it without creating bind groups each frame.

### Creation API

```rust
impl TextureManager {
    pub fn new(device: &wgpu::Device) -> Self { ... }

    /// Create a 2D texture from raw pixel data.
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
    ) -> Result<Arc<ManagedTexture>, TextureError> { ... }

    /// Create a 2D texture array (for voxel block faces, terrain layers, etc.).
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
    ) -> Result<Arc<ManagedTexture>, TextureError> { ... }

    /// Get a previously created texture by name.
    pub fn get(&self, name: &str) -> Option<Arc<ManagedTexture>> { ... }

    /// Remove a texture from the cache, freeing GPU memory if no other references exist.
    pub fn remove(&mut self, name: &str) -> bool { ... }

    /// The shared bind group layout for texture + sampler pairs.
    pub fn bind_group_layout(&self) -> &wgpu::BindGroupLayout { ... }
}
```

### Texture Creation Details

When `create_texture` is called:

1. **Check the cache**. If a texture with the same name already exists, return the cached `Arc<ManagedTexture>` without creating a new GPU texture. This deduplication prevents uploading the same image twice.

2. **Calculate mip levels**. If `generate_mipmaps` is true, compute the mip level count as `floor(log2(max(width, height))) + 1`. For a 256x256 texture, this is 9 levels (256, 128, 64, 32, 16, 8, 4, 2, 1).

3. **Create the GPU texture**:
   ```rust
   device.create_texture(&wgpu::TextureDescriptor {
       label: Some(name),
       size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
       mip_level_count,
       sample_count: 1,
       dimension: wgpu::TextureDimension::D2,
       format,
       usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::RENDER_ATTACHMENT, // for mipmap generation
       view_formats: &[],
   })
   ```

4. **Upload mip level 0** using `queue.write_texture()` with the provided raw pixel data.

5. **Generate mipmaps** if requested. Use a compute pass or a series of render passes that downsample each mip level from the previous one. The downsample shader uses bilinear filtering to average 2x2 texel blocks.

6. **Create the texture view** with `TextureViewDescriptor::default()`.

7. **Create the bind group** using the shared layout, the texture view, and the linear sampler:
   ```rust
   device.create_bind_group(&wgpu::BindGroupDescriptor {
       label: Some(&format!("{name}-bind-group")),
       layout: &self.bind_group_layout,
       entries: &[
           wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
           wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler_linear) },
       ],
   })
   ```

8. **Insert into the cache** and return the `Arc`.

### Bind Group Layout

The shared bind group layout for texture sampling:

```rust
device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
})
```

### Mipmap Level Calculation

```rust
pub fn mip_level_count(width: u32, height: u32) -> u32 {
    (width.max(height) as f32).log2().floor() as u32 + 1
}
```

### TextureLayerData

For texture arrays, each layer is described by:

```rust
pub struct TextureLayerData<'a> {
    pub data: &'a [u8],
    pub label: &'a str,
}
```

All layers must have the same dimensions and format.

### Error Type

```rust
#[derive(Debug, thiserror::Error)]
pub enum TextureError {
    #[error("texture data size ({actual}) does not match expected ({expected}) for {width}x{height} {format:?}")]
    DataSizeMismatch { actual: usize, expected: usize, width: u32, height: u32, format: wgpu::TextureFormat },

    #[error("texture dimensions must be non-zero, got {width}x{height}")]
    ZeroDimensions { width: u32, height: u32 },

    #[error("texture array layers have inconsistent dimensions")]
    InconsistentLayerDimensions,
}
```

## Outcome

A `TextureManager` that handles the full lifecycle of GPU textures: creation from raw pixel data, mipmap generation, bind group creation, caching by name, and cleanup. Downstream systems (materials, voxel rendering, UI) call `texture_manager.create_texture(...)` once and receive an `Arc<ManagedTexture>` with a ready-to-bind `BindGroup`. The cache prevents duplicate GPU textures for the same asset.

## Demo Integration

**Demo crate:** `nebula-demo`

A procedurally generated checkerboard texture is applied to a quad behind the triangles, proving that texture upload, binding, and sampling all work.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `wgpu` | `24.0` | GPU texture creation, mipmap generation, bind groups |
| `thiserror` | `2.0` | Error type derivation |
| `log` | `0.4` | Logging texture creation and cache hits |

No image loading crates here — that responsibility belongs to the asset loading system (`nebula-assets`). The `TextureManager` works with raw pixel data (`&[u8]`). Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_texture_with_valid_dimensions() {
        let (device, queue) = create_test_device_queue();
        let mut manager = TextureManager::new(&device);

        // 4x4 RGBA8 texture = 4 * 4 * 4 = 64 bytes
        let data = vec![255u8; 64];
        let result = manager.create_texture(
            &device, &queue, "test-4x4",
            &data, 4, 4,
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
        assert_eq!(mip_level_count(512, 256), 10); // max(512,256) = 512, log2(512) = 9, +1 = 10
        assert_eq!(mip_level_count(1024, 1024), 11);
    }

    #[test]
    fn test_bind_group_creation_succeeds() {
        let (device, queue) = create_test_device_queue();
        let mut manager = TextureManager::new(&device);

        let data = vec![128u8; 16]; // 2x2 RGBA8
        let tex = manager.create_texture(
            &device, &queue, "test-bind",
            &data, 2, 2,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            false,
        ).unwrap();

        // The ManagedTexture should have a bind group ready to use.
        // If bind group creation failed, create_texture would have returned an error.
        // Accessing the bind group should not panic.
        let _bg = &tex.bind_group;
    }

    #[test]
    fn test_texture_cache_deduplicates() {
        let (device, queue) = create_test_device_queue();
        let mut manager = TextureManager::new(&device);

        let data = vec![255u8; 16]; // 2x2 RGBA8
        let tex1 = manager.create_texture(
            &device, &queue, "shared",
            &data, 2, 2,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            false,
        ).unwrap();

        let tex2 = manager.create_texture(
            &device, &queue, "shared",
            &data, 2, 2,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            false,
        ).unwrap();

        // Both should return the same Arc (same allocation)
        assert!(Arc::ptr_eq(&tex1, &tex2));
    }

    #[test]
    fn test_rgba8_format_handling() {
        let (device, queue) = create_test_device_queue();
        let mut manager = TextureManager::new(&device);

        let data = vec![0u8; 256]; // 8x8 RGBA8 = 8*8*4 = 256 bytes
        let tex = manager.create_texture(
            &device, &queue, "rgba8-test",
            &data, 8, 8,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            false,
        ).unwrap();

        assert_eq!(tex.format, wgpu::TextureFormat::Rgba8UnormSrgb);
    }

    #[test]
    fn test_zero_dimensions_returns_error() {
        let (device, queue) = create_test_device_queue();
        let mut manager = TextureManager::new(&device);

        let result = manager.create_texture(
            &device, &queue, "zero",
            &[], 0, 0,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            false,
        );
        assert!(matches!(result, Err(TextureError::ZeroDimensions { .. })));
    }

    #[test]
    fn test_data_size_mismatch_returns_error() {
        let (device, queue) = create_test_device_queue();
        let mut manager = TextureManager::new(&device);

        // 4x4 RGBA8 expects 64 bytes, but we provide 32
        let data = vec![0u8; 32];
        let result = manager.create_texture(
            &device, &queue, "mismatch",
            &data, 4, 4,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            false,
        );
        assert!(matches!(result, Err(TextureError::DataSizeMismatch { .. })));
    }

    #[test]
    fn test_mipmap_generation_sets_correct_mip_count() {
        let (device, queue) = create_test_device_queue();
        let mut manager = TextureManager::new(&device);

        let data = vec![255u8; 256 * 256 * 4]; // 256x256 RGBA8
        let tex = manager.create_texture(
            &device, &queue, "mipmapped",
            &data, 256, 256,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            true, // generate mipmaps
        ).unwrap();

        assert_eq!(tex.mip_level_count, 9); // log2(256) + 1 = 9
    }

    #[test]
    fn test_remove_texture_from_cache() {
        let (device, queue) = create_test_device_queue();
        let mut manager = TextureManager::new(&device);

        let data = vec![0u8; 16];
        manager.create_texture(
            &device, &queue, "removable",
            &data, 2, 2,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            false,
        ).unwrap();

        assert!(manager.get("removable").is_some());
        assert!(manager.remove("removable"));
        assert!(manager.get("removable").is_none());
    }
}
```
