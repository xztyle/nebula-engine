# Coordinate Frame Transitions

## Problem

The coordinate space hierarchy (Story 01) defines five spaces, and the sector system (Story 02) handles Universe-to-Sector decomposition. But subsystems routinely need to move positions between non-adjacent spaces: the renderer needs Universe-to-Local, terrain generation needs Universe-to-Planet, and networking needs Universe-to-Sector. If each conversion is a standalone free function, the call sites become brittle chains of function calls where the programmer must remember the correct order and manually thread intermediate values. Worse, composite conversions (Universe-to-Local = Universe-to-Sector + Sector-to-Planet + Planet-to-Chunk + Chunk-to-Local) must be rewritten everywhere they are used, and there is no way to precompute or cache a composed transition. The engine needs a composable, type-safe transition abstraction that encodes the source and target space in the type, carries the offset/origin data needed for the conversion, and supports chaining with zero runtime overhead.

## Solution

### The FrameTransition Struct

A `FrameTransition<From, To>` is a struct parameterized by two coordinate space marker types. It holds whatever origin data is needed to perform the conversion. Because each transition has different data requirements, we define concrete transition types and unify them behind a common trait:

```rust
use std::marker::PhantomData;

/// Trait for any object that can convert a position from space `From` to space `To`.
pub trait Transition<From, To> {
    type Input;
    type Output;

    /// Convert a position from the source space to the target space.
    fn apply(&self, pos: &Pos<From, Self::Input>) -> Pos<To, Self::Output>;
}
```

### Concrete Transition Types

**Universe to Sector** — No origin needed; this is a pure bitwise decomposition:

```rust
pub struct UniverseToSector;

impl Transition<UniverseSpace, SectorSpace> for UniverseToSector {
    type Input = IVec3_128;
    type Output = SectorCoord;

    fn apply(&self, pos: &Pos<UniverseSpace, IVec3_128>) -> Pos<SectorSpace, SectorCoord> {
        Pos::new(SectorCoord::from_world(&pos.value))
    }
}
```

**Sector to Planet** — Requires the planet's universe-space origin, decomposed into sector coordinates:

```rust
pub struct SectorToPlanet {
    /// The planet center's sector coordinate.
    pub planet_origin: SectorCoord,
}

impl Transition<SectorSpace, PlanetSpace> for SectorToPlanet {
    type Input = SectorCoord;
    type Output = IVec3_64;

    fn apply(&self, pos: &Pos<SectorSpace, SectorCoord>) -> Pos<PlanetSpace, IVec3_64> {
        // Reconstruct both positions to i128, subtract, truncate to i64.
        let world = pos.value.to_world();
        let origin = self.planet_origin.to_world();
        let delta = world - origin;
        Pos::new(IVec3_64::new(
            delta.x as i64,
            delta.y as i64,
            delta.z as i64,
        ))
    }
}
```

**Planet to Chunk** — Requires the chunk's origin in planet space:

```rust
pub struct PlanetToChunk {
    /// The chunk's origin corner in planet-space millimeters.
    pub chunk_origin: IVec3_64,
}

impl Transition<PlanetSpace, ChunkSpace> for PlanetToChunk {
    type Input = IVec3_64;
    type Output = UVec3_32;

    fn apply(&self, pos: &Pos<PlanetSpace, IVec3_64>) -> Pos<ChunkSpace, UVec3_32> {
        let local = pos.value - self.chunk_origin;
        Pos::new(UVec3_32::new(
            local.x as u32,
            local.y as u32,
            local.z as u32,
        ))
    }
}
```

**Chunk to Local (Camera)** — Requires the camera's position in chunk space (or equivalently, the chunk origin in camera space):

```rust
pub struct ChunkToLocal {
    /// The chunk's origin expressed in camera-local meters (f32).
    pub chunk_origin_in_camera: glam::Vec3,
}

impl Transition<ChunkSpace, LocalSpace> for ChunkToLocal {
    type Input = UVec3_32;
    type Output = glam::Vec3;

    fn apply(&self, pos: &Pos<ChunkSpace, UVec3_32>) -> Pos<LocalSpace, glam::Vec3> {
        let meters = glam::Vec3::new(
            pos.value.x as f32 * MM_TO_METERS,
            pos.value.y as f32 * MM_TO_METERS,
            pos.value.z as f32 * MM_TO_METERS,
        );
        Pos::new(self.chunk_origin_in_camera + meters)
    }
}
```

**Any to Camera (shortcut)** — A direct Universe-to-Local transition for convenience, used when intermediate spaces are not needed:

```rust
pub struct UniverseToLocal {
    /// The camera's universe-space position.
    pub camera_pos: IVec3_128,
}

impl Transition<UniverseSpace, LocalSpace> for UniverseToLocal {
    type Input = IVec3_128;
    type Output = glam::Vec3;

    fn apply(&self, pos: &Pos<UniverseSpace, IVec3_128>) -> Pos<LocalSpace, glam::Vec3> {
        let delta = pos.value - self.camera_pos;
        Pos::new(glam::Vec3::new(
            delta.x as f32 * MM_TO_METERS,
            delta.y as f32 * MM_TO_METERS,
            delta.z as f32 * MM_TO_METERS,
        ))
    }
}
```

### Composition: `then`

Two transitions can be composed if the output space of the first matches the input space of the second:

