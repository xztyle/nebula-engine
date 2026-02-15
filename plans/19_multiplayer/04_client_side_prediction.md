# Client-Side Prediction

## Problem

In a server-authoritative architecture over pure TCP, the round-trip time (RTT) between client and server introduces latency. If the client waits for server confirmation before displaying the result of player input, movement feels sluggish and unresponsive — the player presses "forward" and nothing happens for 30-100+ ms. This is unacceptable for a real-time game engine. The client needs to predict the outcome of its own inputs locally while still deferring to the server as the ultimate authority.

## Solution

### Prediction Pipeline

The client applies its own inputs immediately to a local copy of the player entity state, without waiting for server confirmation. This produces an instant visual response. The predicted state is provisional — the server's authoritative state always takes precedence (see Story 05 for reconciliation).

```rust
pub struct PredictionState {
    pub predicted_position: Coord128,
    pub predicted_velocity: Vec3_128,
    pub tick: u64,
}
```

### Input Buffer

Every input the client generates is stored in a ring buffer tagged with the tick number at which it was applied. This buffer serves two purposes:

1. **Prediction**: apply inputs immediately to produce the predicted state.
2. **Reconciliation**: re-apply unconfirmed inputs when the server corrects the state (Story 05).

```rust
pub struct InputBuffer {
    pub entries: VecDeque<InputEntry>,
    pub max_size: usize,
}

pub struct InputEntry {
    pub tick: u64,
    pub intent: ClientIntent,
    pub predicted_state: PredictionState,
}

impl InputBuffer {
    pub fn new(max_size: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(max_size),
            max_size,
        }
    }

    pub fn push(&mut self, entry: InputEntry) {
        if self.entries.len() >= self.max_size {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    pub fn discard_up_to(&mut self, tick: u64) {
        while self.entries.front().map_or(false, |e| e.tick <= tick) {
            self.entries.pop_front();
        }
    }

    pub fn entries_after(&self, tick: u64) -> impl Iterator<Item = &InputEntry> {
        self.entries.iter().filter(move |e| e.tick > tick)
    }
}
```

### Prediction System

Each client tick:

1. Sample input devices (keyboard, mouse, gamepad).
2. Construct a `ClientIntent` from the sampled input.
3. Apply the intent to the local player state using the same movement logic the server uses (shared simulation code).
4. Store the input and resulting predicted state in the `InputBuffer`.
5. Send the `ClientIntent` to the server over TCP (tagged with the current tick number).
6. Render using the predicted state.

```rust
pub fn client_prediction_system(
    input: Res<PlayerInput>,
    mut player: Query<(&mut Position128, &mut Velocity, &mut InputBuffer), With<LocalPlayer>>,
    tick: Res<ClientTick>,
) {
    let (mut pos, mut vel, mut buffer) = player.single_mut();
    let intent = input.to_intent(tick.current());

    // Apply locally using shared simulation logic
    let new_state = simulate_movement(&pos, &vel, &intent);
    *pos = new_state.position;
    *vel = new_state.velocity;

    buffer.push(InputEntry {
        tick: tick.current(),
        intent: intent.clone(),
        predicted_state: PredictionState {
            predicted_position: new_state.position,
            predicted_velocity: new_state.velocity,
            tick: tick.current(),
        },
    });

    // Send intent to server
    send_intent_to_server(intent, tick.current());
}
```

### Shared Simulation Code

The movement simulation logic is shared between client and server. This is critical: if the client uses different physics constants or update logic, predictions will always diverge. The shared code lives in a `nebula_shared::simulation` module used by both the client and server binaries.

```rust
pub fn simulate_movement(
    pos: &Coord128,
    vel: &Vec3_128,
    intent: &ClientIntent,
) -> MovementResult {
    // Identical logic on client and server
    // ...
}
```

### Buffer Bounds

The input buffer is bounded (default: 128 entries, ~2 seconds at 60 Hz). Entries older than the last server-confirmed tick are discarded. This prevents unbounded memory growth during prolonged network outages.

## Outcome

- `nebula_multiplayer::prediction` module containing `PredictionState`, `InputBuffer`, `InputEntry`, and `client_prediction_system`.
- `nebula_shared::simulation` module with movement logic shared between client and server.
- Input buffer with bounded size and tick-based garbage collection.
- Immediate visual response to player input with no perceived latency.

## Demo Integration

**Demo crate:** `nebula-demo`

The local player's movement is predicted immediately on the client with no input delay. Movement feels responsive even with 50ms latency to the server.

## Crates & Dependencies

| Crate       | Version | Purpose                                       |
| ----------- | ------- | --------------------------------------------- |
| `bevy_ecs`  | 0.18    | ECS system scheduling, queries, components     |
| `serde`     | 1.0     | Serialization of intents for network send      |
| `postcard`  | 1.1     | Binary encoding of intents over TCP            |
| `tokio`     | 1.49    | Async TCP send for client intents              |

## Unit Tests

### `test_local_input_applies_immediately`
Generate a `MoveDirection` intent. Run the prediction system. Assert that the player's `Position128` changes in the same tick, without waiting for any server message.

### `test_prediction_buffer_stores_states`
Generate 5 inputs over 5 ticks. Assert the `InputBuffer` contains 5 entries, each with the correct tick number and predicted state.

### `test_predicted_position_advances_each_tick`
Apply a constant forward-movement intent for 10 ticks. Assert the predicted position advances monotonically in the movement direction each tick.

### `test_prediction_matches_server_for_simple_movement`
Apply a straight-line movement intent on both the client prediction system and the server simulation. Assert the resulting positions match exactly (since the simulation code is shared and there are no obstructions).

### `test_buffer_size_is_bounded`
Set `max_size` to 64. Push 100 entries. Assert the buffer length is exactly 64 and the oldest entry corresponds to tick 37 (entries 0-36 discarded).
