# Coordinate Space Hierarchy

## Problem

A game engine that models an entire universe in 128-bit integer space must deal with wildly different scales simultaneously: sub-millimeter voxel detail at the player's feet and inter-stellar distances measured in light-years overhead. If every system operates directly on raw `i128` triples, two categories of bugs become inevitable. First, precision bugs: a rendering system that converts a 128-bit world position to `f32` for the GPU will lose dozens of bits of mantissa, causing distant objects to jitter or collapse to the origin. Second, semantic bugs: nothing in the type system prevents a developer from accidentally passing a chunk-local offset into a function that expects a universe-absolute position, producing silent garbage. The engine needs a formally defined hierarchy of coordinate spaces, each with an explicit precision range and a documented conversion path to its neighbors. Every subsystem must declare which space it operates in, and the compiler must reject cross-space arithmetic at compile time rather than at runtime.

## Solution

### Coordinate Spaces

Define the following coordinate spaces, ordered from largest to smallest:

1. **Universe Space** — Absolute 128-bit integer coordinates. The unit is 1 mm. The origin is an arbitrary fixed point (e.g., the center of the starting solar system). Range: roughly +/-1.7x10^38 mm, enough to model a universe-scale volume. Type alias: `WorldPosition = IVec3_128`. Used by: persistence layer, sector addressing, spatial hashing, entity canonical positions.

2. **Sector Space** — A two-part coordinate: `(SectorIndex: IVec3_96, LocalOffset: IVec3_32)`. The sector index identifies which 2^32 mm (~4,295 km) cube the position falls in; the local offset gives the position within that cube in millimeters. Used by: chunk management, networking (sector-based interest management), broad-phase spatial queries.

3. **Planet Space** — Position relative to a planet's center, stored as `IVec3_64`. The unit is 1 mm. Range: +/-9.2x10^15 mm (~9.2 billion km), which comfortably exceeds any planet or star radius. Used by: cubesphere face selection, planet-relative physics (gravity direction), terrain generation seed inputs.

4. **Chunk Space** — Position relative to a chunk's origin corner, stored as `UVec3_32` (unsigned, since chunks extend in the positive direction from their origin). Unit is 1 mm, so a 32x32x32-meter chunk spans 0..32,000 on each axis (well within u32 range). Used by: voxel indexing, meshing, ambient occlusion calculation, per-chunk collision shapes.

5. **Local/Camera Space** — Position relative to the camera, stored as `Vec3` (f32). The camera is always at the origin of this space, so all positions are small enough that f32 precision suffices (sub-millimeter accuracy within several kilometers). Used by: GPU vertex shaders, frustum culling (near-field), audio spatialization, particle systems.

### Type-Level Space Tagging

Introduce a zero-sized marker enum and a trait that tags position types with their coordinate space:

```rust
/// Marker enum for coordinate spaces. Each variant is a zero-sized type
/// used purely at the type level.
pub enum Space {
    Universe,
    Sector,
    Planet,
    Chunk,
    Local,
}

/// A position tagged with its coordinate space. The space parameter S is
/// a const generic or phantom type that prevents mixing.
pub struct Position<const S: Space> {
    // The inner storage type varies by space, but for the generic wrapper
    // we store the widest representation and rely on From/Into impls.
    pub inner: PositionStorage<S>,
}
```

Because Rust const generics on enums are not yet stable, the practical implementation uses phantom types instead:

```rust
use std::marker::PhantomData;

pub struct UniverseSpace;
pub struct SectorSpace;
pub struct PlanetSpace;
pub struct ChunkSpace;
pub struct LocalSpace;

/// A position value tagged with a coordinate space marker.
/// `S` is a zero-sized type that exists only at compile time.
pub struct Pos<S, T> {
    pub value: T,
    _space: PhantomData<S>,
}

impl<S, T> Pos<S, T> {
    pub fn new(value: T) -> Self {
        Self { value, _space: PhantomData }
    }
}
```

With this design, `Pos<UniverseSpace, IVec3_128>` and `Pos<ChunkSpace, UVec3_32>` are distinct types. Attempting to pass one where the other is expected produces a compile error, not a runtime bug.

### The `InSpace<S>` Trait

```rust
/// Trait implemented by any value that exists within a specific coordinate space.
/// Provides the canonical storage type and accessors.
pub trait InSpace<S> {
    /// The concrete vector type used in this space.
    type Storage;

    /// Return a reference to the underlying coordinate data.
    fn coords(&self) -> &Self::Storage;

    /// Return the coordinate space as a runtime-inspectable value (for debug).
    fn space_name() -> &'static str;
}
```

Blanket implementations for each space:

```rust
impl InSpace<UniverseSpace> for Pos<UniverseSpace, IVec3_128> {
    type Storage = IVec3_128;
    fn coords(&self) -> &IVec3_128 { &self.value }
    fn space_name() -> &'static str { "Universe" }
}

impl InSpace<LocalSpace> for Pos<LocalSpace, glam::Vec3> {
    type Storage = glam::Vec3;
    fn coords(&self) -> &glam::Vec3 { &self.value }
    fn space_name() -> &'static str { "Local" }
}
// ... and so on for Sector, Planet, Chunk.
```

### Conversion Chain

The canonical conversion chain is:

```
Universe ──► Sector ──► Planet ──► Chunk ──► Local/Camera
   ◄──          ◄──        ◄──       ◄──
```

Each arrow is a dedicated function that requires the origin/offset of the target space as an argument:

```rust
/// Convert a universe-space position to sector-space.
pub fn universe_to_sector(pos: &Pos<UniverseSpace, IVec3_128>) -> Pos<SectorSpace, SectorCoord> {
    let sector_index = IVec3_96::new(
        (pos.value.x >> 32) as i96,
        (pos.value.y >> 32) as i96,
        (pos.value.z >> 32) as i96,
    );
    let local_offset = IVec3_32::new(
        (pos.value.x & 0xFFFF_FFFF) as i32,
        (pos.value.y & 0xFFFF_FFFF) as i32,
        (pos.value.z & 0xFFFF_FFFF) as i32,
    );
    Pos::new(SectorCoord { sector_index, local_offset })
}

/// Convert a universe-space position to camera-local f32 space.
/// `camera_pos` is the camera's universe-space position.
pub fn universe_to_local(
    pos: &Pos<UniverseSpace, IVec3_128>,
    camera_pos: &Pos<UniverseSpace, IVec3_128>,
) -> Pos<LocalSpace, glam::Vec3> {
    let delta = pos.value - camera_pos.value;
    Pos::new(glam::Vec3::new(
        delta.x as f32 * MM_TO_METERS,
        delta.y as f32 * MM_TO_METERS,
        delta.z as f32 * MM_TO_METERS,
    ))
}
```

Skipping intermediate spaces (e.g., Universe -> Local directly) is permitted for convenience, but the implementation must internally follow the chain to maintain correctness.

### Subsystem-to-Space Mapping

| Subsystem | Primary Space | Notes |
|---|---|---|
| Persistence / Save files | Universe | Canonical position of record |
| Networking / Replication | Sector | Sector-based interest management |
| Terrain generation | Planet | Noise seeded from planet-relative coords |
| Voxel storage & meshing | Chunk | All indexing is chunk-local |
| Rendering (vertex shader) | Local/Camera | Camera at origin, f32 precision |
| Physics (Rapier) | Local/Camera | Rapier uses f32 internally |
| Frustum culling (far) | Universe | Coarse 128-bit frustum for planets/stars |
| Frustum culling (near) | Local/Camera | Standard f32 frustum for chunks |
| Audio spatialization | Local/Camera | Listener at origin |
| Spatial hash (entities) | Sector | O(1) lookup by sector key |

## Outcome

The `nebula-coords` crate exports a `CoordinateSpace` enum with five variants, five zero-sized marker types (`UniverseSpace`, `SectorSpace`, `PlanetSpace`, `ChunkSpace`, `LocalSpace`), the generic `Pos<S, T>` wrapper, the `InSpace<S>` trait, and conversion functions between all adjacent spaces. A developer can write a function like `fn apply_gravity(pos: &Pos<PlanetSpace, IVec3_64>)` and the compiler rejects any attempt to pass a universe-space or chunk-space position. The conversion chain compiles and roundtrips correctly. Running `cargo test -p nebula-coords` passes all coordinate space tests.

## Demo Integration

**Demo crate:** `nebula-demo`

The demo decomposes its world position into the five coordinate spaces and displays sector, chunk, and local levels in the window title.

## Crates & Dependencies

- **`nebula-math`** (workspace) — Provides `IVec3_128`, `IVec3_64`, `IVec3_32`, `UVec3_32` types
- **`glam`** 0.29 — f32/f64 vector types for local/camera space
- No other external dependencies; the space tagging is pure Rust generics

## Unit Tests

- **`test_coordinate_space_enum_has_five_variants`** — Instantiate all five variants of the `CoordinateSpace` enum (`Universe`, `Sector`, `Planet`, `Chunk`, `Local`) and assert that a match expression covering all five compiles without a wildcard arm. This confirms no variant was accidentally added or removed.

- **`test_pos_type_safety_prevents_mixing`** — Create a `Pos<UniverseSpace, IVec3_128>` and a `Pos<ChunkSpace, UVec3_32>`. Attempt to assign one to a binding typed as the other. This is a compile-time test (trybuild or `compile_fail` doctest): the code must fail to compile, proving that the type system rejects cross-space substitution.

- **`test_in_space_returns_correct_space_name`** — For each of the five blanket impls, call `space_name()` and assert the returned `&str` matches the expected name (e.g., `"Universe"`, `"Sector"`, `"Planet"`, `"Chunk"`, `"Local"`).

- **`test_conversion_chain_compiles_universe_to_local`** — Call `universe_to_sector`, then `sector_to_planet`, then `planet_to_chunk`, then `chunk_to_local` in sequence, passing the output of each as input to the next. Assert the final result is a `Pos<LocalSpace, glam::Vec3>`. The test primarily validates that the type chain compiles without error.

- **`test_roundtrip_universe_sector_universe`** — Create a `Pos<UniverseSpace, IVec3_128>` with known coordinates (e.g., `(1_000_000, -5_000_000_000, 42)`). Convert to sector space and back to universe space. Assert the result equals the original position exactly (no precision loss, since both directions are integer arithmetic).

- **`test_local_space_values_are_f32`** — Convert a universe position near the camera to local space. Assert that the result's `value` field is a `glam::Vec3` and that the coordinates are within expected f32 range (e.g., less than 10,000.0 meters from camera).

- **`test_subsystem_space_annotations`** — Define a mock function `fn render_chunk(pos: Pos<ChunkSpace, UVec3_32>)` and a mock function `fn save_entity(pos: Pos<UniverseSpace, IVec3_128>)`. Call each with the correctly-typed position and assert compilation succeeds. This is a compile-time correctness test.
