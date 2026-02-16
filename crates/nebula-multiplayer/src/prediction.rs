//! Client-side prediction: immediate local application of player inputs.
//!
//! The client applies its own inputs to a local copy of the player state
//! without waiting for server confirmation. This eliminates perceived
//! latency. The [`InputBuffer`] stores each input alongside its predicted
//! outcome so that reconciliation (Story 05) can replay unconfirmed inputs
//! when the server corrects the authoritative state.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::authority::ClientIntent;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default maximum number of entries in the input buffer (~2 s at 60 Hz).
pub const DEFAULT_BUFFER_SIZE: usize = 128;

// ---------------------------------------------------------------------------
// PredictionState
// ---------------------------------------------------------------------------

/// Snapshot of predicted player state at a specific tick.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PredictionState {
    /// Predicted X position in millimeters.
    pub x: i64,
    /// Predicted Y position in millimeters.
    pub y: i64,
    /// Predicted Z position in millimeters.
    pub z: i64,
    /// Predicted X velocity in mm/tick.
    pub vx: i64,
    /// Predicted Y velocity in mm/tick.
    pub vy: i64,
    /// Predicted Z velocity in mm/tick.
    pub vz: i64,
    /// Tick number this prediction corresponds to.
    pub tick: u64,
}

// ---------------------------------------------------------------------------
// InputEntry
// ---------------------------------------------------------------------------

/// A single buffered input: the intent that was applied, the tick it
/// belongs to, and the resulting predicted state.
#[derive(Debug, Clone)]
pub struct InputEntry {
    /// Server tick at which this input was generated.
    pub tick: u64,
    /// The intent sent to the server.
    pub intent: ClientIntent,
    /// Local predicted state *after* applying this intent.
    pub predicted_state: PredictionState,
}

// ---------------------------------------------------------------------------
// InputBuffer
// ---------------------------------------------------------------------------

/// Bounded ring buffer of [`InputEntry`] items, indexed by tick.
///
/// Used by the prediction system to store unconfirmed inputs and by the
/// reconciliation system (Story 05) to replay them after a server
/// correction.
pub struct InputBuffer {
    entries: VecDeque<InputEntry>,
    max_size: usize,
}

impl InputBuffer {
    /// Creates a new buffer with the given maximum capacity.
    pub fn new(max_size: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(max_size),
            max_size,
        }
    }

    /// Pushes a new entry, evicting the oldest if at capacity.
    pub fn push(&mut self, entry: InputEntry) {
        if self.entries.len() >= self.max_size {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    /// Discards all entries with tick ≤ `tick` (server has confirmed them).
    pub fn discard_up_to(&mut self, tick: u64) {
        while self.entries.front().is_some_and(|e| e.tick <= tick) {
            self.entries.pop_front();
        }
    }

    /// Returns an iterator over entries with tick > `tick`.
    pub fn entries_after(&self, tick: u64) -> impl Iterator<Item = &InputEntry> {
        self.entries.iter().filter(move |e| e.tick > tick)
    }

    /// Returns the number of buffered entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` if the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns a slice-like view of all entries.
    pub fn entries(&self) -> &VecDeque<InputEntry> {
        &self.entries
    }
}

// ---------------------------------------------------------------------------
// Shared simulation
// ---------------------------------------------------------------------------

/// Result of a single movement simulation step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovementResult {
    /// New position (mm).
    pub x: i64,
    /// New position (mm).
    pub y: i64,
    /// New position (mm).
    pub z: i64,
    /// New velocity (mm/tick).
    pub vx: i64,
    /// New velocity (mm/tick).
    pub vy: i64,
    /// New velocity (mm/tick).
    pub vz: i64,
}

/// Applies a [`ClientIntent`] to a position+velocity pair and returns the
/// resulting state. This function is deterministic and **must** be
/// identical on client and server to keep predictions accurate.
pub fn simulate_movement(
    x: i64,
    y: i64,
    z: i64,
    vx: i64,
    vy: i64,
    vz: i64,
    intent: &ClientIntent,
) -> MovementResult {
    match intent {
        ClientIntent::Move { dx, dy, dz, .. } => {
            // Velocity becomes the movement delta for this tick.
            let new_vx = *dx;
            let new_vy = *dy;
            let new_vz = *dz;
            MovementResult {
                x: x.saturating_add(new_vx),
                y: y.saturating_add(new_vy),
                z: z.saturating_add(new_vz),
                vx: new_vx,
                vy: new_vy,
                vz: new_vz,
            }
        }
        // Non-movement intents don't change position/velocity.
        _ => MovementResult {
            x,
            y,
            z,
            vx,
            vy,
            vz,
        },
    }
}

