//! Coordinate origin rebasing for f32 precision maintenance.
//!
//! As the camera moves far from the coordinate origin, f32 precision degrades.
//! The [`OriginManager`] monitors camera distance from the current origin and
//! triggers a rebase when it exceeds a threshold, keeping all GPU-side f32
//! positions within a precision-safe range.

use nebula_math::WorldPosition;

/// Manages coordinate origin rebasing for f32 precision.
///
/// When the camera moves more than [`rebase_threshold`](Self::rebase_threshold) millimeters
/// from the current origin on any axis, the origin is shifted to the camera
/// position and a delta is returned so all local-space positions can be adjusted.
pub struct OriginManager {
    /// Current coordinate origin in world space.
    pub origin: WorldPosition,
    /// Distance threshold (mm) before rebasing. Default: 10 km = 10,000,000,000 mm.
    pub rebase_threshold: i128,
}

impl OriginManager {
    /// Create a new origin manager with default threshold (10 km).
    pub fn new() -> Self {
        Self {
            origin: WorldPosition::new(0, 0, 0),
            rebase_threshold: 10_000_000_000, // 10 km in mm
        }
    }

    /// Check if the origin should be rebased based on camera position.
    ///
    /// If any axis delta exceeds the threshold, the origin is moved to the
    /// camera's position. Returns `Some(delta)` with the shift that was applied
    /// (new_origin − old_origin), or `None` if no rebase was needed.
    pub fn update(&mut self, camera_world: &WorldPosition) -> Option<WorldPosition> {
        let dx = (camera_world.x - self.origin.x).abs();
        let dy = (camera_world.y - self.origin.y).abs();
        let dz = (camera_world.z - self.origin.z).abs();

        if dx > self.rebase_threshold || dy > self.rebase_threshold || dz > self.rebase_threshold {
            let old_origin = self.origin;
            self.origin = *camera_world;
            Some(WorldPosition::new(
                camera_world.x - old_origin.x,
                camera_world.y - old_origin.y,
                camera_world.z - old_origin.z,
            ))
        } else {
            None
        }
    }

    /// Compute the camera position relative to the current origin as f32 meters.
    ///
    /// This is the value that should be used for all GPU-side camera positions.
    pub fn local_camera_pos(&self, camera_world: &WorldPosition) -> glam::Vec3 {
        camera_world.to_local_f32(&self.origin)
    }
}

impl Default for OriginManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_small_movement_no_rebase() {
        let mut origin = OriginManager::new();
        let camera_near = WorldPosition::new(1_000_000, 1_000_000, 1_000_000);
        assert!(
            origin.update(&camera_near).is_none(),
            "Small movement should not trigger rebase"
        );
    }

    #[test]
    fn test_large_movement_triggers_rebase() {
        let mut origin = OriginManager::new();
        let camera_far = WorldPosition::new(100_000_000_000, 0, 0);
        let delta = origin.update(&camera_far);
        assert!(delta.is_some(), "Large movement should trigger rebase");
        assert_eq!(
            origin.origin, camera_far,
            "Origin should be updated to camera position"
        );
    }

    #[test]
    fn test_rebase_returns_correct_delta() {
        let mut origin = OriginManager::new();
        let camera_far = WorldPosition::new(100_000_000_000, 50_000_000_000, 0);
        let delta = origin.update(&camera_far).unwrap();
        assert_eq!(delta.x, 100_000_000_000);
        assert_eq!(delta.y, 50_000_000_000);
        assert_eq!(delta.z, 0);
    }

    #[test]
    fn test_successive_rebases() {
        let mut origin = OriginManager::new();

        // First large move
        let pos1 = WorldPosition::new(100_000_000_000, 0, 0);
        origin.update(&pos1);
        assert_eq!(origin.origin, pos1);

        // Small move from new origin — no rebase
        let pos2 = WorldPosition::new(100_001_000_000, 0, 0);
        assert!(origin.update(&pos2).is_none());

        // Large move from current origin
        let pos3 = WorldPosition::new(200_000_000_000, 0, 0);
        let delta = origin.update(&pos3).unwrap();
        assert_eq!(delta.x, pos3.x - pos1.x);
    }

    #[test]
    fn test_local_camera_pos() {
        let origin = OriginManager::new();
        let camera = WorldPosition::new(5000, 10000, 15000); // 5000mm, 10000mm, 15000mm
        let local = origin.local_camera_pos(&camera);
        // to_local_f32 returns millimeters as f32
        assert!((local.x - 5000.0).abs() < 0.01);
        assert!((local.y - 10000.0).abs() < 0.01);
        assert!((local.z - 15000.0).abs() < 0.01);
    }
}
