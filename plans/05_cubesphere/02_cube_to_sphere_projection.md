# Cube-to-Sphere Projection

## Problem

A cubesphere is a cube whose surface is projected onto a sphere. The naive normalization approach (divide the cube surface point by its length) produces severe area distortion: cells near cube corners are compressed to roughly 67% of the area of cells near face centers. For a voxel engine that needs roughly uniform cell sizes for consistent LOD, collision, and terrain generation, this distortion is unacceptable. The engine needs an analytic projection that maps cube face coordinates to unit sphere points with minimal area distortion, and it needs the inverse mapping as well. Both must be fast enough to call millions of times per frame during chunk generation and mesh construction.

## Solution

Implement the tangent-based equal-area-ish cube-to-sphere projection in the `nebula_cubesphere` crate. The approach used is the "adjusted cube-to-sphere" mapping, which remaps the [-1, 1] cube surface coordinates through an `atan`-based warp before normalizing, distributing area more evenly than raw normalization.

### Cube Surface Point from FaceCoord

First, convert a `FaceCoord` (face, u, v) where u and v are in [0, 1] to a 3D point on the surface of a unit cube (coordinates in [-1, 1]):

```rust
use glam::DVec3;

/// Convert a FaceCoord to a point on the surface of the [-1, 1] cube.
pub fn face_coord_to_cube_point(fc: &FaceCoord) -> DVec3 {
    // Remap u, v from [0, 1] to [-1, 1]
    let s = 2.0 * fc.u - 1.0;
    let t = 2.0 * fc.v - 1.0;

    let face = fc.face;
    let n = face.normal();
    let tang = face.tangent();
    let bitan = face.bitangent();

    // The cube surface point is: normal + s * tangent + t * bitangent
    n + s * tang + t * bitan
}
```

### Projection to Sphere

The standard normalization approach divides by length. The improved projection warps `s` and `t` before normalization to reduce area distortion:

```rust
use std::f64::consts::FRAC_PI_4;

/// Project a FaceCoord onto the unit sphere using the tangent-warp method.
///
/// This produces more uniform cell areas than naive normalization.
/// Returns a unit-length DVec3 on the sphere surface.
pub fn face_coord_to_sphere(fc: &FaceCoord) -> DVec3 {
    // Remap u, v from [0, 1] to [-1, 1]
    let s = 2.0 * fc.u - 1.0;
    let t = 2.0 * fc.v - 1.0;

    // Apply tangent warp: remap through tan(x * pi/4) / tan(pi/4)
    // Since tan(pi/4) = 1, this simplifies to tan(x * pi/4).
    let ws = (s * FRAC_PI_4).tan();
    let wt = (t * FRAC_PI_4).tan();

    let face = fc.face;
    let n = face.normal();
    let tang = face.tangent();
    let bitan = face.bitangent();

    // Construct the warped cube point and normalize to the sphere
    let cube_point = n + ws * tang + wt * bitan;
    cube_point.normalize()
}
```

An alternative implementation uses the analytic Everitt mapping (the "Mathworld" formula), which provides the closest approximation to equal-area:

```rust
/// Analytic cube-to-sphere using the Everitt/Mathworld mapping.
///
/// Given a point (x, y, z) on the cube surface where one coordinate
/// is +/-1, compute the sphere point as:
///   sx = x * sqrt(1 - y^2/2 - z^2/2 + y^2*z^2/3)
///   sy = y * sqrt(1 - x^2/2 - z^2/2 + x^2*z^2/3)
///   sz = z * sqrt(1 - x^2/2 - y^2/2 + x^2*y^2/3)
pub fn cube_to_sphere_everitt(cube_point: DVec3) -> DVec3 {
    let x2 = cube_point.x * cube_point.x;
    let y2 = cube_point.y * cube_point.y;
    let z2 = cube_point.z * cube_point.z;

    DVec3::new(
        cube_point.x * (1.0 - y2 / 2.0 - z2 / 2.0 + y2 * z2 / 3.0).sqrt(),
        cube_point.y * (1.0 - x2 / 2.0 - z2 / 2.0 + x2 * z2 / 3.0).sqrt(),
        cube_point.z * (1.0 - x2 / 2.0 - y2 / 2.0 + x2 * y2 / 3.0).sqrt(),
    )
}

/// Convenience: FaceCoord -> unit sphere using Everitt mapping.
pub fn face_coord_to_sphere_everitt(fc: &FaceCoord) -> DVec3 {
    let cube_point = face_coord_to_cube_point(fc);
    cube_to_sphere_everitt(cube_point)
}
```

The engine will default to the Everitt mapping for terrain generation (where area uniformity matters most) and may use the tangent-warp method as a faster alternative for real-time LOD calculations.

### Inverse: Sphere to Cube Point

The inverse of the Everitt mapping does not have a clean closed-form solution. The engine uses the simpler inverse: find the dominant axis, project onto that cube face, and then un-warp if the forward mapping used warping. The full inverse is implemented in story 03 (`sphere_to_cube_inverse`).

### Design Constraints

