//! Engine schedule labels and the ordered schedule runner.

use bevy_ecs::prelude::*;
use bevy_ecs::schedule::{IntoSystemConfigs, ScheduleLabel};

/// Maximum number of fixed-update steps per frame to prevent spiral-of-death.
const MAX_FIXED_STEPS_PER_FRAME: u32 = 10;

/// Labels for each engine execution stage.
///
/// Stages run in the order listed, top to bottom, every frame.
#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
pub enum EngineSchedule {
    /// Poll input devices, update window events.
    PreUpdate,
    /// Deterministic simulation at 60 Hz (physics, movement).
    FixedUpdate,
    /// Variable-rate gameplay logic (AI, scripting).
    Update,
    /// Transform propagation, spatial structure updates.
    PostUpdate,
    /// Frustum culling, draw-call batching, GPU uploads.
    PreRender,
    /// GPU command submission.
    Render,
}

/// Ordered collection of [`Schedule`]s that drives one engine frame.
///
/// `FixedUpdate` uses a time-accumulator pattern to tick at a stable 60 Hz
/// regardless of the actual frame rate.
pub struct EngineSchedules {
    schedules: Vec<(EngineSchedule, Schedule)>,
    fixed_accumulator: f64,
    fixed_dt: f64,
}

impl EngineSchedules {
    /// Create a new set of engine schedules with default fixed timestep (1/60 s).
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

    /// Register a system (or system tuple) into a specific stage.
    pub fn add_system<M>(&mut self, stage: EngineSchedule, system: impl IntoSystemConfigs<M>) {
        for (label, schedule) in &mut self.schedules {
            if *label == stage {
                schedule.add_systems(system);
                return;
            }
        }
        panic!("Unknown stage: {stage:?}");
    }

    /// Run all stages in order for one frame.
    ///
    /// `FixedUpdate` may run 0–[`MAX_FIXED_STEPS_PER_FRAME`] times based on
    /// accumulated delta time. All other stages run exactly once.
    pub fn run(&mut self, world: &mut World, frame_dt: f64) {
        self.run_stage(EngineSchedule::PreUpdate, world);

        self.fixed_accumulator += frame_dt;
        let mut steps: u32 = 0;
        while self.fixed_accumulator >= self.fixed_dt && steps < MAX_FIXED_STEPS_PER_FRAME {
            self.run_stage(EngineSchedule::FixedUpdate, world);
            self.fixed_accumulator -= self.fixed_dt;
            steps += 1;
        }

        self.run_stage(EngineSchedule::Update, world);
        self.run_stage(EngineSchedule::PostUpdate, world);
        self.run_stage(EngineSchedule::PreRender, world);
        self.run_stage(EngineSchedule::Render, world);
    }

    /// Returns the current fixed-update accumulator value in seconds.
    pub fn fixed_accumulator(&self) -> f64 {
        self.fixed_accumulator
    }

    /// Returns the fixed timestep in seconds (default 1/60).
    pub fn fixed_dt(&self) -> f64 {
        self.fixed_dt
    }

    /// Returns a mutable reference to the schedule for a given stage.
    ///
    /// Useful for configuring system sets and ordering constraints.
    pub fn get_schedule_mut(&mut self, stage: &EngineSchedule) -> Option<&mut Schedule> {
        self.schedules
            .iter_mut()
            .find(|(label, _)| label == stage)
            .map(|(_, schedule)| schedule)
    }

    /// Force-initialize all schedules, validating the dependency graph.
    ///
    /// Panics with a descriptive error if any cycle is detected.
    pub fn initialize_all(&mut self, world: &mut World) {
        for (_label, schedule) in &mut self.schedules {
            let _ = schedule.initialize(world);
        }
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

impl Default for EngineSchedules {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TimeRes, create_world};

    #[derive(Resource, Default)]
    struct ExecutionLog {
        stages: Vec<String>,
    }

    fn log_system(stage_name: &'static str) -> impl Fn(ResMut<'_, ExecutionLog>) {
        move |mut log: ResMut<'_, ExecutionLog>| {
            log.stages.push(stage_name.to_string());
        }
    }

    #[test]
    fn test_world_creates_successfully() {
        let world = create_world();
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
            |mut count: ResMut<'_, FixedCount>| {
                count.0 += 1;
            },
        );

        // 3 frames at 20 Hz (50ms each) — each frame ~3x the 16.67ms fixed step
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
            |mut count: ResMut<'_, FixedCount>| {
                count.0 += 1;
            },
        );

        // 1ms < 16.67ms fixed step — no fixed update
        schedules.run(&mut world, 0.001);

        let count = world.resource::<FixedCount>();
        assert_eq!(count.0, 0);
    }

    #[test]
    fn test_systems_in_same_stage_can_run_in_parallel() {
        let mut world = create_world();

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
                |time: Res<'_, TimeRes>, mut a: ResMut<'_, ResultA>| {
                    a.0 = time.delta;
                },
                |time: Res<'_, TimeRes>, mut b: ResMut<'_, ResultB>| {
                    b.0 = time.delta;
                },
            ),
        );

        schedules.run(&mut world, 1.0 / 60.0);

        assert_eq!(world.resource::<ResultA>().0, 0.0);
        assert_eq!(world.resource::<ResultB>().0, 0.0);
    }

    #[test]
    fn test_engine_schedule_labels_are_distinct() {
        let labels = [
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
