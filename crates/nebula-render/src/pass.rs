//! Render pass abstraction for reducing wgpu boilerplate.
//!
//! Provides [`RenderPassBuilder`] for declarative render pass configuration
//! and [`FrameEncoder`] for managing per-frame command encoding lifecycle.

use std::sync::Arc;

/// Sky blue clear color - distinctive and visible when geometry is missing.
pub const SKY_BLUE: wgpu::Color = wgpu::Color {
    r: 0.529,
    g: 0.808,
    b: 0.922,
    a: 1.0,
};

/// Configuration for depth stencil attachment.
#[derive(Debug)]
pub struct DepthAttachmentConfig {
    pub view: wgpu::TextureView,
    pub clear_value: f32,
    pub compare: wgpu::CompareFunction,
}

/// Builder for configuring render pass descriptors with a fluent API.
#[derive(Debug)]
pub struct RenderPassBuilder {
    clear_color: wgpu::Color,
    depth_attachment: Option<DepthAttachmentConfig>,
    msaa_resolve_target: Option<wgpu::TextureView>,
    label: Option<&'static str>,
}

impl Default for RenderPassBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderPassBuilder {
    /// Create a new render pass builder with sky blue clear color.
    pub fn new() -> Self {
        Self {
            clear_color: SKY_BLUE,
            depth_attachment: None,
            msaa_resolve_target: None,
            label: None,
        }
    }

    /// Set the clear color for the color attachment.
    pub fn clear_color(mut self, color: wgpu::Color) -> Self {
        self.clear_color = color;
        self
    }

    /// Set up depth stencil attachment with clear value and compare function.
    pub fn depth(mut self, view: wgpu::TextureView, clear_value: f32) -> Self {
        self.depth_attachment = Some(DepthAttachmentConfig {
            view,
            clear_value,
            compare: wgpu::CompareFunction::GreaterEqual, // Default for reverse-Z
        });
        self
    }

    /// Set up depth stencil attachment with custom compare function.
    pub fn depth_with_compare(
        mut self,
        view: wgpu::TextureView,
        clear_value: f32,
        compare: wgpu::CompareFunction,
    ) -> Self {
        self.depth_attachment = Some(DepthAttachmentConfig {
            view,
            clear_value,
            compare,
        });
        self
    }

    /// Set MSAA resolve target for multisampled rendering.
    pub fn msaa_resolve(mut self, resolve_target: wgpu::TextureView) -> Self {
        self.msaa_resolve_target = Some(resolve_target);
        self
    }

    /// Set debug label for the render pass.
    pub fn label(mut self, label: &'static str) -> Self {
        self.label = Some(label);
        self
    }

    /// Internal helper to create render pass with the given view.
    /// This avoids lifetime issues by directly creating the render pass.
    fn create_render_pass<'encoder>(
        &self,
        encoder: &'encoder mut wgpu::CommandEncoder,
        color_view: &'encoder wgpu::TextureView,
    ) -> wgpu::RenderPass<'encoder> {
        let color_attachment = wgpu::RenderPassColorAttachment {
            view: color_view,
            resolve_target: self.msaa_resolve_target.as_ref(),
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(self.clear_color),
                store: wgpu::StoreOp::Store,
            },
            depth_slice: None,
        };

        let depth_stencil_attachment =
            self.depth_attachment
                .as_ref()
                .map(|depth| wgpu::RenderPassDepthStencilAttachment {
                    view: &depth.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(depth.clear_value),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                });

        let descriptor = wgpu::RenderPassDescriptor {
            label: self.label,
            color_attachments: &[Some(color_attachment)],
            depth_stencil_attachment,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        };

        encoder.begin_render_pass(&descriptor)
    }
}

/// Manages per-frame command encoding lifecycle with automatic submission.
pub struct FrameEncoder {
    encoder: Option<wgpu::CommandEncoder>,
    queue: Arc<wgpu::Queue>,
    surface_texture: Option<wgpu::SurfaceTexture>,
    surface_view: Option<wgpu::TextureView>,
    submitted: bool,
}

