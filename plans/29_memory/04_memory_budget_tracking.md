# Memory Budget Tracking

## Problem

A voxel engine with cubesphere planets, physics, audio, entity systems, and GPU rendering consumes memory across many independent subsystems. Without centralized tracking, individual subsystems allocate freely until the process hits its OS memory limit and is killed, or until GPU memory is exhausted and `wgpu` returns allocation errors. By that point, it is too late to recover gracefully.

The engine needs a budget system: each subsystem is assigned a maximum memory allowance, and allocations that would exceed the budget are refused (returning an error) rather than proceeding and risking a crash. This enables the engine to make intelligent decisions when memory is tight -- for example, reducing view distance to shed chunk data, lowering texture resolution, or evicting LRU cache entries -- rather than blindly consuming resources until the system fails.

Budget tracking also provides visibility. A debug overlay can display "Chunk Data: 1.4 GB / 2.0 GB (70%)" so that developers can immediately see which subsystem is consuming the most memory and whether the engine is approaching its limits.

## Solution

Implement a `MemoryBudget` system in the `nebula_memory` crate that tracks memory usage per subsystem and enforces configurable limits.

### Subsystem Identifiers

```rust
/// Identifies a memory subsystem in the engine.
///
/// Each subsystem has an independent memory budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MemorySubsystem {
    /// Voxel chunk data (palettes, index arrays, metadata).
    ChunkData,
    /// GPU buffers for chunk meshes (vertex + index buffers).
    MeshGpu,
    /// GPU textures (terrain atlas, skybox, UI textures).
    Textures,
    /// Audio buffers (loaded sound effects and music).
    Audio,
    /// Physics simulation data (collision shapes, broadphase, contact points).
    Physics,
    /// ECS entity and component storage.
    Entities,
}

impl MemorySubsystem {
    /// All subsystem variants, for iteration.
    pub const ALL: &'static [MemorySubsystem] = &[
        Self::ChunkData,
        Self::MeshGpu,
        Self::Textures,
        Self::Audio,
        Self::Physics,
        Self::Entities,
    ];
}
```

### SubsystemBudget

```rust
/// Tracks memory usage for a single subsystem.
#[derive(Clone, Debug)]
pub struct SubsystemBudget {
    /// Maximum allowed memory in bytes.
    pub limit: u64,
    /// Current memory in use in bytes.
    pub used: u64,
    /// Peak memory ever used in bytes (high watermark).
    pub peak: u64,
}

impl SubsystemBudget {
    pub fn new(limit: u64) -> Self {
        Self {
            limit,
            used: 0,
            peak: 0,
        }
    }

    /// Current usage as a fraction of the limit (0.0 to 1.0+).
    pub fn usage_fraction(&self) -> f64 {
        if self.limit == 0 {
            return 0.0;
        }
        self.used as f64 / self.limit as f64
    }

    /// Current usage as a percentage (0.0 to 100.0+).
    pub fn usage_percent(&self) -> f64 {
        self.usage_fraction() * 100.0
    }

    /// Remaining bytes before the budget is exceeded.
    pub fn remaining(&self) -> u64 {
        self.limit.saturating_sub(self.used)
    }

    /// Whether the subsystem is currently over budget.
    pub fn is_over_budget(&self) -> bool {
        self.used > self.limit
    }
}
```

### MemoryBudget

