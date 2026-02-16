//! Physics region management: spatial volumes with distinct physics rules.
//!
//! Entities are assigned to the highest-priority region containing them.
//! Transitions between regions blend physics parameters over 0.5 seconds.

use bevy_ecs::prelude::*;
use glam::Vec3;
use nebula_math::WorldPosition;

use crate::physics_island::IslandWorldPos;

/// Configuration for gravity within a region.
#[derive(Clone, Debug)]
pub struct GravityConfig {
    /// Normalized gravity direction.
    pub direction: Vec3,
    /// Gravity magnitude in m/s².
    pub magnitude: f32,
}

/// Type of physics environment a region represents.
#[derive(Clone, Debug)]
pub enum PhysicsRegionType {
    /// Planet surface: gravity toward center, air resistance, normal friction.
    PlanetSurface {
        /// The planet entity this surface belongs to.
        planet_entity: Entity,
    },
    /// Open space: no gravity, no air resistance, no friction.
    Space,
    /// Vehicle interior: gravity relative to the vehicle's orientation.
    VehicleInterior {
        /// The vehicle entity whose interior this region represents.
        vehicle_entity: Entity,
    },
    /// Space station with artificial gravity (centripetal or generated).
    ArtificialGravity {
        /// Direction of artificial gravity.
        gravity_direction: Vec3,
        /// Magnitude of artificial gravity in m/s².
        gravity_magnitude: f32,
    },
    /// Underwater: high damping, reduced gravity.
    Underwater {
        /// Density of the fluid in kg/m³.
        fluid_density: f32,
    },
    /// Custom region with fully specified parameters.
    Custom,
}

/// Spatial bounds of a physics region.
#[derive(Clone, Debug)]
pub enum RegionBounds {
    /// Sphere defined by center and radius (in millimeters).
    Sphere {
        /// Center of the sphere in world coordinates.
        center: WorldPosition,
        /// Radius in millimeters.
        radius: f64,
    },
    /// Axis-aligned bounding box.
    Aabb {
        /// Minimum corner.
        min: WorldPosition,
        /// Maximum corner.
        max: WorldPosition,
    },
    /// Everything outside a sphere (e.g., space beyond atmosphere).
    OutsideSphere {
        /// Center of the exclusion sphere.
        center: WorldPosition,
        /// Radius in millimeters.
        radius: f64,
    },
    /// Attached to an entity's collider volume.
    EntityVolume {
        /// The entity whose volume defines the bounds.
        entity: Entity,
    },
    /// Infinite region (fallback/default).
    Infinite,
}

impl RegionBounds {
    /// Test whether a world position is inside this region's bounds.
    pub fn contains(&self, pos: &WorldPosition) -> bool {
        match self {
            RegionBounds::Sphere { center, radius } => {
                let dx = (pos.x - center.x) as f64;
                let dy = (pos.y - center.y) as f64;
                let dz = (pos.z - center.z) as f64;
                let dist_sq = dx * dx + dy * dy + dz * dz;
                dist_sq <= radius * radius
            }
            RegionBounds::Aabb { min, max } => {
                pos.x >= min.x
                    && pos.x <= max.x
                    && pos.y >= min.y
                    && pos.y <= max.y
                    && pos.z >= min.z
                    && pos.z <= max.z
            }
            RegionBounds::OutsideSphere { center, radius } => {
                let dx = (pos.x - center.x) as f64;
                let dy = (pos.y - center.y) as f64;
                let dz = (pos.z - center.z) as f64;
                let dist_sq = dx * dx + dy * dy + dz * dz;
                dist_sq > radius * radius
            }
            RegionBounds::EntityVolume { .. } => {
                // Entity volume containment requires collider queries;
                // handled externally. Default to false for static checks.
                false
            }
            RegionBounds::Infinite => true,
        }
    }
}

/// A physics region defining the physics rules for a spatial volume.
#[derive(Component)]
pub struct PhysicsRegion {
    /// Type of physics environment.
    pub region_type: PhysicsRegionType,
    /// Gravity override. If `Some`, replaces computed gravity from sources.
    pub gravity_override: Option<GravityConfig>,
    /// Linear damping applied to entities in this region (air resistance).
    pub linear_damping: f32,
    /// Angular damping applied to entities in this region.
    pub angular_damping: f32,
    /// Friction multiplier applied to all contacts within this region.
    pub friction_multiplier: f32,
    /// Spatial bounds of this region in world coordinates.
    pub bounds: RegionBounds,
    /// Priority. Higher priority regions override lower ones when overlapping.
    pub priority: i32,
}

