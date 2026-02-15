# Frustum in 128-bit Space

## Problem

Standard view frustum culling operates in 32-bit floating-point camera space and works well for nearby geometry (chunks, entities within a few kilometers). But the engine also needs to cull objects at planetary and interstellar distances -- distant planets, moons, stars, asteroid fields -- where f32 precision is wholly inadequate. A planet 50 billion km away, when represented in f32, collapses to a handful of distinguishable positions, making accurate inside/outside classification impossible. The engine needs a frustum defined entirely in 128-bit integer world space that can perform coarse culling of far-away objects before they ever reach the f32 rendering pipeline. This 128-bit frustum is not a replacement for the standard f32 frustum; it is an additional, earlier stage in the culling pipeline that eliminates objects that are astronomically far outside the view.

## Solution

### Plane Representation in i128

A plane in 3D is defined by a normal vector and a signed distance from the origin. In floating-point math this is `(normal: Vec3, d: f32)` where `normal.dot(point) + d > 0` means the point is on the positive (inside) side. In i128 space:

```rust
/// A plane in 128-bit integer space.
/// The plane equation is: normal.dot(point) + distance > 0 means "inside".
/// Normal components are stored as i64 to prevent overflow when dotted with i128 positions.
/// The dot product result fits in i128 because i64 * i128 can be computed
/// by widening to i128 before multiplication.
#[derive(Debug, Clone, Copy)]
pub struct Plane128 {
    pub normal: IVec3_64,
    pub distance: i128,
}

impl Plane128 {
    /// Classify a point relative to this plane.
    /// Returns positive if inside, negative if outside, zero if on the plane.
    pub fn signed_distance(&self, point: &WorldPosition) -> i128 {
        let nx = self.normal.x as i128;
        let ny = self.normal.y as i128;
        let nz = self.normal.z as i128;
        nx.wrapping_mul(point.x)
            .wrapping_add(ny.wrapping_mul(point.y))
            .wrapping_add(nz.wrapping_mul(point.z))
            .wrapping_add(self.distance)
    }

    /// Returns true if the point is on the inside (positive) half-space.
    pub fn contains_point(&self, point: &WorldPosition) -> bool {
        self.signed_distance(point) >= 0
    }
}
```

**Overflow handling:** The dot product of an `i64` normal and an `i128` position produces up to 191 bits, which overflows `i128`. To handle this, the normal vectors are kept small (unit-length scaled to a fixed magnitude, e.g., components in `[-1_000_000, 1_000_000]`), or the engine uses a two-tier check: first test the sector index (upper 96 bits dotted with the normal), and only refine to the full 128-bit position for sectors near the plane boundary. This keeps all intermediate products within i128 range for objects that are clearly inside or outside.

### Alternative: Sector-Granularity Pre-Check

For the coarse culling pass, it is often sufficient to test at sector granularity rather than exact millimeter precision:

```rust
impl Plane128 {
    /// Coarse sector-level test. Classifies the center of a sector.
    pub fn sector_side(&self, sector: &SectorIndex) -> PlaneSide {
        // Sector center = sector_index * 2^32 + 2^31 (midpoint).
        let half_sector: i128 = 1_i128 << 31;
        let center = WorldPosition {
            x: (sector.x << 32) + half_sector,
            y: (sector.y << 32) + half_sector,
            z: (sector.z << 32) + half_sector,
        };
        let d = self.signed_distance(&center);
        if d > 0 { PlaneSide::Inside }
        else if d < 0 { PlaneSide::Outside }
        else { PlaneSide::OnPlane }
    }
}

pub enum PlaneSide {
    Inside,
    Outside,
    OnPlane,
}
```

### The Frustum128 Struct

A standard frustum has six planes: near, far, left, right, top, bottom.

```rust
/// A view frustum defined in 128-bit world space for coarse culling
/// of distant objects (planets, stars, moons).
#[derive(Debug, Clone)]
pub struct Frustum128 {
    /// The six frustum planes, ordered: near, far, left, right, top, bottom.
    /// Each plane's normal points inward (toward the interior of the frustum).
    pub planes: [Plane128; 6],
}

/// Result of testing an AABB against the frustum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intersection {
    /// The object is entirely inside the frustum.
    Inside,
    /// The object is entirely outside the frustum.
    Outside,
    /// The object straddles one or more frustum planes.
    Intersecting,
}
```

