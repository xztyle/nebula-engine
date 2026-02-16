//! Physics debug visualization: wireframes, contact points, raycasts, velocities.
//!
//! All debug systems are **read-only** on [`PhysicsWorld`] — they never modify
//! simulation state, guaranteeing zero side-effects when the overlay is active.

use bevy_ecs::prelude::*;
use rapier3d::prelude::*;

use crate::PhysicsWorld;

/// Convenience alias — rapier may re-export a different glam version than the
/// workspace, so we always use the same `Vec3` that rapier's `Pose3` expects.
type Vec3 = rapier3d::math::Vec3;
/// Pose (position + rotation) type from rapier.
type Pose = rapier3d::math::Pose;

// ---------------------------------------------------------------------------
// Resources
// ---------------------------------------------------------------------------

/// Master toggle and per-feature flags for physics debug rendering.
#[derive(Resource, Clone, Debug)]
pub struct PhysicsDebugState {
    /// Master toggle for all physics debug rendering.
    pub enabled: bool,
    /// Show collider wireframes.
    pub show_colliders: bool,
    /// Show contact points and normals.
    pub show_contacts: bool,
    /// Show raycast lines and hit points.
    pub show_raycasts: bool,
    /// Show velocity vectors on dynamic bodies.
    pub show_velocities: bool,
    /// Show rigid body AABBs (broad-phase).
    pub show_aabbs: bool,
    /// Show the physics island boundary.
    pub show_island_boundary: bool,
    /// Line width for wireframe rendering.
    pub line_width: f32,
}

impl Default for PhysicsDebugState {
    fn default() -> Self {
        Self {
            enabled: false,
            show_colliders: true,
            show_contacts: true,
            show_raycasts: true,
            show_velocities: true,
            show_aabbs: false,
            show_island_boundary: true,
            line_width: 1.5,
        }
    }
}

impl PhysicsDebugState {
    /// Toggle the master `enabled` flag.
    pub fn toggle(&mut self) {
        self.enabled = !self.enabled;
    }
}

/// Color palette for physics debug visualization.
#[derive(Clone, Debug)]
pub struct PhysicsDebugColors {
    /// Static colliders (terrain, walls): green.
    pub static_collider: [f32; 4],
    /// Dynamic rigid bodies: blue.
    pub dynamic_body: [f32; 4],
    /// Kinematic bodies (player, platforms): cyan.
    pub kinematic_body: [f32; 4],
    /// Contact points: red.
    pub contact_point: [f32; 4],
    /// Contact normals: orange.
    pub contact_normal: [f32; 4],
    /// Raycasts: yellow.
    pub raycast: [f32; 4],
    /// Raycast hit points: bright red.
    pub raycast_hit: [f32; 4],
    /// Velocity vectors: magenta.
    pub velocity: [f32; 4],
    /// Island boundary: white, semi-transparent.
    pub island_boundary: [f32; 4],
}

impl Default for PhysicsDebugColors {
    fn default() -> Self {
        Self {
            static_collider: [0.0, 0.8, 0.2, 0.6],
            dynamic_body: [0.2, 0.4, 1.0, 0.6],
            kinematic_body: [0.0, 0.8, 0.8, 0.6],
            contact_point: [1.0, 0.0, 0.0, 1.0],
            contact_normal: [1.0, 0.5, 0.0, 1.0],
            raycast: [1.0, 1.0, 0.0, 0.8],
            raycast_hit: [1.0, 0.2, 0.2, 1.0],
            velocity: [1.0, 0.0, 1.0, 0.8],
            island_boundary: [1.0, 1.0, 1.0, 0.3],
        }
    }
}

/// Global default color palette.
pub const COLORS: PhysicsDebugColors = PhysicsDebugColors {
    static_collider: [0.0, 0.8, 0.2, 0.6],
    dynamic_body: [0.2, 0.4, 1.0, 0.6],
    kinematic_body: [0.0, 0.8, 0.8, 0.6],
    contact_point: [1.0, 0.0, 0.0, 1.0],
    contact_normal: [1.0, 0.5, 0.0, 1.0],
    raycast: [1.0, 1.0, 0.0, 0.8],
    raycast_hit: [1.0, 0.2, 0.2, 1.0],
    velocity: [1.0, 0.0, 1.0, 0.8],
    island_boundary: [1.0, 1.0, 1.0, 0.3],
};

