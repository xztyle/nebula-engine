# Entity Inspector

## Problem

When building a game world, designers need to see exactly what an entity is made of — its position, rotation, scale, name, mesh reference, physics properties, and any custom gameplay components. Without an inspector, debugging requires adding `println!` statements, recompiling, and restarting, which kills iteration speed. The inspector must display live, editable values so that a designer can drag a slider to adjust a light's intensity or type a new position and see the entity move immediately in the viewport. It must also handle the diversity of component types: built-in engine components with known layouts and user-defined components that the inspector discovers at runtime through reflection.

Selecting an entity should be intuitive: click it in the viewport (requiring a raycast from screen coordinates into world space) or pick it from a searchable list when it is occluded or off-screen. The inspector must clearly show the entity's ID, archetype, and every attached component in collapsible sections to avoid overwhelming the user when an entity has dozens of components.

## Solution

Implement an `EntityInspector` struct and associated systems in the `nebula_editor` crate that render an egui side panel showing the selected entity's components.

### Selection State

```rust
use bevy_ecs::prelude::*;

#[derive(Resource, Default)]
pub struct SelectedEntity {
    pub entity: Option<Entity>,
}
```

This resource is written by two input paths:

1. **Viewport click** — A system casts a ray from the mouse position through the camera frustum into world space using the engine's 128-bit coordinate system. The ray is tested against entity bounding volumes (AABBs or mesh colliders). The nearest hit sets `SelectedEntity::entity`.
2. **List selection** — The scene hierarchy panel (Story 06) writes directly to `SelectedEntity` when a row is clicked.

### Raycast Selection System

```rust
pub fn viewport_select_system(
    mouse: Res<MouseState>,
    keyboard: Res<KeyboardState>,
    camera: Query<(&CameraProjection, &Transform), With<EditorCamera>>,
    selectables: Query<(Entity, &WorldPos, &Aabb)>,
    mut selected: ResMut<SelectedEntity>,
    mode: Res<EditorMode>,
    egui_integration: Res<EguiIntegration>,
) {
    if *mode != EditorMode::Editor { return; }
    if egui_integration.wants_pointer() { return; }
    if !mouse.just_pressed(MouseButton::Left) { return; }

    let (proj, cam_transform) = camera.single();
    let ray = screen_to_ray(mouse.position(), proj, cam_transform);

    let mut closest: Option<(Entity, f64)> = None;
    for (entity, world_pos, aabb) in &selectables {
        if let Some(t) = ray_aabb_intersect(&ray, world_pos, aabb) {
            if closest.is_none() || t < closest.unwrap().1 {
                closest = Some((entity, t));
            }
        }
    }

    selected.entity = closest.map(|(e, _)| e);
}
```

### Inspector Panel

```rust
pub fn entity_inspector_ui(
    ctx: &egui::Context,
    selected: Res<SelectedEntity>,
    world: &World,
) {
    let Some(entity) = selected.entity else { return };

    egui::SidePanel::right("entity_inspector")
        .default_width(320.0)
        .show(ctx, |ui| {
            ui.heading("Entity Inspector");
            ui.label(format!("Entity: {:?}", entity));

            if let Some(entity_ref) = world.get_entity(entity) {
                let archetype = entity_ref.archetype();
                ui.label(format!("Archetype: {:?}", archetype.id()));
                ui.separator();

                // Display each component in a collapsible section
                for component_id in archetype.components() {
                    let info = world.components().get_info(component_id).unwrap();
                    let name = info.name();

                    egui::CollapsingHeader::new(name)
                        .default_open(true)
                        .show(ui, |ui| {
                            render_component_fields(ui, entity, component_id, world);
                        });
                }
            }
        });
}
```

### Component Field Rendering

Component fields are rendered using a trait-based dispatch system. Engine-built-in components (`WorldPos`, `LocalPos`, `Rotation`, `Scale`, `EntityName`) implement an `InspectorWidget` trait:

```rust
pub trait InspectorWidget {
    fn inspector_ui(&mut self, ui: &mut egui::Ui) -> bool; // returns true if modified
}

impl InspectorWidget for WorldPos {
    fn inspector_ui(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label("X:");
            changed |= ui.add(egui::DragValue::new(&mut self.x).speed(1.0)).changed();
        });
        ui.horizontal(|ui| {
            ui.label("Y:");
            changed |= ui.add(egui::DragValue::new(&mut self.y).speed(1.0)).changed();
        });
        ui.horizontal(|ui| {
            ui.label("Z:");
            changed |= ui.add(egui::DragValue::new(&mut self.z).speed(1.0)).changed();
        });
        changed
    }
}
```

For custom or unknown components, the inspector falls back to displaying the component's type name and a read-only "(no widget)" label. Components marked with a `ReadOnly` marker trait display their values but disable editing.

### Immediate Application

Changes made through `DragValue`, `TextEdit`, or `Checkbox` widgets are written directly to the component through mutable ECS access. Because the inspector runs during the `Update` stage in editor mode (with time paused), changes are visible immediately in the next render frame.

## Outcome

An `entity_inspector.rs` module in `crates/nebula_editor/src/` exporting `SelectedEntity`, `viewport_select_system`, `entity_inspector_ui`, and the `InspectorWidget` trait. Clicking an entity in the viewport or selecting it from the hierarchy populates a right-side egui panel with all of its components, their fields, and editable controls. Changes apply immediately and are visible in the viewport.

## Demo Integration

**Demo crate:** `nebula-demo`

Clicking an entity opens a side panel showing its name, position, and all components. Values are editable live.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.18` | World queries, component access, entity references, archetype introspection |
| `egui` | `0.31` | Side panel layout, collapsible headers, drag values, text edits |
| `glam` | `0.32` | Vector and quaternion types for transform component display |
| `winit` | `0.30` | Mouse position for viewport raycasting |

Rust edition 2024. Depends on `nebula_input`, `nebula_math` (for 128-bit raycast), `nebula_ecs` (for core components), and `nebula_ui`.

## Unit Tests

| Test Function | Description | Assertion |
|---------------|-------------|-----------|
| `test_selecting_entity_shows_components` | Spawn an entity with `WorldPos` and `EntityName`, set it as `SelectedEntity`, run the inspector system. | Inspector output contains both component names in the rendered UI. |
| `test_editing_position_updates_entity` | Select an entity with `WorldPos(0, 0, 0)`, simulate dragging the X field to 100, run a frame. | The entity's `WorldPos.x` is now `100`. |
| `test_multiple_components_shown` | Spawn an entity with five components, select it. | All five component sections appear in the inspector panel. |
| `test_deselecting_clears_inspector` | Set `SelectedEntity::entity` to `None`. | Inspector panel body is empty (no component sections rendered). |
| `test_read_only_components_not_editable` | Spawn an entity with a read-only component (e.g., `ArchetypeId`), select it. | The component's fields are displayed but the UI widgets are disabled (non-interactive). |
| `test_raycast_selects_nearest_entity` | Place two entities along the same ray at distances 10 and 20, simulate a viewport click. | `SelectedEntity::entity` equals the entity at distance 10. |
| `test_click_on_empty_space_deselects` | Select an entity, then simulate a click that hits no entity bounding box. | `SelectedEntity::entity` is `None`. |