### Point Test

```rust
impl Frustum128 {
    /// Test whether a point is inside all six frustum planes.
    pub fn contains_point(&self, point: &WorldPosition) -> bool {
        self.planes.iter().all(|plane| plane.contains_point(point))
    }
}
```

### AABB Test

An axis-aligned bounding box is defined by its min and max corners in world space:

```rust
/// An axis-aligned bounding box in 128-bit world space.
#[derive(Debug, Clone, Copy)]
pub struct AABB128 {
    pub min: WorldPosition,
    pub max: WorldPosition,
}

impl Frustum128 {
    /// Test an AABB against the frustum using the "p-vertex / n-vertex" method.
    /// For each plane, find the vertex of the AABB most in the direction of the
    /// plane normal (p-vertex) and the vertex most against it (n-vertex).
    /// - If the n-vertex is inside, the AABB is fully inside that plane.
    /// - If the p-vertex is outside, the AABB is fully outside that plane (and
    ///   therefore outside the frustum).
    /// - Otherwise, the AABB intersects that plane.
    pub fn contains_aabb(&self, aabb: &AABB128) -> Intersection {
        let mut all_inside = true;

        for plane in &self.planes {
            // p-vertex: for each axis, choose max if normal component > 0, else min.
            let px = if plane.normal.x >= 0 { aabb.max.x } else { aabb.min.x };
            let py = if plane.normal.y >= 0 { aabb.max.y } else { aabb.min.y };
            let pz = if plane.normal.z >= 0 { aabb.max.z } else { aabb.min.z };
            let p_vertex = WorldPosition { x: px, y: py, z: pz };

            // n-vertex: opposite corners.
            let nx = if plane.normal.x >= 0 { aabb.min.x } else { aabb.max.x };
            let ny = if plane.normal.y >= 0 { aabb.min.y } else { aabb.max.y };
            let nz = if plane.normal.z >= 0 { aabb.min.z } else { aabb.max.z };
            let n_vertex = WorldPosition { x: nx, y: ny, z: nz };

            if !plane.contains_point(&p_vertex) {
                // p-vertex is outside => entire AABB is outside this plane.
                return Intersection::Outside;
            }

            if !plane.contains_point(&n_vertex) {
                // n-vertex is outside => AABB straddles this plane.
                all_inside = false;
            }
        }

        if all_inside {
            Intersection::Inside
        } else {
            Intersection::Intersecting
        }
    }
}
```

### Building the Frustum from Camera State

The frustum is constructed from the camera's world position, orientation (forward, right, up vectors as unit-ish i64 vectors), field of view (encoded as rational tangent values), and near/far distances in millimeters:

```rust
impl Frustum128 {
    /// Build a frustum from camera parameters.
    ///
    /// - `position`: Camera world position (i128).
    /// - `forward`, `right`, `up`: Orientation vectors (i64, scaled to a
    ///   fixed magnitude like 1_000_000 to represent unit vectors).
    /// - `near`, `far`: Near and far plane distances in mm (i128).
    /// - `tan_half_fov_x`, `tan_half_fov_y`: Tangent of half the horizontal
    ///   and vertical field of view, encoded as `(numerator, denominator)` pairs
    ///   to avoid floating point.
    pub fn from_camera(
        position: &WorldPosition,
        forward: &IVec3_64,
        right: &IVec3_64,
        up: &IVec3_64,
        near: i128,
        far: i128,
        tan_half_fov: (i64, i64), // (numerator, denominator)
    ) -> Self {
        // Near plane: normal = forward, passes through position + forward * near
        // Far plane: normal = -forward, passes through position + forward * far
        // Left/right/top/bottom: normals computed from forward +/- right/up
        //   scaled by the FOV tangent ratio.

        // Implementation computes each plane's normal and distance
        // using purely integer arithmetic.
        // ... (detailed implementation)
        todo!()
    }
}
```

The full implementation of `from_camera` uses cross products and scaled integer arithmetic to compute the six plane normals without any floating-point operations. The tangent of the half-FOV is represented as a rational `(numerator, denominator)` pair to avoid f32 entirely in the 128-bit pipeline.

### Integration with the Culling Pipeline