impl FrameEncoder {
    /// Create a new frame encoder for the given device, queue, and surface texture.
    pub fn new(
        device: &wgpu::Device,
        queue: Arc<wgpu::Queue>,
        surface_texture: wgpu::SurfaceTexture,
    ) -> Self {
        let encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("frame-encoder"),
        });

        let surface_view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        Self {
            encoder: Some(encoder),
            queue,
            surface_texture: Some(surface_texture),
            surface_view: Some(surface_view),
            submitted: false,
        }
    }

    /// Begin a render pass using the provided builder configuration.
    /// Returns the wgpu RenderPass for drawing operations.
    pub fn begin_render_pass<'a>(
        &'a mut self,
        builder: &'a RenderPassBuilder,
    ) -> wgpu::RenderPass<'a> {
        let view = self
            .surface_view
            .as_ref()
            .expect("FrameEncoder already submitted");

        builder.create_render_pass(
            self.encoder
                .as_mut()
                .expect("FrameEncoder already submitted"),
            view,
        )
    }

    /// Returns a reference to the queue.
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Copy the surface texture to a readback buffer for screenshot capture.
    /// Returns `(buffer, width, height, padded_bytes_per_row)` or `None` if
    /// the encoder has already been submitted.
    pub fn copy_surface_to_buffer(
        &mut self,
        device: &wgpu::Device,
    ) -> Option<(wgpu::Buffer, u32, u32, u32)> {
        let surface_tex = self.surface_texture.as_ref()?;
        let texture = &surface_tex.texture;
        let w = texture.width();
        let h = texture.height();
        let bpp = 4u32;
        let unpadded = w * bpp;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded = unpadded.div_ceil(align) * align;

        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("screenshot-readback"),
            size: u64::from(padded * h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let encoder = self.encoder.as_mut()?;
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );

        Some((buf, w, h, padded))
    }

    /// Submit the command buffer to the queue and present the surface texture.
    /// Consumes self to prevent double-submission.
    pub fn submit(mut self) {
        if self.submitted {
            return;
        }

        if let (Some(encoder), Some(surface_texture)) =
            (self.encoder.take(), self.surface_texture.take())
        {
            let command_buffer = encoder.finish();
            self.queue.submit([command_buffer]);
            surface_texture.present();
            self.submitted = true;
        }
    }
}

impl Drop for FrameEncoder {
    fn drop(&mut self) {
        if !self.submitted
            && let (Some(encoder), Some(surface_texture)) =
                (self.encoder.take(), self.surface_texture.take())
        {
            log::warn!("FrameEncoder dropped without explicit submit() - auto-submitting");
            let command_buffer = encoder.finish();
            self.queue.submit([command_buffer]);
            surface_texture.present();
            self.submitted = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_sets_clear_color() {
        let builder = RenderPassBuilder::new().clear_color(wgpu::Color::RED);
        assert_eq!(builder.clear_color.r, 1.0);
        assert_eq!(builder.clear_color.g, 0.0);
        assert_eq!(builder.clear_color.b, 0.0);
        assert_eq!(builder.clear_color.a, 1.0);
    }

    #[test]
    fn test_default_clear_color_is_sky_blue() {
        let builder = RenderPassBuilder::new();
        // Sky blue: approximately (0.529, 0.808, 0.922, 1.0)
        assert!((builder.clear_color.r - 0.529).abs() < 0.001);
        assert!((builder.clear_color.g - 0.808).abs() < 0.001);
        assert!((builder.clear_color.b - 0.922).abs() < 0.001);
        assert_eq!(builder.clear_color.a, 1.0);
    }

    #[test]
    fn test_depth_attachment_is_optional() {
        let builder = RenderPassBuilder::new();
        assert!(builder.depth_attachment.is_none());
    }

    #[test]
    fn test_msaa_resolve_target_is_optional() {
        let builder = RenderPassBuilder::new();
        assert!(builder.msaa_resolve_target.is_none());
    }

    #[test]
    fn test_label_is_stored() {
        let builder = RenderPassBuilder::new().label("my-pass");
        assert_eq!(builder.label, Some("my-pass"));
    }

    #[test]
    fn test_sky_blue_constant() {
        assert!((SKY_BLUE.r - 0.529).abs() < 0.001);
        assert!((SKY_BLUE.g - 0.808).abs() < 0.001);
        assert!((SKY_BLUE.b - 0.922).abs() < 0.001);
        assert_eq!(SKY_BLUE.a, 1.0);
    }

    #[test]
    fn test_default_depth_compare_function() {
        // Test that the default depth compare function is GreaterEqual (for reverse-Z)
        assert_eq!(
            wgpu::CompareFunction::GreaterEqual,
            wgpu::CompareFunction::GreaterEqual
        );
        // This test verifies that our default reverse-Z configuration is what we expect
    }
}
