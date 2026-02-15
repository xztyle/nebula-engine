use std::fmt;
use std::ops::{Add, AddAssign, Div, Mul, MulAssign, Neg, Sub, SubAssign};

/// 3D displacement / direction vector in i128 space.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Vec3I128 {
    pub x: i128,
    pub y: i128,
    pub z: i128,
}

impl Vec3I128 {
    /// Create a new Vec3I128 with the given coordinates.
    pub fn new(x: i128, y: i128, z: i128) -> Self {
        Self { x, y, z }
    }

    /// Zero vector (0, 0, 0).
    pub fn zero() -> Self {
        Self::default()
    }

    /// Unit vector in the X direction (1, 0, 0).
    pub fn unit_x() -> Self {
        Self::new(1, 0, 0)
    }

    /// Unit vector in the Y direction (0, 1, 0).
    pub fn unit_y() -> Self {
        Self::new(0, 1, 0)
    }

    /// Unit vector in the Z direction (0, 0, 1).
    pub fn unit_z() -> Self {
        Self::new(0, 0, 1)
    }

    /// Checked addition that returns None on overflow.
    pub fn checked_add(self, rhs: Vec3I128) -> Option<Vec3I128> {
        Some(Vec3I128::new(
            self.x.checked_add(rhs.x)?,
            self.y.checked_add(rhs.y)?,
            self.z.checked_add(rhs.z)?,
        ))
    }

    /// Checked subtraction that returns None on overflow.
    pub fn checked_sub(self, rhs: Vec3I128) -> Option<Vec3I128> {
        Some(Vec3I128::new(
            self.x.checked_sub(rhs.x)?,
            self.y.checked_sub(rhs.y)?,
            self.z.checked_sub(rhs.z)?,
        ))
    }

    /// Saturating addition that clamps to i128::MIN/MAX on overflow.
    pub fn saturating_add(self, rhs: Vec3I128) -> Vec3I128 {
        Vec3I128::new(
            self.x.saturating_add(rhs.x),
            self.y.saturating_add(rhs.y),
            self.z.saturating_add(rhs.z),
        )
    }

    /// Saturating subtraction that clamps to i128::MIN/MAX on overflow.
    pub fn saturating_sub(self, rhs: Vec3I128) -> Vec3I128 {
        Vec3I128::new(
            self.x.saturating_sub(rhs.x),
            self.y.saturating_sub(rhs.y),
            self.z.saturating_sub(rhs.z),
        )
    }
}

impl fmt::Display for Vec3I128 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Vec3I128({}, {}, {})", self.x, self.y, self.z)
    }
}

impl Add for Vec3I128 {
    type Output = Vec3I128;

    fn add(self, rhs: Vec3I128) -> Self::Output {
        Vec3I128::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

impl Sub for Vec3I128 {
    type Output = Vec3I128;

    fn sub(self, rhs: Vec3I128) -> Self::Output {
        Vec3I128::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

impl Neg for Vec3I128 {
    type Output = Vec3I128;

    fn neg(self) -> Self::Output {
        Vec3I128::new(-self.x, -self.y, -self.z)
    }
}

impl Mul<i128> for Vec3I128 {
    type Output = Vec3I128;

    fn mul(self, rhs: i128) -> Self::Output {
        Vec3I128::new(self.x * rhs, self.y * rhs, self.z * rhs)
    }
}

impl Div<i128> for Vec3I128 {
    type Output = Vec3I128;

    fn div(self, rhs: i128) -> Self::Output {
        Vec3I128::new(self.x / rhs, self.y / rhs, self.z / rhs)
    }
}

impl AddAssign for Vec3I128 {
    fn add_assign(&mut self, rhs: Vec3I128) {
        self.x += rhs.x;
        self.y += rhs.y;
        self.z += rhs.z;
    }
}

impl SubAssign for Vec3I128 {
    fn sub_assign(&mut self, rhs: Vec3I128) {
        self.x -= rhs.x;
        self.y -= rhs.y;
        self.z -= rhs.z;
    }
}

impl MulAssign<i128> for Vec3I128 {
    fn mul_assign(&mut self, rhs: i128) {
        self.x *= rhs;
        self.y *= rhs;
        self.z *= rhs;
    }
}

/// 2D vector in i128 space, used for cubesphere face-local coordinates.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Vec2I128 {
    pub x: i128,
    pub y: i128,
}

impl Vec2I128 {
    /// Create a new Vec2I128 with the given coordinates.
    pub fn new(x: i128, y: i128) -> Self {
        Self { x, y }
    }

    /// Zero vector (0, 0).
    pub fn zero() -> Self {
        Self::default()
    }

    /// Unit vector in the X direction (1, 0).
    pub fn unit_x() -> Self {
        Self::new(1, 0)
    }

    /// Unit vector in the Y direction (0, 1).
    pub fn unit_y() -> Self {
        Self::new(0, 1)
    }

    /// Checked addition that returns None on overflow.
    pub fn checked_add(self, rhs: Vec2I128) -> Option<Vec2I128> {
        Some(Vec2I128::new(
            self.x.checked_add(rhs.x)?,
            self.y.checked_add(rhs.y)?,
        ))
    }

    /// Checked subtraction that returns None on overflow.
    pub fn checked_sub(self, rhs: Vec2I128) -> Option<Vec2I128> {
        Some(Vec2I128::new(
            self.x.checked_sub(rhs.x)?,
            self.y.checked_sub(rhs.y)?,
        ))
    }

    /// Saturating addition that clamps to i128::MIN/MAX on overflow.
    pub fn saturating_add(self, rhs: Vec2I128) -> Vec2I128 {
        Vec2I128::new(self.x.saturating_add(rhs.x), self.y.saturating_add(rhs.y))
    }

    /// Saturating subtraction that clamps to i128::MIN/MAX on overflow.
    pub fn saturating_sub(self, rhs: Vec2I128) -> Vec2I128 {
        Vec2I128::new(self.x.saturating_sub(rhs.x), self.y.saturating_sub(rhs.y))
    }
}

impl fmt::Display for Vec2I128 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Vec2I128({}, {})", self.x, self.y)
    }
}

impl Add for Vec2I128 {
    type Output = Vec2I128;

