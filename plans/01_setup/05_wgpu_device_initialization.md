# wgpu Device Initialization

## Problem

Before rendering anything -- a single triangle, a voxel chunk, a debug overlay -- the engine needs a GPU device and command queue. wgpu provides a cross-platform abstraction over Vulkan (Linux/Windows), DX12 (Windows), and Metal (macOS), but its initialization is a multi-step async process that can fail at several points:

- **No suitable adapter** — The system may have no GPU, or the available GPU may not support the required features.
- **Device request rejected** — The adapter may exist but fail to provide a device with the requested limits (e.g., max texture size, max buffer size).
- **Surface incompatibility** — The chosen adapter may not be compatible with the window surface on the current platform.

Each failure mode requires a distinct, human-readable error message so developers and end users can diagnose the problem. The GPU handles (Instance, Adapter, Device, Queue, Surface, SurfaceConfig) must be held for the lifetime of the application and made accessible to all rendering subsystems.

## Solution

### GpuContext Struct

Create a `GpuContext` struct that owns all GPU state:

```rust
pub struct GpuContext {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface: wgpu::Surface<'static>,
    pub surface_config: wgpu::SurfaceConfiguration,
    pub surface_format: wgpu::TextureFormat,
}
```

### Initialization Sequence

```rust
impl GpuContext {
    pub async fn new(window: Arc<Window>) -> Result<Self, GpuInitError> {
        // 1. Create instance with all backends
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        // 2. Create surface from window
        let surface = instance.create_surface(window.clone())
            .map_err(GpuInitError::SurfaceCreation)?;

        // 3. Request adapter (GPU) compatible with our surface
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
            })
            .await
            .ok_or(GpuInitError::NoSuitableAdapter)?;

        // Log adapter info for diagnostics
        let info = adapter.get_info();
        log::info!(
            "Selected GPU: {} ({:?}, {:?})",
            info.name,
            info.backend,
            info.device_type
        );

        // 4. Request device and queue
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("Nebula Device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::Performance,
                },
                None, // No trace path
            )
            .await
            .map_err(GpuInitError::DeviceRequest)?;

        // Set up error handling on the device
        device.on_uncaptured_error(Box::new(|error| {
            log::error!("wgpu device error: {}", error);
            panic!("Fatal GPU error: {}", error);
        }));

        // 5. Configure the surface
        let surface_caps = surface.get_capabilities(&adapter);

        // Prefer sRGB format for correct color handling
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        let size = window.inner_size();
        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo, // VSync
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
```

### Error Type

```rust
#[derive(Debug, thiserror::Error)]
pub enum GpuInitError {
    #[error("Failed to create GPU surface: {0}")]
    SurfaceCreation(wgpu::CreateSurfaceError),

    #[error(
        "No suitable GPU adapter found. Ensure you have a Vulkan, DX12, or Metal compatible GPU \
         with up-to-date drivers."
    )]
    NoSuitableAdapter,

    #[error("Failed to request GPU device: {0}")]
    DeviceRequest(wgpu::RequestDeviceError),
}
```

### Blocking Initialization

Since wgpu's initialization is async but the engine's startup is synchronous (the event loop is not yet running), use `pollster` to block on the async init:

```rust
pub fn init_gpu_blocking(window: Arc<Window>) -> Result<GpuContext, GpuInitError> {
    pollster::block_on(GpuContext::new(window))
}
```

### Key Design Decisions

1. **`Backends::all()`** — Let wgpu choose the best backend for the platform. On Linux it will select Vulkan, on Windows DX12 (falling back to Vulkan), on macOS Metal. This keeps initialization simple while covering all platforms.

2. **`PowerPreference::HighPerformance`** — Prefer discrete GPUs over integrated. On laptops with dual GPUs, this ensures the dedicated GPU is used for rendering.

3. **`PresentMode::Fifo`** — This is vsync, which is the safest default. It prevents tearing and limits frame rate to the display refresh rate. A `PresentMode::Mailbox` option (uncapped FPS with no tearing) can be added as a config option later.

4. **sRGB format preference** — Selecting an sRGB surface format ensures that the gamma correction pipeline is correct from the start. Rendering in linear space and presenting in sRGB is the standard approach for PBR rendering.

5. **`desired_maximum_frame_latency: 2`** — Allows up to 2 frames of latency in the presentation queue. This is the standard default that balances input latency with smooth frame pacing.

6. **Width/height clamped to minimum 1** — wgpu panics if the surface is configured with a width or height of 0 (which can happen during window minimization). Clamping to 1 prevents this.

### Integration with Window Events

When the window is resized (see `04_spawn_window.md`), the `GpuContext::resize()` method must be called:

```rust
WindowEvent::Resized(new_size) => {
    gpu_context.resize(new_size.width, new_size.height);
}
```

## Outcome

On application start, the GPU is initialized and ready for draw calls. The `GpuContext` struct holds all GPU handles for the lifetime of the app. The selected GPU name and backend are logged for diagnostics. If initialization fails at any step, a clear error message is displayed indicating what went wrong and what the user can do about it (update drivers, check GPU compatibility). The surface is configured with vsync and sRGB format, ready for PBR rendering.

## Demo Integration

**Demo crate:** `nebula-demo`

The window background is GPU-cleared to deep space blue `(0.02, 0.02, 0.08)`. The console logs which GPU adapter was selected and which backend is in use.

## Crates & Dependencies

- **`wgpu = "28.0"`** — Cross-platform GPU abstraction over Vulkan, DX12, and Metal. Provides the Instance, Adapter, Device, Queue, and Surface types that form the rendering foundation.
- **`pollster = "0.4"`** — Minimal async runtime for blocking on a single future. Used to run wgpu's async initialization from synchronous startup code. Chosen over `tokio` or `async-std` because we only need to block on a single future, not run a full async runtime.
- **`thiserror = "2"`** — Derive macro for `std::error::Error` implementations. Used for the `GpuInitError` type to provide clean, formatted error messages.

## Unit Tests

- **`test_gpu_context_struct_fields`** — Verify that the `GpuContext` struct has all required public fields: `instance`, `adapter`, `device`, `queue`, `surface`, `surface_config`, and `surface_format`. This is a compile-time test (if the struct definition changes, code using these fields will fail to compile).

- **`test_surface_config_defaults`** — Construct a `SurfaceConfiguration` with the engine's default values and verify:
  - `present_mode` is `PresentMode::Fifo`
  - `usage` includes `TextureUsages::RENDER_ATTACHMENT`
  - `desired_maximum_frame_latency` is 2
  - `width` and `height` are both >= 1

- **`test_preferred_format_selection`** — Given a mock list of surface formats (e.g., `[Bgra8Unorm, Bgra8UnormSrgb, Rgba8Unorm]`), verify that the format selection logic picks the sRGB variant (`Bgra8UnormSrgb`). If no sRGB format is available, verify it falls back to the first format in the list.

- **`test_resize_updates_config`** — Create a `surface_config` with initial dimensions 1280x720, call the resize logic with 1920x1080, and verify the config now has the new dimensions.

- **`test_resize_ignores_zero`** — Call resize with width=0 or height=0 and verify the config retains its previous dimensions. This prevents the wgpu panic on zero-sized surfaces.

- **`test_gpu_init_error_messages`** — Construct each variant of `GpuInitError` and verify that `to_string()` produces a human-readable message containing actionable information.

- **`test_adapter_logging`** — (Integration test, requires GPU) Initialize a real adapter and verify that `adapter.get_info()` returns a non-empty name and a valid backend. This test is skipped in CI environments without GPU access.
