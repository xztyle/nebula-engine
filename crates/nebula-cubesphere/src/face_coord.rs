//! 2D local coordinates on a cube face.

use crate::CubeFace;

/// A 2D coordinate on a cube face. `u` and `v` are in the range \[0, 1\].
///
/// `(u=0, v=0)` is the "bottom-left" corner of the face when viewed from
/// outside the cube looking inward. `(u=1, v=1)` is the "top-right" corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FaceCoord {
    /// Which cube face this coordinate lies on.
    pub face: CubeFace,
    /// Horizontal parameter in \[0, 1\].
    pub u: f64,
    /// Vertical parameter in \[0, 1\].
    pub v: f64,
}

impl FaceCoord {
    /// Construct a `FaceCoord`, clamping `u` and `v` to \[0, 1\].
    #[must_use]
    pub fn new(face: CubeFace, u: f64, v: f64) -> Self {
        Self {
            face,
            u: u.clamp(0.0, 1.0),
            v: v.clamp(0.0, 1.0),
        }
    }

    /// Construct without clamping. Caller guarantees `0 <= u, v <= 1`.
    #[must_use]
    pub fn new_unchecked(face: CubeFace, u: f64, v: f64) -> Self {
        debug_assert!((0.0..=1.0).contains(&u), "u out of range: {u}");
        debug_assert!((0.0..=1.0).contains(&v), "v out of range: {v}");
        Self { face, u, v }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
