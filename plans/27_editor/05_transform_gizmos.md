# Transform Gizmos

## Problem

Moving, rotating, and scaling entities by typing numbers into inspector fields is precise but slow. Designers need visual 3D handles — gizmos — that allow direct manipulation of an entity's transform by clicking and dragging in the viewport. Without gizmos, spatial adjustments require a guess-check-adjust loop of editing numeric fields, which is particularly painful for rotation and scale where the relationship between numbers and visual change is unintuitive.

The gizmos must render on top of the scene (ignoring depth) so they are always visible even when the entity is partially occluded by terrain or other objects. They must use the standard color convention (X = red, Y = green, Z = blue) shared by virtually all 3D editors. The gizmo must track the entity's 128-bit world position accurately but render in camera-relative local space to avoid floating-point precision issues at large distances from the origin.

## Solution

Implement a `TransformGizmo` rendering and interaction system in the `nebula_editor` crate.

### Gizmo Mode

```rust
use bevy_ecs::prelude::*;

#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub enum GizmoMode {
    Translate,
    Rotate,
    Scale,
}

impl Default for GizmoMode {
    fn default() -> Self {
        GizmoMode::Translate
    }
}

#[derive(Resource, Default)]
pub struct GizmoState {
    pub mode: GizmoMode,
    /// Which axis or plane is currently being dragged, if any.
    pub active_axis: Option<GizmoAxis>,
    /// Screen-space position where the drag started.
    pub drag_start: Option<[f32; 2]>,
    /// World-space transform at the start of the drag.
    pub initial_transform: Option<TransformSnapshot>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GizmoAxis {
    X,
    Y,
    Z,
    XY,
    XZ,
    YZ,
}

#[derive(Debug, Clone)]
pub struct TransformSnapshot {
    pub position: WorldPos,
    pub rotation: glam::Quat,
    pub scale: glam::Vec3,
}
```

### Mode Toggle System

```rust
pub fn gizmo_mode_toggle_system(
    keyboard: Res<KeyboardState>,
    mut gizmo: ResMut<GizmoState>,
    mode: Res<EditorMode>,
) {
    if *mode != EditorMode::Editor { return; }

    if keyboard.just_pressed(PhysicalKey::Code(KeyCode::KeyW)) {
        gizmo.mode = GizmoMode::Translate;
    }
    if keyboard.just_pressed(PhysicalKey::Code(KeyCode::KeyE)) {
        gizmo.mode = GizmoMode::Rotate;
    }
    if keyboard.just_pressed(PhysicalKey::Code(KeyCode::KeyR)) {
        gizmo.mode = GizmoMode::Scale;
    }
}
```

### Gizmo Geometry

Each gizmo mode defines a set of geometric primitives rendered in camera-relative space:

**Translate gizmo:**
- Three arrows (cones + shafts) along the local X, Y, Z axes, colored red, green, blue.
- Three small squares at the axis-pair intersections for planar movement (XY, XZ, YZ).
- A small cube at the center for free movement on all axes.

**Rotate gizmo:**
- Three torus rings around the X, Y, Z axes, colored red, green, blue.
- A screen-space circle for free rotation around the view axis.

**Scale gizmo:**
- Three lines with cube endpoints along the X, Y, Z axes, colored red, green, blue.
- A center cube for uniform scale.

The gizmo is rendered with a separate render pass that disables depth testing (`depth_compare: wgpu::CompareFunction::Always`) and depth writing, ensuring it draws on top of all scene geometry. The gizmo size is constant in screen space by scaling inversely with distance from the camera.

```rust
pub fn gizmo_render_system(
    selected: Res<SelectedEntity>,
    gizmo: Res<GizmoState>,
    camera: Query<(&CameraProjection, &Transform), With<EditorCamera>>,
    transforms: Query<(&WorldPos, &Rotation, &Scale)>,
    mut gizmo_renderer: ResMut<GizmoRenderer>,
) {
    let Some(entity) = selected.entity else {
        gizmo_renderer.clear();
        return;
    };

    let Ok((world_pos, rotation, scale)) = transforms.get(entity) else { return };
    let (proj, cam_transform) = camera.single();

    let local_pos = world_to_camera_relative(world_pos, cam_transform);
    let screen_size_factor = compute_screen_size_factor(local_pos.z, proj);

    match gizmo.mode {
        GizmoMode::Translate => {
            gizmo_renderer.draw_arrow(local_pos, glam::Vec3::X, RED, screen_size_factor);
            gizmo_renderer.draw_arrow(local_pos, glam::Vec3::Y, GREEN, screen_size_factor);
            gizmo_renderer.draw_arrow(local_pos, glam::Vec3::Z, BLUE, screen_size_factor);
            gizmo_renderer.draw_plane_handle(local_pos, GizmoAxis::XY, screen_size_factor);
            gizmo_renderer.draw_plane_handle(local_pos, GizmoAxis::XZ, screen_size_factor);
            gizmo_renderer.draw_plane_handle(local_pos, GizmoAxis::YZ, screen_size_factor);
        }
        GizmoMode::Rotate => {
            gizmo_renderer.draw_ring(local_pos, glam::Vec3::X, RED, screen_size_factor);
            gizmo_renderer.draw_ring(local_pos, glam::Vec3::Y, GREEN, screen_size_factor);
            gizmo_renderer.draw_ring(local_pos, glam::Vec3::Z, BLUE, screen_size_factor);
        }
        GizmoMode::Scale => {
            gizmo_renderer.draw_scale_handle(local_pos, glam::Vec3::X, RED, screen_size_factor);
            gizmo_renderer.draw_scale_handle(local_pos, glam::Vec3::Y, GREEN, screen_size_factor);
            gizmo_renderer.draw_scale_handle(local_pos, glam::Vec3::Z, BLUE, screen_size_factor);
        }
    }
}
```

