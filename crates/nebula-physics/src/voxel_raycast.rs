//! Voxel raycasting using the DDA (Amanatides & Woo) algorithm.
//!
//! Steps through an i128 voxel grid with f32 parametric values,
//! returning the first solid voxel hit along with face normal, distance,
//! voxel type, and hit UV coordinates.

use bevy_ecs::prelude::*;
use glam::{IVec3, Vec2, Vec3};
use nebula_math::WorldPosition;
use nebula_voxel::VoxelTypeId;

/// Trait for looking up voxel data by world position.
///
/// Implementors map a [`WorldPosition`] to an optional [`VoxelData`] describing
/// the voxel at that location. Returns `None` for unloaded/out-of-range positions.
pub trait VoxelWorldAccess {
    /// Returns voxel data at the given world position, or `None` if unloaded.
    fn get_voxel(&self, pos: &WorldPosition) -> Option<VoxelData>;
}

/// Minimal voxel information returned by [`VoxelWorldAccess`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VoxelData {
    /// The voxel type identifier.
    pub id: VoxelTypeId,
    /// Whether this voxel blocks raycasts.
    pub solid: bool,
}

/// A ray cast through the voxel grid.
///
/// The origin is an i128 [`WorldPosition`] representing the voxel the ray
/// starts in. `sub_offset` is the fractional position within that voxel
/// (each component in `0.0..1.0`). Direction must be normalized.
pub struct VoxelRay {
    /// Origin voxel in world coordinates (i128).
    pub origin: WorldPosition,
    /// Sub-voxel offset within the origin voxel (0.0..1.0 per axis).
    pub sub_offset: Vec3,
    /// Normalized direction vector.
    pub direction: Vec3,
    /// Maximum ray distance in voxels (blocks).
    pub max_distance: f32,
    /// If true, skip the origin voxel even if it is solid.
    pub skip_origin: bool,
}

/// Result of a successful voxel raycast.
#[derive(Clone, Debug)]
pub struct VoxelRaycastHit {
    /// World-space coordinate of the hit voxel.
    pub voxel_pos: WorldPosition,
    /// The face normal of the entry face (which side of the voxel was hit).
    pub face_normal: IVec3,
    /// Distance from the ray origin to the hit point, in voxels.
    pub distance: f32,
    /// The type/ID of the voxel that was hit.
    pub voxel_type: VoxelTypeId,
    /// Exact hit point within the voxel face (0.0..1.0 UV coordinates).
    pub hit_uv: Vec2,
}

/// Current crosshair target: the voxel the player is aiming at.
#[derive(Resource, Default)]
pub struct BlockTarget {
    /// The current raycast hit, if any.
    pub hit: Option<VoxelRaycastHit>,
}