- All projection functions operate in `f64` to maintain precision during terrain generation. Conversion to `f32` happens only at the GPU submission boundary.
- The functions are pure (no side effects, no state) and `#[inline]` for use in tight mesh-generation loops.
- Both the tangent-warp and Everitt methods are provided behind a `ProjectionMethod` enum so subsystems can choose.

## Outcome

The `nebula_cubesphere` crate exports `face_coord_to_sphere()`, `face_coord_to_sphere_everitt()`, `face_coord_to_cube_point()`, and `cube_to_sphere_everitt()`. Any system that needs to map a 2D face position to a 3D sphere point can call these functions. Running `cargo test -p nebula_cubesphere` passes all projection tests.

## Demo Integration

**Demo crate:** `nebula-demo`

The six flat cube faces are projected onto a sphere via the Everitt mapping. The cube inflates into a sphere. The wireframe reveals the distortion pattern.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `glam` | 0.29 | `DVec3` for 3D point/vector math |

No other external dependencies. Trigonometric functions come from `std::f64`. The crate uses Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;

    const EPSILON: f64 = 1e-10;

    #[test]
    fn test_face_center_maps_to_normal() {
        for face in CubeFace::ALL {
            let fc = FaceCoord::new(face, 0.5, 0.5);
            let sphere_pt = face_coord_to_sphere_everitt(&fc);
            let expected = face.normal();
            assert!(
                (sphere_pt - expected).length() < EPSILON,
                "Face center of {face:?} did not map to normal: got {sphere_pt:?}, expected {expected:?}"
            );
        }
    }

    #[test]
    fn test_all_outputs_on_unit_sphere() {
        for face in CubeFace::ALL {
            for u_steps in 0..=10 {
                for v_steps in 0..=10 {
                    let u = u_steps as f64 / 10.0;
                    let v = v_steps as f64 / 10.0;
                    let fc = FaceCoord::new(face, u, v);
                    let sphere_pt = face_coord_to_sphere_everitt(&fc);
                    assert!(
                        (sphere_pt.length() - 1.0).abs() < EPSILON,
                        "Point not on unit sphere for {face:?} at ({u}, {v}): length = {}",
                        sphere_pt.length()
                    );
                }
            }
        }
    }

    #[test]
    fn test_corners_are_unit_length() {
        let corners = [(0.0, 0.0), (0.0, 1.0), (1.0, 0.0), (1.0, 1.0)];
        for face in CubeFace::ALL {
            for &(u, v) in &corners {
                let fc = FaceCoord::new(face, u, v);
                let sphere_pt = face_coord_to_sphere_everitt(&fc);
                assert!(
                    (sphere_pt.length() - 1.0).abs() < EPSILON,
                    "Corner ({u}, {v}) of {face:?} not unit length: {}",
                    sphere_pt.length()
                );
            }
        }
    }

    #[test]
    fn test_projection_continuous_across_edges() {
        // Two adjacent faces should produce nearby sphere points at their shared edge.
        // PosX face at u=0 (which is the NegZ direction) and PosZ face at u=1 share an edge.
        let steps = 20;
        for i in 0..=steps {
            let v = i as f64 / steps as f64;
            let fc_a = FaceCoord::new(CubeFace::PosX, 0.0, v);
            let fc_b = FaceCoord::new(CubeFace::PosZ, 1.0, v);
            let pt_a = face_coord_to_sphere_everitt(&fc_a);
            let pt_b = face_coord_to_sphere_everitt(&fc_b);
            // These should be very close (ideally identical) at the shared edge.
            // Allowing a small tolerance because the UV conventions on adjacent
            // faces may not align perfectly until cross-face stitching is done.
            assert!(
                (pt_a - pt_b).length() < 0.1,
                "Edge discontinuity at v={v}: distance = {}",
                (pt_a - pt_b).length()
            );
        }
    }

    #[test]
    fn test_tangent_warp_face_center_maps_to_normal() {
        for face in CubeFace::ALL {
            let fc = FaceCoord::new(face, 0.5, 0.5);
            let sphere_pt = face_coord_to_sphere(&fc);
            let expected = face.normal();
            assert!(
                (sphere_pt - expected).length() < EPSILON,
                "Tangent-warp: face center of {face:?} did not map to normal"
            );
        }
    }

    #[test]
    fn test_tangent_warp_outputs_on_unit_sphere() {
        for face in CubeFace::ALL {
            for u_steps in 0..=10 {
                for v_steps in 0..=10 {
                    let u = u_steps as f64 / 10.0;
                    let v = v_steps as f64 / 10.0;
                    let fc = FaceCoord::new(face, u, v);
                    let sphere_pt = face_coord_to_sphere(&fc);
                    assert!(
                        (sphere_pt.length() - 1.0).abs() < EPSILON,
                        "Tangent-warp: not on unit sphere for {face:?} at ({u}, {v})"
                    );
                }
            }
        }
    }

    #[test]
    fn test_cube_point_face_center_is_normal() {
        for face in CubeFace::ALL {
            let fc = FaceCoord::new(face, 0.5, 0.5);
            let cube_pt = face_coord_to_cube_point(&fc);
            let expected = face.normal();
            assert!(
                (cube_pt - expected).length() < EPSILON,
                "Cube point at face center of {face:?} should equal normal"
            );
        }
    }
}
```
