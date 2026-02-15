# Loading Radius Hysteresis

## Problem

When a player stands near the boundary of the chunk load radius, minor camera movement causes chunks at the edge to continuously load and unload. Each cycle wastes CPU time on generation, meshing, and GPU upload, only to discard the result moments later. This "chunk thrashing" creates visible pop-in, hitches in frame rate, and unnecessary disk I/O for persistent chunks. A naive single-radius approach (load when inside, unload when outside) is inherently unstable at the boundary because floating-point camera positions oscillate across the threshold with every frame.

## Solution

Implement a dual-radius hysteresis system in the `nebula_chunk` crate. Two concentric radii define the load and unload boundaries, creating a buffer zone where chunks maintain their current loaded/unloaded state.

### Configuration

```rust
use serde::{Deserialize, Serialize};

/// Configuration for the chunk loading radius with hysteresis.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LoadRadiusConfig {
    /// Chunks within this radius (in chunk units) are loaded.
    /// This is the inner radius.
    pub load_radius: u32,
    /// Chunks outside this radius (in chunk units) are unloaded.
    /// This is the outer radius. Must be > load_radius.
    pub unload_radius: u32,
}

impl LoadRadiusConfig {
    /// Create a config with a specific hysteresis gap.
    /// `load_radius` is the inner radius, and the unload radius is
    /// `load_radius + gap`.
    pub fn with_gap(load_radius: u32, gap: u32) -> Self {
        assert!(gap > 0, "hysteresis gap must be at least 1");
        Self {
            load_radius,
            unload_radius: load_radius + gap,
        }
    }

    /// The hysteresis gap in chunk units.
    pub fn gap(&self) -> u32 {
        self.unload_radius - self.load_radius
    }

    /// Determine what action to take for a chunk at the given distance.
    pub fn evaluate(&self, distance_in_chunks: u32, currently_loaded: bool) -> LoadAction {
        if distance_in_chunks <= self.load_radius {
            LoadAction::Load
        } else if distance_in_chunks > self.unload_radius {
            LoadAction::Unload
        } else {
            // Hysteresis zone: maintain current state
            if currently_loaded {
                LoadAction::KeepLoaded
            } else {
                LoadAction::KeepUnloaded
            }
        }
    }
}

impl Default for LoadRadiusConfig {
    fn default() -> Self {
        Self::with_gap(16, 2)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadAction {
    /// Chunk should be loaded (inside load radius).
    Load,
    /// Chunk should be unloaded (outside unload radius).
    Unload,
    /// Chunk is in the hysteresis zone and was already loaded; keep it.
    KeepLoaded,
    /// Chunk is in the hysteresis zone and was not loaded; keep it unloaded.
    KeepUnloaded,
}
```

### Distance Calculation

Chunk distance is computed in chunk-space coordinates using the 128-bit `ChunkAddress` type. The distance is the Chebyshev distance (max of per-axis deltas) to match the cubic load region shape:

```rust
use crate::coords::ChunkAddress;

/// Compute the Chebyshev distance between two chunk addresses, in chunk units.
/// Uses 128-bit coordinate differences to avoid overflow.
pub fn chunk_distance(a: &ChunkAddress, b: &ChunkAddress) -> u32 {
    let dx = (a.x as i128 - b.x as i128).unsigned_abs();
    let dy = (a.y as i128 - b.y as i128).unsigned_abs();
    let dz = (a.z as i128 - b.z as i128).unsigned_abs();
    let max = dx.max(dy).max(dz);
    // Clamp to u32 for practical radius comparisons
    max.min(u32::MAX as u128) as u32
}
```

### Chunk Evaluation System

A Bevy ECS system runs each frame (or when the camera crosses a chunk boundary) to evaluate all tracked chunks and determine load/unload actions:

```rust
use bevy_ecs::prelude::*;

#[derive(Resource)]
pub struct ChunkLoadRadiusConfig(pub LoadRadiusConfig);

fn evaluate_chunk_loading(
    config: Res<ChunkLoadRadiusConfig>,
    camera_query: Query<&ChunkAddress, With<Camera>>,
    chunk_query: Query<(Entity, &ChunkAddress, &ChunkStateMachine)>,
    mut commands: Commands,
) {
    let Ok(camera_addr) = camera_query.single() else { return };

    for (entity, chunk_addr, state_machine) in &chunk_query {
        let distance = chunk_distance(camera_addr, chunk_addr);
        let currently_loaded = state_machine.state() != ChunkState::Unloaded;

        match config.0.evaluate(distance, currently_loaded) {
            LoadAction::Load if !currently_loaded => {
                // Schedule chunk for loading
                commands.entity(entity).insert(ScheduleForLoad);
            }
            LoadAction::Unload if currently_loaded => {
                // Begin unload process
                commands.entity(entity).insert(ScheduleForUnload);
            }
            _ => {
                // KeepLoaded, KeepUnloaded, or already in the right state
            }
        }
    }
}
```

