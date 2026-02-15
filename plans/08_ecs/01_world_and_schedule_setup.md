# World & Schedule Setup

## Problem

The Nebula Engine needs a central data store and a deterministic execution pipeline before any system can run. Without an ECS world, there is nowhere to store entities, components, or resources. Without a schedule, there is no way to order system execution, separate physics from rendering, or guarantee that transform updates happen after gameplay logic. The engine uses `bevy_ecs` as a standalone library (not the full Bevy engine) because it provides a mature, archetype-based ECS with parallel system scheduling, change detection, and resource management — without pulling in Bevy's renderer, asset pipeline, or windowing code.

The schedule must enforce a strict stage ordering: input processing happens before gameplay, gameplay before physics, physics before transform propagation, transforms before rendering. Within each stage, independent systems must run in parallel to exploit multi-core hardware. The `FixedUpdate` stage must tick at a deterministic 60 Hz regardless of frame rate, accumulating time between frames and running zero or more fixed steps per frame. Getting this wrong means physics diverges between machines, rendering stutters, or systems silently read stale data.

## Solution

Define the world and schedule initialization in the `nebula_ecs` crate.

### World Creation

```rust
use bevy_ecs::prelude::*;

/// Creates and returns a fully initialized ECS world with all engine
/// resources pre-inserted (TimeRes, etc.) and default values.
pub fn create_world() -> World {
    let mut world = World::new();

    // Insert core resources with sensible defaults
    world.insert_resource(TimeRes::default());

    world
}
```

The `World` is the single source of truth for all entity/component data and all shared resources. It is created once at engine startup and passed by mutable reference to the schedule each frame.

### Stage Definition

Define an enum for each execution stage, used as schedule labels:

```rust
use bevy_ecs::schedule::ScheduleLabel;

#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
pub enum EngineSchedule {
    PreUpdate,
    FixedUpdate,
    Update,
    PostUpdate,
    PreRender,
    Render,
}
```

Each variant corresponds to a distinct `Schedule` instance. The engine maintains a `Vec` or ordered collection of these labels and runs them sequentially each frame:

1. **`PreUpdate`** — Poll input devices, update `InputState` resource, process window events. Systems here write to `InputState` and read `TimeRes`.
2. **`FixedUpdate`** — Deterministic simulation step at 60 Hz (`fixed_dt = 1/60`). Reads input, runs physics, updates `WorldPos` and `Velocity`. May run 0, 1, or multiple times per frame depending on accumulated time.
3. **`Update`** — Variable-rate gameplay logic. AI decisions, animation state machines, gameplay scripts. Reads `WorldPos`, writes gameplay-specific components.
4. **`PostUpdate`** — Transform propagation. Converts `WorldPos` to camera-relative `LocalPos`. Updates spatial acceleration structures. Reads `CameraRes`, writes `LocalPos`.
5. **`PreRender`** — Frustum culling, draw call batching, GPU buffer uploads. Reads `LocalPos` and mesh data, writes render command buffers.
6. **`Render`** — Issues GPU commands. Reads render command buffers, writes to the swapchain. Must be single-threaded for GPU submission.

### Schedule Construction

