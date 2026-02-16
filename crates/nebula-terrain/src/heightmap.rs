//! Multi-octave fractal Brownian motion (fBm) heightmap sampler.
//!
//! Composites multiple octaves of simplex noise to produce natural-looking
//! terrain height values with features at many spatial frequencies.

use noise::{NoiseFn, Simplex};

/// Configuration for multi-octave fBm noise used in heightmap generation.
#[derive(Clone, Debug)]
pub struct HeightmapParams {
    /// World seed for deterministic generation.
    pub seed: u64,
    /// Number of noise octaves to composite. More octaves add finer detail
    /// at the cost of additional computation. Typical range: 6–8.
    pub octaves: u32,
    /// Frequency multiplier between successive octaves. Each octave's frequency
    /// is `base_frequency * lacunarity^octave_index`. Default: 2.0.
    pub lacunarity: f64,
    /// Amplitude multiplier between successive octaves. Each octave's amplitude
    /// is `amplitude * persistence^octave_index`. Default: 0.5.
    pub persistence: f64,
    /// Frequency of the first (lowest) octave. Controls the spatial scale of
    /// the broadest terrain features. Default: 0.001 (one full cycle per 1000 units).
    pub base_frequency: f64,
    /// Amplitude of the first octave. The maximum theoretical height contribution
    /// of the first octave in engine units. Default: 4000.0 (4 km).
    pub amplitude: f64,
}

impl Default for HeightmapParams {
    fn default() -> Self {
        Self {
            seed: 0,
            octaves: 6,
            lacunarity: 2.0,
            persistence: 0.5,
            amplitude: 4000.0,
            base_frequency: 0.001,
        }
    }
}

/// Generates terrain height values using fractal Brownian motion over simplex noise.
///
/// Each sample composites multiple octaves of noise, where each successive octave
/// doubles in frequency and halves in amplitude, producing self-similar detail at
/// progressively finer scales.
pub struct HeightmapSampler {
    noise: Simplex,
    params: HeightmapParams,
}

impl HeightmapSampler {
    /// Create a new sampler with the given parameters.
    pub fn new(params: HeightmapParams) -> Self {
        let noise = Simplex::new(params.seed as u32);
        Self { noise, params }
    }

    /// Sample the heightmap at a 2D coordinate on the cube face.
    ///
    /// Returns a height value in engine units. The theoretical range is
    /// approximately `[-max_amplitude, +max_amplitude]` where `max_amplitude`
    /// is the geometric sum of all octave amplitudes.
    pub fn sample(&self, x: f64, y: f64) -> f64 {
        let mut total = 0.0;
        let mut frequency = self.params.base_frequency;
        let mut amplitude = self.params.amplitude;

        for _ in 0..self.params.octaves {
            let nx = x * frequency;
            let ny = y * frequency;
            let noise_val = self.noise.get([nx, ny]);
            total += noise_val * amplitude;

            frequency *= self.params.lacunarity;
            amplitude *= self.params.persistence;
        }

        total
    }

    /// Sample using a 3D sphere-surface coordinate to avoid UV seam artifacts.
    pub fn sample_3d(&self, point: glam::DVec3) -> f64 {
        let mut total = 0.0;
        let mut frequency = self.params.base_frequency;
        let mut amplitude = self.params.amplitude;

        for _ in 0..self.params.octaves {
            let nx = point.x * frequency;
            let ny = point.y * frequency;
            let nz = point.z * frequency;
            let noise_val = self.noise.get([nx, ny, nz]);
            total += noise_val * amplitude;

            frequency *= self.params.lacunarity;
            amplitude *= self.params.persistence;
        }

        total
    }

    /// Compute the theoretical maximum absolute amplitude (geometric series sum).
    ///
    /// Useful for normalizing output or clamping to a known range.
    pub fn max_amplitude(&self) -> f64 {
        let mut sum = 0.0;
        let mut amp = self.params.amplitude;
        for _ in 0..self.params.octaves {
            sum += amp;
            amp *= self.params.persistence;
        }
        sum
    }