/// Tracks which physics region an entity currently occupies.
#[derive(Component)]
pub struct CurrentPhysicsRegion {
    /// Entity of the current region.
    pub region_entity: Option<Entity>,
    /// Type of the current region.
    pub region_type: PhysicsRegionType,
    /// Blend factor for transitions (0.0 = fully previous, 1.0 = fully current).
    pub transition_blend: f32,
    /// Previous region entity for blending during transitions.
    pub previous_region: Option<Entity>,
}

impl Default for CurrentPhysicsRegion {
    fn default() -> Self {
        Self {
            region_entity: None,
            region_type: PhysicsRegionType::Space,
            transition_blend: 1.0,
            previous_region: None,
        }
    }
}

/// Detects which region each entity occupies (highest priority wins).
pub fn region_detection_system(
    regions: Query<(Entity, &PhysicsRegion)>,
    mut entities: Query<(&IslandWorldPos, &mut CurrentPhysicsRegion)>,
) {
    for (island_pos, mut current) in entities.iter_mut() {
        let mut best_region: Option<(Entity, &PhysicsRegion)> = None;
        let mut best_priority = i32::MIN;

        for (region_entity, region) in regions.iter() {
            if region.bounds.contains(&island_pos.0) && region.priority > best_priority {
                best_region = Some((region_entity, region));
                best_priority = region.priority;
            }
        }

        if let Some((region_entity, region)) = best_region
            && current.region_entity != Some(region_entity)
        {
            current.previous_region = current.region_entity;
            current.region_entity = Some(region_entity);
            current.region_type = region.region_type.clone();
            current.transition_blend = 0.0;
        }
    }
}

/// Transition speed: 1.0 / 0.5 seconds = 2.0.
pub const TRANSITION_SPEED: f32 = 2.0;

/// Interpolates gravity between previous and current region based on blend.
pub fn apply_region_gravity(
    current: &CurrentPhysicsRegion,
    regions: &Query<&PhysicsRegion>,
    computed_gravity: &crate::GravityResult,
) -> crate::GravityResult {
    if let Some(region_entity) = current.region_entity
        && let Ok(region) = regions.get(region_entity)
        && let Some(ref override_gravity) = region.gravity_override
    {
        let blend = current.transition_blend;

        // Get previous region gravity if transitioning
        let (base_dir, base_mag) = if blend < 1.0 {
            if let Some(prev_entity) = current.previous_region
                && let Ok(prev_region) = regions.get(prev_entity)
                && let Some(ref prev_grav) = prev_region.gravity_override
            {
                (prev_grav.direction, prev_grav.magnitude)
            } else {
                (computed_gravity.direction, computed_gravity.magnitude)
            }
        } else {
            return crate::GravityResult {
                direction: override_gravity.direction,
                magnitude: override_gravity.magnitude,
            };
        };

        return crate::GravityResult {
            direction: base_dir.lerp(override_gravity.direction, blend),
            magnitude: base_mag * (1.0 - blend) + override_gravity.magnitude * blend,
        };
    }
    crate::GravityResult {
        direction: computed_gravity.direction,
        magnitude: computed_gravity.magnitude,
    }
}

/// Creates the default space region (infinite, priority 0).
pub fn create_default_space_region() -> PhysicsRegion {
    PhysicsRegion {
        region_type: PhysicsRegionType::Space,
        gravity_override: Some(GravityConfig {
            direction: Vec3::ZERO,
            magnitude: 0.0,
        }),
        linear_damping: 0.0,
        angular_damping: 0.0,
        friction_multiplier: 0.0,
        bounds: RegionBounds::Infinite,
        priority: 0,
    }
}

/// Creates a planet surface region with the given parameters.
pub fn create_planet_surface_region(
    planet_entity: Entity,
    center: WorldPosition,
    radius: f64,
    surface_gravity: f32,
) -> PhysicsRegion {
    PhysicsRegion {
        region_type: PhysicsRegionType::PlanetSurface { planet_entity },
        gravity_override: Some(GravityConfig {
            direction: Vec3::NEG_Y,
            magnitude: surface_gravity,
        }),
        linear_damping: 0.5,
        angular_damping: 0.5,
        friction_multiplier: 1.0,
        bounds: RegionBounds::Sphere { center, radius },
        priority: 10,
    }
}