    fn add(self, rhs: Vec2I128) -> Self::Output {
        Vec2I128::new(self.x + rhs.x, self.y + rhs.y)
    }
}

impl Sub for Vec2I128 {
    type Output = Vec2I128;

    fn sub(self, rhs: Vec2I128) -> Self::Output {
        Vec2I128::new(self.x - rhs.x, self.y - rhs.y)
    }
}

impl Neg for Vec2I128 {
    type Output = Vec2I128;

    fn neg(self) -> Self::Output {
        Vec2I128::new(-self.x, -self.y)
    }
}

impl Mul<i128> for Vec2I128 {
    type Output = Vec2I128;

    fn mul(self, rhs: i128) -> Self::Output {
        Vec2I128::new(self.x * rhs, self.y * rhs)
    }
}

impl Div<i128> for Vec2I128 {
    type Output = Vec2I128;

    fn div(self, rhs: i128) -> Self::Output {
        Vec2I128::new(self.x / rhs, self.y / rhs)
    }
}

impl AddAssign for Vec2I128 {
    fn add_assign(&mut self, rhs: Vec2I128) {
        self.x += rhs.x;
        self.y += rhs.y;
    }
}

impl SubAssign for Vec2I128 {
    fn sub_assign(&mut self, rhs: Vec2I128) {
        self.x -= rhs.x;
        self.y -= rhs.y;
    }
}

impl MulAssign<i128> for Vec2I128 {
    fn mul_assign(&mut self, rhs: i128) {
        self.x *= rhs;
        self.y *= rhs;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add() {
        let a = Vec3I128::new(1, 2, 3);
        let b = Vec3I128::new(10, 20, 30);
        assert_eq!(a + b, Vec3I128::new(11, 22, 33));
    }

    #[test]
    fn test_sub() {
        let a = Vec3I128::new(10, 20, 30);
        let b = Vec3I128::new(1, 2, 3);
        assert_eq!(a - b, Vec3I128::new(9, 18, 27));
    }

    #[test]
    fn test_neg() {
        let v = Vec3I128::new(1, -2, 3);
        assert_eq!(-v, Vec3I128::new(-1, 2, -3));
    }

    #[test]
    fn test_scalar_mul() {
        let v = Vec3I128::new(2, 3, 4);
        assert_eq!(v * 10, Vec3I128::new(20, 30, 40));
    }

    #[test]
    fn test_scalar_div() {
        let v = Vec3I128::new(20, 30, 40);
        assert_eq!(v / 10, Vec3I128::new(2, 3, 4));
    }

    #[test]
    fn test_scalar_div_truncates() {
        let v = Vec3I128::new(7, 7, 7);
        assert_eq!(v / 2, Vec3I128::new(3, 3, 3)); // truncation toward zero
    }

    #[test]
    fn test_zero_vector() {
        let z = Vec3I128::zero();
        assert_eq!(z, Vec3I128::new(0, 0, 0));
        assert_eq!(z, Vec3I128::default());
    }

    #[test]
    fn test_basis_vectors() {
        assert_eq!(Vec3I128::unit_x(), Vec3I128::new(1, 0, 0));
        assert_eq!(Vec3I128::unit_y(), Vec3I128::new(0, 1, 0));
        assert_eq!(Vec3I128::unit_z(), Vec3I128::new(0, 0, 1));
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic]
    fn test_overflow_panics_in_debug() {
        let v = Vec3I128::new(i128::MAX, 0, 0);
        let _ = v + Vec3I128::new(1, 0, 0); // overflow
    }

    #[test]
    fn test_checked_add_overflow() {
        let v = Vec3I128::new(i128::MAX, 0, 0);
        assert!(v.checked_add(Vec3I128::new(1, 0, 0)).is_none());
    }

    #[test]
    fn test_saturating_add() {
        let v = Vec3I128::new(i128::MAX, 0, 0);
        let result = v.saturating_add(Vec3I128::new(1, 0, 0));
        assert_eq!(result.x, i128::MAX);
    }

    #[test]
    fn test_vec2_add() {
        let a = Vec2I128::new(1, 2);
        let b = Vec2I128::new(10, 20);
        assert_eq!(a + b, Vec2I128::new(11, 22));
    }
}
