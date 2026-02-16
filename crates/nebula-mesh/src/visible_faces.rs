//! Bitmask for tracking which of a voxel's six faces are visible.

use crate::face_direction::FaceDirection;

/// Bitmask indicating which of a voxel's 6 faces are visible.
///
/// Bit 0 = +X, Bit 1 = −X, Bit 2 = +Y, Bit 3 = −Y, Bit 4 = +Z, Bit 5 = −Z.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VisibleFaces(pub u8);

impl VisibleFaces {
    /// No faces visible.
    pub const NONE: Self = Self(0);
    /// All six faces visible.
    pub const ALL: Self = Self(0b0011_1111);

    /// Returns `true` if the face in the given direction is visible.
    pub fn is_visible(self, direction: FaceDirection) -> bool {
        self.0 & (1 << direction as u8) != 0
    }

    /// Marks the face in the given direction as visible.
    pub fn set_visible(&mut self, direction: FaceDirection) {
        self.0 |= 1 << direction as u8;
    }

    /// Returns the number of visible faces (0–6).
    pub fn count(self) -> u32 {
        self.0.count_ones()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_none_has_zero_count() {
        assert_eq!(VisibleFaces::NONE.count(), 0);
    }

    #[test]
    fn test_all_has_six_count() {
        assert_eq!(VisibleFaces::ALL.count(), 6);
    }

    #[test]
    fn test_set_and_query_individual_face() {
        let mut vf = VisibleFaces::NONE;
        vf.set_visible(FaceDirection::PosZ);
        assert!(vf.is_visible(FaceDirection::PosZ));
        assert!(!vf.is_visible(FaceDirection::NegZ));
        assert_eq!(vf.count(), 1);
    }

    #[test]
    fn test_set_all_directions_individually() {
        let mut vf = VisibleFaces::NONE;
        for dir in FaceDirection::ALL {
            vf.set_visible(dir);
        }
        assert_eq!(vf, VisibleFaces::ALL);
    }
}
