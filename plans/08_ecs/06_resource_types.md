# Resource Types

## Problem

Components live on entities, but many engine-wide values have no natural entity home. Time progression, camera state, input state, the GPU rendering context, the chunk manager, and the voxel registry are all singletons — there is exactly one of each, and every system in the engine needs to read or write them. Storing these as components on a "god entity" is a common anti-pattern that defeats the purpose of the ECS by creating a bottleneck entity that every query must touch.

bevy_ecs provides `Resource` — a typed singleton stored in the `World` alongside entities but accessed through `Res<T>` (shared read) and `ResMut<T>` (exclusive write). Resources integrate with the parallel scheduler: multiple systems can read `Res<TimeRes>` concurrently, but a system writing `ResMut<TimeRes>` blocks all other readers and writers of that resource. Getting the resource types right — with correct fields, sensible defaults, and clear ownership semantics — is critical because every system in the engine depends on at least one of them.

## Solution

Define the following resource types in the `nebula_ecs` crate, each deriving `bevy_ecs::prelude::Resource`:

### TimeRes

```rust
use bevy_ecs::prelude::*;

/// Global time state, updated once per frame in PreUpdate.
///
/// - `delta`: Wall-clock seconds since the previous frame (variable).
/// - `elapsed`: Total wall-clock seconds since engine start.
/// - `fixed_dt`: Fixed timestep for FixedUpdate (constant, default 1/60).
/// - `tick`: Monotonically increasing tick counter, incremented by 1 each
///   frame regardless of how many fixed steps ran.
#[derive(Resource, Clone, Debug)]
pub struct TimeRes {
    pub delta: f32,
    pub elapsed: f64,
    pub fixed_dt: f32,
    pub tick: u64,
}

impl Default for TimeRes {
    fn default() -> Self {
        Self {
            delta: 0.0,
            elapsed: 0.0,
            fixed_dt: 1.0 / 60.0,
            tick: 0,
        }
    }
}
```

### CameraRes

```rust
use nebula_math::WorldPosition;

/// Active camera state, updated by the player/camera controller system.
///
/// - `entity`: The entity ID of the camera entity (for querying its
///   components directly).
/// - `world_origin`: The camera's position in 128-bit world coordinates.
///   PostUpdate subtracts this from every entity's WorldPos to compute
///   their LocalPos. Setting this to the camera position ensures maximum
///   f32 precision for nearby geometry.
#[derive(Resource, Clone, Debug)]
pub struct CameraRes {
    pub entity: Entity,
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
```

### InputState

```rust
use std::collections::HashSet;

/// Aggregated input state for the current frame. Written by PreUpdate,
/// read by FixedUpdate and Update.
///
/// Provides action-based input queries rather than raw key codes.
/// The mapping from physical keys to actions is configured externally
/// (in nebula-input). This resource exposes the processed result.
#[derive(Resource, Clone, Debug, Default)]
pub struct InputState {
    /// Actions that are currently held down.
    pub active_actions: HashSet<String>,
    /// Actions that were first pressed this frame.
    pub just_pressed: HashSet<String>,
    /// Actions that were released this frame.
    pub just_released: HashSet<String>,
    /// Mouse movement delta in pixels since last frame.
    pub mouse_delta: (f32, f32),
    /// Mouse scroll delta (horizontal, vertical).
    pub scroll_delta: (f32, f32),
    /// Current cursor position in window coordinates, if available.
    pub cursor_position: Option<(f32, f32)>,
}

impl InputState {
    /// Returns true if the named action is currently held down.
    pub fn is_active(&self, action: &str) -> bool {
        self.active_actions.contains(action)
    }

    /// Returns true if the named action was first pressed this frame.
    pub fn just_pressed(&self, action: &str) -> bool {
        self.just_pressed.contains(action)
    }

    /// Returns true if the named action was released this frame.
    pub fn just_released(&self, action: &str) -> bool {
        self.just_released.contains(action)
    }

    /// Clear per-frame transient state. Called at the start of PreUpdate
    /// before processing new input events.
    pub fn clear_transients(&mut self) {
        self.just_pressed.clear();
        self.just_released.clear();
        self.mouse_delta = (0.0, 0.0);
        self.scroll_delta = (0.0, 0.0);
    }
}
```

