//! Coordinate spaces (local, chunk, sector, planet, universe), sector addressing, and spatial transformations.
//!
//! This crate provides a type-safe coordinate space hierarchy that prevents accidental mixing
//! of positions from different coordinate systems. Each coordinate space has a specific
//! precision range and documented conversion paths to its neighbors.
//!
//! # Coordinate Spaces
//!
//! 1. **Universe Space** — Absolute 128-bit integer coordinates (1mm precision)
//! 2. **Sector Space** — Two-part coordinate: sector index + local offset
//! 3. **Planet Space** — Position relative to planet center (64-bit integer, 1mm precision)  
//! 4. **Chunk Space** — Position relative to chunk origin (32-bit unsigned, 1mm precision)
//! 5. **Local/Camera Space** — Position relative to camera (f32, sufficient precision)
//!
//! # Type Safety
//!
//! All position types are tagged with their coordinate space using phantom types.
//! The compiler rejects attempts to mix positions from different spaces:
//!
//! ```rust
//! use nebula_coords::{Pos, UniverseSpace, ChunkSpace};
//! use nebula_math::Vec3I128;
//! use glam::UVec3;
//!
//! let universe_pos = Pos::<UniverseSpace, Vec3I128>::new(Vec3I128::new(1000, 2000, 3000));
//! let chunk_pos = Pos::<ChunkSpace, UVec3>::new(UVec3::new(10, 20, 30));
//!
//! // This would not compile:
//! // let mixed: Pos<UniverseSpace, Vec3I128> = chunk_pos; // Error!
//! ```

use std::marker::PhantomData;

use glam::{UVec3, Vec3};

// Re-export commonly used types from nebula-math
pub use nebula_math::{Vec3I128, WorldPosition};

// Additional vector types not in nebula-math
/// 64-bit signed integer 3D vector for planet-space coordinates
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Vec3I64 {
    pub x: i64,
    pub y: i64,
    pub z: i64,
}

impl Vec3I64 {
    pub fn new(x: i64, y: i64, z: i64) -> Self {
        Self { x, y, z }
    }
}

impl std::ops::Sub for Vec3I64 {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        Self::new(self.x - other.x, self.y - other.y, self.z - other.z)
    }
}

impl std::ops::Add for Vec3I64 {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self::new(self.x + other.x, self.y + other.y, self.z + other.z)
    }
}

/// 32-bit signed integer 3D vector for sector local offsets
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Vec3I32 {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl Vec3I32 {
    pub fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }
}

/// 96-bit signed integer 3D vector for sector indices
/// Each component is effectively i32 but stored in i64 for simplicity
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Vec3I96 {
    pub x: i64, // Only uses 32 bits but stored as i64
    pub y: i64,
    pub z: i64,
}

impl Vec3I96 {
    pub fn new(x: i64, y: i64, z: i64) -> Self {
        Self { x, y, z }
    }
}

// Sector Coordinate Types (Plan 03_coords/02)

/// The index of a sector in the universe grid.
/// Each component is the upper 96 bits of the corresponding i128 axis.
/// Stored as three i128 values with the lower 32 bits always zero
/// (or equivalently, as a custom i96 triple).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SectorIndex {
    pub x: i128,
    pub y: i128,
    pub z: i128,
}

/// The local offset within a sector, in millimeters.
/// Range: [0, 2^32 - 1] for each axis when the full i128 coordinate is non-negative.
/// For negative coordinates, the offset is adjusted so it is always non-negative.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectorOffset {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

/// A position decomposed into sector index + local offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectorCoord {
    pub sector: SectorIndex,
    pub offset: SectorOffset,
}

/// Lightweight key type for sector-based hash maps.
/// The hash combines all three axis indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SectorKey(pub SectorIndex);

impl From<&SectorCoord> for SectorKey {
    fn from(coord: &SectorCoord) -> Self {
        SectorKey(coord.sector)
    }
}

// Sector coordinate constants and implementation
const SECTOR_BITS: u32 = 32;
const SECTOR_MASK: i128 = 0xFFFF_FFFF;

impl SectorCoord {
    /// Decompose an absolute world position into sector index + local offset.
    pub fn from_world(pos: &WorldPosition) -> Self {
        SectorCoord {
            sector: SectorIndex {
                // Arithmetic right shift: for -1 >> 32 this gives -1,
                // meaning the sector at index -1 (one sector in the negative direction).
                x: pos.x >> SECTOR_BITS,
                y: pos.y >> SECTOR_BITS,
                z: pos.z >> SECTOR_BITS,
            },
            offset: SectorOffset {
                // Mask off the lower 32 bits. For negative coordinates,
                // this produces a positive offset within the sector.
                // E.g., i128 value -1: sector = -1, offset = 0xFFFFFFFF = 4294967295.
                x: (pos.x & SECTOR_MASK) as i32,
                y: (pos.y & SECTOR_MASK) as i32,
                z: (pos.z & SECTOR_MASK) as i32,
            },
        }
    }

