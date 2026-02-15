# DPI Scaling

## Problem

Modern displays range from 96 DPI standard monitors to 220+ DPI Retina/HiDPI panels, and the same application window can move between monitors with different scale factors on multi-monitor setups. The engine's 3D renderer operates in physical pixels (the actual framebuffer resolution), but egui operates in logical pixels (points) that are multiplied by a scale factor to produce crisp output. If the UI ignores the platform's scale factor, text appears tiny and unreadable on HiDPI screens, or enormous and blurry on standard screens. Additionally, some players prefer to override the system scale -- making UI elements larger for accessibility or smaller for more screen real estate. The engine must handle automatic DPI detection from winit, feed the correct scale factor to egui, and allow the user to apply a custom multiplier on top, all without causing layout breakage (overlapping elements, off-screen widgets, or misaligned text).

## Solution

Implement a `DpiScaling` resource and integration layer in the `nebula_ui` crate that manages the relationship between the platform scale factor, the user's custom UI scale preference, and the effective `pixels_per_point` value passed to egui.

### Scale Factor Hierarchy

The effective scale is computed as:

```rust
effective_pixels_per_point = platform_scale_factor * user_scale_override
```

- `platform_scale_factor: f32` -- Obtained from `winit::window::Window::scale_factor()`, which returns an `f64` (cast to `f32` for egui). On macOS Retina this is typically `2.0`. On Windows with 150% scaling this is `1.5`. On standard Linux X11/Wayland this is `1.0` unless the user has configured HiDPI.

- `user_scale_override: f32` -- A multiplier from the settings menu (Story 04), ranging from `0.5` to `2.0` in steps of `0.1`. Default is `1.0`. This allows a player on a 1080p monitor to shrink the UI (0.7x) for more viewport, or a player on a 4K monitor to enlarge it (1.5x) for readability.

### DpiScaling Resource

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DpiScaling {
    pub platform_scale_factor: f32,
    pub user_scale_override: f32,
}

