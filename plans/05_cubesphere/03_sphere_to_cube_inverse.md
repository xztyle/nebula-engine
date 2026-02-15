# Sphere-to-Cube Inverse

## Problem

Many operations require the reverse of the cube-to-sphere projection: given a direction or point on the unit sphere, determine which cube face it belongs to and compute the (u, v) coordinates within that face. This inverse is needed for placing objects on a planet surface from geographic coordinates, determining which chunk a ray-cast hit belongs to, converting noise samples from spherical coordinates to face-local coordinates, and importing heightmap data from equirectangular projections. The inverse must be exact enough that a roundtrip (cube-to-sphere-to-cube) returns the original coordinates to within floating-point epsilon. Edge and corner cases — where the sphere point lies exactly between two or three faces — must be handled deterministically.

## Solution

Implement the inverse projection in the `nebula_cubesphere` crate.

### Face Detection

Given a direction vector `dir: DVec3` (not necessarily unit length), the dominant axis determines the cube face:

```rust
use glam::DVec3;

/// Determine which cube face a direction vector belongs to.
///
/// The face is determined by the axis with the largest absolute component.
/// Ties are broken by a fixed priority: X > Y > Z, positive > negative.
pub fn direction_to_face(dir: DVec3) -> CubeFace {
    let ax = dir.x.abs();
    let ay = dir.y.abs();
    let az = dir.z.abs();

    if ax >= ay && ax >= az {
        if dir.x >= 0.0 { CubeFace::PosX } else { CubeFace::NegX }
    } else if ay >= az {
        if dir.y >= 0.0 { CubeFace::PosY } else { CubeFace::NegY }
    } else {
        if dir.z >= 0.0 { CubeFace::PosZ } else { CubeFace::NegZ }
    }
}
```

The tie-breaking rule (X > Y > Z, positive > negative) ensures that edge and corner points are always assigned to exactly one face. This must be consistent everywhere in the engine.

### UV Computation

Once the face is known, project the direction onto that face to recover (s, t) in [-1, 1], then remap to (u, v) in [0, 1]:

```rust
/// Convert a direction vector to a FaceCoord by inverse projection.
///
/// This is the inverse of `face_coord_to_cube_point` followed by normalization.
/// The direction does not need to be unit length.
pub fn direction_to_face_coord(dir: DVec3) -> FaceCoord {
    let face = direction_to_face(dir);
    let n = face.normal();
    let tang = face.tangent();
    let bitan = face.bitangent();

    // Project onto the cube face plane: divide by the dot with the normal
    // to get the point where the ray from origin in direction `dir` hits
    // the face at distance 1 along the normal.
    let d = dir.dot(n);
    let projected = dir / d; // Point on the cube face (normal component = 1)

    // Extract the tangent and bitangent components
    let s = projected.dot(tang); // in [-1, 1]
    let t = projected.dot(bitan); // in [-1, 1]

    // Remap from [-1, 1] to [0, 1]
    let u = (s + 1.0) * 0.5;
    let v = (t + 1.0) * 0.5;

    FaceCoord::new(face, u, v)
}
```

### Inverse of Everitt Mapping

The Everitt cube-to-sphere mapping (story 02) does not have a simple analytic inverse. The inverse is computed by:

1. Using `direction_to_face` to find the face.
2. Projecting the sphere point onto the cube face to get approximate (s, t).
3. Applying Newton-Raphson refinement to solve for the exact (s, t) that would produce the given sphere point through the forward Everitt mapping.

```rust
/// Inverse of the Everitt cube-to-sphere mapping.
///
/// Given a unit sphere point, returns the FaceCoord that would produce it
/// via `face_coord_to_sphere_everitt`.
///
/// Uses Newton-Raphson iteration for sub-epsilon accuracy.
pub fn sphere_to_face_coord_everitt(sphere_point: DVec3) -> FaceCoord {
    // Step 1: Get face and initial approximation via simple projection
    let approx = direction_to_face_coord(sphere_point);
    let face = approx.face;

    // Step 2: Newton-Raphson refinement (typically converges in 3-5 iterations)
    let mut u = approx.u;
    let mut v = approx.v;
    let target = sphere_point.normalize();

    for _ in 0..10 {
        let fc = FaceCoord::new_unchecked(face, u, v);
        let current = face_coord_to_sphere_everitt(&fc);
        let error = target - current;

        if error.length() < 1e-14 {
            break;
        }

        // Compute Jacobian numerically (partial derivatives of sphere point w.r.t. u, v)
        let du = 1e-8;
        let dv = 1e-8;
        let fc_du = FaceCoord::new_unchecked(face, (u + du).min(1.0), v);
        let fc_dv = FaceCoord::new_unchecked(face, u, (v + dv).min(1.0));
        let dp_du = (face_coord_to_sphere_everitt(&fc_du) - current) / du;
        let dp_dv = (face_coord_to_sphere_everitt(&fc_dv) - current) / dv;

        // Solve 2x2 least-squares: [dp_du dp_dv]^T [dp_du dp_dv] [delta_u delta_v]^T = [dp_du dp_dv]^T error
        let a11 = dp_du.dot(dp_du);
        let a12 = dp_du.dot(dp_dv);
        let a22 = dp_dv.dot(dp_dv);
        let b1 = dp_du.dot(error);
        let b2 = dp_dv.dot(error);

        let det = a11 * a22 - a12 * a12;
        if det.abs() < 1e-20 {
            break;
        }

        let delta_u = (a22 * b1 - a12 * b2) / det;
        let delta_v = (a11 * b2 - a12 * b1) / det;

        u = (u + delta_u).clamp(0.0, 1.0);
        v = (v + delta_v).clamp(0.0, 1.0);
    }

    FaceCoord::new(face, u, v)
}
```

