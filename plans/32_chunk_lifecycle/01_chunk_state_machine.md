# Chunk State Machine

## Problem

A chunk in Nebula Engine passes through many stages during its lifetime: it must be scheduled for generation, generated on a background thread, meshed into renderable geometry, made active for gameplay, and eventually unloaded when the player moves away. Without an explicit state machine, the engine risks performing operations on chunks that are not ready (meshing a chunk that has not been generated, modifying a chunk that is still being meshed, or unloading a chunk mid-generation). These invalid operations lead to corrupted voxel data, rendering artifacts, crashes, and subtle race conditions in the async pipeline. The engine needs a well-defined set of lifecycle states with strictly enforced transitions so that every system can query a chunk's current state and be confident about what operations are legal.

## Solution

Define a `ChunkState` enum and a `ChunkStateMachine` struct in the `nebula_chunk` crate that tracks the current state of each chunk and validates every transition at runtime.

### State Definitions

```rust
/// Every state a chunk can occupy during its lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ChunkState {
    /// Not loaded in memory. The default/initial state.
    Unloaded,
    /// Queued in the priority loading system, awaiting a generation slot.
    Scheduled,
    /// A background task is actively generating voxel data.
    Generating,
    /// Voxel data is fully generated and resident in memory.
    Generated,
    /// A background task is actively building the render mesh.
    Meshing,
    /// The mesh is complete and ready for GPU upload.
    Meshed,
    /// Fully active: rendered, interactive, modifiable by gameplay systems.
    Active,
    /// Marked for removal; async save or cleanup may be in progress.
    Unloading,
}
```

### Allowed Transitions

The state machine enforces a directed graph of allowed transitions:

| From         | To           | Trigger                               |
|-------------|-------------|---------------------------------------|
| `Unloaded`   | `Scheduled`  | Chunk enters the load radius          |
| `Scheduled`  | `Generating` | A generation worker picks up the task |
| `Scheduled`  | `Unloaded`   | Cancelled before generation started   |
| `Generating` | `Generated`  | Generation task completes             |
| `Generating` | `Unloaded`   | Cancelled (cooperative cancellation)  |
| `Generated`  | `Meshing`    | All neighbors are at least Generated  |
| `Generated`  | `Unloading`  | Chunk exits load radius before mesh   |
| `Meshing`    | `Meshed`     | Meshing task completes                |
| `Meshing`    | `Unloading`  | Cancelled during meshing              |
| `Meshed`     | `Active`     | Mesh uploaded to GPU, chunk is live   |
| `Active`     | `Meshing`    | Voxel data modified, remesh needed    |
| `Active`     | `Unloading`  | Chunk exits unload radius             |
| `Unloading`  | `Unloaded`   | Save/cleanup complete                 |

### State Machine Implementation

```rust
use bevy_ecs::prelude::*;
use tracing::{error, trace};

/// Tracks the current lifecycle state of a single chunk.
#[derive(Component, Debug)]
pub struct ChunkStateMachine {
    state: ChunkState,
    /// Monotonic counter incremented on every successful transition.
    transition_count: u64,
}

impl ChunkStateMachine {
    pub fn new() -> Self {
        Self {
            state: ChunkState::Unloaded,
            transition_count: 0,
        }
    }

    pub fn state(&self) -> ChunkState {
        self.state
    }

    pub fn transition_count(&self) -> u64 {
        self.transition_count
    }

    /// Attempt a state transition. Returns `Ok(())` on success, or
    /// `Err(InvalidTransition)` if the transition is not allowed.
    pub fn transition(&mut self, target: ChunkState) -> Result<(), InvalidTransition> {
        if Self::is_valid_transition(self.state, target) {
            trace!(from = ?self.state, to = ?target, "chunk state transition");
            self.state = target;
            self.transition_count += 1;
            Ok(())
        } else {
            error!(from = ?self.state, to = ?target, "invalid chunk state transition");
            Err(InvalidTransition {
                from: self.state,
                to: target,
            })
        }
    }

    fn is_valid_transition(from: ChunkState, to: ChunkState) -> bool {
        matches!(
            (from, to),
            (ChunkState::Unloaded, ChunkState::Scheduled)
                | (ChunkState::Scheduled, ChunkState::Generating)
                | (ChunkState::Scheduled, ChunkState::Unloaded)
                | (ChunkState::Generating, ChunkState::Generated)
                | (ChunkState::Generating, ChunkState::Unloaded)
                | (ChunkState::Generated, ChunkState::Meshing)
                | (ChunkState::Generated, ChunkState::Unloading)
                | (ChunkState::Meshing, ChunkState::Meshed)
                | (ChunkState::Meshing, ChunkState::Unloading)
                | (ChunkState::Meshed, ChunkState::Active)
                | (ChunkState::Active, ChunkState::Meshing)
                | (ChunkState::Active, ChunkState::Unloading)
                | (ChunkState::Unloading, ChunkState::Unloaded)
        )
    }

    /// Returns true if the chunk can accept voxel modifications.
    pub fn is_modifiable(&self) -> bool {
        self.state == ChunkState::Active
    }

    /// Returns true if the chunk is eligible to begin meshing.
    pub fn is_meshable(&self) -> bool {
        self.state == ChunkState::Generated
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidTransition {
    pub from: ChunkState,
    pub to: ChunkState,
}

impl std::fmt::Display for InvalidTransition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid chunk transition: {:?} -> {:?}", self.from, self.to)
    }
}

impl std::error::Error for InvalidTransition {}
```

### State-Gated Operations

Systems throughout the engine query the state machine before acting. For example, the meshing system checks `is_meshable()` before submitting a meshing task. The voxel editing system checks `is_modifiable()` before applying player edits. The renderer only draws chunks in the `Active` state.

