# Spherical Flow Direction

## Problem

On a cubesphere planet, gravity points toward the planet center, not along the world -Y axis. A flat plain on the +X face of the cube has gravity pointing in the -X direction in world space; on the +Y face, gravity is -Y; on the -Z face, gravity is +Z. The cellular automaton from story 02 needs a notion of "down" and "horizontal" for each voxel, and these directions vary continuously across the planet surface. At cube face boundaries and corners, the gravity direction rotates sharply and the local coordinate frame changes entirely. If the fluid system uses a naive world-space "down = -Y" assumption, water on 5 of the 6 cube faces will flow sideways or uphill. The engine needs a gravity-aware flow direction system that maps each voxel's 6 neighbors to "below", "above", and "lateral" categories based on the local gravity vector at that voxel's position.

## Solution

### Local Gravity Vector

The gravity system (from Epic 17, story 07) provides a gravity direction at any world position. For a spherical planet with center `C` and a voxel at position `P`, the local gravity direction is:

```rust
/// Returns the unit gravity vector at position `p` relative to planet center `c`.
/// This always points toward the center (downward in the planet's frame).
fn gravity_direction(planet_center: &WorldPosition, voxel_pos: &WorldPosition) -> [f64; 3] {
    let dx = (planet_center.x - voxel_pos.x) as f64;
    let dy = (planet_center.y - voxel_pos.y) as f64;
    let dz = (planet_center.z - voxel_pos.z) as f64;
    let len = (dx * dx + dy * dy + dz * dz).sqrt();
    if len < 1e-10 {
        return [0.0, -1.0, 0.0]; // Fallback at planet center
    }
    [dx / len, dy / len, dz / len]
}
```

The 128-bit `WorldPosition` subtraction is performed in i128 and then converted to f64 for the normalization. This gives sub-voxel directional accuracy even at planetary scale.

### Neighbor Classification

Each voxel has 6 face neighbors (in the standard voxel grid: +X, -X, +Y, -Y, +Z, -Z). The fluid system classifies each neighbor as "below", "above", or "lateral" by projecting the neighbor offset onto the local gravity vector:

```rust
/// Classification of a voxel neighbor relative to local gravity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlowDirection {
    /// Neighbor is below this cell (fluid flows here first).
    Below,
    /// Neighbor is above this cell (fluid flows away from here).
    Above,
    /// Neighbor is at roughly the same gravitational height (lateral spread).
    Lateral,
}

/// The 6 axis-aligned neighbor offsets in chunk-local space.
const NEIGHBOR_OFFSETS: [[i32; 3]; 6] = [
    [ 1,  0,  0], // +X
    [-1,  0,  0], // -X
    [ 0,  1,  0], // +Y
    [ 0, -1,  0], // -Y
    [ 0,  0,  1], // +Z
    [ 0,  0, -1], // -Z
];

/// Classify each of the 6 neighbors based on local gravity.
///
/// `gravity_dir` is the unit vector pointing toward the planet center
/// (the "down" direction) in the chunk's local coordinate frame.
///
/// A neighbor is classified as:
/// - Below if dot(offset, gravity) > threshold  (offset aligns with gravity)
/// - Above if dot(offset, gravity) < -threshold (offset opposes gravity)
/// - Lateral otherwise
pub fn classify_neighbors(gravity_dir: [f64; 3]) -> [FlowDirection; 6] {
    const THRESHOLD: f64 = 0.3; // ~72 degrees from gravity to be "lateral"
    let mut result = [FlowDirection::Lateral; 6];
    for (i, offset) in NEIGHBOR_OFFSETS.iter().enumerate() {
        let dot = offset[0] as f64 * gravity_dir[0]
                + offset[1] as f64 * gravity_dir[1]
                + offset[2] as f64 * gravity_dir[2];
        result[i] = if dot > THRESHOLD {
            FlowDirection::Below
        } else if dot < -THRESHOLD {
            FlowDirection::Above
        } else {
            FlowDirection::Lateral
        };
    }
    result
}
```

### Gravity-Projected Height

For lateral flow, fluid should still flow "downhill" on a slope. Two lateral neighbors at different distances from the planet center have different gravitational potential. The flow system computes the gravity-projected height of each neighbor:

```rust
/// Returns the gravitational height of a voxel position — its distance from
/// the planet center. Fluid flows from higher to lower height.
fn gravity_height(planet_center: &WorldPosition, voxel_pos: &WorldPosition) -> f64 {
    let dx = (voxel_pos.x - planet_center.x) as f64;
    let dy = (voxel_pos.y - planet_center.y) as f64;
    let dz = (voxel_pos.z - planet_center.z) as f64;
    (dx * dx + dy * dy + dz * dz).sqrt()
}
```

