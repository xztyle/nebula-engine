//! Camera resource for tracking the active camera's world-space origin.

use bevy_ecs::prelude::*;
use nebula_math::WorldPosition;

/// Global camera resource that tracks the active camera entity and its
/// world-space origin. Used by the PostUpdate stage to compute
/// camera-relative [`LocalPos`](crate::LocalPos) from [`WorldPos`](crate::WorldPos).
#[derive(Resource, Debug, Clone)]
pub struct CameraRes {
    /// The entity that represents the active camera.
    pub entity: Entity,
    /// The camera's position in world-space (i128 millimeters).
    pub world_origin: WorldPosition,
}

impl Default for CameraRes {
    fn default() -> Self {
        Self {
            entity: Entity::PLACEHOLDER,
            world_origin: WorldPosition::default(),
        }
    }
}
