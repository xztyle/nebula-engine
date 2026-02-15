# Player Character Physics

## Problem

The player must move through a voxel world with physically plausible behavior: walking on surfaces, colliding with walls and ceilings, jumping, climbing stairs, and sliding along slopes. This requires more than a simple rigid body — a standard dynamic body would topple over, bounce off walls, and fail to climb single-voxel steps. Game-feel demands a **kinematic character controller** that uses physics for collision resolution but is driven by player input rather than forces. The controller must handle voxel terrain (axis-aligned 1m steps), slope limits, ground detection, and the engine's planet-relative gravity system where "down" varies based on the nearest gravity source. The capsule shape must be tuned for a voxel world where doorways are 2 blocks tall and corridors are 1 block wide.

## Solution

### Kinematic Character Controller

Use Rapier 0.32's built-in `KinematicCharacterController` which handles collision resolution, stair stepping, and slope detection. The player entity gets a kinematic rigid body (position-controlled, not force-controlled) and a capsule collider:

```rust
pub struct PlayerPhysics {
    pub body_handle: RigidBodyHandle,
    pub collider_handle: ColliderHandle,
    pub controller: KinematicCharacterController,
    pub grounded: bool,
    pub vertical_velocity: f32,
}

pub fn spawn_player_physics(
    physics: &mut PhysicsWorld,
    local_pos: glam::Vec3,
) -> PlayerPhysics {
    // Kinematic position-based body.
    let body = RigidBodyBuilder::kinematic_position_based()
        .translation(vector![local_pos.x, local_pos.y, local_pos.z])
        .build();
    let body_handle = physics.rigid_body_set.insert(body);

    // Capsule: total height 1.8m, radius 0.3m.
    // Rapier capsule is defined by half-height of the cylinder segment + radius.
    // Total height = 2 * half_height + 2 * radius = 2 * 0.6 + 2 * 0.3 = 1.8m.
    let collider = ColliderBuilder::capsule_y(0.6, 0.3)
        .friction(0.0) // Friction handled by movement logic, not the collider.
        .build();
    let collider_handle = physics.collider_set.insert_with_parent(
        collider, body_handle, &mut physics.rigid_body_set,
    );

    // Character controller with game-tuned parameters.
    let mut controller = KinematicCharacterController::default();
    controller.max_slope_climb_angle = std::f32::consts::FRAC_PI_4; // 45 degrees
    controller.min_slope_slide_angle = std::f32::consts::FRAC_PI_4; // Slide above 45 deg
    controller.autostep = Some(CharacterAutostep {
        max_height: CharacterLength::Absolute(0.5),  // Step up half-blocks
        min_width: CharacterLength::Absolute(0.3),   // Need room to step
        include_dynamic_bodies: false,
    });
    controller.snap_to_ground = Some(CharacterLength::Absolute(0.2));
    controller.offset = CharacterLength::Absolute(0.01); // Skin width

    PlayerPhysics {
        body_handle,
        collider_handle,
        controller,
        grounded: false,
        vertical_velocity: 0.0,
    }
}
```

### Movement System

Each `FixedUpdate` tick, the player movement system reads input, computes a desired movement vector, and lets Rapier resolve collisions:

```rust
fn player_movement_system(
    mut player: ResMut<PlayerPhysics>,
    mut physics: ResMut<PhysicsWorld>,
    input: Res<InputState>,
    gravity: Res<LocalGravity>, // Current gravity direction in local space
    time: Res<FixedTime>,
) {
    let dt = time.delta_seconds(); // 1/60
    let walk_speed = 5.0; // m/s
    let jump_impulse = 7.0; // m/s upward

    // Horizontal movement from input (relative to camera facing direction).
    let move_dir = input.movement_vector(); // normalized Vec2 from WASD
    let forward = input.camera_forward_xz(); // camera forward projected onto ground plane
    let right = input.camera_right_xz();

    let horizontal = (forward * move_dir.y + right * move_dir.x) * walk_speed;

    // Vertical movement: gravity + jump.
    if player.grounded {
        player.vertical_velocity = 0.0;
        if input.just_pressed(Action::Jump) {
            player.vertical_velocity = jump_impulse;
        }
    } else {
        // Apply gravity (direction from gravity source, magnitude from config).
        let gravity_accel = gravity.direction * gravity.magnitude;
        player.vertical_velocity += gravity_accel.y * dt;
        // Also apply horizontal gravity components for non-vertical gravity.
        // (On a sphere, gravity points toward center, not always straight down.)
    }

    let gravity_dir = gravity.direction;
    let vertical = gravity_dir * -player.vertical_velocity; // Negate because gravity_dir points "down"

    let desired_translation = vector![
        horizontal.x * dt + vertical.x * dt,
        player.vertical_velocity * dt,
        horizontal.z * dt + vertical.z * dt
    ];

    // Let Rapier resolve collisions.
    let corrected = player.controller.move_shape(
        dt,
        &physics.rigid_body_set,
        &physics.collider_set,
        &physics.query_pipeline,
        player.controller_collider_shape(),
        physics.rigid_body_set[player.body_handle].translation(),
        desired_translation,
        QueryFilter::default().exclude_rigid_body(player.body_handle),
        |_| {}, // collision event callback
    );

    // Apply the corrected movement.
    let body = &mut physics.rigid_body_set[player.body_handle];
    let new_pos = *body.translation() + corrected.translation;
    body.set_next_kinematic_translation(new_pos);

    // Update grounded state.
    player.grounded = corrected.grounded;
}
```

