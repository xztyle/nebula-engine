# Cube Face Enum & Local Coords

## Problem

A cubesphere planet is built from six faces of a cube projected onto a sphere. Every system that touches the cubesphere — terrain generation, chunk addressing, neighbor finding, meshing, UV mapping — needs a shared, canonical enumeration of these six faces and a consistent 2D local coordinate system on each face. Without a formally defined `CubeFace` enum and `FaceCoord` type, each subsystem will invent its own face numbering, its own UV orientation, and its own normal/tangent conventions, leading to subtle winding-order bugs, seams at face boundaries, and impossible-to-diagnose rendering artifacts. The foundation must be laid here: one enum, one 2D coordinate type, one set of basis vectors per face, used everywhere.

## Solution

Define the types in the `nebula_cubesphere` crate.

### CubeFace Enum

```rust
/// The six faces of the cube that forms the cubesphere.
///
/// Each variant corresponds to a face whose outward normal points
/// along the named axis direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum CubeFace {
    PosX = 0,
    NegX = 1,
    PosY = 2,
    NegY = 3,
    PosZ = 4,
    NegZ = 5,
}
```

The `repr(u8)` ensures compact storage and deterministic ordering. Derive `Hash`, `Eq`, and `Ord` so faces can be used as map keys and sorted.

Provide an iterator over all faces:

```rust
impl CubeFace {
    /// All six faces in canonical order.
    pub const ALL: [CubeFace; 6] = [
        CubeFace::PosX, CubeFace::NegX,
        CubeFace::PosY, CubeFace::NegY,
        CubeFace::PosZ, CubeFace::NegZ,
    ];

    /// The opposite face (e.g., PosX -> NegX).
    pub fn opposite(self) -> CubeFace {
        match self {
            CubeFace::PosX => CubeFace::NegX,
            CubeFace::NegX => CubeFace::PosX,
            CubeFace::PosY => CubeFace::NegY,
            CubeFace::NegY => CubeFace::PosY,
            CubeFace::PosZ => CubeFace::NegZ,
            CubeFace::NegZ => CubeFace::PosZ,
        }
    }
}
```

### FaceCoord

```rust
/// A 2D coordinate on a cube face. `u` and `v` are in the range [0, 1].
///
/// (u=0, v=0) is the "bottom-left" corner of the face when viewed from
/// outside the cube looking inward. (u=1, v=1) is the "top-right" corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FaceCoord {
    pub face: CubeFace,
    pub u: f64,
    pub v: f64,
}

impl FaceCoord {
    /// Construct a FaceCoord, clamping u and v to [0, 1].
    pub fn new(face: CubeFace, u: f64, v: f64) -> Self {
        Self {
            face,
            u: u.clamp(0.0, 1.0),
            v: v.clamp(0.0, 1.0),
        }
    }

    /// Construct without clamping. Caller guarantees 0 <= u, v <= 1.
    pub fn new_unchecked(face: CubeFace, u: f64, v: f64) -> Self {
        debug_assert!((0.0..=1.0).contains(&u), "u out of range: {u}");
        debug_assert!((0.0..=1.0).contains(&v), "v out of range: {v}");
        Self { face, u, v }
    }
}
```

### Per-Face Basis Vectors

Each face has a normal vector (pointing outward from the cube), a tangent vector (the direction of increasing `u`), and a bitangent vector (the direction of increasing `v`). Together they form a right-handed orthonormal basis: `tangent x bitangent = normal`.

```rust
use glam::DVec3;

impl CubeFace {
    /// Outward-pointing unit normal for this face.
    pub fn normal(self) -> DVec3 {
        match self {
            CubeFace::PosX => DVec3::X,
            CubeFace::NegX => DVec3::NEG_X,
            CubeFace::PosY => DVec3::Y,
            CubeFace::NegY => DVec3::NEG_Y,
            CubeFace::PosZ => DVec3::Z,
            CubeFace::NegZ => DVec3::NEG_Z,
        }
    }

    /// Tangent vector: direction of increasing `u` on this face.
    pub fn tangent(self) -> DVec3 {
        match self {
            CubeFace::PosX => DVec3::NEG_Z,
            CubeFace::NegX => DVec3::Z,
            CubeFace::PosY => DVec3::X,
            CubeFace::NegY => DVec3::X,
            CubeFace::PosZ => DVec3::X,
            CubeFace::NegZ => DVec3::NEG_X,
        }
    }

    /// Bitangent vector: direction of increasing `v` on this face.
    pub fn bitangent(self) -> DVec3 {
        match self {
            CubeFace::PosX => DVec3::Y,
            CubeFace::NegX => DVec3::Y,
            CubeFace::PosY => DVec3::NEG_Z,
            CubeFace::NegY => DVec3::Z,
            CubeFace::PosZ => DVec3::Y,
            CubeFace::NegZ => DVec3::Y,
        }
    }
}
```

The assignment is chosen so that the tangent and bitangent always lie on the face plane (perpendicular to the normal), and `tangent.cross(bitangent) == normal` for every face. This guarantees consistent winding order when generating mesh triangles from UV coordinates.