    /// Reconstruct the absolute world position from sector + offset.
    pub fn to_world(&self) -> WorldPosition {
        WorldPosition {
            x: (self.sector.x << SECTOR_BITS) | (self.offset.x as i128 & SECTOR_MASK),
            y: (self.sector.y << SECTOR_BITS) | (self.offset.y as i128 & SECTOR_MASK),
            z: (self.sector.z << SECTOR_BITS) | (self.offset.z as i128 & SECTOR_MASK),
        }
    }
}

// Space marker types (zero-sized)
/// Marker type for universe-space coordinates  
pub struct UniverseSpace;

/// Marker type for sector-space coordinates
pub struct SectorSpace;

/// Marker type for planet-space coordinates
pub struct PlanetSpace;

/// Marker type for chunk-space coordinates
pub struct ChunkSpace;

/// Marker type for local/camera-space coordinates
pub struct LocalSpace;

/// Enumeration of all coordinate spaces for runtime inspection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CoordinateSpace {
    Universe,
    Sector,
    Planet,
    Chunk,
    Local,
}

/// A position value tagged with a coordinate space marker.
/// `S` is a zero-sized type that exists only at compile time.
/// `T` is the storage type (e.g., Vec3I128, Vec3, etc.)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pos<S, T> {
    pub value: T,
    _space: PhantomData<S>,
}

impl<S, T> Pos<S, T> {
    /// Create a new position in the specified coordinate space
    pub fn new(value: T) -> Self {
        Self {
            value,
            _space: PhantomData,
        }
    }
}

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

// Blanket implementations for each space
impl InSpace<UniverseSpace> for Pos<UniverseSpace, nebula_math::Vec3I128> {
    type Storage = nebula_math::Vec3I128;

    fn coords(&self) -> &nebula_math::Vec3I128 {
        &self.value
    }

    fn space_name() -> &'static str {
        "Universe"
    }
}

impl InSpace<LocalSpace> for Pos<LocalSpace, Vec3> {
    type Storage = Vec3;

    fn coords(&self) -> &Vec3 {
        &self.value
    }

    fn space_name() -> &'static str {
        "Local"
    }
}

impl InSpace<PlanetSpace> for Pos<PlanetSpace, Vec3I64> {
    type Storage = Vec3I64;

    fn coords(&self) -> &Vec3I64 {
        &self.value
    }

    fn space_name() -> &'static str {
        "Planet"
    }
}

impl InSpace<ChunkSpace> for Pos<ChunkSpace, UVec3> {
    type Storage = UVec3;

    fn coords(&self) -> &UVec3 {
        &self.value
    }

    fn space_name() -> &'static str {
        "Chunk"
    }
}

// Note: SectorCoord is now defined above with the new sector coordinate types

impl InSpace<SectorSpace> for Pos<SectorSpace, SectorCoord> {
    type Storage = SectorCoord;

    fn coords(&self) -> &SectorCoord {
        &self.value
    }

    fn space_name() -> &'static str {
        "Sector"
    }
}

// Constants
/// Millimeters to meters conversion factor
const MM_TO_METERS: f32 = 0.001;

// Conversion functions between coordinate spaces

/// Convert a universe-space position to sector-space.
pub fn universe_to_sector(
    pos: &Pos<UniverseSpace, nebula_math::Vec3I128>,
) -> Pos<SectorSpace, SectorCoord> {
    let world_pos = WorldPosition::new(pos.value.x, pos.value.y, pos.value.z);
    Pos::new(SectorCoord::from_world(&world_pos))
}

/// Convert a sector-space position back to universe-space.
pub fn sector_to_universe(
    pos: &Pos<SectorSpace, SectorCoord>,
) -> Pos<UniverseSpace, nebula_math::Vec3I128> {
    let world_pos = pos.value.to_world();
    Pos::new(nebula_math::Vec3I128::new(
        world_pos.x,
        world_pos.y,
        world_pos.z,
    ))
}

/// Convert a universe-space position to planet-space.
/// `planet_origin` is the planet's center in universe coordinates.
pub fn universe_to_planet(
    pos: &Pos<UniverseSpace, nebula_math::Vec3I128>,
    planet_origin: &Pos<UniverseSpace, nebula_math::Vec3I128>,
) -> Pos<PlanetSpace, Vec3I64> {
    let delta = pos.value - planet_origin.value;
    Pos::new(Vec3I64::new(delta.x as i64, delta.y as i64, delta.z as i64))
}

/// Convert a planet-space position back to universe-space.
/// `planet_origin` is the planet's center in universe coordinates.
pub fn planet_to_universe(
    pos: &Pos<PlanetSpace, Vec3I64>,
    planet_origin: &Pos<UniverseSpace, nebula_math::Vec3I128>,
) -> Pos<UniverseSpace, nebula_math::Vec3I128> {
    let offset = nebula_math::Vec3I128::new(
        pos.value.x as i128,
        pos.value.y as i128,
        pos.value.z as i128,
    );
    Pos::new(planet_origin.value + offset)
}

/// Convert a planet-space position to chunk-space.
/// `chunk_origin` is the chunk's origin corner in planet coordinates.
pub fn planet_to_chunk(
    pos: &Pos<PlanetSpace, Vec3I64>,
    chunk_origin: &Pos<PlanetSpace, Vec3I64>,
) -> Pos<ChunkSpace, UVec3> {
    let delta_x = (pos.value.x - chunk_origin.value.x) as u32;
    let delta_y = (pos.value.y - chunk_origin.value.y) as u32;
    let delta_z = (pos.value.z - chunk_origin.value.z) as u32;
    Pos::new(UVec3::new(delta_x, delta_y, delta_z))
}