### Ground Detection

Rapier's `KinematicCharacterController::move_shape` returns a `grounded` flag based on its internal collision resolution. As a supplement, a downward raycast confirms ground contact:

```rust
fn ground_raycast(
    physics: &PhysicsWorld,
    player: &PlayerPhysics,
) -> bool {
    let body = &physics.rigid_body_set[player.body_handle];
    let origin = Point::from(*body.translation());
    let direction = vector![0.0, -1.0, 0.0]; // Local "down" — adjusted by gravity
    let max_distance = 1.0; // Capsule half-height + margin

    physics.query_pipeline.cast_ray(
        &physics.rigid_body_set,
        &physics.collider_set,
        &Ray::new(origin, direction),
        max_distance,
        true,
        QueryFilter::default().exclude_rigid_body(player.body_handle),
    ).is_some()
}
```

### Stair Stepping

The `CharacterAutostep` configuration allows the player to smoothly walk up voxel steps up to 0.5m tall without jumping. In a voxel world where blocks are 1m cubes, half-slab steps and terrain variations are common. The autostep checks for headroom above the step and sufficient width to stand, preventing the player from stepping up into walls.

### Capsule Dimensions

The capsule is 1.8m tall with a 0.3m radius, fitting through 1-block-wide corridors (1.0m > 0.6m diameter) and under 2-block-tall doorways (2.0m > 1.8m). The eye-height is at approximately 1.6m (offset from capsule center, which is at 0.9m).

## Outcome

The player moves through the voxel world with responsive, physically grounded character movement. Walking on surfaces feels solid, walls are impassable, stairs are climbed automatically, and jumping has the expected arc. `cargo test -p nebula-physics` passes all character controller tests.

## Demo Integration

**Demo crate:** `nebula-demo`

The player capsule has a physics body. It falls under gravity, slides along slopes, and stops at walls. Walking feels like a game character, not a floating camera.

## Crates & Dependencies

- `rapier3d = "0.32"` — `KinematicCharacterController`, `RigidBodyBuilder::kinematic_position_based()`, capsule collider, query pipeline raycasts
- `bevy_ecs = "0.18"` — ECS framework for systems, resources, and input integration
- `glam = "0.32"` — Vector math for movement calculations
- `nebula-input` (internal) — `InputState` resource with action-based queries and camera-relative movement vectors
- `nebula-physics` (internal, self) — `PhysicsWorld` resource, bridge systems

## Unit Tests

- **`test_player_stands_on_solid_ground`** — Create a flat voxel floor chunk. Spawn the player capsule 2m above the floor. Step physics 120 times. Assert the player's y-position has stabilized at approximately 0.9m (capsule center, half-height above the floor). The player should not fall through.

- **`test_player_cannot_walk_through_walls`** — Create a solid voxel wall 3 blocks tall. Position the player 1m from the wall facing it. Apply forward movement input for 60 ticks. Assert the player's position has not crossed the wall plane. The x/z position should be approximately `wall_position - capsule_radius - skin_width`.

- **`test_jump_applies_upward_velocity`** — Place the player on a flat floor (grounded). Trigger the jump action. Step physics for 1 tick. Assert `player.vertical_velocity > 0.0`. Step for 30 more ticks. Assert the player's y-position went above the initial rest position (the arc's peak) and then returned (or is returning) to the ground.

- **`test_ground_detection_on_flat_surface`** — Place the player on a flat floor. Step physics until stable. Assert `player.grounded == true`. Remove the floor voxel beneath the player (or move the player off an edge). Step physics. Assert `player.grounded == false`.

- **`test_stair_stepping_climbs_small_steps`** — Create a staircase of voxel blocks: block at y=0 for x<5, block at y=1 for x>=5 (a single 1m step). Position the player on the lower section, moving toward the step. Apply forward movement for 60 ticks. Assert the player's y-position has increased by approximately 1m — the autostep feature should have carried the player up without requiring a jump.

- **`test_slope_slide_above_max_angle`** — Create a steep slope (60 degrees, above the 45-degree max climb angle). Position the player at the base, apply upward movement. Step physics. Assert the player slides back down or cannot ascend — the slope angle limit prevents climbing.
