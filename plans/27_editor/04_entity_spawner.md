# Entity Spawner

## Problem

Level designers need to populate the world with entities — lights, NPCs, collectibles, obstacles, spawn points, triggers — without writing code. A drag-and-drop spawner backed by a catalog of available entity types is essential for productive world-building. Without one, every entity must be created programmatically, requiring a recompile cycle for each placement adjustment.

The spawner must support a prefab-like catalog system where each entry defines a bundle of default components. Clicking a catalog item and then clicking in the viewport should place the entity at the raycast hit point on the terrain or voxel surface, correctly accounting for the cubesphere planet geometry. Common operations like duplicating an existing entity (Ctrl+D) and deleting one (Delete key) must be fast keyboard shortcuts to keep the workflow fluid.

## Solution

Implement an `EntitySpawner` system and catalog UI in the `nebula_editor` crate.

### Entity Catalog

The catalog is a registry of spawnable entity templates, each defining a name, icon, and a closure that inserts the entity's default components into the world:

```rust
use bevy_ecs::prelude::*;
use std::sync::Arc;

pub struct EntityTemplate {
    /// Display name in the catalog UI.
    pub name: String,
    /// Icon identifier for the catalog list.
    pub icon: EntityIcon,
    /// Category for grouping (e.g., "Lights", "Props", "Gameplay").
    pub category: String,
    /// Function that spawns the entity with default components.
    pub spawn_fn: Arc<dyn Fn(&mut World, WorldPos) -> Entity + Send + Sync>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityIcon {
    Mesh,
    Light,
    Camera,
    Trigger,
    SpawnPoint,
    Generic,
}

#[derive(Resource, Default)]
pub struct EntityCatalog {
    pub templates: Vec<EntityTemplate>,
}
```

Templates are registered during engine initialization:

```rust
catalog.templates.push(EntityTemplate {
    name: "Point Light".into(),
    icon: EntityIcon::Light,
    category: "Lights".into(),
    spawn_fn: Arc::new(|world, pos| {
        world.spawn((
            WorldPos::from(pos),
            LocalPos::default(),
            PointLight { intensity: 1.0, radius: 10.0, color: [1.0, 1.0, 1.0] },
            EntityName("Point Light".into()),
        )).id()
    }),
});
```

### Spawner State

```rust
#[derive(Resource, Default)]
pub struct SpawnerState {
    /// Index into `EntityCatalog::templates` that the user has selected.
    pub selected_template: Option<usize>,
    /// Whether the spawner is in "placement mode" (click to place).
    pub placing: bool,
}
```

### Catalog UI

An egui panel lists all templates grouped by category. Clicking an entry activates placement mode:

```rust
pub fn entity_catalog_ui(
    ctx: &egui::Context,
    catalog: Res<EntityCatalog>,
    mut spawner: ResMut<SpawnerState>,
) {
    egui::SidePanel::left("entity_catalog")
        .default_width(220.0)
        .show(ctx, |ui| {
            ui.heading("Entity Catalog");
            ui.separator();

            let mut categories: Vec<&str> = catalog.templates
                .iter()
                .map(|t| t.category.as_str())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();

            for category in categories {
                egui::CollapsingHeader::new(category)
                    .default_open(true)
                    .show(ui, |ui| {
                        for (idx, template) in catalog.templates.iter().enumerate() {
                            if template.category != category { continue; }
                            let selected = spawner.selected_template == Some(idx);
                            let label = format!("{} {}", icon_char(template.icon), &template.name);
                            if ui.selectable_label(selected, label).clicked() {
                                spawner.selected_template = Some(idx);
                                spawner.placing = true;
                            }
                        }
                    });
            }
        });
}
```

### Placement System

When placement mode is active and the user clicks in the viewport, the system raycasts to the terrain surface and spawns the entity at the hit position:

```rust
pub fn entity_place_system(
    mouse: Res<MouseState>,
    camera: Query<(&CameraProjection, &Transform), With<EditorCamera>>,
    chunks: Res<ChunkManager>,
    catalog: Res<EntityCatalog>,
    mut spawner: ResMut<SpawnerState>,
    mut selected: ResMut<SelectedEntity>,
    world: &mut World,
    egui_integration: Res<EguiIntegration>,
) {
    if !spawner.placing { return; }
    if egui_integration.wants_pointer() { return; }
    if !mouse.just_pressed(MouseButton::Left) { return; }

    let Some(template_idx) = spawner.selected_template else { return };
    let template = &catalog.templates[template_idx];

    let ray = compute_editor_ray(&mouse, &camera);
    if let Some(hit) = voxel_raycast(&ray, &chunks) {
        let spawn_pos = hit.surface_position;
        let entity = (template.spawn_fn)(world, spawn_pos);
        selected.entity = Some(entity);
        spawner.placing = false;
    }
}
```

