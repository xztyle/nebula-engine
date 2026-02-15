# Physics Debug Visualization

## Problem

Physics bugs are notoriously difficult to diagnose without visual feedback. When a player falls through the floor, clips through a wall, or floats in mid-air, the developer needs to see exactly what the physics engine perceives: the shapes of colliders, the positions of contact points, the direction of raycasts, and the velocity vectors of moving bodies. Without debug visualization, developers must resort to logging coordinates and mentally reconstructing the 3D scene — a slow, error-prone process. The engine needs a toggle-able debug rendering overlay that draws Rapier's internal state using the engine's existing line and wireframe rendering capabilities, color-coded by object type, with zero impact on the physics simulation itself.

## Solution

### Debug State Resource

A resource controls the debug visualization state:

```rust
#[derive(Resource)]
pub struct PhysicsDebugState {
    /// Master toggle for all physics debug rendering.
    pub enabled: bool,
    /// Show collider wireframes.
    pub show_colliders: bool,
    /// Show contact points and normals.
    pub show_contacts: bool,
    /// Show raycast lines and hit points.
    pub show_raycasts: bool,
    /// Show velocity vectors on dynamic bodies.
    pub show_velocities: bool,
    /// Show rigid body AABBs (broad-phase).
    pub show_aabbs: bool,
    /// Show the physics island boundary.
    pub show_island_boundary: bool,
    /// Line width for wireframe rendering.
    pub line_width: f32,
}

impl Default for PhysicsDebugState {
    fn default() -> Self {
        Self {
            enabled: false,
            show_colliders: true,
            show_contacts: true,
            show_raycasts: true,
            show_velocities: true,
            show_aabbs: false,
            show_island_boundary: true,
            line_width: 1.5,
        }
    }
}
```

### Color Coding

Each type of physics object gets a distinct color for instant visual identification:

```rust
pub struct PhysicsDebugColors {
    /// Static colliders (terrain, walls): green.
    pub static_collider: [f32; 4],    // [0.0, 0.8, 0.2, 0.6]
    /// Dynamic rigid bodies: blue.
    pub dynamic_body: [f32; 4],       // [0.2, 0.4, 1.0, 0.6]
    /// Kinematic bodies (player, platforms): cyan.
    pub kinematic_body: [f32; 4],     // [0.0, 0.8, 0.8, 0.6]
    /// Contact points: red.
    pub contact_point: [f32; 4],      // [1.0, 0.0, 0.0, 1.0]
    /// Contact normals: orange.
    pub contact_normal: [f32; 4],     // [1.0, 0.5, 0.0, 1.0]
    /// Raycasts: yellow.
    pub raycast: [f32; 4],            // [1.0, 1.0, 0.0, 0.8]
    /// Raycast hit points: bright red.
    pub raycast_hit: [f32; 4],        // [1.0, 0.2, 0.2, 1.0]
    /// Velocity vectors: magenta.
    pub velocity: [f32; 4],           // [1.0, 0.0, 1.0, 0.8]
    /// Island boundary: white, semi-transparent.
    pub island_boundary: [f32; 4],    // [1.0, 1.0, 1.0, 0.3]
}
```

### Collider Wireframe Rendering

The debug system iterates over all colliders and emits wireframe draw commands using the engine's line-drawing API:

```rust
fn debug_render_colliders_system(
    debug: Res<PhysicsDebugState>,
    physics: Res<PhysicsWorld>,
    mut lines: ResMut<DebugLineBuffer>,
) {
    if !debug.enabled || !debug.show_colliders {
        return;
    }

    for (handle, collider) in physics.collider_set.iter() {
        let color = match collider.parent() {
            Some(body_handle) => {
                match physics.rigid_body_set.get(body_handle) {
                    Some(body) if body.is_dynamic() => COLORS.dynamic_body,
                    Some(body) if body.is_kinematic() => COLORS.kinematic_body,
                    _ => COLORS.static_collider,
                }
            }
            None => COLORS.static_collider,
        };

        let position = collider.position();
        let shape = collider.shape();

        match shape.shape_type() {
            ShapeType::Cuboid => {
                let cuboid = shape.as_cuboid().unwrap();
                emit_cuboid_wireframe(&mut lines, position, cuboid, color);
            }
            ShapeType::Capsule => {
                let capsule = shape.as_capsule().unwrap();
                emit_capsule_wireframe(&mut lines, position, capsule, color);
            }
            ShapeType::Ball => {
                let ball = shape.as_ball().unwrap();
                emit_sphere_wireframe(&mut lines, position, ball.radius, color);
            }
            ShapeType::ConvexPolyhedron => {
                let poly = shape.as_convex_polyhedron().unwrap();
                emit_convex_wireframe(&mut lines, position, poly, color);
            }
            // Voxel colliders: render occupied cell outlines.
            _ => {
                emit_aabb_wireframe(&mut lines, position, collider.compute_aabb(), color);
            }
        }
    }
}
```

For sparse voxel colliders (from story 04), rendering every individual voxel cell would be prohibitively expensive. Instead, the debug renderer draws the chunk AABB and optionally the outer surface wireframe (matching the visual mesh).

### Contact Point Rendering

After each physics step, contact points are extracted from the narrow phase:

