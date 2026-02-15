//! GPU device initialization and surface management.
//!
//! Provides [`GpuContext`] which owns all wgpu GPU state, and [`GpuInitError`]
//! for clear diagnostics when initialization fails.

use std::sync::Arc;

use winit::window::Window;

/// Error type for GPU initialization failures.
#[derive(Debug, thiserror::Error)]
pub enum GpuInitError {
    /// The GPU surface could not be created from the window.
    #[error("Failed to create GPU surface: {0}")]
    SurfaceCreation(#[from] wgpu::CreateSurfaceError),

    /// No GPU adapter compatible with the surface was found.
    #[error(
        "No suitable GPU adapter found. Ensure you have a Vulkan, DX12, or Metal compatible GPU \
         with up-to-date drivers."
    )]
    NoSuitableAdapter,

    /// The adapter could not provide a device with the requested features/limits.
    #[error("Failed to request GPU device: {0}")]
    DeviceRequest(#[from] wgpu::RequestDeviceError),
}

/// Owns all GPU state: instance, adapter, device, queue, and surface.
pub struct GpuContext {
    /// The wgpu instance.
    pub instance: wgpu::Instance,
    /// The selected GPU adapter.
    pub adapter: wgpu::Adapter,
    /// The logical GPU device.
    pub device: wgpu::Device,
    /// The command queue.
    pub queue: wgpu::Queue,
    /// The window surface.
    pub surface: wgpu::Surface<'static>,
    /// Current surface configuration.
    pub surface_config: wgpu::SurfaceConfiguration,
    /// The chosen surface texture format.
    pub surface_format: wgpu::TextureFormat,
}

impl GpuContext {
    /// Initialize the GPU asynchronously from a window handle.
    pub async fn new(window: Arc<Window>) -> Result<Self, GpuInitError> {
        // 1. Create instance with all backends
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        // 2. Create surface from window
        let surface = instance.create_surface(window.clone())?;

        // 3. Request adapter compatible with surface
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
            })
            .await
            .map_err(|_| GpuInitError::NoSuitableAdapter)?;

        let info = adapter.get_info();
        log::info!(
            "Selected GPU: {} ({:?}, {:?})",
            info.name,
            info.backend,
            info.device_type
        );

        // 4. Request device and queue
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("Nebula Device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
                experimental_features: wgpu::ExperimentalFeatures::default(),
            })
            .await?;

        device.on_uncaptured_error(Arc::new(|error| {
            log::error!("wgpu device error: {}", error);
            panic!("Fatal GPU error: {}", error);
        }));

        // 5. Configure the surface
        let surface_caps = surface.get_capabilities(&adapter);

        let surface_format = select_preferred_format(&surface_caps.formats);

        let size = window.inner_size();
        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
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
    pub fn resize(&mut self, new_width: u32, new_height: u32) {
        if new_width > 0 && new_height > 0 {
            self.surface_config.width = new_width;
            self.surface_config.height = new_height;
            self.surface.configure(&self.device, &self.surface_config);
        }
    }
}

/// Initialize the GPU synchronously using `pollster`.
pub fn init_gpu_blocking(window: Arc<Window>) -> Result<GpuContext, GpuInitError> {
    pollster::block_on(GpuContext::new(window))
}

/// Select the preferred surface format, preferring sRGB.
pub fn select_preferred_format(formats: &[wgpu::TextureFormat]) -> wgpu::TextureFormat {
    formats
        .iter()
        .copied()
        .find(|f| f.is_srgb())
        .unwrap_or(formats[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_surface_config_defaults() {
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: wgpu::TextureFormat::Bgra8UnormSrgb,
            width: 1280,
            height: 720,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        assert_eq!(config.present_mode, wgpu::PresentMode::Fifo);
        assert!(
            config
                .usage
                .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
        );
        assert_eq!(config.desired_maximum_frame_latency, 2);
        assert!(config.width >= 1);
        assert!(config.height >= 1);
    }

    #[test]
    fn test_preferred_format_selection() {
        let formats = [
            wgpu::TextureFormat::Bgra8Unorm,
            wgpu::TextureFormat::Bgra8UnormSrgb,
            wgpu::TextureFormat::Rgba8Unorm,
        ];
        assert_eq!(
            select_preferred_format(&formats),
            wgpu::TextureFormat::Bgra8UnormSrgb
        );
    }

    #[test]
    fn test_preferred_format_fallback() {
        let formats = [
            wgpu::TextureFormat::Bgra8Unorm,
            wgpu::TextureFormat::Rgba8Unorm,
        ];
        assert_eq!(
            select_preferred_format(&formats),
            wgpu::TextureFormat::Bgra8Unorm
        );
    }

    #[test]
    fn test_resize_updates_config() {
        let mut config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: wgpu::TextureFormat::Bgra8UnormSrgb,
            width: 1280,
            height: 720,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        // Simulate resize logic
        let (new_w, new_h) = (1920u32, 1080u32);
        if new_w > 0 && new_h > 0 {
            config.width = new_w;
            config.height = new_h;
        }
        assert_eq!(config.width, 1920);
        assert_eq!(config.height, 1080);
    }

    #[test]
    fn test_resize_ignores_zero() {
        let mut config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: wgpu::TextureFormat::Bgra8UnormSrgb,
            width: 1280,
            height: 720,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        // Zero width - should not update
        let (new_w, new_h) = (0u32, 1080u32);
        if new_w > 0 && new_h > 0 {
            config.width = new_w;
            config.height = new_h;
        }
        assert_eq!(config.width, 1280);
        assert_eq!(config.height, 720);

        // Zero height - should not update
        let (new_w, new_h) = (1920u32, 0u32);
        if new_w > 0 && new_h > 0 {
            config.width = new_w;
            config.height = new_h;
        }
        assert_eq!(config.width, 1280);
        assert_eq!(config.height, 720);
    }

    #[test]
    fn test_gpu_init_error_messages() {
        let err = GpuInitError::NoSuitableAdapter;
        let msg = err.to_string();
        assert!(msg.contains("No suitable GPU adapter found"));
        assert!(msg.contains("Vulkan"));
        assert!(msg.contains("DX12"));
        assert!(msg.contains("Metal"));
    }
}