/// Casts a ray through the voxel grid using the DDA algorithm.
///
/// Returns the first solid voxel hit, or `None` if the ray exceeds
/// `max_distance` without hitting anything.
pub fn voxel_raycast(ray: &VoxelRay, world: &dyn VoxelWorldAccess) -> Option<VoxelRaycastHit> {
    let dir = ray.direction;

    // Current voxel position in i128 world space.
    let mut voxel = ray.origin;

    // Step direction per axis: +1 or -1.
    let step_x: i128 = if dir.x >= 0.0 { 1 } else { -1 };
    let step_y: i128 = if dir.y >= 0.0 { 1 } else { -1 };
    let step_z: i128 = if dir.z >= 0.0 { 1 } else { -1 };

    // Distance in t-units to cross one full voxel on each axis.
    let t_delta = Vec3::new(
        safe_inv(dir.x.abs()),
        safe_inv(dir.y.abs()),
        safe_inv(dir.z.abs()),
    );

    // Distance in t-units to the first voxel boundary on each axis.
    let mut t_max = Vec3::new(
        initial_t_max(ray.sub_offset.x, dir.x, t_delta.x),
        initial_t_max(ray.sub_offset.y, dir.y, t_delta.y),
        initial_t_max(ray.sub_offset.z, dir.z, t_delta.z),
    );

    let mut last_normal = IVec3::ZERO;
    let mut t = 0.0_f32;
    let mut is_origin = true;

    loop {
        // Check the current voxel.
        if let Some(data) = world.get_voxel(&voxel)
            && data.solid
            && !(is_origin && ray.skip_origin)
        {
            return Some(VoxelRaycastHit {
                voxel_pos: voxel,
                face_normal: last_normal,
                distance: t,
                voxel_type: data.id,
                hit_uv: compute_hit_uv(ray, t, &last_normal),
            });
        }
        is_origin = false;

        // Advance along the axis with the smallest t_max.
        if t_max.x < t_max.y && t_max.x < t_max.z {
            t = t_max.x;
            t_max.x += t_delta.x;
            voxel.x += step_x;
            last_normal = IVec3::new(-step_x as i32, 0, 0);
        } else if t_max.y < t_max.z {
            t = t_max.y;
            t_max.y += t_delta.y;
            voxel.y += step_y;
            last_normal = IVec3::new(0, -step_y as i32, 0);
        } else {
            t = t_max.z;
            t_max.z += t_delta.z;
            voxel.z += step_z;
            last_normal = IVec3::new(0, 0, -step_z as i32);
        }

        if t > ray.max_distance {
            return None;
        }
    }
}

/// Safely compute 1.0 / x, clamping to `f32::MAX` when x ≈ 0.
fn safe_inv(x: f32) -> f32 {
    if x.abs() < f32::EPSILON {
        f32::MAX
    } else {
        1.0 / x
    }
}

/// Compute the initial parametric distance to the first voxel boundary.
fn initial_t_max(sub: f32, dir_component: f32, t_delta: f32) -> f32 {
    if dir_component > 0.0 {
        (1.0 - sub) * t_delta
    } else if dir_component < 0.0 {
        sub * t_delta
    } else {
        f32::MAX
    }
}