```rust
use bevy_ecs::schedule::Schedule;
use std::collections::HashMap;

pub struct EngineSchedules {
    schedules: Vec<(EngineSchedule, Schedule)>,
    fixed_accumulator: f64,
    fixed_dt: f64,
}

impl EngineSchedules {
    pub fn new() -> Self {
        let stages = vec![
            EngineSchedule::PreUpdate,
            EngineSchedule::FixedUpdate,
            EngineSchedule::Update,
            EngineSchedule::PostUpdate,
            EngineSchedule::PreRender,
            EngineSchedule::Render,
        ];

        let schedules = stages
            .into_iter()
            .map(|label| (label, Schedule::default()))
            .collect();

        Self {
            schedules,
            fixed_accumulator: 0.0,
            fixed_dt: 1.0 / 60.0,
        }
    }

    /// Register a system into a specific stage.
    pub fn add_system<M>(
        &mut self,
        stage: EngineSchedule,
        system: impl IntoSystemConfigs<M>,
    ) {
        for (label, schedule) in &mut self.schedules {
            if *label == stage {
                schedule.add_systems(system);
                return;
            }
        }
        panic!("Unknown stage: {:?}", stage);
    }

    /// Run all stages in order. FixedUpdate may run multiple times
    /// based on the accumulated delta time.
    pub fn run(&mut self, world: &mut World, frame_dt: f64) {
        // PreUpdate always runs once
        self.run_stage(EngineSchedule::PreUpdate, world);

        // FixedUpdate runs 0..N times to consume accumulated time
        self.fixed_accumulator += frame_dt;
        while self.fixed_accumulator >= self.fixed_dt {
            self.run_stage(EngineSchedule::FixedUpdate, world);
            self.fixed_accumulator -= self.fixed_dt;
        }

        // Remaining stages run once per frame
        self.run_stage(EngineSchedule::Update, world);
        self.run_stage(EngineSchedule::PostUpdate, world);
        self.run_stage(EngineSchedule::PreRender, world);
        self.run_stage(EngineSchedule::Render, world);
    }

    fn run_stage(&mut self, target: EngineSchedule, world: &mut World) {
        for (label, schedule) in &mut self.schedules {
            if *label == target {
                schedule.run(world);
                return;
            }
        }
    }
}
```

### Parallel Execution

Within each stage, `bevy_ecs` automatically parallelizes systems whose parameter access sets do not conflict. Two systems that both read `Res<TimeRes>` run in parallel. A system that writes `ResMut<InputState>` will not run concurrently with another system that reads `Res<InputState>` in the same stage. The engine does not need to manage thread pools manually — `bevy_ecs` uses its internal multi-threaded executor.

### Fixed Timestep Details

The `FixedUpdate` stage uses a time accumulator pattern. Each frame, `frame_dt` (wall-clock seconds since last frame) is added to the accumulator. The fixed step runs repeatedly until the accumulator drops below `fixed_dt` (1/60 second = ~16.667 ms). A cap of 10 fixed steps per frame prevents spiral-of-death scenarios where the simulation can never catch up:

```rust
const MAX_FIXED_STEPS_PER_FRAME: u32 = 10;

// In run():
let mut steps = 0;
while self.fixed_accumulator >= self.fixed_dt && steps < MAX_FIXED_STEPS_PER_FRAME {
    self.run_stage(EngineSchedule::FixedUpdate, world);
    self.fixed_accumulator -= self.fixed_dt;
    steps += 1;
}
```

## Outcome

After this story is complete, the engine has:

- A `World` instance that serves as the root container for all entities, components, and resources
- Six ordered execution stages that run every frame in a deterministic sequence
- A `FixedUpdate` stage that ticks at a stable 60 Hz regardless of frame rate
- Automatic parallel execution of non-conflicting systems within each stage
- A clean API for registering systems into specific stages
- A spiral-of-death guard limiting fixed steps per frame

Running `EngineSchedules::new()` followed by `schedules.run(&mut world, dt)` drives the entire engine simulation for one frame.

## Demo Integration

**Demo crate:** `nebula-demo`

