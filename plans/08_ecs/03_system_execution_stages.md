# System Execution Stages

## Problem

Having six named schedule stages (from story 01) is not enough. Without a documented contract specifying which resources and components each stage may read or write, systems will inevitably conflict: a gameplay system writes `WorldPos` at the same time the transform propagation system reads it, or a rendering system mutates the draw list while culling is still populating it. These conflicts either cause runtime panics (bevy_ecs detects conflicting mutable borrows) or, worse, produce silent data races if the access patterns happen to interleave.

The engine needs a formal ordering contract that defines, for each stage: what data flows in, what data flows out, what invariants hold before the stage runs, and what invariants hold after. This contract enables subsystem authors to register their systems in the correct stage with confidence, and it enables the schedule validator to detect violations at startup rather than mid-game.

## Solution

### Stage Contract Specification

Each stage has a defined set of readable and writable resources/components. Systems registered in a stage must respect these contracts. The contracts are documented here and enforced by convention and by bevy_ecs's access conflict detection.

#### PreUpdate

**Purpose:** Process external input and advance time.

| Access | Resource / Component | Mode |
|--------|---------------------|------|
| Write | `TimeRes` | Update `delta`, `elapsed`, `tick` |
| Write | `InputState` | Poll keyboard, mouse, gamepad |
| Read | OS window events | Via winit event forwarding |

**Invariants after PreUpdate:**
- `TimeRes.delta` reflects the wall-clock time since the last frame
- `TimeRes.tick` is incremented by 1
- `InputState` reflects the current frame's input state
- No entity components are modified

#### FixedUpdate

**Purpose:** Deterministic physics and simulation at 60 Hz.

| Access | Resource / Component | Mode |
|--------|---------------------|------|
| Read | `TimeRes` | Read `fixed_dt` for integration |
| Read | `InputState` | Read player movement intents |
| Write | `WorldPos` | Apply velocity, physics corrections |
| Write | `Velocity` | Apply forces, drag, gravity |
| Read | `Rotation` | Directional velocity application |
| Read | `ChunkManager` | Collision queries against voxel terrain |
| Write | Physics internal state | Rapier world step |

**Invariants after FixedUpdate:**
- All `WorldPos` components reflect physics-corrected positions
- All `Velocity` components reflect post-force-application values
- Physics broadphase/narrowphase structures are up to date
- The simulation has advanced by exactly `fixed_dt` seconds

**Execution:** Runs 0 to N times per frame (typically 1, capped at 10) based on the time accumulator. Each invocation advances the simulation by exactly `fixed_dt`.

#### Update

**Purpose:** Variable-rate gameplay logic.

| Access | Resource / Component | Mode |
|--------|---------------------|------|
| Read | `TimeRes` | Delta time for animations, timers |
| Read | `InputState` | UI interaction, non-physics input |
| Read | `WorldPos` | Spatial queries for AI, triggers |
| Write | Gameplay components | Health, inventory, AI state, etc. |
| Read | `ChunkManager` | Voxel modification requests queued |
| Write | `VoxelRegistry` | Register new block types if needed |

**Invariants after Update:**
- All gameplay state is current for this frame
- Voxel modification requests are queued but not yet applied
- No spatial components (`WorldPos`, `LocalPos`) are modified

#### PostUpdate

**Purpose:** Derive rendering-space data from simulation-space data.

| Access | Resource / Component | Mode |
|--------|---------------------|------|
| Read | `WorldPos` | Source position for conversion |
| Read | `CameraRes` | Camera's world origin for offset |
| Write | `LocalPos` | Computed as `WorldPos - CameraRes.world_origin` |
| Write | Spatial acceleration structures | Update octree/BVH |
| Read | `Rotation`, `Scale` | Build model matrices |

**Invariants after PostUpdate:**
- All `LocalPos` components are consistent with their `WorldPos` and the current camera position
- Spatial acceleration structures are rebuilt and query-ready
- Model matrices are computed and cached for rendering

The core transform propagation system:

```rust
fn update_local_positions(
    camera: Res<CameraRes>,
    mut query: Query<(&WorldPos, &mut LocalPos)>,
) {
    for (world_pos, mut local_pos) in &mut query {
        // Subtract camera origin from world position, convert to f32
        let offset = world_pos.0 - camera.world_origin;
        local_pos.0 = LocalPosition::new(
            offset.x as f32,
            offset.y as f32,
            offset.z as f32,
        );
    }
}
```

#### PreRender

**Purpose:** Prepare GPU work: cull, batch, upload.

