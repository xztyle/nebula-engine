use glam::Vec3;
use std::fmt;
use std::ops::{Add, Div, Mul, Neg, Sub};

/// Position relative to a local origin (camera, chunk center, etc.)
/// in f32 space. Each unit is 1 millimeter, same as WorldPosition.
///
/// Precision: f32 has 23 mantissa bits, giving ~7 decimal digits.
/// At 1mm resolution, positions are exact up to ±8,388 meters (~8.4 km).
/// Beyond that, sub-millimeter precision degrades but sub-centimeter
/// precision holds to ~83 km.
///
/// For rendering, the origin should be set to the camera position
/// so that nearby geometry has maximum precision.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct LocalPosition {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl LocalPosition {
    /// Creates a new LocalPosition with the given coordinates.
    pub fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    /// Creates a LocalPosition at the origin (0, 0, 0).
    pub fn zero() -> Self {
        Self::default()
    }

    /// Returns true if all components are within epsilon of the other.
    pub fn approx_eq(self, other: LocalPosition, epsilon: f32) -> bool {
        (self.x - other.x).abs() < epsilon
            && (self.y - other.y).abs() < epsilon
            && (self.z - other.z).abs() < epsilon
    }
}

impl From<LocalPosition> for Vec3 {
    fn from(lp: LocalPosition) -> Vec3 {
        Vec3::new(lp.x, lp.y, lp.z)
    }
}

impl From<Vec3> for LocalPosition {
    fn from(v: Vec3) -> LocalPosition {
        LocalPosition::new(v.x, v.y, v.z)
    }
}

impl Add<LocalPosition> for LocalPosition {
    type Output = LocalPosition;

    fn add(self, rhs: LocalPosition) -> Self::Output {
        LocalPosition::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

impl Sub<LocalPosition> for LocalPosition {
    type Output = LocalPosition;

    fn sub(self, rhs: LocalPosition) -> Self::Output {
        LocalPosition::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

impl Neg for LocalPosition {
    type Output = LocalPosition;

    fn neg(self) -> Self::Output {
        LocalPosition::new(-self.x, -self.y, -self.z)
    }
}

impl Mul<f32> for LocalPosition {
    type Output = LocalPosition;

    fn mul(self, scalar: f32) -> Self::Output {
        LocalPosition::new(self.x * scalar, self.y * scalar, self.z * scalar)
    }
}

impl Div<f32> for LocalPosition {
    type Output = LocalPosition;

    fn div(self, scalar: f32) -> Self::Output {
        LocalPosition::new(self.x / scalar, self.y / scalar, self.z / scalar)
    }
}

impl fmt::Display for LocalPosition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Local({:.3}, {:.3}, {:.3})", self.x, self.y, self.z)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn test_construction() {
        let lp = LocalPosition::new(1.0, 2.0, 3.0);
        assert_eq!(lp.x, 1.0);
        assert_eq!(lp.y, 2.0);
        assert_eq!(lp.z, 3.0);
    }

    #[test]
    fn test_default_is_zero() {
        let lp = LocalPosition::default();
        assert_eq!(lp.x, 0.0);
        assert_eq!(lp.y, 0.0);
        assert_eq!(lp.z, 0.0);
    }

    #[test]
    fn test_zero() {
        let lp = LocalPosition::zero();
        assert_eq!(lp.x, 0.0);
        assert_eq!(lp.y, 0.0);
        assert_eq!(lp.z, 0.0);
    }

    #[test]
    fn test_to_glam_vec3() {
        let lp = LocalPosition::new(1.0, 2.0, 3.0);
        let v: Vec3 = lp.into();
        assert_eq!(v, Vec3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn test_from_glam_vec3() {
        let v = Vec3::new(4.0, 5.0, 6.0);
        let lp: LocalPosition = v.into();
        assert_eq!(lp, LocalPosition::new(4.0, 5.0, 6.0));
    }

    #[test]
    fn test_roundtrip_glam() {
        let original = LocalPosition::new(1.5, -2.7, 3.17);
        let roundtrip: LocalPosition = Vec3::from(original).into();
        assert!(original.approx_eq(roundtrip, 1e-6));
    }

    #[test]
    fn test_precision_at_500m() {
        // At 500,000 mm from origin, f32 should resolve to 1mm
        let lp = LocalPosition::new(500_000.0, 500_000.0, 500_000.0);
        let nudged = LocalPosition::new(500_001.0, 500_000.0, 500_000.0);
        // The 1mm difference should be preserved
        assert!((nudged.x - lp.x - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_approx_eq() {
        let a = LocalPosition::new(1.0, 2.0, 3.0);
        let b = LocalPosition::new(1.0 + 1e-7, 2.0, 3.0);
        assert!(a.approx_eq(b, 1e-6));
        assert!(!a.approx_eq(LocalPosition::new(2.0, 2.0, 3.0), 0.5));
    }

    #[test]
    fn test_add() {
        let a = LocalPosition::new(1.0, 2.0, 3.0);
        let b = LocalPosition::new(0.5, 0.5, 0.5);
        let c = a + b;
        assert!(c.approx_eq(LocalPosition::new(1.5, 2.5, 3.5), 1e-6));
    }

    #[test]
    fn test_sub() {
        let a = LocalPosition::new(3.0, 4.0, 5.0);
        let b = LocalPosition::new(1.0, 1.0, 1.0);
        let c = a - b;
        assert!(c.approx_eq(LocalPosition::new(2.0, 3.0, 4.0), 1e-6));
    }

    #[test]
    fn test_scalar_mul() {
        let a = LocalPosition::new(1.0, 2.0, 3.0);
        let b = a * 2.0;
        assert!(b.approx_eq(LocalPosition::new(2.0, 4.0, 6.0), 1e-6));
    }

    #[test]
    fn test_scalar_div() {
        let a = LocalPosition::new(6.0, 8.0, 10.0);
        let b = a / 2.0;
        assert!(b.approx_eq(LocalPosition::new(3.0, 4.0, 5.0), 1e-6));
    }

    #[test]
    fn test_neg() {
        let a = LocalPosition::new(1.0, -2.0, 3.0);
        let b = -a;
        assert!(b.approx_eq(LocalPosition::new(-1.0, 2.0, -3.0), 1e-6));
    }

    #[test]
    fn test_display() {
        let lp = LocalPosition::new(1.234, -2.567, 3.891);
        let formatted = format!("{}", lp);
        assert_eq!(formatted, "Local(1.234, -2.567, 3.891)");
    }
}
