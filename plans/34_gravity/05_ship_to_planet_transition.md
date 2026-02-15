# Ship-to-Planet Transition

## Problem

A spaceship traveling through the void experiences zero gravity. As it approaches a planet, it enters the planet's gravity well and must transition from zero-g free flight to gravity-affected descent, culminating in a surface landing. This transition involves multiple interacting systems: gravity magnitude increasing continuously, ship orientation gradually aligning to the surface, flight controls transitioning from 6-DOF thruster control to atmospheric/aerodynamic behavior, and ultimately landing gear deployment and ground-mode physics. The reverse (takeoff) must also be smooth. Without a well-defined transition sequence, the player experiences jarring snaps in gravity, orientation, and control scheme — breaking immersion at one of the most dramatic moments in gameplay.

## Solution

### Gravity Well Zones

Define concentric zones around each gravity source that trigger gameplay transitions. These are not physics constructs (the gravity field itself is continuous from story 02/03) — they are gameplay thresholds:

```rust
use bevy_ecs::component::Component;

/// Defines the gameplay transition zones for a gravity source.
/// Attached alongside GravitySource on planet/moon entities.
#[derive(Component, Debug, Clone)]
pub struct GravityWell {
    /// Distance at which gravity becomes "noticeable" and the HUD
    /// begins showing approach indicators. Typically 3-5× the surface radius.
    pub detection_radius: i128,

    /// Distance at which the ship begins to feel gravitational pull
    /// strong enough to affect trajectory. Typically 2-3× surface radius.
    pub influence_threshold: i128,

    /// Distance at which atmospheric drag begins (if applicable).
    /// Determines when the ship transitions from vacuum to atmospheric flight.
    pub atmosphere_radius: i128,

    /// Distance above surface at which landing mode becomes available.
    /// Typically a few hundred meters to a few kilometers.
    pub landing_altitude: i128,
}
```

### ShipFlightState Component

Tracks the ship's current flight mode and manages transitions:

```rust
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub enum ShipFlightState {
    /// No significant gravity. Full 6-DOF thruster control.
    FreeFlight,

    /// Gravity well detected. Gravity force is being applied.
    /// Ship can still maneuver freely but is being pulled.
    GravityApproach,

    /// Within atmosphere. Drag forces apply. Orientation begins
    /// aligning to surface. Controls transition to atmospheric flight.
    AtmosphericEntry,

    /// Low altitude, landing gear deployed. Vertical descent mode.
    /// Gravity alignment fully active. Contact with surface imminent.
    Landing,

    /// On the ground. Physics switches to ground mode (no flight forces).
    /// Ship is a static/kinematic body on the surface.
    Grounded,

    /// Takeoff initiated. Vertical ascent with gravity alignment active.
    Takeoff,
}
```

### Transition Detection System

A system evaluates the ship's distance to all gravity sources and triggers state transitions:

```rust
fn ship_gravity_transition_system(
    registry: Res<GravitySourceRegistry>,
    wells: Query<(&WorldPos, &GravityWell)>,
    mut ships: Query<(&WorldPos, &mut ShipFlightState, &LocalGravity)>,
) {
    for (ship_pos, mut state, gravity) in ships.iter_mut() {
        let nearest = registry.nearest_source(ship_pos);

        let Some(nearest) = nearest else {
            // No gravity source in range — free flight.
            *state = ShipFlightState::FreeFlight;
            continue;
        };

        // Compute distance to nearest source center.
        let dx = (nearest.position.x - ship_pos.x) as f64;
        let dy = (nearest.position.y - ship_pos.y) as f64;
        let dz = (nearest.position.z - ship_pos.z) as f64;
        let distance = (dx * dx + dy * dy + dz * dz).sqrt() as i128;

        // Look up the GravityWell for this source.
        let Ok((_, well)) = wells.get(nearest.entity) else {
            continue;
        };

        // Determine target state based on distance.
        let target_state = if distance <= nearest.source.radius + well.landing_altitude {
            ShipFlightState::Landing
        } else if distance <= well.atmosphere_radius {
            ShipFlightState::AtmosphericEntry
        } else if distance <= well.influence_threshold {
            ShipFlightState::GravityApproach
        } else {
            ShipFlightState::FreeFlight
        };

        // Only transition forward (approach) or detect takeoff.
        // Grounded state is set by the landing system, not here.
        if *state != ShipFlightState::Grounded
            && *state != ShipFlightState::Takeoff
        {
            *state = target_state;
        }
    }
}
```

### Orientation Transition

