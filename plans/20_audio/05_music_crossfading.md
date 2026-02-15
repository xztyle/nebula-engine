# Music Crossfading

## Problem

Background music is fundamental to setting the emotional tone of a game. Nebula Engine needs a music system that plays contextual soundtracks -- calm exploration themes on a planet surface, tense combat music when enemies appear, sweeping orchestral tracks while traveling through space, and menu music on the title screen. Transitions between music states must be smooth crossfades, not jarring cuts. The music system must maintain its own volume independent of SFX and ambient audio, and support playlists (multiple tracks per state) so that players do not hear the same song on repeat for hours.

## Solution

### Music states

Define an enum of gameplay contexts that drive music selection:

```rust
#[derive(Clone, Copy, Hash, Eq, PartialEq)]
pub enum MusicState {
    Menu,
    Exploration,
    Combat,
    SpaceTravel,
    Building,
    Custom(u32), // extensible for mods/scripting
}
```

### Playlist registry

A `MusicPlaylistRegistry` maps each `MusicState` to a list of track asset paths. When a state becomes active, the system picks the next track from the list in a shuffled order, ensuring variety.

```rust
pub struct MusicPlaylist {
    pub tracks: Vec<String>,      // asset paths
    pub shuffle_order: Vec<usize>, // pre-shuffled indices
    pub current_index: usize,
}

pub struct MusicPlaylistRegistry {
    playlists: HashMap<MusicState, MusicPlaylist>,
}
```

When the playlist reaches the end, it reshuffles (Fisher-Yates) and restarts, avoiding playing the same track twice in a row by ensuring the last track of the previous cycle is not the first track of the new cycle.

### Music manager

```rust
pub struct MusicManager {
    pub current_state: Option<MusicState>,
    pub current_handle: Option<StaticSoundHandle>,
    pub next_handle: Option<StaticSoundHandle>,
    pub crossfade_duration: f64, // seconds, default: 3.0
    pub crossfade_timer: f64,
}
```

### State transition flow

When a game system sets a new `MusicState`:

1. If the new state equals `current_state`, do nothing (the current track continues).
2. Select the next track from the new state's playlist.
3. Load the track as `StaticSoundData` and begin playback on the **music** sub-track (from Story 01) at volume `0.0`.
4. Begin fading out `current_handle` and fading in `next_handle` over `crossfade_duration` using kira `Tween::linear`.
5. After the fade completes, stop and drop the old handle. Promote `next_handle` to `current_handle`.

### Track-end handling

When a music track finishes naturally (no loop), the system automatically advances to the next track in the playlist with a short crossfade (default: 1.5s), ensuring continuous music within a state.

### Volume independence

Music volume is controlled exclusively through the music sub-track established in Story 01. Changing SFX or ambient volume has zero effect on music. The `MusicManager` respects the track volume set by the player in audio settings.

## Outcome

A `music.rs` module in `crates/nebula_audio/src/` exporting `MusicState`, `MusicPlaylist`, `MusicPlaylistRegistry`, `MusicManager`, and a `music_update` ECS system that handles crossfade timing and track advancement each frame.

## Demo Integration

**Demo crate:** `nebula-demo`

Surface and space music themes crossfade over 3 seconds when transitioning between planet surface and orbit.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| kira | 0.10 | `StaticSoundData` playback, `Tween` for crossfade volume ramps, music track output routing |
| symphonia | 0.5 | Decoding OGG Vorbis music files (used internally by kira) |
| rand | 0.8 | Fisher-Yates shuffle for playlist randomization |
| log | 0.4 | Info-level logging for music state transitions |

## Unit Tests

| Test Function | Description | Assertion |
|---------------|-------------|-----------|
| `test_music_starts_playing` | Set `MusicState::Exploration` with a valid playlist. | `current_handle` is `Some` and the sound is in a playing state. |
| `test_state_change_triggers_crossfade` | Transition from `Exploration` to `Combat`. | Both `current_handle` (fading out) and `next_handle` (fading in) exist during the transition window. |
| `test_crossfade_duration_is_correct` | Set `crossfade_duration` to `2.0` seconds and trigger a state change. Advance the timer by `1.0` seconds. | `crossfade_timer` reads approximately `1.0`; old track volume is approximately `0.5`; new track volume is approximately `0.5`. |
| `test_playlist_shuffles_tracks` | Register a playlist with 5 tracks. Trigger playback, record track order over 5 advancements. Reset and repeat. | The two orderings are not identical (with overwhelming probability for 5 tracks). |
| `test_music_volume_independent_of_sfx` | Set music track volume to `0.8` and SFX track volume to `0.3`. | Music handle's effective volume reflects `0.8`; SFX track volume is `0.3`; neither influences the other. |
| `test_same_state_does_not_restart` | Set `MusicState::Menu` twice in succession. | The second call is a no-op; `current_handle` remains the same instance. |
| `test_track_end_advances_playlist` | Play a very short test track (< 1 second). Wait for it to finish. | The `MusicManager` automatically loads and begins playing the next track in the playlist. |
