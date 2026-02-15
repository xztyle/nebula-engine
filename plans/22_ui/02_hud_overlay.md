# HUD Overlay

## Problem

During active gameplay the player needs constant, at-a-glance feedback about their status and surroundings: current health, selected item, orientation, and position. Without a heads-up display, the player must open separate menus to check this information, breaking the flow of exploration and combat. The HUD must be non-intrusive (transparent background, positioned at screen edges), resolution-independent (scales from 720p to 4K without overlap or pixel artifacts), and toggleable (hidden when a full-screen menu such as inventory or pause is open, so that UI layers do not stack confusingly). Because the engine uses 128-bit world coordinates and cubesphere-voxel planets, the coordinate debug readout must correctly format `i128` values and the compass must derive direction from the player's facing vector on a spherical surface, not a flat XZ plane.

## Solution

Implement a `HudOverlay` system in the `nebula_ui` crate that draws all HUD elements using egui panels and widgets each frame. The system reads ECS components (`Health`, `Inventory`, `Transform`, `PlayerCamera`) and the engine's `RenderContext` for screen dimensions.

### Layout

The HUD uses egui's `Area`, `TopBottomPanel`, and `Window` primitives to anchor elements to screen edges:

```
+--------------------------------------------------+
|  [Compass: N/S/E/W]            [Minimap]         |  <- top edge
|                                                    |
|                                                    |
|               [Crosshair]                          |  <- center (drawn by Story 07)
|                                                    |
|  [Health Bar]                                      |  <- bottom-left
|  [Hotbar: 1 2 3 4 5 6 7 8 9]                      |  <- bottom-center
|  [Debug Coords]                                    |  <- bottom-left, below health
+--------------------------------------------------+
```

### Components

1. **Health Bar** -- A horizontal progress bar anchored to the bottom-left corner. Reads `Health { current: f32, max: f32 }` from the player entity. The bar fill color transitions from green (>60%) to yellow (30--60%) to red (<30%). Background is semi-transparent dark gray. Width is 200 logical pixels; height is 20 logical pixels.

2. **Hotbar** -- A row of 9 slots anchored to the bottom-center. Each slot is a 48x48 logical-pixel rectangle with a 2px border. The currently selected slot has a highlighted border (gold). Slots display the item's icon texture (loaded via `egui::TextureId` from the asset system) and a stack count label in the bottom-right corner. The hotbar reads `Inventory` component data and the `selected_slot: usize` field from a `HotbarSelection` resource.

3. **Compass** -- A text label at the top-center showing the cardinal/intercardinal direction the player is facing. Computed from the player camera's forward vector projected onto the planet's local tangent plane (using the surface normal at the player's position). Displays one of: N, NE, E, SE, S, SW, W, NW.

4. **Minimap Placeholder** -- A 128x128 logical-pixel square in the top-right corner with a semi-transparent background and a "Minimap" label. This is a placeholder for a future top-down chunk view; for now it establishes the reserved screen region.

5. **Debug Coordinates** -- Toggled separately with F3 (debug mode). Displays the player's world position as three `i128` values (X, Y, Z), the current sector coordinates, the chunk address, and the current FPS. Formatted with thousands separators for readability. Anchored below the health bar.

### Visibility Toggle

The `HudOverlay` system checks a `UiState` resource:

```rust
pub struct UiState {
    pub hud_visible: bool,
    pub debug_visible: bool,
    pub active_menu: Option<MenuKind>,
}
```

When `active_menu` is `Some(...)`, the HUD is hidden entirely (menus take over the screen). When `active_menu` is `None`, the HUD is visible. The `debug_visible` flag independently controls the coordinate overlay.

### Scaling

All sizes are specified in egui logical pixels. egui multiplies by the platform's `pixels_per_point` (from Story 08) automatically. The hotbar and health bar widths are expressed as fractions of the screen width when the screen is narrow (below 1280 logical pixels) to prevent overflow.

## Outcome

A `hud_overlay.rs` module in `crates/nebula_ui/src/` exporting a `hud_overlay_system` function that runs each frame during the UI construction phase (between `begin_frame` and `end_frame_and_render` of `EguiIntegration` from Story 01). The system draws health bar, hotbar, compass, minimap placeholder, and debug coordinates using egui widgets.

## Demo Integration

**Demo crate:** `nebula-demo`

A HUD displays FPS top-left, world coordinates top-right, a minimap bottom-right, and health/stamina bars.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `egui` | `0.31` | Immediate-mode UI: panels, progress bars, labels, textures |
| `wgpu` | `28.0` | Texture handles for item icons (via `egui-wgpu` texture bridge) |
| `serde` | `1.0` | Serialize/Deserialize `UiState` and `HotbarSelection` for save/load |
| `log` | `0.4` | Logging for missing item textures and component lookup failures |

## Unit Tests

| Test Function | Description | Assertion |
|---------------|-------------|-----------|
| `test_health_bar_reflects_value` | Set `Health { current: 50.0, max: 100.0 }` and run the HUD system. | The health bar's fill fraction is `0.5`. The bar color is yellow (between 30% and 60%). |
| `test_health_bar_color_green_above_60` | Set `Health { current: 80.0, max: 100.0 }`. | Health bar color is green. |
| `test_health_bar_color_red_below_30` | Set `Health { current: 20.0, max: 100.0 }`. | Health bar color is red. |
| `test_hotbar_shows_selected_slot` | Set `HotbarSelection { selected_slot: 3 }`. | Slot index 3 has a highlighted border; all other slots have default borders. |
| `test_hotbar_displays_stack_count` | Place an item with stack count 42 in hotbar slot 0. | Slot 0 displays the label "42". |
| `test_compass_north` | Set the player camera's forward vector to the local north direction on the planet surface. | Compass label reads "N". |
| `test_crosshair_centered` | Read the crosshair's screen position after layout. | Position equals `(screen_width / 2.0, screen_height / 2.0)` within 1 logical pixel tolerance. |
| `test_hud_hidden_when_menu_open` | Set `UiState { active_menu: Some(MenuKind::MainMenu), .. }` and run the HUD system. | No HUD widgets are drawn (egui's output contains zero HUD-related shapes). |
| `test_hud_visible_when_no_menu` | Set `UiState { active_menu: None, hud_visible: true, .. }`. | HUD widgets are present in egui output. |
| `test_debug_coords_display_i128` | Set player position to `(i128::MAX, 0, -42)` and enable debug mode. | Debug overlay contains the formatted string for `i128::MAX` with thousands separators. |
