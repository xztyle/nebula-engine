# Scene Hierarchy View

## Problem

In a complex scene with hundreds or thousands of entities, designers need a structured overview of every object and its relationships. Without a hierarchy view, discovering what exists in the scene requires either clicking blindly in the viewport (missing invisible entities like triggers, spawn points, and audio sources) or querying the ECS programmatically. Parent-child relationships are central to transform inheritance (a turret mounted on a vehicle should move with it), but these relationships are invisible without a tree visualization.

The hierarchy must support the full range of scene management operations: selecting, renaming, duplicating, deleting, and reparenting entities. Search and filtering are critical once the entity count exceeds a screenful. Icons differentiate entity types at a glance — a light bulb for lights, a camera for cameras, a cube for meshes — reducing cognitive load when scanning a long list.

## Solution

Implement a `SceneHierarchyView` system in the `nebula_editor` crate that renders an egui tree panel of all entities.

### Data Model

Parent-child relationships are stored using bevy_ecs's built-in `Parent` and `Children` components:

```rust
use bevy_ecs::prelude::*;
use bevy_ecs::hierarchy::{Parent, Children};
```

The hierarchy view queries for root entities (those without a `Parent` component) and recursively renders their children.

### Hierarchy Panel

```rust
pub fn scene_hierarchy_ui(
    ctx: &egui::Context,
    mut selected: ResMut<SelectedEntity>,
    roots: Query<Entity, Without<Parent>>,
    children_query: Query<&Children>,
    names: Query<&EntityName>,
    icons: Query<&EntityTypeIcon>,
    mut filter_text: Local<String>,
) {
    egui::SidePanel::left("scene_hierarchy")
        .default_width(260.0)
        .show(ctx, |ui| {
            ui.heading("Scene Hierarchy");

            // Search bar
            ui.horizontal(|ui| {
                ui.label("Search:");
                ui.text_edit_singleline(&mut *filter_text);
            });
            ui.separator();

            egui::ScrollArea::vertical().show(ui, |ui| {
                for entity in roots.iter() {
                    render_entity_tree(
                        ui,
                        entity,
                        &children_query,
                        &names,
                        &icons,
                        &mut selected,
                        &filter_text,
                        0,
                    );
                }
            });
        });
}

fn render_entity_tree(
    ui: &mut egui::Ui,
    entity: Entity,
    children_query: &Query<&Children>,
    names: &Query<&EntityName>,
    icons: &Query<&EntityTypeIcon>,
    selected: &mut ResMut<SelectedEntity>,
    filter: &str,
    depth: usize,
) {
    let name = names
        .get(entity)
        .map(|n| n.0.as_str())
        .unwrap_or("(unnamed)");
    let icon = icons
        .get(entity)
        .map(|i| icon_char(i.0))
        .unwrap_or(' ');

    // Filter: skip entities whose names do not contain the filter text
    let has_children = children_query.get(entity).is_ok();
    let matches_filter = filter.is_empty()
        || name.to_lowercase().contains(&filter.to_lowercase());

    if !matches_filter && !has_matching_descendant(entity, children_query, names, filter) {
        return;
    }

    let is_selected = selected.entity == Some(entity);
    let label = format!("{} {}", icon, name);

    if has_children {
        let response = egui::CollapsingHeader::new(label)
            .id_salt(entity)
            .default_open(depth < 2)
            .show(ui, |ui| {
                if let Ok(children) = children_query.get(entity) {
                    for &child in children.iter() {
                        render_entity_tree(
                            ui, child, children_query, names, icons,
                            selected, filter, depth + 1,
                        );
                    }
                }
            });

        if response.header_response.clicked() {
            selected.entity = Some(entity);
        }

        // Highlight selected
        if is_selected {
            response.header_response.highlight();
        }
    } else {
        let response = ui.selectable_label(is_selected, label);
        if response.clicked() {
            selected.entity = Some(entity);
        }
    }
}
```

### Context Menu

Right-clicking an entity in the hierarchy opens a context menu with common actions:

