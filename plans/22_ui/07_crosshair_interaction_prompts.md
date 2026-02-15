# Crosshair & Interaction Prompts

## Problem

In a first-person voxel game, the player needs constant visual feedback about what they are aiming at. A center-screen crosshair provides targeting reference, but without additional context the player cannot distinguish between an empty block face, a minable resource, an interactable object (chest, crafting table, NPC), or a hostile entity. The crosshair must change color to convey target type at a glance, and a text prompt must appear to indicate available actions and their keybindings. This requires integration with the voxel raycasting system (Epic 17) to determine the targeted block or entity each frame, and with the input action mapping (Epic 15) to display the correct key label for each action.

## Solution

Implement a `CrosshairPrompt` system in the `nebula_ui` crate that draws a crosshair and context-sensitive interaction prompts using egui. The system reads the result of a per-frame raycast from the player camera.

### Raycast Integration

Each frame, the physics/interaction system (Epic 17) performs a voxel raycast from the player camera's position along the forward vector, with a configurable max distance (default: 8 blocks). The result is stored in a `CrosshairTarget` ECS resource:

```rust
pub enum TargetKind {
    None,
    Block { block_type: VoxelType, face: BlockFace },
    Interactable { entity: Entity, name: String, action: InteractionAction },
    Enemy { entity: Entity, name: String },
}

pub enum InteractionAction {
    Open,      // chest, crafting table
    Talk,      // NPC
    Activate,  // lever, button
    Mine,      // any block (default)
}

pub struct CrosshairTarget {
    pub kind: TargetKind,
    pub distance: f32,
}
```

### Crosshair Rendering

The crosshair is drawn at the exact center of the screen using `egui::Area::new("crosshair")` with `anchor(Align2::CENTER_CENTER, [0.0, 0.0])`:

```rust
let center = egui::pos2(screen_width / 2.0, screen_height / 2.0);
```

The crosshair shape is four small lines forming a plus sign, with a 2px gap at the center to avoid obscuring the exact target point. Each arm is 8 logical pixels long and 2 logical pixels thick. The crosshair is drawn using `egui::Painter::line_segment` calls.

### Crosshair Color

The crosshair color depends on `CrosshairTarget::kind`:

| Target Kind | Color | Hex |
|-------------|-------|-----|
| `None` | White | `#FFFFFF` |
| `Block` (standard) | White | `#FFFFFF` |
| `Interactable` | Yellow | `#FFD700` |
| `Enemy` | Red | `#FF4444` |

A subtle 1px black outline (shadow) behind the crosshair ensures visibility against both light and dark backgrounds.

### Interaction Prompt

When the target is `Interactable` or `Block` (minable), a text prompt appears just below the crosshair (offset by 24 logical pixels downward). The prompt shows the keybinding and the action:

| Target | Prompt Text |
|--------|-------------|
| `Interactable { action: Open, .. }` | `"[E] Open"` |
| `Interactable { action: Talk, .. }` | `"[E] Talk"` |
| `Interactable { action: Activate, .. }` | `"[E] Activate"` |
| `Block { .. }` | `"[LMB] Mine"` |
| `Enemy { .. }` | `"[LMB] Attack"` |

The keybinding label (`E`, `LMB`) is resolved dynamically from the `InputMap` (Epic 15, Story 04) so that if the player has rebound the Interact action to a different key, the prompt reflects the new binding.

The prompt text uses `egui::Label` with a semi-transparent black background for readability, centered horizontally below the crosshair. If the target has a name (e.g., "Iron Chest", "Zombie"), the name is displayed above the action prompt in a smaller font.

### Performance

The raycast result is computed once per frame by the physics system and cached in the `CrosshairTarget` resource. The crosshair UI system only reads the resource; it does not perform raycasting itself. Drawing four line segments and one or two text labels per frame has negligible performance impact.

### HUD Interaction

The crosshair is drawn as part of the HUD layer. When `UiState::active_menu` is `Some(...)` (e.g., inventory is open), the crosshair is hidden along with the rest of the HUD (Story 02). When the HUD is visible, the crosshair is always drawn.

## Outcome

A `crosshair_prompt.rs` module in `crates/nebula_ui/src/` exporting `crosshair_prompt_system`, `CrosshairTarget`, `TargetKind`, and `InteractionAction`. The system draws the crosshair and prompts during the UI construction phase each frame when the HUD is visible.

## Demo Integration

**Demo crate:** `nebula-demo`

A crosshair sits at screen center. Targeting a voxel shows a contextual prompt like `Stone [LMB: Break]`.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `egui` | `0.31` | Immediate-mode UI: painter for line segments, labels for prompts, areas for positioning |
| `wgpu` | `28.0` | Referenced indirectly for coordinate system alignment between 3D and 2D |
| `log` | `0.4` | Logging target changes for debugging interaction issues |

## Unit Tests

| Test Function | Description | Assertion |
|---------------|-------------|-----------|
| `test_crosshair_centered` | Run the crosshair system on an 800x600 screen. | Crosshair center position equals `(400.0, 300.0)` within 0.5 logical pixel tolerance. |
| `test_crosshair_centered_after_resize` | Resize to 1920x1080 and run the crosshair system. | Crosshair center position equals `(960.0, 540.0)` within 0.5 logical pixel tolerance. |
| `test_prompt_appears_on_interactable` | Set `CrosshairTarget::kind` to `Interactable { action: Open, name: "Chest" }`. | egui output contains label text including "[E] Open" and "Chest". |
| `test_prompt_disappears_off_interactable` | Set `CrosshairTarget::kind` to `None`. | egui output contains no prompt labels (only crosshair lines). |
| `test_color_white_on_nothing` | Set `CrosshairTarget::kind` to `None`. | Crosshair line color is `Color32::WHITE`. |
| `test_color_yellow_on_interactable` | Set `CrosshairTarget::kind` to `Interactable { .. }`. | Crosshair line color is `Color32::from_rgb(255, 215, 0)`. |
| `test_color_red_on_enemy` | Set `CrosshairTarget::kind` to `Enemy { .. }`. | Crosshair line color is `Color32::from_rgb(255, 68, 68)`. |
| `test_raycast_updates_each_frame` | Set target to `Block` on frame 1, then `Enemy` on frame 2. | Frame 1: crosshair is white and prompt shows "[LMB] Mine". Frame 2: crosshair is red and prompt shows "[LMB] Attack". |
| `test_prompt_uses_rebound_key` | Rebind `Interact` from `KeyE` to `KeyF` in the `InputMap`. Set target to `Interactable { action: Open }`. | Prompt text reads "[F] Open" instead of "[E] Open". |
| `test_crosshair_hidden_when_menu_open` | Set `UiState::active_menu` to `Some(MenuKind::Inventory)`. | No crosshair or prompt is drawn in the egui output. |
| `test_mine_prompt_on_block` | Set `CrosshairTarget::kind` to `Block { block_type: VoxelType::Stone, .. }`. | Prompt text reads "[LMB] Mine". |