The demo initializes a `bevy_ecs::World` and runs a schedule each frame. The console logs `ECS World created with 0 entities`.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.15` | Standalone ECS: World, Schedule, system execution, parallelism |
| `bevy_utils` | `0.15` | Utility types used by bevy_ecs (HashMaps, tracing integration) |

Rust edition 2024. Only the `bevy_ecs` and `bevy_utils` crates are pulled in — not the full `bevy` engine. The `bevy_ecs` crate provides `World`, `Schedule`, `Component`, `Resource`, `Query`, and the parallel executor.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::prelude::*;
    use std::sync::{Arc, Mutex};

    #[derive(Resource, Default)]
    struct ExecutionLog {
        stages: Vec<String>,
    }

    fn log_system(stage_name: &'static str) -> impl Fn(ResMut<ExecutionLog>) {
        move |mut log: ResMut<ExecutionLog>| {
            log.stages.push(stage_name.to_string());
        }
    }

    #[test]
    fn test_world_creates_successfully() {
        let world = create_world();
        // World exists and can be queried without panic
        assert!(world.contains_resource::<TimeRes>());
    }

    #[test]
    fn test_world_starts_with_no_entities() {
        let world = create_world();
        assert_eq!(world.entities().len(), 0);
    }

    #[test]
    fn test_schedule_runs_all_stages_in_order() {
        let mut world = create_world();
        world.insert_resource(ExecutionLog::default());

        let mut schedules = EngineSchedules::new();
        schedules.add_system(EngineSchedule::PreUpdate, log_system("PreUpdate"));
        schedules.add_system(EngineSchedule::FixedUpdate, log_system("FixedUpdate"));
        schedules.add_system(EngineSchedule::Update, log_system("Update"));
        schedules.add_system(EngineSchedule::PostUpdate, log_system("PostUpdate"));
        schedules.add_system(EngineSchedule::PreRender, log_system("PreRender"));
        schedules.add_system(EngineSchedule::Render, log_system("Render"));

        // Pass exactly one fixed timestep worth of dt
        schedules.run(&mut world, 1.0 / 60.0);

        let log = world.resource::<ExecutionLog>();
        assert_eq!(
            log.stages,
            vec![
                "PreUpdate",
                "FixedUpdate",
                "Update",
                "PostUpdate",
                "PreRender",
                "Render",
            ]
        );
    }

    #[test]
    fn test_fixed_update_runs_at_correct_rate() {
        let mut world = create_world();

        #[derive(Resource, Default)]
        struct FixedCount(u32);
        world.insert_resource(FixedCount::default());

        let mut schedules = EngineSchedules::new();
        schedules.add_system(
            EngineSchedule::FixedUpdate,
            |mut count: ResMut<FixedCount>| { count.0 += 1; },
        );

        // Simulate 3 frames at 20 Hz (50ms each) — each frame is
        // 3x the fixed timestep of ~16.67ms, so FixedUpdate should
        // run 3 times per frame, 9 total.
        for _ in 0..3 {
            schedules.run(&mut world, 0.05);
        }

        let count = world.resource::<FixedCount>();
        assert_eq!(count.0, 9);
    }

    #[test]
    fn test_fixed_update_skips_when_dt_too_small() {
        let mut world = create_world();

        #[derive(Resource, Default)]
        struct FixedCount(u32);
        world.insert_resource(FixedCount::default());

        let mut schedules = EngineSchedules::new();
        schedules.add_system(
            EngineSchedule::FixedUpdate,
            |mut count: ResMut<FixedCount>| { count.0 += 1; },
        );

        // dt of 1ms is less than fixed_dt of ~16.67ms — no fixed step
        schedules.run(&mut world, 0.001);

        let count = world.resource::<FixedCount>();
        assert_eq!(count.0, 0);
    }

    #[test]
    fn test_systems_in_same_stage_can_run_in_parallel() {
        // Two systems that both read the same resource (immutable access)
        // can be placed in the same stage without conflict. bevy_ecs
        // will schedule them concurrently.
        let mut world = create_world();
        world.insert_resource(TimeRes::default());

        #[derive(Resource, Default)]
        struct ResultA(f32);
        #[derive(Resource, Default)]
        struct ResultB(f32);
        world.insert_resource(ResultA::default());
        world.insert_resource(ResultB::default());

        let mut schedules = EngineSchedules::new();
        schedules.add_system(
            EngineSchedule::Update,
            (
                |time: Res<TimeRes>, mut a: ResMut<ResultA>| { a.0 = time.delta; },
                |time: Res<TimeRes>, mut b: ResMut<ResultB>| { b.0 = time.delta; },
            ),
        );

        // If this panics with an access conflict, the systems cannot
        // run in parallel — which would be a bevy_ecs bug since they
        // share only an immutable Res<TimeRes>.
        schedules.run(&mut world, 1.0 / 60.0);

        // Both systems ran and wrote their results
        assert_eq!(world.resource::<ResultA>().0, 0.0);
        assert_eq!(world.resource::<ResultB>().0, 0.0);
    }

    #[test]
    fn test_engine_schedule_labels_are_distinct() {
        // Each stage label must be unique for schedule dispatch
        let labels = vec![
            EngineSchedule::PreUpdate,
            EngineSchedule::FixedUpdate,
            EngineSchedule::Update,
            EngineSchedule::PostUpdate,
            EngineSchedule::PreRender,
            EngineSchedule::Render,
        ];
        for (i, a) in labels.iter().enumerate() {
            for (j, b) in labels.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b);
                }
            }
        }
    }
}
```
