# Kira Audio Manager

## Problem

A game engine without audio is silent and lifeless. Nebula Engine needs a centralized audio back-end that initializes hardware output, manages multiple independent audio tracks (music, sound effects, ambient, UI), and exposes volume controls -- all while living cleanly inside the existing ECS architecture. The audio system must handle edge cases such as missing or disconnected audio devices without panicking, because players may run headless servers, use remote desktops, or hot-unplug headsets. The chosen back-end must be pure Rust, cross-platform (Linux via ALSA/PulseAudio/PipeWire, Windows via WASAPI, macOS via CoreAudio), and low-latency enough for real-time gameplay feedback.

## Solution

Use **kira 0.10** as the audio engine. Kira provides a high-level `AudioManager` with built-in support for tracks (sub-mixes), tween-based volume changes, and streaming -- exactly the primitives we need.

### Initialization

Create an `AudioManagerConfig` wrapper that stores user-facing settings:

```rust
pub struct AudioConfig {
    pub sample_rate: u32,       // default: 44100
    pub buffer_size: usize,     // default: 1024 frames
    pub master_volume: f64,     // default: 1.0
}
```

At engine startup, attempt to build a kira `AudioManager<DefaultBackend>` with these parameters. If device initialization fails (no audio device, permission denied, etc.), log a warning and set the ECS resource to a `None` variant so that all subsequent `play` calls become silent no-ops rather than panics.

### Track layout

Four sub-tracks are created immediately after the manager initializes:

| Track | Purpose | Default Volume |
|-------|---------|----------------|
| `music` | Background music, crossfaded by story 05 | 1.0 |
| `sfx` | One-shot sound effects (footsteps, explosions) | 1.0 |
| `ambient` | Looping environmental audio (wind, water) | 1.0 |
| `ui` | Interface clicks, notifications | 1.0 |

Each track is a kira `TrackHandle` stored in a `TrackSet` struct. Volume for each track is set independently via kira's `TrackBuilder::volume()` tween, and a master volume multiplier is applied at the top level.

```rust
use kira::manager::{AudioManager, AudioManagerSettings, DefaultBackend};
use kira::track::{TrackBuilder, TrackHandle};

pub struct TrackSet {
    pub music: TrackHandle,
    pub sfx: TrackHandle,
    pub ambient: TrackHandle,
    pub ui: TrackHandle,
}

pub struct NebulaAudioManager {
    pub manager: AudioManager<DefaultBackend>,
    pub tracks: TrackSet,
    pub config: AudioConfig,
}
```

### ECS integration

`NebulaAudioManager` (or `Option<NebulaAudioManager>` when no device is available) is inserted as a global ECS resource during the engine initialization phase, making it accessible to any system that needs to play or control audio. Systems obtain a mutable reference through the standard ECS resource query pattern.

### Volume control

- `set_master_volume(vol: f64)` -- adjusts the main output of the kira manager.
- `set_track_volume(track: TrackCategory, vol: f64)` -- adjusts an individual track via its handle.
- All volume values are clamped to `[0.0, 1.0]`.

## Outcome

An `audio_manager.rs` module in `crates/nebula_audio/src/` exporting `AudioConfig`, `TrackCategory`, `TrackSet`, and `NebulaAudioManager`. The manager is registered as an ECS resource at startup and provides the foundation that all subsequent audio stories build upon.

## Demo Integration

**Demo crate:** `nebula-demo`

Kira audio backend initializes at startup. A brief startup chime plays confirming the audio pipeline is alive.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| kira | 0.10 | Audio engine: manager, tracks, playback, tweens |
| serde | 1.0 | Serialize/Deserialize `AudioConfig` for settings persistence |
| log | 0.4 | Warn-level logging when audio device is unavailable |

## Unit Tests

| Test Function | Description | Assertion |
|---------------|-------------|-----------|
| `test_audio_manager_initializes` | Construct a `NebulaAudioManager` with default `AudioConfig` on a system that has an audio device (CI may use a virtual sink). | Manager is `Some` and no panic occurs. |
| `test_tracks_created_for_each_category` | After initialization, inspect the `TrackSet`. | All four track handles (`music`, `sfx`, `ambient`, `ui`) are present and valid. |
| `test_volume_defaults_to_one` | Read the `AudioConfig` from a freshly created manager. | `config.master_volume` equals `1.0`; each track's initial volume equals `1.0`. |
| `test_missing_audio_device_does_not_crash` | Force initialization with an invalid backend configuration (or mock). | Result is `None` (or a graceful error variant), no panic. |
| `test_manager_accessible_as_ecs_resource` | Insert a `NebulaAudioManager` into an ECS world, then retrieve it via resource query. | The retrieved resource is the same instance and its tracks are intact. |
| `test_set_master_volume_clamps` | Call `set_master_volume(1.5)` then `set_master_volume(-0.3)`. | Volume is clamped to `1.0` and `0.0` respectively. |
| `test_set_track_volume_independent` | Set `sfx` track to `0.5`, leave others at `1.0`. | `sfx` volume reads `0.5`; `music`, `ambient`, `ui` volumes still read `1.0`. |