### Design Constraints

- `FaceCoord` uses `f64` for `u` and `v` to maintain precision during cube-to-sphere projection and terrain generation. The final mesh vertices will be converted to `f32` at the rendering boundary.
- The UV convention is fixed globally. No face has a flipped or rotated coordinate system relative to the others in a way that isn't captured by the tangent/bitangent vectors.
- `CubeFace::ALL` is a `const` array, enabling compile-time iteration in procedural macros or const contexts.

## Outcome

The `nebula_cubesphere` crate exports `CubeFace` (with 6 variants, `ALL` constant, `opposite()`, `normal()`, `tangent()`, `bitangent()`) and `FaceCoord` (with `new()` and `new_unchecked()`). Every downstream system — projection, chunk addressing, neighbor finding, meshing — imports these types as its canonical face and coordinate representation. Running `cargo test -p nebula_cubesphere` passes all face and coordinate tests.

## Demo Integration

**Demo crate:** `nebula-demo`

The demo renders six colored quads arranged as a cube's faces, each a distinct color (red, green, blue, cyan, magenta, yellow). The cube floats in space.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `glam` | 0.29 | `DVec3` for normal, tangent, and bitangent vectors |

No other external dependencies. The enum and struct are pure Rust with standard derives. The crate uses Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;

    #[test]
    fn test_all_six_face_variants_exist() {
        assert_eq!(CubeFace::ALL.len(), 6);
        let faces: Vec<CubeFace> = CubeFace::ALL.to_vec();
        assert!(faces.contains(&CubeFace::PosX));
        assert!(faces.contains(&CubeFace::NegX));
        assert!(faces.contains(&CubeFace::PosY));
        assert!(faces.contains(&CubeFace::NegY));
        assert!(faces.contains(&CubeFace::PosZ));
        assert!(faces.contains(&CubeFace::NegZ));
    }

    #[test]
    fn test_normals_are_unit_length() {
        for face in CubeFace::ALL {
            let n = face.normal();
            assert!(
                (n.length() - 1.0).abs() < 1e-12,
                "Normal for {face:?} is not unit length: {}",
                n.length()
            );
        }
    }

    #[test]
    fn test_opposite_face_normals_are_antiparallel() {
        for face in CubeFace::ALL {
            let n = face.normal();
            let opp_n = face.opposite().normal();
            assert!(
                (n + opp_n).length() < 1e-12,
                "Normals for {face:?} and {:?} are not antiparallel",
                face.opposite()
            );
        }
    }

    #[test]
    fn test_tangent_cross_bitangent_equals_normal() {
        for face in CubeFace::ALL {
            let t = face.tangent();
            let b = face.bitangent();
            let n = face.normal();
            let cross = t.cross(b);
            assert!(
                (cross - n).length() < 1e-12,
                "tangent x bitangent != normal for {face:?}: got {cross:?}, expected {n:?}"
            );
        }
    }

    #[test]
    fn test_tangent_and_bitangent_are_unit_length() {
        for face in CubeFace::ALL {
            let t = face.tangent();
            let b = face.bitangent();
            assert!((t.length() - 1.0).abs() < 1e-12, "Tangent not unit for {face:?}");
            assert!((b.length() - 1.0).abs() < 1e-12, "Bitangent not unit for {face:?}");
        }
    }

    #[test]
    fn test_tangent_perpendicular_to_normal() {
        for face in CubeFace::ALL {
            let t = face.tangent();
            let n = face.normal();
            assert!(
                t.dot(n).abs() < 1e-12,
                "Tangent not perpendicular to normal for {face:?}"
            );
        }
    }

    #[test]
    fn test_bitangent_perpendicular_to_normal() {
        for face in CubeFace::ALL {
            let b = face.bitangent();
            let n = face.normal();
            assert!(
                b.dot(n).abs() < 1e-12,
                "Bitangent not perpendicular to normal for {face:?}"
            );
        }
    }

    #[test]
    fn test_face_coord_clamping() {
        let fc = FaceCoord::new(CubeFace::PosX, -0.5, 1.5);
        assert_eq!(fc.u, 0.0);
        assert_eq!(fc.v, 1.0);
    }

    #[test]
    fn test_face_coord_valid_range() {
        let fc = FaceCoord::new(CubeFace::NegZ, 0.25, 0.75);
        assert_eq!(fc.u, 0.25);
        assert_eq!(fc.v, 0.75);
        assert_eq!(fc.face, CubeFace::NegZ);
    }

    #[test]
    fn test_face_coord_corners() {
        for face in CubeFace::ALL {
            for &u in &[0.0, 1.0] {
                for &v in &[0.0, 1.0] {
                    let fc = FaceCoord::new(face, u, v);
                    assert_eq!(fc.u, u);
                    assert_eq!(fc.v, v);
                }
            }
        }
    }

    #[test]
    fn test_opposite_is_involution() {
        for face in CubeFace::ALL {
            assert_eq!(face.opposite().opposite(), face);
        }
    }
}
```
