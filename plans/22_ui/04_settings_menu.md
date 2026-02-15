# Settings Menu

## Problem

Players need to customize the engine's behavior to match their hardware capabilities and personal preferences. Graphics settings (resolution, fullscreen, render distance, VSync) directly affect performance and visual quality. Audio settings (volume levels) affect the experience. Control settings (keybindings) affect playability across different keyboard layouts and playstyles. Network settings (player name, default server) affect multiplayer identity. Without a settings menu, players must hand-edit configuration files, which is error-prone and inaccessible. Changes must persist across sessions (written to disk), and some changes (resolution, fullscreen) require a renderer restart while others (volume, keybindings) must apply immediately.

## Solution

Implement a `SettingsMenu` system in the `nebula_ui` crate that renders a tabbed settings interface using egui. Settings values are backed by a `GameConfig` struct that is serialized to and deserialized from a `config.ron` file using the RON format.

### GameConfig

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameConfig {
    pub graphics: GraphicsConfig,
    pub audio: AudioConfig,
    pub controls: ControlsConfig,
    pub network: NetworkConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphicsConfig {
    pub resolution: (u32, u32),
    pub fullscreen: bool,
    pub render_distance: u32,      // in chunks, e.g. 8, 16, 32
    pub vsync: bool,
    pub quality_preset: QualityPreset, // Low, Medium, High, Ultra
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioConfig {
    pub master_volume: f32,   // 0.0 to 1.0
    pub sfx_volume: f32,
    pub music_volume: f32,
    pub ambient_volume: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlsConfig {
    pub bindings: HashMap<Action, Vec<InputBinding>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    pub player_name: String,
    pub default_server: String,  // "host:port"
}
```

### Tab Layout

The settings menu uses `egui::TopBottomPanel::top` for a tab bar with four tabs: Graphics, Audio, Controls, Network. The active tab's content is drawn in `egui::CentralPanel`. A "Back" button in the top-left returns to the previous `GameState` (main menu or in-game pause menu). An "Apply" button at the bottom-right saves changes. A "Reset to Defaults" button restores factory settings.

```
+--------------------------------------------------+
| [Back]   Graphics | Audio | Controls | Network   |
+--------------------------------------------------+
|                                                    |
|  Resolution:    [1920x1080 v]                     |
|  Fullscreen:    [x]                                |
|  Render Dist:   [====16====]                      |
|  VSync:         [x]                                |
|  Quality:       [High v]                           |
|                                                    |
|                         [Reset Defaults] [Apply]  |
+--------------------------------------------------+
```

### Graphics Tab

- **Resolution** -- A combo box listing available resolutions queried from the monitor via `winit::monitor::MonitorHandle::video_modes()`. Changing resolution sets a `pending_restart` flag because the surface and swap chain must be reconfigured.
- **Fullscreen** -- A checkbox. Toggling fullscreen calls `window.set_fullscreen()` immediately (no restart needed on most platforms, but marked as pending on Wayland where it may require a surface reconfigure).
- **Render Distance** -- An integer slider from 4 to 64 chunks. Applied immediately by updating the chunk loading system's radius.
- **VSync** -- A checkbox. Toggles `PresentMode::Fifo` vs. `PresentMode::Mailbox`. Requires surface reconfiguration (restart flag).
- **Quality Preset** -- A combo box (Low, Medium, High, Ultra) that bulk-sets shadow resolution, texture filtering, AO quality, and draw distance. Individual sub-settings can still be overridden.

### Audio Tab

- Four horizontal sliders (0% to 100%) for master, SFX, music, and ambient volumes. Changes are applied immediately by writing to the audio engine's volume resources (from Epic 20). No restart required.

### Controls Tab

- A scrollable list of all `Action` variants (from Epic 15, Story 04) with their current bindings displayed as button labels. Clicking a binding enters "rebind mode": the button label changes to "Press a key...", and the next key/button press replaces the binding. Pressing Escape cancels rebind. Duplicate bindings are highlighted in yellow with a warning tooltip. A "Reset to Defaults" button restores the default `InputMap`.

### Network Tab

- **Player Name** -- A text input field, 3--16 characters, alphanumeric plus underscores. Validated on each keystroke.
- **Default Server** -- A text input field for `host:port`. Validated with basic format check (non-empty host, port in 1--65535 range).

### Persistence

When "Apply" is clicked:

1. The current `GameConfig` is serialized to RON using `ron::ser::to_string_pretty`.
2. The RON string is written to `config.ron` in the engine's data directory (determined by the `dirs` crate: `~/.config/nebula-engine/` on Linux, `%APPDATA%/nebula-engine/` on Windows, `~/Library/Application Support/nebula-engine/` on macOS).
3. If any setting has the `pending_restart` flag, a dialog appears: "Some changes require a restart to take effect. Restart now?" with Yes/No buttons.

On startup, `GameConfig::load()` reads from `config.ron`. If the file does not exist or fails to parse, defaults are used and a warning is logged.

### Live vs. Restart Changes

| Setting | Apply Mode |
|---------|-----------|
| Resolution | Restart |
| Fullscreen | Immediate (with potential Wayland restart) |
| Render Distance | Immediate |
| VSync | Restart |
| Quality Preset | Immediate (shader recompile deferred) |
| All Audio | Immediate |
| Keybindings | Immediate |
| Network | Immediate (applied on next connect) |

## Outcome

A `settings_menu.rs` module in `crates/nebula_ui/src/` exporting a `settings_menu_system` function and the `GameConfig` struct with all sub-configs. A `config.rs` module handles RON serialization/deserialization and file I/O. The settings menu is drawn when `GameState::Settings` is active.

## Demo Integration

**Demo crate:** `nebula-demo`

The settings menu has sliders for render distance, shadow quality, volume, and mouse sensitivity. Changes apply immediately.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `egui` | `0.31` | Immediate-mode UI: tabs, sliders, combo boxes, text fields, checkboxes |
| `serde` | `1.0` | Derive `Serialize`/`Deserialize` for `GameConfig` and all sub-configs |
| `ron` | `0.12` | Human-readable serialization format for config files |
| `winit` | `0.30` | Monitor video mode enumeration, fullscreen toggling |
| `dirs` | `6.0` | Platform-specific config directory resolution |
| `log` | `0.4` | Logging config load/save events and parse errors |

## Unit Tests

| Test Function | Description | Assertion |
|---------------|-------------|-----------|
| `test_settings_load_current_values` | Serialize a `GameConfig` to RON, load it back, and verify all fields match. | Loaded config equals the original config via `PartialEq`. |
| `test_changes_persist_to_file` | Modify `master_volume` to `0.5`, save to a temp file, load it back. | Loaded `master_volume` equals `0.5`. |
| `test_volume_slider_changes_audio_immediately` | Move the master volume slider to `0.3` and check the audio engine resource. | The audio manager's master volume equals `0.3` within `f32::EPSILON`. |
| `test_resolution_change_sets_restart_flag` | Change resolution from `1920x1080` to `2560x1440`. | `pending_restart` flag is `true`. |
| `test_keybinding_rebind_replaces_binding` | Enter rebind mode for `Jump`, press `KeyJ`. | `ControlsConfig.bindings[Jump]` contains `InputBinding::Key(KeyJ)` and no longer contains the old binding. |
| `test_keybinding_rebind_cancel_with_escape` | Enter rebind mode for `Jump`, press `Escape`. | Binding is unchanged from before entering rebind mode. |
| `test_duplicate_binding_warning` | Bind `Jump` and `Crouch` both to `Space`. | The controls tab displays a warning indicator on both conflicting bindings. |
| `test_default_config_is_valid` | Construct `GameConfig::default()` and validate all fields. | Resolution is non-zero, volumes are in `[0.0, 1.0]`, player name is non-empty, render distance is in `[4, 64]`. |
| `test_missing_config_file_uses_defaults` | Attempt to load from a nonexistent path. | Returns `GameConfig::default()` without panicking; a warning is logged. |
| `test_player_name_validation` | Enter names: "", "ab", "valid_name", "this_name_is_way_too_long_for_the_field". | Empty and 2-char names are rejected; "valid_name" is accepted; 17+ char name is truncated to 16. |
