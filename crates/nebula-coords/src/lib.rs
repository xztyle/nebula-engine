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

/// Two-part coordinate for sector space: sector index + local offset
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SectorCoord {
    /// Which 2^32 mm (~4,295 km) cube this position falls in
    pub sector_index: Vec3I96,
    /// Position within that cube in millimeters
    pub local_offset: Vec3I32,
}

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
    let sector_index = Vec3I96::new(
        (pos.value.x >> 32) as i64,
        (pos.value.y >> 32) as i64,
        (pos.value.z >> 32) as i64,
    );
    let local_offset = Vec3I32::new(pos.value.x as i32, pos.value.y as i32, pos.value.z as i32);
    Pos::new(SectorCoord {
        sector_index,
        local_offset,
    })
}

/// Convert a sector-space position back to universe-space.
pub fn sector_to_universe(
    pos: &Pos<SectorSpace, SectorCoord>,
) -> Pos<UniverseSpace, nebula_math::Vec3I128> {
    let x = (pos.value.sector_index.x as i128) << 32 | (pos.value.local_offset.x as u32 as i128);
    let y = (pos.value.sector_index.y as i128) << 32 | (pos.value.local_offset.y as u32 as i128);
    let z = (pos.value.sector_index.z as i128) << 32 | (pos.value.local_offset.z as u32 as i128);
    Pos::new(nebula_math::Vec3I128::new(x, y, z))
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
        let sector_index = Vec3I96::new(1, -2, 3);
        let local_offset = Vec3I32::new(1000, 2000, 3000);

        let sector_coord = SectorCoord {
            sector_index,
            local_offset,
        };

        assert_eq!(sector_coord.sector_index.x, 1);
        assert_eq!(sector_coord.sector_index.y, -2);
        assert_eq!(sector_coord.sector_index.z, 3);
        assert_eq!(sector_coord.local_offset.x, 1000);
        assert_eq!(sector_coord.local_offset.y, 2000);
        assert_eq!(sector_coord.local_offset.z, 3000);
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
}
