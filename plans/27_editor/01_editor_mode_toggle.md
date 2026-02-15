# Editor Mode Toggle

## Problem

During development, artists and designers need to inspect the world, select entities, tweak parameters, and paint terrain without restarting the engine. This requires a distinct "Editor mode" that coexists with the normal "Play mode" inside the same process. Without a clean toggle mechanism, editor functionality would either always be active (wasting resources and polluting the screen in release builds) or require a separate editor binary (losing the ability to live-test changes). The toggle must preserve the entire world state so that switching from Editor to Play and back does not destroy entity positions, voxel data, or resource state. Editor mode must also be compile-time gated to debug builds only, ensuring zero overhead in shipping binaries.

The F5 key is the conventional play/pause toggle in game editors (Unity, Unreal, Godot), and adopting it provides immediate familiarity. When Editor mode is active, the simulation clock must pause so that physics, AI, and animations freeze, giving the designer a stable snapshot to work with. A visible "EDITOR" watermark prevents confusion about which mode is active during screen recordings or screenshots.

## Solution

Introduce an `EditorMode` resource and an `editor_mode_toggle_system` in the `nebula_editor` crate. The resource tracks the current mode, and the system listens for F5 key presses to flip between states.

### Mode Resource

```rust
use bevy_ecs::prelude::*;

#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorMode {
    Play,
    Editor,
}

impl Default for EditorMode {
    fn default() -> Self {
        EditorMode::Play
    }
}
```

The resource is inserted into the ECS world at startup. All editor systems use a run condition that checks `Res<EditorMode>` before executing:

```rust
pub fn in_editor_mode(mode: Res<EditorMode>) -> bool {
    *mode == EditorMode::Editor
}

pub fn in_play_mode(mode: Res<EditorMode>) -> bool {
    *mode == EditorMode::Play
}
```

### Toggle System

```rust
pub fn editor_mode_toggle_system(
    keyboard: Res<KeyboardState>,
    mut mode: ResMut<EditorMode>,
    mut time: ResMut<TimeRes>,
) {
    if keyboard.just_pressed(PhysicalKey::Code(KeyCode::F5)) {
        match *mode {
            EditorMode::Play => {
                *mode = EditorMode::Editor;
                time.paused = true;
            }
            EditorMode::Editor => {
                *mode = EditorMode::Play;
                time.paused = false;
            }
        }
    }
}
```

This system runs in the `PreUpdate` stage so that all downstream systems in the same frame see the updated mode. The `TimeRes` resource gains a `paused: bool` field; when `paused` is true, `delta` is reported as `0.0` and the fixed-update accumulator does not advance, freezing the simulation in place.

### Debug-Only Compilation

The entire `nebula_editor` crate is included only when the `editor` feature is active, which defaults to on in debug profiles and off in release:

```toml
# In workspace Cargo.toml
[features]
default = ["editor"]
editor = ["dep:nebula_editor"]
```

Alternatively, key modules are gated with `#[cfg(debug_assertions)]` so that editor systems are stripped from release builds without requiring feature flags:

```rust
#[cfg(debug_assertions)]
pub fn editor_mode_toggle_system(/* ... */) { /* ... */ }
```

### Free Camera

When entering Editor mode, the system swaps the active camera from the gameplay first-person camera to an `EditorCamera` entity. The editor camera supports free-fly movement (WASD + mouse look + scroll to change speed) independent of the player entity. When returning to Play mode, the gameplay camera is restored.

```rust
#[derive(Component)]
pub struct EditorCamera {
    pub speed: f32,
    pub fast_speed: f32,
}
```

### Editor Watermark

An egui overlay renders "EDITOR" in semi-transparent text at the top-center of the viewport when `EditorMode::Editor` is active:

```rust
pub fn editor_watermark_system(ctx: &egui::Context, mode: Res<EditorMode>) {
    if *mode == EditorMode::Editor {
        egui::Area::new(egui::Id::new("editor_watermark"))
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 8.0))
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new("EDITOR")
                        .size(24.0)
                        .color(egui::Color32::from_rgba_unmultiplied(255, 255, 255, 100)),
                );
            });
    }
}
```

### World State Preservation

No special serialization or snapshot is needed for the toggle. The ECS world is the same `World` instance in both modes. Toggling only changes the `EditorMode` resource and the `TimeRes::paused` flag. All entities, components, and resources remain in place. The editor camera entity is always present but only active (driving the view matrix) when in Editor mode.

## Outcome

An `editor_mode.rs` module in `crates/nebula_editor/src/` exporting `EditorMode`, `in_editor_mode`, `in_play_mode`, `editor_mode_toggle_system`, `EditorCamera`, and `editor_watermark_system`. Pressing F5 in a debug build toggles between Play and Editor modes with time pausing, camera switching, and a visible watermark. Release builds compile out all editor code, incurring zero runtime cost.

## Demo Integration

**Demo crate:** `nebula-demo`

F4 pauses the simulation and enters editor mode. The cursor becomes visible and a grid overlay appears. F4 returns to gameplay.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.18` | ECS resource, system, and run-condition infrastructure |
| `egui` | `0.31` | Immediate-mode watermark overlay rendering |
| `winit` | `0.30` | `PhysicalKey` and `KeyCode` types for F5 detection |
| `glam` | `0.32` | Vector/matrix types for editor camera transform |

Rust edition 2024. The `nebula_editor` crate depends on `nebula_input` (for `KeyboardState`) and `nebula_ui` (for `EguiIntegration`).

## Unit Tests

| Test Function | Description | Assertion |
|---------------|-------------|-----------|
| `test_f5_toggles_play_to_editor` | Start in `EditorMode::Play`, simulate an F5 press, run the toggle system. | `*mode == EditorMode::Editor`. |
| `test_f5_toggles_editor_to_play` | Start in `EditorMode::Editor`, simulate an F5 press, run the toggle system. | `*mode == EditorMode::Play`. |
| `test_editor_mode_pauses_time` | Toggle to Editor mode and inspect `TimeRes`. | `time.paused == true`. |
| `test_play_mode_resumes_time` | Toggle to Editor, then back to Play, and inspect `TimeRes`. | `time.paused == false`. |
| `test_editor_ui_visible_in_editor_mode` | Set `EditorMode::Editor` and call `in_editor_mode()`. | Returns `true`, confirming editor UI systems will run. |
| `test_editor_ui_hidden_in_play_mode` | Set `EditorMode::Play` and call `in_editor_mode()`. | Returns `false`, confirming editor UI systems are skipped. |
| `test_world_state_preserved_across_toggle` | Spawn an entity with a position component, toggle Play -> Editor -> Play. | Entity still exists with the same position after both toggles. |
| `test_editor_mode_debug_only` | Verify that `EditorMode` and toggle system are gated behind `#[cfg(debug_assertions)]` or the `editor` feature. | In a release build (or without the feature), the toggle system is not present in the schedule. |
