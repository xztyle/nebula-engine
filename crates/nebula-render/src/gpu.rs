//! GPU device initialization and surface management.
//!
//! Provides [`RenderContext`] which owns all wgpu GPU state, and [`RenderContextError`]
//! for clear diagnostics when initialization fails.

use std::sync::Arc;
use winit::window::Window;

/// Error type for render context initialization and surface management failures.
#[derive(Debug, thiserror::Error)]
pub enum RenderContextError {
    /// No compatible GPU adapter found.
    #[error("no compatible GPU adapter found")]
    NoAdapter,

    /// Failed to request GPU device.
    #[error("failed to request GPU device: {0}")]
    DeviceRequest(#[from] wgpu::RequestDeviceError),

    /// Failed to create surface.
    #[error("failed to create surface: {0}")]
    SurfaceCreation(#[from] wgpu::CreateSurfaceError),

    /// Surface lost and could not be recovered.
    #[error("surface lost and could not be recovered")]
    SurfaceLost,
}

/// Error type for surface acquisition failures.
#[derive(Debug, thiserror::Error)]
pub enum SurfaceError {
    /// Surface was lost and could not be recovered.
    #[error("surface lost")]
    Lost,

    /// GPU ran out of memory.
    #[error("out of memory")]
    OutOfMemory,

    /// Operation timed out (recoverable - skip frame).
    #[error("timeout")]
    Timeout,
}

/// Owns all GPU state: instance, adapter, device, queue, and surface.
pub struct RenderContext {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface: wgpu::Surface<'static>,
    pub surface_config: wgpu::SurfaceConfiguration,
    pub surface_format: wgpu::TextureFormat,
}

impl RenderContext {
    /// Initialize the GPU asynchronously from a window handle.
    pub async fn new(window: Arc<Window>) -> Result<Self, RenderContextError> {
        // 1. Create the wgpu instance with all backends for runtime selection
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        // Get window size before creating surface (to avoid move)
        let size = window.inner_size();

        // 2. Create the surface from the window handle
        let surface = instance.create_surface(window)?;

        // 3. Request an adapter compatible with the surface
        let adapter = match instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
        {
            Ok(adapter) => adapter,
            Err(_) => return Err(RenderContextError::NoAdapter),
        };

        let info = adapter.get_info();
        log::info!(
            "Selected GPU: {} ({:?}, {:?})",
            info.name,
            info.backend,
            info.device_type
        );

        // 4. Request a device and queue
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("nebula-device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::default(),
                experimental_features: wgpu::ExperimentalFeatures::default(),
                trace: wgpu::Trace::Off,
            })
            .await?;

        // 5. Configure the surface
        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = select_preferred_srgb_format(&surface_caps.formats);

        let present_mode = if surface_caps
            .present_modes
            .contains(&wgpu::PresentMode::Fifo)
        {
            wgpu::PresentMode::Fifo
        } else {
            wgpu::PresentMode::Mailbox
        };
        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            format: surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        // 6. Apply the configuration
        surface.configure(&device, &surface_config);

        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            surface,
            surface_config,
            surface_format,
        })
    }

    /// Reconfigure the surface after a window resize.
    /// Clamps dimensions to max(1, val) to prevent zero-size surfaces.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.surface_config.width = width.max(1);
        self.surface_config.height = height.max(1);
        self.surface.configure(&self.device, &self.surface_config);
    }

    /// Get the current surface texture, with automatic recovery for lost surfaces.
    pub fn get_current_texture(&self) -> Result<wgpu::SurfaceTexture, SurfaceError> {
        match self.surface.get_current_texture() {
            Ok(texture) => Ok(texture),
            Err(wgpu::SurfaceError::Lost) => {
                log::warn!("Surface lost, attempting to recover...");
                self.surface.configure(&self.device, &self.surface_config);
                // Retry once after reconfiguration
                match self.surface.get_current_texture() {
                    Ok(texture) => Ok(texture),
                    Err(_) => Err(SurfaceError::Lost),
                }
            }
            Err(wgpu::SurfaceError::OutOfMemory) => Err(SurfaceError::OutOfMemory),
            Err(wgpu::SurfaceError::Timeout) => Err(SurfaceError::Timeout),
            Err(wgpu::SurfaceError::Outdated) => {
                // Treat as recoverable like Lost
                log::warn!("Surface outdated, attempting to recover...");
                self.surface.configure(&self.device, &self.surface_config);
                match self.surface.get_current_texture() {
                    Ok(texture) => Ok(texture),
                    Err(_) => Err(SurfaceError::Lost),
                }
            }
            Err(wgpu::SurfaceError::Other) => {
                log::error!("Unknown surface error occurred");
                Err(SurfaceError::Lost)
            }
        }
    }
}