    /// Return a reference to the current parameters.
    pub fn params(&self) -> &HeightmapParams {
        &self.params
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f64 = 1e-12;

    #[test]
    fn test_determinism_same_seed_same_coord() {
        let params = HeightmapParams {
            seed: 42,
            ..Default::default()
        };
        let sampler_a = HeightmapSampler::new(params.clone());
        let sampler_b = HeightmapSampler::new(params);

        let h1 = sampler_a.sample(100.0, 200.0);
        let h2 = sampler_b.sample(100.0, 200.0);
        assert!(
            (h1 - h2).abs() < EPSILON,
            "Same seed + same coord must produce identical height: {h1} vs {h2}"
        );
    }

    #[test]
    fn test_different_seeds_produce_different_heights() {
        let sampler_a = HeightmapSampler::new(HeightmapParams {
            seed: 1,
            ..Default::default()
        });
        let sampler_b = HeightmapSampler::new(HeightmapParams {
            seed: 999,
            ..Default::default()
        });

        let h1 = sampler_a.sample(500.0, 500.0);
        let h2 = sampler_b.sample(500.0, 500.0);
        assert!(
            (h1 - h2).abs() > EPSILON,
            "Different seeds should produce different heights: {h1} vs {h2}"
        );
    }

    #[test]
    fn test_height_within_expected_range() {
        let params = HeightmapParams::default();
        let sampler = HeightmapSampler::new(params);
        let max_amp = sampler.max_amplitude();

        for x in (0..100).map(|i| i as f64 * 10.0) {
            for y in (0..100).map(|i| i as f64 * 10.0) {
                let h = sampler.sample(x, y);
                assert!(
                    h.abs() <= max_amp + EPSILON,
                    "Height {h} exceeds max amplitude {max_amp} at ({x}, {y})"
                );
            }
        }
    }

    #[test]
    fn test_more_octaves_adds_detail() {
        let step = 0.5;
        let mut diff_1oct = 0.0;
        let mut diff_8oct = 0.0;
        let count = 1000;

        let sampler_1 = HeightmapSampler::new(HeightmapParams {
            seed: 7,
            octaves: 1,
            ..Default::default()
        });
        let sampler_8 = HeightmapSampler::new(HeightmapParams {
            seed: 7,
            octaves: 8,
            ..Default::default()
        });

        for i in 0..count {
            let x = i as f64 * step;
            let h1_a = sampler_1.sample(x, 0.0);
            let h1_b = sampler_1.sample(x + step, 0.0);
            diff_1oct += (h1_b - h1_a).abs();

            let h8_a = sampler_8.sample(x, 0.0);
            let h8_b = sampler_8.sample(x + step, 0.0);
            diff_8oct += (h8_b - h8_a).abs();
        }

        diff_1oct /= count as f64;
        diff_8oct /= count as f64;

        assert!(
            diff_8oct > diff_1oct,
            "8 octaves should have more high-frequency detail than 1 octave: \
             avg_diff_1={diff_1oct}, avg_diff_8={diff_8oct}"
        );
    }

    #[test]
    fn test_smooth_gradient_no_discontinuities() {
        let sampler = HeightmapSampler::new(HeightmapParams {
            seed: 42,
            ..Default::default()
        });
        let step = 0.01;
        let max_allowed_delta = sampler.max_amplitude() * 0.1;

        for i in 0..10_000 {
            let x = i as f64 * step;
            let h_a = sampler.sample(x, 0.0);
            let h_b = sampler.sample(x + step, 0.0);
            let delta = (h_b - h_a).abs();
            assert!(
                delta < max_allowed_delta,
                "Discontinuity at x={x}: delta={delta} exceeds max={max_allowed_delta}"
            );
        }
    }

    #[test]
    fn test_max_amplitude_calculation() {
        let params = HeightmapParams {
            amplitude: 1000.0,
            persistence: 0.5,
            octaves: 4,
            ..Default::default()
        };
        let sampler = HeightmapSampler::new(params);
        let expected = 1875.0;
        assert!(
            (sampler.max_amplitude() - expected).abs() < EPSILON,
            "Max amplitude should be {expected}, got {}",
            sampler.max_amplitude()
        );
    }

    #[test]
    fn test_zero_amplitude_returns_zero() {
        let sampler = HeightmapSampler::new(HeightmapParams {
            amplitude: 0.0,
            ..Default::default()
        });
        let h = sampler.sample(123.0, 456.0);
        assert!(
            h.abs() < EPSILON,
            "Zero amplitude should produce zero height, got {h}"
        );
    }
}
