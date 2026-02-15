# Depth Buffer

## Problem

Without a depth buffer, geometry drawn later always appears in front of geometry drawn earlier, regardless of actual distance from the camera. This makes 3D rendering useless — a mountain behind the player would appear in front of the player's hand if it is drawn second. A depth buffer stores a per-pixel depth value so the GPU can reject fragments that are behind already-drawn geometry. Additionally, standard depth buffer configurations waste most of their floating-point precision on the near range, causing z-fighting (flickering surfaces) at medium and far distances. For a planetary engine where the camera needs to see from hand-held objects (0.1m) to distant terrain (10+ km), z-fighting is a critical problem that must be solved from the start.

## Solution

### Reverse-Z Depth

Use reverse-Z depth mapping, where the near plane maps to depth 1.0 and the far plane maps to depth 0.0. This is the opposite of the traditional convention (near=0, far=1) and dramatically improves depth precision at large distances.

**Why reverse-Z works**: Floating-point numbers have more precision near zero. In traditional depth mapping, this precision is "wasted" on distant objects (depth values near 1.0 are crowded together). By flipping the mapping, the high precision near zero is used for distant objects — exactly where z-fighting is worst. Combined with a 32-bit float depth buffer, reverse-Z provides nearly uniform precision across the entire depth range.

The reverse-Z mapping is established by:

1. **Projection matrix** (story 06): Swap near and far in the projection matrix construction.
2. **Depth clear value**: Clear the depth buffer to 0.0 (the far plane), not 1.0.
3. **Depth compare function**: Use `GreaterEqual` instead of `LessEqual`. A fragment passes the depth test if its depth is greater than or equal to the stored value — i.e., closer to the camera.

### DepthBuffer Struct

```rust
pub struct DepthBuffer {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub format: wgpu::TextureFormat,
    width: u32,
    height: u32,
}
```

### Creation

```rust
impl DepthBuffer {
    pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
    pub const CLEAR_VALUE: f32 = 0.0; // reverse-Z: far plane
    pub const COMPARE_FUNCTION: wgpu::CompareFunction = wgpu::CompareFunction::GreaterEqual;

    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("depth-buffer"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: Self::FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                 | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        Self {
            texture,
            view,
            format: Self::FORMAT,
            width,
            height,
        }
    }
}
```

The `TEXTURE_BINDING` usage flag is included alongside `RENDER_ATTACHMENT` to allow the depth buffer to be sampled in later passes (e.g., SSAO, screen-space reflections, depth-based fog).

### Resize

```rust
impl DepthBuffer {
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if self.width == width && self.height == height {
            return; // no-op if dimensions unchanged
        }
        *self = Self::new(device, width, height);
    }

    pub fn width(&self) -> u32 { self.width }
    pub fn height(&self) -> u32 { self.height }
}
```

This method is called from the window resize handler, immediately after `RenderContext::resize()`. The old texture and view are dropped (GPU resources freed) when `*self` is reassigned.

### Integration with Render Pass

The depth buffer view is passed to the `RenderPassBuilder` (story 02):

```rust
let render_pass = RenderPassBuilder::new()
    .clear_color(SKY_BLUE)
    .depth(depth_buffer.view.clone(), DepthBuffer::CLEAR_VALUE)
    .build(encoder, surface_view);
```

### Integration with Pipelines

Pipelines that perform depth testing include the depth stencil state:

```rust
depth_stencil: Some(wgpu::DepthStencilState {
    format: DepthBuffer::FORMAT,
    depth_write_enabled: true,
    depth_compare: DepthBuffer::COMPARE_FUNCTION,
    stencil: wgpu::StencilState::default(),
    bias: wgpu::DepthBiasState::default(),
}),
```

### Depth32Float vs Depth24PlusStencil8

`Depth32Float` is chosen over `Depth24PlusStencil8` because:

- Full 32-bit float precision is essential for reverse-Z to work well at planetary distances.
- The engine does not currently need a stencil buffer. If stencil is needed later (e.g., for decals or portal rendering), a separate stencil attachment can be added without compromising depth precision.
- `Depth32Float` is universally supported on all wgpu backends.

## Outcome

A `DepthBuffer` struct that creates and manages a `Depth32Float` texture with reverse-Z conventions. The depth buffer is automatically recreated on window resize. Pipelines reference `DepthBuffer::FORMAT` and `DepthBuffer::COMPARE_FUNCTION` as constants, ensuring consistency across the entire rendering stack. Reverse-Z eliminates z-fighting at planetary distances.

## Demo Integration

**Demo crate:** `nebula-demo`

A second triangle is placed behind the first, partially overlapping. Depth testing ensures the front triangle correctly occludes the rear one, proving 3D occlusion works.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `wgpu` | `24.0` | Depth texture creation and render attachment |

No additional dependencies. Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_depth_texture_format_is_depth32float() {
        assert_eq!(DepthBuffer::FORMAT, wgpu::TextureFormat::Depth32Float);
    }

    #[test]
    fn test_depth_texture_dimensions_match_surface() {
        let device = create_test_device();
        let depth = DepthBuffer::new(&device, 1920, 1080);
        assert_eq!(depth.width(), 1920);
        assert_eq!(depth.height(), 1080);
    }

    #[test]
    fn test_reverse_z_clear_value_is_zero() {
        // In reverse-Z, the far plane is 0.0, which is the clear value.
        assert_eq!(DepthBuffer::CLEAR_VALUE, 0.0);
    }

    #[test]
    fn test_depth_compare_function_is_greater_equal() {
        // Reverse-Z: closer objects have HIGHER depth values.
        // GreaterEqual means "pass if new depth >= stored depth" — i.e., closer wins.
        assert_eq!(
            DepthBuffer::COMPARE_FUNCTION,
            wgpu::CompareFunction::GreaterEqual
        );
    }

    #[test]
    fn test_resize_updates_dimensions() {
        let device = create_test_device();
        let mut depth = DepthBuffer::new(&device, 800, 600);
        assert_eq!(depth.width(), 800);
        assert_eq!(depth.height(), 600);

        depth.resize(&device, 1920, 1080);
        assert_eq!(depth.width(), 1920);
        assert_eq!(depth.height(), 1080);
    }

    #[test]
    fn test_resize_noop_when_same_dimensions() {
        let device = create_test_device();
        let mut depth = DepthBuffer::new(&device, 800, 600);
        let original_id = depth.texture.global_id();

        depth.resize(&device, 800, 600); // same dimensions
        // The texture should not be recreated
        assert_eq!(depth.texture.global_id(), original_id);
    }

    #[test]
    fn test_depth_texture_has_render_attachment_usage() {
        let device = create_test_device();
        let depth = DepthBuffer::new(&device, 800, 600);
        let usage = depth.texture.usage();
        assert!(usage.contains(wgpu::TextureUsages::RENDER_ATTACHMENT));
    }

    #[test]
    fn test_depth_texture_has_texture_binding_usage() {
        let device = create_test_device();
        let depth = DepthBuffer::new(&device, 800, 600);
        let usage = depth.texture.usage();
        assert!(usage.contains(wgpu::TextureUsages::TEXTURE_BINDING));
    }
}
```
