# Magic Effect Particles

## Problem

Fantasy and sci-fi games rely heavily on particle effects to communicate magical abilities — a fireball must trail embers, a healing spell should radiate green light, and a shield must shimmer with protective energy. These effects require visual techniques beyond what standard particle presets offer: additive blending for glowing luminosity (particles brighten when overlapping instead of occluding), sinusoidal velocity modulation for spiraling motion, radial burst spawning for impact effects, and continuous trail emission along a moving path. Without purpose-built magic effect presets and the blending/motion infrastructure to support them, gameplay programmers are forced to hack around the particle system's limitations, producing effects that look flat and unconvincing.

Each magic school (fire, ice, nature, arcane, dark) also needs a distinct color palette so players can instantly identify spell types at a glance. Color consistency across all effects of a school is a design requirement, not an afterthought.

## Solution

Extend the `nebula-particles` crate with magic-specific particle presets in a `presets::magic` module, and add support for additive blending and sinusoidal velocity modulation to the emitter and rendering infrastructure.

### Additive Blending

Standard alpha blending (`src_alpha * src + (1 - src_alpha) * dst`) darkens overlapping transparent particles. For glowing effects, additive blending (`src_alpha * src + dst`) is needed — overlapping particles become brighter, simulating light emission.

Add a `BlendMode` field to `ParticleEmitter`:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticleBlendMode {
    /// Standard alpha blending. Particles occlude based on alpha.
    AlphaBlend,
    /// Additive blending. Overlapping particles brighten.
    Additive,
}

pub struct ParticleEmitter {
    // ... existing fields from Story 01 ...
    pub blend_mode: ParticleBlendMode,
}
```

The render pipeline creates two `wgpu::RenderPipeline` variants — one for each blend mode — and sorts emitters by blend mode to minimize pipeline switches. The additive pipeline uses:

```rust
wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::SrcAlpha,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    },
}
```

### Sinusoidal Velocity Modulation (Spiral Motion)

For spiraling trails and orbiting particles, add an optional oscillation modifier:

```rust
/// Sinusoidal offset applied to particle velocity each frame.
#[derive(Clone, Debug)]
pub struct VelocityOscillation {
    /// Amplitude of the sine wave on the X/Z plane (perpendicular to forward).
    pub amplitude: f32,
    /// Frequency in Hz (full cycles per second).
    pub frequency: f32,
    /// Phase offset in radians (randomized per particle for variation).
    pub phase_offset_range: (f32, f32),
}
```

In the compute shader, the oscillation is applied:

```wgsl
// Spiral: offset velocity perpendicular to the main direction.
let phase = particle.age * oscillation.frequency * 6.2831853 + particle.phase_offset;
let spiral_x = sin(phase) * oscillation.amplitude;
let spiral_z = cos(phase) * oscillation.amplitude;
p.velocity.x += spiral_x * params.dt;
p.velocity.z += spiral_z * params.dt;
```

### Color Palettes by Magic School

```rust
pub struct MagicSchoolPalette {
    pub primary: Vec4,
    pub secondary: Vec4,
    pub accent: Vec4,
}

pub fn fire_palette() -> MagicSchoolPalette {
    MagicSchoolPalette {
        primary: Vec4::new(1.0, 0.4, 0.0, 1.0),    // orange
        secondary: Vec4::new(1.0, 0.15, 0.0, 1.0),  // red-orange
        accent: Vec4::new(1.0, 1.0, 0.5, 1.0),      // bright yellow core
    }
}

pub fn ice_palette() -> MagicSchoolPalette {
    MagicSchoolPalette {
        primary: Vec4::new(0.5, 0.8, 1.0, 1.0),    // light blue
        secondary: Vec4::new(0.2, 0.5, 0.9, 1.0),  // medium blue
        accent: Vec4::new(1.0, 1.0, 1.0, 1.0),     // white
    }
}

pub fn nature_palette() -> MagicSchoolPalette {
    MagicSchoolPalette {
        primary: Vec4::new(0.2, 0.8, 0.3, 1.0),    // green
        secondary: Vec4::new(0.1, 0.6, 0.2, 1.0),  // dark green
        accent: Vec4::new(0.9, 1.0, 0.5, 1.0),     // yellow-green glow
    }
}

