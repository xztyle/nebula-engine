# Looping Ambient

## Problem

Environments in Nebula Engine are diverse -- windswept mountain surfaces, dripping cave interiors, rolling ocean waves, the hum of a ship's engine in space. Each environment demands persistent, seamlessly looping ambient audio that immerses the player. When the player transitions between environments (stepping from a cave into open air, diving underwater, launching into orbit), the audio must crossfade smoothly rather than cutting abruptly. Multiple ambient categories may coexist (e.g., wind *and* engine hum while flying in atmosphere), but within a single category only one track should be active at a time to prevent cacophony.

## Solution

### Ambient categories

Define a set of ambient categories, each representing an independent "slot" that can hold one looping track:

```rust
#[derive(Clone, Copy, Hash, Eq, PartialEq)]
pub enum AmbientCategory {
    Weather,    // wind, rain, storm
    Interior,   // cave drips, ship engine hum
    Liquid,     // ocean waves, underwater rumble
    Mechanical, // engine hum, machinery
}
```

### Ambient track state

```rust
use kira::sound::static_sound::StaticSoundHandle;
use kira::tween::Tween;

pub struct AmbientSlot {
    pub category: AmbientCategory,
    pub current: Option<ActiveAmbient>,
}

pub struct ActiveAmbient {
    pub name: String,
    pub handle: StaticSoundHandle,
}

pub struct AmbientManager {
    slots: HashMap<AmbientCategory, AmbientSlot>,
    pub crossfade_duration: f64, // seconds, default: 2.0
}
```

### Seamless looping

Ambient sound files are loaded as `StaticSoundData` with a loop region covering the entire file (or a specified sub-region) using kira's `StaticSoundSettings::loop_region`. Well-authored ambient assets have loop points that avoid clicks; the engine trusts the asset but the loop region ensures the sound never stops on its own.

### Crossfading on environment change

When a game system detects an environment change (e.g., the player's current biome/zone shifts), it calls:

```rust
pub fn set_ambient(
    &mut self,
    manager: &mut NebulaAudioManager,
    category: AmbientCategory,
    name: &str,
    library: &AmbientLibrary,
)
```

1. If the requested `name` is already playing in that category's slot, do nothing.
2. If a different track is playing, fade it out over `crossfade_duration` using a kira `Tween::linear`.
3. Start the new track at volume `0.0` on the ambient sub-track and fade it in over the same duration.
4. Replace the slot's `ActiveAmbient` with the new handle.

### Stopping ambient

`stop_ambient(category)` fades out the current track over `crossfade_duration` and clears the slot. No new track is started.

### Per-category constraint

The `AmbientManager` enforces that each `AmbientCategory` slot holds at most one active track. Starting a new track in a slot automatically fades out the old one.

## Outcome

An `ambient.rs` module in `crates/nebula_audio/src/` exporting `AmbientCategory`, `AmbientSlot`, `AmbientManager`, and an `AmbientLibrary` (analogous to `SoundEffectLibrary` but for looping assets). The `AmbientManager` is registered as an ECS resource.

## Demo Integration

**Demo crate:** `nebula-demo`

Wind loops on the planet surface. Cave ambience plays underground. Sounds crossfade smoothly based on the player's environment.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| kira | 0.10 | `StaticSoundData` loop regions, `Tween` for crossfade volume ramps |
| symphonia | 0.5 | Decoding OGG Vorbis ambient files (used internally by kira) |
| log | 0.4 | Debug-level logging for ambient transitions |

## Unit Tests

| Test Function | Description | Assertion |
|---------------|-------------|-----------|
| `test_ambient_loops_seamlessly` | Start an ambient track with a loop region and let it play beyond the file's natural duration. | The sound handle reports it is still playing after 2x the file's length (i.e., it looped). |
| `test_crossfade_transitions_smoothly` | Start ambient "wind", then call `set_ambient` with "cave_drips" in the same category. Sample the old track's volume and new track's volume at the midpoint of the crossfade. | Old track volume is approximately `0.5`; new track volume is approximately `0.5` (within tolerance). |
| `test_environment_change_triggers_crossfade` | Simulate a biome change from `Weather::Wind` to `Weather::Rain`. | The `AmbientSlot` for `Weather` transitions from "wind" to "rain" and both handles exist during the fade window. |
| `test_only_one_ambient_per_category` | Call `set_ambient(Weather, "wind")` then immediately `set_ambient(Weather, "rain")`. | After the crossfade completes, only one handle is active in the `Weather` slot; the old handle has been stopped. |
| `test_stopping_ambient_fades_out` | Start an ambient track, then call `stop_ambient`. | The handle's volume ramps to `0.0` over the crossfade duration; the slot is `None` after completion. |
| `test_same_track_does_not_restart` | Call `set_ambient(Weather, "wind")` twice in a row. | The second call is a no-op; the handle remains the same instance and no fade is triggered. |
