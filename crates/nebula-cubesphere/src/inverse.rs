//! Sphere-to-cube inverse projection: recover face and UV from a direction or sphere point.

use glam::DVec3;

use crate::projection::face_coord_to_sphere_everitt;
use crate::{CubeFace, FaceCoord};

/// Determine which cube face a direction vector belongs to.
///
/// The face is determined by the axis with the largest absolute component.
/// Ties are broken by a fixed priority: X > Y > Z, positive > negative.
/// A zero vector maps to [`CubeFace::PosX`].
#[must_use]
pub fn direction_to_face(dir: DVec3) -> CubeFace {
    let ax = dir.x.abs();
    let ay = dir.y.abs();
    let az = dir.z.abs();

    if ax >= ay && ax >= az {
        if dir.x >= 0.0 {
            CubeFace::PosX
        } else {
            CubeFace::NegX
        }
    } else if ay >= az {
        if dir.y >= 0.0 {
            CubeFace::PosY
        } else {
            CubeFace::NegY
        }
    } else if dir.z >= 0.0 {
        CubeFace::PosZ
    } else {
        CubeFace::NegZ
    }
}

/// Convert a direction vector to a [`FaceCoord`] by simple projection.
///
/// This is the exact inverse of [`face_coord_to_cube_point`](crate::face_coord_to_cube_point)
/// followed by normalization. The direction does not need to be unit length.
#[must_use]
pub fn direction_to_face_coord(dir: DVec3) -> FaceCoord {
    let face = direction_to_face(dir);
    let n = face.normal();
    let tang = face.tangent();
    let bitan = face.bitangent();

    // Project onto the cube face plane.
    let d = dir.dot(n);
    // Guard against zero (degenerate direction along the face plane).
    if d.abs() < 1e-30 {
        return FaceCoord::new(face, 0.5, 0.5);
    }
    let projected = dir / d;

    let s = projected.dot(tang);
    let t = projected.dot(bitan);

    let u = (s + 1.0) * 0.5;
    let v = (t + 1.0) * 0.5;

    FaceCoord::new(face, u, v)
}

/// Inverse of the Everitt cube-to-sphere mapping.
///
/// Given a unit sphere point, returns the [`FaceCoord`] that would produce it
/// via [`face_coord_to_sphere_everitt`]. Uses Newton-Raphson iteration for
/// sub-epsilon accuracy. Typically converges in 3–5 iterations.
#[must_use]
pub fn sphere_to_face_coord_everitt(sphere_point: DVec3) -> FaceCoord {
    let approx = direction_to_face_coord(sphere_point);
    let face = approx.face;

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

        let du = 1e-8;
        let dv = 1e-8;
        let fc_du = FaceCoord::new_unchecked(face, (u + du).min(1.0), v);
        let fc_dv = FaceCoord::new_unchecked(face, u, (v + dv).min(1.0));
        let dp_du = (face_coord_to_sphere_everitt(&fc_du) - current) / du;
        let dp_dv = (face_coord_to_sphere_everitt(&fc_dv) - current) / dv;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projection::face_coord_to_cube_point;

    const EPSILON: f64 = 1e-9;

    #[test]
    fn test_roundtrip_cube_sphere_cube() {
        // Use interior points (1..9) to avoid face-edge ambiguity where
        // multiple faces share the same sphere point.
        for face in CubeFace::ALL {
            for u_steps in 1..=9 {
                for v_steps in 1..=9 {
                    let u = u_steps as f64 / 10.0;
                    let v = v_steps as f64 / 10.0;
                    let original = FaceCoord::new(face, u, v);
                    let sphere_pt = face_coord_to_sphere_everitt(&original);
                    let recovered = sphere_to_face_coord_everitt(sphere_pt);

                    assert_eq!(
                        recovered.face, original.face,
                        "Face mismatch for {face:?} at ({u}, {v})"
                    );
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
        let edge_point = DVec3::new(1.0, 1.0, 0.0).normalize();
        let face = direction_to_face(edge_point);
        assert_eq!(face, CubeFace::PosX);
        assert_eq!(direction_to_face(edge_point), face);
    }

    #[test]
    fn test_corner_points_consistently_assigned() {
        let corner = DVec3::new(1.0, 1.0, 1.0).normalize();
        let face = direction_to_face(corner);
        assert_eq!(face, CubeFace::PosX);
    }

    #[test]
    fn test_inverse_handles_southern_hemisphere() {
        let dirs = [
            DVec3::new(0.0, -1.0, 0.0),
            DVec3::new(0.3, -0.9, 0.2).normalize(),
            DVec3::new(-0.5, -0.7, 0.5).normalize(),
        ];
        for dir in dirs {
            let fc = direction_to_face_coord(dir);
            assert!(fc.u >= 0.0 && fc.u <= 1.0, "u out of range for {dir:?}");
            assert!(fc.v >= 0.0 && fc.v <= 1.0, "v out of range for {dir:?}");

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

    #[test]
    fn test_direction_to_face_zero_vector() {
        let face = direction_to_face(DVec3::ZERO);
        assert_eq!(face, CubeFace::PosX);
    }
}