/// Convert a chunk-space position back to planet-space.
/// `chunk_origin` is the chunk's origin corner in planet coordinates.  
pub fn chunk_to_planet(
    pos: &Pos<ChunkSpace, UVec3>,
    chunk_origin: &Pos<PlanetSpace, Vec3I64>,
) -> Pos<PlanetSpace, Vec3I64> {
    let x = chunk_origin.value.x + pos.value.x as i64;
    let y = chunk_origin.value.y + pos.value.y as i64;
    let z = chunk_origin.value.z + pos.value.z as i64;
    Pos::new(Vec3I64::new(x, y, z))
}

/// Convert a universe-space position to camera-local f32 space.
/// `camera_pos` is the camera's universe-space position.
pub fn universe_to_local(
    pos: &Pos<UniverseSpace, nebula_math::Vec3I128>,
    camera_pos: &Pos<UniverseSpace, nebula_math::Vec3I128>,
) -> Pos<LocalSpace, Vec3> {
    let delta = pos.value - camera_pos.value;
    Pos::new(Vec3::new(
        delta.x as f32 * MM_TO_METERS,
        delta.y as f32 * MM_TO_METERS,
        delta.z as f32 * MM_TO_METERS,
    ))
}

/// Convert a local-space position back to universe-space.
/// `camera_pos` is the camera's universe-space position.
pub fn local_to_universe(
    pos: &Pos<LocalSpace, Vec3>,
    camera_pos: &Pos<UniverseSpace, nebula_math::Vec3I128>,
) -> Pos<UniverseSpace, nebula_math::Vec3I128> {
    let offset_mm = nebula_math::Vec3I128::new(
        (pos.value.x / MM_TO_METERS) as i128,
        (pos.value.y / MM_TO_METERS) as i128,
        (pos.value.z / MM_TO_METERS) as i128,
    );
    Pos::new(camera_pos.value + offset_mm)
}

// Coordinate Frame Transitions (Plan 03_coords/03)

/// Trait for any object that can convert a position from space `From` to space `To`.
pub trait Transition<From, To> {
    type Input;
    type Output;

    /// Convert a position from the source space to the target space.
    fn apply(&self, pos: &Pos<From, Self::Input>) -> Pos<To, Self::Output>;
}

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

/// Trait for transitions that can be inverted.
pub trait Invertible<From, To>: Transition<From, To> {
    type Inverse: Transition<To, From>;
    fn inverse(&self) -> Self::Inverse;
}

/// Identity transition - converts a position to the same space (no-op).
pub struct Identity<S, T>(PhantomData<(S, T)>);

impl<S, T> Default for Identity<S, T> {
    fn default() -> Self {
        Identity(PhantomData)
    }
}

impl<S, T: Clone> Transition<S, S> for Identity<S, T> {
    type Input = T;
    type Output = T;

    fn apply(&self, pos: &Pos<S, T>) -> Pos<S, T> {
        Pos::new(pos.value.clone())
    }
}

// Concrete Transition Types

/// Universe to Sector transition - pure bitwise decomposition.
pub struct UniverseToSector;

impl Transition<UniverseSpace, SectorSpace> for UniverseToSector {
    type Input = nebula_math::Vec3I128;
    type Output = SectorCoord;

    fn apply(
        &self,
        pos: &Pos<UniverseSpace, nebula_math::Vec3I128>,
    ) -> Pos<SectorSpace, SectorCoord> {
        let world_pos = WorldPosition::new(pos.value.x, pos.value.y, pos.value.z);
        Pos::new(SectorCoord::from_world(&world_pos))
    }
}

/// Sector to Universe transition - reconstruct from sector coordinates.
pub struct SectorToUniverse;

impl Transition<SectorSpace, UniverseSpace> for SectorToUniverse {
    type Input = SectorCoord;
    type Output = nebula_math::Vec3I128;

    fn apply(
        &self,
        pos: &Pos<SectorSpace, SectorCoord>,
    ) -> Pos<UniverseSpace, nebula_math::Vec3I128> {
        let world_pos = pos.value.to_world();
        Pos::new(nebula_math::Vec3I128::new(
            world_pos.x,
            world_pos.y,
            world_pos.z,
        ))
    }
}

impl Invertible<UniverseSpace, SectorSpace> for UniverseToSector {
    type Inverse = SectorToUniverse;

    fn inverse(&self) -> Self::Inverse {
        SectorToUniverse
    }
}

impl Invertible<SectorSpace, UniverseSpace> for SectorToUniverse {
    type Inverse = UniverseToSector;

    fn inverse(&self) -> Self::Inverse {
        UniverseToSector
    }
}

/// Sector to Planet transition - requires the planet's universe-space origin, decomposed into sector coordinates.
pub struct SectorToPlanet {
    /// The planet center's sector coordinate.
    pub planet_origin: SectorCoord,
}

impl Transition<SectorSpace, PlanetSpace> for SectorToPlanet {
    type Input = SectorCoord;
    type Output = Vec3I64;

