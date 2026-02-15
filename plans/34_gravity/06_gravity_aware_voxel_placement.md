# Gravity-Aware Voxel Placement

## Problem

On a flat-world game, voxels align to world axes: X, Y, Z. On a cubesphere planet, "down" varies by position — a block placed at the equator has a different "down" than one placed at a pole or at 45 degrees latitude. If the build grid remains aligned to world axes, blocks placed on the far side of the planet appear rotated or tilted relative to the local surface. The player expects blocks to sit flat on the ground beneath their feet regardless of where on the planet they stand. The voxel placement system must compute a local coordinate frame aligned to the gravity direction at the placement point, snap the block to a grid within that frame, and ensure adjacent blocks across the cubesphere surface connect seamlessly without gaps or overlaps.

## Solution

### Local Placement Frame

At any point on the planet surface, define a local coordinate frame where:
- **Up**: opposite to the local gravity direction (radially outward from planet center)
- **Forward**: an arbitrary but consistent tangent direction (derived from the cubesphere face)
- **Right**: the cross product of up and forward

```rust
use glam::{Vec3, Mat3};

/// A local coordinate frame aligned to the surface gravity at a specific point.
#[derive(Debug, Clone, Copy)]
pub struct PlacementFrame {
    /// "Up" direction — away from planet center, opposite gravity.
    pub up: Vec3,
    /// "Forward" tangent direction along the surface.
    pub forward: Vec3,
    /// "Right" tangent direction along the surface.
    pub right: Vec3,
}

/// Compute the placement frame at a world position given the local gravity direction.
///
/// The gravity direction points "down" (toward the planet center).
/// The frame's up is the negation of gravity direction.
/// Forward and right are derived to form an orthonormal basis.
pub fn compute_placement_frame(gravity_direction: Vec3) -> PlacementFrame {
    let up = -gravity_direction;

    // Choose a reference vector that is not parallel to up.
    // Use world Z unless up is nearly parallel to Z, then use X.
    let reference = if up.dot(Vec3::Z).abs() < 0.99 {
        Vec3::Z
    } else {
        Vec3::X
    };

    let right = up.cross(reference).normalize();
    let forward = right.cross(up).normalize();

    PlacementFrame { up, forward, right }
}
```

### Grid-Aligned Snapping

The player's crosshair targets a voxel face on the surface. The placement system computes the frame at that point and snaps the new block position to the local grid:

```rust
/// Snap a world position to the nearest voxel grid point in the local placement frame.
///
/// `origin` is the planet center (or chunk origin for local grids).
/// `voxel_size` is the edge length of one voxel in world units.
pub fn snap_to_local_grid(
    world_pos: Vec3,
    frame: &PlacementFrame,
    origin: Vec3,
    voxel_size: f32,
) -> Vec3 {
    // Express the position relative to origin in the local frame.
    let offset = world_pos - origin;
    let local_up = offset.dot(frame.up);
    let local_fwd = offset.dot(frame.forward);
    let local_right = offset.dot(frame.right);

    // Snap each local coordinate to the voxel grid.
    let snapped_up = (local_up / voxel_size).round() * voxel_size;
    let snapped_fwd = (local_fwd / voxel_size).round() * voxel_size;
    let snapped_right = (local_right / voxel_size).round() * voxel_size;

    // Reconstruct world position from snapped local coordinates.
    origin
        + frame.up * snapped_up
        + frame.forward * snapped_fwd
        + frame.right * snapped_right
}
```

### Cubesphere Face Alignment

On each face of the cubesphere, the voxel grid aligns to that face's natural tangent and bitangent directions. This means:
- On the +Y face (north pole), blocks align to the X and Z axes.
- On the +X face (equator, 0 degrees longitude), blocks align to Y and Z.
- Within a face, the grid is uniform and rectilinear.

At face boundaries, adjacent faces have different tangent frames. The engine handles this by:
1. Each chunk knows which cubesphere face it belongs to.
2. Blocks within a chunk use that face's tangent frame.
3. At face edges, blocks from adjacent faces meet at an angle (inherent to the cubesphere geometry).
4. A seam-stitching pass (from the meshing system, plan 07) ensures mesh vertices align at face boundaries.

### Block Orientation Storage

Each placed block stores its orientation as a rotation relative to the cubesphere face frame:

