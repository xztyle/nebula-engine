# egui Integration

## Problem

The engine needs an immediate-mode GUI framework for all in-game UI: menus, HUDs, inventories, settings panels, chat, and debug overlays. Building a retained-mode GUI from scratch is a multi-month effort with limited payoff for a voxel game engine. egui provides a mature, batteries-included immediate-mode GUI that renders entirely on the GPU, but integrating it with the engine's existing wgpu 28.0 render pipeline and winit 0.30 event loop requires careful orchestration. egui's render output (textured triangles with clipping rectangles) must be composited as a post-process overlay on top of the 3D scene without disrupting depth testing, blending, or frame timing. Input events from winit must be forwarded to egui before the engine's own input systems consume them, and egui must be able to signal when it has captured keyboard or mouse focus so that gameplay input is suppressed.

Without a centralized integration point, every UI feature would need its own ad-hoc rendering and input plumbing, leading to duplicated code, inconsistent styling, and frame-rate regressions from redundant draw calls.

## Solution

Add an `EguiIntegration` struct to the `nebula_ui` crate that owns the egui context, the `egui-winit` platform adapter, and the `egui-wgpu` renderer, providing a single entry point for the entire UI pipeline.

```rust
pub struct EguiIntegration {
    pub ctx: egui::Context,
    platform: egui_winit::State,
    renderer: egui_wgpu::Renderer,
    screen_descriptor: egui_wgpu::ScreenDescriptor,
}
```

### Initialization

`EguiIntegration::new(device: &wgpu::Device, surface_format: wgpu::TextureFormat, window: &winit::window::Window) -> Self`:

1. **Create the egui context** with `egui::Context::default()`. Configure default visuals (dark theme) and default fonts. The context is `Send + Sync` and can be shared across systems.

2. **Create the winit platform adapter** with `egui_winit::State::new(ctx.clone(), ctx.viewport_id(), window, None, None)`. This adapter translates winit `WindowEvent` values into egui `RawInput`, handling keyboard, mouse, touch, and IME events. The scale factor is read from the window automatically.

3. **Create the wgpu renderer** with `egui_wgpu::Renderer::new(device, surface_format, None, 1, false)`. The `None` depth format means egui renders without depth testing (it is a 2D overlay). The MSAA sample count of `1` matches the engine's resolve target. The `false` flag disables dithering.

4. **Build the screen descriptor** from the window's inner size and scale factor:
   ```rust
   egui_wgpu::ScreenDescriptor {
       size_in_pixels: [width, height],
       pixels_per_point: window.scale_factor() as f32,
   }
   ```

### Per-Frame Flow

The UI pass executes after the 3D render pass completes but before the surface texture is presented:

1. **Input forwarding** -- In the winit event loop, before the engine's own `KeyboardState` / `MouseState` processing, call `self.platform.on_window_event(window, &event)`. This returns an `EventResponse` indicating whether egui wants the event. If `consumed` is true, the engine skips its own handling of that event.

2. **Begin frame** -- At the start of the UI pass, call `let raw_input = self.platform.take_egui_input(window)` followed by `self.ctx.begin_pass(raw_input)`. This gives egui the accumulated input and starts a new frame.

3. **UI construction** -- All UI systems (HUD, menus, inventory, chat, etc.) run their immediate-mode egui code against `&self.ctx` during this phase. Each system calls `egui::Window::new(...)`, `egui::TopBottomPanel`, `egui::CentralPanel`, etc.

4. **End frame** -- Call `let full_output = self.ctx.end_pass()`. This returns `egui::FullOutput` containing the paint jobs, platform output (cursor changes, clipboard, open URLs), and texture deltas.

5. **Handle platform output** -- Forward `full_output.platform_output` to `self.platform.handle_platform_output(window, platform_output)` to apply cursor icon changes and clipboard writes.

6. **Tessellate** -- Convert the paint jobs to triangles: `let paint_jobs = self.ctx.tessellate(full_output.shapes, full_output.pixels_per_point)`.

