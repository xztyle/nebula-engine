# Voxel Painting Tools

## Problem

Building voxel worlds by hand — placing and removing individual blocks — is the most fundamental editor operation for a voxel engine. Without painting tools, level designers must either write procedural generation code for every structure or edit raw chunk data files, both of which are slow and error-prone. The tool must feel responsive: click and the voxel appears instantly, with visual feedback before the click confirms the action. Brush sizes larger than a single voxel are essential for shaping terrain and building large structures efficiently. Undo support is non-negotiable because voxel placement is destructive — overwriting existing voxels — and mistakes in a 3D environment are easy to make and hard to spot.

The tool must integrate with the engine's cubesphere-voxel planet topology, where voxel coordinates live on one of six cube faces and are projected onto a sphere. Placing a voxel near a face edge must correctly resolve to the neighboring face's coordinate space. The ghost preview overlay must respect this projection so the preview aligns with the actual surface.

## Solution

Implement a `VoxelPaintTool` system and supporting UI in the `nebula_editor` crate. The tool is active when Editor mode is enabled and the voxel brush is selected from the tool bar.

### Tool State

```rust
use bevy_ecs::prelude::*;

#[derive(Resource)]
pub struct VoxelPaintState {
    /// Currently selected voxel type from the palette.
    pub selected_type: VoxelTypeId,
    /// Brush shape and size.
    pub brush: BrushShape,
    /// Whether the ghost preview is currently showing.
    pub preview_active: bool,
    /// World position where the ghost preview is anchored.
    pub preview_position: Option<VoxelCoord>,
    /// The face normal of the surface being targeted.
    pub preview_face: Option<FaceNormal>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrushShape {
    Single,
    Cube3,   // 3x3x3
    Cube5,   // 5x5x5
    Sphere3, // radius-3 sphere
    Sphere5, // radius-5 sphere
}

impl BrushShape {
    /// Returns all voxel offsets relative to the center for this brush.
    pub fn offsets(&self) -> Vec<[i32; 3]> {
        match self {
            BrushShape::Single => vec![[0, 0, 0]],
            BrushShape::Cube3 => Self::cube_offsets(1),
            BrushShape::Cube5 => Self::cube_offsets(2),
            BrushShape::Sphere3 => Self::sphere_offsets(1),
            BrushShape::Sphere5 => Self::sphere_offsets(2),
        }
    }

    fn cube_offsets(radius: i32) -> Vec<[i32; 3]> {
        let mut offsets = Vec::new();
        for x in -radius..=radius {
            for y in -radius..=radius {
                for z in -radius..=radius {
                    offsets.push([x, y, z]);
                }
            }
        }
        offsets
    }

    fn sphere_offsets(radius: i32) -> Vec<[i32; 3]> {
        let mut offsets = Vec::new();
        let r2 = (radius * radius) as f32;
        for x in -radius..=radius {
            for y in -radius..=radius {
                for z in -radius..=radius {
                    let d2 = (x * x + y * y + z * z) as f32;
                    if d2 <= r2 + 0.5 {
                        offsets.push([x, y, z]);
                    }
                }
            }
        }
        offsets
    }
}
```

### Voxel Palette Panel

An egui side panel displays all registered voxel types from `VoxelTypeRegistry`. Each entry shows the type name and a colored preview swatch. Clicking an entry sets `VoxelPaintState::selected_type`.

```rust
pub fn voxel_palette_ui(
    ctx: &egui::Context,
    registry: Res<Arc<VoxelTypeRegistry>>,
    mut paint_state: ResMut<VoxelPaintState>,
) {
    egui::SidePanel::left("voxel_palette")
        .default_width(200.0)
        .show(ctx, |ui| {
            ui.heading("Voxel Palette");
            ui.separator();

            // Brush shape selector
            ui.horizontal(|ui| {
                ui.selectable_value(&mut paint_state.brush, BrushShape::Single, "1x1");
                ui.selectable_value(&mut paint_state.brush, BrushShape::Cube3, "3x3");
                ui.selectable_value(&mut paint_state.brush, BrushShape::Cube5, "5x5");
                ui.selectable_value(&mut paint_state.brush, BrushShape::Sphere3, "S3");
                ui.selectable_value(&mut paint_state.brush, BrushShape::Sphere5, "S5");
            });
            ui.separator();

            for id in 1..registry.len() {
                let voxel_id = VoxelTypeId(id as u16);
                let def = registry.get(voxel_id);
                let selected = paint_state.selected_type == voxel_id;
                if ui.selectable_label(selected, &def.name).clicked() {
                    paint_state.selected_type = voxel_id;
                }
            }
        });
}
```

### Raycast and Preview

Each frame, a system casts a ray from the mouse position into the voxel world. The ray marches through the chunk grid using a DDA (Digital Differential Analyzer) algorithm adapted for cubesphere coordinates. When the ray hits a solid voxel, the system records the hit position and the face normal (which adjacent empty cell the ray entered from).

For placement, the target coordinate is the empty cell adjacent to the hit face (place next to the surface). For removal, the target coordinate is the solid voxel itself.

The ghost preview renders semi-transparent voxels at all positions the brush would affect, using a distinct "preview" material with alpha blending:

