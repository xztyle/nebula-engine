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

    /// Returns the dot product: x₁x₂ + y₁y₂ + z₁z₂
    ///
    /// # Overflow
    /// Each multiplication can produce a value up to i128::MAX.
    /// The sum of three such products can overflow i128.
    /// Safe when each component magnitude is below 2⁶² (~4.6×10¹⁸),
    /// guaranteeing each product fits in i128 and the triple-sum
    /// stays within range.
    ///
    /// For components up to ~2⁴¹ (~2.2×10¹²), the result fits
    /// comfortably with no risk.
    pub fn dot(self, rhs: Vec3I128) -> i128 {
        self.x * rhs.x + self.y * rhs.y + self.z * rhs.z
    }

    /// Returns the cross product self × rhs.
    ///
    /// Each output component is a difference of two products:
    ///   result.x = self.y * rhs.z - self.z * rhs.y
    ///   result.y = self.z * rhs.x - self.x * rhs.z
    ///   result.z = self.x * rhs.y - self.y * rhs.x
    ///
    /// # Overflow
    /// Same constraints as dot product — each component pair
    /// multiplication must fit i128, and their difference must
    /// not overflow. Safe for components below 2⁶² in magnitude.
    pub fn cross(self, rhs: Vec3I128) -> Vec3I128 {
        Vec3I128::new(
            self.y * rhs.z - self.z * rhs.y,
            self.z * rhs.x - self.x * rhs.z,
            self.x * rhs.y - self.y * rhs.x,
        )
    }

    /// Returns a vector whose each component is the minimum
    /// of the corresponding components of self and rhs.
    pub fn component_min(self, rhs: Vec3I128) -> Vec3I128 {
        Vec3I128::new(self.x.min(rhs.x), self.y.min(rhs.y), self.z.min(rhs.z))
    }

    /// Returns a vector whose each component is the maximum
    /// of the corresponding components of self and rhs.
    pub fn component_max(self, rhs: Vec3I128) -> Vec3I128 {
        Vec3I128::new(self.x.max(rhs.x), self.y.max(rhs.y), self.z.max(rhs.z))
    }

    /// Checked dot product that returns None on overflow.
    pub fn checked_dot(self, rhs: Vec3I128) -> Option<i128> {
        let x_product = self.x.checked_mul(rhs.x)?;
        let y_product = self.y.checked_mul(rhs.y)?;
        let z_product = self.z.checked_mul(rhs.z)?;
        x_product.checked_add(y_product)?.checked_add(z_product)
    }

    /// Checked cross product that returns None on overflow.
    pub fn checked_cross(self, rhs: Vec3I128) -> Option<Vec3I128> {
        let x_comp = self
            .y
            .checked_mul(rhs.z)?
            .checked_sub(self.z.checked_mul(rhs.y)?)?;
        let y_comp = self
            .z
            .checked_mul(rhs.x)?
            .checked_sub(self.x.checked_mul(rhs.z)?)?;
        let z_comp = self
            .x
            .checked_mul(rhs.y)?
            .checked_sub(self.y.checked_mul(rhs.x)?)?;
        Some(Vec3I128::new(x_comp, y_comp, z_comp))
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

    /// Returns the 2D dot product: x₁x₂ + y₁y₂
    pub fn dot(self, rhs: Vec2I128) -> i128 {
        self.x * rhs.x + self.y * rhs.y
    }

    /// Returns the 2D "cross product" scalar: x₁y₂ - y₁x₂
    /// (sometimes called the perp-dot product).
    pub fn perp_dot(self, rhs: Vec2I128) -> i128 {
        self.x * rhs.y - self.y * rhs.x
    }

    /// Returns a vector whose each component is the minimum
    /// of the corresponding components of self and rhs.
    pub fn component_min(self, rhs: Vec2I128) -> Vec2I128 {
        Vec2I128::new(self.x.min(rhs.x), self.y.min(rhs.y))
    }

    /// Returns a vector whose each component is the maximum
    /// of the corresponding components of self and rhs.
    pub fn component_max(self, rhs: Vec2I128) -> Vec2I128 {
        Vec2I128::new(self.x.max(rhs.x), self.y.max(rhs.y))
    }

    /// Checked dot product that returns None on overflow.
    pub fn checked_dot(self, rhs: Vec2I128) -> Option<i128> {
        let x_product = self.x.checked_mul(rhs.x)?;
        let y_product = self.y.checked_mul(rhs.y)?;
        x_product.checked_add(y_product)
    }

    /// Checked perp-dot product that returns None on overflow.
    pub fn checked_perp_dot(self, rhs: Vec2I128) -> Option<i128> {
        let first_product = self.x.checked_mul(rhs.y)?;
        let second_product = self.y.checked_mul(rhs.x)?;
        first_product.checked_sub(second_product)
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

    #[test]
    fn test_dot_orthogonal_is_zero() {
        let x = Vec3I128::unit_x();
        let y = Vec3I128::unit_y();
        assert_eq!(x.dot(y), 0);
    }

    #[test]
    fn test_dot_parallel() {
        let v = Vec3I128::new(3, 0, 0);
        let w = Vec3I128::new(5, 0, 0);
        assert_eq!(v.dot(w), 15);
    }

    #[test]
    fn test_dot_antiparallel() {
        let v = Vec3I128::new(3, 0, 0);
        let w = Vec3I128::new(-5, 0, 0);
        assert_eq!(v.dot(w), -15);
    }

    #[test]
    fn test_dot_general() {
        let a = Vec3I128::new(1, 2, 3);
        let b = Vec3I128::new(4, 5, 6);
        // 1*4 + 2*5 + 3*6 = 4 + 10 + 18 = 32
        assert_eq!(a.dot(b), 32);
    }

    #[test]
    fn test_cross_basis_vectors() {
        let x = Vec3I128::unit_x();
        let y = Vec3I128::unit_y();
        let z = Vec3I128::unit_z();
        assert_eq!(x.cross(y), z); // x × y = z
        assert_eq!(y.cross(z), x); // y × z = x
        assert_eq!(z.cross(x), y); // z × x = y
    }

    #[test]
    fn test_cross_anti_commutativity() {
        let a = Vec3I128::new(1, 2, 3);
        let b = Vec3I128::new(4, 5, 6);
        assert_eq!(a.cross(b), -b.cross(a));
    }

    #[test]
    fn test_cross_self_is_zero() {
        let v = Vec3I128::new(7, 11, 13);
        assert_eq!(v.cross(v), Vec3I128::zero());
    }

    #[test]
    fn test_component_min() {
        let a = Vec3I128::new(1, 5, 3);
        let b = Vec3I128::new(4, 2, 6);
        assert_eq!(a.component_min(b), Vec3I128::new(1, 2, 3));
    }

    #[test]
    fn test_component_max() {
        let a = Vec3I128::new(1, 5, 3);
        let b = Vec3I128::new(4, 2, 6);
        assert_eq!(a.component_max(b), Vec3I128::new(4, 5, 6));
    }

    #[test]
    fn test_checked_dot_overflow_returns_none() {
        let v = Vec3I128::new(i128::MAX, 0, 0);
        let w = Vec3I128::new(2, 0, 0);
        assert!(v.checked_dot(w).is_none());
    }

    #[test]
    fn test_checked_cross_overflow_returns_none() {
        let v = Vec3I128::new(0, i128::MAX, 0);
        let w = Vec3I128::new(0, 0, 2);
        assert!(v.checked_cross(w).is_none());
    }

    #[test]
    fn test_vec2_dot() {
        let a = Vec2I128::new(3, 4);
        let b = Vec2I128::new(1, 2);
        assert_eq!(a.dot(b), 11); // 3*1 + 4*2
    }

    #[test]
    fn test_vec2_perp_dot() {
        let a = Vec2I128::new(1, 0);
        let b = Vec2I128::new(0, 1);
        assert_eq!(a.perp_dot(b), 1); // 1*1 - 0*0
    }
}