```rust
use std::collections::HashMap;

/// Budget allocation error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BudgetError {
    /// The requested allocation would exceed the subsystem's budget.
    OverBudget {
        subsystem: MemorySubsystem,
        requested: u64,
        available: u64,
        limit: u64,
    },
    /// The subsystem is not registered (should not happen with the enum approach,
    /// but included for defensive programming).
    UnknownSubsystem,
}

impl std::fmt::Display for BudgetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BudgetError::OverBudget {
                subsystem,
                requested,
                available,
                limit,
            } => write!(
                f,
                "Memory budget exceeded for {subsystem:?}: \
                 requested {requested} bytes, {available} available (limit: {limit})"
            ),
            BudgetError::UnknownSubsystem => write!(f, "Unknown memory subsystem"),
        }
    }
}

impl std::error::Error for BudgetError {}

/// Central memory budget tracker for all engine subsystems.
///
/// Stored as a Bevy ECS resource. Systems call `allocate()` before allocating
/// memory and `free()` when releasing it.
pub struct MemoryBudget {
    subsystems: HashMap<MemorySubsystem, SubsystemBudget>,
}

impl MemoryBudget {
    /// Create a new budget tracker with default limits.
    ///
    /// Default budgets:
    /// - ChunkData:  2 GB
    /// - MeshGpu:    1 GB
    /// - Textures:   512 MB
    /// - Audio:      256 MB
    /// - Physics:    256 MB
    /// - Entities:   256 MB
    pub fn with_defaults() -> Self {
        let gb = 1024 * 1024 * 1024;
        let mb = 1024 * 1024;

        let mut subsystems = HashMap::new();
        subsystems.insert(MemorySubsystem::ChunkData, SubsystemBudget::new(2 * gb));
        subsystems.insert(MemorySubsystem::MeshGpu, SubsystemBudget::new(1 * gb));
        subsystems.insert(MemorySubsystem::Textures, SubsystemBudget::new(512 * mb));
        subsystems.insert(MemorySubsystem::Audio, SubsystemBudget::new(256 * mb));
        subsystems.insert(MemorySubsystem::Physics, SubsystemBudget::new(256 * mb));
        subsystems.insert(MemorySubsystem::Entities, SubsystemBudget::new(256 * mb));

        Self { subsystems }
    }

    /// Create a budget tracker with custom limits.
    pub fn with_limits(limits: &[(MemorySubsystem, u64)]) -> Self {
        let mut subsystems = HashMap::new();
        for &(subsystem, limit) in limits {
            subsystems.insert(subsystem, SubsystemBudget::new(limit));
        }
        Self { subsystems }
    }

    /// Set or update the budget limit for a subsystem.
    pub fn set_limit(&mut self, subsystem: MemorySubsystem, limit: u64) {
        self.subsystems
            .entry(subsystem)
            .and_modify(|b| b.limit = limit)
            .or_insert_with(|| SubsystemBudget::new(limit));
    }

    /// Request an allocation of `bytes` from the given subsystem's budget.
    ///
    /// Returns `Ok(())` if the allocation fits within the budget.
    /// Returns `Err(BudgetError::OverBudget)` if it would exceed the limit.
    ///
    /// This does NOT perform the actual allocation -- it only updates the
    /// accounting. The caller is responsible for the actual memory allocation.
    pub fn allocate(
        &mut self,
        subsystem: MemorySubsystem,
        bytes: u64,
    ) -> Result<(), BudgetError> {
        let budget = self
            .subsystems
            .get_mut(&subsystem)
            .ok_or(BudgetError::UnknownSubsystem)?;

        if budget.used + bytes > budget.limit {
            return Err(BudgetError::OverBudget {
                subsystem,
                requested: bytes,
                available: budget.remaining(),
                limit: budget.limit,
            });
        }

        budget.used += bytes;
        if budget.used > budget.peak {
            budget.peak = budget.used;
        }

        Ok(())
    }

    /// Record that `bytes` of memory have been freed from the given subsystem.
    ///
    /// # Panics
    /// Debug-asserts that `bytes <= budget.used` (freeing more than allocated
    /// indicates a bookkeeping bug).
    pub fn free(&mut self, subsystem: MemorySubsystem, bytes: u64) {
        if let Some(budget) = self.subsystems.get_mut(&subsystem) {
            debug_assert!(
                bytes <= budget.used,
                "freeing {bytes} bytes from {subsystem:?} but only {} are allocated",
                budget.used
            );
            budget.used = budget.used.saturating_sub(bytes);
        }
    }

    /// Get the budget status for a subsystem.
    pub fn get(&self, subsystem: MemorySubsystem) -> Option<&SubsystemBudget> {
        self.subsystems.get(&subsystem)
    }

    /// Total memory in use across all subsystems.
    pub fn total_used(&self) -> u64 {
        self.subsystems.values().map(|b| b.used).sum()
    }

    /// Total budget limit across all subsystems.
    pub fn total_limit(&self) -> u64 {
        self.subsystems.values().map(|b| b.limit).sum()
    }

    /// Generate a report of all subsystem budgets (for debug overlay).
    pub fn report(&self) -> Vec<BudgetReport> {
        let mut reports: Vec<_> = self
            .subsystems
            .iter()
            .map(|(&subsystem, budget)| BudgetReport {
                subsystem,
                used: budget.used,
                limit: budget.limit,
                peak: budget.peak,
                percent: budget.usage_percent(),
            })
            .collect();
        reports.sort_by_key(|r| std::cmp::Reverse(r.used));
        reports
    }
}

/// A snapshot of a subsystem's memory budget status.
#[derive(Clone, Debug)]
pub struct BudgetReport {
    pub subsystem: MemorySubsystem,
    pub used: u64,
    pub limit: u64,
    pub peak: u64,
    pub percent: f64,
}

impl std::fmt::Display for BudgetReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let used_mb = self.used as f64 / (1024.0 * 1024.0);
        let limit_mb = self.limit as f64 / (1024.0 * 1024.0);
        write!(
            f,
            "{:?}: {used_mb:.1} MB / {limit_mb:.1} MB ({:.1}%)",
            self.subsystem, self.percent
        )
    }
}
```