```rust
/// A composed transition that applies `first` then `second`.
pub struct Composed<A, B, Mid, First, Second>
where
    First: Transition<A, Mid>,
    Second: Transition<Mid, B>,
{
    pub first: First,
    pub second: Second,
    _phantom: PhantomData<(A, B, Mid)>,
}

impl<A, B, Mid, First, Second> Transition<A, B> for Composed<A, B, Mid, First, Second>
where
    First: Transition<A, Mid>,
    Second: Transition<Mid, B, Input = First::Output>,
{
    type Input = First::Input;
    type Output = Second::Output;

    fn apply(&self, pos: &Pos<A, Self::Input>) -> Pos<B, Self::Output> {
        let mid = self.first.apply(pos);
        self.second.apply(&mid)
    }
}
```

A convenience method on all `Transition` implementors:

```rust
/// Extension trait providing the `then` combinator.
pub trait TransitionExt<From, Mid>: Transition<From, Mid> + Sized {
    fn then<To, Next>(self, next: Next) -> Composed<From, To, Mid, Self, Next>
    where
        Next: Transition<Mid, To, Input = Self::Output>,
    {
        Composed {
            first: self,
            second: next,
            _phantom: PhantomData,
        }
    }
}

impl<T, From, Mid> TransitionExt<From, Mid> for T where T: Transition<From, Mid> {}
```

Usage:

```rust
let universe_to_chunk = UniverseToSector
    .then(SectorToPlanet { planet_origin })
    .then(PlanetToChunk { chunk_origin });

let chunk_pos: Pos<ChunkSpace, UVec3_32> = universe_to_chunk.apply(&world_pos);
```

### Inverse Transitions

Each concrete transition type optionally implements an `Invertible` trait:

```rust
pub trait Invertible<From, To>: Transition<From, To> {
    type Inverse: Transition<To, From>;
    fn inverse(&self) -> Self::Inverse;
}
```

For example, `UniverseToSector` is trivially invertible (call `SectorCoord::to_world`). Composed transitions are invertible if both components are, by applying the inverses in reverse order.

### Identity Transition

```rust
pub struct Identity<S>(PhantomData<S>);

impl<S, T: Clone> Transition<S, S> for Identity<S> {
    type Input = T;
    type Output = T;

    fn apply(&self, pos: &Pos<S, T>) -> Pos<S, T> {
        Pos::new(pos.value.clone())
    }
}
```

This is useful as the base case for fold-style composition and for testing.

## Outcome

The `nebula-coords` crate exports the `Transition` trait, five concrete transition structs (one per adjacent-space pair plus `UniverseToLocal`), the `Composed` combinator, the `Identity` transition, and the `Invertible` trait. Any subsystem can build a transition pipeline at initialization time, store it, and apply it to positions in a hot loop without allocations. The type system guarantees that only compatible transitions can be composed. Running `cargo test -p nebula-coords` passes all transition tests.

## Demo Integration

**Demo crate:** `nebula-demo`

The demo converts between coordinate frames each tick and validates that the round-tripped position matches the original, displaying intermediate values.

## Crates & Dependencies

- **`nebula-math`** (workspace) — Integer vector types
- **`glam`** 0.29 — f32 vector types for `LocalSpace` output
- No other external dependencies; composition is pure generics with zero-cost abstraction

## Unit Tests

- **`test_identity_transition`** — Create an `Identity<UniverseSpace>` and apply it to a `Pos<UniverseSpace, IVec3_128>` with value `(100, 200, 300)`. Assert the output value equals the input value exactly.

- **`test_compose_two_transitions_equals_direct`** — Create a `Pos<UniverseSpace, IVec3_128>` with known coordinates. Apply `UniverseToSector` followed by `SectorToPlanet` as two separate calls and record the result. Then compose them with `.then()` and apply the composed transition. Assert both results are identical.

- **`test_compose_full_chain`** — Compose all four transitions: `UniverseToSector.then(SectorToPlanet).then(PlanetToChunk).then(ChunkToLocal)`. Apply to a known universe position. Assert the output type is `Pos<LocalSpace, glam::Vec3>` and the value is within `f32::EPSILON` of the expected camera-relative position.

- **`test_inverse_transition_roundtrips`** — Create a `UniverseToSector` transition. Apply it to a position, then apply `inverse()` to the result. Assert the final position equals the original.

- **`test_composed_inverse_roundtrips`** — Compose `UniverseToSector.then(SectorToPlanet)`. Apply to a position, then apply the inverse of the composed transition. Assert the result equals the original.

- **`test_camera_transition_produces_f32`** — Create a `ChunkToLocal` transition with `chunk_origin_in_camera = Vec3::new(10.0, 20.0, 30.0)`. Apply to `Pos<ChunkSpace, UVec3_32>` with value `(1000, 2000, 3000)` (1m, 2m, 3m in mm). Assert the output is `Pos<LocalSpace, Vec3>` with value approximately `(11.0, 22.0, 33.0)`.

- **`test_transition_type_safety`** — A compile-fail test (trybuild): attempt to compose `UniverseToSector.then(PlanetToChunk)`. This must fail because `UniverseToSector` outputs `SectorSpace` but `PlanetToChunk` expects `PlanetSpace` as input. The compiler rejects the mismatched spaces.

- **`test_then_is_associative`** — Compose `(A.then(B)).then(C)` and `A.then(B.then(C))` (using intermediate type aliases to satisfy the type checker). Apply both to the same input and assert identical output, verifying that composition order does not affect the result.