### Design Constraints

- `direction_to_face` must handle zero vectors gracefully (return a default face, e.g., `PosX`).
- Edge tie-breaking is deterministic and consistent with the forward projection.
- Newton-Raphson is only needed for the Everitt inverse; the simple projection inverse (`direction_to_face_coord`) is exact for the naive cube-to-sphere mapping and is the fast path.

## Outcome

The `nebula_cubesphere` crate exports `direction_to_face()`, `direction_to_face_coord()`, and `sphere_to_face_coord_everitt()`. These enable any system to convert from sphere/direction coordinates back to the face-local (u, v) system. Running `cargo test -p nebula_cubesphere` passes all inverse projection and roundtrip tests.

## Demo Integration

**Demo crate:** `nebula-demo`

The demo picks random points on the sphere surface and draws small markers colored by which face they map back to, proving the inverse projection is correct.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `glam` | 0.29 | `DVec3` for 3D vector operations |

No other external dependencies. Newton-Raphson iteration uses only `std::f64` arithmetic. The crate uses Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;

    const EPSILON: f64 = 1e-9;

    #[test]
    fn test_roundtrip_cube_sphere_cube() {
        for face in CubeFace::ALL {
            for u_steps in 0..=10 {
                for v_steps in 0..=10 {
                    let u = u_steps as f64 / 10.0;
                    let v = v_steps as f64 / 10.0;
                    let original = FaceCoord::new(face, u, v);
                    let sphere_pt = face_coord_to_sphere_everitt(&original);
                    let recovered = sphere_to_face_coord_everitt(sphere_pt);

                    assert_eq!(recovered.face, original.face,
                        "Face mismatch for {face:?} at ({u}, {v})");
                    assert!(
                        (recovered.u - original.u).abs() < EPSILON,
                        "u mismatch for {face:?}: original {u}, recovered {}",
                        recovered.u
                    );
                    assert!(
                        (recovered.v - original.v).abs() < EPSILON,
                        "v mismatch for {face:?}: original {v}, recovered {}",
                        recovered.v
                    );
                }
            }
        }
    }

    #[test]
    fn test_face_detection_axis_aligned_directions() {
        assert_eq!(direction_to_face(DVec3::X), CubeFace::PosX);
        assert_eq!(direction_to_face(DVec3::NEG_X), CubeFace::NegX);
        assert_eq!(direction_to_face(DVec3::Y), CubeFace::PosY);
        assert_eq!(direction_to_face(DVec3::NEG_Y), CubeFace::NegY);
        assert_eq!(direction_to_face(DVec3::Z), CubeFace::PosZ);
        assert_eq!(direction_to_face(DVec3::NEG_Z), CubeFace::NegZ);
    }

    #[test]
    fn test_edge_points_consistently_assigned() {
        // Point equidistant between PosX and PosY faces
        let edge_point = DVec3::new(1.0, 1.0, 0.0).normalize();
        let face = direction_to_face(edge_point);
        // Tie-breaking: X wins over Y
        assert_eq!(face, CubeFace::PosX);

        // Call again to ensure determinism
        let face2 = direction_to_face(edge_point);
        assert_eq!(face, face2);
    }

    #[test]
    fn test_corner_points_consistently_assigned() {
        // Point equidistant from PosX, PosY, PosZ (cube corner)
        let corner = DVec3::new(1.0, 1.0, 1.0).normalize();
        let face = direction_to_face(corner);
        // Tie-breaking: X > Y > Z
        assert_eq!(face, CubeFace::PosX);
    }

    #[test]
    fn test_inverse_handles_southern_hemisphere() {
        // Points in the -Y hemisphere
        let dirs = [
            DVec3::new(0.0, -1.0, 0.0),
            DVec3::new(0.3, -0.9, 0.2).normalize(),
            DVec3::new(-0.5, -0.7, 0.5).normalize(),
        ];
        for dir in dirs {
            let fc = direction_to_face_coord(dir);
            assert!(fc.u >= 0.0 && fc.u <= 1.0, "u out of range for {dir:?}");
            assert!(fc.v >= 0.0 && fc.v <= 1.0, "v out of range for {dir:?}");

            // Roundtrip
            let cube_pt = face_coord_to_cube_point(&fc);
            let back = cube_pt.normalize();
            let dir_norm = dir.normalize();
            assert!(
                (back - dir_norm).length() < 0.01,
                "Southern hemisphere roundtrip failed for {dir:?}"
            );
        }
    }

    #[test]
    fn test_simple_projection_roundtrip() {
        // For the non-Everitt (simple normalization) path, the roundtrip
        // through direction_to_face_coord should be exact.
        for face in CubeFace::ALL {
            let fc = FaceCoord::new(face, 0.3, 0.7);
            let cube_pt = face_coord_to_cube_point(&fc);
            let sphere_pt = cube_pt.normalize();
            let recovered = direction_to_face_coord(sphere_pt);
            assert_eq!(recovered.face, face);
            assert!((recovered.u - 0.3).abs() < EPSILON);
            assert!((recovered.v - 0.7).abs() < EPSILON);
        }
    }

    #[test]
    fn test_direction_to_face_all_negative() {
        let dir = DVec3::new(-0.1, -0.5, -0.9);
        let face = direction_to_face(dir);
        assert_eq!(face, CubeFace::NegZ);
    }
}
```