### ECS Integration

```rust
/// Register the memory budget as a Bevy ECS resource.
app.insert_resource(MemoryBudget::with_defaults());

/// Example: chunk loading system checks the budget before loading a chunk.
fn load_chunk_system(
    mut budget: ResMut<MemoryBudget>,
    mut chunk_manager: ResMut<ChunkManager>,
    load_queue: Res<ChunkLoadQueue>,
) {
    for addr in load_queue.iter() {
        let chunk_size = std::mem::size_of::<Chunk>() as u64 + CHUNK_DATA_ESTIMATE;

        match budget.allocate(MemorySubsystem::ChunkData, chunk_size) {
            Ok(()) => {
                let chunk = chunk_manager.acquire_chunk();
                // ... load chunk data ...
                chunk_manager.load_chunk(*addr, chunk);
            }
            Err(BudgetError::OverBudget { .. }) => {
                // Budget exceeded -- skip loading this chunk.
                // The LRU eviction system (story 05) will free some memory.
                tracing::warn!("Chunk data budget exceeded, skipping load of {addr:?}");
                break;
            }
            Err(_) => unreachable!(),
        }
    }
}
```

## Outcome

The `nebula_memory` crate exports `MemoryBudget`, `MemorySubsystem`, `SubsystemBudget`, `BudgetError`, and `BudgetReport`. All subsystems that allocate significant memory call `budget.allocate()` before allocating and `budget.free()` when releasing. The debug overlay displays per-subsystem memory usage via `budget.report()`. Running `cargo test -p nebula_memory` passes all budget tests. The crate uses Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

