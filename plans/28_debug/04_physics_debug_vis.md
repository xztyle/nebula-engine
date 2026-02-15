# Physics Debug Visualization

## Problem

Physics engines operate on an invisible abstraction layer: colliders, rigid bodies, joints, contact manifolds, and raycasts exist only as mathematical constructs with no visual representation. When physics behaves unexpectedly — a player falls through terrain, a projectile passes through a wall, a character gets stuck on an invisible ledge, gravity seems wrong — the first question is always "what does the physics world actually look like?" Without debug visualization:

- **Collider misalignment is invisible** — The voxel mesh might not match the physics collider exactly. A visual mesh might extend past the collider boundary, letting the player see geometry they cannot collide with, or the collider might extend past the mesh, creating invisible walls.
- **Contact points are unknowable** — When two bodies collide, the exact contact points and normals determine how they bounce or slide. Without rendering these, force application bugs require stepping through code to diagnose.
- **Raycasts are black boxes** — A raycast that misses when it should hit (or vice versa) is nearly impossible to debug without seeing the ray and its intersection point.
- **Body types are indistinguishable** — Static terrain, dynamic objects, and kinematic platforms all look the same in the game view. Knowing which is which requires inspecting code or entity data.

Rapier provides a built-in debug renderer interface that outputs line segments. This story hooks into that interface and renders the output using the engine's line drawing capabilities.

## Solution

### Rapier Debug Render Pipeline

Rapier exposes a `DebugRenderPipeline` that produces colored line segments for all physics primitives. The engine wraps this in an ECS system:

```rust
use rapier3d::prelude::*;
use rapier3d::pipeline::DebugRenderPipeline;

pub struct PhysicsDebugState {
    pub enabled: bool,
    pub render_colliders: bool,
    pub render_rigid_body_centers: bool,
    pub render_contacts: bool,
    pub render_raycasts: bool,
    pub render_joints: bool,
}

impl Default for PhysicsDebugState {
    fn default() -> Self {
        Self {
            enabled: false,
            render_colliders: true,
            render_rigid_body_centers: true,
            render_contacts: true,
            render_raycasts: true,
            render_joints: true,
        }
    }
}
```

### Line Segment Collection

Implement Rapier's `DebugRenderBackend` trait to collect line segments into a buffer:

```rust
pub struct DebugLineBuffer {
    pub lines: Vec<DebugLine>,
}

pub struct DebugLine {
    pub start: [f32; 3],
    pub end: [f32; 3],
    pub color: [f32; 4],
}

impl DebugRenderBackend for DebugLineBuffer {
    fn draw_line(
        &mut self,
        _object: DebugRenderObject,
        a: Point<f32>,
        b: Point<f32>,
        color: [f32; 4],
    ) {
        self.lines.push(DebugLine {
            start: [a.x, a.y, a.z],
            end: [b.x, b.y, b.z],
            color,
        });
    }
}
```

### Color Coding by Body Type

The colors are overridden based on rigid body type to provide consistent, meaningful visual feedback:

| Body Type   | Color                    | RGB                 | Rationale                                    |
|-------------|--------------------------|---------------------|----------------------------------------------|
| Static      | Green                    | (0.2, 0.9, 0.2, 0.8) | Static terrain is the "ground truth," green = stable |
| Dynamic     | Cyan                     | (0.2, 0.8, 0.9, 0.8) | Dynamic objects are active and moving         |
| Kinematic   | Yellow                   | (0.9, 0.9, 0.2, 0.8) | Kinematic bodies are script-controlled, caution color |
| Contacts    | Red                      | (0.9, 0.2, 0.2, 1.0) | Contact points are collision events, alert color |
| Raycasts    | Magenta                  | (0.9, 0.2, 0.9, 0.8) | Raycasts are diagnostic queries, distinct color |
| Joints      | Orange                   | (0.9, 0.6, 0.2, 0.8) | Joints connect bodies, warm linking color     |

The color override is applied during the debug render pass:

```rust
fn color_for_body_type(body_type: RigidBodyType) -> [f32; 4] {
    match body_type {
        RigidBodyType::Fixed => [0.2, 0.9, 0.2, 0.8],
        RigidBodyType::Dynamic => [0.2, 0.8, 0.9, 0.8],
        RigidBodyType::KinematicPositionBased
        | RigidBodyType::KinematicVelocityBased => [0.9, 0.9, 0.2, 0.8],
    }
}
```

### Contact Point Rendering

Beyond Rapier's built-in debug rendering, the engine adds custom contact point visualization. For each active contact manifold, render:

1. A small cross or sphere at each contact point position.
2. A line along the contact normal, scaled by the penetration depth.
3. The contact is colored red, with brightness proportional to the contact impulse.

```rust
fn render_contact_points(
    narrow_phase: &NarrowPhase,
    line_buffer: &mut DebugLineBuffer,
) {
    for pair in narrow_phase.contact_pairs() {
        for manifold in &pair.manifolds {
            for contact in &manifold.points {
                let point = manifold.local_n1 * contact.dist + contact.local_p1.coords;
                let normal_end = point + manifold.local_n1.into_inner() * 0.3;

                line_buffer.lines.push(DebugLine {
                    start: [point.x, point.y, point.z],
                    end: [normal_end.x, normal_end.y, normal_end.z],
                    color: [0.9, 0.2, 0.2, 1.0],
                });
            }
        }
    }
}
```

