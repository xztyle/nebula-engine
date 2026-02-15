use std::fmt;
use std::ops::{Add, AddAssign, Div, Mul, MulAssign, Neg, Sub, SubAssign};

use crate::WorldPosition;

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

    /// Returns x² + y² + z².
    ///
    /// # Overflow
    /// Each component squared can reach i128::MAX if the component
    /// itself exceeds ~1.3×10¹⁹ (√(i128::MAX) ≈ 1.3×10¹⁹).
    /// The sum of three squares overflows if any component exceeds
    /// ~7.75×10¹⁸ (~2⁶² / √3).
    ///
    /// For positions within a single solar system (~10¹⁵ mm), this
    /// is safe. For interstellar distances, use magnitude_f64().
    pub fn magnitude_squared(self) -> i128 {
        self.x * self.x + self.y * self.y + self.z * self.z
    }

    /// Converts to f64 and computes √(x² + y² + z²).
    ///
    /// Precision: f64 has 53 bits of mantissa, so values above 2⁵³
    /// (~9×10¹⁵) lose precision. For distances up to ~9×10¹² km
    /// this is exact to the millimeter.
    pub fn magnitude_f64(self) -> f64 {
        let x = self.x as f64;
        let y = self.y as f64;
        let z = self.z as f64;
        (x * x + y * y + z * z).sqrt()
    }

    /// Manhattan (L1) magnitude: |x| + |y| + |z|.
    ///
    /// Cannot overflow unless the sum of three i128::MAX-magnitude
    /// values is taken, which requires components near i128::MAX/3.
    /// For all practical game distances, this is completely safe.
    pub fn manhattan_magnitude(self) -> i128 {
        self.x.abs() + self.y.abs() + self.z.abs()
    }

    /// Returns None if any intermediate multiplication or the
    /// final sum overflows i128.
    pub fn checked_magnitude_squared(self) -> Option<i128> {
        let x2 = self.x.checked_mul(self.x)?;
        let y2 = self.y.checked_mul(self.y)?;
        let z2 = self.z.checked_mul(self.z)?;
        x2.checked_add(y2)?.checked_add(z2)
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

// Free functions for distance calculations

/// Squared Euclidean distance between two world positions.
pub fn distance_squared(a: WorldPosition, b: WorldPosition) -> i128 {
    (a - b).magnitude_squared()
}

/// Euclidean distance between two world positions, returned as f64.
pub fn distance_f64(a: WorldPosition, b: WorldPosition) -> f64 {
    (a - b).magnitude_f64()
}

/// Manhattan distance between two world positions.
pub fn manhattan_distance(a: WorldPosition, b: WorldPosition) -> i128 {
    (a - b).manhattan_magnitude()
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

    #[test]
    fn test_distance_to_self_is_zero() {
        let p = WorldPosition::new(1000, 2000, 3000);
        assert_eq!(distance_squared(p, p), 0);
        assert_eq!(distance_f64(p, p), 0.0);
    }

    #[test]
    fn test_distance_3_4_5_triangle() {
        // Displacement of (3000, 4000, 0) mm -> distance = 5000 mm
        let a = WorldPosition::new(0, 0, 0);
        let b = WorldPosition::new(3000, 4000, 0);
        assert_eq!(distance_squared(a, b), 25_000_000);
        assert!((distance_f64(a, b) - 5000.0).abs() < 1e-10);
    }

    #[test]
    fn test_distance_symmetric() {
        let a = WorldPosition::new(10, 20, 30);
        let b = WorldPosition::new(40, 50, 60);
        assert_eq!(distance_squared(a, b), distance_squared(b, a));
        assert_eq!(distance_f64(a, b), distance_f64(b, a));
    }

    #[test]
    fn test_manhattan_distance() {
        let a = WorldPosition::new(0, 0, 0);
        let b = WorldPosition::new(3, 4, 5);
        assert_eq!(manhattan_distance(a, b), 12);
    }

    #[test]
    fn test_manhattan_distance_with_negatives() {
        let a = WorldPosition::new(10, 10, 10);
        let b = WorldPosition::new(7, 14, 10);
        // |3| + |-4| + |0| = 7
        assert_eq!(manhattan_distance(a, b), 7);
    }

    #[test]
    fn test_squared_distance_manual_calc() {
        let a = WorldPosition::new(1, 2, 3);
        let b = WorldPosition::new(4, 6, 8);
        // (3² + 4² + 5²) = 9 + 16 + 25 = 50
        assert_eq!(distance_squared(a, b), 50);
    }

    #[test]
    fn test_magnitude_squared() {
        let v = Vec3I128::new(3, 4, 0);
        assert_eq!(v.magnitude_squared(), 25);
    }

    #[test]
    fn test_magnitude_f64() {
        let v = Vec3I128::new(3, 4, 0);
        assert!((v.magnitude_f64() - 5.0).abs() < 1e-10);
    }

    #[test]
    fn test_checked_magnitude_squared_safe() {
        let v = Vec3I128::new(1000, 2000, 3000);
        assert_eq!(v.checked_magnitude_squared(), Some(14_000_000));
    }

    #[test]
    fn test_checked_magnitude_squared_overflow() {
        let v = Vec3I128::new(i128::MAX, 0, 0);
        assert!(v.checked_magnitude_squared().is_none());
    }

    #[test]
    fn test_large_coordinates_f64_distance() {
        // Two points 1 AU apart along x-axis
        // 1 AU ≈ 1.496×10¹¹ m = 1.496×10¹⁴ mm
        let au_mm: i128 = 149_597_870_700_000;
        let a = WorldPosition::new(0, 0, 0);
        let b = WorldPosition::new(au_mm, 0, 0);
        let d = distance_f64(a, b);
        assert!((d - au_mm as f64).abs() / (au_mm as f64) < 1e-10);
    }
}
