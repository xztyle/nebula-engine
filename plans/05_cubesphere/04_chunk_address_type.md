# Chunk Address Type

## Problem

The cubesphere planet is divided into chunks at multiple levels of detail (LODs). Every chunk must have a unique, lightweight identifier that can serve as a hash-map key, be serialized for networking and persistence, and encode enough information to derive the chunk's spatial extent on the cube face. Without a purpose-built address type, chunk identity becomes a fragile combination of ad-hoc fields scattered across different systems, leading to duplicate loads, missed cache hits, and LOD selection bugs. A single canonical `ChunkAddress` type is required.

## Solution

Define `ChunkAddress` in the `nebula_cubesphere` crate.

### ChunkAddress Struct

```rust
/// Uniquely identifies a chunk on the cubesphere.
///
/// - `face`: which of the 6 cube faces this chunk belongs to.
/// - `lod`: level of detail. LOD 0 is the highest detail (smallest chunks).
///   Higher LOD values represent coarser, larger chunks.
/// - `x`, `y`: grid coordinates within the face at this LOD level.
///   At LOD `l`, the face is divided into a `grid_size(l) x grid_size(l)` grid,
///   where `grid_size(l) = MAX_CHUNKS_PER_AXIS >> l`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChunkAddress {
    pub face: CubeFace,
    pub lod: u8,
    pub x: u32,
    pub y: u32,
}
```

### Constants and Grid Sizing

```rust
impl ChunkAddress {
    /// Maximum number of chunks along one axis at LOD 0 (highest detail).
    /// This gives 2^20 = 1,048,576 chunks per face axis at max detail,
    /// which at 32m per chunk covers a planet with ~33,554 km circumference
    /// (Earth-like).
    pub const MAX_LOD0_AXIS: u32 = 1 << 20;

    /// Maximum valid LOD level. LOD 20 means the entire face is one chunk.
    pub const MAX_LOD: u8 = 20;

    /// Number of chunks along one axis at the given LOD level.
    pub fn grid_size(lod: u8) -> u32 {
        assert!(lod <= Self::MAX_LOD, "LOD {lod} exceeds MAX_LOD {}", Self::MAX_LOD);
        Self::MAX_LOD0_AXIS >> lod
    }

    /// Construct a ChunkAddress, validating that x and y are within
    /// the grid bounds for the given LOD.
    pub fn new(face: CubeFace, lod: u8, x: u32, y: u32) -> Self {
        let size = Self::grid_size(lod);
        assert!(x < size, "x={x} out of range for LOD {lod} (max {size})");
        assert!(y < size, "y={y} out of range for LOD {lod} (max {size})");
        Self { face, lod, x, y }
    }
}
```

### UV Bounds Derivation

Each `ChunkAddress` corresponds to a rectangular region on the cube face in UV space:

```rust
impl ChunkAddress {
    /// Compute the UV bounding box of this chunk on its face.
    /// Returns `(u_min, v_min, u_max, v_max)` where all values are in [0, 1].
    pub fn uv_bounds(&self) -> (f64, f64, f64, f64) {
        let size = Self::grid_size(self.lod) as f64;
        let u_min = self.x as f64 / size;
        let v_min = self.y as f64 / size;
        let u_max = (self.x + 1) as f64 / size;
        let v_max = (self.y + 1) as f64 / size;
        (u_min, v_min, u_max, v_max)
    }

    /// Return the FaceCoord at the center of this chunk.
    pub fn center_face_coord(&self) -> FaceCoord {
        let (u_min, v_min, u_max, v_max) = self.uv_bounds();
        FaceCoord::new(self.face, (u_min + u_max) * 0.5, (v_min + v_max) * 0.5)
    }
}
```

### Parent and Child Addresses

```rust
impl ChunkAddress {
    /// The parent chunk at the next coarser LOD level.
    /// Returns `None` if already at MAX_LOD.
    pub fn parent(&self) -> Option<ChunkAddress> {
        if self.lod >= Self::MAX_LOD {
            return None;
        }
        Some(ChunkAddress {
            face: self.face,
            lod: self.lod + 1,
            x: self.x / 2,
            y: self.y / 2,
        })
    }

    /// The four child chunks at the next finer LOD level.
    /// Returns `None` if already at LOD 0.
    pub fn children(&self) -> Option<[ChunkAddress; 4]> {
        if self.lod == 0 {
            return None;
        }
        let child_lod = self.lod - 1;
        let cx = self.x * 2;
        let cy = self.y * 2;
        Some([
            ChunkAddress::new(self.face, child_lod, cx, cy),
            ChunkAddress::new(self.face, child_lod, cx + 1, cy),
            ChunkAddress::new(self.face, child_lod, cx, cy + 1),
            ChunkAddress::new(self.face, child_lod, cx + 1, cy + 1),
        ])
    }
}
```

### Design Constraints

- The struct is 8 bytes total (1 byte face as `u8`, 1 byte `lod`, 2 bytes padding, 4 bytes `x`, 4 bytes `y` — actually 10 bytes, but `#[repr(C)]` is not required; the compiler may pack it).
- `Hash`, `Eq`, and `Ord` are derived, making `ChunkAddress` directly usable as a key in `HashMap`, `BTreeMap`, and `HashSet`.
- The `lod` field uses `u8`, supporting up to 255 LOD levels (far more than the 20 actually used).
- No allocations in any method.

## Outcome

