# Sound Occlusion

## Problem

In a voxel world, players can hear sounds through solid walls, which breaks immersion. A mining operation on the other side of a mountain should sound muffled, not crystal clear. An explosion underground should be felt more than heard on the surface. Without occlusion, every sound within range plays at full clarity regardless of intervening geometry, making it impossible for the player to judge whether a threat is in the open or behind cover. The engine needs a sound occlusion system that uses the voxel data already available to determine how much solid material lies between the listener and each sound source, and applies appropriate filtering.

## Solution

### Voxel raycasting for occlusion

Reuse the voxel raycasting infrastructure from Epic 17 (physics). For each active `SpatialSound` (from Story 04), cast a ray from the `AudioListener` position to the sound's position in local voxel space. Count the number of solid voxel cells the ray passes through. This count becomes the **occlusion factor**.

```rust
pub struct OcclusionResult {
    pub solid_voxels_hit: u32,
    pub total_distance: f32,
}

pub fn compute_occlusion(
    listener_pos: [i128; 3],
    sound_pos: [i128; 3],
    voxel_world: &VoxelWorld,
) -> OcclusionResult
```

The ray steps through the voxel grid using a DDA (Digital Differential Analyzer) traversal, the same algorithm used for physics raycasts. Each cell the ray enters is tested for solidity. The traversal terminates when it reaches the sound source or exceeds a configurable maximum distance (to avoid expensive casts for distant sounds that are already quiet).

### Low-pass filter application

The occlusion factor maps to a low-pass filter cutoff frequency:

| Solid Voxels Hit | Filter Cutoff | Perceptual Effect |
|-------------------|---------------|-------------------|
| 0 | 20,000 Hz (no filter) | Clear, unoccluded |
| 1--2 | ~4,000 Hz | Slightly muffled (thin wall) |
| 3--5 | ~1,500 Hz | Noticeably muffled (thick wall) |
| 6--10 | ~600 Hz | Heavily muffled (multiple rooms) |
| 11+ | ~200 Hz | Deep bass only (underground) |

The mapping uses a logarithmic curve: `cutoff = 20000.0 / (1.0 + occlusion_factor as f32 * occlusion_strength)`, clamped to a minimum of `200 Hz`. The `occlusion_strength` parameter is configurable (default: `3.0`).

Kira supports applying effects to tracks or individual sounds. The filter is applied per-sound by routing occluded sounds through a dynamically adjusted filter effect. Alternatively, a volume reduction proportional to occlusion may be layered on top for efficiency on lower-end hardware.

### Per-frame update

An ECS system (`sound_occlusion_update`) runs after `spatial_audio_update`:

1. For each entity with a `SpatialSound` and an `OcclusionState` component, cast the occlusion ray.
2. Smooth the occlusion factor over several frames (exponential moving average, time constant ~0.1s) to prevent popping when the player peeks around a corner.
3. Update the sound's filter cutoff and/or volume reduction.

### Performance considerations

Raycasting every spatial sound every frame can be expensive. Mitigations:
- Only cast for sounds within audible range (after distance attenuation, skip sounds at near-zero volume).
- Stagger casts across frames (update 1/4 of sounds per frame in round-robin).
- Use a coarser voxel LOD for long-distance occlusion queries.

## Outcome

An `occlusion.rs` module in `crates/nebula_audio/src/` exporting `OcclusionResult`, `OcclusionState` (per-entity component), `OcclusionConfig`, and the `sound_occlusion_update` system. Depends on the voxel raycasting module from `crates/nebula_physics/`.

## Demo Integration

**Demo crate:** `nebula-demo`

Sound is muffled when a solid mountain or wall stands between the listener and the source. Occlusion is computed from voxel raycast.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| kira | 0.10 | Per-sound filter effects, volume adjustment |
| glam | 0.29 | Vector math for ray direction computation |
| log | 0.4 | Trace-level logging for occlusion debugging |

## Unit Tests

| Test Function | Description | Assertion |
|---------------|-------------|-----------|
| `test_unoccluded_sound_is_clear` | Listener and sound in the same open-air space with no voxels between them. | `solid_voxels_hit` is `0`; filter cutoff is `20000` Hz (effectively no filter). |
| `test_sound_through_one_wall_is_muffled` | Place a single solid voxel layer between listener and sound. | `solid_voxels_hit` is `1`; filter cutoff is approximately `5000` Hz. |
| `test_sound_through_many_walls_heavily_muffled` | Place 10 solid voxels between listener and sound. | `solid_voxels_hit` is `10`; filter cutoff is below `700` Hz. |
| `test_sound_in_same_room_is_clear` | Listener and sound in the same enclosed voxel room (walls around both, but line of sight is clear). | `solid_voxels_hit` is `0`; no muffling applied. |
| `test_occlusion_updates_as_voxels_change` | Initially 3 solid voxels between listener and sound. Remove 2 voxels (mining). Re-run occlusion. | `solid_voxels_hit` drops to `1`; filter cutoff increases toward clear. |
| `test_occlusion_smoothing_prevents_popping` | Alternate between 0 and 5 occlusion hits on consecutive frames. | The effective filter cutoff moves gradually (exponential moving average), never jumping instantly. |
| `test_distant_sounds_skip_raycast` | Sound is beyond `max_distance` (volume already `0.0` from spatial attenuation). | The occlusion raycast is not performed (performance optimization). |