```rust
fn show_context_menu(
    ui: &mut egui::Ui,
    entity: Entity,
    commands: &mut Commands,
    selected: &mut ResMut<SelectedEntity>,
    undo_stack: &mut ResMut<UndoStack>,
) {
    if ui.button("Rename").clicked() {
        // Open inline rename text field (tracked via RenameState resource)
        ui.close_menu();
    }
    if ui.button("Duplicate").clicked() {
        // Clone entity and all children
        ui.close_menu();
    }
    if ui.button("Delete").clicked() {
        undo_stack.push(UndoAction::DespawnEntity(entity));
        commands.entity(entity).despawn_recursive();
        if selected.entity == Some(entity) {
            selected.entity = None;
        }
        ui.close_menu();
    }
    if ui.button("Add Child").clicked() {
        let child = commands.spawn((
            EntityName("New Entity".into()),
            WorldPos::default(),
            LocalPos::default(),
        )).id();
        commands.entity(entity).add_child(child);
        ui.close_menu();
    }
}
```

### Drag-to-Reparent

The hierarchy supports drag-and-drop reordering. When a user drags an entity node onto another node, the dragged entity becomes a child of the target. This is implemented using egui's drag-and-drop API with entity IDs as the payload:

```rust
pub fn handle_reparent(
    entity: Entity,
    new_parent: Entity,
    commands: &mut Commands,
    undo_stack: &mut ResMut<UndoStack>,
    parent_query: &Query<&Parent>,
) {
    let old_parent = parent_query.get(entity).map(|p| p.get()).ok();
    undo_stack.push(UndoAction::Reparent {
        entity,
        old_parent,
        new_parent: Some(new_parent),
    });
    commands.entity(entity).set_parent(new_parent);
}
```

### Entity Type Icons

A marker component maps entities to icon types for display:

```rust
#[derive(Component, Clone, Copy)]
pub struct EntityTypeIcon(pub EntityIcon);

fn icon_char(icon: EntityIcon) -> char {
    match icon {
        EntityIcon::Mesh => 'M',
        EntityIcon::Light => 'L',
        EntityIcon::Camera => 'C',
        EntityIcon::Trigger => 'T',
        EntityIcon::SpawnPoint => 'S',
        EntityIcon::Generic => 'E',
    }
}
```

## Outcome

A `scene_hierarchy.rs` module in `crates/nebula_editor/src/` exporting `scene_hierarchy_ui`, `EntityTypeIcon`, `handle_reparent`, and `show_context_menu`. The left-side panel displays a searchable, collapsible tree of all entities with parent-child nesting, type icons, selection, drag-to-reparent, and context menu actions for rename, duplicate, delete, and add child.

## Demo Integration

**Demo crate:** `nebula-demo`

A tree panel displays all entities in parent/child hierarchy. Nodes are draggable, collapsible, and show entity names.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.18` | Entity queries, `Parent`/`Children` hierarchy, `Commands` for spawn/despawn/reparent |
| `egui` | `0.31` | Tree panel, collapsible headers, selectable labels, context menus, drag-and-drop |
| `glam` | `0.32` | Default transform values for newly spawned child entities |

Rust edition 2024. Depends on `nebula_ecs` (for `WorldPos`, `LocalPos`, `EntityName`) and `nebula_ui`.

## Unit Tests

| Test Function | Description | Assertion |
|---------------|-------------|-----------|
| `test_all_entities_listed` | Spawn five root entities, render the hierarchy. | All five entity names appear in the UI output. |
| `test_parent_child_nesting_shown` | Spawn a parent with two children, render the hierarchy. | The children appear indented under the parent node. |
| `test_click_selects_entity` | Simulate clicking an entity row in the hierarchy. | `SelectedEntity::entity` matches the clicked entity. |
| `test_drag_reparents_entity` | Drag entity A onto entity B. | Entity A's `Parent` component points to entity B. |
| `test_search_filters_list` | Spawn entities named "Sun", "Moon", "Star". Set filter to "oon". | Only "Moon" appears in the rendered hierarchy. |
| `test_context_menu_delete` | Right-click an entity and select "Delete". | The entity is despawned from the world. |
| `test_context_menu_add_child` | Right-click an entity and select "Add Child". | A new entity exists with a `Parent` pointing to the right-clicked entity. |
| `test_context_menu_duplicate` | Right-click an entity named "Tree" and select "Duplicate". | A second entity with a similar name and matching components exists in the world. |
| `test_icons_displayed_by_type` | Spawn a light entity with `EntityTypeIcon(EntityIcon::Light)`. | The hierarchy row contains the light icon character. |
