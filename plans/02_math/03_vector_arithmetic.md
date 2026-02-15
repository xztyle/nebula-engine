# Vector Arithmetic

## Problem

Basic vector algebra — dot product, cross product, component-wise min/max — is required for nearly every spatial computation in the engine: collision normals, face orientation on the cubesphere, frustum culling planes, lighting direction. These operations must work in the i128 domain so that precision is maintained for large-scale computations before any conversion to floating point occurs. However, i128 dot and cross products risk intermediate overflow when operands are large, so the safe operating ranges must be clearly documented.

## Solution

Implement the following as methods on `Vec3I128`:

### Dot product

```rust
impl Vec3I128 {
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
}
```

### Cross product

```rust
impl Vec3I128 {
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
}
```

### Component-wise min / max

```rust
impl Vec3I128 {
    /// Returns a vector whose each component is the minimum
    /// of the corresponding components of self and rhs.
    pub fn component_min(self, rhs: Vec3I128) -> Vec3I128 {
        Vec3I128::new(
            self.x.min(rhs.x),
            self.y.min(rhs.y),
            self.z.min(rhs.z),
        )
    }

    /// Returns a vector whose each component is the maximum
    /// of the corresponding components of self and rhs.
    pub fn component_max(self, rhs: Vec3I128) -> Vec3I128 {
        Vec3I128::new(
            self.x.max(rhs.x),
            self.y.max(rhs.y),
            self.z.max(rhs.z),
        )
    }
}
```

### Checked variants

Provide `checked_dot` returning `Option<i128>` and `checked_cross` returning `Option<Vec3I128>` that use `i128::checked_mul` and `checked_add`/`checked_sub` internally, returning `None` on any overflow.

### Design notes

- All operations are `const`-eligible once Rust edition 2024 stabilizes const trait impls, but initially implemented as regular `fn`.
- These methods live on `Vec3I128`, not as free functions, to keep the API discoverable.
- `Vec2I128` gets a 2D dot product (`x₁x₂ + y₁y₂`) and a 2D "cross product" scalar (`x₁y₂ - y₁x₂`, sometimes called the perp-dot product).

## Outcome

After this story is complete:

- `Vec3I128::dot()` and `Vec3I128::cross()` are available for spatial math
- Checked variants are available for safety-critical paths
- Component-wise min/max enables AABB construction (story 08)
- 2D variants exist for cubesphere face math

## Demo Integration

**Demo crate:** `nebula-demo`

The position update now uses a diagonal displacement via proper vector addition; both X and Z coordinates advance visibly in the title.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| *(none)* | — | Pure `std` only |

Rust edition 2024. No external crates needed.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(x.cross(y), z);   // x × y = z
        assert_eq!(y.cross(z), x);   // y × z = x
        assert_eq!(z.cross(x), y);   // z × x = y
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
```
