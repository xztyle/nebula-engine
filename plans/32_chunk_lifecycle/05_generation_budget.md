# Generation Budget

## Problem

Chunk generation and meshing are CPU-intensive operations. During initial world load or rapid player movement, hundreds of chunks may need to be generated and meshed simultaneously. If the engine processes all pending chunks in a single frame, the main thread stalls — the frame takes 200ms instead of 16ms, producing a visible hitch and unresponsive controls. Even with async generation on worker threads, the main thread must still snapshot chunk neighborhoods, submit tasks, and upload completed meshes to the GPU. These coordination costs add up. The engine needs a per-frame time budget that limits how much work is done, spreading the load across multiple frames to maintain smooth frame rates.

## Solution

Implement a frame budget system in the `nebula_chunk` crate that tracks elapsed time per frame and defers excess work to subsequent frames. Separate budgets for generation and meshing allow independent tuning.

### Budget Configuration

```rust
use std::time::{Duration, Instant};
use serde::{Deserialize, Serialize};

/// Configuration for per-frame chunk processing budgets.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChunkBudgetConfig {
    /// Maximum time to spend on chunk generation tasks per frame.
    pub generation_budget: Duration,
    /// Maximum time to spend on chunk meshing tasks per frame.
    pub meshing_budget: Duration,
    /// Multiplier applied to both budgets when no chunks are currently visible.
    /// This allows faster initial loading when the screen would be empty anyway.
    pub emergency_multiplier: f32,
}

impl Default for ChunkBudgetConfig {
    fn default() -> Self {
        Self {
            generation_budget: Duration::from_millis(4),
            meshing_budget: Duration::from_millis(3),
            emergency_multiplier: 4.0,
        }
    }
}

impl ChunkBudgetConfig {
    pub fn with_budgets(generation_ms: u64, meshing_ms: u64) -> Self {
        Self {
            generation_budget: Duration::from_millis(generation_ms),
            meshing_budget: Duration::from_millis(meshing_ms),
            ..Default::default()
        }
    }
}
```

### Frame Budget Tracker

```rust
/// Tracks time spent during the current frame for a single budget category.
#[derive(Debug)]
pub struct FrameBudget {
    budget: Duration,
    elapsed: Duration,
    start: Option<Instant>,
    items_processed: u32,
    items_deferred: u32,
}

impl FrameBudget {
    pub fn new(budget: Duration) -> Self {
        Self {
            budget,
            elapsed: Duration::ZERO,
            start: None,
            items_processed: 0,
            items_deferred: 0,
        }
    }

    /// Set the budget (used when switching between normal and emergency mode).
    pub fn set_budget(&mut self, budget: Duration) {
        self.budget = budget;
    }

    /// Returns true if there is remaining budget this frame.
    pub fn has_remaining(&self) -> bool {
        self.elapsed < self.budget
    }

    /// Returns the remaining budget duration.
    pub fn remaining(&self) -> Duration {
        self.budget.saturating_sub(self.elapsed)
    }

    /// Begin timing an operation.
    pub fn begin_item(&mut self) {
        self.start = Some(Instant::now());
    }

    /// End timing an operation and accumulate the elapsed time.
    pub fn end_item(&mut self) {
        if let Some(start) = self.start.take() {
            self.elapsed += start.elapsed();
            self.items_processed += 1;
        }
    }

    /// Record that an item was deferred due to budget exhaustion.
    pub fn record_deferred(&mut self) {
        self.items_deferred += 1;
    }

    /// Reset for a new frame.
    pub fn reset(&mut self) {
        self.elapsed = Duration::ZERO;
        self.start = None;
        self.items_processed = 0;
        self.items_deferred = 0;
    }

    pub fn items_processed(&self) -> u32 {
        self.items_processed
    }

    pub fn items_deferred(&self) -> u32 {
        self.items_deferred
    }

    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }
}
```

### Budgeted Processing Loop

The generation and meshing systems use the budget tracker to limit work per frame:

```rust
use bevy_ecs::prelude::*;

#[derive(Resource)]
pub struct ChunkBudgets {
    pub config: ChunkBudgetConfig,
    pub generation: FrameBudget,
    pub meshing: FrameBudget,
}

impl ChunkBudgets {
    pub fn new(config: ChunkBudgetConfig) -> Self {
        let generation = FrameBudget::new(config.generation_budget);
        let meshing = FrameBudget::new(config.meshing_budget);
        Self { config, generation, meshing }
    }

    /// Call at the start of each frame to reset counters and apply
    /// emergency multiplier if needed.
    pub fn begin_frame(&mut self, any_chunks_visible: bool) {
        self.generation.reset();
        self.meshing.reset();

        if any_chunks_visible {
            self.generation.set_budget(self.config.generation_budget);
            self.meshing.set_budget(self.config.meshing_budget);
        } else {
            let mult = self.config.emergency_multiplier;
            self.generation.set_budget(
                self.config.generation_budget.mul_f32(mult),
            );
            self.meshing.set_budget(
                self.config.meshing_budget.mul_f32(mult),
            );
        }
    }
}

fn budgeted_generation_system(
    mut budgets: ResMut<ChunkBudgets>,
    mut load_queue: ResMut<ChunkLoadQueue>,
    // ... other params
) {
    while budgets.generation.has_remaining() {
        let Some(next_chunk) = load_queue.dequeue() else { break };

        budgets.generation.begin_item();
        // Submit generation task for next_chunk (snapshot + send to worker)
        submit_generation_task(next_chunk);
        budgets.generation.end_item();

        if !budgets.generation.has_remaining() {
            // Any remaining chunks stay in the queue for next frame
            budgets.generation.record_deferred();
        }
    }
}
```

### Diagnostics