/// Runs one tick of client-side prediction: applies `intent` to the
/// current state, stores the result in `buffer`, and returns the new
/// predicted state.
pub fn client_prediction_step(
    current: &PredictionState,
    tick: u64,
    intent: ClientIntent,
    buffer: &mut InputBuffer,
) -> PredictionState {
    let result = simulate_movement(
        current.x, current.y, current.z, current.vx, current.vy, current.vz, &intent,
    );
    let state = PredictionState {
        x: result.x,
        y: result.y,
        z: result.z,
        vx: result.vx,
        vy: result.vy,
        vz: result.vz,
        tick,
    };
    buffer.push(InputEntry {
        tick,
        intent,
        predicted_state: state.clone(),
    });
    state
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create a Move intent for player 1.
    fn move_intent(dx: i64, dy: i64, dz: i64) -> ClientIntent {
        ClientIntent::Move {
            player_id: 1,
            dx,
            dy,
            dz,
        }
    }

    fn zero_state() -> PredictionState {
        PredictionState {
            x: 0,
            y: 0,
            z: 0,
            vx: 0,
            vy: 0,
            vz: 0,
            tick: 0,
        }
    }

    #[test]
    fn test_local_input_applies_immediately() {
        let mut buffer = InputBuffer::new(DEFAULT_BUFFER_SIZE);
        let current = zero_state();
        let state = client_prediction_step(&current, 1, move_intent(100, 0, 0), &mut buffer);
        assert_eq!(state.x, 100);
        assert_eq!(state.y, 0);
        assert_eq!(state.z, 0);
        assert_eq!(state.tick, 1);
    }

    #[test]
    fn test_prediction_buffer_stores_states() {
        let mut buffer = InputBuffer::new(DEFAULT_BUFFER_SIZE);
        let mut current = zero_state();

        for tick in 1..=5 {
            current = client_prediction_step(&current, tick, move_intent(10, 20, 30), &mut buffer);
        }

        assert_eq!(buffer.len(), 5);
        for (i, entry) in buffer.entries().iter().enumerate() {
            assert_eq!(entry.tick, (i + 1) as u64);
            assert_eq!(entry.predicted_state.x, 10 * (i as i64 + 1));
        }
    }

    #[test]
    fn test_predicted_position_advances_each_tick() {
        let mut buffer = InputBuffer::new(DEFAULT_BUFFER_SIZE);
        let mut current = zero_state();
        let mut prev_z = i64::MIN;

        for tick in 1..=10 {
            current = client_prediction_step(&current, tick, move_intent(0, 0, 50), &mut buffer);
            assert!(
                current.z > prev_z,
                "tick {tick}: z={} should exceed prev={prev_z}",
                current.z
            );
            prev_z = current.z;
        }
    }

    #[test]
    fn test_prediction_matches_server_for_simple_movement() {
        use crate::authority::{AuthoritativeWorld, IntentValidator, PlayerState};

        let mut world = AuthoritativeWorld::new();
        world.spawn_player(PlayerState {
            player_id: 1,
            x: 0,
            y: 0,
            z: 0,
            yaw_mrad: 0,
            pitch_mrad: 0,
        });

        let mut buffer = InputBuffer::new(DEFAULT_BUFFER_SIZE);
        let mut current = zero_state();

        for tick in 1..=5 {
            let intent = move_intent(100, -50, 25);
            current = client_prediction_step(&current, tick, intent.clone(), &mut buffer);
            IntentValidator::validate_and_apply(&intent, &mut world).unwrap();
        }

        let server = world.find_player(1).unwrap();
        assert_eq!(current.x, server.x);
        assert_eq!(current.y, server.y);
        assert_eq!(current.z, server.z);
    }

    #[test]
    fn test_buffer_size_is_bounded() {
        let mut buffer = InputBuffer::new(64);
        for tick in 0..100u64 {
            buffer.push(InputEntry {
                tick,
                intent: move_intent(1, 0, 0),
                predicted_state: PredictionState {
                    x: tick as i64,
                    y: 0,
                    z: 0,
                    vx: 1,
                    vy: 0,
                    vz: 0,
                    tick,
                },
            });
        }
        assert_eq!(buffer.len(), 64);
        // Oldest surviving entry: ticks 0..99, kept 36..99 (64 entries).
        assert_eq!(buffer.entries().front().unwrap().tick, 36);
    }
}