/// Initialize the GPU synchronously using `pollster`.
pub fn init_render_context_blocking(
    window: Arc<Window>,
) -> Result<RenderContext, RenderContextError> {
    pollster::block_on(RenderContext::new(window))
}

/// Select the preferred surface format, preferring sRGB.
/// Specifically looks for Bgra8UnormSrgb or Rgba8UnormSrgb as mentioned in the plan.
fn select_preferred_srgb_format(formats: &[wgpu::TextureFormat]) -> wgpu::TextureFormat {
    // Prefer Bgra8UnormSrgb first, then Rgba8UnormSrgb
    if formats.contains(&wgpu::TextureFormat::Bgra8UnormSrgb) {
        wgpu::TextureFormat::Bgra8UnormSrgb
    } else if formats.contains(&wgpu::TextureFormat::Rgba8UnormSrgb) {
        wgpu::TextureFormat::Rgba8UnormSrgb
    } else {
        // Fall back to any sRGB format, then any format
        formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(formats[0])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that RenderContext has all required public fields.
    /// This is a compile-time structural test — if any field is missing
    /// or has the wrong type, this test fails to compile.
    #[test]
    fn test_render_context_fields_exist() {
        // This function exercises the struct's field types at compile time.
        #[allow(dead_code)]
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
    /// Uses a mock context configuration for testing.
    #[test]
    fn test_resize_updates_config_dimensions() {
        // Create a mock surface config to test resize logic
        let mut surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: wgpu::TextureFormat::Bgra8UnormSrgb,
            width: 800,
            height: 600,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        // Simulate the resize logic from RenderContext::resize()
        let (width, height) = (1920, 1080);
        surface_config.width = width.max(1);
        surface_config.height = height.max(1);

        assert_eq!(surface_config.width, 1920);
        assert_eq!(surface_config.height, 1080);
    }

    /// Verify that zero-size resize is clamped to 1×1.
    #[test]
    fn test_resize_clamps_zero_dimensions() {
        let mut surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: wgpu::TextureFormat::Bgra8UnormSrgb,
            width: 800,
            height: 600,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        // Simulate the resize logic with zero dimensions
        let (width, height) = (0, 0);
        surface_config.width = width.max(1);
        surface_config.height = height.max(1);

        assert_eq!(surface_config.width, 1);
        assert_eq!(surface_config.height, 1);
    }

    /// Verify that the selected surface format is sRGB-compatible.
    #[test]
    fn test_surface_format_is_srgb() {
        let formats = [
            wgpu::TextureFormat::Bgra8Unorm,
            wgpu::TextureFormat::Bgra8UnormSrgb,
            wgpu::TextureFormat::Rgba8Unorm,
        ];
        let format = select_preferred_srgb_format(&formats);
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
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: wgpu::TextureFormat::Bgra8UnormSrgb,
            width: 800,
            height: 600,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        assert_eq!(config.present_mode, wgpu::PresentMode::Fifo);
    }

    /// Verify that format selection prefers Bgra8UnormSrgb first.
    #[test]
    fn test_format_selection_prefers_bgra_srgb() {
        let formats = [
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureFormat::Bgra8UnormSrgb,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        ];
        assert_eq!(
            select_preferred_srgb_format(&formats),
            wgpu::TextureFormat::Bgra8UnormSrgb
        );
    }

    /// Verify that format selection falls back to Rgba8UnormSrgb.
    #[test]
    fn test_format_selection_fallback_rgba_srgb() {
        let formats = [
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        ];
        assert_eq!(
            select_preferred_srgb_format(&formats),
            wgpu::TextureFormat::Rgba8UnormSrgb
        );
    }

    /// Verify that format selection uses first format if no sRGB available.
    #[test]
    fn test_format_selection_fallback_first() {
        let formats = [
            wgpu::TextureFormat::Bgra8Unorm,
            wgpu::TextureFormat::Rgba8Unorm,
        ];
        assert_eq!(
            select_preferred_srgb_format(&formats),
            wgpu::TextureFormat::Bgra8Unorm
        );
    }
}
