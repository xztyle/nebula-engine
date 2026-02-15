# Inventory UI

## Problem

The player needs a way to view, organize, and manage the items they have collected. A grid-based inventory is the standard interface for voxel games, but implementing it in an immediate-mode GUI requires careful handling of drag-and-drop state, click interactions, stack arithmetic, and synchronization with the ECS-backed inventory data. The inventory must support drag-and-drop between slots, stack splitting, context menus, and a hotbar that reflects the bottom row of the inventory grid. The UI must toggle open and closed with a single key press, and while open, gameplay input (movement, camera) must be suppressed so the cursor is free to interact with slots.

## Solution

Implement an `InventoryUi` system in the `nebula_ui` crate that draws a grid-based inventory window using egui. The system reads and writes an `Inventory` ECS component on the player entity.

### Data Model

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Inventory {
    pub slots: Vec<Option<ItemStack>>,
    pub rows: usize,
    pub cols: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemStack {
    pub item_id: u32,
    pub count: u32,
    pub max_stack: u32,
}
```

The default inventory is 4 rows by 9 columns (36 slots). The bottom row (slots 27--35) maps to the hotbar (9 slots visible in the HUD from Story 02). The `Inventory` struct lives as a component on the player entity in the ECS world.

### Grid Rendering

The inventory window is drawn with `egui::Window::new("Inventory")` set to a fixed size and centered on screen. Inside, a `egui::Grid` lays out slots in `rows x cols` arrangement:

```
+--------------------------------------------------+
|  Inventory                                   [X]  |
+--------------------------------------------------+
|  [  ] [  ] [  ] [  ] [  ] [  ] [  ] [  ] [  ]   |
|  [  ] [  ] [  ] [  ] [  ] [  ] [  ] [  ] [  ]   |
|  [  ] [  ] [  ] [  ] [  ] [  ] [  ] [  ] [  ]   |
|  -------- hotbar row --------                     |
|  [01] [  ] [  ] [  ] [  ] [  ] [  ] [  ] [  ]   |
+--------------------------------------------------+
```

Each slot is rendered as a 48x48 logical-pixel `egui::Button` with an image (item icon texture) or an empty background. If the slot contains an `ItemStack`, the stack count is drawn as a small label in the bottom-right corner of the slot. Empty slots are clickable but display only a dark border.

### Drag-and-Drop

Drag-and-drop is implemented using egui's built-in drag-and-drop API (`egui::DragValue` is not used; instead, custom `Sense::drag()` and `Sense::click()` are combined):

1. **Pickup** -- Left-click on a non-empty slot sets `DragState::Carrying { source_index: usize, stack: ItemStack }` in a `DragState` resource. The source slot is cleared in the `Inventory`. The carried item is rendered as a semi-transparent icon following the cursor.

2. **Place** -- Left-click on another slot while carrying:
   - If the target slot is empty, the carried stack is placed there.
   - If the target slot contains the same `item_id` and the combined count does not exceed `max_stack`, the stacks merge.
   - If the target slot contains a different item, the two stacks are swapped: the carried stack goes into the target, and the target's old stack becomes the new carried item.
   - If the target is the same slot as the source, the item is returned (cancel drag).

3. **Drop outside** -- If the player clicks outside the inventory window while carrying, the item is dropped into the world (emits a `DropItemEvent` to the ECS).

### Right-Click Context Menu

Right-clicking a non-empty slot opens a small `egui::menu::context_menu` with options:

- **Drop** -- Removes the stack from the slot and emits a `DropItemEvent`.
- **Split Stack** -- If count > 1, splits the stack in half (ceil/floor). Half remains in the slot; half becomes the carried item in `DragState`.

### Hotbar Synchronization

The hotbar (Story 02) always reflects slots `[rows * cols - cols .. rows * cols)` (the last row). When items are moved in or out of the bottom row, the hotbar updates automatically because both the HUD and the inventory read the same `Inventory` component.

### Toggle

The inventory opens and closes with the `E` key (mapped to `Action::OpenInventory` from Epic 15). When the inventory opens:
- `UiState::active_menu` is set to `Some(MenuKind::Inventory)`.
- The cursor is released from capture (visible and free to move).
- `EguiIntegration::wants_keyboard()` and `wants_pointer()` return true, suppressing gameplay input.

When the inventory closes:
- `UiState::active_menu` is set to `None`.
- Any carried item in `DragState` is returned to its source slot.
- The cursor is recaptured for first-person camera control.

## Outcome

An `inventory_ui.rs` module in `crates/nebula_ui/src/` exporting `inventory_ui_system`, `DragState`, and the `Inventory` / `ItemStack` types (or re-exporting them from a shared `nebula_items` crate). The system draws the inventory window when `UiState::active_menu == Some(MenuKind::Inventory)`.

## Demo Integration

**Demo crate:** `nebula-demo`

Pressing Tab opens a grid-based inventory. Items can be dragged between slots with visual feedback.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `egui` | `0.31` | Immediate-mode UI: grid layout, buttons, images, context menus, drag sensing |
| `serde` | `1.0` | Serialize/Deserialize `Inventory`, `ItemStack`, `DragState` for save/load |
| `log` | `0.4` | Logging drag-and-drop operations, stack merge events, drop events |

## Unit Tests

| Test Function | Description | Assertion |
|---------------|-------------|-----------|
| `test_inventory_displays_items` | Place items in slots 0, 5, and 27 of a 4x9 inventory and run the UI system. | egui output contains three slot widgets with non-empty item textures at the correct grid positions. |
| `test_drag_and_drop_moves_item` | Place an item in slot 0, simulate drag from slot 0 to slot 1. | Slot 0 is `None`; slot 1 contains the original `ItemStack`. |
| `test_drag_and_drop_swap` | Place item A in slot 0 and item B in slot 1, drag from slot 0 to slot 1. | Slot 0 contains item B; slot 1 contains item A. |
| `test_stack_merge_same_item` | Slot 0 has `ItemStack { item_id: 1, count: 10, max_stack: 64 }`, slot 1 has `{ item_id: 1, count: 20, max_stack: 64 }`. Drag slot 0 to slot 1. | Slot 0 is `None`; slot 1 has `count: 30`. |
| `test_stack_merge_overflow` | Slot 0 has `{ item_id: 1, count: 50, max_stack: 64 }`, slot 1 has `{ item_id: 1, count: 40, max_stack: 64 }`. Drag slot 0 to slot 1. | Slot 1 has `count: 64`; slot 0 has `count: 26` (overflow returned). |
| `test_stack_count_label` | Place `ItemStack { count: 42, .. }` in slot 3 and render. | The slot widget contains the label text "42". |
| `test_empty_slot_clickable` | Click on an empty slot while not carrying anything. | No panic; `DragState` remains `None`. |
| `test_inventory_toggle_open_close` | Simulate pressing `E` twice. | First press: `UiState::active_menu` becomes `Some(MenuKind::Inventory)`. Second press: becomes `None`. |
| `test_hotbar_reflects_bottom_row` | Place an item in slot 27 (first slot of bottom row in a 4x9 grid). | The hotbar's slot 0 contains the same `ItemStack`. |
| `test_right_click_split_stack` | Right-click slot with `count: 11`, select "Split Stack". | Slot has `count: 6`; `DragState` carries `count: 5`. |
| `test_right_click_drop_emits_event` | Right-click slot with an item, select "Drop". | Slot is `None`; a `DropItemEvent` is emitted to the ECS event queue. |
| `test_close_inventory_returns_carried_item` | Pick up item from slot 0, then close inventory with `E`. | Slot 0 has the original item back; `DragState` is `None`. |