### Duplicate and Delete

```rust
pub fn entity_duplicate_system(
    keyboard: Res<KeyboardState>,
    selected: Res<SelectedEntity>,
    mut commands: Commands,
    query: Query<(&WorldPos, &EntityName)>,
    mut new_selected: ResMut<SelectedEntity>,
) {
    if keyboard.is_pressed(PhysicalKey::Code(KeyCode::ControlLeft))
        && keyboard.just_pressed(PhysicalKey::Code(KeyCode::KeyD))
    {
        if let Some(entity) = selected.entity {
            if let Ok((pos, name)) = query.get(entity) {
                // Clone the entity with a slight position offset
                let new_pos = WorldPos {
                    x: pos.x + 2,
                    y: pos.y,
                    z: pos.z,
                };
                let new_entity = commands.spawn((
                    new_pos,
                    LocalPos::default(),
                    EntityName(format!("{} (Copy)", name.0)),
                )).id();
                new_selected.entity = Some(new_entity);
            }
        }
    }
}

pub fn entity_delete_system(
    keyboard: Res<KeyboardState>,
    mut selected: ResMut<SelectedEntity>,
    mut commands: Commands,
    mut undo_stack: ResMut<UndoStack>,
) {
    if keyboard.just_pressed(PhysicalKey::Code(KeyCode::Delete)) {
        if let Some(entity) = selected.entity {
            undo_stack.push(UndoAction::DespawnEntity(entity));
            commands.entity(entity).despawn();
            selected.entity = None;
        }
    }
}
```

## Outcome

An `entity_spawner.rs` module in `crates/nebula_editor/src/` exporting `EntityCatalog`, `EntityTemplate`, `EntityIcon`, `SpawnerState`, `entity_catalog_ui`, `entity_place_system`, `entity_duplicate_system`, and `entity_delete_system`. Designers browse a categorized catalog, click to enter placement mode, click in the viewport to spawn an entity at the terrain surface, and use keyboard shortcuts to duplicate or delete entities.

## Demo Integration

**Demo crate:** `nebula-demo`

A palette panel lists available entity types: lights, props, NPCs. Clicking one and clicking the world places it with physics and rendering.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.18` | Entity spawning, commands, queries, resources |
| `egui` | `0.31` | Catalog panel, collapsible categories, selectable labels |
| `glam` | `0.32` | Position math for placement offset and surface hit computation |
| `winit` | `0.30` | Key codes for Ctrl+D, Delete, mouse button for click-to-place |

Rust edition 2024. Depends on `nebula_voxel` (for surface raycasting), `nebula_input`, `nebula_math`, `nebula_ecs`, and `nebula_ui`.

## Unit Tests

| Test Function | Description | Assertion |
|---------------|-------------|-----------|
| `test_spawning_creates_entity_at_position` | Select a template, simulate a click at a surface position, run the place system. | A new entity exists in the world with `WorldPos` matching the hit position. |
| `test_catalog_lists_available_types` | Register three templates in `EntityCatalog` and render the catalog UI. | All three template names appear in the UI output. |
| `test_delete_removes_entity` | Spawn an entity, select it, press Delete, run the delete system. | `world.get_entity(entity)` returns `None`. |
| `test_duplicate_creates_copy` | Spawn an entity with `EntityName("Torch")`, select it, press Ctrl+D, run the duplicate system. | A second entity exists with `EntityName("Torch (Copy)")` and a nearby position. |
| `test_spawned_entity_has_correct_components` | Use the "Point Light" template to spawn an entity. | The spawned entity has `WorldPos`, `LocalPos`, `PointLight`, and `EntityName` components. |
| `test_placement_mode_deactivates_after_place` | Enter placement mode, place an entity. | `spawner.placing` is `false` after the placement completes. |
| `test_click_on_ui_does_not_place` | Enter placement mode, simulate a click while `egui_integration.wants_pointer()` returns `true`. | No new entity is spawned. |
