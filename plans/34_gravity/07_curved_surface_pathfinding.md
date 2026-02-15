# Curved Surface Pathfinding

## Problem

AI entities on a cubesphere planet need to navigate from point A to point B across the surface. Standard A* pathfinding on a flat grid uses Euclidean distance as the heuristic, but on a sphere, Euclidean distance underestimates the actual travel distance (the straight line cuts through the planet interior). The correct heuristic is the great-circle (geodesic) distance along the surface. Additionally, the cubesphere has six faces with different coordinate frames, and paths frequently cross face boundaries — the pathfinding graph must handle these transitions without discontinuities. Terrain height variation adds another dimension: steep slopes (cliff faces, mountain ridges) may be impassable even though they connect adjacent surface cells. Gravity defines which surfaces are "walkable" — a surface normal too far from the local "up" direction (opposite gravity) is a wall, not a floor.

## Solution

### Surface Navigation Graph

The pathfinding system operates on a navigation graph derived from the cubesphere voxel terrain. Each walkable surface voxel is a node. Edges connect adjacent walkable voxels (6-connectivity on each face, plus cross-face connections at boundaries):

```rust
/// A node in the surface navigation graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NavNode {
    /// The cubesphere face this node is on.
    pub face: CubeFace,
    /// Position within the face grid (u, v coordinates).
    pub u: u32,
    pub v: u32,
    /// Height above the base terrain (for multi-level surfaces).
    pub height: i32,
}

/// An edge in the navigation graph with traversal cost.
#[derive(Debug, Clone, Copy)]
pub struct NavEdge {
    pub target: NavNode,
    /// Cost to traverse this edge. Based on surface distance, slope penalty, etc.
    pub cost: f32,
}
```

### Walkability Determination

A surface voxel is walkable if the angle between the surface normal and the local "up" direction (opposite gravity) is below a configurable threshold:

```rust
/// Maximum slope angle (in radians) that an AI entity can walk on.
/// ~45 degrees is a reasonable default for humanoid characters.
pub const MAX_WALKABLE_SLOPE: f32 = std::f32::consts::FRAC_PI_4;

/// Determine if a surface voxel is walkable given its normal and the local gravity.
pub fn is_walkable(surface_normal: Vec3, gravity_direction: Vec3, max_slope: f32) -> bool {
    let up = -gravity_direction;
    let cos_angle = surface_normal.dot(up);
    // cos(max_slope) gives the minimum dot product for walkability.
    cos_angle >= max_slope.cos()
}
```

### Great-Circle Distance Heuristic

The A* heuristic uses the geodesic (great-circle) distance on the planet surface rather than Euclidean distance. For two points on a sphere of radius R, the great-circle distance is `R * arccos(dot(a, b))` where `a` and `b` are unit vectors from the planet center:

```rust
/// Compute the great-circle (geodesic) distance between two surface points.
///
/// `pos_a` and `pos_b` are world positions on the planet surface.
/// `planet_center` is the planet's WorldPos.
/// `planet_radius` is the planet's radius in world units.
///
/// Returns the surface distance in world units (f64 for precision).
pub fn great_circle_distance(
    pos_a: &WorldPos,
    pos_b: &WorldPos,
    planet_center: &WorldPos,
    planet_radius: f64,
) -> f64 {
    // Compute unit direction vectors from planet center to each point.
    let da = glam::DVec3::new(
        (pos_a.x - planet_center.x) as f64,
        (pos_a.y - planet_center.y) as f64,
        (pos_a.z - planet_center.z) as f64,
    ).normalize();

    let db = glam::DVec3::new(
        (pos_b.x - planet_center.x) as f64,
        (pos_b.y - planet_center.y) as f64,
        (pos_b.z - planet_center.z) as f64,
    ).normalize();

    // Great-circle angle = arccos(dot(a, b)), clamped for numerical safety.
    let dot = da.dot(db).clamp(-1.0, 1.0);
    let angle = dot.acos();

    planet_radius * angle
}
```

### A* Implementation

The pathfinder uses A* with the great-circle heuristic:

```rust
use std::collections::{BinaryHeap, HashMap};
use std::cmp::Ordering;

#[derive(Debug, Clone)]
struct AStarEntry {
    node: NavNode,
    g_cost: f32,   // Cost from start to this node.
    f_cost: f32,   // g_cost + heuristic.
}

impl Ord for AStarEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        other.f_cost.partial_cmp(&self.f_cost).unwrap_or(Ordering::Equal)
    }
}
impl PartialOrd for AStarEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl PartialEq for AStarEntry {
    fn eq(&self, other: &Self) -> bool {
        self.f_cost == other.f_cost
    }
}
impl Eq for AStarEntry {}

/// Find a path from `start` to `goal` on the planet surface.
///
/// Returns `None` if no path exists (goal is unreachable).
/// Returns `Some(path)` as an ordered list of NavNodes from start to goal.
pub fn find_path(
    start: NavNode,
    goal: NavNode,
    nav_graph: &NavGraph,
    planet_center: &WorldPos,
    planet_radius: f64,
) -> Option<Vec<NavNode>> {
    let mut open = BinaryHeap::new();
    let mut came_from: HashMap<NavNode, NavNode> = HashMap::new();
    let mut g_scores: HashMap<NavNode, f32> = HashMap::new();

    g_scores.insert(start, 0.0);
    let h = heuristic(start, goal, nav_graph, planet_center, planet_radius);
    open.push(AStarEntry { node: start, g_cost: 0.0, f_cost: h });

    while let Some(current) = open.pop() {
        if current.node == goal {
            return Some(reconstruct_path(&came_from, goal));
        }

        let current_g = g_scores[&current.node];

        for edge in nav_graph.neighbors(current.node) {
            let tentative_g = current_g + edge.cost;
            let existing_g = g_scores.get(&edge.target).copied().unwrap_or(f32::INFINITY);

            if tentative_g < existing_g {
                came_from.insert(edge.target, current.node);
                g_scores.insert(edge.target, tentative_g);
                let h = heuristic(edge.target, goal, nav_graph, planet_center, planet_radius);
                open.push(AStarEntry {
                    node: edge.target,
                    g_cost: tentative_g,
                    f_cost: tentative_g + h,
                });
            }
        }
    }

    None // No path found.
}

fn heuristic(
    from: NavNode,
    to: NavNode,
    nav_graph: &NavGraph,
    planet_center: &WorldPos,
    planet_radius: f64,
) -> f32 {
    let pos_a = nav_graph.world_position(from);
    let pos_b = nav_graph.world_position(to);
    great_circle_distance(&pos_a, &pos_b, planet_center, planet_radius) as f32
}
```

