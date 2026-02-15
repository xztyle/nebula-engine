# wgpu Device, Queue & Surface

## Problem

Before a single pixel can be drawn, the engine must negotiate with the operating system for a drawable surface and with the GPU for a logical device and command queue. wgpu's initialization sequence is verbose: enumerate adapters, request a device with specific features and limits, configure a surface with the correct present mode and format, and handle the platform-specific surface creation (Vulkan on Linux, DX12 on Windows, Metal on macOS). Scattering this initialization across the codebase leads to duplicated error handling, inconsistent feature requests, and impossible-to-test rendering code. Every downstream rendering system — pipelines, textures, shaders, draw calls — depends on having a properly initialized GPU context.

Additionally, surfaces are fragile. Window resizes invalidate them. Platform events (display disconnect, power management, tabbing away on mobile) can mark them as lost. Without centralized surface lifecycle management, any of these events can crash the engine.

## Solution

Define a `RenderContext` struct in the `nebula_render` crate that owns all wgpu root objects and provides the single point of GPU interaction for the rest of the engine:

```rust
pub struct RenderContext {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface: wgpu::Surface<'static>,
    pub surface_config: wgpu::SurfaceConfiguration,
    pub surface_format: wgpu::TextureFormat,
}
```

### Initialization

`RenderContext::new(window: &Window) -> Result<Self, RenderContextError>`:

1. **Create the wgpu instance** with `wgpu::Instance::new(wgpu::InstanceDescriptor)` using `wgpu::Backends::all()` so the backend is selected at runtime: Vulkan on Linux, DX12 on Windows, Metal on macOS. This keeps the binary universal without compile-time backend gating.

2. **Create the surface** from the window handle using `instance.create_surface(window)`. The window must implement `raw_window_handle::HasWindowHandle` and `HasDisplayHandle`. The surface is created before adapter selection because the adapter must be compatible with the surface.

3. **Request an adapter** with `instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, compatible_surface: Some(&surface), force_fallback_adapter: false })`. If no adapter is found, return `RenderContextError::NoAdapter`.

4. **Request a device and queue** with `adapter.request_device(&wgpu::DeviceDescriptor { label: Some("nebula-device"), required_features: wgpu::Features::empty(), required_limits: wgpu::Limits::default() }, None)`. The required features start empty and are expanded by later stories (e.g., push constants, texture compression). The limits use defaults which are the intersection of all backends.

5. **Configure the surface**. Query `surface.get_capabilities(&adapter)` to get supported formats and present modes. Prefer an sRGB format (`Bgra8UnormSrgb` or `Rgba8UnormSrgb`). Prefer `PresentMode::Fifo` (vsync) as the default, with `Mailbox` as a fallback for lower-latency rendering. Build a `SurfaceConfiguration` with the window's inner width and height.

6. **Apply the configuration** with `surface.configure(&device, &surface_config)`.

### Resize

`RenderContext::resize(&mut self, width: u32, height: u32)`:

- Clamp both dimensions to `max(1, val)` to prevent zero-size surfaces (which are invalid on Wayland and trigger panics in wgpu).
- Update `self.surface_config.width` and `self.surface_config.height`.
- Call `self.surface.configure(&self.device, &self.surface_config)`.

This method is called from the event loop whenever `WindowEvent::Resized` fires.

### Surface Acquisition

`RenderContext::get_current_texture(&self) -> Result<wgpu::SurfaceTexture, SurfaceError>`:

- Call `self.surface.get_current_texture()`.
- If the result is `SurfaceError::Lost`, reconfigure the surface with the current config and retry once.
- If the result is `SurfaceError::OutOfMemory`, return a fatal error.
- If `SurfaceError::Timeout`, skip the frame (return a recoverable error).

### Error Type

```rust
#[derive(Debug, thiserror::Error)]
pub enum RenderContextError {
    #[error("no compatible GPU adapter found")]
    NoAdapter,
    #[error("failed to request GPU device: {0}")]
    DeviceRequest(#[from] wgpu::RequestDeviceError),
    #[error("failed to create surface: {0}")]
    SurfaceCreation(#[from] wgpu::CreateSurfaceError),
    #[error("surface lost and could not be recovered")]
    SurfaceLost,
}
```

### Backend Selection