```rust
/// Orientation of a placed voxel block.
/// Stored as a compact rotation index (24 possible orientations for a cube).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockOrientation(pub u8);

impl BlockOrientation {
    /// Identity orientation — block axes align with the local placement frame.
    pub const IDENTITY: Self = Self(0);

    /// Compute the world-space rotation matrix for this orientation
    /// given the placement frame.
    pub fn to_world_rotation(&self, frame: &PlacementFrame) -> Mat3 {
        let local_rotation = Self::local_rotation_matrix(self.0);
        let frame_matrix = Mat3::from_cols(frame.right, frame.up, frame.forward);
        frame_matrix * local_rotation
    }

    fn local_rotation_matrix(index: u8) -> Mat3 {
        // 24 rotation matrices for the cube rotation group.
        // Index 0 = identity, indices 1-23 = the other orientations.
        // Implementation: lookup table of the 24 elements of the
        // chiral octahedral symmetry group.
        todo!("lookup table")
    }
}
```

### Integration with Gravity System

The placement system queries the `LocalGravity` component of the player entity to determine the current gravity direction. This ensures blocks are always placed relative to the player's current "down":

```rust
fn voxel_placement_system(
    input: Res<PlacementInput>,
    player: Query<(&WorldPos, &LocalGravity), With<Player>>,
    mut voxel_world: ResMut<VoxelWorld>,
) {
    if !input.place_requested {
        return;
    }

    let (player_pos, gravity) = player.single();
    let frame = compute_placement_frame(gravity.direction);
    let target = snap_to_local_grid(
        input.target_world_pos,
        &frame,
        input.chunk_origin,
        1.0, // 1m voxel size
    );

    voxel_world.place_block(target, BlockOrientation::IDENTITY, input.block_type);
}
```

## Outcome

Voxel placement uses a gravity-aligned local coordinate frame so blocks always sit flat relative to the local surface normal. The `PlacementFrame` is computed from the `LocalGravity` direction at the placement point. Grid snapping operates in the local frame, producing correct alignment at any latitude, longitude, or cubesphere face. `BlockOrientation` stores compact rotation indices relative to the local frame. `cargo test -p nebula-gravity` passes all gravity-aware placement tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Voxels placed in mid-air fall toward the nearest gravity source. Dropping a block off a cliff causes it to tumble down and land on the surface below.

## Crates & Dependencies

- `bevy_ecs = "0.18"` — ECS framework for `Query`, `Res`, `ResMut`, system scheduling, `Component`
- `glam = "0.32"` — `Vec3` for direction vectors, `Mat3` for frame matrices and rotation, `cross`/`dot` operations
- `nebula-gravity` (internal) — `LocalGravity` component for gravity direction at the placement point
- `nebula-voxel` (internal) — `VoxelWorld` resource for block placement, `BlockType`
- `nebula-cubesphere` (internal) — Face tangent/bitangent directions for cubesphere-aligned grids

## Unit Tests

- **`test_blocks_align_to_surface_normal`** — Compute a `PlacementFrame` with gravity direction `(0, -1, 0)` (standard downward). Assert `frame.up == (0, 1, 0)`. Snap a position to the grid. Assert the snapped block's up axis is `(0, 1, 0)`. Repeat with gravity direction `(-1, 0, 0)`. Assert `frame.up == (1, 0, 0)`. Verifies blocks align to the local surface normal, not world Y.

- **`test_block_grid_rotates_around_planet`** — Compute placement frames at four points around a planet: `(R, 0, 0)`, `(0, R, 0)`, `(-R, 0, 0)`, `(0, -R, 0)` where gravity direction points toward the origin from each point. Assert each frame has a different `up` direction corresponding to the radial outward direction. Assert `up` vectors at opposite points are anti-parallel. Verifies the grid rotates with the surface.

- **`test_blocks_at_different_latitudes_have_different_orientations`** — Compute frames at the equator (gravity direction `(-1, 0, 0)`) and at 45 degrees latitude (gravity direction approximately `(-0.707, -0.707, 0)`). Assert the two frames have different `up` directions. Compute the angle between the up vectors and assert it is approximately 45 degrees.

- **`test_blocks_at_poles_correctly_oriented`** — Compute a frame at the north pole where gravity direction is `(0, -1, 0)`. Assert `frame.up == (0, 1, 0)`. Assert `frame.forward` and `frame.right` are perpendicular to up and to each other (dot products are approximately 0). Verifies no degenerate frame at the pole.

- **`test_adjacent_blocks_connect_seamlessly`** — Place two blocks at adjacent grid positions within the same placement frame: one at `snap_to_local_grid(pos_a, ...)` and one at `snap_to_local_grid(pos_a + frame.right * voxel_size, ...)`. Assert the distance between the two snapped positions equals exactly `voxel_size` (within f32 epsilon). Verifies adjacent blocks form a continuous grid with no gaps.