### Why Hysteresis Works

Consider a player at exactly the load radius boundary. Without hysteresis, a 0.01-unit camera jitter causes alternating load/unload signals. With a 2-chunk hysteresis gap, the chunk must move 2 full chunks beyond the load radius before it triggers an unload. This means a player must travel a significant distance away before chunks start disappearing, and oscillation at the boundary is impossible because the load and unload thresholds are separated.

## Outcome

The `nebula_chunk` crate exports `LoadRadiusConfig`, `LoadAction`, and `chunk_distance()`. The chunk evaluation system uses `LoadRadiusConfig::evaluate()` to decide per-chunk actions each frame. Chunk thrashing at the load boundary is eliminated. Running `cargo test -p nebula_chunk` passes all hysteresis tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Chunks are loaded at radius 12 but unloaded at radius 14. Moving back and forth at the boundary does not thrash chunks in and out repeatedly.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.18` | ECS system, `Resource`/`Component` derives, queries |
| `serde` | `1.0` | Serialize/deserialize `LoadRadiusConfig` for settings files |

No external math crates are needed. Distance calculations use Rust's built-in 128-bit integer arithmetic, matching the engine's 128-bit coordinate system. The crate uses Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// A chunk inside the load radius should produce a Load action.
    #[test]
    fn test_chunk_inside_load_radius_is_loaded() {
        let config = LoadRadiusConfig::with_gap(16, 2);
        let action = config.evaluate(10, false);
        assert_eq!(action, LoadAction::Load);
    }

    /// A chunk outside the unload radius should produce an Unload action.
    #[test]
    fn test_chunk_outside_unload_radius_is_unloaded() {
        let config = LoadRadiusConfig::with_gap(16, 2);
        // Unload radius is 18, so distance 19 is outside
        let action = config.evaluate(19, true);
        assert_eq!(action, LoadAction::Unload);
    }

    /// A chunk in the hysteresis zone that is already loaded stays loaded.
    #[test]
    fn test_hysteresis_zone_keeps_loaded_chunk_loaded() {
        let config = LoadRadiusConfig::with_gap(16, 2);
        // Distance 17 is between load_radius (16) and unload_radius (18)
        let action = config.evaluate(17, true);
        assert_eq!(action, LoadAction::KeepLoaded);
    }

    /// A chunk in the hysteresis zone that is not loaded stays unloaded.
    #[test]
    fn test_hysteresis_zone_keeps_unloaded_chunk_unloaded() {
        let config = LoadRadiusConfig::with_gap(16, 2);
        let action = config.evaluate(17, false);
        assert_eq!(action, LoadAction::KeepUnloaded);
    }

    /// The hysteresis gap is configurable and reflected correctly.
    #[test]
    fn test_gap_is_configurable() {
        let config_small = LoadRadiusConfig::with_gap(10, 1);
        assert_eq!(config_small.load_radius, 10);
        assert_eq!(config_small.unload_radius, 11);
        assert_eq!(config_small.gap(), 1);

        let config_large = LoadRadiusConfig::with_gap(10, 5);
        assert_eq!(config_large.load_radius, 10);
        assert_eq!(config_large.unload_radius, 15);
        assert_eq!(config_large.gap(), 5);

        // Distance 12 is in zone for large gap but outside for small gap
        assert_eq!(config_small.evaluate(12, true), LoadAction::Unload);
        assert_eq!(config_large.evaluate(12, true), LoadAction::KeepLoaded);
    }

    /// The default config should have a load radius of 16 and gap of 2.
    #[test]
    fn test_default_config() {
        let config = LoadRadiusConfig::default();
        assert_eq!(config.load_radius, 16);
        assert_eq!(config.unload_radius, 18);
        assert_eq!(config.gap(), 2);
    }

    /// Chunk at exactly the load radius boundary should be loaded.
    #[test]
    fn test_exact_load_radius_boundary_is_loaded() {
        let config = LoadRadiusConfig::with_gap(16, 2);
        let action = config.evaluate(16, false);
        assert_eq!(action, LoadAction::Load);
    }

    /// Chunk at exactly the unload radius boundary is in the hysteresis zone.
    #[test]
    fn test_exact_unload_radius_boundary_is_hysteresis() {
        let config = LoadRadiusConfig::with_gap(16, 2);
        // Distance 18 == unload_radius, which is NOT > unload_radius
        let action = config.evaluate(18, true);
        assert_eq!(action, LoadAction::KeepLoaded);
    }
}
```