### Raycast Visualization

When the engine performs raycasts (for player interaction, AI line-of-sight, etc.), the debug system logs them to a per-frame buffer:

```rust
pub struct RaycastDebugEntry {
    pub origin: [f32; 3],
    pub direction: [f32; 3],
    pub max_distance: f32,
    pub hit: Option<[f32; 3]>,
}

pub struct RaycastDebugBuffer {
    pub rays: Vec<RaycastDebugEntry>,
}
```

Each raycast is rendered as a magenta line from origin to hit point (or to `origin + direction * max_distance` if no hit). A small marker is drawn at the hit point.

### GPU Line Rendering

The collected `DebugLine` segments are uploaded to a GPU vertex buffer each frame and drawn with a simple line rendering pipeline:

```rust
pub fn upload_debug_lines(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    lines: &[DebugLine],
) -> (wgpu::Buffer, u32) {
    let vertices: Vec<LineVertex> = lines
        .iter()
        .flat_map(|line| {
            [
                LineVertex { position: line.start, color: line.color },
                LineVertex { position: line.end, color: line.color },
            ]
        })
        .collect();

    let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("physics_debug_lines"),
        contents: bytemuck::cast_slice(&vertices),
        usage: wgpu::BufferUsages::VERTEX,
    });

    (buffer, vertices.len() as u32)
}
```

The line pipeline uses `PrimitiveTopology::LineList`, no face culling, and depth testing with `LessEqual` so lines render on top of solid geometry (with a slight depth bias to avoid z-fighting).

### Toggle

A debug key (F7 by default) toggles the entire physics debug visualization. Individual categories (colliders, contacts, raycasts, joints) can be toggled via the debug panel in the egui editor.

## Outcome

Pressing F7 overlays physics debug visualization on the game view. All active colliders are rendered as colored wireframe outlines: static green, dynamic cyan, kinematic yellow. Contact points appear as red lines along their normals. Raycasts are rendered as magenta lines from origin to hit point. Rigid body centers are marked with small axis gizmos. The visualization updates every frame from Rapier's `DebugRenderPipeline` and is rendered using GPU line drawing with proper depth testing. Implementation lives in `crates/nebula-debug/src/physics_debug.rs` with line rendering support in `crates/nebula-render/src/line_renderer.rs`.

## Demo Integration

**Demo crate:** `nebula-demo`

Physics colliders are drawn as green wireframe overlays. Velocity vectors are red arrows. Gravity direction is blue. Contact points are yellow dots.

## Crates & Dependencies

- **`rapier3d = "0.32"`** — The `DebugRenderPipeline` and `DebugRenderBackend` trait for extracting debug line geometry from the physics world. Also `NarrowPhase` for contact manifold inspection.
- **`wgpu = "28.0"`** — Line rendering pipeline (`PrimitiveTopology::LineList`), vertex buffer creation for debug lines, and depth stencil configuration.
- **`egui = "0.31"`** — Toggle checkboxes for individual debug categories (colliders, contacts, raycasts, joints) in the debug panel.
- **`tracing = "0.1"`** — Logging debug visualization toggle events and line count statistics per frame.
- **`bytemuck = { version = "1", features = ["derive"] }`** — Safe casting of `LineVertex` arrays to byte slices for GPU buffer uploads.

## Unit Tests

- **`test_collider_shapes_match_physics_bodies`** — Create a physics world with a box collider (half-extents 1,1,1) at position (5, 0, 0). Run the debug render pipeline into a `DebugLineBuffer`. Assert the line buffer contains lines that form a box shape. Verify the line endpoints are within the expected range (centered at 5,0,0 with extents of 1 in each axis).

- **`test_contact_points_appear_at_collision`** — Create two box colliders overlapping by 0.1 units. Step the physics world. Run contact point rendering. Assert at least one `DebugLine` with the contact color (red) is produced, and the line's start point is within the overlap region.

- **`test_raycast_lines_visible`** — Perform a raycast from (0,10,0) downward with max distance 20. Add a ground plane at y=0. Assert the `RaycastDebugBuffer` contains an entry with origin (0,10,0), a hit point near (0,0,0), and the debug line is rendered as magenta.

- **`test_debug_vis_toggles`** — Create a `PhysicsDebugState` with `enabled: false`. Assert no debug lines are produced. Set `enabled: true` and assert debug lines are produced for existing physics bodies. Toggle individual categories (e.g., `render_contacts: false`) and assert contact lines are suppressed while collider lines remain.

- **`test_colors_match_body_type`** — Create three rigid bodies: one static, one dynamic, one kinematic. Run the debug renderer. Extract the line colors for each body. Assert static lines are green, dynamic lines are cyan, and kinematic lines are yellow, using approximate color matching (within 0.05 per channel).

- **`test_empty_physics_world`** — Run the debug renderer on a physics world with no bodies. Assert the `DebugLineBuffer` is empty and no panic occurs.

- **`test_line_buffer_upload`** — Create a `DebugLineBuffer` with 100 known lines. Call `upload_debug_lines` and assert the returned vertex count is 200 (2 vertices per line). Verify the buffer usage includes `VERTEX`.
