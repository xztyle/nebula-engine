use nebula_coords::*;

fn main() {
    // This should fail to compile: UniverseToSector outputs SectorSpace
    // but PlanetToChunk expects PlanetSpace as input
    let _invalid_composition = UniverseToSector.then(PlanetToChunk {
        chunk_origin: Vec3I64::new(0, 0, 0),
    });
}