| Access | Resource / Component | Mode |
|--------|---------------------|------|
| Read | `LocalPos` | Frustum culling tests |
| Read | `Rotation`, `Scale` | Final transform for GPU |
| Read | Mesh handles, material handles | Determine what to draw |
| Write | `RenderContext` internal buffers | Upload vertex/index data |
| Write | Draw command lists | Build indirect draw commands |
| Read | `Active` | Skip inactive entities |

**Invariants after PreRender:**
- All visible entities have been identified via frustum culling
- Draw commands are batched by material/mesh for minimal state changes
- GPU buffers are uploaded and ready for rendering
- Inactive entities (`Active(false)`) are excluded from all draw lists

#### Render

**Purpose:** Execute GPU commands.

| Access | Resource / Component | Mode |
|--------|---------------------|------|
| Write | `RenderContext` | Surface acquisition, command submission |
| Read | Draw command lists | Issue draw calls |
| Read | GPU buffers | Bind vertex/index/uniform buffers |

**Invariants after Render:**
- All draw calls for this frame have been submitted to the GPU queue
- The swapchain frame has been presented
- No entity components are modified

### Stage Ordering Enforcement

The stages execute in strict sequence: `PreUpdate -> FixedUpdate -> Update -> PostUpdate -> PreRender -> Render`. This is enforced by the `EngineSchedules::run()` method (story 01), which iterates the stages in a fixed-order vector. There is no mechanism to reorder stages at runtime.

### Data Flow Diagram

```
PreUpdate                    FixedUpdate              Update
  |                            |                       |
  +--> TimeRes (write)         +--> WorldPos (write)   +--> Gameplay (write)
  +--> InputState (write)      +--> Velocity (write)   |
  |                            |                       |
  v                            v                       v
                          PostUpdate                PreRender
                            |                        |
                            +--> LocalPos (write)    +--> DrawCmds (write)
                            |                        +--> GPU buffers (write)
                            v                        |
                                                     v
                                                   Render
                                                     |
                                                     +--> Surface present
```

### Conflict Prevention Rules

1. **WorldPos** is written only in `FixedUpdate`. All other stages read it.
2. **LocalPos** is written only in `PostUpdate`. PreRender and Render read it.
3. **InputState** is written only in `PreUpdate`. FixedUpdate and Update read it.
4. **TimeRes** is written only in `PreUpdate`. All other stages read it.
5. **RenderContext** is written only in `PreRender` and `Render`. No other stage touches it.
6. Systems that violate these rules will trigger bevy_ecs access conflict panics at schedule build time.

## Outcome

After this story is complete:

- Every engine stage has a documented data-access contract
- Subsystem authors know exactly which stage to register their systems in
- The data flow between stages is explicit and unambiguous
- bevy_ecs's automatic conflict detection enforces the contracts at runtime
- The `update_local_positions` system is implemented and registered in `PostUpdate`
- The fixed timestep guarantee from story 01 is reinforced with concrete invariants

## Demo Integration

**Demo crate:** `nebula-demo`