During `GravityApproach`, the ship's `GravityAlignment.override_active` remains `true` — the pilot has full orientation control. Upon entering `AtmosphericEntry`, the override is gradually released:

```rust
fn ship_orientation_transition_system(
    mut ships: Query<(&ShipFlightState, &mut GravityAlignment)>,
) {
    for (state, mut alignment) in ships.iter_mut() {
        match state {
            ShipFlightState::FreeFlight | ShipFlightState::GravityApproach => {
                alignment.override_active = true;
            }
            ShipFlightState::AtmosphericEntry => {
                // Slow alignment — ship gradually orients to surface.
                alignment.override_active = false;
                alignment.alignment_speed = 1.5; // Gentle for large ships.
            }
            ShipFlightState::Landing | ShipFlightState::Grounded => {
                alignment.override_active = false;
                alignment.alignment_speed = 4.0; // Firmer near ground.
            }
            ShipFlightState::Takeoff => {
                // During takeoff, maintain surface alignment until
                // clearing the atmosphere.
                alignment.override_active = false;
                alignment.alignment_speed = 2.0;
            }
        }
    }
}
```

### Landing and Takeoff

Landing is detected when the ship's `ShipFlightState::Landing` collides with the terrain surface (via Rapier contact events). The state transitions to `Grounded`, the rigid body is set to kinematic, and flight forces are disabled.

Takeoff is initiated by player input. The state transitions to `Takeoff`, the rigid body is set back to dynamic, and upward thrust is applied. As altitude increases, the transition system naturally moves the state back through `AtmosphericEntry` → `GravityApproach` → `FreeFlight`.

## Outcome

Ships smoothly transition between zero-g free flight and planetary surface through well-defined gameplay zones. The `ShipFlightState` component drives control scheme changes, orientation alignment activation, and physics mode switching. Gravity influence increases continuously (no discrete steps in the physics), while gameplay systems use zone thresholds for control transitions. `cargo test -p nebula-gravity` passes all ship-to-planet transition tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Flying a spaceship toward a planet triggers a gravity transition: the HUD shows increasing G-force, ship orientation aligns to surface, flight controls shift to atmospheric.

## Crates & Dependencies

- `rapier3d = "0.32"` — Rigid body mode switching (dynamic/kinematic) for landing/grounded states, contact event detection for landing
- `bevy_ecs = "0.18"` — ECS framework for `Component`, `Query`, `Res`, system scheduling, state management
- `glam = "0.32"` — Vector math for distance computation and orientation transitions
- `nebula-gravity` (internal) — `GravitySource`, `GravitySourceRegistry`, `LocalGravity`, `GravityAlignment` from stories 01-04
- `nebula-math` (internal) — `WorldPos` for i128 distance calculations

## Unit Tests

- **`test_gravity_well_entry_detected`** — Place a planet with `GravityWell { detection_radius: 30_000_000, influence_threshold: 20_000_000, atmosphere_radius: 7_000_000, landing_altitude: 5_000 }`. Place a ship at `WorldPos(25_000_000, 0, 0)` (within detection but beyond influence). Run the transition system. Assert `ShipFlightState::FreeFlight` (detection alone does not change flight state). Move ship to `WorldPos(15_000_000, 0, 0)` (within influence threshold). Run system. Assert `ShipFlightState::GravityApproach`.

- **`test_gravity_increases_with_proximity`** — Place a planet at the origin. Sample `LocalGravity.magnitude` at distances `20_000_000`, `10_000_000`, `7_000_000`, and `6_500_000` from center. Assert each successive sample has strictly greater magnitude than the previous. Verifies gravity increases continuously as the ship approaches.

- **`test_orientation_aligns_to_surface`** — Set ship state to `AtmosphericEntry`. Set `LocalGravity.direction = Vec3::new(-1.0, 0.0, 0.0)`. Run the orientation transition system and alignment system for 60 ticks. Assert `GravityAlignment.current_up` is approximately `Vec3::new(1.0, 0.0, 0.0)` (aligned away from gravity). Assert `override_active == false`.

- **`test_landing_transitions_to_ground_mode`** — Set ship state to `Landing`. Simulate a collision contact with the terrain surface. Assert state transitions to `ShipFlightState::Grounded`. Assert the rigid body mode is set to kinematic (not dynamic).

- **`test_takeoff_transitions_to_flight_mode`** — Start with `ShipFlightState::Grounded`. Initiate takeoff (set state to `Takeoff`). Simulate upward movement increasing distance from planet center. Run transition system at increasing altitudes. Assert state progresses through `Takeoff` → `AtmosphericEntry` → `GravityApproach` → `FreeFlight` as altitude increases past each threshold.
