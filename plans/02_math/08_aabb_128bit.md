# AABB 128-bit

## Problem

Axis-Aligned Bounding Boxes (AABBs) are the workhorse spatial primitive for broad-phase collision detection, frustum culling, chunk bounds, LOD selection, and spatial indexing (octrees, BVH). In the Nebula Engine, AABBs must operate in the full i128 world coordinate space so that chunk boundaries, planet bounds, and solar-system-scale collision regions can be represented exactly without floating-point imprecision.

## Solution

Define an `Aabb128` struct using `WorldPosition` for its corners:

```rust
/// Axis-Aligned Bounding Box in i128 world space.
///
/// Invariant: min.x <= max.x, min.y <= max.y, min.z <= max.z.
/// The constructor enforces this by swapping components if needed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Aabb128 {
    pub min: WorldPosition,
    pub max: WorldPosition,
}
```

### Constructors

```rust
impl Aabb128 {
    /// Create an AABB from two corners. Automatically sorts
    /// components so that min <= max on every axis.
    pub fn new(a: WorldPosition, b: WorldPosition) -> Self {
        Self {
            min: WorldPosition::new(
                a.x.min(b.x),
                a.y.min(b.y),
                a.z.min(b.z),
            ),
            max: WorldPosition::new(
                a.x.max(b.x),
                a.y.max(b.y),
                a.z.max(b.z),
            ),
        }
    }

    /// Create an AABB from a center point and half-extents.
    pub fn from_center_half_extents(
        center: WorldPosition,
        half: Vec3I128,
    ) -> Self {
        Self {
            min: center - half,
            max: center + half,
        }
    }
}
```

### Methods

```rust
impl Aabb128 {
    /// Returns true if the point lies inside or on the boundary.
    pub fn contains_point(&self, p: WorldPosition) -> bool {
        p.x >= self.min.x && p.x <= self.max.x
            && p.y >= self.min.y && p.y <= self.max.y
            && p.z >= self.min.z && p.z <= self.max.z
    }

    /// Returns true if this AABB overlaps with other
    /// (including touching edges/faces).
    pub fn intersects(&self, other: &Aabb128) -> bool {
        self.min.x <= other.max.x && self.max.x >= other.min.x
            && self.min.y <= other.max.y && self.max.y >= other.min.y
            && self.min.z <= other.max.z && self.max.z >= other.min.z
    }

    /// Returns the smallest AABB enclosing both self and other.
    pub fn union(&self, other: &Aabb128) -> Aabb128 {
        Aabb128 {
            min: WorldPosition::new(
                self.min.x.min(other.min.x),
                self.min.y.min(other.min.y),
                self.min.z.min(other.min.z),
            ),
            max: WorldPosition::new(
                self.max.x.max(other.max.x),
                self.max.y.max(other.max.y),
                self.max.z.max(other.max.z),
            ),
        }
    }

    /// Returns the volume in cubic millimeters (i128).
    ///
    /// # Overflow
    /// Each dimension can be up to 2×i128::MAX. The product of three
    /// such values vastly exceeds i128. For large AABBs (e.g., planet-
    /// scale), use checked_volume() or compute volume in f64.
    pub fn volume(&self) -> i128 {
        let dx = self.max.x - self.min.x;
        let dy = self.max.y - self.min.y;
        let dz = self.max.z - self.min.z;
        dx * dy * dz
    }

    /// Returns the center point of the AABB.
    /// Uses integer division (truncates toward zero).
    pub fn center(&self) -> WorldPosition {
        WorldPosition::new(
            self.min.x + (self.max.x - self.min.x) / 2,
            self.min.y + (self.max.y - self.min.y) / 2,
            self.min.z + (self.max.z - self.min.z) / 2,
        )
    }

    /// Returns a new AABB expanded by `margin` on each side
    /// (6 faces expanded outward).
    pub fn expand_by(&self, margin: i128) -> Aabb128 {
        Aabb128 {
            min: WorldPosition::new(
                self.min.x - margin,
                self.min.y - margin,
                self.min.z - margin,
            ),
            max: WorldPosition::new(
                self.max.x + margin,
                self.max.y + margin,
                self.max.z + margin,
            ),
        }
    }

    /// Returns the size along each axis as a Vec3I128.
    pub fn size(&self) -> Vec3I128 {
        self.max - self.min
    }

    /// Returns true if the AABB has zero volume
    /// (degenerate on at least one axis).
    pub fn is_degenerate(&self) -> bool {
        self.min.x == self.max.x
            || self.min.y == self.max.y
            || self.min.z == self.max.z
    }
}
```

### Design notes

- The `contains_point` check uses inclusive bounds (`<=`) — a point on the face of the AABB is considered inside. This matches standard game engine conventions.
- `intersects` also treats touching (shared face/edge) as intersection. This prevents cracks in spatial subdivision.
- `volume()` returns i128 and can overflow for AABBs spanning more than ~5.5×10¹² mm (~5.5 billion km) per axis. A `volume_f64()` method should be provided as a fallback for planet/solar-system scale boxes.
- The AABB is intentionally not generic over the position type. A separate `AabbLocal` using `LocalPosition` and `f32` can be added later for render-space culling.

## Outcome

After this story is complete:

- `Aabb128` provides exact bounding-box math in world space
- Chunk bounds, entity bounds, and planet bounds can all use the same type
- Broad-phase collision and spatial queries have a reliable primitive
- Union and expand operations support spatial index construction

## Demo Integration

**Demo crate:** `nebula-demo`