Systems run in ordered stages: PreUpdate, Update, PostUpdate, Render. The console logs the stage execution order each frame.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.15` | Schedule execution, system access validation, queries |
| `nebula-math` | workspace | `WorldPosition`, `LocalPosition`, `Vec3I128` |

Rust edition 2024. No additional dependencies beyond what stories 01 and 02 already require.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::prelude::*;

    #[derive(Resource, Default)]
    struct StageOrder(Vec<&'static str>);

    #[test]
    fn test_postupdate_runs_after_update() {
        let mut world = World::new();
        world.insert_resource(StageOrder::default());
        world.insert_resource(TimeRes::default());

        let mut schedules = EngineSchedules::new();
        schedules.add_system(
            EngineSchedule::Update,
            |mut order: ResMut<StageOrder>| { order.0.push("Update"); },
        );
        schedules.add_system(
            EngineSchedule::PostUpdate,
            |mut order: ResMut<StageOrder>| { order.0.push("PostUpdate"); },
        );

        schedules.run(&mut world, 1.0 / 60.0);

        let order = world.resource::<StageOrder>();
        let update_idx = order.0.iter().position(|s| *s == "Update").unwrap();
        let post_idx = order.0.iter().position(|s| *s == "PostUpdate").unwrap();
        assert!(update_idx < post_idx, "PostUpdate must run after Update");
    }

    #[test]
    fn test_prerender_runs_after_postupdate() {
        let mut world = World::new();
        world.insert_resource(StageOrder::default());
        world.insert_resource(TimeRes::default());

        let mut schedules = EngineSchedules::new();
        schedules.add_system(
            EngineSchedule::PostUpdate,
            |mut order: ResMut<StageOrder>| { order.0.push("PostUpdate"); },
        );
        schedules.add_system(
            EngineSchedule::PreRender,
            |mut order: ResMut<StageOrder>| { order.0.push("PreRender"); },
        );

        schedules.run(&mut world, 1.0 / 60.0);

        let order = world.resource::<StageOrder>();
        let post_idx = order.0.iter().position(|s| *s == "PostUpdate").unwrap();
        let pre_render_idx = order.0.iter().position(|s| *s == "PreRender").unwrap();
        assert!(post_idx < pre_render_idx);
    }

    #[test]
    fn test_parallel_systems_no_conflicting_writes() {
        // Two read-only systems in the same stage should not conflict.
        // bevy_ecs will panic at schedule build time if they do.
        let mut world = World::new();
        world.insert_resource(TimeRes::default());

        #[derive(Resource, Default)]
        struct OutputA(f64);
        #[derive(Resource, Default)]
        struct OutputB(f64);
        world.insert_resource(OutputA::default());
        world.insert_resource(OutputB::default());

        let mut schedules = EngineSchedules::new();
        schedules.add_system(
            EngineSchedule::Update,
            (
                |time: Res<TimeRes>, mut a: ResMut<OutputA>| {
                    a.0 = time.elapsed;
                },
                |time: Res<TimeRes>, mut b: ResMut<OutputB>| {
                    b.0 = time.elapsed;
                },
            ),
        );

        // This must not panic — the systems share only Res<TimeRes> (read)
        // and write to disjoint resources.
        schedules.run(&mut world, 1.0 / 60.0);
    }

    #[test]
    fn test_fixed_update_respects_timestep_budget() {
        let mut world = World::new();
        world.insert_resource(TimeRes::default());

        #[derive(Resource, Default)]
        struct SimTime(f64);
        world.insert_resource(SimTime(0.0));

        let mut schedules = EngineSchedules::new();
        schedules.add_system(
            EngineSchedule::FixedUpdate,
            |mut sim: ResMut<SimTime>| {
                sim.0 += 1.0 / 60.0; // Each tick advances by fixed_dt
            },
        );

        // Run 60 frames at 60 FPS — should produce exactly 60 fixed ticks
        for _ in 0..60 {
            schedules.run(&mut world, 1.0 / 60.0);
        }

        let sim = world.resource::<SimTime>();
        assert!(
            (sim.0 - 1.0).abs() < 1e-9,
            "60 ticks at 1/60 should equal 1.0 second, got {}",
            sim.0
        );
    }

    #[test]
    fn test_local_pos_updated_from_world_pos_in_postupdate() {
        let mut world = World::new();
        world.insert_resource(TimeRes::default());
        world.insert_resource(CameraRes {
            entity: Entity::PLACEHOLDER,
            world_origin: WorldPosition::new(1000, 2000, 3000),
        });

        let entity = world
            .spawn((
                WorldPos::new(1100, 2200, 3300),
                LocalPos::default(),
            ))
            .id();

        let mut schedules = EngineSchedules::new();
        schedules.add_system(EngineSchedule::PostUpdate, update_local_positions);

        schedules.run(&mut world, 1.0 / 60.0);

        let local = world.get::<LocalPos>(entity).unwrap();
        assert_eq!(local.0.x, 100.0); // 1100 - 1000
        assert_eq!(local.0.y, 200.0); // 2200 - 2000
        assert_eq!(local.0.z, 300.0); // 3300 - 3000
    }

    #[test]
    fn test_render_runs_last() {
        let mut world = World::new();
        world.insert_resource(StageOrder::default());
        world.insert_resource(TimeRes::default());

        let mut schedules = EngineSchedules::new();
        for (stage, name) in [
            (EngineSchedule::PreUpdate, "PreUpdate"),
            (EngineSchedule::FixedUpdate, "FixedUpdate"),
            (EngineSchedule::Update, "Update"),
            (EngineSchedule::PostUpdate, "PostUpdate"),
            (EngineSchedule::PreRender, "PreRender"),
            (EngineSchedule::Render, "Render"),
        ] {
            let n = name;
            schedules.add_system(stage, move |mut order: ResMut<StageOrder>| {
                order.0.push(n);
            });
        }

        schedules.run(&mut world, 1.0 / 60.0);

        let order = world.resource::<StageOrder>();
        assert_eq!(*order.0.last().unwrap(), "Render");
    }
}
```