### RenderContext

```rust
/// GPU rendering context. Wraps wgpu objects and frame state.
/// Written only by PreRender and Render stages.
///
/// The actual wgpu types (Device, Queue, Surface) are defined in
/// nebula-render. This resource type in nebula-ecs is a placeholder
/// that nebula-render will extend with the concrete GPU state.
/// The ECS crate defines the resource slot; the render crate fills it.
#[derive(Resource)]
pub struct RenderContext {
    /// Opaque handle to the GPU context. The concrete type is defined
    /// in nebula-render and stored as a type-erased box here to avoid
    /// a dependency cycle (nebula-ecs cannot depend on nebula-render).
    inner: Box<dyn std::any::Any + Send + Sync>,
}

impl RenderContext {
    pub fn new<T: Send + Sync + 'static>(context: T) -> Self {
        Self {
            inner: Box::new(context),
        }
    }

    /// Downcast to the concrete GPU context type.
    pub fn get<T: 'static>(&self) -> Option<&T> {
        self.inner.downcast_ref::<T>()
    }

    /// Downcast to the concrete GPU context type (mutable).
    pub fn get_mut<T: 'static>(&mut self) -> Option<&mut T> {
        self.inner.downcast_mut::<T>()
    }
}
```

### ChunkManager

```rust
use std::collections::HashMap;

/// Manages the lifecycle of voxel chunks: which chunks are loaded,
/// which are pending generation, which should be unloaded.
///
/// The actual chunk data storage is in nebula-voxel. This resource
/// tracks chunk entities and their loading state.
#[derive(Resource, Debug, Default)]
pub struct ChunkManager {
    /// Map from chunk coordinate to the entity representing that chunk.
    pub loaded_chunks: HashMap<(i64, i64, i64), Entity>,
    /// Chunk coordinates queued for generation.
    pub pending_load: Vec<(i64, i64, i64)>,
    /// Chunk coordinates queued for unloading.
    pub pending_unload: Vec<(i64, i64, i64)>,
    /// The render distance in chunks.
    pub render_distance: u32,
}

impl ChunkManager {
    pub fn new(render_distance: u32) -> Self {
        Self {
            render_distance,
            ..Default::default()
        }
    }

    pub fn is_loaded(&self, coord: (i64, i64, i64)) -> bool {
        self.loaded_chunks.contains_key(&coord)
    }

    pub fn chunk_entity(&self, coord: (i64, i64, i64)) -> Option<Entity> {
        self.loaded_chunks.get(&coord).copied()
    }

    pub fn loaded_count(&self) -> usize {
        self.loaded_chunks.len()
    }
}
```

### VoxelRegistry

```rust
/// Registry of all known voxel/block types. Maps block IDs to their
/// properties (name, is_solid, is_transparent, texture indices, etc.).
///
/// This resource is populated at startup and may be extended at runtime
/// by mods or procedural generation. It is read-only during normal
/// simulation — only Update may add new types.
#[derive(Resource, Debug, Default)]
pub struct VoxelRegistry {
    entries: Vec<VoxelTypeEntry>,
}

#[derive(Debug, Clone)]
pub struct VoxelTypeEntry {
    pub id: u16,
    pub name: String,
    pub is_solid: bool,
    pub is_transparent: bool,
}

impl VoxelRegistry {
    pub fn register(&mut self, entry: VoxelTypeEntry) -> u16 {
        let id = self.entries.len() as u16;
        self.entries.push(VoxelTypeEntry { id, ..entry });
        id
    }

    pub fn get(&self, id: u16) -> Option<&VoxelTypeEntry> {
        self.entries.get(id as usize)
    }

    pub fn count(&self) -> usize {
        self.entries.len()
    }
}
```

### Resource Registration

All resources are inserted into the world during initialization:

```rust
pub fn register_core_resources(world: &mut World) {
    world.insert_resource(TimeRes::default());
    world.insert_resource(CameraRes::default());
    world.insert_resource(InputState::default());
    world.insert_resource(ChunkManager::new(16)); // default 16 chunk render distance
    world.insert_resource(VoxelRegistry::default());
    // RenderContext is inserted later by nebula-render after GPU init
}
```

