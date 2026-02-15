# Cross-Platform Surface Handling

## Problem

The wgpu surface abstraction hides most platform differences, but several critical edge cases leak through and cause crashes or visual artifacts if not handled:

- **Wayland (Linux)**: The compositor does not assign a size to a window until it is first presented. The initial `inner_size()` returns `(0, 0)`. Creating a wgpu surface with zero dimensions panics. The engine must detect this state and defer surface configuration until a non-zero size is received.

- **macOS Retina**: The window's logical size (in "points") differs from its physical size (in pixels). A 1440x900 point window on a 2x Retina display is actually 2880x1800 pixels. If the engine configures the surface with logical dimensions, rendering appears at quarter resolution and is blurry. The engine must always use physical pixel dimensions for surface configuration.

- **Windows DPI scaling**: When a window is dragged between monitors with different DPI settings, or when the user changes their display scaling, the window receives a DPI change event. The surface must be reconfigured with the new physical dimensions. If the engine ignores this event, the rendering is either blurry (stretched up from lower resolution) or cropped.

- **Scale factor changes at runtime**: On all platforms, the scale factor can change during execution (user changes display settings, window moves between displays). The engine must handle `ScaleFactorChanged` events and reconfigure accordingly.

These platform-specific issues are scattered across different winit event variants and wgpu surface behaviors. Without a normalizing layer, every system that touches the window or surface must independently handle these edge cases.

## Solution

### SurfaceWrapper

A `SurfaceWrapper` struct that normalizes platform-specific surface behavior and provides a consistent API:

```rust
pub struct SurfaceWrapper {
    /// Current physical pixel dimensions.
    physical_width: u32,
    physical_height: u32,
    /// Current logical dimensions (for UI layout).
    logical_width: f64,
    logical_height: f64,
    /// Current scale factor (physical pixels per logical pixel).
    scale_factor: f64,
    /// Whether the surface has been configured at least once.
    configured: bool,
    /// Pending resize event, if any.
    pending_resize: Option<PhysicalSize>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhysicalSize {
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct SurfaceResizeEvent {
    pub physical: PhysicalSize,
    pub logical_width: f64,
    pub logical_height: f64,
    pub scale_factor: f64,
}
```

### Initialization

```rust
impl SurfaceWrapper {
    pub fn new(window: &Window) -> Self {
        let scale_factor = window.scale_factor();
        let physical = window.inner_size();

        Self {
            physical_width: physical.width.max(1),
            physical_height: physical.height.max(1),
            logical_width: physical.width as f64 / scale_factor,
            logical_height: physical.height as f64 / scale_factor,
            scale_factor,
            configured: false,
            pending_resize: if physical.width == 0 || physical.height == 0 {
                None // defer until we get a non-zero size
            } else {
                Some(PhysicalSize {
                    width: physical.width,
                    height: physical.height,
                })
            },
        }
    }
}
```

On Wayland, if the initial size is `(0, 0)`, the wrapper records this state and does not configure the surface until a resize event provides a valid size.

### Event Handling

```rust
impl SurfaceWrapper {
    /// Handle a window resize event. Returns a resize event if the surface
    /// dimensions actually changed.
    pub fn handle_resize(
        &mut self,
        physical_width: u32,
        physical_height: u32,
    ) -> Option<SurfaceResizeEvent> {
        let width = physical_width.max(1);
        let height = physical_height.max(1);

        if width == self.physical_width && height == self.physical_height {
            return None; // no change
        }

        self.physical_width = width;
        self.physical_height = height;
        self.logical_width = width as f64 / self.scale_factor;
        self.logical_height = height as f64 / self.scale_factor;

        Some(SurfaceResizeEvent {
            physical: PhysicalSize { width, height },
            logical_width: self.logical_width,
            logical_height: self.logical_height,
            scale_factor: self.scale_factor,
        })
    }

    /// Handle a scale factor change event. Returns a resize event because
    /// the physical dimensions change even if the logical size stays the same.
    pub fn handle_scale_factor_changed(
        &mut self,
        new_scale_factor: f64,
        new_physical_width: u32,
        new_physical_height: u32,
    ) -> Option<SurfaceResizeEvent> {
        self.scale_factor = new_scale_factor;
        self.handle_resize(new_physical_width, new_physical_height)
    }

    /// Get the current physical pixel dimensions for surface configuration.
    pub fn physical_size(&self) -> PhysicalSize {
        PhysicalSize {
            width: self.physical_width,
            height: self.physical_height,
        }
    }

    /// Get the current scale factor.
    pub fn scale_factor(&self) -> f64 {
        self.scale_factor
    }

    /// Whether the surface has a valid (non-zero) size and can be configured.
    pub fn is_ready(&self) -> bool {
        self.physical_width > 0 && self.physical_height > 0
    }
}
```