/// A single debug line segment with color.
#[derive(Clone, Debug)]
pub struct DebugLine {
    /// Start point (x, y, z) in local physics space.
    pub start: [f32; 3],
    /// End point (x, y, z) in local physics space.
    pub end: [f32; 3],
    /// RGBA color.
    pub color: [f32; 4],
}

/// Buffer of debug line segments to be drawn this frame.
#[derive(Resource, Clone, Debug, Default)]
pub struct DebugLineBuffer {
    /// Line segments accumulated during the current frame.
    pub lines: Vec<DebugLine>,
}

impl DebugLineBuffer {
    /// Push a line segment from raw `Vec3` values.
    pub fn push_line(&mut self, start: Vec3, end: Vec3, color: [f32; 4]) {
        self.lines.push(DebugLine {
            start: [start.x, start.y, start.z],
            end: [end.x, end.y, end.z],
            color,
        });
    }

    /// Clear all lines (call at start of each frame).
    pub fn clear(&mut self) {
        self.lines.clear();
    }
}

/// A raycast registered for debug visualization.
#[derive(Clone, Debug)]
pub struct DebugRay {
    /// Ray origin in local physics space.
    pub origin: [f32; 3],
    /// Ray direction (unit vector).
    pub direction: [f32; 3],
    /// Maximum cast distance.
    pub max_distance: f32,
    /// Hit point, if the ray hit something.
    pub hit_point: Option<[f32; 3]>,
}

/// Buffer of raycasts to visualize this frame.
#[derive(Resource, Clone, Debug, Default)]
pub struct DebugRaycastBuffer {
    /// Rays accumulated during the current frame.
    pub rays: Vec<DebugRay>,
}

impl DebugRaycastBuffer {
    /// Register a ray for debug visualization.
    pub fn push_ray(
        &mut self,
        origin: [f32; 3],
        direction: [f32; 3],
        max_distance: f32,
        hit_point: Option<[f32; 3]>,
    ) {
        self.rays.push(DebugRay {
            origin,
            direction,
            max_distance,
            hit_point,
        });
    }

    /// Clear all rays (call at start of each frame).
    pub fn clear(&mut self) {
        self.rays.clear();
    }
}

// ---------------------------------------------------------------------------
// Wireframe helpers
// ---------------------------------------------------------------------------

/// Emit 12 edges of a cuboid wireframe.
fn emit_cuboid_wireframe(
    lines: &mut DebugLineBuffer,
    pos: &Pose,
    cuboid: &Cuboid,
    color: [f32; 4],
) {
    let he = cuboid.half_extents;
    let corners_local: [Vec3; 8] = [
        Vec3::new(-he.x, -he.y, -he.z),
        Vec3::new(he.x, -he.y, -he.z),
        Vec3::new(he.x, he.y, -he.z),
        Vec3::new(-he.x, he.y, -he.z),
        Vec3::new(-he.x, -he.y, he.z),
        Vec3::new(he.x, -he.y, he.z),
        Vec3::new(he.x, he.y, he.z),
        Vec3::new(-he.x, he.y, he.z),
    ];
    let corners: Vec<Vec3> = corners_local.iter().map(|c| *pos * *c).collect();

    const EDGES: [(usize, usize); 12] = [
        (0, 1),
        (1, 2),
        (2, 3),
        (3, 0),
        (4, 5),
        (5, 6),
        (6, 7),
        (7, 4),
        (0, 4),
        (1, 5),
        (2, 6),
        (3, 7),
    ];
    for (a, b) in EDGES {
        lines.push_line(corners[a], corners[b], color);
    }
}