7. **Update textures** -- Apply texture deltas to the GPU: iterate `full_output.textures_delta.set` and call `self.renderer.update_buffers(device, queue, encoder, &paint_jobs, &screen_descriptor)` plus texture uploads via the renderer.

8. **Render** -- Begin a render pass on the surface texture view with `LoadOp::Load` (preserving the 3D scene underneath) and call `self.renderer.render(&mut render_pass, &paint_jobs, &screen_descriptor)`.

9. **Free textures** -- Iterate `full_output.textures_delta.free` and call `self.renderer.free_texture(id)` for each.

### Resize Handling

When `WindowEvent::Resized` fires, update the screen descriptor:
```rust
pub fn resize(&mut self, width: u32, height: u32, scale_factor: f32) {
    self.screen_descriptor.size_in_pixels = [width, height];
    self.screen_descriptor.pixels_per_point = scale_factor;
}
```

### Input Focus Query

Expose `wants_keyboard(&self) -> bool` and `wants_pointer(&self) -> bool` by reading `self.ctx.wants_keyboard_input()` and `self.ctx.wants_pointer_input()`. Game systems check these before processing gameplay input so that typing in a chat box does not also move the player.

## Outcome

An `egui_integration.rs` module in `crates/nebula_ui/src/` exporting `EguiIntegration` with `new()`, `on_event()`, `begin_frame()`, `end_frame_and_render()`, `resize()`, `wants_keyboard()`, and `wants_pointer()`. The struct is stored as an ECS resource and invoked from the main render loop. All subsequent UI stories build on this integration without touching wgpu or winit directly.

## Demo Integration

**Demo crate:** `nebula-demo`

egui renders as an overlay on top of the 3D scene. "Hello, Nebula!" text appears in the corner confirming integration.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `egui` | `0.31` | Immediate-mode GUI framework: context, layout, widgets, painting |
| `egui-wgpu` | `0.31` | Renders egui paint output using wgpu render passes |
| `egui-winit` | `0.31` | Translates winit 0.30 events into egui `RawInput` |
| `wgpu` | `28.0` | GPU device, queue, render pass, texture management |
| `winit` | `0.30` | Window events, scale factor, cursor control |
| `log` | `0.4` | Logging for initialization, texture upload errors, frame timing |

All dependencies are declared in `[workspace.dependencies]` and consumed via `{ workspace = true }` in the `nebula_ui` crate's `Cargo.toml`. Rust edition 2024.

## Unit Tests

| Test Function | Description | Assertion |
|---------------|-------------|-----------|
| `test_egui_context_initializes` | Construct an `egui::Context` via `EguiIntegration::new` and verify it is usable. | `ctx.style()` returns a valid `Style`; no panic occurs. |
| `test_input_event_forwarded_to_egui` | Feed a synthetic `WindowEvent::KeyboardInput` through `on_event()` and begin a frame. | `ctx.input(|i| i.events.len())` is greater than zero, confirming the event reached egui. |
| `test_egui_render_pass_no_panic` | Run a complete begin/UI/end/render cycle with an empty UI (no widgets). | No panic or wgpu validation error; the function returns successfully. |
| `test_egui_overlay_preserves_3d` | Render a 3D pass that clears to blue, then run the egui pass with `LoadOp::Load`. | The resulting surface texture still contains non-zero color data from the 3D pass (not cleared to black). |
| `test_frame_time_within_budget` | Run 100 frames of an empty egui pass and measure average duration. | Average egui pass time is below 1 ms on the test hardware, ensuring no significant regression. |
| `test_wants_keyboard_false_by_default` | Initialize egui and query `wants_keyboard()` without any text widget focused. | Returns `false`. |
| `test_wants_keyboard_true_when_text_edit_focused` | Begin a frame, create an `egui::TextEdit` and request focus, end the frame. | `wants_keyboard()` returns `true`. |
| `test_resize_updates_screen_descriptor` | Call `resize(1920, 1080, 2.0)` on the integration. | `screen_descriptor.size_in_pixels` equals `[1920, 1080]` and `pixels_per_point` equals `2.0`. |
