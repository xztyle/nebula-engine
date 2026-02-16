//! Cross-platform surface handling that normalizes platform-specific behavior.
//!
//! Handles Wayland zero-size windows, macOS Retina scaling, and Windows DPI
//! changes by providing a consistent API for surface dimensions.

/// Minimum surface dimension (prevents zero-size panics).
pub const MIN_SURFACE_DIMENSION: u32 = 1;

/// Physical pixel dimensions of a surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhysicalSize {
    /// Width in physical pixels.
    pub width: u32,
    /// Height in physical pixels.
    pub height: u32,
}

/// Event produced when the surface dimensions or scale factor change.
#[derive(Clone, Copy, Debug)]
pub struct SurfaceResizeEvent {
    /// New physical pixel dimensions.
    pub physical: PhysicalSize,
    /// New logical width (physical / scale_factor).
    pub logical_width: f64,
    /// New logical height (physical / scale_factor).
    pub logical_height: f64,
    /// Current scale factor.
    pub scale_factor: f64,
}

/// Normalizes platform-specific surface behavior across Linux (Wayland/X11),
/// macOS (Retina), and Windows (DPI scaling).
///
/// Always reports physical pixel dimensions for GPU surface configuration.
/// Zero-size surfaces (common on Wayland) are clamped to 1×1 to prevent panics.
pub struct SurfaceWrapper {
    /// Current physical pixel width (clamped to >= 1).
    physical_width: u32,
    /// Current physical pixel height (clamped to >= 1).
    physical_height: u32,
    /// Current logical width (for UI layout).
    logical_width: f64,
    /// Current logical height (for UI layout).
    logical_height: f64,
    /// Current scale factor (physical pixels per logical pixel).
    scale_factor: f64,
    /// Whether the surface has been configured at least once with valid dimensions.
    configured: bool,
}

impl SurfaceWrapper {
    /// Creates a new `SurfaceWrapper` from initial physical dimensions and scale factor.
    ///
    /// If the initial dimensions are zero (common on Wayland before the compositor
    /// assigns a size), they are clamped to 1 and the wrapper is marked as unconfigured.
    pub fn new(physical_width: u32, physical_height: u32, scale_factor: f64) -> Self {
        let has_valid_size = physical_width > 0 && physical_height > 0;
        let width = physical_width.max(MIN_SURFACE_DIMENSION);
        let height = physical_height.max(MIN_SURFACE_DIMENSION);

        Self {
            physical_width: width,
            physical_height: height,
            logical_width: width as f64 / scale_factor,
            logical_height: height as f64 / scale_factor,
            scale_factor,
            configured: has_valid_size,
        }
    }

    /// Handle a window resize event. Returns a resize event if the surface
    /// dimensions actually changed.
    ///
    /// Dimensions are clamped to a minimum of 1×1 to prevent wgpu panics.
    pub fn handle_resize(
        &mut self,
        physical_width: u32,
        physical_height: u32,
    ) -> Option<SurfaceResizeEvent> {
        let width = physical_width.max(MIN_SURFACE_DIMENSION);
        let height = physical_height.max(MIN_SURFACE_DIMENSION);

        if width == self.physical_width && height == self.physical_height {
            return None;
        }

        self.physical_width = width;
        self.physical_height = height;
        self.logical_width = width as f64 / self.scale_factor;
        self.logical_height = height as f64 / self.scale_factor;
        self.configured = true;

        Some(SurfaceResizeEvent {
            physical: PhysicalSize { width, height },
            logical_width: self.logical_width,
            logical_height: self.logical_height,
            scale_factor: self.scale_factor,
        })
    }

    /// Handle a scale factor change event. Returns a resize event because
    /// the physical dimensions change even if the logical size stays the same.
    ///
    /// This is triggered when a window moves between displays with different
    /// DPI settings or when the user changes display scaling.
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

    /// Get the current physical width in pixels.
    pub fn physical_width(&self) -> u32 {
        self.physical_width
    }

    /// Get the current physical height in pixels.
    pub fn physical_height(&self) -> u32 {
        self.physical_height
    }

    /// Get the current logical width (physical / scale_factor).
    pub fn logical_width(&self) -> f64 {
        self.logical_width
    }

    /// Get the current logical height (physical / scale_factor).
    pub fn logical_height(&self) -> f64 {
        self.logical_height
    }

    /// Get the current scale factor.
    pub fn scale_factor(&self) -> f64 {
        self.scale_factor
    }

    /// Whether the surface has a valid (non-zero original) size and can be configured.
    pub fn is_ready(&self) -> bool {
        self.physical_width > 0 && self.physical_height > 0
    }

    /// Whether the surface has been configured at least once with valid dimensions.
    pub fn is_configured(&self) -> bool {
        self.configured
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_surface_wrapper_reports_physical_pixels() {
        let wrapper = SurfaceWrapper {
            physical_width: 2880,
            physical_height: 1800,
            logical_width: 1440.0,
            logical_height: 900.0,
            scale_factor: 2.0,
            configured: true,
        };

        let size = wrapper.physical_size();
        assert_eq!(size.width, 2880);
        assert_eq!(size.height, 1800);
        assert_ne!(size.width, 1440);
        assert_ne!(size.height, 900);
    }

    #[test]
    fn test_zero_size_surface_handled_gracefully() {
        let mut wrapper = SurfaceWrapper::new(0, 0, 1.0);

        // Clamped to 1x1, still "ready" but not "configured"
        assert!(wrapper.is_ready());
        assert!(!wrapper.is_configured());
        let size = wrapper.physical_size();
        assert!(size.width >= 1);
        assert!(size.height >= 1);

        // Now simulate the first real resize from the compositor
        let event = wrapper.handle_resize(1920, 1080);
        assert!(event.is_some());
        let event = event.unwrap();
        assert_eq!(event.physical.width, 1920);
        assert_eq!(event.physical.height, 1080);
        assert!(wrapper.is_configured());
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
        };

        let event = wrapper.handle_resize(3840, 2160);
        assert!(event.is_some());
        let event = event.unwrap();

        assert_eq!(event.physical.width, 3840);
        assert_eq!(event.physical.height, 2160);
        assert!((event.logical_width - 1920.0).abs() < 0.1);
        assert!((event.logical_height - 1080.0).abs() < 0.1);
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
        };

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
        };

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
        };

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
        };

        wrapper.handle_resize(1024, 768);
        assert_eq!(
            wrapper.physical_size(),
            PhysicalSize {
                width: 1024,
                height: 768
            }
        );

        wrapper.handle_resize(1920, 1080);
        assert_eq!(
            wrapper.physical_size(),
            PhysicalSize {
                width: 1920,
                height: 1080
            }
        );

        wrapper.handle_scale_factor_changed(1.5, 2880, 1620);
        assert_eq!(
            wrapper.physical_size(),
            PhysicalSize {
                width: 2880,
                height: 1620
            }
        );
        assert_eq!(wrapper.scale_factor(), 1.5);
    }

    #[test]
    fn test_new_with_valid_dimensions() {
        let wrapper = SurfaceWrapper::new(1920, 1080, 2.0);
        assert_eq!(wrapper.physical_width(), 1920);
        assert_eq!(wrapper.physical_height(), 1080);
        assert!((wrapper.logical_width() - 960.0).abs() < 0.1);
        assert!((wrapper.logical_height() - 540.0).abs() < 0.1);
        assert_eq!(wrapper.scale_factor(), 2.0);
        assert!(wrapper.is_configured());
    }
}