/// Emit a sphere wireframe as 3 great circles (16 segments each).
fn emit_sphere_wireframe(lines: &mut DebugLineBuffer, pos: &Pose, radius: f32, color: [f32; 4]) {
    const SEGMENTS: usize = 16;
    let center = pos.translation;

    for plane in 0..3 {
        let mut prev = None;
        for i in 0..=SEGMENTS {
            let angle = (i as f32 / SEGMENTS as f32) * std::f32::consts::TAU;
            let (s, c) = angle.sin_cos();
            let local = match plane {
                0 => Vec3::new(c * radius, s * radius, 0.0),
                1 => Vec3::new(c * radius, 0.0, s * radius),
                _ => Vec3::new(0.0, c * radius, s * radius),
            };
            let world = center + local;
            if let Some(p) = prev {
                lines.push_line(p, world, color);
            }
            prev = Some(world);
        }
    }
}

/// Emit a capsule wireframe (two rings + connecting lines).
fn emit_capsule_wireframe(
    lines: &mut DebugLineBuffer,
    pos: &Pose,
    capsule: &Capsule,
    color: [f32; 4],
) {
    let half_height = capsule.half_height();
    let radius = capsule.radius;
    let center = pos.translation;

    const SEGMENTS: usize = 16;
    for offset in [-half_height, half_height] {
        let ring_center = center + Vec3::new(0.0, offset, 0.0);
        let mut prev = None;
        for i in 0..=SEGMENTS {
            let angle = (i as f32 / SEGMENTS as f32) * std::f32::consts::TAU;
            let (s, c) = angle.sin_cos();
            let world = ring_center + Vec3::new(c * radius, 0.0, s * radius);
            if let Some(p) = prev {
                lines.push_line(p, world, color);
            }
            prev = Some(world);
        }
    }

    for angle_idx in [0, 4, 8, 12] {
        let angle = (angle_idx as f32 / SEGMENTS as f32) * std::f32::consts::TAU;
        let (s, c) = angle.sin_cos();
        let bottom = center + Vec3::new(c * radius, -half_height, s * radius);
        let top = center + Vec3::new(c * radius, half_height, s * radius);
        lines.push_line(bottom, top, color);
    }
}

/// Emit an AABB wireframe (fallback for unknown/voxel shapes).
fn emit_aabb_wireframe(lines: &mut DebugLineBuffer, aabb: &Aabb, color: [f32; 4]) {
    let min = aabb.mins;
    let max = aabb.maxs;

    let corners = [
        Vec3::new(min.x, min.y, min.z),
        Vec3::new(max.x, min.y, min.z),
        Vec3::new(max.x, max.y, min.z),
        Vec3::new(min.x, max.y, min.z),
        Vec3::new(min.x, min.y, max.z),
        Vec3::new(max.x, min.y, max.z),
        Vec3::new(max.x, max.y, max.z),
        Vec3::new(min.x, max.y, max.z),
    ];

    const EDGES: [(usize, usize); 12] = [
        (0, 1),
        (1, 2),
        (2, 3),
        (3, 0),
        (4, 5),
        (5, 6),
        (6, 7),
        (7, 4),
        (0, 4),
        (1, 5),
        (2, 6),
        (3, 7),
    ];
    for (a, b) in EDGES {
        lines.push_line(corners[a], corners[b], color);
    }
}

