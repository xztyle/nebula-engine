# Terrain Brush Tools

## Problem

Procedurally generated terrain provides a starting point, but designers always need to hand-sculpt specific areas: flattening a plateau for a village, raising a mountain ridge, smoothing a cliff face, or painting a biome transition. Without brush-based sculpting tools, terrain modifications require editing noise parameters and regenerating entire chunks, a workflow that is indirect and destructive to prior manual edits.

Terrain in the Nebula Engine is voxel-based on a cubesphere planet, so "sculpting" means adding or removing columns of voxels to change the effective heightmap, or modifying voxel types to change biome appearance. The brush tools must project correctly onto the curved planetary surface, handle chunk boundaries transparently, and provide real-time visual feedback showing the brush footprint before any modification is committed. Performance is critical: a large brush may touch thousands of voxels per stroke, and the chunk mesh must regenerate fast enough to feel interactive.

## Solution

Implement a `TerrainBrush` system and UI in the `nebula_editor` crate with four brush operations: Raise, Lower, Smooth, and Flatten, plus a Biome Paint mode.

### Brush State

```rust
use bevy_ecs::prelude::*;

#[derive(Resource)]
pub struct TerrainBrushState {
    pub tool: TerrainBrushTool,
    pub radius: f32,
    pub strength: f32,
    pub flatten_target_height: i32,
    pub selected_biome: BiomeId,
    /// Whether the brush is currently being applied (mouse held down).
    pub painting: bool,
    /// Current brush center on the terrain surface.
    pub brush_center: Option<WorldPos>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerrainBrushTool {
    Raise,
    Lower,
    Smooth,
    Flatten,
    PaintBiome,
}

impl Default for TerrainBrushState {
    fn default() -> Self {
        Self {
            tool: TerrainBrushTool::Raise,
            radius: 5.0,
            strength: 1.0,
            flatten_target_height: 64,
            selected_biome: BiomeId(0),
            painting: false,
            brush_center: None,
        }
    }
}
```

### Brush UI Panel

```rust
pub fn terrain_brush_ui(
    ctx: &egui::Context,
    mut brush: ResMut<TerrainBrushState>,
    biome_registry: Res<BiomeRegistry>,
) {
    egui::SidePanel::left("terrain_brush")
        .default_width(220.0)
        .show(ctx, |ui| {
            ui.heading("Terrain Brush");
            ui.separator();

            // Tool selector
            ui.horizontal(|ui| {
                ui.selectable_value(&mut brush.tool, TerrainBrushTool::Raise, "Raise");
                ui.selectable_value(&mut brush.tool, TerrainBrushTool::Lower, "Lower");
            });
            ui.horizontal(|ui| {
                ui.selectable_value(&mut brush.tool, TerrainBrushTool::Smooth, "Smooth");
                ui.selectable_value(&mut brush.tool, TerrainBrushTool::Flatten, "Flatten");
            });
            ui.selectable_value(&mut brush.tool, TerrainBrushTool::PaintBiome, "Paint Biome");
            ui.separator();

            // Radius slider
            ui.horizontal(|ui| {
                ui.label("Radius:");
                ui.add(egui::Slider::new(&mut brush.radius, 1.0..=50.0).integer());
            });

            // Strength slider
            ui.horizontal(|ui| {
                ui.label("Strength:");
                ui.add(egui::Slider::new(&mut brush.strength, 0.1..=10.0));
            });

            // Flatten target (only shown for Flatten tool)
            if brush.tool == TerrainBrushTool::Flatten {
                ui.horizontal(|ui| {
                    ui.label("Target Height:");
                    ui.add(egui::DragValue::new(&mut brush.flatten_target_height));
                });
            }

            // Biome selector (only shown for PaintBiome tool)
            if brush.tool == TerrainBrushTool::PaintBiome {
                ui.separator();
                ui.label("Biome:");
                for id in 0..biome_registry.len() {
                    let biome_id = BiomeId(id as u16);
                    let name = biome_registry.get(biome_id).name.as_str();
                    let selected = brush.selected_biome == biome_id;
                    if ui.selectable_label(selected, name).clicked() {
                        brush.selected_biome = biome_id;
                    }
                }
            }
        });
}
```

### Brush Projection and Preview

Each frame, the system raycasts from the mouse to the terrain surface to determine the brush center. A preview circle is rendered on the terrain surface using a projective decal or a ring of line segments that follow the terrain contour:

```rust
pub fn terrain_brush_preview_system(
    mouse: Res<MouseState>,
    camera: Query<(&CameraProjection, &Transform), With<EditorCamera>>,
    chunks: Res<ChunkManager>,
    mut brush: ResMut<TerrainBrushState>,
    mut preview: ResMut<BrushPreviewMesh>,
) {
    let ray = compute_editor_ray(&mouse, &camera);
    match terrain_surface_raycast(&ray, &chunks) {
        Some(hit) => {
            brush.brush_center = Some(hit.world_pos);
            preview.generate_circle(hit.world_pos, brush.radius, &chunks);
        }
        None => {
            brush.brush_center = None;
            preview.clear();
        }
    }
}
```

### Brush Operations

When the mouse is held down, the brush applies its operation each frame to all voxel columns within the brush radius. A falloff function (linear or smooth-step) attenuates the effect toward the edge of the brush:

```rust
pub fn terrain_brush_apply_system(
    mouse: Res<MouseState>,
    mut brush: ResMut<TerrainBrushState>,
    mut chunks: ResMut<ChunkManager>,
    mut undo_stack: ResMut<UndoStack>,
    egui_integration: Res<EguiIntegration>,
) {
    if egui_integration.wants_pointer() { return; }

    if mouse.just_pressed(MouseButton::Left) {
        brush.painting = true;
    }
    if mouse.just_released(MouseButton::Left) {
        brush.painting = false;
    }

    if !brush.painting { return; }
    let Some(center) = brush.brush_center else { return };

    let affected_columns = get_columns_in_radius(center, brush.radius, &chunks);
    let mut batch = VoxelBatchOp::new();

    for column in &affected_columns {
        let distance = column_distance(center, column);
        let falloff = 1.0 - (distance / brush.radius).clamp(0.0, 1.0);
        let effective_strength = brush.strength * falloff;

        match brush.tool {
            TerrainBrushTool::Raise => {
                let layers = (effective_strength).ceil() as i32;
                for dy in 0..layers {
                    let coord = column.top().offset_y(dy);
                    let old = chunks.get_voxel(coord);
                    batch.push(coord, old, column.surface_type());
                    chunks.set_voxel(coord, column.surface_type());
                }
            }
            TerrainBrushTool::Lower => {
                let layers = (effective_strength).ceil() as i32;
                for dy in 0..layers {
                    let coord = column.top().offset_y(-dy);
                    let old = chunks.get_voxel(coord);
                    batch.push(coord, old, VoxelTypeId(0));
                    chunks.set_voxel(coord, VoxelTypeId(0));
                }
            }
            TerrainBrushTool::Smooth => {
                let avg_height = average_neighbor_height(column, &affected_columns);
                let current = column.height();
                let target = current + ((avg_height - current as f32) * effective_strength) as i32;
                adjust_column_height(column, target, &mut chunks, &mut batch);
            }
            TerrainBrushTool::Flatten => {
                let target = brush.flatten_target_height;
                adjust_column_height(column, target, &mut chunks, &mut batch);
            }
            TerrainBrushTool::PaintBiome => {
                // Modify the biome data for the surface voxel
                let coord = column.top();
                let old_biome = chunks.get_biome(coord);
                batch.push_biome(coord, old_biome, brush.selected_biome);
                chunks.set_biome(coord, brush.selected_biome);
            }
        }
    }

    if !batch.is_empty() {
        undo_stack.push(UndoAction::VoxelBatch(batch));
    }
}
```

### Column Height Adjustment

The `adjust_column_height` helper either adds solid voxels (if the target is higher than current) or removes them (if lower), recording each change in the batch for undo:

```rust
fn adjust_column_height(
    column: &VoxelColumn,
    target: i32,
    chunks: &mut ChunkManager,
    batch: &mut VoxelBatchOp,
) {
    let current = column.height();
    if target > current {
        for y in current..target {
            let coord = column.base().offset_y(y);
            let old = chunks.get_voxel(coord);
            batch.push(coord, old, column.surface_type());
            chunks.set_voxel(coord, column.surface_type());
        }
    } else if target < current {
        for y in target..current {
            let coord = column.base().offset_y(y);
            let old = chunks.get_voxel(coord);
            batch.push(coord, old, VoxelTypeId(0));
            chunks.set_voxel(coord, VoxelTypeId(0));
        }
    }
}
```

## Outcome

A `terrain_brush.rs` module in `crates/nebula_editor/src/` exporting `TerrainBrushState`, `TerrainBrushTool`, `terrain_brush_ui`, `terrain_brush_preview_system`, and `terrain_brush_apply_system`. Designers can select a terrain tool (Raise, Lower, Smooth, Flatten, Paint Biome), configure radius and strength via sliders, see a preview circle on the terrain, and click-drag to sculpt. All modifications are recorded in the undo stack as voxel batch operations.

## Demo Integration

**Demo crate:** `nebula-demo`

Raise, lower, smooth, and flatten brushes sculpt voxel terrain. Brush radius and strength are adjustable via sliders.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.18` | Resources, systems, queries for brush state and chunk access |
| `egui` | `0.31` | Tool selector panel, sliders, selectable labels for biome list |
| `glam` | `0.32` | Distance calculations, falloff math, vector projections on curved surface |
| `winit` | `0.30` | Mouse button events for paint start/stop |

Rust edition 2024. Depends on `nebula_voxel` (for `ChunkManager`, `VoxelTypeId`), `nebula_terrain` (for `BiomeRegistry`, `BiomeId`), `nebula_input`, `nebula_math`, and `nebula_ui`.

## Unit Tests

| Test Function | Description | Assertion |
|---------------|-------------|-----------|
| `test_raise_increases_terrain_height` | Set brush to Raise with strength 1.0, apply at a column of height 10. | Column height is now 11. |
| `test_lower_decreases_terrain_height` | Set brush to Lower with strength 1.0, apply at a column of height 10. | Column height is now 9. |
| `test_smooth_averages_neighbors` | Create a 3x3 area with center height 20 and neighbors at height 10, apply Smooth. | Center height moves toward the average (closer to 10). |
| `test_flatten_sets_target_height` | Set brush to Flatten with target 15, apply at a column of height 10. | Column height is now 15. |
| `test_brush_radius_affects_area` | Set radius to 5, apply Raise at a center point. | All columns within distance 5 of center are raised; columns at distance 6 are not. |
| `test_preview_circle_matches_brush_size` | Set radius to 8, hover over terrain. | The preview mesh radius matches the brush radius of 8. |
| `test_strength_falloff_at_edge` | Set radius to 5 and strength to 2.0, apply Raise. | Columns at the center are raised by 2; columns at the edge are raised by less. |
| `test_paint_biome_changes_surface_biome` | Set brush to PaintBiome with desert biome, apply at a grass column. | The surface voxel's biome ID is now desert. |
| `test_brush_operations_recorded_for_undo` | Apply any brush operation. | The `UndoStack` contains a `VoxelBatch` action with the affected coordinates. |
