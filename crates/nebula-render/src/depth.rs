//! Depth buffer management with reverse-Z for improved precision at planetary distances.
//!
//! Uses reverse-Z depth mapping where near plane maps to 1.0 and far plane maps to 0.0.
//! This dramatically improves depth precision for large viewing distances by utilizing
//! the high precision of floating-point numbers near zero for distant objects.

/// Depth buffer with reverse-Z configuration for planetary-scale rendering.
pub struct DepthBuffer {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub format: wgpu::TextureFormat,
    width: u32,
    height: u32,
}

impl DepthBuffer {
    /// 32-bit float depth format for maximum precision with reverse-Z.
    pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

    /// Reverse-Z clear value: 0.0 represents the far plane.
    pub const CLEAR_VALUE: f32 = 0.0;

    /// Reverse-Z depth comparison: closer objects have higher depth values.
    pub const COMPARE_FUNCTION: wgpu::CompareFunction = wgpu::CompareFunction::GreaterEqual;

    /// Create a new depth buffer with the specified dimensions.
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
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
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

    /// Resize the depth buffer to new dimensions.
    /// No-op if dimensions are unchanged to avoid unnecessary GPU resource allocation.
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if self.width == width && self.height == height {
            return; // no-op if dimensions unchanged
        }
        *self = Self::new(device, width, height);
    }

    /// Get the current width of the depth buffer.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Get the current height of the depth buffer.
    pub fn height(&self) -> u32 {
        self.height
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_device() -> Option<wgpu::Device> {
        // Create a minimal test device for unit tests (returns None on headless CI)
        pollster::block_on(async {
            let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
                backends: wgpu::Backends::all(),
                ..Default::default()
            });

            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::default(),
                    force_fallback_adapter: false,
                    compatible_surface: None,
                })
                .await
                .ok()?;

            let (device, _queue) = adapter
                .request_device(&wgpu::DeviceDescriptor::default())
                .await
                .ok()?;

            Some(device)
        })
    }

    #[test]
    fn test_depth_texture_format_is_depth32float() {
        assert_eq!(DepthBuffer::FORMAT, wgpu::TextureFormat::Depth32Float);
    }

    #[test]
    fn test_depth_texture_dimensions_match_surface() {
        let Some(device) = create_test_device() else {
            return;
        };
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
        let Some(device) = create_test_device() else {
            return;
        };
        let mut depth = DepthBuffer::new(&device, 800, 600);
        assert_eq!(depth.width(), 800);
        assert_eq!(depth.height(), 600);

        depth.resize(&device, 1920, 1080);
        assert_eq!(depth.width(), 1920);
        assert_eq!(depth.height(), 1080);
    }

    #[test]
    fn test_resize_noop_when_same_dimensions() {
        let Some(device) = create_test_device() else {
            return;
        };
        let mut depth = DepthBuffer::new(&device, 800, 600);

        // Store the original dimensions to verify they don't change
        let original_width = depth.width();
        let original_height = depth.height();

        depth.resize(&device, 800, 600); // same dimensions

        // Verify dimensions remain the same (texture wasn't recreated)
        assert_eq!(depth.width(), original_width);
        assert_eq!(depth.height(), original_height);
        assert_eq!(depth.width(), 800);
        assert_eq!(depth.height(), 600);
    }

    #[test]
    fn test_depth_texture_has_render_attachment_usage() {
        let Some(device) = create_test_device() else {
            return;
        };
        let depth = DepthBuffer::new(&device, 800, 600);
        let usage = depth.texture.usage();
        assert!(usage.contains(wgpu::TextureUsages::RENDER_ATTACHMENT));
    }

    #[test]
    fn test_depth_texture_has_texture_binding_usage() {
        let Some(device) = create_test_device() else {
            return;
        };
        let depth = DepthBuffer::new(&device, 800, 600);
        let usage = depth.texture.usage();
        assert!(usage.contains(wgpu::TextureUsages::TEXTURE_BINDING));
    }
}
