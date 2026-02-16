//! Cube-to-sphere projection methods.
//!
//! Provides two projection approaches:
//! - **Tangent warp**: Fast approximate equal-area via `tan(x * π/4)` remapping.
//! - **Everitt**: Analytic mapping with better area uniformity for terrain generation.

use std::f64::consts::FRAC_PI_4;

use glam::DVec3;

use crate::FaceCoord;

/// Selects which cube-to-sphere projection method to use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum ProjectionMethod {
    /// Tangent-warp projection: faster, slightly less uniform.
    TangentWarp,
    /// Everitt/Mathworld analytic projection: better area uniformity.
    #[default]
    Everitt,
}

/// Convert a [`FaceCoord`] to a point on the surface of the `[-1, 1]` cube.
///
/// The face center `(u=0.5, v=0.5)` maps to the face normal vector.
#[inline]
#[must_use]
pub fn face_coord_to_cube_point(fc: &FaceCoord) -> DVec3 {
    // Remap u, v from [0, 1] to [-1, 1]
    let s = 2.0 * fc.u - 1.0;
    let t = 2.0 * fc.v - 1.0;

    let n = fc.face.normal();
    let tang = fc.face.tangent();
    let bitan = fc.face.bitangent();

    n + s * tang + t * bitan
}

/// Project a [`FaceCoord`] onto the unit sphere using the tangent-warp method.
///
/// This produces more uniform cell areas than naive normalization by remapping
/// coordinates through `tan(x * π/4)` before normalizing.
/// Returns a unit-length `DVec3` on the sphere surface.
#[inline]
#[must_use]
pub fn face_coord_to_sphere(fc: &FaceCoord) -> DVec3 {
    let s = 2.0 * fc.u - 1.0;
    let t = 2.0 * fc.v - 1.0;

    // tan(π/4) = 1, so tan(x * π/4) is identity at x = ±1 and warps the interior.
    let ws = (s * FRAC_PI_4).tan();
    let wt = (t * FRAC_PI_4).tan();

    let n = fc.face.normal();
    let tang = fc.face.tangent();
    let bitan = fc.face.bitangent();

    let cube_point = n + ws * tang + wt * bitan;
    cube_point.normalize()
}

/// Analytic cube-to-sphere using the Everitt/Mathworld mapping.
///
/// Given a point on the cube surface (one coordinate is `±1`), compute the
/// corresponding unit sphere point with minimal area distortion:
///
/// ```text
/// sx = x * sqrt(1 - y²/2 - z²/2 + y²z²/3)
/// sy = y * sqrt(1 - x²/2 - z²/2 + x²z²/3)
/// sz = z * sqrt(1 - x²/2 - y²/2 + x²y²/3)
/// ```
#[inline]
#[must_use]
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

/// Convenience: [`FaceCoord`] → unit sphere using the Everitt mapping.
#[inline]
#[must_use]
pub fn face_coord_to_sphere_everitt(fc: &FaceCoord) -> DVec3 {
    let cube_point = face_coord_to_cube_point(fc);
    cube_to_sphere_everitt(cube_point)
}

/// Project a [`FaceCoord`] onto the unit sphere using the specified method.
#[inline]
#[must_use]
pub fn project(fc: &FaceCoord, method: ProjectionMethod) -> DVec3 {
    match method {
        ProjectionMethod::TangentWarp => face_coord_to_sphere(fc),
        ProjectionMethod::Everitt => face_coord_to_sphere_everitt(fc),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CubeFace;

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
        let steps = 20;
        for i in 0..=steps {
            let v = i as f64 / steps as f64;
            let fc_a = FaceCoord::new(CubeFace::PosX, 0.0, v);
            let fc_b = FaceCoord::new(CubeFace::PosZ, 1.0, v);
            let pt_a = face_coord_to_sphere_everitt(&fc_a);
            let pt_b = face_coord_to_sphere_everitt(&fc_b);
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

    #[test]
    fn test_project_dispatches_correctly() {
        let fc = FaceCoord::new(CubeFace::PosX, 0.3, 0.7);
        let tangent_result = face_coord_to_sphere(&fc);
        let everitt_result = face_coord_to_sphere_everitt(&fc);

        assert_eq!(project(&fc, ProjectionMethod::TangentWarp), tangent_result);
        assert_eq!(project(&fc, ProjectionMethod::Everitt), everitt_result);
    }

    #[test]
    fn test_default_projection_method_is_everitt() {
        assert_eq!(ProjectionMethod::default(), ProjectionMethod::Everitt);
    }
}
