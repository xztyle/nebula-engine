//! The six faces of a cubesphere and their basis vectors.

use glam::DVec3;

/// The six faces of the cube that forms the cubesphere.
///
/// Each variant corresponds to a face whose outward normal points
/// along the named axis direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum CubeFace {
    /// +X face
    PosX = 0,
    /// −X face
    NegX = 1,
    /// +Y face
    PosY = 2,
    /// −Y face
    NegY = 3,
    /// +Z face
    PosZ = 4,
    /// −Z face
    NegZ = 5,
}

impl CubeFace {
    /// All six faces in canonical order.
    pub const ALL: [CubeFace; 6] = [
        CubeFace::PosX,
        CubeFace::NegX,
        CubeFace::PosY,
        CubeFace::NegY,
        CubeFace::PosZ,
        CubeFace::NegZ,
    ];

    /// The opposite face (e.g., `PosX` → `NegX`).
    #[must_use]
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

    /// Outward-pointing unit normal for this face.
    #[must_use]
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
    #[must_use]
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
    #[must_use]
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

#[cfg(test)]
mod tests {
    use super::*;

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
            assert!(
                (t.length() - 1.0).abs() < 1e-12,
                "Tangent not unit for {face:?}"
            );
            assert!(
                (b.length() - 1.0).abs() < 1e-12,
                "Bitangent not unit for {face:?}"
            );
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
    fn test_opposite_is_involution() {
        for face in CubeFace::ALL {
            assert_eq!(face.opposite().opposite(), face);
        }
    }
}