    fn apply(&self, pos: &Pos<SectorSpace, SectorCoord>) -> Pos<PlanetSpace, Vec3I64> {
        // Reconstruct both positions to i128, subtract, truncate to i64.
        let world = pos.value.to_world();
        let origin = self.planet_origin.to_world();
        let delta = world - origin;
        Pos::new(Vec3I64::new(delta.x as i64, delta.y as i64, delta.z as i64))
    }
}

/// Planet to Sector transition.
pub struct PlanetToSector {
    /// The planet center's sector coordinate.
    pub planet_origin: SectorCoord,
}

impl Transition<PlanetSpace, SectorSpace> for PlanetToSector {
    type Input = Vec3I64;
    type Output = SectorCoord;

    fn apply(&self, pos: &Pos<PlanetSpace, Vec3I64>) -> Pos<SectorSpace, SectorCoord> {
        let origin_world = self.planet_origin.to_world();
        let offset = nebula_math::Vec3I128::new(
            pos.value.x as i128,
            pos.value.y as i128,
            pos.value.z as i128,
        );
        let world_pos = WorldPosition::new(
            origin_world.x + offset.x,
            origin_world.y + offset.y,
            origin_world.z + offset.z,
        );
        Pos::new(SectorCoord::from_world(&world_pos))
    }
}

impl Invertible<SectorSpace, PlanetSpace> for SectorToPlanet {
    type Inverse = PlanetToSector;

    fn inverse(&self) -> Self::Inverse {
        PlanetToSector {
            planet_origin: self.planet_origin,
        }
    }
}

/// Planet to Chunk transition - requires the chunk's origin in planet space.
pub struct PlanetToChunk {
    /// The chunk's origin corner in planet-space millimeters.
    pub chunk_origin: Vec3I64,
}

impl Transition<PlanetSpace, ChunkSpace> for PlanetToChunk {
    type Input = Vec3I64;
    type Output = UVec3;

    fn apply(&self, pos: &Pos<PlanetSpace, Vec3I64>) -> Pos<ChunkSpace, UVec3> {
        let local = pos.value - self.chunk_origin;
        Pos::new(UVec3::new(local.x as u32, local.y as u32, local.z as u32))
    }
}

/// Chunk to Planet transition.
pub struct ChunkToPlanet {
    /// The chunk's origin corner in planet-space millimeters.
    pub chunk_origin: Vec3I64,
}

impl Transition<ChunkSpace, PlanetSpace> for ChunkToPlanet {
    type Input = UVec3;
    type Output = Vec3I64;

    fn apply(&self, pos: &Pos<ChunkSpace, UVec3>) -> Pos<PlanetSpace, Vec3I64> {
        let x = self.chunk_origin.x + pos.value.x as i64;
        let y = self.chunk_origin.y + pos.value.y as i64;
        let z = self.chunk_origin.z + pos.value.z as i64;
        Pos::new(Vec3I64::new(x, y, z))
    }
}

impl Invertible<PlanetSpace, ChunkSpace> for PlanetToChunk {
    type Inverse = ChunkToPlanet;

    fn inverse(&self) -> Self::Inverse {
        ChunkToPlanet {
            chunk_origin: self.chunk_origin,
        }
    }
}

/// Chunk to Local (Camera) transition - requires the camera's position in chunk space.
pub struct ChunkToLocal {
    /// The chunk's origin expressed in camera-local meters (f32).
    pub chunk_origin_in_camera: Vec3,
}

impl Transition<ChunkSpace, LocalSpace> for ChunkToLocal {
    type Input = UVec3;
    type Output = Vec3;

    fn apply(&self, pos: &Pos<ChunkSpace, UVec3>) -> Pos<LocalSpace, Vec3> {
        let meters = Vec3::new(
            pos.value.x as f32 * MM_TO_METERS,
            pos.value.y as f32 * MM_TO_METERS,
            pos.value.z as f32 * MM_TO_METERS,
        );
        Pos::new(self.chunk_origin_in_camera + meters)
    }
}

/// Universe to Local transition - direct conversion for convenience.
pub struct UniverseToLocal {
    /// The camera's universe-space position.
    pub camera_pos: nebula_math::Vec3I128,
}

impl Transition<UniverseSpace, LocalSpace> for UniverseToLocal {
    type Input = nebula_math::Vec3I128;
    type Output = Vec3;

