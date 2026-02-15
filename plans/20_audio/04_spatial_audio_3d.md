# Spatial Audio 3D

## Problem

A voxel world with mining sounds, explosions, and creature calls needs spatial audio to ground the player in 3D space. Sounds should appear to come from specific locations -- louder when close, quieter when far, panned left or right based on the listener's orientation. The core challenge in Nebula Engine is that world positions use 128-bit integer coordinates (i128 per axis) for planetary-scale precision, but audio calculations require floating-point math in a local coordinate frame. The spatial audio layer must bridge these coordinate systems without precision loss near the listener.

## Solution

### Coordinate conversion

Spatial audio operates entirely in **local f32 space** relative to the camera/listener origin. The listener is always at the origin `(0.0, 0.0, 0.0)` of the audio coordinate frame. Sound emitter positions are computed by subtracting the listener's world `i128` position from the emitter's world `i128` position, then casting the result to `f32`. Because audio is only relevant within a few hundred meters, the i128-to-f32 conversion loses no meaningful precision at these ranges. This reuses the same origin-rebasing strategy employed by the rendering pipeline (Epic 03).

```rust
pub struct AudioListener {
    pub world_pos: [i128; 3],
    pub forward: [f32; 3],  // unit vector, listener facing direction
    pub up: [f32; 3],       // unit vector, listener up direction
}

pub struct SpatialSound {
    pub world_pos: [i128; 3],
    pub handle: StaticSoundHandle,
    pub rolloff: AttenuationRolloff,
    pub max_distance: f32,   // beyond this, volume is 0
    pub ref_distance: f32,   // distance at which volume is 1.0
}
```

### Distance attenuation

Volume is attenuated based on distance from the listener using a configurable rolloff model:

```rust
pub enum AttenuationRolloff {
    Linear,                         // 1.0 - (d - ref) / (max - ref)
    InverseDistance { exponent: f32 }, // ref / (ref + exponent * (d - ref))
    Exponential { exponent: f32 },    // (d / ref).powf(-exponent)
}
```

Default is `InverseDistance { exponent: 1.0 }`, which models natural sound falloff. At distances beyond `max_distance`, the sound is fully silent (volume set to `0.0` via kira tween).

### Panning

Left/right panning is computed from the angle between the listener's forward vector and the direction to the sound source, projected onto the listener's horizontal plane. A sound directly to the left yields a pan value of `-1.0`; directly to the right yields `1.0`; directly ahead or behind yields `0.0`. Kira's `Panning` setting on the sound handle is updated each frame.

### Behind attenuation

Sounds originating behind the listener receive an additional configurable attenuation factor (default: `0.7`) to simulate the natural shadowing effect of the listener's head. This is applied as a multiplier on top of distance-based volume.

### Per-frame update system

An ECS system runs each frame:
1. Read the `AudioListener` resource (set by the camera system).
2. For each entity with a `SpatialSound` component, compute the local-space offset, distance, attenuation, and pan.
3. Update the kira sound handle's volume and panning via tweens with a very short duration (one frame, essentially instant).

## Outcome

A `spatial.rs` module in `crates/nebula_audio/src/` exporting `AudioListener`, `SpatialSound`, `AttenuationRolloff`, and the `spatial_audio_update` system. The system integrates with the ECS and the camera origin established in Epic 03.

## Demo Integration

**Demo crate:** `nebula-demo`

Sound sources are positioned in 3D space. A crackling campfire grows louder when the player faces it and quieter when turning away.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| kira | 0.10 | Per-sound volume and panning control via `StaticSoundHandle` |
| glam | 0.29 | `Vec3` math for direction, distance, dot products |
| log | 0.4 | Debug-level logging for spatial audio diagnostics |

## Unit Tests

| Test Function | Description | Assertion |
|---------------|-------------|-----------|
| `test_sound_at_listener_position_centered_full_volume` | Place a `SpatialSound` at the same world position as the `AudioListener`. | Computed volume is `1.0`; computed pan is `0.0`. |
| `test_distant_sound_is_quieter` | Place a sound at `ref_distance * 5` from the listener using `InverseDistance` rolloff. | Computed volume is significantly less than `1.0` (approximately `0.2` for exponent 1.0). |
| `test_sound_to_the_left_pans_left` | Place a sound directly to the listener's left (perpendicular to forward, in the left direction). | Computed pan value is approximately `-1.0`. |
| `test_sound_behind_is_attenuated` | Place a sound directly behind the listener at `ref_distance`. | Computed volume is `1.0 * behind_factor` (default `0.7`), less than a sound at the same distance in front. |
| `test_attenuation_follows_rolloff_curve` | Place sounds at distances `ref`, `2*ref`, `4*ref`, `8*ref` using `InverseDistance { exponent: 1.0 }`. | Volume values follow the inverse distance formula within f32 epsilon. |
| `test_beyond_max_distance_is_silent` | Place a sound at `max_distance + 10.0`. | Computed volume is `0.0`. |
| `test_i128_to_f32_conversion_accuracy` | Listener at `i128::MAX / 2`; sound at `listener + 100` on one axis. | Local-space offset is `(100.0, 0.0, 0.0)` without precision loss. |
