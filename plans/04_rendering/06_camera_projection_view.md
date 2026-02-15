# Camera Projection & View Matrices

## Problem

The unlit pipeline (story 05) requires a view-projection matrix to transform vertices from local space to clip space. That matrix comes from a camera, which needs position, rotation, field of view, aspect ratio, and near/far clip planes. The camera must operate entirely in local `f32` space — the 128-bit world coordinate system is too large for GPU floats. The world-to-local conversion (subtracting the camera's `WorldPosition` from object positions to get `f32` offsets) happens before camera math, not inside the camera itself. This is the "origin rebasing" strategy that makes 128-bit coordinates compatible with standard GPU rendering.

The engine also needs orthographic projection for UI rendering, 2D views, and editor orthographic viewports. Both projection types must produce matrices compatible with wgpu's clip space conventions (NDC: x/y in [-1, 1], z in [0, 1] with reverse-Z).

## Solution

### Camera Struct

```rust
pub struct Camera {
    /// Position in local f32 space (after origin rebasing).
    pub position: Vec3,
    /// Rotation as a unit quaternion.
    pub rotation: Quat,
    /// Projection parameters.
    pub projection: Projection,
    /// Near clip plane distance (always positive).
    pub near: f32,
    /// Far clip plane distance (always positive, > near).
    pub far: f32,
}

pub enum Projection {
    Perspective {
        /// Vertical field of view in radians.
        fov_y: f32,
        /// Width / height.
        aspect_ratio: f32,
    },
    Orthographic {
        /// Half-width of the view volume in world units.
        half_width: f32,
        /// Half-height of the view volume in world units.
        half_height: f32,
    },
}
```

### View Matrix

The view matrix is the inverse of the camera's transform (position + rotation):

```rust
impl Camera {
    pub fn view_matrix(&self) -> Mat4 {
        let rotation_matrix = Mat4::from_quat(self.rotation);
        let translation_matrix = Mat4::from_translation(self.position);
        // View = inverse(Translation * Rotation) = inverse(Rotation) * inverse(Translation)
        (translation_matrix * rotation_matrix).inverse()
        // Equivalent and faster:
        // Mat4::look_to_rh(self.position, self.forward(), self.up())
    }

    /// The forward direction vector (-Z in camera space).
    pub fn forward(&self) -> Vec3 {
        self.rotation * Vec3::NEG_Z
    }

    /// The up direction vector (+Y in camera space).
    pub fn up(&self) -> Vec3 {
        self.rotation * Vec3::Y
    }

    /// The right direction vector (+X in camera space).
    pub fn right(&self) -> Vec3 {
        self.rotation * Vec3::X
    }
}
```

The camera uses a right-handed coordinate system where -Z is forward, +Y is up, and +X is right. This matches wgpu/WebGPU conventions.

### Projection Matrix

```rust
impl Camera {
    pub fn projection_matrix(&self) -> Mat4 {
        match &self.projection {
            Projection::Perspective { fov_y, aspect_ratio } => {
                // Reverse-Z: near plane maps to z=1, far plane maps to z=0.
                // This is handled by swapping near/far in the projection matrix.
                Mat4::perspective_rh(
                    *fov_y,
                    *aspect_ratio,
                    self.far,   // swapped: far as "near" parameter
                    self.near,  // swapped: near as "far" parameter
                )
            }
            Projection::Orthographic { half_width, half_height } => {
                // Reverse-Z orthographic: near maps to z=1, far maps to z=0.
                Mat4::orthographic_rh(
                    -*half_width,
                    *half_width,
                    -*half_height,
                    *half_height,
                    self.far,   // swapped
                    self.near,  // swapped
                )
            }
        }
    }
}
```

The reverse-Z convention (story 07) is baked into the projection matrix by swapping near and far. This distributes floating-point precision more evenly across the depth range, preventing z-fighting at large distances — critical for planetary-scale rendering.

### Combined View-Projection

```rust
impl Camera {
    pub fn view_projection_matrix(&self) -> Mat4 {
        self.projection_matrix() * self.view_matrix()
    }
}
```

This is the matrix uploaded to the GPU's camera uniform buffer (story 05).

### Default Camera

```rust
impl Default for Camera {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            projection: Projection::Perspective {
                fov_y: std::f32::consts::FRAC_PI_4, // 45 degrees
                aspect_ratio: 16.0 / 9.0,
            },
            near: 0.1,
            far: 10000.0,
        }
    }
}
```

The default camera sits at the origin, looks down -Z, has a 45-degree vertical FOV, 16:9 aspect ratio, and a 0.1 to 10,000 unit depth range. These defaults produce a sensible view for development and testing.

### Aspect Ratio Updates

```rust
impl Camera {
    pub fn set_aspect_ratio(&mut self, width: f32, height: f32) {
        if let Projection::Perspective { aspect_ratio, .. } = &mut self.projection {
            *aspect_ratio = width / height;
        }
    }
}
```

Called from the window resize handler alongside `RenderContext::resize()`.

### Origin Rebasing (Conceptual)

The camera itself does not know about `WorldPosition` or `i128` coordinates. The ECS system responsible for rendering performs origin rebasing:

```rust
// In the render system (not in this story, but illustrating the contract):
let camera_world_pos: WorldPosition = camera_entity.world_position;
for entity in renderable_entities {
    let offset: Vec3 = (entity.world_position - camera_world_pos).to_f32_vec3();
    // `offset` is a small f32 displacement from the camera — no precision loss.
    // This offset is used as the entity's position in the local f32 coordinate space.
}
```

The camera's `position` field is always `Vec3::ZERO` or a small offset in local space. The world never moves — instead, everything is expressed relative to the camera's world position.

## Outcome

A `Camera` struct that produces view and projection matrices compatible with wgpu's clip space, using `glam` for all math. Supports perspective and orthographic projection with reverse-Z. The camera operates entirely in local `f32` space, and the origin rebasing contract is clearly documented. The `view_projection_matrix()` output is directly uploadable to the camera uniform buffer from story 05.

## Demo Integration

**Demo crate:** `nebula-demo`

The triangle is now rendered with perspective projection. The camera slowly orbits around the triangle at a fixed distance, proving that view and projection matrices work. The triangle appears to rotate.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `glam` | `0.29` | Vec3, Quat, Mat4 types and operations |

The `Camera` struct lives in the `nebula_render` crate. `glam` is the only dependency — no GPU interaction in this module (matrix computation is CPU-side). Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Vec3, Quat, Mat4};
    use std::f32::consts::FRAC_PI_4;

    #[test]
    fn test_identity_camera_looks_down_neg_z() {
        let camera = Camera::default();
        let forward = camera.forward();
        // Forward should be approximately (0, 0, -1)
        assert!((forward.x).abs() < 1e-6);
        assert!((forward.y).abs() < 1e-6);
        assert!((forward.z + 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_projection_matrix_aspect_ratio() {
        let mut camera = Camera::default();
        camera.set_aspect_ratio(1920.0, 1080.0);
        if let Projection::Perspective { aspect_ratio, .. } = camera.projection {
            assert!((aspect_ratio - 16.0 / 9.0).abs() < 1e-6);
        } else {
            panic!("expected perspective projection");
        }
    }

    #[test]
    fn test_near_far_clip_values() {
        let camera = Camera {
            near: 0.5,
            far: 5000.0,
            ..Camera::default()
        };
        assert_eq!(camera.near, 0.5);
        assert_eq!(camera.far, 5000.0);
    }

    #[test]
    fn test_view_matrix_inverse_is_camera_transform() {
        let camera = Camera {
            position: Vec3::new(10.0, 20.0, 30.0),
            rotation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
            ..Camera::default()
        };
        let view = camera.view_matrix();
        let inv_view = view.inverse();

        // The inverse view matrix should reconstruct the camera's world transform.
        // The translation column (column 3) should equal the camera position.
        let reconstructed_pos = inv_view.col(3).truncate();
        assert!((reconstructed_pos - camera.position).length() < 1e-4);
    }

    #[test]
    fn test_ortho_projection_produces_correct_bounds() {
        let camera = Camera {
            projection: Projection::Orthographic {
                half_width: 10.0,
                half_height: 5.0,
            },
            near: 0.1,
            far: 100.0,
            ..Camera::default()
        };
        let proj = camera.projection_matrix();

        // A point at the right edge of the ortho volume should map to x=1 in NDC.
        let right_edge = proj * glam::Vec4::new(10.0, 0.0, -50.0, 1.0);
        let ndc_x = right_edge.x / right_edge.w;
        assert!((ndc_x - 1.0).abs() < 1e-4);

        // A point at the top edge should map to y=1 in NDC.
        let top_edge = proj * glam::Vec4::new(0.0, 5.0, -50.0, 1.0);
        let ndc_y = top_edge.y / top_edge.w;
        assert!((ndc_y - 1.0).abs() < 1e-4);
    }

    #[test]
    fn test_default_fov_is_45_degrees() {
        let camera = Camera::default();
        if let Projection::Perspective { fov_y, .. } = camera.projection {
            assert!((fov_y - FRAC_PI_4).abs() < 1e-6);
        } else {
            panic!("expected perspective projection");
        }
    }

    #[test]
    fn test_up_right_forward_orthogonal() {
        let camera = Camera::default();
        let f = camera.forward();
        let u = camera.up();
        let r = camera.right();

        // All three should be unit vectors
        assert!((f.length() - 1.0).abs() < 1e-6);
        assert!((u.length() - 1.0).abs() < 1e-6);
        assert!((r.length() - 1.0).abs() < 1e-6);

        // All three should be mutually orthogonal
        assert!(f.dot(u).abs() < 1e-6);
        assert!(f.dot(r).abs() < 1e-6);
        assert!(u.dot(r).abs() < 1e-6);
    }

    #[test]
    fn test_view_projection_combines_correctly() {
        let camera = Camera::default();
        let vp = camera.view_projection_matrix();
        let expected = camera.projection_matrix() * camera.view_matrix();
        // Each element should match
        for col in 0..4 {
            for row in 0..4 {
                assert!(
                    (vp.col(col)[row] - expected.col(col)[row]).abs() < 1e-6,
                    "mismatch at col={col}, row={row}"
                );
            }
        }
    }
}
```
