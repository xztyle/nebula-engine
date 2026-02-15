# Voxel Raycasting

## Problem

Voxel games require precise ray-voxel intersection for core gameplay mechanics: the player aims a crosshair at the world and needs to know which block they are looking at, which face of that block is targeted (for placing adjacent blocks), and how far away it is. This raycasting must work at the voxel grid level — not through Rapier's collision system — because it needs to operate on the full voxel data including chunks that may not have physics colliders loaded. Mining, block placement, line-of-sight checks, NPC targeting, and tool interactions all depend on fast, accurate voxel raycasting. The challenge is that Nebula Engine uses i128 world coordinates, so the raycast must step through an i128 voxel grid without floating-point precision loss, while still accepting a camera ray defined in f32 local space.

## Solution

### DDA Algorithm (Digital Differential Analyzer)

The classic DDA (also known as Amanatides & Woo) algorithm steps through a 3D grid one voxel at a time along a ray. It is exact — it visits every voxel the ray passes through in order, never skips one, and never visits one the ray misses. The algorithm tracks the parametric distance `t` to the next voxel boundary on each axis and always advances along the axis with the smallest `t`.

### Ray Definition

The ray originates from the player's eye position and extends in the camera's forward direction. The ray is defined in world space (i128 for the origin, f32/f64 for the direction):

```rust
pub struct VoxelRay {
    /// Origin in world coordinates (i128), at sub-voxel precision.
    pub origin: WorldPos,
    /// Sub-voxel offset within the origin voxel (0.0..1.0 per axis).
    pub sub_offset: glam::Vec3,
    /// Normalized direction vector.
    pub direction: glam::Vec3,
    /// Maximum ray distance in voxels (blocks).
    pub max_distance: f32,
}
```

### DDA Implementation

```rust
pub struct VoxelRaycastHit {
    /// World-space coordinate of the hit voxel.
    pub voxel_pos: WorldPos,
    /// The face normal of the entry face (which side of the voxel was hit).
    pub face_normal: IVec3,
    /// Distance from the ray origin to the hit point, in voxels.
    pub distance: f32,
    /// The type/ID of the voxel that was hit.
    pub voxel_type: VoxelId,
    /// Exact hit point within the voxel face (0.0..1.0 UV coordinates).
    pub hit_uv: glam::Vec2,
}

pub fn voxel_raycast(
    ray: &VoxelRay,
    world: &VoxelWorld,
) -> Option<VoxelRaycastHit> {
    // Current voxel position in i128 world space.
    let mut voxel = WorldPos {
        x: if ray.sub_offset.x >= 0.0 { ray.origin.x } else { ray.origin.x - 1 },
        y: if ray.sub_offset.y >= 0.0 { ray.origin.y } else { ray.origin.y - 1 },
        z: if ray.sub_offset.z >= 0.0 { ray.origin.z } else { ray.origin.z - 1 },
    };

    // Step direction: +1 or -1 per axis.
    let step_x: i128 = if ray.direction.x >= 0.0 { 1 } else { -1 };
    let step_y: i128 = if ray.direction.y >= 0.0 { 1 } else { -1 };
    let step_z: i128 = if ray.direction.z >= 0.0 { 1 } else { -1 };

    // Distance in t-units to cross one full voxel on each axis.
    let t_delta_x = (1.0 / ray.direction.x.abs()).min(f32::MAX);
    let t_delta_y = (1.0 / ray.direction.y.abs()).min(f32::MAX);
    let t_delta_z = (1.0 / ray.direction.z.abs()).min(f32::MAX);

    // Distance in t-units to the first voxel boundary on each axis.
    let t_max_x = if ray.direction.x > 0.0 {
        (1.0 - ray.sub_offset.x) * t_delta_x
    } else {
        ray.sub_offset.x * t_delta_x
    };
    // ... similarly for y, z

    let mut t_max = glam::Vec3::new(t_max_x, t_max_y, t_max_z);
    let t_delta = glam::Vec3::new(t_delta_x, t_delta_y, t_delta_z);
    let mut last_normal = IVec3::ZERO;
    let mut t = 0.0_f32;

    loop {
        // Check the current voxel.
        if let Some(voxel_data) = world.get_voxel(&voxel) {
            if voxel_data.is_solid() {
                return Some(VoxelRaycastHit {
                    voxel_pos: voxel,
                    face_normal: last_normal,
                    distance: t,
                    voxel_type: voxel_data.id(),
                    hit_uv: compute_hit_uv(ray, t, &last_normal),
                });
            }
        }

        // Advance along the axis with the smallest t_max.
        if t_max.x < t_max.y && t_max.x < t_max.z {
            t = t_max.x;
            t_max.x += t_delta.x;
            voxel.x += step_x;
            last_normal = IVec3::new(-step_x as i32, 0, 0);
        } else if t_max.y < t_max.z {
            t = t_max.y;
            t_max.y += t_delta.y;
            voxel.y += step_y;
            last_normal = IVec3::new(0, -step_y as i32, 0);
        } else {
            t = t_max.z;
            t_max.z += t_delta.z;
            voxel.z += step_z;
            last_normal = IVec3::new(0, 0, -step_z as i32);
        }

        if t > ray.max_distance {
            return None; // Exceeded maximum search distance.
        }
    }
}
```