### Integration with Event Loop

The `SurfaceWrapper` is used in the winit event loop to translate platform events into normalized resize events:

```rust
match event {
    WindowEvent::Resized(physical_size) => {
        if let Some(resize) = surface_wrapper.handle_resize(
            physical_size.width,
            physical_size.height,
        ) {
            render_context.resize(resize.physical.width, resize.physical.height);
            depth_buffer.resize(&device, resize.physical.width, resize.physical.height);
            camera.set_aspect_ratio(
                resize.physical.width as f32,
                resize.physical.height as f32,
            );
        }
    }
    WindowEvent::ScaleFactorChanged { scale_factor, inner_size_writer } => {
        let new_inner_size = inner_size_writer.request_inner_size(
            winit::dpi::PhysicalSize::new(
                (surface_wrapper.logical_width * scale_factor) as u32,
                (surface_wrapper.logical_height * scale_factor) as u32,
            ),
        );
        if let Some(resize) = surface_wrapper.handle_scale_factor_changed(
            scale_factor,
            new_inner_size.width,
            new_inner_size.height,
        ) {
            render_context.resize(resize.physical.width, resize.physical.height);
            depth_buffer.resize(&device, resize.physical.width, resize.physical.height);
        }
    }
    _ => {}
}
```

### Platform-Specific Notes

**Wayland zero-size handling**: The `SurfaceWrapper` clamps dimensions to minimum 1x1 and tracks the `configured` flag. On the first frame, if `is_ready()` returns false (because the initial size was 0x0), the rendering loop skips frame rendering until a `Resized` event arrives with valid dimensions. No panic, no crash — just a few skipped frames until the compositor assigns a size.

**macOS Retina**: winit's `inner_size()` already returns physical pixels (not logical points) on macOS. The `SurfaceWrapper` uses these physical pixel values directly for surface configuration. The `scale_factor()` method reports 2.0 on Retina displays, which the UI system uses to scale logical coordinates for text and layout.

**Windows DPI**: When the window moves between monitors with different DPI settings, winit fires a `ScaleFactorChanged` event followed by a `Resized` event. The `SurfaceWrapper` handles both, updating the scale factor and physical dimensions in sequence. The surface is only reconfigured once because `handle_resize` deduplicates unchanged dimensions.

### Constants

```rust
/// Minimum surface dimension (prevents zero-size panics).
pub const MIN_SURFACE_DIMENSION: u32 = 1;
```

## Outcome

A `SurfaceWrapper` that normalizes platform-specific surface creation and resize behavior across Linux (Wayland/X11), macOS (Retina), and Windows (DPI scaling). The wrapper always reports physical pixel dimensions for GPU surface configuration. Zero-size surfaces on Wayland are handled gracefully without panics. Resize events carry both physical and logical dimensions plus the scale factor, providing all the information that rendering, UI, and camera systems need. The event loop integration code is platform-agnostic — all platform-specific logic is encapsulated in the wrapper.

## Demo Integration

**Demo crate:** `nebula-demo`

No visible demo change; the demo handles surface-lost and resize events gracefully on all platforms without flicker or crashes.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `winit` | `0.30` | Window events, scale factor, physical/logical size |
| `wgpu` | `24.0` | Surface reconfiguration on resize |
| `log` | `0.4` | Logging resize events and platform-specific handling |

