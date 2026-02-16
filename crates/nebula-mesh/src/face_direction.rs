//! Six cardinal face directions for voxel geometry.

/// One of the six cardinal directions a voxel face can point.
///
/// The `repr(u8)` discriminant doubles as the bit index inside [`super::VisibleFaces`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum FaceDirection {
    /// +X direction.
    PosX = 0,
    /// −X direction.
    NegX = 1,
    /// +Y direction.
    PosY = 2,
    /// −Y direction.
    NegY = 3,
    /// +Z direction.
    PosZ = 4,
    /// −Z direction.
    NegZ = 5,
}

impl FaceDirection {
    /// All six directions in order.
    pub const ALL: [FaceDirection; 6] = [
        Self::PosX,
        Self::NegX,
        Self::PosY,
        Self::NegY,
        Self::PosZ,
        Self::NegZ,
    ];

    /// Returns the neighbor coordinate offset for this direction.
    pub fn offset(self, x: i32, y: i32, z: i32) -> (i32, i32, i32) {
        match self {
            Self::PosX => (x + 1, y, z),
            Self::NegX => (x - 1, y, z),
            Self::PosY => (x, y + 1, z),
            Self::NegY => (x, y - 1, z),
            Self::PosZ => (x, y, z + 1),
            Self::NegZ => (x, y, z - 1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_directions_unique() {
        for (i, a) in FaceDirection::ALL.iter().enumerate() {
            for (j, b) in FaceDirection::ALL.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b);
                }
            }
        }
    }

    #[test]
    fn test_offset_pos_x() {
        assert_eq!(FaceDirection::PosX.offset(5, 10, 15), (6, 10, 15));
    }

    #[test]
    fn test_offset_neg_x() {
        assert_eq!(FaceDirection::NegX.offset(5, 10, 15), (4, 10, 15));
    }

    #[test]
    fn test_offset_negative_result() {
        assert_eq!(FaceDirection::NegX.offset(0, 0, 0), (-1, 0, 0));
    }
}
