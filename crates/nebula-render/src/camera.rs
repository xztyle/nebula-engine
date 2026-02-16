//! Camera system for view and projection matrix generation.

use crate::pipeline::CameraUniform;
use glam::{Mat4, Quat, Vec3};

/// A camera that generates view and projection matrices for rendering.
/// Operates entirely in local f32 space after origin rebasing.
#[derive(Debug, Clone)]
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

/// Projection type for the camera.
#[derive(Debug, Clone)]
pub enum Projection {
    /// Perspective projection for 3D scenes.
    Perspective {
        /// Vertical field of view in radians.
        fov_y: f32,
        /// Width / height.
        aspect_ratio: f32,
    },
    /// Orthographic projection for UI rendering and 2D views.
    Orthographic {
        /// Half-width of the view volume in world units.
        half_width: f32,
        /// Half-height of the view volume in world units.
        half_height: f32,
    },
}

impl Camera {
    /// Compute the view matrix (inverse of camera transform).
    pub fn view_matrix(&self) -> Mat4 {
        let rotation_matrix = Mat4::from_quat(self.rotation);
        let translation_matrix = Mat4::from_translation(self.position);
        // View = inverse(Translation * Rotation) = inverse(Rotation) * inverse(Translation)
        (translation_matrix * rotation_matrix).inverse()
        // Equivalent and faster:
        // Mat4::look_to_rh(self.position, self.forward(), self.up())
    }

    /// Compute the projection matrix with reverse-Z.
    pub fn projection_matrix(&self) -> Mat4 {
        match &self.projection {
            Projection::Perspective {
                fov_y,
                aspect_ratio,
            } => {
                // Reverse-Z: near plane maps to z=1, far plane maps to z=0.
                // This is handled by swapping near/far in the projection matrix.
                Mat4::perspective_rh(
                    *fov_y,
                    *aspect_ratio,
                    self.far,  // swapped: far as "near" parameter
                    self.near, // swapped: near as "far" parameter
                )
            }
            Projection::Orthographic {
                half_width,
                half_height,
            } => {
                // Reverse-Z orthographic: near maps to z=1, far maps to z=0.
                Mat4::orthographic_rh(
                    -*half_width,
                    *half_width,
                    -*half_height,
                    *half_height,
                    self.far,  // swapped
                    self.near, // swapped
                )
            }
        }
    }

    /// Compute the combined view-projection matrix.
    pub fn view_projection_matrix(&self) -> Mat4 {
        self.projection_matrix() * self.view_matrix()
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

    /// Update the aspect ratio for perspective projection.
    pub fn set_aspect_ratio(&mut self, width: f32, height: f32) {
        if let Projection::Perspective { aspect_ratio, .. } = &mut self.projection {
            *aspect_ratio = width / height;
        }
    }

    /// Convert the camera to a uniform suitable for GPU upload.
    pub fn to_uniform(&self) -> CameraUniform {
        CameraUniform {
            view_proj: self.view_projection_matrix().to_cols_array_2d(),
            camera_pos: [self.position.x, self.position.y, self.position.z, 0.0],
        }
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Quat, Vec3};
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
