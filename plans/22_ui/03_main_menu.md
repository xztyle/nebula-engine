# Main Menu

## Problem

When the engine starts, the player must be presented with a clear entry point: a main menu that offers navigation to single-player gameplay, multiplayer server connection, settings configuration, and application exit. Without a dedicated menu state, the engine would either drop the player directly into a world (confusing and inflexible) or show a blank screen. The main menu is also the first visual impression of the engine, so it must be visually polished, respond crisply to input, and transition cleanly into gameplay or sub-menus without leaving orphaned state or resources. Because the engine targets Linux, Windows, and macOS, the menu must work identically across all three platforms, handling differences in window decoration, DPI, and quit behavior (e.g., macOS `Cmd+Q` vs. window close button).

## Solution

Implement a `MainMenu` UI system and a `GameState` enum to manage top-level application state transitions. The main menu is drawn using egui via the `EguiIntegration` from Story 01.

### Game State

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameState {
    MainMenu,
    Loading { world_name: String },
    InGame,
    Settings,
    MultiplayerConnect,
}
```

The `GameState` is stored as an ECS resource. Systems check the current state to decide what to draw and what systems to run. Transitions are performed by writing a new value to the resource.

### Menu Layout

The main menu uses `egui::CentralPanel` with a vertical layout, centered horizontally and vertically:

```
+--------------------------------------------------+
|                                                    |
|                                                    |
|              [Engine Logo / Title]                 |
|              "Nebula Engine"                       |
|                                                    |
|              [ Single Player ]                     |
|              [ Multiplayer   ]                     |
|              [ Settings      ]                     |
|              [ Quit          ]                     |
|                                                    |
|                                                    |
+--------------------------------------------------+
```

Each button is 240 logical pixels wide and 48 logical pixels tall, with 12px vertical spacing. The engine title uses a large font (32pt). Buttons use the default egui style with rounded corners (4px radius).

### Button Actions

1. **Single Player** -- Transitions `GameState` to `Loading { world_name: "default" }`. The loading state triggers world generation / loading systems. Once loading completes, the state moves to `InGame`.

2. **Multiplayer** -- Transitions to `MultiplayerConnect`. This opens the server browser / direct connect UI (a separate panel drawn by the multiplayer UI system). The panel shows a text field for server address (`host:port`), a "Connect" button, and a "Back" button that returns to `MainMenu`.

3. **Settings** -- Transitions to `Settings`. This opens the settings menu (Story 04). The settings menu has a "Back" button that returns to `MainMenu`.

4. **Quit** -- Sends a close request to the winit event loop via `ctx.send_viewport_cmd(egui::ViewportCommand::Close)`, which triggers the standard application shutdown sequence.

### Background

While the main menu is active, the 3D render pass draws a slowly rotating cubesphere planet at a fixed camera distance, or alternatively a procedural starfield (from Epic 12). The background is rendered before the egui pass, so the menu appears overlaid on top of it. A semi-transparent dark overlay (`Color32::from_black_alpha(180)`) is drawn behind the menu buttons to ensure readability against any background.

### Input

The main menu captures all input via egui. `EguiIntegration::wants_keyboard()` and `wants_pointer()` return true, so no gameplay input processing occurs. Keyboard navigation is supported: Up/Down arrows move focus between buttons, Enter activates the focused button. Escape from a sub-menu (Settings, Multiplayer) returns to the main menu.

### Platform-Specific Quit

On macOS, `Cmd+Q` produces a `WindowEvent::CloseRequested`, which the engine handles the same way as clicking "Quit". On Linux and Windows, the close button and `Alt+F4` produce the same event. The main menu does not add a confirmation dialog for quit (the game has not started yet), but the in-game pause menu (a separate system) will.

## Outcome

A `main_menu.rs` module in `crates/nebula_ui/src/` exporting a `main_menu_system` function and the `GameState` enum. The system draws the main menu when `GameState::MainMenu` is active. The `GameState` resource is registered in the ECS world at startup with an initial value of `MainMenu`.

## Demo Integration

**Demo crate:** `nebula-demo`

Pressing Escape opens a main menu with Resume, Settings, and Quit buttons. The 3D scene is visible but dimmed behind it.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `egui` | `0.31` | Immediate-mode UI: buttons, labels, panels, viewport commands |
| `winit` | `0.30` | Window close events, platform quit handling |
| `serde` | `1.0` | Serialize/Deserialize `GameState` for debugging and state snapshots |
| `log` | `0.4` | Logging state transitions and button clicks |

## Unit Tests

| Test Function | Description | Assertion |
|---------------|-------------|-----------|
| `test_menu_displays_all_options` | Run `main_menu_system` with `GameState::MainMenu` and inspect egui output. | The output contains four buttons labeled "Single Player", "Multiplayer", "Settings", and "Quit". |
| `test_single_player_transitions_to_loading` | Simulate a click on the "Single Player" button. | `GameState` transitions to `Loading { world_name: "default" }`. |
| `test_multiplayer_opens_connect_ui` | Simulate a click on the "Multiplayer" button. | `GameState` transitions to `MultiplayerConnect`. |
| `test_settings_opens_settings_menu` | Simulate a click on the "Settings" button. | `GameState` transitions to `Settings`. |
| `test_quit_sends_close_command` | Simulate a click on the "Quit" button. | The egui viewport command queue contains `ViewportCommand::Close`. |
| `test_initial_game_state_is_main_menu` | Check the `GameState` resource immediately after ECS world setup. | Value equals `GameState::MainMenu`. |
| `test_escape_from_settings_returns_to_main` | Set `GameState::Settings`, simulate Escape key press. | `GameState` transitions back to `MainMenu`. |
| `test_escape_from_multiplayer_returns_to_main` | Set `GameState::MultiplayerConnect`, simulate Escape key press. | `GameState` transitions back to `MainMenu`. |
| `test_keyboard_navigation` | Simulate Down arrow key presses to move focus, then Enter to activate. | Focus moves between buttons sequentially; Enter on "Settings" transitions to `Settings`. |