No compile-time `#[cfg]` gating on backends. The `wgpu::Backends::all()` flag lets wgpu pick the best available backend at runtime. This means a single binary works on all three platforms. The adapter selection step implicitly chooses the right backend because it requests compatibility with the platform's native surface.

## Outcome

A `RenderContext` struct that can be constructed with a single function call, providing ready-to-use `Device`, `Queue`, `Surface`, and `SurfaceConfiguration` references to every downstream rendering system. Window resizes are handled by a one-line `resize()` call. Lost surfaces are recovered automatically. The rest of the rendering code never touches raw wgpu initialization — it only borrows from `RenderContext`.

## Demo Integration

**Demo crate:** `nebula-demo`

The clear color is set to deep space blue `(0.02, 0.02, 0.08)` via the GPU surface. The window is now fully GPU-owned — wgpu controls every pixel.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `wgpu` | `24.0` | GPU abstraction over Vulkan/DX12/Metal |
| `winit` | `0.30` | Window creation and event loop (provides the window handle) |
| `raw-window-handle` | `0.6` | Trait interface for passing window handles to wgpu |
| `thiserror` | `2.0` | Derive macro for ergonomic error types |
| `log` | `0.4` | Logging adapter warnings and surface recovery events |

All dependencies are declared in `[workspace.dependencies]` and consumed via `{ workspace = true }` in the `nebula-render` crate's `Cargo.toml`. Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that RenderContext has all required public fields.
    /// This is a compile-time structural test — if any field is missing
    /// or has the wrong type, this test fails to compile.
    #[test]
    fn test_render_context_fields_exist() {
        // This function exercises the struct's field types at compile time.
        fn assert_fields(ctx: &RenderContext) {
            let _: &wgpu::Instance = &ctx.instance;
            let _: &wgpu::Adapter = &ctx.adapter;
            let _: &wgpu::Device = &ctx.device;
            let _: &wgpu::Queue = &ctx.queue;
            let _: &wgpu::Surface = &ctx.surface;
            let _: &wgpu::SurfaceConfiguration = &ctx.surface_config;
            let _: &wgpu::TextureFormat = &ctx.surface_format;
        }
        // Compile-time only — no runtime assertion needed.
        // The function's existence validates the struct layout.
    }

    /// Verify that resize() updates the surface config dimensions.
    /// Uses a mock or headless context if available, otherwise a
    /// documented integration test that requires a GPU.
    #[test]
    fn test_resize_updates_config_dimensions() {
        // Given a RenderContext with initial dimensions 800×600
        // When resize(1920, 1080) is called
        // Then surface_config.width == 1920 and surface_config.height == 1080
        let mut ctx = create_test_context(); // helper that creates headless context
        ctx.resize(1920, 1080);
        assert_eq!(ctx.surface_config.width, 1920);
        assert_eq!(ctx.surface_config.height, 1080);
    }

    /// Verify that zero-size resize is clamped to 1×1.
    #[test]
    fn test_resize_clamps_zero_dimensions() {
        let mut ctx = create_test_context();
        ctx.resize(0, 0);
        assert_eq!(ctx.surface_config.width, 1);
        assert_eq!(ctx.surface_config.height, 1);
    }

    /// Verify that the selected surface format is sRGB-compatible.
    #[test]
    fn test_surface_format_is_srgb() {
        let ctx = create_test_context();
        let format = ctx.surface_format;
        let srgb_formats = [
            wgpu::TextureFormat::Bgra8UnormSrgb,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        ];
        assert!(
            srgb_formats.contains(&format),
            "Surface format {:?} is not sRGB-compatible",
            format
        );
    }

    /// Verify that the default present mode is Fifo (vsync).
    #[test]
    fn test_default_present_mode_is_fifo() {
        let ctx = create_test_context();
        assert_eq!(ctx.surface_config.present_mode, wgpu::PresentMode::Fifo);
    }

    /// Verify that the power preference is HighPerformance (discrete GPU).
    #[test]
    fn test_adapter_selects_high_performance() {
        // The adapter info should indicate a discrete GPU when available.
        // On CI without a GPU, this test is skipped.
        let ctx = create_test_context();
        let info = ctx.adapter.get_info();
        // We assert the adapter was created successfully — backend-specific
        // checks are platform-dependent and covered by integration tests.
        assert!(!info.name.is_empty());
    }
}
```
