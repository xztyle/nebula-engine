# Configurable Keybindings

## Problem

Players expect to remap controls to match their preferences and accessibility needs. The engine's `InputMap` (Story 04) stores action-to-input bindings, but there is no mechanism to persist those bindings across sessions, detect conflicting assignments (two actions on the same key), or support modifier-key combinations (Ctrl+X, Shift+Click). A settings UI (covered in Epic 22) will need a backend API for listing current bindings, initiating a rebind-listen mode, validating the new binding, and saving the result.

## Solution

### Serialization to RON

The `InputMap` derives `Serialize` and `Deserialize` via serde. The canonical on-disk format is RON (Rusty Object Notation) version 0.12, stored at the platform-appropriate config directory:

- Linux: `$XDG_CONFIG_HOME/nebula/input.ron` (typically `~/.config/nebula/input.ron`)
- Windows: `%APPDATA%\nebula\input.ron`
- macOS: `~/Library/Application Support/nebula/input.ron`

```ron
// input.ron
InputMap(
    bindings: {
        MoveForward: [Key(KeyW), GamepadAxis(LeftStickY(Positive))],
        MoveBack:    [Key(KeyS), GamepadAxis(LeftStickY(Negative))],
        Jump:        [Key(Space), GamepadButton(South)],
        Sprint:      [Key(ShiftLeft)],
        Interact:    [Key(KeyE), GamepadButton(West)],
        OpenInventory: [Key(KeyI)],
        Pause:       [Key(Escape), GamepadButton(Start)],
    },
)
```

### Modifier key support

Extend `InputBinding` with a modifier variant:

```rust
pub enum InputBinding {
    Key(PhysicalKey),
    KeyWithModifiers {
        key: PhysicalKey,
        modifiers: Modifiers, // bitflags: SHIFT, CTRL, ALT, SUPER
    },
    MouseButton(MouseButton),
    MouseButtonWithModifiers {
        button: MouseButton,
        modifiers: Modifiers,
    },
    GamepadButton(UnifiedButton),
    GamepadAxis(GamepadAxisBinding),
}
```

When evaluating a `KeyWithModifiers` binding, the `ActionResolver` checks that both the key is pressed and the required modifier bits are active in `KeyboardState::active_modifiers()`.

### Conflict detection

Before accepting a rebind, `InputMap::detect_conflicts(&self) -> Vec<Conflict>` iterates all bindings and flags any `InputBinding` that appears in more than one action's binding list. The UI can display a warning ("Space is already bound to Jump. Unbind it first?") and let the player confirm or cancel. Conflicts within the same action (duplicate entries) are also flagged.

### Rebind flow

1. The settings UI calls `InputMap::start_rebind(action: Action)` which puts the input system into a "listen" mode for the next meaningful input event.
2. The next key press, mouse button click, or gamepad button press is captured as the new `InputBinding`.
3. Conflict detection runs. If clean, the binding is applied. If conflicting, the UI is notified.
4. `InputMap::save(&self, path: &Path) -> Result<()>` serializes to RON.
5. On engine startup, `InputMap::load(path: &Path) -> Result<InputMap>` deserializes. If the file is missing or malformed, the default map is used and a warning is logged.

### Loading priority

1. User config file (if present and valid).
2. Default hardcoded bindings (compiled into the engine).

This ensures first-time players get sensible defaults and returning players get their customized layout.

## Outcome

A `keybindings.rs` module in `crates/nebula_input/src/` exporting `Modifiers`, the extended `InputBinding` variants, `Conflict`, and the `save`/`load`/`detect_conflicts`/`start_rebind` methods on `InputMap`. The user's `input.ron` file is stored alongside other engine settings in the platform config directory.

## Demo Integration

**Demo crate:** `nebula-demo`

Keybindings are loaded from `config.ron`. The player can remap controls by editing the file. The demo logs the active bindings at startup.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| serde | 1.0 | Derive `Serialize` / `Deserialize` on `InputMap`, `Action`, `InputBinding`, `Modifiers` |
| ron | 0.12 | Human-readable serialization format for `input.ron` |
| winit | 0.30 | `PhysicalKey`, `KeyCode`, modifier key detection |
| dirs | 5.0 | Platform-appropriate config directory resolution |

## Unit Tests

| Test Function | Description | Assertion |
|---------------|-------------|-----------|
| `test_default_bindings_serialize_to_ron` | Serialize the default `InputMap` to a RON string. | The output string is valid RON; deserializing it produces an `InputMap` equal to the original. |
| `test_custom_bindings_deserialize_correctly` | Provide a RON string with `Jump` bound to `KeyJ`. Deserialize. | `input_map.bindings[Jump]` contains `Key(KeyJ)`. |
| `test_conflict_detection_flags_duplicates` | Bind `Jump` to `Space` and `Sprint` to `Space`. | `detect_conflicts()` returns a `Vec` containing one `Conflict` referencing `Space`, `Jump`, and `Sprint`. |
| `test_no_conflicts_on_clean_map` | Use the default bindings (no overlaps). | `detect_conflicts()` returns an empty `Vec`. |
| `test_modifier_combinations_work` | Bind `OpenInventory` to `Ctrl+I`. Press `I` alone, then `Ctrl+I`. | `I` alone: `is_action_active(OpenInventory)` false. `Ctrl+I`: true. |
| `test_modifier_subset_does_not_match` | Bind to `Ctrl+Shift+S`. Press only `Ctrl+S`. | `is_action_active` returns false (missing `Shift`). |
| `test_rebinding_persists_across_save_load` | Rebind `Jump` from `Space` to `KeyK`. Save to a temp file. Load from that file. | Loaded map has `Jump` bound to `KeyK`, not `Space`. |
| `test_malformed_ron_falls_back_to_defaults` | Attempt to load from a file containing `"not valid ron {{{"`. | Returns the default `InputMap`; no panic. |
| `test_missing_file_falls_back_to_defaults` | Attempt to load from a nonexistent path. | Returns the default `InputMap`; no panic. |