When spreading horizontally, the automaton preferentially flows toward the neighbor with the lowest gravity height (closest to the planet center), producing natural downhill flow on sloped terrain.

### Cube Face Boundary Handling

At cube face boundaries, the local coordinate frame rotates. A voxel on the edge of the +X face has a neighbor on the +Y face (or -Z, etc.). The cubesphere neighbor system (Epic 05, stories 06 and 07) handles the coordinate remapping. The fluid system consumes the neighbor's world position and computes gravity classification regardless of which face the neighbor is on:

```rust
/// Resolve the flow direction to a neighbor that may be on a different cube face.
/// Uses the cross-face neighbor system to get the actual world position of the
/// neighbor voxel, then classifies based on gravity at that position.
pub fn classify_cross_face_neighbor(
    planet_center: &WorldPosition,
    cell_pos: &WorldPosition,
    neighbor_pos: &WorldPosition,
) -> FlowDirection {
    let cell_height = gravity_height(planet_center, cell_pos);
    let neighbor_height = gravity_height(planet_center, neighbor_pos);
    let diff = neighbor_height - cell_height;

    if diff < -0.3 {
        FlowDirection::Below
    } else if diff > 0.3 {
        FlowDirection::Above
    } else {
        FlowDirection::Lateral
    }
}
```

### Cached Gravity Classification per Chunk

Recomputing gravity direction per-voxel per-tick is expensive. Since all voxels in a single chunk are close together and share approximately the same gravity direction, the system caches the neighbor classification per chunk:

```rust
/// Cached gravity-based flow classification for an entire chunk.
pub struct ChunkFlowDirections {
    /// The dominant gravity direction for this chunk (unit vector in local frame).
    pub gravity_dir: [f64; 3],
    /// Classification of the 6 neighbor directions for the bulk of the chunk.
    pub neighbor_classes: [FlowDirection; 6],
}
```

This is recomputed only when a chunk is loaded or when the planet moves (which effectively never happens for a static planet). Edge voxels at cube face seams use the per-voxel `classify_cross_face_neighbor` for accuracy.

## Outcome

The `nebula-fluid` crate exports `FlowDirection`, `classify_neighbors`, `gravity_height`, `classify_cross_face_neighbor`, and `ChunkFlowDirections`. Fluid simulation uses the local gravity vector to determine "down", enabling correct flow on all 6 faces of the cubesphere and across face boundaries. Running `cargo test -p nebula-fluid` passes all spherical flow direction tests. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