### Key Design Decisions

**Stepping happens in i128 space.** The voxel coordinates are incremented/decremented using i128 arithmetic, so the raycast can traverse the entire 128-bit world without precision issues. Only the parametric `t` values and direction vector use f32, which is fine because they represent relative distances (small numbers).

**Face normal tracking.** The `last_normal` records which axis the ray crossed most recently. When a solid voxel is found, this normal tells the caller which face of the voxel the ray entered through — essential for block placement (place the new block on the hit face) and mining feedback (show the crack overlay on the correct face).

**Maximum distance.** Default `max_distance` is 8.0 for block targeting (standard reach distance), configurable up to 256 for line-of-sight checks and long-range tools.

**Starting inside a solid voxel.** If the ray origin is inside a solid voxel, the algorithm can either immediately return that voxel (useful for detecting suffocation) or skip it and continue to the first exit face (useful for escaping enclosed spaces). A parameter controls this behavior.

### Integration with Block Targeting

The player's crosshair target system runs every frame (not just on FixedUpdate):

```rust
fn block_target_system(
    player: Query<(&WorldPos, &CameraTransform), With<Player>>,
    voxel_world: Res<VoxelWorld>,
    mut target: ResMut<BlockTarget>,
) {
    let (pos, camera) = player.single();
    let ray = VoxelRay {
        origin: *pos,
        sub_offset: camera.eye_sub_offset(),
        direction: camera.forward(),
        max_distance: 8.0,
    };

    target.hit = voxel_raycast(&ray, &voxel_world);
}
```

`BlockTarget` stores the current hit result so that mining and placement systems can read it.

## Outcome

A `voxel_raycast` function exists that casts a ray through the voxel grid using the DDA algorithm, returning the first solid voxel hit along with its face normal, distance, and type. Block targeting, mining, placement, and line-of-sight systems use this function. `cargo test -p nebula-physics` passes all raycast tests including edge cases.

## Demo Integration

**Demo crate:** `nebula-demo`

A crosshair in the center of the screen casts a ray into the voxel world. The targeted voxel is highlighted with a wireframe cube outline showing its type and position.

## Crates & Dependencies

- `glam = "0.32"` — Vector math for ray direction, parametric stepping, and UV computation
- `bevy_ecs = "0.18"` — ECS framework for the block-targeting system, resources, and queries
- `nebula-math` (internal) — `WorldPos` (i128), `IVec3` for face normals
- `nebula-voxel` (internal) — `VoxelWorld` for looking up voxel data by world position, `VoxelId`
- `nebula-coords` (internal) — Coordinate conversions between world, chunk, and local spaces

## Unit Tests

- **`test_ray_hits_solid_voxel`** — Place a solid voxel at `WorldPos(5, 0, 0)`. Cast a ray from `WorldPos(0, 0, 0)` in the +x direction with `max_distance = 10`. Assert the hit is `Some` and `hit.voxel_pos == WorldPos(5, 0, 0)`. Assert `hit.distance` is approximately 5.0.

- **`test_ray_misses_empty_space`** — Create a world with no solid voxels along the ray path. Cast a ray with `max_distance = 100`. Assert the result is `None`. The ray should exhaust its maximum distance without hitting anything.

- **`test_hit_face_normal_correct`** — Place a solid voxel at `WorldPos(5, 0, 0)`. Cast a ray from the -x side (origin at `(0, 0, 0)`, direction `(1, 0, 0)`). Assert `hit.face_normal == IVec3(-1, 0, 0)` (the ray entered through the -x face). Repeat for all 6 faces by casting from each cardinal direction.

- **`test_max_distance_limits_search`** — Place a solid voxel at `WorldPos(20, 0, 0)`. Cast a ray with `max_distance = 10`. Assert result is `None` — the voxel is beyond the search limit. Increase `max_distance` to 25 and recast. Assert result is `Some`.

- **`test_ray_from_inside_solid_escapes`** — Place a 3x3x3 solid cube. Cast a ray starting from the center voxel with the "skip origin" flag. Assert the hit is on the first solid voxel encountered **after** exiting the origin voxel, not the origin voxel itself.

- **`test_diagonal_ray_crosses_voxels_correctly`** — Cast a ray in the direction `(1, 1, 0)` (45-degree diagonal in the xz plane). Place a solid voxel at `WorldPos(3, 3, 0)`. Assert the ray hits it. Verify the `distance` is approximately `3 * sqrt(2)`. This tests that the DDA correctly handles diagonal traversal across voxel boundaries.

- **`test_ray_returns_correct_voxel_type`** — Place voxels of different types: stone at `(5, 0, 0)`, dirt at `(10, 0, 0)`. Cast a ray in +x. Assert `hit.voxel_type` matches the stone type (first hit). Cast another ray starting past the stone. Assert the second hit returns the dirt type.
