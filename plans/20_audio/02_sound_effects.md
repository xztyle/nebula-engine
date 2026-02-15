# Sound Effects

## Problem

Gameplay depends on immediate audio feedback for player actions and world events -- the crunch of mining a voxel, the thud of footsteps on stone, the boom of an explosion. These one-shot sounds must load quickly, play with minimal latency, and support small randomized variations (pitch shifts) so that repetitive actions do not sound robotic. Without a concurrency limit, a rapid-fire event (e.g., walking over gravel) can spawn hundreds of overlapping instances, saturating the mixer and producing distortion. The engine needs a managed sound-effect layer that loads, caches, and plays SFX through the kira back-end established in Story 01.

## Solution

### Sound effect library

Introduce a `SoundEffectLibrary` resource that maps string identifiers to pre-loaded kira `StaticSoundData` handles. Sound files are loaded at asset-loading time (OGG via symphonia, WAV natively) and stored in memory for instant playback.

```rust
use std::collections::HashMap;
use kira::sound::static_sound::StaticSoundData;

pub struct SoundEffectEntry {
    pub data: StaticSoundData,
    pub max_concurrent: usize,  // default: 8
    pub active_count: usize,
}

pub struct SoundEffectLibrary {
    effects: HashMap<String, SoundEffectEntry>,
}
```

### Loading

`SoundEffectLibrary::load(name: &str, path: &Path)` reads the file from disk, decodes it into a `StaticSoundData` via kira's built-in symphonia integration (OGG Vorbis, WAV), and stores the entry. If a file cannot be loaded, a warning is logged and the entry is skipped -- callers that request an unknown or failed name receive a logged warning and a silent no-op.

### Playback

```rust
pub fn play_sfx(
    &mut self,
    manager: &mut NebulaAudioManager,
    name: &str,
    volume: f64,          // 0.0..=1.0, multiplied with SFX track volume
    pitch_variation: f64, // e.g., 0.05 means +/- 5% random shift
) -> Option<StaticSoundHandle>
```

1. Look up the `SoundEffectEntry` by `name`. If missing, log a warning and return `None`.
2. Check `active_count` against `max_concurrent`. If at the limit, skip playback (or steal the oldest instance).
3. Clone the `StaticSoundData`, apply a random playback rate between `1.0 - pitch_variation` and `1.0 + pitch_variation` using a thread-local RNG.
4. Set the output track to the SFX sub-track from Story 01.
5. Set volume on the sound data.
6. Play through the kira `AudioManager` and increment `active_count`. Register a callback or poll to decrement `active_count` when the sound finishes.

### Pitch variation

A small random pitch shift (default +/- 3--5%) applied per play call ensures that repeated sounds (e.g., consecutive footsteps) feel natural. The variation range is configurable per effect entry.

## Outcome

A `sound_effects.rs` module in `crates/nebula_audio/src/` exporting `SoundEffectLibrary`, `SoundEffectEntry`, and the `play_sfx` function. The library is registered as an ECS resource alongside the `NebulaAudioManager`.

## Demo Integration

**Demo crate:** `nebula-demo`

Breaking and placing voxels play distinct sound effects. Footstep sounds accompany player movement, varying by surface type.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| kira | 0.10 | `StaticSoundData`, `StaticSoundSettings`, playback rate tweens |
| symphonia | 0.5 | Decoding OGG Vorbis and other compressed formats (used internally by kira) |
| rand | 0.8 | Thread-local RNG for pitch variation |
| log | 0.4 | Warning-level logging for missing sound names or load failures |

## Unit Tests

| Test Function | Description | Assertion |
|---------------|-------------|-----------|
| `test_sfx_plays_without_error` | Load a short WAV test file into the library and call `play_sfx`. | Returns `Some(handle)` and no error is logged. |
| `test_pitch_variation_produces_different_pitches` | Call `play_sfx` 20 times with `pitch_variation = 0.1` and record the effective playback rate of each handle. | Not all playback rates are identical; values fall within `[0.9, 1.1]`. |
| `test_concurrent_limit_enforced` | Set `max_concurrent` to 3 for an effect. Call `play_sfx` 5 times rapidly. | Only 3 handles are returned as `Some`; the remaining 2 are `None` (or oldest stolen). |
| `test_unknown_sound_name_logged_as_warning` | Call `play_sfx("nonexistent", ...)` with a test logger. | Returns `None`; the log contains a warning mentioning the unknown name. |
| `test_volume_scales_correctly` | Play the same effect at volume `0.0`, `0.5`, and `1.0`. | The `StaticSoundData` volume setting on each handle matches the requested value (within f64 epsilon). |
| `test_load_ogg_file` | Load an OGG Vorbis test file via `SoundEffectLibrary::load`. | Entry is present in the library and `data` is valid. |
| `test_load_invalid_file_does_not_panic` | Attempt to load a non-existent path. | No panic; the entry is absent from the library and a warning is logged. |