The budget tracker exposes per-frame statistics via a diagnostic resource:

```rust
#[derive(Debug, Default)]
pub struct ChunkBudgetDiagnostics {
    pub generation_processed: u32,
    pub generation_deferred: u32,
    pub generation_elapsed: Duration,
    pub meshing_processed: u32,
    pub meshing_deferred: u32,
    pub meshing_elapsed: Duration,
}
```

These diagnostics can be displayed in the debug overlay (Epic 28) to help tune budget values.

## Outcome

The `nebula_chunk` crate exports `ChunkBudgetConfig`, `FrameBudget`, `ChunkBudgets`, and `ChunkBudgetDiagnostics`. Generation and meshing are time-budgeted per frame, preventing main-thread stalls. The emergency multiplier ensures fast initial loading when the screen is empty. Running `cargo test -p nebula_chunk` passes all budget tests.

## Demo Integration

**Demo crate:** `nebula-demo`

At most 4 chunks are generated per frame to prevent frame hitching. If 100 chunks need generation, they are spread across 25 frames for stable performance.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.18` | `Resource` derive for `ChunkBudgets`, ECS system integration |
| `serde` | `1.0` | Serialize/deserialize `ChunkBudgetConfig` for settings files |

The budget tracker uses `std::time::Instant` and `std::time::Duration` from the standard library. No external timing crate is needed. The crate uses Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use std::thread;

    /// The budget should limit the number of items processed.
    #[test]
    fn test_budget_limits_generation_count() {
        // Create a very small budget (1ms)
        let mut budget = FrameBudget::new(Duration::from_millis(1));

        let mut count = 0;
        while budget.has_remaining() && count < 1000 {
            budget.begin_item();
            // Simulate work that takes ~200us
            thread::sleep(Duration::from_micros(200));
            budget.end_item();
            count += 1;
        }

        // Should have processed some but not all 1000
        assert!(budget.items_processed() > 0, "should process at least one item");
        assert!(budget.items_processed() < 1000, "should not process all items");
        assert!(!budget.has_remaining(), "budget should be exhausted");
    }

    /// Excess chunks should be deferred and tracked.
    #[test]
    fn test_excess_chunks_deferred() {
        let mut budget = FrameBudget::new(Duration::from_millis(1));

        // Exhaust the budget
        budget.begin_item();
        thread::sleep(Duration::from_millis(2));
        budget.end_item();

        // Now record deferrals
        budget.record_deferred();
        budget.record_deferred();
        budget.record_deferred();

        assert_eq!(budget.items_processed(), 1);
        assert_eq!(budget.items_deferred(), 3);
    }

    /// Generation and meshing budgets operate independently.
    #[test]
    fn test_separate_budgets_work_independently() {
        let config = ChunkBudgetConfig::with_budgets(2, 5);
        let mut budgets = ChunkBudgets::new(config);
        budgets.begin_frame(true);

        // Exhaust generation budget
        budgets.generation.begin_item();
        thread::sleep(Duration::from_millis(3));
        budgets.generation.end_item();

        // Generation is exhausted, but meshing should still have budget
        assert!(!budgets.generation.has_remaining());
        assert!(budgets.meshing.has_remaining());

        // Meshing can still process
        budgets.meshing.begin_item();
        thread::sleep(Duration::from_micros(100));
        budgets.meshing.end_item();
        assert_eq!(budgets.meshing.items_processed(), 1);
    }

    /// When no chunks are visible, the emergency multiplier increases the budget.
    #[test]
    fn test_emergency_budget_increases_when_nothing_visible() {
        let config = ChunkBudgetConfig {
            generation_budget: Duration::from_millis(4),
            meshing_budget: Duration::from_millis(3),
            emergency_multiplier: 4.0,
        };

        let mut budgets_normal = ChunkBudgets::new(config.clone());
        budgets_normal.begin_frame(true); // chunks visible

        let mut budgets_emergency = ChunkBudgets::new(config);
        budgets_emergency.begin_frame(false); // no chunks visible

        // Emergency budget should be 4x the normal budget
        let normal_remaining = budgets_normal.generation.remaining();
        let emergency_remaining = budgets_emergency.generation.remaining();

        assert!(
            emergency_remaining > normal_remaining,
            "emergency ({emergency_remaining:?}) should be > normal ({normal_remaining:?})"
        );

        // Specifically, 4ms * 4.0 = 16ms
        assert_eq!(emergency_remaining, Duration::from_millis(16));
    }

    /// Budget values should be configurable.
    #[test]
    fn test_budget_is_configurable() {
        let config = ChunkBudgetConfig::with_budgets(10, 8);
        assert_eq!(config.generation_budget, Duration::from_millis(10));
        assert_eq!(config.meshing_budget, Duration::from_millis(8));

        let mut budgets = ChunkBudgets::new(config);
        budgets.begin_frame(true);
        assert_eq!(budgets.generation.remaining(), Duration::from_millis(10));
        assert_eq!(budgets.meshing.remaining(), Duration::from_millis(8));
    }

    /// Resetting the budget should clear all counters.
    #[test]
    fn test_budget_reset_clears_counters() {
        let mut budget = FrameBudget::new(Duration::from_millis(5));

        budget.begin_item();
        thread::sleep(Duration::from_millis(1));
        budget.end_item();
        budget.record_deferred();

        assert!(budget.items_processed() > 0);
        assert!(budget.items_deferred() > 0);
        assert!(budget.elapsed() > Duration::ZERO);

        budget.reset();

        assert_eq!(budget.items_processed(), 0);
        assert_eq!(budget.items_deferred(), 0);
        assert_eq!(budget.elapsed(), Duration::ZERO);
        assert!(budget.has_remaining());
    }
}
```