The `nebula_cubesphere` crate exports `ChunkAddress` with construction, validation, UV bounds derivation, parent/child traversal, and all standard key traits. Every system that needs to identify or look up a chunk — LOD manager, chunk loader, mesh cache, neighbor finder — uses this single type. Running `cargo test -p nebula_cubesphere` passes all address tests.

## Demo Integration

**Demo crate:** `nebula-demo`

The wireframe sphere now shows chunk boundaries as grid lines on each face. Each chunk displays its address `(face, lod, u, v)` as tiny text labels when the camera is close.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| *(none)* | — | Pure `std` only; no external dependencies for this type |

This type lives in the `nebula_cubesphere` crate with Rust edition 2024. All trait implementations are derived or use `std` only.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    #[test]
    fn test_address_equality() {
        let a = ChunkAddress::new(CubeFace::PosX, 5, 10, 20);
        let b = ChunkAddress::new(CubeFace::PosX, 5, 10, 20);
        let c = ChunkAddress::new(CubeFace::PosX, 5, 10, 21);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn test_hashing_consistency() {
        let mut set = HashSet::new();
        let addr = ChunkAddress::new(CubeFace::NegY, 3, 100, 200);
        set.insert(addr);
        set.insert(addr); // duplicate
        assert_eq!(set.len(), 1);

        let mut map = HashMap::new();
        map.insert(addr, "chunk_data");
        assert_eq!(map.get(&addr), Some(&"chunk_data"));
    }

    #[test]
    fn test_lod0_has_maximum_range() {
        let size = ChunkAddress::grid_size(0);
        assert_eq!(size, ChunkAddress::MAX_LOD0_AXIS);

        // Valid at LOD 0
        let _ = ChunkAddress::new(CubeFace::PosZ, 0, size - 1, size - 1);
    }

    #[test]
    fn test_higher_lod_has_smaller_range() {
        let size_0 = ChunkAddress::grid_size(0);
        let size_1 = ChunkAddress::grid_size(1);
        let size_5 = ChunkAddress::grid_size(5);
        assert_eq!(size_1, size_0 / 2);
        assert_eq!(size_5, size_0 / 32);
    }

    #[test]
    fn test_uv_bounds_computation() {
        // At LOD 0 with grid_size = MAX, a chunk at (0,0) covers [0, 1/grid_size]^2
        let addr = ChunkAddress::new(CubeFace::PosX, 0, 0, 0);
        let (u_min, v_min, u_max, v_max) = addr.uv_bounds();
        assert_eq!(u_min, 0.0);
        assert_eq!(v_min, 0.0);
        assert!(u_max > 0.0);
        assert!(v_max > 0.0);

        // At MAX_LOD, the single chunk covers [0, 1]
        let addr_max = ChunkAddress::new(CubeFace::NegZ, ChunkAddress::MAX_LOD, 0, 0);
        let (u_min, v_min, u_max, v_max) = addr_max.uv_bounds();
        assert!((u_min - 0.0).abs() < 1e-12);
        assert!((v_min - 0.0).abs() < 1e-12);
        assert!((u_max - 1.0).abs() < 1e-12);
        assert!((v_max - 1.0).abs() < 1e-12);
    }

    #[test]
    fn test_parent_address_at_lod_plus_1() {
        let child = ChunkAddress::new(CubeFace::PosY, 2, 6, 8);
        let parent = child.parent().expect("should have parent");
        assert_eq!(parent.face, CubeFace::PosY);
        assert_eq!(parent.lod, 3);
        assert_eq!(parent.x, 3);
        assert_eq!(parent.y, 4);
    }

    #[test]
    fn test_max_lod_has_no_parent() {
        let addr = ChunkAddress::new(CubeFace::PosX, ChunkAddress::MAX_LOD, 0, 0);
        assert!(addr.parent().is_none());
    }

    #[test]
    fn test_children_cover_parent_area() {
        let parent = ChunkAddress::new(CubeFace::NegX, 5, 10, 20);
        let (pu_min, pv_min, pu_max, pv_max) = parent.uv_bounds();

        let children = parent.children().expect("should have children");
        assert_eq!(children.len(), 4);

        // Union of children's UV bounds should equal parent's UV bounds
        let mut cu_min = f64::MAX;
        let mut cv_min = f64::MAX;
        let mut cu_max = f64::MIN;
        let mut cv_max = f64::MIN;
        for child in &children {
            let (u0, v0, u1, v1) = child.uv_bounds();
            cu_min = cu_min.min(u0);
            cv_min = cv_min.min(v0);
            cu_max = cu_max.max(u1);
            cv_max = cv_max.max(v1);
        }
        assert!((cu_min - pu_min).abs() < 1e-12);
        assert!((cv_min - pv_min).abs() < 1e-12);
        assert!((cu_max - pu_max).abs() < 1e-12);
        assert!((cv_max - pv_max).abs() < 1e-12);
    }

    #[test]
    fn test_lod0_has_no_children() {
        let addr = ChunkAddress::new(CubeFace::PosZ, 0, 100, 200);
        assert!(addr.children().is_none());
    }

    #[test]
    fn test_ordering() {
        let a = ChunkAddress::new(CubeFace::PosX, 5, 10, 20);
        let b = ChunkAddress::new(CubeFace::NegX, 5, 10, 20);
        // PosX (0) < NegX (1)
        assert!(a < b);
    }

    #[test]
    #[should_panic(expected = "out of range")]
    fn test_invalid_coordinates_panic() {
        let size = ChunkAddress::grid_size(5);
        ChunkAddress::new(CubeFace::PosX, 5, size, 0); // x == size is out of range
    }
}
```