"Downhill" is computed from the gravity vector, not a fixed Y axis. Water flows toward the planet center, behaving naturally on any point of the cubesphere.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.18` | Access planet center and chunk positions as ECS resources/components |
| `glam` | `0.32` | Vector math for gravity computations and dot products |

Depends on Epic 05 (cubesphere neighbor system), Epic 17 story 07 (gravity system), and stories 01-02 of this epic.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn planet_center() -> WorldPosition {
        WorldPosition { x: 0, y: 0, z: 0 }
    }

    #[test]
    fn test_fluid_flows_toward_planet_center() {
        // Voxel on the +Y face, gravity should point in -Y direction (toward center)
        let center = planet_center();
        let voxel = WorldPosition { x: 0, y: 1_000_000, z: 0 };
        let grav = gravity_direction(&center, &voxel);

        // Gravity should point downward (negative Y)
        assert!(grav[1] < -0.99, "Gravity on +Y face should point toward -Y, got {:?}", grav);
        assert!(grav[0].abs() < 0.01);
        assert!(grav[2].abs() < 0.01);

        // Classify neighbors: -Y should be Below, +Y should be Above
        let classes = classify_neighbors(grav);
        // NEIGHBOR_OFFSETS[3] is [0, -1, 0] which is -Y
        assert_eq!(classes[3], FlowDirection::Below, "-Y neighbor should be Below on +Y face");
        // NEIGHBOR_OFFSETS[2] is [0, 1, 0] which is +Y
        assert_eq!(classes[2], FlowDirection::Above, "+Y neighbor should be Above on +Y face");
    }

    #[test]
    fn test_fluid_flows_on_cubesphere_surface_correctly() {
        // Voxel on the +X face: gravity points in -X direction
        let center = planet_center();
        let voxel = WorldPosition { x: 1_000_000, y: 0, z: 0 };
        let grav = gravity_direction(&center, &voxel);

        assert!(grav[0] < -0.99, "Gravity on +X face should point toward -X");
        let classes = classify_neighbors(grav);
        // NEIGHBOR_OFFSETS[1] is [-1, 0, 0] which is -X
        assert_eq!(classes[1], FlowDirection::Below, "-X neighbor should be Below on +X face");
        // NEIGHBOR_OFFSETS[0] is [1, 0, 0] which is +X
        assert_eq!(classes[0], FlowDirection::Above, "+X neighbor should be Above on +X face");
    }

    #[test]
    fn test_flow_crosses_cube_face_boundaries() {
        let center = planet_center();
        let radius: i128 = 1_000_000;

        // Cell on the edge of +X face, near the +Y face boundary
        let cell_pos = WorldPosition { x: radius, y: radius - 1, z: 0 };
        // Neighbor is on the +Y face
        let neighbor_pos = WorldPosition { x: radius - 1, y: radius, z: 0 };

        // Both are at roughly the same distance from center
        let cell_h = gravity_height(&center, &cell_pos);
        let neighbor_h = gravity_height(&center, &neighbor_pos);
        let height_diff = (cell_h - neighbor_h).abs();

        // They should be classified as Lateral since they're at similar heights
        let dir = classify_cross_face_neighbor(&center, &cell_pos, &neighbor_pos);
        assert_eq!(dir, FlowDirection::Lateral,
            "Neighbors at similar height across face boundary should be Lateral (diff={height_diff})");
    }

    #[test]
    fn test_flat_surface_spreads_evenly() {
        // On a perfectly flat surface (all neighbors equidistant from center),
        // gravity is purely radial. The 4 lateral neighbors should all be Lateral.
        let center = planet_center();
        let voxel = WorldPosition { x: 0, y: 1_000_000, z: 0 };
        let grav = gravity_direction(&center, &voxel);
        let classes = classify_neighbors(grav);

        let lateral_count = classes.iter().filter(|&&c| c == FlowDirection::Lateral).count();
        assert_eq!(lateral_count, 4, "Flat surface should have 4 lateral neighbors, got {lateral_count}");
    }

    #[test]
    fn test_fluid_collects_in_terrain_basins() {
        // A basin is a point closer to the planet center than its neighbors.
        // Fluid should flow toward the basin from all lateral directions.
        let center = planet_center();
        let radius: i128 = 1_000_000;

        // Basin voxel at the surface
        let basin = WorldPosition { x: 0, y: radius - 10, z: 0 };
        // Neighboring voxel slightly higher (further from center)
        let rim = WorldPosition { x: 1, y: radius, z: 0 };

        let basin_h = gravity_height(&center, &basin);
        let rim_h = gravity_height(&center, &rim);

        assert!(basin_h < rim_h, "Basin should be closer to center than rim");

        // From rim's perspective, the basin neighbor is Below (or Lateral trending down)
        let dir = classify_cross_face_neighbor(&center, &rim, &basin);
        assert!(
            dir == FlowDirection::Below || dir == FlowDirection::Lateral,
            "Fluid on rim should flow toward basin, got {dir:?}"
        );
    }

    #[test]
    fn test_gravity_direction_on_all_six_faces() {
        let center = planet_center();
        let r: i128 = 1_000_000;

        let faces = [
            (WorldPosition { x: r, y: 0, z: 0 }, 0, -1.0),  // +X face, gravity is -X
            (WorldPosition { x: -r, y: 0, z: 0 }, 0, 1.0),  // -X face, gravity is +X
            (WorldPosition { x: 0, y: r, z: 0 }, 1, -1.0),  // +Y face, gravity is -Y
            (WorldPosition { x: 0, y: -r, z: 0 }, 1, 1.0),  // -Y face, gravity is +Y
            (WorldPosition { x: 0, y: 0, z: r }, 2, -1.0),  // +Z face, gravity is -Z
            (WorldPosition { x: 0, y: 0, z: -r }, 2, 1.0),  // -Z face, gravity is +Z
        ];

        for (pos, axis, expected_sign) in &faces {
            let grav = gravity_direction(&center, pos);
            let component = grav[*axis];
            assert!(
                (component - expected_sign).abs() < 0.01,
                "Face at {:?}: expected gravity axis {} = {}, got {}",
                pos, axis, expected_sign, component
            );
        }
    }
}
```