impl DpiScaling {
    pub fn effective(&self) -> f32 {
        (self.platform_scale_factor * self.user_scale_override).clamp(0.5, 4.0)
    }
}
```

The `clamp(0.5, 4.0)` prevents extreme values: a user_scale_override of `0.5` on a platform factor of `1.0` gives `0.5` (minimum usable); a user_scale_override of `2.0` on a Retina `2.0` gives `4.0` (maximum before layout breaks).

### Integration Points

1. **Window creation** -- When the window is created (Epic 01), the initial `platform_scale_factor` is read from `window.scale_factor()` and stored in the `DpiScaling` resource.

2. **Scale factor change event** -- winit emits `WindowEvent::ScaleFactorChanged { scale_factor, .. }` when the window moves to a monitor with a different DPI. The event handler updates `DpiScaling::platform_scale_factor` and calls `EguiIntegration::resize()` with the new effective scale.

3. **egui context** -- Each frame, before `begin_frame`, the effective `pixels_per_point` is applied to the egui screen descriptor:
   ```rust
   integration.screen_descriptor.pixels_per_point = dpi_scaling.effective();
   ```
   egui uses this value to convert logical coordinates to physical pixels in its tessellation step.

4. **egui context style** -- The `egui::Context::set_pixels_per_point()` method is also called to ensure egui's internal layout calculations use the correct scale:
   ```rust
   ctx.set_pixels_per_point(dpi_scaling.effective());
   ```

5. **Settings menu** -- The UI Scale slider in the settings menu (Story 04) modifies `DpiScaling::user_scale_override`. The change is applied immediately (no restart required) because only the `pixels_per_point` value changes; no GPU resources need reallocation.

6. **Framebuffer** -- The wgpu surface and render targets use the physical pixel dimensions from `window.inner_size()` (which already accounts for the platform scale factor). No additional scaling is needed on the rendering side. The 3D scene renders at the full physical resolution.

### Font Rendering

egui rasterizes fonts at the effective `pixels_per_point`. At higher scale factors, egui generates denser glyph atlases with more detail, ensuring crisp text. The font atlas texture is rebuilt when `pixels_per_point` changes significantly (egui handles this automatically via its texture delta system). The default font sizes in logical pixels (e.g., 14pt body text, 20pt heading) remain constant across DPI; only the physical pixel count per glyph changes.

### Layout Robustness

To prevent overlap at high effective scales (where UI elements become larger in logical pixels relative to the screen):

- The HUD (Story 02) uses `available_width()` and `available_height()` queries instead of hardcoded positions.
- The inventory grid (Story 05) constrains its window size to `min(preferred_size, screen_size * 0.9)`.
- The settings menu (Story 04) uses a scrollable area when content exceeds the screen height.
- Font sizes are not scaled manually; they are expressed in logical pixels and egui handles physical scaling.

## Outcome

A `dpi_scaling.rs` module in `crates/nebula_ui/src/` exporting `DpiScaling` with `effective()`, plus integration hooks in `EguiIntegration` to apply the scale factor each frame. The `DpiScaling` resource is stored in the ECS world and updated from winit events and user settings.

## Demo Integration

**Demo crate:** `nebula-demo`

On HiDPI displays, all UI elements are crisp and correctly sized. Scaling factor is detected automatically.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `egui` | `0.31` | `set_pixels_per_point()`, font atlas regeneration, logical pixel layout |
| `egui-wgpu` | `0.31` | `ScreenDescriptor::pixels_per_point` for tessellation scaling |
| `winit` | `0.30` | `Window::scale_factor()`, `WindowEvent::ScaleFactorChanged` |
| `serde` | `1.0` | Serialize/Deserialize `DpiScaling` for config persistence |
| `log` | `0.4` | Logging scale factor changes and effective scale computations |

## Unit Tests

| Test Function | Description | Assertion |
|---------------|-------------|-----------|
| `test_effective_scale_default` | Construct `DpiScaling { platform_scale_factor: 1.0, user_scale_override: 1.0 }`. | `effective()` returns `1.0`. |
| `test_effective_scale_retina` | Construct `DpiScaling { platform_scale_factor: 2.0, user_scale_override: 1.0 }`. | `effective()` returns `2.0`. |
| `test_effective_scale_custom_override` | Construct `DpiScaling { platform_scale_factor: 1.0, user_scale_override: 1.5 }`. | `effective()` returns `1.5`. |
| `test_effective_scale_combined` | Construct `DpiScaling { platform_scale_factor: 2.0, user_scale_override: 1.5 }`. | `effective()` returns `3.0`. |
| `test_effective_scale_clamped_low` | Construct `DpiScaling { platform_scale_factor: 0.5, user_scale_override: 0.5 }`. | `effective()` returns `0.5` (clamped, not `0.25`). |
| `test_effective_scale_clamped_high` | Construct `DpiScaling { platform_scale_factor: 2.0, user_scale_override: 2.5 }`. | `effective()` returns `4.0` (clamped, not `5.0`). |
| `test_ui_renders_crisply_at_2x` | Set effective scale to `2.0`, render a text label "Hello", and verify font atlas contains 2x glyph resolution. | The egui font atlas texture dimensions are larger than at scale `1.0` for the same text. |
| `test_custom_scale_applies_to_egui_context` | Set `user_scale_override` to `1.5`, call the integration update. | `ctx.pixels_per_point()` returns `1.5` (assuming platform factor is `1.0`). |
| `test_text_readable_at_all_scales` | Render the text "Settings" at scales `0.5`, `1.0`, `1.5`, `2.0`. | At all scales, the computed text bounding box height is proportional to the scale factor (no zero-height or negative-height output). |
| `test_ui_elements_no_overlap_at_high_scale` | Set effective scale to `3.0`, render the HUD with health bar and hotbar on an 800x600 screen. | Health bar and hotbar bounding rectangles do not intersect. |
| `test_scale_factor_from_winit_detected` | Simulate `WindowEvent::ScaleFactorChanged { scale_factor: 1.5 }`. | `DpiScaling::platform_scale_factor` updates to `1.5`. |
| `test_scale_change_no_restart` | Change `user_scale_override` from `1.0` to `1.5` at runtime. | The change takes effect on the next frame without requiring an application restart. |