/// Emit a small cross marker at a point.
fn emit_cross(lines: &mut DebugLineBuffer, point: Vec3, size: f32, color: [f32; 4]) {
    lines.push_line(point - Vec3::X * size, point + Vec3::X * size, color);
    lines.push_line(point - Vec3::Y * size, point + Vec3::Y * size, color);
    lines.push_line(point - Vec3::Z * size, point + Vec3::Z * size, color);
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Render collider wireframes color-coded by body type.
pub fn debug_render_colliders_system(
    debug: Res<PhysicsDebugState>,
    physics: Res<PhysicsWorld>,
    mut lines: ResMut<DebugLineBuffer>,
) {
    if !debug.enabled || !debug.show_colliders {
        return;
    }

    for (_handle, collider) in physics.collider_set.iter() {
        let color = match collider.parent() {
            Some(body_handle) => match physics.rigid_body_set.get(body_handle) {
                Some(body) if body.is_dynamic() => COLORS.dynamic_body,
                Some(body) if body.is_kinematic() => COLORS.kinematic_body,
                _ => COLORS.static_collider,
            },
            None => COLORS.static_collider,
        };

        let position = collider.position();
        let shape = collider.shape();

        match shape.shape_type() {
            ShapeType::Cuboid => {
                if let Some(cuboid) = shape.as_cuboid() {
                    emit_cuboid_wireframe(&mut lines, position, cuboid, color);
                }
            }
            ShapeType::Ball => {
                if let Some(ball) = shape.as_ball() {
                    emit_sphere_wireframe(&mut lines, position, ball.radius, color);
                }
            }
            ShapeType::Capsule => {
                if let Some(capsule) = shape.as_capsule() {
                    emit_capsule_wireframe(&mut lines, position, capsule, color);
                }
            }
            _ => {
                let aabb = collider.compute_aabb();
                emit_aabb_wireframe(&mut lines, &aabb, color);
            }
        }
    }
}

/// Render contact points and normals from the narrow phase.
pub fn debug_render_contacts_system(
    debug: Res<PhysicsDebugState>,
    physics: Res<PhysicsWorld>,
    mut lines: ResMut<DebugLineBuffer>,
) {
    if !debug.enabled || !debug.show_contacts {
        return;
    }

    for pair in physics.narrow_phase.contact_pairs() {
        for manifold in pair.manifolds.iter() {
            for contact in manifold.contacts() {
                let point = Vec3::new(contact.local_p1.x, contact.local_p1.y, contact.local_p1.z);
                let world_point = if let Some(sub_pos) = &manifold.subshape_pos1 {
                    *sub_pos * point
                } else {
                    point
                };

                emit_cross(&mut lines, world_point, 0.05, COLORS.contact_point);

                let normal = Vec3::new(
                    manifold.local_n1.x,
                    manifold.local_n1.y,
                    manifold.local_n1.z,
                );
                let normal_end = world_point + normal * 0.3;
                lines.push_line(world_point, normal_end, COLORS.contact_normal);
            }
        }
    }
}

/// Render registered debug raycasts.
pub fn debug_render_raycasts_system(
    debug: Res<PhysicsDebugState>,
    rays: Res<DebugRaycastBuffer>,
    mut lines: ResMut<DebugLineBuffer>,
) {
    if !debug.enabled || !debug.show_raycasts {
        return;
    }

    for ray in &rays.rays {
        let origin = Vec3::new(ray.origin[0], ray.origin[1], ray.origin[2]);
        let dir = Vec3::new(ray.direction[0], ray.direction[1], ray.direction[2]);
        let end = origin + dir * ray.max_distance;
        lines.push_line(origin, end, COLORS.raycast);

        if let Some(hit) = ray.hit_point {
            let hp = Vec3::new(hit[0], hit[1], hit[2]);
            emit_cross(&mut lines, hp, 0.1, COLORS.raycast_hit);
        }
    }
}

/// Render velocity vectors on dynamic bodies.
pub fn debug_render_velocities_system(
    debug: Res<PhysicsDebugState>,
    physics: Res<PhysicsWorld>,
    mut lines: ResMut<DebugLineBuffer>,
) {
    if !debug.enabled || !debug.show_velocities {
        return;
    }

    for (_handle, body) in physics.rigid_body_set.iter() {
        if !body.is_dynamic() {
            continue;
        }
        let pos = body.translation();
        let vel = body.linvel();
        let start = Vec3::new(pos.x, pos.y, pos.z);
        let end = start + Vec3::new(vel.x, vel.y, vel.z);
        lines.push_line(start, end, COLORS.velocity);
    }
}

/// Toggle physics debug visualization. Call with `true` when the toggle
/// key (F2/F3) was just pressed this frame.
pub fn physics_debug_toggle_system(just_pressed: bool, debug: &mut PhysicsDebugState) {
    if just_pressed {
        debug.toggle();
    }
}

#[cfg(test)]
#[path = "physics_debug_tests.rs"]
mod tests;