    fn apply(&self, pos: &Pos<UniverseSpace, nebula_math::Vec3I128>) -> Pos<LocalSpace, Vec3> {
        let delta = pos.value - self.camera_pos;
        Pos::new(Vec3::new(
            delta.x as f32 * MM_TO_METERS,
            delta.y as f32 * MM_TO_METERS,
            delta.z as f32 * MM_TO_METERS,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_coordinate_space_enum_has_five_variants() {
        // Test that all five variants exist and can be matched without wildcard
        let spaces = [
            CoordinateSpace::Universe,
            CoordinateSpace::Sector,
            CoordinateSpace::Planet,
            CoordinateSpace::Chunk,
            CoordinateSpace::Local,
        ];

        for space in spaces {
            match space {
                CoordinateSpace::Universe => {}
                CoordinateSpace::Sector => {}
                CoordinateSpace::Planet => {}
                CoordinateSpace::Chunk => {}
                CoordinateSpace::Local => {} // No wildcard arm - this ensures all variants are covered
            }
        }
    }

    #[test]
    fn test_pos_type_safety_prevents_mixing() {
        // This is a compile-time test - these should be distinct types
        let universe_pos = Pos::<UniverseSpace, nebula_math::Vec3I128>::new(
            nebula_math::Vec3I128::new(1000, 2000, 3000),
        );
        let chunk_pos = Pos::<ChunkSpace, UVec3>::new(UVec3::new(10, 20, 30));

        // These should be different types (checked at compile time)
        // If this compiles, the type system is working correctly
        assert_ne!(
            std::any::TypeId::of::<Pos<UniverseSpace, nebula_math::Vec3I128>>(),
            std::any::TypeId::of::<Pos<ChunkSpace, UVec3>>()
        );

        // Use the values to avoid unused variable warnings
        let _ = (universe_pos, chunk_pos);
    }

    #[test]
    fn test_in_space_returns_correct_space_name() {
        assert_eq!(
            Pos::<UniverseSpace, nebula_math::Vec3I128>::space_name(),
            "Universe"
        );
        assert_eq!(Pos::<SectorSpace, SectorCoord>::space_name(), "Sector");
        assert_eq!(Pos::<PlanetSpace, Vec3I64>::space_name(), "Planet");
        assert_eq!(Pos::<ChunkSpace, UVec3>::space_name(), "Chunk");
        assert_eq!(Pos::<LocalSpace, Vec3>::space_name(), "Local");
    }

    #[test]
    fn test_conversion_chain_compiles_universe_to_local() {
        // Create a universe position
        let universe_pos = Pos::<UniverseSpace, nebula_math::Vec3I128>::new(
            nebula_math::Vec3I128::new(1_000_000, -5_000_000_000, 42),
        );

        // Step 1: Universe -> Sector
        let sector_pos = universe_to_sector(&universe_pos);

        // Step 2: Sector -> Universe (for planet conversion)
        let universe_pos2 = sector_to_universe(&sector_pos);

        // Step 3: Universe -> Planet (using same position as planet origin for simplicity)
        let planet_origin =
            Pos::<UniverseSpace, nebula_math::Vec3I128>::new(nebula_math::Vec3I128::new(0, 0, 0));
        let planet_pos = universe_to_planet(&universe_pos2, &planet_origin);

        // Step 4: Planet -> Chunk (using same position as chunk origin)
        let chunk_origin = Pos::<PlanetSpace, Vec3I64>::new(Vec3I64::new(0, 0, 0));
        let chunk_pos = planet_to_chunk(&planet_pos, &chunk_origin);

        // Step 5: Back to universe for local conversion
        let planet_pos2 = chunk_to_planet(&chunk_pos, &chunk_origin);
        let universe_pos3 = planet_to_universe(&planet_pos2, &planet_origin);

        // Step 6: Universe -> Local
        let camera_pos =
            Pos::<UniverseSpace, nebula_math::Vec3I128>::new(nebula_math::Vec3I128::new(0, 0, 0));
        let local_pos = universe_to_local(&universe_pos3, &camera_pos);

        // Assert final result is correct type
        assert_eq!(Pos::<LocalSpace, Vec3>::space_name(), "Local");
        let _ = local_pos; // Use the value
    }

    #[test]
    fn test_roundtrip_universe_sector_universe() {
        let original = Pos::<UniverseSpace, nebula_math::Vec3I128>::new(
            nebula_math::Vec3I128::new(1_000_000, -5_000_000_000, 42),
        );

        let sector = universe_to_sector(&original);
        let roundtrip = sector_to_universe(&sector);

        assert_eq!(original.value, roundtrip.value);
    }

    #[test]
    fn test_local_space_values_are_f32() {
        let universe_pos = Pos::<UniverseSpace, nebula_math::Vec3I128>::new(
            nebula_math::Vec3I128::new(10_000, 20_000, 30_000),
        );
        let camera_pos =
            Pos::<UniverseSpace, nebula_math::Vec3I128>::new(nebula_math::Vec3I128::new(0, 0, 0));

        let local_pos = universe_to_local(&universe_pos, &camera_pos);

        // Check that coordinates are in meters and within reasonable f32 precision
        // f32 has limited precision, so we use a more reasonable tolerance
        assert!((local_pos.value.x - 10.0).abs() < 1e-4); // 10_000 mm = 10 m
        assert!((local_pos.value.y - 20.0).abs() < 1e-4); // 20_000 mm = 20 m  
        assert!((local_pos.value.z - 30.0).abs() < 1e-4); // 30_000 mm = 30 m

        // Verify coordinates are within reasonable range
        assert!(local_pos.value.x.abs() < 10_000.0);
        assert!(local_pos.value.y.abs() < 10_000.0);
        assert!(local_pos.value.z.abs() < 10_000.0);
    }

    #[test]
    fn test_subsystem_space_annotations() {
        // Mock functions that require specific coordinate spaces
        fn render_chunk(_pos: Pos<ChunkSpace, UVec3>) {
            // This function only accepts chunk-space positions
        }

        fn save_entity(_pos: Pos<UniverseSpace, nebula_math::Vec3I128>) {
            // This function only accepts universe-space positions
        }

        // Create correctly-typed positions
        let chunk_pos = Pos::<ChunkSpace, UVec3>::new(UVec3::new(10, 20, 30));
        let universe_pos = Pos::<UniverseSpace, nebula_math::Vec3I128>::new(
            nebula_math::Vec3I128::new(1000, 2000, 3000),
        );

        // These calls should compile successfully
        render_chunk(chunk_pos);
        save_entity(universe_pos);

        // If this compiles, the type system is correctly enforcing space annotations
    }

    #[test]
    fn test_sector_coord_components() {
        let sector_index = SectorIndex { x: 1, y: -2, z: 3 };
        let local_offset = SectorOffset {
            x: 1000,
            y: 2000,
            z: 3000,
        };

        let sector_coord = SectorCoord {
            sector: sector_index,
            offset: local_offset,
        };

        assert_eq!(sector_coord.sector.x, 1);
        assert_eq!(sector_coord.sector.y, -2);
        assert_eq!(sector_coord.sector.z, 3);
        assert_eq!(sector_coord.offset.x, 1000);
        assert_eq!(sector_coord.offset.y, 2000);
        assert_eq!(sector_coord.offset.z, 3000);
    }

    #[test]
    fn test_universe_to_local_roundtrip() {
        let original = Pos::<UniverseSpace, nebula_math::Vec3I128>::new(
            nebula_math::Vec3I128::new(50_000, 100_000, 150_000),
        );
        let camera_pos =
            Pos::<UniverseSpace, nebula_math::Vec3I128>::new(nebula_math::Vec3I128::new(0, 0, 0));

        let local_pos = universe_to_local(&original, &camera_pos);
        let roundtrip = local_to_universe(&local_pos, &camera_pos);

        // Should be very close (within rounding error from f32 conversion)
        let delta = original.value - roundtrip.value;
        assert!(delta.x.abs() <= 1); // Within 1mm
        assert!(delta.y.abs() <= 1);
        assert!(delta.z.abs() <= 1);
    }

    #[test]
    fn test_planet_chunk_roundtrip() {
        let original = Pos::<PlanetSpace, Vec3I64>::new(Vec3I64::new(100_000, 200_000, 300_000));
        let chunk_origin = Pos::<PlanetSpace, Vec3I64>::new(Vec3I64::new(99_000, 199_000, 299_000));

        let chunk_pos = planet_to_chunk(&original, &chunk_origin);
        let roundtrip = chunk_to_planet(&chunk_pos, &chunk_origin);

        assert_eq!(original.value, roundtrip.value);
    }

    // Sector Coordinate Tests (Plan 03_coords/02)

    #[test]
    fn test_origin_maps_to_sector_zero() {
        let world_pos = WorldPosition::new(0, 0, 0);
        let sector_coord = SectorCoord::from_world(&world_pos);

        assert_eq!(sector_coord.sector.x, 0);
        assert_eq!(sector_coord.sector.y, 0);
        assert_eq!(sector_coord.sector.z, 0);
        assert_eq!(sector_coord.offset.x, 0);
        assert_eq!(sector_coord.offset.y, 0);
        assert_eq!(sector_coord.offset.z, 0);
    }

    #[test]
    fn test_position_at_sector_boundary() {
        // Test exactly at the start of sector 1
        let world_pos = WorldPosition::new(1_i128 << 32, 0, 0);
        let sector_coord = SectorCoord::from_world(&world_pos);

        assert_eq!(sector_coord.sector.x, 1);
        assert_eq!(sector_coord.offset.x, 0);

        // Test last position in sector 0
        let world_pos = WorldPosition::new((1_i128 << 32) - 1, 0, 0);
        let sector_coord = SectorCoord::from_world(&world_pos);

        assert_eq!(sector_coord.sector.x, 0);
        assert_eq!(sector_coord.offset.x, 4294967295_u32 as i32);
    }

    #[test]
    fn test_roundtrip_world_to_sector_to_world() {
        let test_positions = [
            WorldPosition::new(0, 0, 0),
            WorldPosition::new(1_000_000, -5_000_000_000, 42),
            WorldPosition::new(i128::MAX, i128::MIN, 0),
            WorldPosition::new(12345678901234, -98765432109876, 555444333222),
            WorldPosition::new(-1000, 2000, -3000),
        ];

        for pos in test_positions {
            let sector_coord = SectorCoord::from_world(&pos);
            let reconstructed = sector_coord.to_world();
            assert_eq!(
                pos, reconstructed,
                "Roundtrip failed for position: {:?}",
                pos
            );
        }
    }

    #[test]
    fn test_negative_coordinates() {
        let world_pos = WorldPosition::new(-1, -4294967296, -4294967297);
        let sector_coord = SectorCoord::from_world(&world_pos);

        // x = -1: sector = -1, offset = 4294967295
        assert_eq!(sector_coord.sector.x, -1);
        assert_eq!(sector_coord.offset.x, 4294967295_u32 as i32);

        // y = -4294967296 = -(1 << 32): sector = -1, offset = 0
        assert_eq!(sector_coord.sector.y, -1);
        assert_eq!(sector_coord.offset.y, 0);

        // z = -4294967297 = -(1 << 32) - 1: sector = -2, offset = 4294967295
        assert_eq!(sector_coord.sector.z, -2);
        assert_eq!(sector_coord.offset.z, 4294967295_u32 as i32);
    }

    #[test]
    fn test_sector_index_for_large_coordinates() {
        let world_pos = WorldPosition::new(100_i128 << 32, -(50_i128 << 32), 1_i128 << 96);
        let sector_coord = SectorCoord::from_world(&world_pos);

        assert_eq!(sector_coord.sector.x, 100);
        assert_eq!(sector_coord.sector.y, -50);
        assert_eq!(sector_coord.sector.z, 1_i128 << 64);
        assert_eq!(sector_coord.offset.x, 0);
        assert_eq!(sector_coord.offset.y, 0);
        assert_eq!(sector_coord.offset.z, 0);
    }

    #[test]
    fn test_sector_key_hash_equality() {
        let sector_coord1 = SectorCoord {
            sector: SectorIndex { x: 1, y: 2, z: 3 },
            offset: SectorOffset {
                x: 100,
                y: 200,
                z: 300,
            },
        };
        let sector_coord2 = SectorCoord {
            sector: SectorIndex { x: 1, y: 2, z: 3 },
            offset: SectorOffset {
                x: 400,
                y: 500,
                z: 600,
            }, // Different offset
        };
        let sector_coord3 = SectorCoord {
            sector: SectorIndex { x: 4, y: 5, z: 6 }, // Different sector
            offset: SectorOffset {
                x: 100,
                y: 200,
                z: 300,
            },
        };

        let key1 = SectorKey::from(&sector_coord1);
        let key2 = SectorKey::from(&sector_coord2);
        let key3 = SectorKey::from(&sector_coord3);

        // Same sector index should produce same key
        assert_eq!(key1, key2);

        // Different sector index should produce different key
        assert_ne!(key1, key3);

        // Hash equality
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher1 = DefaultHasher::new();
        key1.hash(&mut hasher1);
        let hash1 = hasher1.finish();

        let mut hasher2 = DefaultHasher::new();
        key2.hash(&mut hasher2);
        let hash2 = hasher2.finish();

        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_sector_key_usable_as_hashmap_key() {
        use std::collections::HashMap;
        let mut map: HashMap<SectorKey, String> = HashMap::new();

        let key1 = SectorKey(SectorIndex { x: 1, y: 2, z: 3 });
        let key2 = SectorKey(SectorIndex { x: 4, y: 5, z: 6 });
        let key3 = SectorKey(SectorIndex { x: 7, y: 8, z: 9 });

        map.insert(key1, "sector one".to_string());
        map.insert(key2, "sector two".to_string());
        map.insert(key3, "sector three".to_string());

        assert_eq!(map.get(&key1), Some(&"sector one".to_string()));
        assert_eq!(map.get(&key2), Some(&"sector two".to_string()));
        assert_eq!(map.get(&key3), Some(&"sector three".to_string()));

        let key_not_in_map = SectorKey(SectorIndex {
            x: 99,
            y: 99,
            z: 99,
        });
        assert_eq!(map.get(&key_not_in_map), None);
    }

    // Coordinate Frame Transition Tests (Plan 03_coords/03)

    #[test]
    fn test_identity_transition() {
        let identity = Identity::<UniverseSpace, nebula_math::Vec3I128>::default();
        let pos = Pos::<UniverseSpace, nebula_math::Vec3I128>::new(nebula_math::Vec3I128::new(
            100, 200, 300,
        ));

        let result = identity.apply(&pos);
        assert_eq!(result.value, pos.value);
    }

    #[test]
    fn test_compose_two_transitions_equals_direct() {
        let universe_pos = Pos::<UniverseSpace, nebula_math::Vec3I128>::new(
            nebula_math::Vec3I128::new(1_000_000_000, 2_000_000_000, 3_000_000_000),
        );

        let planet_origin = SectorCoord {
            sector: SectorIndex { x: 0, y: 0, z: 0 },
            offset: SectorOffset { x: 0, y: 0, z: 0 },
        };

        // Apply transitions separately
        let universe_to_sector = UniverseToSector;
        let sector_pos = universe_to_sector.apply(&universe_pos);

        let sector_to_planet = SectorToPlanet { planet_origin };
        let planet_pos1 = sector_to_planet.apply(&sector_pos);

        // Apply composed transition
        let composed = universe_to_sector.then(sector_to_planet);
        let planet_pos2 = composed.apply(&universe_pos);

        assert_eq!(planet_pos1.value, planet_pos2.value);
    }

    #[test]
    fn test_compose_full_chain() {
        let universe_pos = Pos::<UniverseSpace, nebula_math::Vec3I128>::new(
            nebula_math::Vec3I128::new(10_000, 20_000, 30_000),
        );

        let planet_origin = SectorCoord {
            sector: SectorIndex { x: 0, y: 0, z: 0 },
            offset: SectorOffset { x: 0, y: 0, z: 0 },
        };

        let chunk_origin = Vec3I64::new(0, 0, 0);
        let camera_origin = Vec3::new(0.0, 0.0, 0.0);

        let full_chain = UniverseToSector
            .then(SectorToPlanet { planet_origin })
            .then(PlanetToChunk { chunk_origin })
            .then(ChunkToLocal {
                chunk_origin_in_camera: camera_origin,
            });

        let local_pos = full_chain.apply(&universe_pos);

        // Should be Pos<LocalSpace, Vec3>
        assert_eq!(Pos::<LocalSpace, Vec3>::space_name(), "Local");

        // Check that the coordinates are converted to meters
        let expected_x = 10_000_f32 * MM_TO_METERS; // 10m
        let expected_y = 20_000_f32 * MM_TO_METERS; // 20m
        let expected_z = 30_000_f32 * MM_TO_METERS; // 30m

        assert!((local_pos.value.x - expected_x).abs() < f32::EPSILON);
        assert!((local_pos.value.y - expected_y).abs() < f32::EPSILON);
        assert!((local_pos.value.z - expected_z).abs() < f32::EPSILON);
    }

    #[test]
    fn test_inverse_transition_roundtrips() {
        let universe_pos = Pos::<UniverseSpace, nebula_math::Vec3I128>::new(
            nebula_math::Vec3I128::new(1_000_000, 2_000_000, 3_000_000),
        );

        let universe_to_sector = UniverseToSector;
        let sector_pos = universe_to_sector.apply(&universe_pos);

        let inverse = universe_to_sector.inverse();
        let roundtrip = inverse.apply(&sector_pos);

        assert_eq!(universe_pos.value, roundtrip.value);
    }

    #[test]
    fn test_composed_inverse_roundtrips() {
        let universe_pos = Pos::<UniverseSpace, nebula_math::Vec3I128>::new(
            nebula_math::Vec3I128::new(5_000_000, 10_000_000, 15_000_000),
        );

        let planet_origin = SectorCoord {
            sector: SectorIndex { x: 0, y: 0, z: 0 },
            offset: SectorOffset { x: 0, y: 0, z: 0 },
        };

        let composed = UniverseToSector.then(SectorToPlanet { planet_origin });
        let planet_pos = composed.apply(&universe_pos);

        // For composed inverse, we need to create the inverse manually
        let inverse_composed = PlanetToSector { planet_origin }.then(SectorToUniverse);
        let roundtrip = inverse_composed.apply(&planet_pos);

        assert_eq!(universe_pos.value, roundtrip.value);
    }

    #[test]
    fn test_camera_transition_produces_f32() {
        let chunk_pos = Pos::<ChunkSpace, UVec3>::new(UVec3::new(1000, 2000, 3000));
        let chunk_to_local = ChunkToLocal {
            chunk_origin_in_camera: Vec3::new(10.0, 20.0, 30.0),
        };

        let local_pos = chunk_to_local.apply(&chunk_pos);

        // 1000mm = 1m, 2000mm = 2m, 3000mm = 3m
        // Plus camera origin offset
        let expected = Vec3::new(11.0, 22.0, 33.0);

        assert!((local_pos.value.x - expected.x).abs() < f32::EPSILON);
        assert!((local_pos.value.y - expected.y).abs() < f32::EPSILON);
        assert!((local_pos.value.z - expected.z).abs() < f32::EPSILON);
    }

    #[test]
    fn test_then_is_associative() {
        let universe_pos = Pos::<UniverseSpace, nebula_math::Vec3I128>::new(
            nebula_math::Vec3I128::new(100_000, 200_000, 300_000),
        );

        let planet_origin = SectorCoord {
            sector: SectorIndex { x: 0, y: 0, z: 0 },
            offset: SectorOffset { x: 0, y: 0, z: 0 },
        };
        let chunk_origin = Vec3I64::new(0, 0, 0);

        let a = UniverseToSector;
        let b = SectorToPlanet { planet_origin };
        let c = PlanetToChunk { chunk_origin };

        // (A.then(B)).then(C)
        let left_assoc = a.then(b).then(c);
        let result1 = left_assoc.apply(&universe_pos);

        // A.then(B.then(C))
        let right_assoc = UniverseToSector
            .then(SectorToPlanet { planet_origin }.then(PlanetToChunk { chunk_origin }));
        let result2 = right_assoc.apply(&universe_pos);

        assert_eq!(result1.value, result2.value);
    }

    #[test]
    fn test_universe_to_local_direct() {
        let universe_pos = Pos::<UniverseSpace, nebula_math::Vec3I128>::new(
            nebula_math::Vec3I128::new(50_000, 100_000, 150_000),
        );
        let camera_pos = nebula_math::Vec3I128::new(10_000, 20_000, 30_000);

        let universe_to_local = UniverseToLocal { camera_pos };
        let local_pos = universe_to_local.apply(&universe_pos);

        // Expected delta: (40_000, 80_000, 120_000) mm = (40, 80, 120) m
        assert!((local_pos.value.x - 40.0).abs() < 1e-4);
        assert!((local_pos.value.y - 80.0).abs() < 1e-4);
        assert!((local_pos.value.z - 120.0).abs() < 1e-4);
    }
}