/// Compute UV coordinates of the hit point on the entry face.
fn compute_hit_uv(ray: &VoxelRay, t: f32, normal: &IVec3) -> Vec2 {
    // Hit point in local space relative to origin voxel.
    let hit_point = ray.sub_offset + ray.direction * t;

    // Project onto the face plane based on the normal axis.
    if normal.x != 0 {
        Vec2::new(hit_point.z.fract().abs(), hit_point.y.fract().abs())
    } else if normal.y != 0 {
        Vec2::new(hit_point.x.fract().abs(), hit_point.z.fract().abs())
    } else {
        Vec2::new(hit_point.x.fract().abs(), hit_point.y.fract().abs())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Simple test world backed by a hash map.
    struct TestWorld {
        voxels: HashMap<(i128, i128, i128), VoxelData>,
    }

    impl TestWorld {
        fn new() -> Self {
            Self {
                voxels: HashMap::new(),
            }
        }

        fn set_solid(&mut self, x: i128, y: i128, z: i128, id: u16) {
            self.voxels.insert(
                (x, y, z),
                VoxelData {
                    id: VoxelTypeId(id),
                    solid: true,
                },
            );
        }
    }

    impl VoxelWorldAccess for TestWorld {
        fn get_voxel(&self, pos: &WorldPosition) -> Option<VoxelData> {
            self.voxels.get(&(pos.x, pos.y, pos.z)).copied()
        }
    }

    fn ray_along(dx: f32, dy: f32, dz: f32, max_dist: f32) -> VoxelRay {
        let dir = Vec3::new(dx, dy, dz).normalize();
        VoxelRay {
            origin: WorldPosition::new(0, 0, 0),
            sub_offset: Vec3::new(0.5, 0.5, 0.5),
            direction: dir,
            max_distance: max_dist,
            skip_origin: false,
        }
    }

    #[test]
    fn test_ray_hits_solid_voxel() {
        let mut world = TestWorld::new();
        world.set_solid(5, 0, 0, 1);

        let ray = VoxelRay {
            origin: WorldPosition::new(0, 0, 0),
            sub_offset: Vec3::new(0.5, 0.5, 0.5),
            direction: Vec3::X,
            max_distance: 10.0,
            skip_origin: false,
        };

        let hit = voxel_raycast(&ray, &world).expect("should hit");
        assert_eq!(hit.voxel_pos, WorldPosition::new(5, 0, 0));
        assert!((hit.distance - 4.5).abs() < 0.1); // 5 - 0.5 sub_offset
    }

    #[test]
    fn test_ray_misses_empty_space() {
        let world = TestWorld::new();
        let ray = ray_along(1.0, 0.0, 0.0, 100.0);
        assert!(voxel_raycast(&ray, &world).is_none());
    }

    #[test]
    fn test_hit_face_normal_correct() {
        let mut world = TestWorld::new();
        world.set_solid(5, 0, 0, 1);

        // Ray from -X side
        let ray = VoxelRay {
            origin: WorldPosition::new(0, 0, 0),
            sub_offset: Vec3::new(0.5, 0.5, 0.5),
            direction: Vec3::X,
            max_distance: 10.0,
            skip_origin: false,
        };
        let hit = voxel_raycast(&ray, &world).unwrap();
        assert_eq!(hit.face_normal, IVec3::new(-1, 0, 0));

        // Ray from +X side
        let mut world2 = TestWorld::new();
        world2.set_solid(0, 0, 0, 1);
        let ray_neg = VoxelRay {
            origin: WorldPosition::new(5, 0, 0),
            sub_offset: Vec3::new(0.5, 0.5, 0.5),
            direction: Vec3::NEG_X,
            max_distance: 10.0,
            skip_origin: false,
        };
        let hit_neg = voxel_raycast(&ray_neg, &world2).unwrap();
        assert_eq!(hit_neg.face_normal, IVec3::new(1, 0, 0));

        // Ray from -Y side
        let mut world3 = TestWorld::new();
        world3.set_solid(0, 5, 0, 1);
        let ray_y = VoxelRay {
            origin: WorldPosition::new(0, 0, 0),
            sub_offset: Vec3::new(0.5, 0.5, 0.5),
            direction: Vec3::Y,
            max_distance: 10.0,
            skip_origin: false,
        };
        let hit_y = voxel_raycast(&ray_y, &world3).unwrap();
        assert_eq!(hit_y.face_normal, IVec3::new(0, -1, 0));

        // Ray from +Y side
        let ray_neg_y = VoxelRay {
            origin: WorldPosition::new(0, 10, 0),
            sub_offset: Vec3::new(0.5, 0.5, 0.5),
            direction: Vec3::NEG_Y,
            max_distance: 15.0,
            skip_origin: false,
        };
        let hit_neg_y = voxel_raycast(&ray_neg_y, &world3).unwrap();
        assert_eq!(hit_neg_y.face_normal, IVec3::new(0, 1, 0));

        // Ray from -Z side
        let mut world4 = TestWorld::new();
        world4.set_solid(0, 0, 5, 1);
        let ray_z = VoxelRay {
            origin: WorldPosition::new(0, 0, 0),
            sub_offset: Vec3::new(0.5, 0.5, 0.5),
            direction: Vec3::Z,
            max_distance: 10.0,
            skip_origin: false,
        };
        let hit_z = voxel_raycast(&ray_z, &world4).unwrap();
        assert_eq!(hit_z.face_normal, IVec3::new(0, 0, -1));

        // Ray from +Z side
        let ray_neg_z = VoxelRay {
            origin: WorldPosition::new(0, 0, 10),
            sub_offset: Vec3::new(0.5, 0.5, 0.5),
            direction: Vec3::NEG_Z,
            max_distance: 15.0,
            skip_origin: false,
        };
        let hit_neg_z = voxel_raycast(&ray_neg_z, &world4).unwrap();
        assert_eq!(hit_neg_z.face_normal, IVec3::new(0, 0, 1));
    }

    #[test]
    fn test_max_distance_limits_search() {
        let mut world = TestWorld::new();
        world.set_solid(20, 0, 0, 1);

        let ray_short = VoxelRay {
            origin: WorldPosition::new(0, 0, 0),
            sub_offset: Vec3::new(0.5, 0.5, 0.5),
            direction: Vec3::X,
            max_distance: 10.0,
            skip_origin: false,
        };
        assert!(voxel_raycast(&ray_short, &world).is_none());

        let ray_long = VoxelRay {
            origin: WorldPosition::new(0, 0, 0),
            sub_offset: Vec3::new(0.5, 0.5, 0.5),
            direction: Vec3::X,
            max_distance: 25.0,
            skip_origin: false,
        };
        assert!(voxel_raycast(&ray_long, &world).is_some());
    }

    #[test]
    fn test_ray_from_inside_solid_escapes() {
        let mut world = TestWorld::new();
        // 3x3x3 solid cube centered at (1,1,1): positions 0..=2 on each axis.
        for x in 0..=2_i128 {
            for y in 0..=2_i128 {
                for z in 0..=2_i128 {
                    world.set_solid(x, y, z, 1);
                }
            }
        }
        // Place another solid voxel outside the cube to catch.
        world.set_solid(5, 1, 1, 2);

        let ray = VoxelRay {
            origin: WorldPosition::new(1, 1, 1),
            sub_offset: Vec3::new(0.5, 0.5, 0.5),
            direction: Vec3::X,
            max_distance: 20.0,
            skip_origin: true,
        };

        let hit = voxel_raycast(&ray, &world).expect("should hit after escaping");
        // The first solid voxel after origin (1,1,1) in +X is (2,1,1) which is
        // still in the cube. skip_origin only skips the origin voxel itself.
        assert_eq!(hit.voxel_pos, WorldPosition::new(2, 1, 1));
    }

    #[test]
    fn test_diagonal_ray_crosses_voxels_correctly() {
        let mut world = TestWorld::new();
        world.set_solid(3, 3, 0, 1);

        let dir = Vec3::new(1.0, 1.0, 0.0).normalize();
        let ray = VoxelRay {
            origin: WorldPosition::new(0, 0, 0),
            sub_offset: Vec3::new(0.5, 0.5, 0.5),
            direction: dir,
            max_distance: 20.0,
            skip_origin: false,
        };

        let hit = voxel_raycast(&ray, &world).expect("should hit diagonal voxel");
        assert_eq!(hit.voxel_pos, WorldPosition::new(3, 3, 0));
        // Distance should be approximately 3 * sqrt(2) - some offset for sub_offset
        let expected = (3.0_f32 - 0.5) * 2.0_f32.sqrt();
        assert!(
            (hit.distance - expected).abs() < 0.5,
            "distance {} expected ~{}",
            hit.distance,
            expected,
        );
    }

    #[test]
    fn test_ray_returns_correct_voxel_type() {
        let mut world = TestWorld::new();
        world.set_solid(5, 0, 0, 10); // stone
        world.set_solid(10, 0, 0, 20); // dirt

        // First ray hits stone
        let ray1 = VoxelRay {
            origin: WorldPosition::new(0, 0, 0),
            sub_offset: Vec3::new(0.5, 0.5, 0.5),
            direction: Vec3::X,
            max_distance: 20.0,
            skip_origin: false,
        };
        let hit1 = voxel_raycast(&ray1, &world).unwrap();
        assert_eq!(hit1.voxel_type, VoxelTypeId(10));

        // Second ray starts past stone, hits dirt
        let ray2 = VoxelRay {
            origin: WorldPosition::new(6, 0, 0),
            sub_offset: Vec3::new(0.5, 0.5, 0.5),
            direction: Vec3::X,
            max_distance: 20.0,
            skip_origin: false,
        };
        let hit2 = voxel_raycast(&ray2, &world).unwrap();
        assert_eq!(hit2.voxel_type, VoxelTypeId(20));
    }
}
