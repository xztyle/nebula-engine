# Basic Frustum Culling

## Problem

Without frustum culling, the engine submits draw calls for every loaded chunk and object, regardless of whether they are visible to the camera. In a planetary engine with potentially thousands of loaded chunks, most of which are behind the camera or far to the side, this wastes enormous GPU time on geometry that produces zero visible pixels. The GPU still processes vertices through the vertex shader and clips them against the view frustum — but the draw call overhead, vertex fetch, and pipeline state changes are wasted CPU and GPU work.

This story implements local-space frustum culling — testing axis-aligned bounding boxes (AABBs) in `f32` local space against the camera's view frustum. This complements the coarse `i128` coordinate culling from Epic 03 (which rejects objects that are too far away to even convert to `f32`). Together, the two culling stages form a pipeline: i128 coarse cull first (cheap, eliminates objects light-years away), then f32 frustum cull second (precise, eliminates objects outside the camera's field of view).

## Solution

### Frustum Representation

A frustum is defined by six planes: left, right, top, bottom, near, and far. Each plane is represented as a `Vec4` where `(a, b, c)` is the plane normal (pointing inward) and `d` is the signed distance from the origin:

```rust
pub struct Frustum {
    planes: [Vec4; 6],
}

/// Indices into the planes array.
const LEFT: usize = 0;
const RIGHT: usize = 1;
const BOTTOM: usize = 2;
const TOP: usize = 3;
const NEAR: usize = 4;
const FAR: usize = 5;
```

### Extracting Planes from View-Projection Matrix

The Griggs-Hartmann method extracts frustum planes directly from the combined view-projection matrix. This avoids computing planes from camera parameters manually and automatically handles both perspective and orthographic projections:

```rust
impl Frustum {
    pub fn from_view_projection(vp: &Mat4) -> Self {
        let rows = [
            vp.row(0),
            vp.row(1),
            vp.row(2),
            vp.row(3),
        ];

        let mut planes = [Vec4::ZERO; 6];
        planes[LEFT]   = rows[3] + rows[0]; // row3 + row0
        planes[RIGHT]  = rows[3] - rows[0]; // row3 - row0
        planes[BOTTOM] = rows[3] + rows[1]; // row3 + row1
        planes[TOP]    = rows[3] - rows[1]; // row3 - row1
        planes[NEAR]   = rows[3] + rows[2]; // row3 + row2 (adjusted for reverse-Z)
        planes[FAR]    = rows[3] - rows[2]; // row3 - row2

        // Normalize each plane so that (a,b,c) is a unit vector.
        for plane in &mut planes {
            let len = plane.truncate().length();
            if len > 0.0 {
                *plane /= len;
            }
        }

        Self { planes }
    }
}
```

Note: With reverse-Z (story 07), the near/far plane extraction may need sign adjustments. The exact signs depend on whether the projection maps z to [1, 0] (reverse-Z) vs [0, 1] (standard). The implementation must be validated against the actual projection matrices from story 06.

### AABB

```rust
#[derive(Clone, Copy, Debug)]
pub struct AABB {
    pub min: Vec3,
    pub max: Vec3,
}

impl AABB {
    pub fn new(min: Vec3, max: Vec3) -> Self {
        Self { min, max }
    }

    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    pub fn extents(&self) -> Vec3 {
        (self.max - self.min) * 0.5
    }
}
```

### Frustum-AABB Test

The standard approach tests each plane against the AABB's "positive vertex" (the corner most in the direction of the plane normal). If the positive vertex is behind any plane, the AABB is fully outside the frustum:

```rust
impl Frustum {
    pub fn is_visible(&self, aabb: &AABB) -> bool {
        for plane in &self.planes {
            let normal = plane.truncate();
            let d = plane.w;

            // Find the positive vertex: the corner furthest along the plane normal.
            let p = Vec3::new(
                if normal.x >= 0.0 { aabb.max.x } else { aabb.min.x },
                if normal.y >= 0.0 { aabb.max.y } else { aabb.min.y },
                if normal.z >= 0.0 { aabb.max.z } else { aabb.min.z },
            );

            // If the positive vertex is behind the plane, the AABB is fully outside.
            if normal.dot(p) + d < 0.0 {
                return false;
            }
        }
        true
    }
}
```

This test is conservative: it returns `true` for objects that are partially inside the frustum (which is correct — they should be drawn) and may return `true` for some objects that are fully outside (false positives near frustum corners). False positives are acceptable because they only waste a few draw calls, while false negatives would cause visible popping.

### FrustumCuller

A convenience struct for per-frame culling:

```rust
pub struct FrustumCuller {
    frustum: Frustum,
}

impl FrustumCuller {
    pub fn new(view_projection: &Mat4) -> Self {
        Self {
            frustum: Frustum::from_view_projection(view_projection),
        }
    }

    pub fn is_visible(&self, aabb: &AABB) -> bool {
        self.frustum.is_visible(aabb)
    }
}
```

### Usage Pattern

Each frame, the render system constructs a `FrustumCuller` from the camera's view-projection matrix and tests each chunk/object before issuing its draw call:

```rust
let culler = FrustumCuller::new(&camera.view_projection_matrix());
for chunk in loaded_chunks {
    if culler.is_visible(&chunk.bounding_box) {
        draw_chunk(render_pass, chunk);
    }
}
```

## Outcome

A `FrustumCuller` that extracts six frustum planes from the view-projection matrix and tests AABBs against them. Objects fully outside the frustum are skipped, eliminating their draw calls entirely. The culler handles all six planes (left, right, top, bottom, near, far) and correctly identifies partially-visible objects as visible. This is the second stage of the culling pipeline after i128 coarse culling.

## Demo Integration

**Demo crate:** `nebula-demo`

The demo spawns 100 small cubes scattered in a sphere around the camera. Cubes outside the frustum are not submitted for rendering. The console logs `Culled: 58/100 objects`.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `glam` | `0.29` | Vec3, Vec4, Mat4 for plane extraction and AABB math |

No GPU dependencies — frustum culling is entirely CPU-side. Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Vec3, Mat4};

    fn default_camera_vp() -> Mat4 {
        // Default camera at origin, looking down -Z, 45° FOV, 16:9, near=0.1, far=1000.
        let view = Mat4::look_to_rh(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y);
        let proj = Mat4::perspective_rh(
            std::f32::consts::FRAC_PI_4,
            16.0 / 9.0,
            1000.0, // reverse-Z: far as near param
            0.1,    // reverse-Z: near as far param
        );
        proj * view
    }

    #[test]
    fn test_object_at_origin_visible() {
        let culler = FrustumCuller::new(&default_camera_vp());
        let aabb = AABB::new(Vec3::new(-1.0, -1.0, -5.0), Vec3::new(1.0, 1.0, -3.0));
        // Object is directly in front of the camera, 3–5 units away on -Z
        assert!(culler.is_visible(&aabb));
    }

    #[test]
    fn test_object_behind_camera_not_visible() {
        let culler = FrustumCuller::new(&default_camera_vp());
        // Object is behind the camera (positive Z)
        let aabb = AABB::new(Vec3::new(-1.0, -1.0, 5.0), Vec3::new(1.0, 1.0, 10.0));
        assert!(!culler.is_visible(&aabb));
    }

    #[test]
    fn test_object_far_to_the_side_not_visible() {
        let culler = FrustumCuller::new(&default_camera_vp());
        // Object is 1000 units to the right and only 5 units forward — well outside FOV
        let aabb = AABB::new(
            Vec3::new(1000.0, -1.0, -6.0),
            Vec3::new(1002.0, 1.0, -4.0),
        );
        assert!(!culler.is_visible(&aabb));
    }

    #[test]
    fn test_object_partially_in_frustum_is_visible() {
        let culler = FrustumCuller::new(&default_camera_vp());
        // Large object that straddles the left edge of the frustum
        let aabb = AABB::new(Vec3::new(-100.0, -1.0, -10.0), Vec3::new(1.0, 1.0, -5.0));
        assert!(culler.is_visible(&aabb));
    }

    #[test]
    fn test_all_six_planes_tested() {
        let culler = FrustumCuller::new(&default_camera_vp());

        // Behind camera (fails near/far plane depending on reverse-Z)
        let behind = AABB::new(Vec3::splat(10.0), Vec3::splat(20.0));
        assert!(!culler.is_visible(&behind));

        // Far left
        let left = AABB::new(Vec3::new(-1000.0, 0.0, -5.0), Vec3::new(-999.0, 1.0, -4.0));
        assert!(!culler.is_visible(&left));

        // Far right
        let right = AABB::new(Vec3::new(999.0, 0.0, -5.0), Vec3::new(1000.0, 1.0, -4.0));
        assert!(!culler.is_visible(&right));

        // Far above
        let above = AABB::new(Vec3::new(0.0, 999.0, -5.0), Vec3::new(1.0, 1000.0, -4.0));
        assert!(!culler.is_visible(&above));

        // Far below
        let below = AABB::new(Vec3::new(0.0, -1000.0, -5.0), Vec3::new(1.0, -999.0, -4.0));
        assert!(!culler.is_visible(&below));

        // Beyond far plane
        let beyond_far = AABB::new(Vec3::new(0.0, 0.0, -2000.0), Vec3::new(1.0, 1.0, -1500.0));
        assert!(!culler.is_visible(&beyond_far));
    }

    #[test]
    fn test_aabb_center_and_extents() {
        let aabb = AABB::new(Vec3::new(-2.0, -3.0, -4.0), Vec3::new(2.0, 3.0, 4.0));
        assert_eq!(aabb.center(), Vec3::ZERO);
        assert_eq!(aabb.extents(), Vec3::new(2.0, 3.0, 4.0));
    }

    #[test]
    fn test_frustum_has_six_planes() {
        let frustum = Frustum::from_view_projection(&default_camera_vp());
        assert_eq!(frustum.planes.len(), 6);
        // All plane normals should be unit length (after normalization)
        for plane in &frustum.planes {
            let normal_len = plane.truncate().length();
            assert!((normal_len - 1.0).abs() < 1e-4, "plane normal not normalized: {}", normal_len);
        }
    }
}
```
