//! Whittaker diagram: maps (temperature, moisture) pairs to biome IDs.

use super::BiomeId;

/// A rectangular region in temperature–moisture space mapped to a biome.
#[derive(Clone, Debug)]
pub struct WhittakerRegion {
    /// Minimum temperature (inclusive), in `[0.0, 1.0]`.
    pub temp_min: f64,
    /// Maximum temperature (exclusive), in `[0.0, 1.0]`.
    pub temp_max: f64,
    /// Minimum moisture (inclusive), in `[0.0, 1.0]`.
    pub moisture_min: f64,
    /// Maximum moisture (exclusive), in `[0.0, 1.0]`.
    pub moisture_max: f64,
    /// Biome assigned to points within this region.
    pub biome_id: BiomeId,
}

/// A Whittaker-style 2D lookup diagram that assigns biomes based on
/// temperature and moisture values.
pub struct WhittakerDiagram {
    /// Ordered list of regions; first match wins.
    pub regions: Vec<WhittakerRegion>,
    /// Fallback biome if no region matches.
    pub fallback: BiomeId,
}

impl WhittakerDiagram {
    /// Looks up the biome for a given temperature and moisture, both in `[0.0, 1.0]`.
    pub fn lookup(&self, temperature: f64, moisture: f64) -> BiomeId {
        for region in &self.regions {
            if temperature >= region.temp_min
                && temperature < region.temp_max
                && moisture >= region.moisture_min
                && moisture < region.moisture_max
            {
                return region.biome_id;
            }
        }
        self.fallback
    }
}
