//! i128/u128 vector types, fixed-point arithmetic, and fundamental math operations for the Nebula Engine.

use std::fmt;

/// Canonical position in the universe. Each unit equals 1 millimeter.
///
/// The i128 range of ±1.7×10³⁸ units corresponds to ±1.7×10³⁵ kilometers,
/// or roughly ±18 billion light-years — more than the observable universe.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct WorldPosition {
    pub x: i128,
    pub y: i128,
    pub z: i128,
}

impl WorldPosition {
    /// Create a new WorldPosition with the given coordinates.
    pub fn new(x: i128, y: i128, z: i128) -> Self {
        Self { x, y, z }
    }
}

impl fmt::Display for WorldPosition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "WorldPosition({}, {}, {})", self.x, self.y, self.z)
    }
}

impl From<(i128, i128, i128)> for WorldPosition {
    fn from((x, y, z): (i128, i128, i128)) -> Self {
        Self::new(x, y, z)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_construction() {
        let pos = WorldPosition::new(10, -20, 30);
        assert_eq!(pos.x, 10);
        assert_eq!(pos.y, -20);
        assert_eq!(pos.z, 30);
    }

    #[test]
    fn test_default_is_origin() {
        let pos = WorldPosition::default();
        assert_eq!(pos.x, 0);
        assert_eq!(pos.y, 0);
        assert_eq!(pos.z, 0);
    }

    #[test]
    fn test_display_format() {
        let pos = WorldPosition::new(1, -2, 3);
        let s = format!("{}", pos);
        assert_eq!(s, "WorldPosition(1, -2, 3)");
    }

    #[test]
    fn test_equality() {
        let a = WorldPosition::new(5, 5, 5);
        let b = WorldPosition::new(5, 5, 5);
        let c = WorldPosition::new(5, 5, 6);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn test_hashing_same_position() {
        let mut set = HashSet::new();
        set.insert(WorldPosition::new(1, 2, 3));
        set.insert(WorldPosition::new(1, 2, 3));
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn test_hashing_different_positions() {
        let mut set = HashSet::new();
        set.insert(WorldPosition::new(1, 2, 3));
        set.insert(WorldPosition::new(4, 5, 6));
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn test_from_tuple() {
        let pos: WorldPosition = (100i128, 200i128, 300i128).into();
        assert_eq!(pos, WorldPosition::new(100, 200, 300));
    }

    #[test]
    fn test_extreme_values() {
        let pos = WorldPosition::new(i128::MAX, i128::MIN, 0);
        assert_eq!(pos.x, i128::MAX);
        assert_eq!(pos.y, i128::MIN);
        assert_eq!(pos.z, 0);
    }

    #[test]
    fn test_copy_semantics() {
        let a = WorldPosition::new(1, 2, 3);
        let b = a; // Copy
        assert_eq!(a, b); // `a` is still valid
    }
}