The memory debug panel shows budgets being enforced: `Chunks: 180/256 MB`, `GPU: 95/128 MB`. Approaching the budget triggers visible warnings.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.15` | ECS resource storage and system access (workspace dependency) |
| `tracing` | `0.1` | Warning logs when budget is exceeded |

No external memory tracking crates are required. The budget system is a lightweight accounting layer, not an allocator.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Allocating bytes should increase the subsystem's usage counter.
    #[test]
    fn test_allocation_increases_usage() {
        let mut budget = MemoryBudget::with_limits(&[
            (MemorySubsystem::ChunkData, 1024 * 1024), // 1 MB limit
        ]);

        let before = budget.get(MemorySubsystem::ChunkData).unwrap().used;
        budget.allocate(MemorySubsystem::ChunkData, 1000).unwrap();
        let after = budget.get(MemorySubsystem::ChunkData).unwrap().used;

        assert_eq!(after - before, 1000);
        assert_eq!(after, 1000);
    }

    /// Freeing bytes should decrease the subsystem's usage counter.
    #[test]
    fn test_free_decreases_usage() {
        let mut budget = MemoryBudget::with_limits(&[
            (MemorySubsystem::MeshGpu, 1024 * 1024),
        ]);

        budget.allocate(MemorySubsystem::MeshGpu, 5000).unwrap();
        assert_eq!(budget.get(MemorySubsystem::MeshGpu).unwrap().used, 5000);

        budget.free(MemorySubsystem::MeshGpu, 3000);
        assert_eq!(budget.get(MemorySubsystem::MeshGpu).unwrap().used, 2000);

        budget.free(MemorySubsystem::MeshGpu, 2000);
        assert_eq!(budget.get(MemorySubsystem::MeshGpu).unwrap().used, 0);
    }

    /// An allocation that would exceed the budget should return OverBudget error.
    #[test]
    fn test_over_budget_allocation_returns_error() {
        let mut budget = MemoryBudget::with_limits(&[
            (MemorySubsystem::Audio, 1000), // tiny budget for testing
        ]);

        // First allocation fits.
        assert!(budget.allocate(MemorySubsystem::Audio, 800).is_ok());

        // Second allocation would exceed the 1000-byte limit (800 + 300 = 1100).
        let result = budget.allocate(MemorySubsystem::Audio, 300);
        assert!(result.is_err());

        match result {
            Err(BudgetError::OverBudget {
                subsystem,
                requested,
                available,
                limit,
            }) => {
                assert_eq!(subsystem, MemorySubsystem::Audio);
                assert_eq!(requested, 300);
                assert_eq!(available, 200);
                assert_eq!(limit, 1000);
            }
            _ => panic!("expected OverBudget error"),
        }

        // Usage should not have changed (failed allocation is not counted).
        assert_eq!(budget.get(MemorySubsystem::Audio).unwrap().used, 800);
    }

    /// Usage percentage should be calculated correctly.
    #[test]
    fn test_usage_percentage_is_correct() {
        let mut budget = MemoryBudget::with_limits(&[
            (MemorySubsystem::Textures, 1000),
        ]);

        budget.allocate(MemorySubsystem::Textures, 250).unwrap();
        let pct = budget.get(MemorySubsystem::Textures).unwrap().usage_percent();
        assert!(
            (pct - 25.0).abs() < 0.001,
            "expected 25%, got {pct}%"
        );

        budget.allocate(MemorySubsystem::Textures, 250).unwrap();
        let pct = budget.get(MemorySubsystem::Textures).unwrap().usage_percent();
        assert!(
            (pct - 50.0).abs() < 0.001,
            "expected 50%, got {pct}%"
        );

        budget.allocate(MemorySubsystem::Textures, 500).unwrap();
        let pct = budget.get(MemorySubsystem::Textures).unwrap().usage_percent();
        assert!(
            (pct - 100.0).abs() < 0.001,
            "expected 100%, got {pct}%"
        );
    }

    /// Multiple subsystems should be tracked independently.
    #[test]
    fn test_multiple_subsystems_tracked_independently() {
        let mut budget = MemoryBudget::with_limits(&[
            (MemorySubsystem::ChunkData, 10_000),
            (MemorySubsystem::MeshGpu, 5_000),
            (MemorySubsystem::Physics, 3_000),
        ]);

        budget.allocate(MemorySubsystem::ChunkData, 7000).unwrap();
        budget.allocate(MemorySubsystem::MeshGpu, 2000).unwrap();
        budget.allocate(MemorySubsystem::Physics, 1500).unwrap();

        assert_eq!(budget.get(MemorySubsystem::ChunkData).unwrap().used, 7000);
        assert_eq!(budget.get(MemorySubsystem::MeshGpu).unwrap().used, 2000);
        assert_eq!(budget.get(MemorySubsystem::Physics).unwrap().used, 1500);

        // Freeing from one subsystem should not affect others.
        budget.free(MemorySubsystem::MeshGpu, 2000);
        assert_eq!(budget.get(MemorySubsystem::ChunkData).unwrap().used, 7000);
        assert_eq!(budget.get(MemorySubsystem::MeshGpu).unwrap().used, 0);
        assert_eq!(budget.get(MemorySubsystem::Physics).unwrap().used, 1500);

        // Total used should sum all subsystems.
        assert_eq!(budget.total_used(), 7000 + 0 + 1500);
    }

    /// The peak (high watermark) should track the maximum usage for each subsystem.
    #[test]
    fn test_peak_tracks_maximum_usage() {
        let mut budget = MemoryBudget::with_limits(&[
            (MemorySubsystem::Entities, 10_000),
        ]);

        budget.allocate(MemorySubsystem::Entities, 3000).unwrap();
        budget.allocate(MemorySubsystem::Entities, 4000).unwrap();
        assert_eq!(budget.get(MemorySubsystem::Entities).unwrap().peak, 7000);

        budget.free(MemorySubsystem::Entities, 5000);
        assert_eq!(
            budget.get(MemorySubsystem::Entities).unwrap().peak,
            7000,
            "peak should not decrease after free"
        );

        budget.allocate(MemorySubsystem::Entities, 1000).unwrap();
        assert_eq!(
            budget.get(MemorySubsystem::Entities).unwrap().peak,
            7000,
            "peak should not change if current usage is below previous peak"
        );
    }
}
```