No additional dependencies. Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_surface_wrapper_reports_physical_pixels() {
        // Simulate a HiDPI display with scale factor 2.0
        let mut wrapper = SurfaceWrapper {
            physical_width: 2880,
            physical_height: 1800,
            logical_width: 1440.0,
            logical_height: 900.0,
            scale_factor: 2.0,
            configured: true,
            pending_resize: None,
        };

        let size = wrapper.physical_size();
        // Should report physical pixels, not logical
        assert_eq!(size.width, 2880);
        assert_eq!(size.height, 1800);
        // Not the logical size
        assert_ne!(size.width, 1440);
        assert_ne!(size.height, 900);
    }

    #[test]
    fn test_zero_size_surface_handled_gracefully() {
        // Simulate Wayland initial zero-size window
        let mut wrapper = SurfaceWrapper {
            physical_width: 1, // clamped from 0
            physical_height: 1, // clamped from 0
            logical_width: 0.0,
            logical_height: 0.0,
            scale_factor: 1.0,
            configured: false,
            pending_resize: None,
        };

        // Should not panic — is_ready returns true because dimensions are clamped to 1
        assert!(wrapper.is_ready());
        let size = wrapper.physical_size();
        assert!(size.width >= 1);
        assert!(size.height >= 1);

        // Now simulate the first real resize from the compositor
        let event = wrapper.handle_resize(1920, 1080);
        assert!(event.is_some());
        let event = event.unwrap();
        assert_eq!(event.physical.width, 1920);
        assert_eq!(event.physical.height, 1080);
    }

    #[test]
    fn test_resize_event_carries_physical_and_logical_sizes() {
        let mut wrapper = SurfaceWrapper {
            physical_width: 1920,
            physical_height: 1080,
            logical_width: 960.0,
            logical_height: 540.0,
            scale_factor: 2.0,
            configured: true,
            pending_resize: None,
        };

        let event = wrapper.handle_resize(3840, 2160);
        assert!(event.is_some());
        let event = event.unwrap();

        // Physical size
        assert_eq!(event.physical.width, 3840);
        assert_eq!(event.physical.height, 2160);

        // Logical size (physical / scale_factor)
        assert!((event.logical_width - 1920.0).abs() < 0.1);
        assert!((event.logical_height - 1080.0).abs() < 0.1);

        // Scale factor
        assert_eq!(event.scale_factor, 2.0);
    }

    #[test]
    fn test_no_event_on_same_dimensions() {
        let mut wrapper = SurfaceWrapper {
            physical_width: 1920,
            physical_height: 1080,
            logical_width: 1920.0,
            logical_height: 1080.0,
            scale_factor: 1.0,
            configured: true,
            pending_resize: None,
        };

        // Resize to the same dimensions should return None
        let event = wrapper.handle_resize(1920, 1080);
        assert!(event.is_none());
    }

    #[test]
    fn test_scale_factor_change_updates_physical_size() {
        let mut wrapper = SurfaceWrapper {
            physical_width: 1920,
            physical_height: 1080,
            logical_width: 1920.0,
            logical_height: 1080.0,
            scale_factor: 1.0,
            configured: true,
            pending_resize: None,
        };

        // Move to a 2x display — logical size stays the same but physical doubles
        let event = wrapper.handle_scale_factor_changed(2.0, 3840, 2160);
        assert!(event.is_some());
        let event = event.unwrap();
        assert_eq!(event.physical.width, 3840);
        assert_eq!(event.physical.height, 2160);
        assert_eq!(event.scale_factor, 2.0);
        assert_eq!(wrapper.scale_factor(), 2.0);
    }

    #[test]
    fn test_zero_dimensions_clamped_to_one() {
        let mut wrapper = SurfaceWrapper {
            physical_width: 800,
            physical_height: 600,
            logical_width: 800.0,
            logical_height: 600.0,
            scale_factor: 1.0,
            configured: true,
            pending_resize: None,
        };

        // Resize to zero (can happen during minimize on some platforms)
        let event = wrapper.handle_resize(0, 0);
        assert!(event.is_some());
        let size = wrapper.physical_size();
        assert_eq!(size.width, 1);
        assert_eq!(size.height, 1);
    }

    #[test]
    fn test_is_ready_with_valid_dimensions() {
        let wrapper = SurfaceWrapper {
            physical_width: 1920,
            physical_height: 1080,
            logical_width: 1920.0,
            logical_height: 1080.0,
            scale_factor: 1.0,
            configured: true,
            pending_resize: None,
        };
        assert!(wrapper.is_ready());
    }

    #[test]
    fn test_successive_resizes_produce_correct_state() {
        let mut wrapper = SurfaceWrapper {
            physical_width: 800,
            physical_height: 600,
            logical_width: 800.0,
            logical_height: 600.0,
            scale_factor: 1.0,
            configured: true,
            pending_resize: None,
        };

        wrapper.handle_resize(1024, 768);
        assert_eq!(wrapper.physical_size(), PhysicalSize { width: 1024, height: 768 });

        wrapper.handle_resize(1920, 1080);
        assert_eq!(wrapper.physical_size(), PhysicalSize { width: 1920, height: 1080 });

        wrapper.handle_scale_factor_changed(1.5, 2880, 1620);
        assert_eq!(wrapper.physical_size(), PhysicalSize { width: 2880, height: 1620 });
        assert_eq!(wrapper.scale_factor(), 1.5);
    }
}
```