### Bevy ECS Integration

The `ChunkStateMachine` is attached as a Bevy ECS `Component` on the chunk entity. Systems query it with standard Bevy queries:

```rust
fn meshing_system(query: Query<(Entity, &ChunkStateMachine, &ChunkAddress)>) {
    for (entity, state_machine, address) in &query {
        if state_machine.is_meshable() {
            // Submit meshing task
        }
    }
}
```

## Outcome

The `nebula_chunk` crate exports `ChunkState`, `ChunkStateMachine`, and `InvalidTransition`. Every chunk entity in the ECS world carries a `ChunkStateMachine` component. All lifecycle-dependent systems query the state before operating, and invalid transitions are caught at runtime with error-level logging. Running `cargo test -p nebula_chunk` passes all state machine tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Each chunk transitions through well-defined states: Unloaded → Loading → Generating → Meshing → Active → Unloading. The debug overlay colors chunks by current state.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.18` | `Component` derive, ECS integration for chunk entities |
| `serde` | `1.0` | Serialize/deserialize `ChunkState` for diagnostics and persistence |
| `tracing` | `0.1` | Structured logging for transitions and errors |

The state machine itself is pure logic with no allocations beyond the struct. The crate uses Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// A new state machine should begin in the Unloaded state.
    #[test]
    fn test_initial_state_is_unloaded() {
        let sm = ChunkStateMachine::new();
        assert_eq!(sm.state(), ChunkState::Unloaded);
        assert_eq!(sm.transition_count(), 0);
    }

    /// A valid transition (Unloaded -> Scheduled) should succeed.
    #[test]
    fn test_valid_transition_succeeds() {
        let mut sm = ChunkStateMachine::new();
        let result = sm.transition(ChunkState::Scheduled);
        assert!(result.is_ok());
        assert_eq!(sm.state(), ChunkState::Scheduled);
        assert_eq!(sm.transition_count(), 1);
    }

    /// An invalid transition (Unloaded -> Active) should return an error.
    #[test]
    fn test_invalid_transition_is_rejected() {
        let mut sm = ChunkStateMachine::new();
        let result = sm.transition(ChunkState::Active);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.from, ChunkState::Unloaded);
        assert_eq!(err.to, ChunkState::Active);
        // State should not have changed
        assert_eq!(sm.state(), ChunkState::Unloaded);
        assert_eq!(sm.transition_count(), 0);
    }

    /// The current state can be queried at any time.
    #[test]
    fn test_state_can_be_queried() {
        let mut sm = ChunkStateMachine::new();
        assert_eq!(sm.state(), ChunkState::Unloaded);
        sm.transition(ChunkState::Scheduled).unwrap();
        assert_eq!(sm.state(), ChunkState::Scheduled);
        sm.transition(ChunkState::Generating).unwrap();
        assert_eq!(sm.state(), ChunkState::Generating);
    }

    /// The full happy-path lifecycle should work end to end.
    #[test]
    fn test_full_lifecycle_path() {
        let mut sm = ChunkStateMachine::new();
        sm.transition(ChunkState::Scheduled).unwrap();
        sm.transition(ChunkState::Generating).unwrap();
        sm.transition(ChunkState::Generated).unwrap();
        sm.transition(ChunkState::Meshing).unwrap();
        sm.transition(ChunkState::Meshed).unwrap();
        sm.transition(ChunkState::Active).unwrap();
        sm.transition(ChunkState::Unloading).unwrap();
        sm.transition(ChunkState::Unloaded).unwrap();
        assert_eq!(sm.state(), ChunkState::Unloaded);
        assert_eq!(sm.transition_count(), 8);
    }

    /// Only Active chunks are modifiable.
    #[test]
    fn test_only_active_chunks_are_modifiable() {
        let mut sm = ChunkStateMachine::new();
        assert!(!sm.is_modifiable());
        sm.transition(ChunkState::Scheduled).unwrap();
        assert!(!sm.is_modifiable());
        sm.transition(ChunkState::Generating).unwrap();
        sm.transition(ChunkState::Generated).unwrap();
        sm.transition(ChunkState::Meshing).unwrap();
        sm.transition(ChunkState::Meshed).unwrap();
        sm.transition(ChunkState::Active).unwrap();
        assert!(sm.is_modifiable());
    }

    /// Only Generated chunks can be meshed.
    #[test]
    fn test_only_generated_chunks_are_meshable() {
        let mut sm = ChunkStateMachine::new();
        assert!(!sm.is_meshable());
        sm.transition(ChunkState::Scheduled).unwrap();
        sm.transition(ChunkState::Generating).unwrap();
        assert!(!sm.is_meshable());
        sm.transition(ChunkState::Generated).unwrap();
        assert!(sm.is_meshable());
    }

    /// Active chunks can transition back to Meshing (remesh after edit).
    #[test]
    fn test_active_to_meshing_remesh_cycle() {
        let mut sm = ChunkStateMachine::new();
        sm.transition(ChunkState::Scheduled).unwrap();
        sm.transition(ChunkState::Generating).unwrap();
        sm.transition(ChunkState::Generated).unwrap();
        sm.transition(ChunkState::Meshing).unwrap();
        sm.transition(ChunkState::Meshed).unwrap();
        sm.transition(ChunkState::Active).unwrap();
        // Player edits the chunk, needs remesh
        sm.transition(ChunkState::Meshing).unwrap();
        sm.transition(ChunkState::Meshed).unwrap();
        sm.transition(ChunkState::Active).unwrap();
        assert_eq!(sm.state(), ChunkState::Active);
    }
}
```
