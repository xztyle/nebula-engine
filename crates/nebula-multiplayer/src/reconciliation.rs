//! Server reconciliation: corrects client prediction when the server's
//! authoritative state diverges from the locally predicted state.
//!
//! When the server sends an authoritative update for a player, the client
//! compares it against its prediction for the same tick. If they differ,
//! the client rewinds to the server state and replays all unconfirmed
//! inputs to produce a corrected prediction.

use serde::{Deserialize, Serialize};

use crate::prediction::{InputBuffer, PredictionState, simulate_movement};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Threshold in millimeters below which corrections are smoothed visually.
/// 500 mm = 0.5 m.
pub const SMALL_CORRECTION_THRESHOLD_MM: i64 = 500;

/// Default exponential decay rate for visual offset (per second).
pub const DEFAULT_DECAY_RATE: f32 = 10.0;

/// Minimum visual offset magnitude (mm as f32) before snapping to zero.
const MIN_OFFSET_MAGNITUDE: f32 = 1.0;

// ---------------------------------------------------------------------------
// AuthoritativePlayerState
// ---------------------------------------------------------------------------

/// Server-authoritative state for a player at a given tick.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthoritativePlayerState {
    /// The server tick this state corresponds to.
    pub tick: u64,
    /// Authoritative X position in millimeters.
    pub x: i64,
    /// Authoritative Y position in millimeters.
    pub y: i64,
    /// Authoritative Z position in millimeters.
    pub z: i64,
    /// Authoritative X velocity in mm/tick.
    pub vx: i64,
    /// Authoritative Y velocity in mm/tick.
    pub vy: i64,
    /// Authoritative Z velocity in mm/tick.
    pub vz: i64,
}

// ---------------------------------------------------------------------------
// CorrectionSmoothing
// ---------------------------------------------------------------------------

/// Visual smoothing for small prediction corrections.
///
/// The logical position snaps immediately to the corrected value, but the
/// rendered position is offset by `visual_offset` which decays
/// exponentially each frame.
#[derive(Debug, Clone)]
pub struct CorrectionSmoothing {
    /// Current visual offset from the logical position (mm, as f32).
    pub visual_offset_x: f32,
    /// Current visual offset from the logical position (mm, as f32).
    pub visual_offset_y: f32,
    /// Current visual offset from the logical position (mm, as f32).
    pub visual_offset_z: f32,
    /// Exponential decay rate per second.
    pub decay_rate: f32,
}

impl Default for CorrectionSmoothing {
    fn default() -> Self {
        Self {
            visual_offset_x: 0.0,
            visual_offset_y: 0.0,
            visual_offset_z: 0.0,
            decay_rate: DEFAULT_DECAY_RATE,
        }
    }
}

impl CorrectionSmoothing {
    /// Creates a new smoothing state with the given decay rate.
    pub fn new(decay_rate: f32) -> Self {
        Self {
            decay_rate,
            ..Default::default()
        }
    }

    /// Records a correction. For small corrections (< 0.5 m), accumulates
    /// a visual offset that will decay over time. For large corrections,
    /// snaps immediately (zeroes the visual offset).
    pub fn apply_correction(&mut self, delta_x: i64, delta_y: i64, delta_z: i64) {
        let dist_sq =
            (delta_x as f64).powi(2) + (delta_y as f64).powi(2) + (delta_z as f64).powi(2);
        let threshold_sq = (SMALL_CORRECTION_THRESHOLD_MM as f64).powi(2);

        if dist_sq < threshold_sq {
            // Small correction: accumulate visual offset (old pos - new pos).
            // The offset represents where the visual was relative to the
            // new logical position, so it's the *negative* of the correction.
            self.visual_offset_x -= delta_x as f32;
            self.visual_offset_y -= delta_y as f32;
            self.visual_offset_z -= delta_z as f32;
        } else {
            // Large correction: snap instantly.
            self.visual_offset_x = 0.0;
            self.visual_offset_y = 0.0;
            self.visual_offset_z = 0.0;
        }
    }

    /// Decays the visual offset over `dt` seconds.
    pub fn update(&mut self, dt: f32) {
        let factor = (-self.decay_rate * dt).exp();
        self.visual_offset_x *= factor;
        self.visual_offset_y *= factor;
        self.visual_offset_z *= factor;

        let mag_sq = self.visual_offset_x.powi(2)
            + self.visual_offset_y.powi(2)
            + self.visual_offset_z.powi(2);
        if mag_sq < MIN_OFFSET_MAGNITUDE * MIN_OFFSET_MAGNITUDE {
            self.visual_offset_x = 0.0;
            self.visual_offset_y = 0.0;
            self.visual_offset_z = 0.0;
        }
    }