pub fn arcane_palette() -> MagicSchoolPalette {
    MagicSchoolPalette {
        primary: Vec4::new(0.6, 0.2, 1.0, 1.0),    // purple
        secondary: Vec4::new(0.3, 0.0, 0.8, 1.0),  // deep violet
        accent: Vec4::new(0.9, 0.7, 1.0, 1.0),     // lavender
    }
}
```

### Magic Presets

#### Glowing Orbs

```rust
/// Floating luminous orbs with additive glow. Used for ambient magic, collectibles.
pub fn glowing_orbs(palette: &MagicSchoolPalette) -> ParticleEmitter {
    ParticleEmitter {
        emission_rate: 5.0,
        lifetime_range: (2.0, 4.0),
        velocity_range: (Vec3::new(-0.2, -0.1, -0.2), Vec3::new(0.2, 0.3, 0.2)),
        color_over_lifetime: ColorGradient {
            stops: vec![
                (0.0, palette.accent),
                (0.3, palette.primary),
                (0.8, palette.secondary),
                (1.0, Vec4::new(palette.secondary.x, palette.secondary.y, palette.secondary.z, 0.0)),
            ],
        },
        size_over_lifetime: Curve {
            keyframes: vec![(0.0, 0.05), (0.5, 0.15), (1.0, 0.05)],
        },
        gravity_influence: 0.0,
        max_particles: 50,
        blend_mode: ParticleBlendMode::Additive,
        enabled: true,
        emission_accumulator: 0.0,
    }
}
```

#### Spiraling Trails

```rust
/// Particles that spiral along a path. Used for projectile trails, channeling effects.
pub fn spiral_trail(palette: &MagicSchoolPalette) -> (ParticleEmitter, VelocityOscillation) {
    let emitter = ParticleEmitter {
        emission_rate: 60.0,
        lifetime_range: (0.3, 0.6),
        velocity_range: (Vec3::new(0.0, 0.0, -5.0), Vec3::new(0.0, 0.0, -8.0)),
        color_over_lifetime: ColorGradient {
            stops: vec![
                (0.0, palette.accent),
                (0.5, palette.primary),
                (1.0, Vec4::new(palette.primary.x, palette.primary.y, palette.primary.z, 0.0)),
            ],
        },
        size_over_lifetime: Curve {
            keyframes: vec![(0.0, 0.03), (0.5, 0.06), (1.0, 0.01)],
        },
        gravity_influence: 0.0,
        max_particles: 300,
        blend_mode: ParticleBlendMode::Additive,
        enabled: true,
        emission_accumulator: 0.0,
    };

    let oscillation = VelocityOscillation {
        amplitude: 2.0,
        frequency: 3.0,
        phase_offset_range: (0.0, std::f32::consts::TAU),
    };

    (emitter, oscillation)
}
```

#### Impact Burst

```rust
/// Radial burst of particles from a single point. Used for spell impacts, explosions.
pub fn impact_burst(palette: &MagicSchoolPalette) -> ParticleEmitter {
    ParticleEmitter {
        emission_rate: 500.0, // high burst, then disable after one frame
        lifetime_range: (0.2, 0.5),
        velocity_range: (
            Vec3::new(-5.0, -5.0, -5.0),
            Vec3::new(5.0, 5.0, 5.0),
        ),
        color_over_lifetime: ColorGradient {
            stops: vec![
                (0.0, palette.accent),
                (0.3, palette.primary),
                (1.0, Vec4::ZERO),
            ],
        },
        size_over_lifetime: Curve {
            keyframes: vec![(0.0, 0.1), (1.0, 0.02)],
        },
        gravity_influence: 0.0,
        max_particles: 200,
        blend_mode: ParticleBlendMode::Additive,
        enabled: true,
        emission_accumulator: 0.0,
    }
}
```

#### Healing Aura

```rust
/// Rising, gentle particles surrounding a character. Used for heals, buffs, regen.
pub fn healing_aura(palette: &MagicSchoolPalette) -> ParticleEmitter {
    ParticleEmitter {
        emission_rate: 25.0,
        lifetime_range: (0.8, 1.5),
        velocity_range: (Vec3::new(-0.5, 0.5, -0.5), Vec3::new(0.5, 1.5, 0.5)),
        color_over_lifetime: ColorGradient {
            stops: vec![
                (0.0, Vec4::new(palette.primary.x, palette.primary.y, palette.primary.z, 0.0)),
                (0.2, palette.primary),
                (0.8, palette.secondary),
                (1.0, Vec4::new(palette.secondary.x, palette.secondary.y, palette.secondary.z, 0.0)),
            ],
        },
        size_over_lifetime: Curve {
            keyframes: vec![(0.0, 0.02), (0.5, 0.1), (1.0, 0.02)],
        },
        gravity_influence: -0.2,
        max_particles: 150,
        blend_mode: ParticleBlendMode::Additive,
        enabled: true,
        emission_accumulator: 0.0,
    }
}
```

#### Shield Effect

```rust
/// Particles orbiting around a character in a spherical shell. Used for shields, barriers.
pub fn shield_effect(palette: &MagicSchoolPalette) -> (ParticleEmitter, VelocityOscillation) {
    let emitter = ParticleEmitter {
        emission_rate: 40.0,
        lifetime_range: (0.5, 1.0),
        velocity_range: (Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0)),
        color_over_lifetime: ColorGradient {
            stops: vec![
                (0.0, palette.accent),
                (0.5, palette.primary),
                (1.0, Vec4::new(palette.primary.x, palette.primary.y, palette.primary.z, 0.0)),
            ],
        },
        size_over_lifetime: Curve {
            keyframes: vec![(0.0, 0.04), (0.5, 0.08), (1.0, 0.02)],
        },
        gravity_influence: 0.0,
        max_particles: 250,
        blend_mode: ParticleBlendMode::Additive,
        enabled: true,
        emission_accumulator: 0.0,
    };

    let oscillation = VelocityOscillation {
        amplitude: 1.5,
        frequency: 2.0,
        phase_offset_range: (0.0, std::f32::consts::TAU),
    };

    (emitter, oscillation)
}
```

### Trail Spawning Along a Path

For projectile trails, particles must be spawned at interpolated positions along the emitter's movement path between frames (not just at the current position). The `TrailSpawner` subdivides the emitter's displacement:

```rust
pub struct TrailSpawner;