The title shows `Inside planet AABB: true/false`, which flips when the moving position crosses the bounding box boundary.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| *(none)* | — | Uses only `WorldPosition` and `Vec3I128` from `nebula_math` |

Rust edition 2024. No external crates needed.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contains_point_inside() {
        let aabb = Aabb128::new(
            WorldPosition::new(0, 0, 0),
            WorldPosition::new(10, 10, 10),
        );
        assert!(aabb.contains_point(WorldPosition::new(5, 5, 5)));
    }

    #[test]
    fn test_contains_point_outside() {
        let aabb = Aabb128::new(
            WorldPosition::new(0, 0, 0),
            WorldPosition::new(10, 10, 10),
        );
        assert!(!aabb.contains_point(WorldPosition::new(11, 5, 5)));
    }

    #[test]
    fn test_contains_point_on_edge() {
        let aabb = Aabb128::new(
            WorldPosition::new(0, 0, 0),
            WorldPosition::new(10, 10, 10),
        );
        assert!(aabb.contains_point(WorldPosition::new(0, 0, 0)));   // min corner
        assert!(aabb.contains_point(WorldPosition::new(10, 10, 10))); // max corner
        assert!(aabb.contains_point(WorldPosition::new(10, 5, 5)));   // face
    }

    #[test]
    fn test_intersects_overlapping() {
        let a = Aabb128::new(
            WorldPosition::new(0, 0, 0),
            WorldPosition::new(10, 10, 10),
        );
        let b = Aabb128::new(
            WorldPosition::new(5, 5, 5),
            WorldPosition::new(15, 15, 15),
        );
        assert!(a.intersects(&b));
        assert!(b.intersects(&a)); // symmetric
    }

    #[test]
    fn test_intersects_disjoint() {
        let a = Aabb128::new(
            WorldPosition::new(0, 0, 0),
            WorldPosition::new(10, 10, 10),
        );
        let b = Aabb128::new(
            WorldPosition::new(20, 20, 20),
            WorldPosition::new(30, 30, 30),
        );
        assert!(!a.intersects(&b));
        assert!(!b.intersects(&a));
    }

    #[test]
    fn test_intersects_touching() {
        let a = Aabb128::new(
            WorldPosition::new(0, 0, 0),
            WorldPosition::new(10, 10, 10),
        );
        let b = Aabb128::new(
            WorldPosition::new(10, 0, 0),
            WorldPosition::new(20, 10, 10),
        );
        assert!(a.intersects(&b)); // shared face counts as intersection
    }

    #[test]
    fn test_union_encloses_both() {
        let a = Aabb128::new(
            WorldPosition::new(0, 0, 0),
            WorldPosition::new(5, 5, 5),
        );
        let b = Aabb128::new(
            WorldPosition::new(3, 3, 3),
            WorldPosition::new(10, 10, 10),
        );
        let u = a.union(&b);
        assert_eq!(u.min, WorldPosition::new(0, 0, 0));
        assert_eq!(u.max, WorldPosition::new(10, 10, 10));
        // Union must contain all points from both boxes
        assert!(u.contains_point(WorldPosition::new(0, 0, 0)));
        assert!(u.contains_point(WorldPosition::new(10, 10, 10)));
        assert!(u.contains_point(WorldPosition::new(5, 5, 5)));
    }

    #[test]
    fn test_volume() {
        let aabb = Aabb128::new(
            WorldPosition::new(0, 0, 0),
            WorldPosition::new(10, 20, 30),
        );
        assert_eq!(aabb.volume(), 6000); // 10 * 20 * 30
    }

    #[test]
    fn test_volume_unit_cube() {
        let aabb = Aabb128::new(
            WorldPosition::new(0, 0, 0),
            WorldPosition::new(1, 1, 1),
        );
        assert_eq!(aabb.volume(), 1);
    }

    #[test]
    fn test_center() {
        let aabb = Aabb128::new(
            WorldPosition::new(0, 0, 0),
            WorldPosition::new(10, 10, 10),
        );
        assert_eq!(aabb.center(), WorldPosition::new(5, 5, 5));
    }

    #[test]
    fn test_expand_by() {
        let aabb = Aabb128::new(
            WorldPosition::new(5, 5, 5),
            WorldPosition::new(15, 15, 15),
        );
        let expanded = aabb.expand_by(2);
        assert_eq!(expanded.min, WorldPosition::new(3, 3, 3));
        assert_eq!(expanded.max, WorldPosition::new(17, 17, 17));
    }

    #[test]
    fn test_expand_grows_volume() {
        let aabb = Aabb128::new(
            WorldPosition::new(0, 0, 0),
            WorldPosition::new(10, 10, 10),
        );
        let expanded = aabb.expand_by(1);
        assert!(expanded.volume() > aabb.volume());
        // Original: 10³ = 1000. Expanded: 12³ = 1728.
        assert_eq!(expanded.volume(), 1728);
    }

    #[test]
    fn test_constructor_auto_sorts() {
        let aabb = Aabb128::new(
            WorldPosition::new(10, 10, 10),
            WorldPosition::new(0, 0, 0),
        );
        assert_eq!(aabb.min, WorldPosition::new(0, 0, 0));
        assert_eq!(aabb.max, WorldPosition::new(10, 10, 10));
    }

    #[test]
    fn test_size() {
        let aabb = Aabb128::new(
            WorldPosition::new(2, 3, 4),
            WorldPosition::new(12, 13, 14),
        );
        assert_eq!(aabb.size(), Vec3I128::new(10, 10, 10));
    }
}
```