The 128-bit frustum is the first stage of a two-stage culling pipeline:

1. **Stage 1 (i128):** `Frustum128::contains_aabb` tests planets, stars, and other massive distant objects. Objects classified as `Outside` are immediately discarded. Objects classified as `Inside` or `Intersecting` proceed to stage 2.
2. **Stage 2 (f32):** The standard camera-space `Frustum` (from `glam` or custom) performs precise culling on chunks and nearby entities using f32 positions relative to the camera.

This two-stage approach ensures that objects billions of kilometers away are culled without ever touching the f32 pipeline, while nearby objects get precise per-triangle-level culling.

## Outcome

The `nebula-coords` crate exports `Plane128`, `Frustum128`, `AABB128`, and the `Intersection` enum. `Frustum128::contains_point` tests a single world position against all six planes. `Frustum128::contains_aabb` performs the p-vertex/n-vertex test for axis-aligned bounding boxes. `Frustum128::from_camera` constructs the frustum from integer camera parameters. A demo can construct a frustum from a camera looking down the +Z axis, scatter 1,000 points/AABBs around the scene, and classify each as inside, outside, or intersecting, printing a summary of how many were culled.

## Demo Integration

**Demo crate:** `nebula-demo`

The demo defines a camera frustum in i128 space and tests which of the 1000 entities are inside it. The title shows `Frustum: 342/1000 visible`, changing as the virtual camera rotates.

## Crates & Dependencies

- **`nebula-math`** (workspace) — `IVec3_128`, `IVec3_64`, `WorldPosition`, integer vector operations
- **`nebula-coords`** (internal, same crate) — `SectorIndex`, `SectorCoord` for sector-granularity pre-check
- No external dependencies; all arithmetic is integer-only using Rust primitives

## Unit Tests

- **`test_point_inside_frustum`** — Construct a `Frustum128` from a camera at the origin looking down +Z with a 90-degree FOV and far plane at 1,000,000 mm. Test a point at `(0, 0, 500_000)` (500m ahead, on axis). Assert `contains_point` returns `true`.

- **`test_point_behind_camera`** — Same frustum as above. Test a point at `(0, 0, -100)` (behind the camera). Assert `contains_point` returns `false`.

- **`test_point_outside_left_plane`** — Same frustum. Test a point at `(-1_000_000, 0, 100)` (far to the left, barely in front). Assert `contains_point` returns `false` because it is outside the left plane.

- **`test_point_outside_right_plane`** — Same frustum. Test a point at `(1_000_000, 0, 100)`. Assert `contains_point` returns `false`.

- **`test_point_beyond_far_plane`** — Same frustum with far = 1,000,000 mm. Test a point at `(0, 0, 2_000_000)`. Assert `contains_point` returns `false`.

- **`test_aabb_fully_inside`** — Construct a frustum looking down +Z. Create an AABB with min `(−100, −100, 400_000)` and max `(100, 100, 600_000)` (a small box 400-600m ahead). Assert `contains_aabb` returns `Intersection::Inside`.

- **`test_aabb_fully_outside`** — Same frustum. Create an AABB with min `(−100, −100, −200)` and max `(100, 100, −100)` (entirely behind the camera). Assert `contains_aabb` returns `Intersection::Outside`.

- **`test_aabb_intersecting`** — Same frustum with near = 1000. Create an AABB that straddles the near plane: min `(−100, −100, 500)` and max `(100, 100, 1500)`. Assert `contains_aabb` returns `Intersection::Intersecting`.

- **`test_degenerate_frustum_zero_volume`** — Construct a frustum where near == far (zero depth). Test a point that would be inside a normal frustum. Assert `contains_point` returns `false`, because the frustum has zero volume and no point can satisfy both the near and far plane simultaneously.

- **`test_large_distance_culling`** — Construct a frustum at position `(0, 0, 0)` looking down +Z with far plane at `1_i128 << 60` (~1.15 * 10^18 mm = ~1.15 billion km). Test a planet AABB centered at `(0, 0, 1_i128 << 59)` with radius `1_i128 << 40`. Assert `contains_aabb` returns `Intersection::Inside`. Test another AABB at `(1_i128 << 62, 0, 0)` (way off to the right). Assert it returns `Intersection::Outside`. This validates that the frustum works correctly at interstellar scales.