### Cross-Face Boundary Handling

When the navigation graph is built, edges at cubesphere face boundaries are constructed using the cross-face neighbor finding system from plan 05 (stories 07/08). A node at the edge of face `PosX` connects to the corresponding node on face `PosZ` (or whichever face is adjacent). The edge cost accounts for the actual surface distance, which is slightly longer at face edges due to the cubesphere geometry.

### Slope Cost Penalty

Edges that traverse elevation changes receive a cost multiplier based on the slope angle:

```rust
/// Compute the traversal cost for an edge, factoring in distance and slope.
pub fn edge_cost(
    from_height: f32,
    to_height: f32,
    surface_distance: f32,
    slope_penalty_factor: f32,
) -> f32 {
    let height_diff = (to_height - from_height).abs();
    let slope = (height_diff / surface_distance).atan();
    let penalty = 1.0 + slope_penalty_factor * (slope / MAX_WALKABLE_SLOPE);
    surface_distance * penalty
}
```

Steep slopes that exceed `MAX_WALKABLE_SLOPE` are excluded from the graph entirely (no edge is created), forcing paths to go around them.

## Outcome

AI entities pathfind across the cubesphere surface using A* with a great-circle distance heuristic. The navigation graph respects cubesphere face boundaries, gravity-defined walkability, and terrain slope constraints. Paths are optimal in surface distance and avoid impassable slopes. `cargo test -p nebula-gravity` passes all curved surface pathfinding tests.

## Demo Integration

**Demo crate:** `nebula-demo`

A debug overlay shows pathfinding routes that follow the planet's curved surface, going around hills and avoiding cliffs — demonstrating gravity-aware AI navigation.

## Crates & Dependencies

- `bevy_ecs = "0.18"` — ECS framework for system scheduling, entity queries for AI entities requesting paths
- `glam = "0.32"` — `DVec3` for great-circle distance computation, `Vec3` for surface normal and gravity direction comparison
- `nebula-math` (internal) — `WorldPos` for i128 coordinate positions in distance calculations
- `nebula-gravity` (internal) — `LocalGravity` for walkability determination (gravity defines "up")
- `nebula-cubesphere` (internal) — `CubeFace`, cross-face neighbor lookup for graph edges at face boundaries
- `nebula-voxel` (internal) — Terrain height data and surface normal queries for graph construction

## Unit Tests

- **`test_path_found_on_flat_surface`** — Construct a flat 10x10 navigation grid (single cubesphere face, uniform height). Request a path from `(0, 0)` to `(9, 9)`. Assert the path is `Some` and contains at most `18` nodes (Manhattan distance on a grid). Assert start and goal are the first and last nodes respectively.

- **`test_path_goes_around_obstacles`** — Construct a 10x10 grid with a wall of non-walkable nodes at column 5, rows 0-8 (leaving row 9 open). Request a path from `(0, 5)` to `(9, 5)`. Assert the path is `Some`. Assert no node in the path has `u == 5` and `v < 9` (the wall). Assert the path goes through `(5, 9)` (the gap).

- **`test_path_crosses_cube_face_boundaries`** — Construct a navigation graph spanning two adjacent cubesphere faces (`PosX` and `PosZ`). Place the start node on `PosX` near the boundary and the goal on `PosZ` beyond the boundary. Assert the path is `Some`. Assert the path contains nodes on both faces. Assert consecutive nodes across the boundary are valid neighbors in the graph.

- **`test_steep_slope_is_avoided`** — Construct a grid where a direct path would cross a steep slope (height difference of 10 over a surface distance of 5, yielding slope > 63 degrees > `MAX_WALKABLE_SLOPE`). Place an alternative flat route around the slope. Assert the path is `Some` and takes the flat route. Assert no node in the path lies on the steep slope cells.

- **`test_distance_metric_is_surface_distance`** — Compute `great_circle_distance` between two points on opposite sides of a planet (antipodal points) with `planet_radius = 6_371_000.0`. Assert the distance is approximately `pi * 6_371_000.0` (half circumference). Compute Euclidean distance between the same two points. Assert Euclidean distance is `2 * 6_371_000.0` (diameter). Assert `great_circle_distance > euclidean_distance`. Verifies the surface metric is used, not straight-line distance.