    /// Returns `true` if the visual offset is effectively zero.
    pub fn is_zero(&self) -> bool {
        self.visual_offset_x == 0.0 && self.visual_offset_y == 0.0 && self.visual_offset_z == 0.0
    }
}

// ---------------------------------------------------------------------------
// positions_match
// ---------------------------------------------------------------------------

/// Returns `true` if two positions are identical (exact integer match).
pub fn positions_match(a: &PredictionState, server: &AuthoritativePlayerState) -> bool {
    a.x == server.x && a.y == server.y && a.z == server.z
}

// ---------------------------------------------------------------------------
// reconcile
// ---------------------------------------------------------------------------

/// Result of a reconciliation step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconciliationResult {
    /// Whether a correction was applied.
    pub corrected: bool,
    /// The new predicted position after reconciliation.
    pub x: i64,
    /// The new predicted position after reconciliation.
    pub y: i64,
    /// The new predicted position after reconciliation.
    pub z: i64,
    /// The new predicted velocity after reconciliation.
    pub vx: i64,
    /// The new predicted velocity after reconciliation.
    pub vy: i64,
    /// The new predicted velocity after reconciliation.
    pub vz: i64,
}

/// Reconciles the client's predicted state against the server's
/// authoritative state.
///
/// 1. Finds the prediction for the server's tick in the buffer.
/// 2. If the prediction matches, discards confirmed entries — done.
/// 3. If not, rewinds to the server state and replays unconfirmed inputs.
///
/// Returns the corrected state. The caller should apply visual smoothing
/// via [`CorrectionSmoothing`] if desired.
pub fn reconcile(
    server_state: &AuthoritativePlayerState,
    buffer: &mut InputBuffer,
) -> ReconciliationResult {
    // Check if we have a matching prediction
    let needs_correction = {
        let matching = buffer
            .entries()
            .iter()
            .find(|e| e.tick == server_state.tick);
        match matching {
            Some(entry) => !positions_match(&entry.predicted_state, server_state),
            None => true, // No prediction for this tick; treat as correction needed
        }
    };

    // Discard confirmed entries
    buffer.discard_up_to(server_state.tick);

    if !needs_correction {
        // Prediction was correct. Return server state (they match).
        return ReconciliationResult {
            corrected: false,
            x: server_state.x,
            y: server_state.y,
            z: server_state.z,
            vx: server_state.vx,
            vy: server_state.vy,
            vz: server_state.vz,
        };
    }

    // Rewind to server state and replay unconfirmed inputs
    let mut x = server_state.x;
    let mut y = server_state.y;
    let mut z = server_state.z;
    let mut vx = server_state.vx;
    let mut vy = server_state.vy;
    let mut vz = server_state.vz;

    for entry in buffer.entries().iter() {
        let result = simulate_movement(x, y, z, vx, vy, vz, &entry.intent);
        x = result.x;
        y = result.y;
        z = result.z;
        vx = result.vx;
        vy = result.vy;
        vz = result.vz;
    }

    ReconciliationResult {
        corrected: true,
        x,
        y,
        z,
        vx,
        vy,
        vz,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authority::ClientIntent;
    use crate::prediction::{InputBuffer, PredictionState, client_prediction_step};

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
    fn test_small_correction_is_smooth() {
        // Client predicted (100, 0, 0) at tick 5; server says (400, 0, 0).
        // Delta = 300 mm < 500 mm threshold → smoothing active.
        let mut buffer = InputBuffer::new(128);
        let mut current = zero_state();
        for tick in 1..=5 {
            current = client_prediction_step(&current, tick, move_intent(20, 0, 0), &mut buffer);
        }
        assert_eq!(current.x, 100);

        let server = AuthoritativePlayerState {
            tick: 5,
            x: 400,
            y: 0,
            z: 0,
            vx: 0,
            vy: 0,
            vz: 0,
        };

        let result = reconcile(&server, &mut buffer);
        assert!(result.corrected);

        // Apply correction smoothing
        let mut smoothing = CorrectionSmoothing::default();
        let delta_x = result.x - current.x;
        let delta_y = result.y - current.y;
        let delta_z = result.z - current.z;
        smoothing.apply_correction(delta_x, delta_y, delta_z);

        // Small correction: visual offset should be non-zero
        assert!(
            !smoothing.is_zero(),
            "smoothing should be active for small correction"
        );
    }

    #[test]
    fn test_large_correction_is_instant_snap() {
        // Client predicted (100, 0, 0) at tick 5; server says (5100, 0, 0).
        // Delta = 5000 mm >= 500 mm threshold → snap.
        let mut buffer = InputBuffer::new(128);
        let mut current = zero_state();
        for tick in 1..=5 {
            current = client_prediction_step(&current, tick, move_intent(20, 0, 0), &mut buffer);
        }
        assert_eq!(current.x, 100);

        let server = AuthoritativePlayerState {
            tick: 5,
            x: 5100,
            y: 0,
            z: 0,
            vx: 0,
            vy: 0,
            vz: 0,
        };

        let result = reconcile(&server, &mut buffer);
        assert!(result.corrected);
        assert_eq!(result.x, 5100);

        let mut smoothing = CorrectionSmoothing::default();
        let delta_x = result.x - current.x;
        smoothing.apply_correction(delta_x, 0, 0);

        // Large correction: visual offset should be zero (instant snap)
        assert!(
            smoothing.is_zero(),
            "large correction should snap instantly"
        );
    }

    #[test]
    fn test_re_simulation_produces_correct_state() {
        // Server says (0,0,0) at tick 10. Buffer has inputs for ticks
        // 11, 12, 13 each moving +1000 mm forward on X.
        let mut buffer = InputBuffer::new(128);
        let mut current = PredictionState {
            x: 0,
            y: 0,
            z: 0,
            vx: 0,
            vy: 0,
            vz: 0,
            tick: 10,
        };

        for tick in 11..=13 {
            current = client_prediction_step(&current, tick, move_intent(1000, 0, 0), &mut buffer);
        }
        assert_eq!(current.x, 3000);

        let server = AuthoritativePlayerState {
            tick: 10,
            x: 0,
            y: 0,
            z: 0,
            vx: 0,
            vy: 0,
            vz: 0,
        };

        let result = reconcile(&server, &mut buffer);
        // Prediction matched server at tick 10 baseline, and replay of
        // 3 inputs (+1000 each) → 3000.
        assert_eq!(result.x, 3000);
        assert_eq!(result.y, 0);
        assert_eq!(result.z, 0);
    }

    #[test]
    fn test_inputs_after_correction_are_preserved() {
        // Server corrects position at tick 5. Buffer has 5 unconfirmed
        // inputs (ticks 6-10), each +100 mm on X.
        let mut buffer = InputBuffer::new(128);
        let mut current = PredictionState {
            x: 500,
            y: 0,
            z: 0,
            vx: 100,
            vy: 0,
            vz: 0,
            tick: 5,
        };

        for tick in 6..=10 {
            current = client_prediction_step(&current, tick, move_intent(100, 0, 0), &mut buffer);
        }
        assert_eq!(buffer.len(), 5);

        // Server says player was actually at x=200 at tick 5
        let server = AuthoritativePlayerState {
            tick: 5,
            x: 200,
            y: 0,
            z: 0,
            vx: 0,
            vy: 0,
            vz: 0,
        };

        let result = reconcile(&server, &mut buffer);
        assert!(result.corrected);
        // 200 + 5 * 100 = 700
        assert_eq!(result.x, 700);
    }

    #[test]
    fn test_reconciliation_happens_in_one_frame() {
        // Fill buffer with 64 unconfirmed inputs and measure that
        // reconciliation completes without deferred work.
        let mut buffer = InputBuffer::new(128);
        let mut current = zero_state();

        for tick in 1..=64 {
            current = client_prediction_step(&current, tick, move_intent(10, 5, 3), &mut buffer);
        }

        let server = AuthoritativePlayerState {
            tick: 0,
            x: 999,
            y: 0,
            z: 0,
            vx: 0,
            vy: 0,
            vz: 0,
        };

        let start = std::time::Instant::now();
        let result = reconcile(&server, &mut buffer);
        let elapsed = start.elapsed();

        assert!(result.corrected);
        // 999 + 64*10 = 1639
        assert_eq!(result.x, 1639);
        // Must complete well within one 60Hz frame (16.6ms)
        assert!(
            elapsed.as_millis() < 16,
            "reconciliation took {elapsed:?}, exceeds single frame"
        );
    }
}
