# Space Audio Rules

## Problem

In reality, space is a vacuum and sound does not propagate. Applying strict physical realism would mean complete silence whenever the player is outside a pressurized environment -- no explosion sounds, no thruster roar, no warning klaxons from nearby ships. This is technically accurate but miserable for gameplay. Players need audio feedback to understand what is happening around them. The engine must adopt a "movie space" approach: external sounds in vacuum are present but heavily filtered (muffled, bass-heavy, distant-feeling), while sounds originating inside the player's ship or suit remain crisp and normal. UI sounds (menu clicks, notifications, HUD alerts) are always unaffected. The transition between atmosphere and vacuum must be smooth, not a jarring on/off switch.

## Solution

### Environment classification

The audio system maintains an `AudioEnvironment` state derived from the player's current location:

```rust
#[derive(Clone, Copy, PartialEq)]
pub enum AudioEnvironment {
    Atmosphere,          // on a planet with air, inside a pressurized station
    Vacuum,              // in space, on an airless body's surface
    Transitioning(f32),  // lerp factor 0.0 (full atmosphere) to 1.0 (full vacuum)
}
```

The environment is set by game systems that know whether the player is inside a pressurized volume (ship interior, space station, planet with atmosphere) or in vacuum (EVA, airless moon surface, open space). The `Transitioning` state is used during airlock sequences or when crossing an atmospheric boundary, with the `f32` factor ramping over a configurable duration (default: 1.5 seconds).

### Sound source classification

Every spatial sound and ambient sound carries a `SoundContext` tag:

```rust
#[derive(Clone, Copy, PartialEq)]
pub enum SoundContext {
    External,   // explosion in space, thruster outside the hull, asteroid impact
    Internal,   // footsteps inside ship, cockpit beeps, suit life-support hiss
    UI,         // menu clicks, HUD alerts, notification chimes
}
```

### Filter rules

| Player Environment | Sound Context | Treatment |
|--------------------|---------------|-----------|
| Atmosphere | External | Normal (no filter) |
| Atmosphere | Internal | Normal |
| Atmosphere | UI | Normal |
| Vacuum | External | Heavy low-pass filter (cutoff ~300 Hz) + volume reduction to ~30% |
| Vacuum | Internal | Normal (sound travels through the suit/ship structure) |
| Vacuum | UI | Normal (non-diegetic, always clear) |
| Transitioning(t) | External | Lerp filter cutoff from 20,000 Hz to 300 Hz and volume from 100% to 30% based on `t` |
| Transitioning(t) | Internal | Normal |
| Transitioning(t) | UI | Normal |

### Implementation

A `SpaceAudioSystem` ECS system runs each frame after spatial audio and occlusion:

1. Read the current `AudioEnvironment` from the ECS resource (set by the player/ship systems).
2. For each active sound with a `SoundContext::External` tag:
   - If `Atmosphere`: ensure no space filter is applied (or set cutoff to 20,000 Hz).
   - If `Vacuum`: apply the heavy low-pass filter and volume reduction.
   - If `Transitioning(t)`: interpolate the filter parameters linearly based on `t`.
3. `Internal` and `UI` sounds are always left unmodified.

The low-pass filter is the same mechanism used by the occlusion system (Story 06), so the two effects stack: a sound that is both occluded by voxels *and* in vacuum receives both the occlusion filter and the space filter, resulting in an extremely muffled bass rumble -- which is exactly the right feel for hearing a distant explosion through a ship's hull while in space.

### Transition smoothing

The `Transitioning` state ramps linearly over its duration. Game systems set it when detecting boundary crossings (leaving an airlock, entering/exiting atmosphere during planetary descent). The audio system does not control the ramp itself -- it simply reads the current `t` value and applies the corresponding filter parameters, ensuring a smooth audible transition.

## Outcome

A `space_audio.rs` module in `crates/nebula_audio/src/` exporting `AudioEnvironment`, `SoundContext`, `SpaceAudioConfig`, and the `space_audio_update` system. The system reads the environment state and applies per-sound filtering rules each frame.

## Demo Integration

**Demo crate:** `nebula-demo`

In space, near-silence. Only suit sounds and radio comms are audible. Entering atmosphere gradually restores full environmental audio.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| kira | 0.10 | Per-sound low-pass filter effects and volume control |
| glam | 0.29 | Interpolation utilities for transition smoothing |
| log | 0.4 | Debug-level logging for environment transitions |

## Unit Tests

| Test Function | Description | Assertion |
|---------------|-------------|-----------|
| `test_external_sounds_in_space_are_filtered` | Set `AudioEnvironment::Vacuum`. Play an `External` sound. | Filter cutoff is approximately `300` Hz; volume is approximately `0.3`. |
| `test_internal_sounds_are_normal` | Set `AudioEnvironment::Vacuum`. Play an `Internal` sound. | Filter cutoff is `20000` Hz (no filter); volume is `1.0`. |
| `test_ui_sounds_are_unaffected` | Set `AudioEnvironment::Vacuum`. Play a `UI` sound. | No filter applied; volume is `1.0`. |
| `test_transitioning_from_atmosphere_to_space_applies_filter` | Set `AudioEnvironment::Transitioning(0.5)`. Play an `External` sound. | Filter cutoff is approximately midway between `20000` Hz and `300` Hz; volume is approximately `0.65`. |
| `test_filter_removed_when_reentering_atmosphere` | Start in `Vacuum` (filter applied), then switch to `Atmosphere`. | `External` sound filter cutoff returns to `20000` Hz; volume returns to `1.0`. |
| `test_space_filter_stacks_with_occlusion` | Set `Vacuum` and apply 3 voxels of occlusion to an `External` sound. | Both the space low-pass filter and the occlusion filter are applied; effective cutoff is the minimum of the two. |
| `test_all_contexts_normal_in_atmosphere` | Set `AudioEnvironment::Atmosphere`. Play `External`, `Internal`, and `UI` sounds. | All three have no filter applied and volume at `1.0`. |