### Access Patterns

| Resource | PreUpdate | FixedUpdate | Update | PostUpdate | PreRender | Render |
|----------|-----------|-------------|--------|------------|-----------|--------|
| `TimeRes` | **Write** | Read | Read | Read | Read | Read |
| `CameraRes` | - | - | Read | **Read** | Read | - |
| `InputState` | **Write** | Read | Read | - | - | - |
| `RenderContext` | - | - | - | - | **Write** | **Write** |
| `ChunkManager` | - | Read | Read | Read | Read | - |
| `VoxelRegistry` | - | - | Read/Write | - | Read | - |

## Outcome

After this story is complete:

- Six resource types are defined and ready for insertion into the ECS world
- `register_core_resources(world)` sets up all resources with sensible defaults
- Systems access resources via `Res<T>` (concurrent read) and `ResMut<T>` (exclusive write)
- The access table above documents which stages may read/write each resource
- `InputState` provides action-based queries with transient per-frame state
- `ChunkManager` tracks chunk loading state by coordinate
- `VoxelRegistry` maps block IDs to properties
- `RenderContext` uses type erasure to avoid a dependency cycle with `nebula-render`

## Demo Integration

**Demo crate:** `nebula-demo`

`RenderContext`, `ChunkManager`, and `VoxelTypeRegistry` are now ECS resources accessed via `Res<>` and `ResMut<>`. No visible change; architectural upgrade.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.15` | `Resource` derive, `Res<T>`, `ResMut<T>` |
| `nebula-math` | workspace | `WorldPosition` for `CameraRes` |

Rust edition 2024. The `std::collections` module provides `HashMap` and `HashSet` used by `ChunkManager` and `InputState`. `std::any::Any` provides type erasure for `RenderContext`.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::prelude::*;

    #[test]
    fn test_time_res_insertion_and_retrieval() {
        let mut world = World::new();
        world.insert_resource(TimeRes::default());

        let time = world.resource::<TimeRes>();
        assert_eq!(time.delta, 0.0);
        assert_eq!(time.elapsed, 0.0);
        assert_eq!(time.tick, 0);
        assert!((time.fixed_dt - 1.0 / 60.0).abs() < 1e-7);
    }

    #[test]
    fn test_camera_res_insertion_and_retrieval() {
        let mut world = World::new();
        world.insert_resource(CameraRes::default());

        let cam = world.resource::<CameraRes>();
        assert_eq!(cam.world_origin, WorldPosition::default());
    }

    #[test]
    fn test_input_state_action_queries() {
        let mut input = InputState::default();
        input.active_actions.insert("jump".to_string());
        input.just_pressed.insert("fire".to_string());
        input.just_released.insert("crouch".to_string());

        assert!(input.is_active("jump"));
        assert!(!input.is_active("fire"));
        assert!(input.just_pressed("fire"));
        assert!(!input.just_pressed("jump"));
        assert!(input.just_released("crouch"));
    }

    #[test]
    fn test_input_state_clear_transients() {
        let mut input = InputState::default();
        input.just_pressed.insert("fire".to_string());
        input.just_released.insert("crouch".to_string());
        input.mouse_delta = (10.0, 20.0);
        input.scroll_delta = (0.0, 3.0);

        input.clear_transients();

        assert!(input.just_pressed.is_empty());
        assert!(input.just_released.is_empty());
        assert_eq!(input.mouse_delta, (0.0, 0.0));
        assert_eq!(input.scroll_delta, (0.0, 0.0));
    }

    #[test]
    #[should_panic]
    fn test_missing_resource_panics_at_runtime() {
        let world = World::new();
        // Accessing a resource that was never inserted should panic
        let _time = world.resource::<TimeRes>();
    }

    #[test]
    fn test_resmut_allows_mutation() {
        let mut world = World::new();
        world.insert_resource(TimeRes::default());

        // Simulate a system writing to TimeRes
        let mut time = world.resource_mut::<TimeRes>();
        time.delta = 0.016;
        time.elapsed = 1.5;
        time.tick = 90;

        let time = world.resource::<TimeRes>();
        assert_eq!(time.delta, 0.016);
        assert_eq!(time.elapsed, 1.5);
        assert_eq!(time.tick, 90);
    }

    #[test]
    fn test_multiple_res_reads_are_concurrent() {
        // Two systems that both read Res<TimeRes> should be able to
        // coexist in the same stage. bevy_ecs schedules them in parallel.
        let mut world = World::new();
        world.insert_resource(TimeRes {
            delta: 0.016,
            elapsed: 5.0,
            fixed_dt: 1.0 / 60.0,
            tick: 300,
        });

        #[derive(Resource, Default)]
        struct ReadA(f64);
        #[derive(Resource, Default)]
        struct ReadB(f64);
        world.insert_resource(ReadA::default());
        world.insert_resource(ReadB::default());

        let mut schedule = Schedule::default();
        schedule.add_systems((
            |time: Res<TimeRes>, mut a: ResMut<ReadA>| {
                a.0 = time.elapsed;
            },
            |time: Res<TimeRes>, mut b: ResMut<ReadB>| {
                b.0 = time.elapsed;
            },
        ));

        // Must not panic with a conflicting access error
        schedule.run(&mut world);

        assert_eq!(world.resource::<ReadA>().0, 5.0);
        assert_eq!(world.resource::<ReadB>().0, 5.0);
    }

    #[test]
    fn test_chunk_manager_operations() {
        let mut cm = ChunkManager::new(8);
        assert_eq!(cm.render_distance, 8);
        assert_eq!(cm.loaded_count(), 0);
        assert!(!cm.is_loaded((0, 0, 0)));

        let mut world = World::new();
        let entity = world.spawn_empty().id();
        cm.loaded_chunks.insert((0, 0, 0), entity);

        assert!(cm.is_loaded((0, 0, 0)));
        assert_eq!(cm.chunk_entity((0, 0, 0)), Some(entity));
        assert_eq!(cm.loaded_count(), 1);
    }

    #[test]
    fn test_voxel_registry_register_and_get() {
        let mut registry = VoxelRegistry::default();
        assert_eq!(registry.count(), 0);

        let id = registry.register(VoxelTypeEntry {
            id: 0, // overwritten by register()
            name: "stone".to_string(),
            is_solid: true,
            is_transparent: false,
        });

        assert_eq!(id, 0);
        assert_eq!(registry.count(), 1);

        let entry = registry.get(id).unwrap();
        assert_eq!(entry.name, "stone");
        assert!(entry.is_solid);
        assert!(!entry.is_transparent);
    }

    #[test]
    fn test_voxel_registry_multiple_types() {
        let mut registry = VoxelRegistry::default();
        let air = registry.register(VoxelTypeEntry {
            id: 0,
            name: "air".to_string(),
            is_solid: false,
            is_transparent: true,
        });
        let stone = registry.register(VoxelTypeEntry {
            id: 0,
            name: "stone".to_string(),
            is_solid: true,
            is_transparent: false,
        });
        let glass = registry.register(VoxelTypeEntry {
            id: 0,
            name: "glass".to_string(),
            is_solid: true,
            is_transparent: true,
        });

        assert_eq!(air, 0);
        assert_eq!(stone, 1);
        assert_eq!(glass, 2);
        assert_eq!(registry.count(), 3);
        assert!(registry.get(stone).unwrap().is_solid);
        assert!(registry.get(glass).unwrap().is_transparent);
    }

    #[test]
    fn test_render_context_type_erasure() {
        struct MockGpuContext {
            device_name: String,
        }

        let ctx = RenderContext::new(MockGpuContext {
            device_name: "Test GPU".to_string(),
        });

        let gpu = ctx.get::<MockGpuContext>().unwrap();
        assert_eq!(gpu.device_name, "Test GPU");

        // Wrong type returns None
        assert!(ctx.get::<String>().is_none());
    }

    #[test]
    fn test_register_core_resources_inserts_all() {
        let mut world = World::new();
        register_core_resources(&mut world);

        assert!(world.contains_resource::<TimeRes>());
        assert!(world.contains_resource::<CameraRes>());
        assert!(world.contains_resource::<InputState>());
        assert!(world.contains_resource::<ChunkManager>());
        assert!(world.contains_resource::<VoxelRegistry>());
    }
}
```
