# Server Reconciliation

## Problem

Client-side prediction (Story 04) provides responsive movement, but predictions are not always correct. The server may compute a different outcome due to collisions with other players, server-side physics corrections, or environmental changes the client did not yet know about. When the server's authoritative state diverges from the client's predicted state, the client must correct itself without causing jarring visual snaps or losing subsequent unconfirmed inputs.

## Solution

### Reconciliation Trigger

Each tick, the server sends the client an authoritative state update for the player entity, tagged with the server tick number. The client compares this against the predicted state stored in its `InputBuffer` for the matching tick.

```rust
#[derive(Serialize, Deserialize)]
pub struct AuthoritativePlayerState {
    pub tick: u64,
    pub position: Coord128,
    pub velocity: Vec3_128,
}
```

### Reconciliation Algorithm

When the client receives an `AuthoritativePlayerState`:

1. **Find the matching prediction** in the input buffer for the server's tick.
2. **Compare** the server position against the predicted position for that tick.
3. If the positions match (within a small epsilon), the prediction was correct. Discard confirmed entries from the buffer.
4. If the positions differ:
   a. **Rewind** the player state to the server's authoritative position and velocity at the confirmed tick.
   b. **Replay** all unconfirmed inputs from the buffer (ticks after the confirmed tick) using the shared simulation logic.
   c. This produces a **corrected predicted state** that incorporates the server's correction plus subsequent inputs.

```rust
pub fn reconcile(
    server_state: &AuthoritativePlayerState,
    buffer: &mut InputBuffer,
    current_pos: &mut Coord128,
    current_vel: &mut Vec3_128,
) {
    // Discard confirmed inputs
    buffer.discard_up_to(server_state.tick);

    // Check if correction is needed
    let needs_correction = if let Some(predicted) = buffer.prediction_at(server_state.tick) {
        !positions_match(&predicted.predicted_position, &server_state.position)
    } else {
        true
    };

    if needs_correction {
        // Rewind to server state
        let mut pos = server_state.position;
        let mut vel = server_state.velocity;

        // Replay unconfirmed inputs
        for entry in buffer.entries_after(server_state.tick) {
            let result = simulate_movement(&pos, &vel, &entry.intent);
            pos = result.position;
            vel = result.velocity;
        }

        // Apply correction
        *current_pos = pos;
        *current_vel = vel;
    }
}
```

### Correction Smoothing

Not all corrections should be applied the same way. The system distinguishes between small and large corrections:

- **Small correction** (< 0.5 m): Interpolate smoothly from the current visual position to the corrected position over 100 ms. The logical position snaps immediately, but the rendered position smooths. This hides minor jitter.
- **Large correction** (>= 0.5 m): Snap instantly. Large corrections typically indicate teleportation, rubber-banding from severe lag, or server-enforced repositioning. Smoothing a large gap would look like sliding and be more disorienting than snapping.

```rust
pub struct CorrectionSmoothing {
    pub visual_offset: Vec3,
    pub decay_rate: f32, // per second
}

impl CorrectionSmoothing {
    pub fn apply_correction(&mut self, logical_pos: &Coord128, corrected_pos: &Coord128) {
        let delta = logical_pos.offset_to(corrected_pos);
        let distance = delta.length_f64();

        if distance < 0.5 {
            // Small: accumulate visual offset, let it decay
            self.visual_offset += delta.to_vec3();
        } else {
            // Large: snap, zero out any prior smoothing
            self.visual_offset = Vec3::ZERO;
        }
    }

    pub fn update(&mut self, dt: f32) {
        self.visual_offset *= (-self.decay_rate * dt).exp();
        if self.visual_offset.length() < 0.001 {
            self.visual_offset = Vec3::ZERO;
        }
    }

    pub fn visual_position(&self, logical_pos: &Coord128) -> Coord128 {
        logical_pos.offset_by_vec3(self.visual_offset)
    }
}
```

### Preserving Subsequent Inputs

The replay step is critical: without it, all inputs applied after the confirmed tick would be lost. By replaying the unconfirmed portion of the input buffer against the corrected state, the player's recent actions are preserved even during correction.

### Single-Frame Reconciliation

The entire reconciliation process — rewind, replay, correction — executes within a single frame. The replay is cheap because it only involves the player entity and a small number of buffered inputs (typically < 10 at 60 Hz with reasonable latency). This ensures no multi-frame artifacts.

## Outcome

- `nebula_multiplayer::reconciliation` module containing the `reconcile` function, `CorrectionSmoothing`, and `AuthoritativePlayerState`.
- Seamless correction of client prediction errors without losing unconfirmed inputs.
- Visual smoothing for small corrections, instant snapping for large corrections.
- Integration with the prediction system from Story 04 and the input buffer.

## Demo Integration

**Demo crate:** `nebula-demo`

If the server disagrees with the client's predicted position, the client smoothly corrects to the server's authoritative position over several frames.

## Crates & Dependencies

| Crate       | Version | Purpose                                             |
| ----------- | ------- | --------------------------------------------------- |
| `bevy_ecs`  | 0.18    | ECS component access for player state               |
| `serde`     | 1.0     | Deserialization of authoritative state messages      |
| `postcard`  | 1.1     | Binary decoding of server state updates              |
| `tokio`     | 1.49    | Async TCP for receiving authoritative updates        |

## Unit Tests

### `test_small_correction_is_smooth`
Set predicted position to (100.0, 0.0, 0.0). Receive authoritative position (100.3, 0.0, 0.0) — delta 0.3 m. Run reconciliation. Assert the logical position is corrected and `CorrectionSmoothing.visual_offset` is non-zero (smoothing active).

### `test_large_correction_is_instant_snap`
Set predicted position to (100.0, 0.0, 0.0). Receive authoritative position (105.0, 0.0, 0.0) — delta 5.0 m. Run reconciliation. Assert the logical position snaps to the authoritative value and `CorrectionSmoothing.visual_offset` is zero.

### `test_re_simulation_produces_correct_state`
Set authoritative position to (0, 0, 0) at tick 10. Buffer contains inputs for ticks 11, 12, 13 (each moving +1m forward). Run reconciliation. Assert the corrected position is (3, 0, 0) — the server state plus three replayed inputs.

### `test_inputs_after_correction_are_preserved`
Buffer has 5 unconfirmed inputs after the server tick. Run reconciliation. Assert all 5 inputs are replayed and the resulting position accounts for all 5 movement steps.

### `test_reconciliation_happens_in_one_frame`
Measure the frame boundary before and after reconciliation with 64 buffered inputs. Assert reconciliation completes within a single tick (no deferred work, no multi-frame spread).