impl TrailSpawner {
    /// Spawn trail particles evenly distributed between prev_pos and current_pos.
    pub fn spawn_along_path(
        emitter: &mut ParticleEmitter,
        buffer: &mut ParticleBuffer,
        prev_pos: Vec3,
        current_pos: Vec3,
        dt: f32,
    ) {
        let distance = (current_pos - prev_pos).length();
        let count = (emitter.emission_rate * dt) as u32;
        for i in 0..count {
            let t = i as f32 / count.max(1) as f32;
            let spawn_pos = prev_pos.lerp(current_pos, t);
            // Spawn particle at spawn_pos with emitter's configured properties.
            ...
        }
    }
}
```

## Outcome

A `presets::magic` module containing factory functions for glowing orbs, spiral trails, impact bursts, healing auras, and shield effects. Each uses a `MagicSchoolPalette` for consistent per-school coloring. Additive blending support via `ParticleBlendMode::Additive` produces luminous glow effects. `VelocityOscillation` enables spiral and orbital motion in the compute shader. `TrailSpawner` distributes particles along a moving path for smooth projectile trails. All effects render through the standard GPU particle pipeline from Story 02.

## Demo Integration

**Demo crate:** `nebula-demo`

A spell cast produces a swirl of colored energy particles that converge on a target point and burst on impact.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `wgpu` | `28.0` | Additive blend state configuration in render pipeline |
| `glam` | `0.29` | Vec3, Vec4 for positions, colors, and interpolation |
| `rand` | `0.9` | Random phase offset sampling for spiral variation |

No new external dependencies beyond what `nebula-particles` already requires. Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_additive_blend_mode_produces_glow() {
        let palette = fire_palette();
        let emitter = glowing_orbs(&palette);
        assert_eq!(
            emitter.blend_mode,
            ParticleBlendMode::Additive,
            "glowing orbs must use additive blending"
        );
    }

    #[test]
    fn test_all_magic_presets_use_additive_blending() {
        let palette = arcane_palette();
        assert_eq!(glowing_orbs(&palette).blend_mode, ParticleBlendMode::Additive);
        assert_eq!(spiral_trail(&palette).0.blend_mode, ParticleBlendMode::Additive);
        assert_eq!(impact_burst(&palette).blend_mode, ParticleBlendMode::Additive);
        assert_eq!(healing_aura(&palette).blend_mode, ParticleBlendMode::Additive);
        assert_eq!(shield_effect(&palette).0.blend_mode, ParticleBlendMode::Additive);
    }

    #[test]
    fn test_spiral_motion_follows_sine_curve() {
        let (_emitter, oscillation) = spiral_trail(&arcane_palette());
        // Verify oscillation parameters produce sinusoidal motion.
        let amplitude = oscillation.amplitude;
        let frequency = oscillation.frequency;
        assert!(amplitude > 0.0, "spiral amplitude must be positive");
        assert!(frequency > 0.0, "spiral frequency must be positive");

        // Compute expected displacement at t=0.25 (quarter period).
        let t = 0.25;
        let phase = t * frequency * std::f32::consts::TAU;
        let x_offset = phase.sin() * amplitude;
        // At quarter cycle of 3Hz, phase = 0.25 * 3 * TAU = 4.712 rad.
        // sin(4.712) ~ -1.0, so offset ~ -2.0.
        assert!(
            x_offset.abs() <= amplitude,
            "sine offset {} should be within amplitude {}",
            x_offset,
            amplitude
        );
    }

    #[test]
    fn test_fire_palette_color_matches_school() {
        let palette = fire_palette();
        // Fire primary should be orange-ish: high red, medium green, low blue.
        assert!(palette.primary.x > 0.8, "fire primary red should be high");
        assert!(palette.primary.z < 0.3, "fire primary blue should be low");
    }

    #[test]
    fn test_ice_palette_color_matches_school() {
        let palette = ice_palette();
        // Ice primary should be blue-ish: high blue, medium-high green.
        assert!(palette.primary.z > 0.8, "ice primary blue should be high");
    }

    #[test]
    fn test_nature_palette_color_matches_school() {
        let palette = nature_palette();
        // Nature primary should be green-ish.
        assert!(palette.primary.y > 0.6, "nature primary green should be high");
        assert!(palette.primary.x < 0.5, "nature primary red should be low");
    }

    #[test]
    fn test_trail_particles_follow_path() {
        let palette = arcane_palette();
        let (mut emitter, _osc) = spiral_trail(&palette);
        let mut buffer = empty_buffer();

        let prev_pos = Vec3::new(0.0, 0.0, 0.0);
        let current_pos = Vec3::new(10.0, 0.0, 0.0);

        TrailSpawner::spawn_along_path(&mut emitter, &mut buffer, prev_pos, current_pos, 0.1);

        // All spawned particles should have positions between prev_pos and current_pos.
        for i in 0..buffer.alive_count as usize {
            let pos = buffer.positions[i];
            assert!(pos.x >= 0.0 && pos.x <= 10.0, "particle x={} should be between 0 and 10", pos.x);
        }
    }

    #[test]
    fn test_impact_burst_spawns_radially() {
        let palette = fire_palette();
        let emitter = impact_burst(&palette);
        // Impact burst should have symmetric velocity range (negative to positive).
        let min_vel = emitter.velocity_range.0;
        let max_vel = emitter.velocity_range.1;
        assert!(min_vel.x < 0.0 && max_vel.x > 0.0, "burst should emit in all x directions");
        assert!(min_vel.y < 0.0 && max_vel.y > 0.0, "burst should emit in all y directions");
        assert!(min_vel.z < 0.0 && max_vel.z > 0.0, "burst should emit in all z directions");
    }

    #[test]
    fn test_healing_aura_rises() {
        let palette = nature_palette();
        let emitter = healing_aura(&palette);
        assert!(
            emitter.velocity_range.0.y > 0.0,
            "healing aura particles should rise (positive min y velocity)"
        );
    }

    #[test]
    fn test_shield_has_orbital_oscillation() {
        let palette = arcane_palette();
        let (_emitter, oscillation) = shield_effect(&palette);
        assert!(oscillation.amplitude > 0.0, "shield should have orbital oscillation");
        assert!(oscillation.frequency > 0.0, "shield oscillation frequency must be positive");
    }
}
```
