//! Cardinal directions for voxel geometry: face (6), edge (12), and corner (8).

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

    /// Returns the sweep axes for greedy meshing: `(layer_axis, u_axis, v_axis)`.
    ///
    /// `layer_axis` is the axis perpendicular to the face (the normal direction).
    /// `u_axis` and `v_axis` span the face plane. Each value is 0=X, 1=Y, 2=Z.
    pub fn sweep_axes(self) -> (usize, usize, usize) {
        match self {
            Self::PosX | Self::NegX => (0, 2, 1), // layer=X, u=Z, v=Y
            Self::PosY | Self::NegY => (1, 0, 2), // layer=Y, u=X, v=Z
            Self::PosZ | Self::NegZ => (2, 0, 1), // layer=Z, u=X, v=Y
        }
    }

    /// Returns the unit normal as `[f32; 3]` for this face direction.
    pub fn normal(self) -> [f32; 3] {
        match self {
            Self::PosX => [1.0, 0.0, 0.0],
            Self::NegX => [-1.0, 0.0, 0.0],
            Self::PosY => [0.0, 1.0, 0.0],
            Self::NegY => [0.0, -1.0, 0.0],
            Self::PosZ => [0.0, 0.0, 1.0],
            Self::NegZ => [0.0, 0.0, -1.0],
        }
    }

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

    /// Returns the opposite face direction.
    pub fn opposite(self) -> Self {
        match self {
            Self::PosX => Self::NegX,
            Self::NegX => Self::PosX,
            Self::PosY => Self::NegY,
            Self::NegY => Self::PosY,
            Self::PosZ => Self::NegZ,
            Self::NegZ => Self::PosZ,
        }
    }

    /// Returns the direction index (0–5).
    pub fn index(self) -> usize {
        self as usize
    }
}

/// One of 12 edge-adjacent directions (two axes out of bounds).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum EdgeDirection {
    /// +X +Y edge.
    PosXPosY = 0,
    /// +X −Y edge.
    PosXNegY = 1,
    /// +X +Z edge.
    PosXPosZ = 2,
    /// +X −Z edge.
    PosXNegZ = 3,
    /// −X +Y edge.
    NegXPosY = 4,
    /// −X −Y edge.
    NegXNegY = 5,
    /// −X +Z edge.
    NegXPosZ = 6,
    /// −X −Z edge.
    NegXNegZ = 7,
    /// +Y +Z edge.
    PosYPosZ = 8,
    /// +Y −Z edge.
    PosYNegZ = 9,
    /// −Y +Z edge.
    NegYPosZ = 10,
    /// −Y −Z edge.
    NegYNegZ = 11,
}

impl EdgeDirection {
    /// All 12 edge directions.
    pub const ALL: [EdgeDirection; 12] = [
        Self::PosXPosY,
        Self::PosXNegY,
        Self::PosXPosZ,
        Self::PosXNegZ,
        Self::NegXPosY,
        Self::NegXNegY,
        Self::NegXPosZ,
        Self::NegXNegZ,
        Self::PosYPosZ,
        Self::PosYNegZ,
        Self::NegYPosZ,
        Self::NegYNegZ,
    ];

    /// Returns the direction index (0–11).
    pub fn index(self) -> usize {
        self as usize
    }
}

/// One of 8 corner-adjacent directions (all three axes out of bounds).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum CornerDirection {
    /// −X −Y −Z corner.
    NegXNegYNegZ = 0,
    /// +X −Y −Z corner.
    PosXNegYNegZ = 1,
    /// −X +Y −Z corner.
    NegXPosYNegZ = 2,
    /// +X +Y −Z corner.
    PosXPosYNegZ = 3,
    /// −X −Y +Z corner.
    NegXNegYPosZ = 4,
    /// +X −Y +Z corner.
    PosXNegYPosZ = 5,
    /// −X +Y +Z corner.
    NegXPosYPosZ = 6,
    /// +X +Y +Z corner.
    PosXPosYPosZ = 7,
}

impl CornerDirection {
    /// All 8 corner directions.
    pub const ALL: [CornerDirection; 8] = [
        Self::NegXNegYNegZ,
        Self::PosXNegYNegZ,
        Self::NegXPosYNegZ,
        Self::PosXPosYNegZ,
        Self::NegXNegYPosZ,
        Self::PosXNegYPosZ,
        Self::NegXPosYPosZ,
        Self::PosXPosYPosZ,
    ];

    /// Returns the direction index (0–7).
    pub fn index(self) -> usize {
        self as usize
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