```rust
pub fn voxel_preview_system(
    mouse: Res<MouseState>,
    camera: Query<(&CameraProjection, &Transform), With<EditorCamera>>,
    chunks: Res<ChunkManager>,
    mut paint_state: ResMut<VoxelPaintState>,
    mut preview_meshes: ResMut<PreviewMeshBuffer>,
) {
    let ray = compute_editor_ray(&mouse, &camera);
    match voxel_raycast(&ray, &chunks) {
        Some(hit) => {
            paint_state.preview_position = Some(hit.adjacent_coord);
            paint_state.preview_face = Some(hit.face_normal);
            paint_state.preview_active = true;

            let offsets = paint_state.brush.offsets();
            preview_meshes.clear();
            for offset in &offsets {
                let coord = hit.adjacent_coord.offset(offset);
                preview_meshes.add_ghost_voxel(coord);
            }
        }
        None => {
            paint_state.preview_active = false;
            preview_meshes.clear();
        }
    }
}
```

### Place and Remove

```rust
pub fn voxel_paint_system(
    mouse: Res<MouseState>,
    paint_state: Res<VoxelPaintState>,
    mut chunks: ResMut<ChunkManager>,
    mut undo_stack: ResMut<UndoStack>,
    egui_integration: Res<EguiIntegration>,
) {
    if egui_integration.wants_pointer() { return; }

    if mouse.just_pressed(MouseButton::Left) {
        if let Some(center) = paint_state.preview_position {
            let mut batch = VoxelBatchOp::new();
            for offset in paint_state.brush.offsets() {
                let coord = center.offset(&offset);
                let old_type = chunks.get_voxel(coord);
                batch.push(coord, old_type, paint_state.selected_type);
                chunks.set_voxel(coord, paint_state.selected_type);
            }
            undo_stack.push(UndoAction::VoxelBatch(batch));
        }
    }

    if mouse.just_pressed(MouseButton::Right) {
        if let Some(hit) = &paint_state.preview_position {
            // For removal, target the solid voxel itself (not adjacent)
            // Recast to get the solid voxel coordinate
            let mut batch = VoxelBatchOp::new();
            for offset in paint_state.brush.offsets() {
                let coord = hit.offset(&offset); // adjusted for removal target
                let old_type = chunks.get_voxel(coord);
                if old_type != VoxelTypeId(0) {
                    batch.push(coord, old_type, VoxelTypeId(0)); // 0 = Air
                    chunks.set_voxel(coord, VoxelTypeId(0));
                }
            }
            undo_stack.push(UndoAction::VoxelBatch(batch));
        }
    }
}
```

### Undo Integration

Each paint or remove operation creates a `VoxelBatchOp` containing the coordinate, old voxel type, and new voxel type for every affected cell. This batch is pushed onto the `UndoStack` (Story 08) as a single undoable action. Undoing replays the old types; redoing replays the new types.

## Outcome

A `voxel_paint.rs` module in `crates/nebula_editor/src/` exporting `VoxelPaintState`, `BrushShape`, `voxel_palette_ui`, `voxel_preview_system`, and `voxel_paint_system`. Designers can select a voxel type from a palette, choose a brush size, see a ghost preview, and left-click to place or right-click to remove voxels. All operations are undoable through the undo system.

## Demo Integration

**Demo crate:** `nebula-demo`

Place, remove, and paint voxel brushes with configurable size. Real-time terrain editing with immediate visual feedback.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.18` | Resources, systems, queries for tool state and chunk access |
| `egui` | `0.31` | Palette panel, brush size selector, selectable labels |
| `glam` | `0.32` | Ray math, vector operations for DDA voxel traversal |
| `winit` | `0.30` | Mouse button events for place/remove triggers |

Rust edition 2024. Depends on `nebula_voxel` (for `ChunkManager`, `VoxelTypeId`, `VoxelTypeRegistry`), `nebula_input`, `nebula_math` (for 128-bit raycast), and `nebula_ui`.

## Unit Tests

| Test Function | Description | Assertion |
|---------------|-------------|-----------|
| `test_single_voxel_placement` | Select stone, `BrushShape::Single`, click on an empty adjacent cell. | `chunks.get_voxel(coord)` returns stone's `VoxelTypeId`. |
| `test_single_voxel_removal` | Right-click on a solid voxel with `BrushShape::Single`. | `chunks.get_voxel(coord)` returns `VoxelTypeId(0)` (Air). |
| `test_brush_cube3_places_27_voxels` | Select `BrushShape::Cube3`, place at a coordinate. | 27 voxels in a 3x3x3 cube around the target are set to the selected type. |
| `test_brush_sphere_places_correct_count` | Select `BrushShape::Sphere3`, compute expected offsets. | The number of placed voxels matches `BrushShape::Sphere3.offsets().len()`. |
| `test_ghost_preview_appears_on_hover` | Move mouse over a solid surface in editor mode. | `paint_state.preview_active` is `true` and `preview_position` is `Some`. |
| `test_ghost_preview_disappears_off_surface` | Move mouse to empty sky (no voxel hit). | `paint_state.preview_active` is `false` and `preview_position` is `None`. |
| `test_undo_reverses_placement` | Place a voxel, then trigger undo. | The voxel at that coordinate reverts to its previous type (Air). |
| `test_voxel_type_matches_palette_selection` | Select grass from palette, place a voxel. | Placed voxel type equals the grass `VoxelTypeId`, not the previously selected type. |
