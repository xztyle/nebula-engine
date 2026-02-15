# Render Pass Abstraction

## Problem

wgpu's render pass setup is verbose and repetitive. Every frame requires creating a `CommandEncoder`, configuring `RenderPassColorAttachment` with load/store operations and clear colors, optionally attaching a depth stencil, setting up MSAA resolve targets, and finally submitting the encoder to the queue. Duplicating this boilerplate across every rendering subsystem (terrain, UI, particles, debug overlays) leads to inconsistent clear colors, forgotten depth attachments, and subtle bugs where command encoders are created but never submitted. The engine needs a thin abstraction that eliminates this boilerplate while remaining transparent — not a render graph, just ergonomic wrappers around wgpu's existing concepts.

## Solution

### RenderPassDescriptor Builder

Create a `RenderPassBuilder` that accumulates render pass configuration and produces a wgpu `RenderPassDescriptor`:

```rust
pub struct RenderPassBuilder {
    clear_color: wgpu::Color,
    depth_attachment: Option<DepthAttachmentConfig>,
    msaa_resolve_target: Option<wgpu::TextureView>,
    label: Option<&'static str>,
}

pub struct DepthAttachmentConfig {
    pub view: wgpu::TextureView,
    pub clear_value: f32,
    pub compare: wgpu::CompareFunction,
}
```

The builder provides a fluent API:

```rust
RenderPassBuilder::new()
    .label("main-pass")
    .clear_color(SKY_BLUE)
    .depth(depth_view, 0.0) // reverse-Z clear value
    .msaa_resolve(resolve_view)
    .build(encoder, surface_view)
```

**Default clear color** is sky blue `(0.529, 0.808, 0.922, 1.0)` — a visible, distinctive color that makes it obvious when geometry is missing, unlike black which hides rendering errors.

The `.depth()` method is optional. When omitted, no depth stencil attachment is created. This is useful for UI overlay passes that should not perform depth testing.

The `.msaa_resolve()` method is optional. When provided, the color attachment uses a multisampled texture as its view and the resolve target receives the resolved output.

### FrameEncoder

A `FrameEncoder` struct that manages the per-frame command encoding lifecycle:

```rust
pub struct FrameEncoder {
    encoder: wgpu::CommandEncoder,
    queue: Arc<wgpu::Queue>,
    surface_texture: wgpu::SurfaceTexture,
    submitted: bool,
}
```

Construction: `FrameEncoder::new(device, queue, surface_texture) -> Self`

The `FrameEncoder` provides:

- **`begin_render_pass(&mut self, builder: &RenderPassBuilder) -> wgpu::RenderPass`** — Creates a render pass using the builder's configuration and the surface texture's view. Returns the wgpu `RenderPass` for draw calls.

- **`submit(self)`** — Finishes the encoder, submits the command buffer to the queue, and presents the surface texture. Consumes `self` to prevent double-submission.

- **`Drop` implementation** — If `submit()` was not called explicitly, the `Drop` impl calls `submit()` automatically. This prevents frames from being silently lost. A warning is logged when auto-submit triggers, because it usually indicates a control flow bug. The `submitted` flag prevents double-submission.

### Constants

```rust
pub const SKY_BLUE: wgpu::Color = wgpu::Color {
    r: 0.529,
    g: 0.808,
    b: 0.922,
    a: 1.0,
};
```

### Multiple Render Passes

A single `FrameEncoder` can create multiple sequential render passes (e.g., main geometry pass, then transparent pass, then UI overlay pass). Each `begin_render_pass` call borrows the encoder mutably, and the render pass must be dropped before the next one begins. This is enforced by Rust's borrow checker — no runtime tracking needed.

## Outcome

A two-struct abstraction (`RenderPassBuilder` + `FrameEncoder`) that reduces per-frame rendering boilerplate from ~30 lines to ~5 lines. The `FrameEncoder`'s drop-based submission guarantees no frame is silently lost. Render pass configuration is declarative and reusable. Downstream systems (terrain renderer, UI renderer, debug overlay) each use a `RenderPassBuilder` for their specific pass without duplicating attachment setup code.

## Demo Integration

**Demo crate:** `nebula-demo`

No visible demo change; the internal rendering path now uses the formalized `FrameEncoder` and `RenderPassBuilder` structure for all future passes.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `wgpu` | `24.0` | GPU abstraction — provides CommandEncoder, RenderPass, etc. |
| `log` | `0.4` | Warn when FrameEncoder auto-submits on drop |

No additional dependencies beyond what `nebula-render` already requires from story 01. This story uses types from `RenderContext` (story 01) for device and queue access. Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_sets_clear_color() {
        let builder = RenderPassBuilder::new()
            .clear_color(wgpu::Color::RED);
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
    fn test_depth_attachment_can_be_set() {
        let depth_view = create_test_depth_view(); // helper
        let builder = RenderPassBuilder::new()
            .depth(depth_view, 0.0);
        assert!(builder.depth_attachment.is_some());
        let depth_cfg = builder.depth_attachment.as_ref().unwrap();
        assert_eq!(depth_cfg.clear_value, 0.0);
    }

    #[test]
    fn test_msaa_resolve_target_is_optional() {
        let builder = RenderPassBuilder::new();
        assert!(builder.msaa_resolve_target.is_none());
    }

    #[test]
    fn test_label_is_stored() {
        let builder = RenderPassBuilder::new()
            .label("my-pass");
        assert_eq!(builder.label, Some("my-pass"));
    }

    /// Verify that FrameEncoder submits on drop if not explicitly submitted.
    /// This test uses a mock queue that tracks submission count.
    #[test]
    fn test_encoder_submits_on_drop() {
        let (device, queue) = create_test_device_queue();
        let surface_texture = create_test_surface_texture();
        let submission_count = Arc::new(AtomicU32::new(0));
        {
            let encoder = FrameEncoder::new(&device, queue.clone(), surface_texture);
            // Do not call submit() — drop should handle it
        }
        // After drop, the command buffer should have been submitted.
        // Exact verification depends on test infrastructure.
    }

    /// Verify that calling submit() explicitly prevents double-submission on drop.
    #[test]
    fn test_explicit_submit_prevents_double_submit() {
        let (device, queue) = create_test_device_queue();
        let surface_texture = create_test_surface_texture();
        let encoder = FrameEncoder::new(&device, queue.clone(), surface_texture);
        encoder.submit(); // explicit submit
        // Drop runs here — should not submit again
    }
}
```