```rust
fn debug_render_contacts_system(
    debug: Res<PhysicsDebugState>,
    physics: Res<PhysicsWorld>,
    mut lines: ResMut<DebugLineBuffer>,
) {
    if !debug.enabled || !debug.show_contacts {
        return;
    }

    for pair in physics.narrow_phase.contact_pairs() {
        for manifold in pair.manifolds.iter() {
            for contact in manifold.contacts() {
                let point = manifold.subshape_pos1 * contact.local_p1;
                // Draw a small cross at the contact point.
                emit_cross(&mut lines, &point, 0.05, COLORS.contact_point);
                // Draw the contact normal.
                let normal_end = point + manifold.local_n1 * 0.3;
                lines.push(point, normal_end, COLORS.contact_normal);
            }
        }
    }
}
```

### Raycast Debug Rendering

The voxel raycast system (story 06) and any Rapier raycasts can register their rays for debug rendering:

```rust
#[derive(Resource, Default)]
pub struct DebugRaycastBuffer {
    pub rays: Vec<DebugRay>,
}

pub struct DebugRay {
    pub origin: glam::Vec3,
    pub direction: glam::Vec3,
    pub max_distance: f32,
    pub hit_point: Option<glam::Vec3>,
}

fn debug_render_raycasts_system(
    debug: Res<PhysicsDebugState>,
    rays: Res<DebugRaycastBuffer>,
    mut lines: ResMut<DebugLineBuffer>,
) {
    if !debug.enabled || !debug.show_raycasts {
        return;
    }

    for ray in &rays.rays {
        let end = ray.origin + ray.direction * ray.max_distance;
        lines.push_line(ray.origin, end, COLORS.raycast);

        if let Some(hit) = ray.hit_point {
            emit_cross(&mut lines, &hit, 0.1, COLORS.raycast_hit);
        }
    }
}
```

### Toggle Input

The debug visualization toggles with a configurable key (default: F3):

```rust
fn physics_debug_toggle_system(
    input: Res<InputState>,
    mut debug: ResMut<PhysicsDebugState>,
) {
    if input.just_pressed(KeyCode::F3) {
        debug.enabled = !debug.enabled;
    }
}
```

Additional sub-toggles can be bound to shift+F3, ctrl+F3, etc., or exposed through the editor UI.

### Performance Isolation

All debug rendering is gated behind the `enabled` flag and checks it at the top of each system with an early return. When disabled, these systems execute in microseconds (a single branch). The debug systems never modify `PhysicsWorld` — they only read from it. This guarantees that enabling debug visualization cannot change physics behavior, preventing Heisenbugs where the act of observing changes the outcome.

Debug systems are placed in the `PostUpdate` schedule, after physics has completed, ensuring they visualize the final state of each frame.

## Outcome

Pressing F3 toggles a physics debug overlay showing collider wireframes (green/blue/cyan by type), contact points (red), raycasts (yellow), and velocity vectors (magenta). Developers can visually inspect the physics state in real time. The overlay has zero effect on simulation behavior. `cargo test -p nebula-physics` passes all debug visualization tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Pressing F2 toggles the physics debug overlay: green wireframes for colliders, red arrows for velocity, blue arrows for gravity direction, yellow dots for contacts.

## Crates & Dependencies

- `rapier3d = "0.32"` — Read-only access to collider shapes, contact manifolds, rigid body states, and AABBs for visualization
- `parry3d = "0.26"` — Shape type introspection (`as_cuboid`, `as_capsule`, `compute_aabb`) for wireframe generation
- `bevy_ecs = "0.18"` — ECS framework for systems, resources (`PhysicsDebugState`, `DebugLineBuffer`), and schedule ordering
- `glam = "0.32"` — Vector math for line endpoints, cross markers, and wireframe vertex calculations
- `nebula-render` (internal) — `DebugLineBuffer` resource consumed by the rendering pipeline to draw debug lines
- `nebula-input` (internal) — `InputState` for the F3 toggle key

## Unit Tests

- **`test_debug_vis_toggles_on_off`** — Create `PhysicsDebugState` with `enabled = false`. Simulate pressing F3. Assert `enabled == true`. Press F3 again. Assert `enabled == false`. The toggle must be idempotent and instant.

- **`test_collider_shapes_rendered`** — Enable debug rendering. Insert a cuboid collider and a capsule collider into the physics world. Run the debug render system. Assert the `DebugLineBuffer` contains lines (line count > 0). Verify that distinct line groups exist for each collider (e.g., a cuboid produces 12 edge lines, a capsule produces hemisphere arcs plus cylinder lines).

- **`test_contacts_shown_at_correct_positions`** — Enable contact rendering. Create two overlapping colliders that produce a contact. Step physics once. Run the contact debug system. Assert that at least one cross marker exists in the `DebugLineBuffer` at a position within the overlap region of the two colliders.

- **`test_debug_rendering_does_not_affect_physics`** — Create a physics world with a falling body. Step for 60 ticks with debug disabled — record final position A. Reset the world identically. Step for 60 ticks with debug enabled — record final position B. Assert A == B exactly (bitwise identical f32). Debug visualization must have zero side effects.

- **`test_disabled_debug_produces_no_lines`** — Set `debug.enabled = false`. Insert colliders and contacts. Run all debug systems. Assert `DebugLineBuffer` is empty. No lines should be generated when debug is off.

- **`test_velocity_vectors_rendered`** — Enable velocity rendering. Create a dynamic body with linear velocity `(5, 0, 0)`. Run the velocity debug system. Assert a line exists in the buffer starting at the body's position and extending in the +x direction proportional to the velocity magnitude.