### Drag Interaction

When the user clicks on a gizmo axis, the system enters drag mode. Mouse movement is projected onto the relevant axis or plane in world space, and the entity's transform is updated in real-time:

```rust
pub fn gizmo_interaction_system(
    mouse: Res<MouseState>,
    mut gizmo: ResMut<GizmoState>,
    mut transforms: Query<(&mut WorldPos, &mut Rotation, &mut Scale)>,
    selected: Res<SelectedEntity>,
    camera: Query<(&CameraProjection, &Transform), With<EditorCamera>>,
    mut undo_stack: ResMut<UndoStack>,
) {
    let Some(entity) = selected.entity else { return };

    if mouse.just_pressed(MouseButton::Left) {
        // Hit-test gizmo handles to determine active_axis
        if let Some(axis) = hit_test_gizmo(&mouse, &gizmo, &camera) {
            let (pos, rot, scl) = transforms.get(entity).unwrap();
            gizmo.active_axis = Some(axis);
            gizmo.drag_start = Some(mouse.position());
            gizmo.initial_transform = Some(TransformSnapshot {
                position: *pos,
                rotation: rot.0,
                scale: scl.0,
            });
        }
    }

    if mouse.is_pressed(MouseButton::Left) && gizmo.active_axis.is_some() {
        let delta = compute_axis_delta(&mouse, &gizmo, &camera);
        let (mut pos, mut rot, mut scl) = transforms.get_mut(entity).unwrap();
        let initial = gizmo.initial_transform.as_ref().unwrap();

        match gizmo.mode {
            GizmoMode::Translate => {
                *pos = initial.position.offset_by(delta);
            }
            GizmoMode::Rotate => {
                rot.0 = initial.rotation * delta_to_rotation(delta, gizmo.active_axis.unwrap());
            }
            GizmoMode::Scale => {
                scl.0 = initial.scale + delta;
            }
        }
    }

    if mouse.just_released(MouseButton::Left) && gizmo.active_axis.is_some() {
        // Record the transform change for undo
        let (pos, rot, scl) = transforms.get(entity).unwrap();
        if let Some(initial) = &gizmo.initial_transform {
            undo_stack.push(UndoAction::TransformChange {
                entity,
                old: initial.clone(),
                new: TransformSnapshot {
                    position: *pos,
                    rotation: rot.0,
                    scale: scl.0,
                },
            });
        }
        gizmo.active_axis = None;
        gizmo.drag_start = None;
        gizmo.initial_transform = None;
    }
}
```

## Outcome

A `transform_gizmo.rs` module in `crates/nebula_editor/src/` exporting `GizmoMode`, `GizmoState`, `GizmoAxis`, `TransformSnapshot`, `gizmo_mode_toggle_system`, `gizmo_render_system`, and `gizmo_interaction_system`. Selecting an entity displays 3D gizmo handles that can be dragged to translate, rotate, or scale the entity in real-time. Gizmos render on top of the scene and maintain a constant screen size. Mode switching is done with W/E/R keys.

## Demo Integration

**Demo crate:** `nebula-demo`

A selected entity shows RGB translation arrows. Dragging an arrow moves the entity along that axis. Rotation and scale via toolbar.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.18` | Entity queries, resources, system parameters for transform access |
| `glam` | `0.32` | Quaternion, Vec3, Mat4 for transform math and axis projection |
| `wgpu` | `28.0` | Depth-disabled render pass for gizmo overlay rendering |
| `winit` | `0.30` | Mouse position/button for drag interaction, key codes for mode toggle |

Rust edition 2024. Depends on `nebula_input`, `nebula_math` (for 128-bit to local-space conversion), `nebula_rendering` (for gizmo draw calls), and `nebula_ecs`.

## Unit Tests

| Test Function | Description | Assertion |
|---------------|-------------|-----------|
| `test_translate_gizmo_moves_entity` | Select an entity at origin, drag the X-axis arrow by 10 units. | Entity's `WorldPos.x` has increased by 10. |
| `test_rotate_gizmo_rotates_entity` | Select an entity, drag the Y-rotation ring by 90 degrees. | Entity's `Rotation` quaternion is approximately 90 degrees around Y. |
| `test_scale_gizmo_scales_entity` | Select an entity with uniform scale 1.0, drag the X-scale handle to 2.0. | Entity's `Scale.x` is `2.0`. |
| `test_gizmo_renders_over_scene` | Render a gizmo with an occluding object in front of the entity. | Gizmo pixels are visible in the framebuffer (depth test disabled). |
| `test_mode_toggle_w_e_r` | Press W, check mode; press E, check mode; press R, check mode. | `GizmoMode::Translate`, `GizmoMode::Rotate`, `GizmoMode::Scale` respectively. |
| `test_gizmo_hidden_when_no_selection` | Set `SelectedEntity::entity` to `None`, run the render system. | `gizmo_renderer` contains zero draw commands. |
| `test_drag_records_undo` | Drag the translate gizmo and release. | `UndoStack` contains a `TransformChange` action with the old and new positions. |
| `test_gizmo_constant_screen_size` | Position the camera at distances 10 and 100 from the entity. | The gizmo's screen-space pixel size is approximately equal at both distances. |
