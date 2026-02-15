# Particle Emitter Component

## Problem

Nebula Engine needs a particle system that integrates with the existing ECS architecture. Before any visual effects can be rendered — dust kicked up by footsteps, sparks from collisions, engine exhaust trails — there must be a data model that defines what a particle emitter is and how it stores its particles. Without a well-defined emitter component, particle behavior would be scattered across ad-hoc systems with no unified control over emission rate, lifetime, color, or memory budgets. Every downstream particle system (GPU simulation, LOD, presets) depends on having a clean, data-driven emitter component that the ECS can query, enable, disable, and configure at runtime.

The particle storage itself is a critical design decision. Storing each particle as a separate struct-of-arrays (SoA) rather than an array-of-structs (AoS) is essential for GPU upload performance and cache locality during CPU-side updates. Millions of particles across hundreds of emitters demand a flat, contiguous buffer layout that can be memcpy'd to GPU buffers without per-particle indirection.

## Solution

Define a `ParticleEmitter` component and its associated `ParticleBuffer` in the `nebula-particles` crate. The emitter is an ECS component that attaches to any entity and describes how particles are spawned. The buffer holds the live particle data in SoA layout.

### Emitter Component

```rust
/// ECS component that defines a particle emitter's configuration.
#[derive(Clone, Debug)]
pub struct ParticleEmitter {
    /// Particles spawned per second.
    pub emission_rate: f32,
    /// Minimum and maximum lifetime in seconds for each particle.
    pub lifetime_range: (f32, f32),
    /// Minimum and maximum initial velocity (local space).
    pub velocity_range: (Vec3, Vec3),
    /// Color gradient over normalized lifetime [0.0, 1.0].
    pub color_over_lifetime: ColorGradient,
    /// Size curve over normalized lifetime [0.0, 1.0].
    pub size_over_lifetime: Curve<f32>,
    /// Gravity multiplier. 0.0 = no gravity, 1.0 = full world gravity.
    pub gravity_influence: f32,
    /// Hard cap on particles alive at any time for this emitter.
    pub max_particles: u32,
    /// Whether the emitter is currently active and spawning.
    pub enabled: bool,
    /// Accumulated fractional particles from previous frames.
    pub emission_accumulator: f32,
}
```

The `emission_accumulator` tracks sub-frame particle spawning. If the emission rate is 30 particles/sec and the frame delta is 0.016s, the system should spawn 0.48 particles. The accumulator carries the 0.48 forward until it crosses 1.0, at which point a particle is emitted and the integer part is subtracted. This prevents emission rate drift at varying frame rates.

### Particle Buffer (SoA Layout)

```rust
/// Flat, contiguous storage for all live particles belonging to one emitter.
/// Uses struct-of-arrays layout for cache efficiency and GPU upload.
pub struct ParticleBuffer {
    /// World-space positions. Length == alive count.
    pub positions: Vec<Vec3>,
    /// World-space velocities.
    pub velocities: Vec<Vec3>,
    /// Current age in seconds for each particle.
    pub ages: Vec<f32>,
    /// Maximum lifetime in seconds for each particle (sampled at spawn).
    pub lifetimes: Vec<f32>,
    /// Current RGBA color for each particle.
    pub colors: Vec<Vec4>,
    /// Current billboard size for each particle.
    pub sizes: Vec<f32>,
    /// Number of alive particles. Always <= emitter.max_particles.
    pub alive_count: u32,
}
```

### Spawning Logic

Each frame, the `ParticleSpawnSystem` iterates all entities with a `ParticleEmitter` and `ParticleBuffer`:

1. **Skip disabled emitters.** If `emitter.enabled == false`, do nothing.
2. **Accumulate emission.** Add `emitter.emission_rate * dt` to `emitter.emission_accumulator`.
3. **Spawn particles.** While `emission_accumulator >= 1.0` and `buffer.alive_count < emitter.max_particles`:
   - Sample a random lifetime from `lifetime_range`.
   - Sample a random initial velocity from `velocity_range`.
   - Sample the initial color from `color_over_lifetime` at `t = 0.0`.
   - Sample the initial size from `size_over_lifetime` at `t = 0.0`.
   - Push values into each SoA vector.
   - Increment `alive_count`.
   - Subtract 1.0 from `emission_accumulator`.
4. **Cap accumulator.** Clamp `emission_accumulator` to `[0.0, max_particles as f32]` to prevent burst spawning after long pauses (e.g., when the emitter was off-screen).

### Particle Removal

The `ParticleUpdateSystem` runs before spawning and removes expired particles:

1. Iterate all particles by index.
2. If `ages[i] >= lifetimes[i]`, the particle is dead.
3. Swap the dead particle with the last alive particle in all SoA arrays (swap-remove).
4. Decrement `alive_count`.

This swap-remove approach keeps all alive particles contiguous at the front of the arrays, which is essential for efficient GPU buffer uploads (a single `write_buffer` call with a contiguous slice).

### ColorGradient and Curve

```rust
/// A gradient defined by color stops at normalized time values.
#[derive(Clone, Debug)]
pub struct ColorGradient {
    pub stops: Vec<(f32, Vec4)>,
}

impl ColorGradient {
    pub fn sample(&self, t: f32) -> Vec4 {
        // Linearly interpolate between the two nearest stops.
        // Clamp t to [0.0, 1.0].
        // If only one stop, return that color.
        ...
    }
}

/// A piecewise-linear curve for scalar values over normalized time.
#[derive(Clone, Debug)]
pub struct Curve<T: Lerp> {
    pub keyframes: Vec<(f32, T)>,
}

impl<T: Lerp> Curve<T> {
    pub fn sample(&self, t: f32) -> T { ... }
}
```

### Default Emitter

```rust
impl Default for ParticleEmitter {
    fn default() -> Self {
        Self {
            emission_rate: 10.0,
            lifetime_range: (1.0, 2.0),
            velocity_range: (Vec3::new(-1.0, 0.0, -1.0), Vec3::new(1.0, 2.0, 1.0)),
            color_over_lifetime: ColorGradient {
                stops: vec![
                    (0.0, Vec4::new(1.0, 1.0, 1.0, 1.0)),
                    (1.0, Vec4::new(1.0, 1.0, 1.0, 0.0)),
                ],
            },
            size_over_lifetime: Curve {
                keyframes: vec![(0.0, 0.1), (1.0, 0.0)],
            },
            gravity_influence: 0.0,
            max_particles: 1000,
            enabled: true,
            emission_accumulator: 0.0,
        }
    }
}
```

## Outcome

A `ParticleEmitter` ECS component and `ParticleBuffer` storage type in the `nebula-particles` crate. The emitter describes emission behavior declaratively (rate, lifetime, velocity, color, size, gravity, budget). The buffer stores live particle data in SoA layout for cache-friendly iteration and efficient GPU upload. A `ParticleSpawnSystem` spawns particles each frame using fractional accumulation, and a `ParticleUpdateSystem` removes expired particles via swap-remove. Every downstream system — GPU simulation, rendering, LOD, presets — builds on this component.

## Demo Integration

**Demo crate:** `nebula-demo`

A particle emitter component spawns colored quads that rise upward in a cone pattern from a point on the terrain.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `wgpu` | `28.0` | GPU buffer types referenced by particle buffer |
| `glam` | `0.29` | `Vec3`, `Vec4` math types for position, velocity, color |
| `rand` | `0.9` | Random sampling for lifetime, velocity, and spawn jitter |
| `serde` | `1.0` | Serialization of emitter configurations for asset loading |
| `thiserror` | `2.0` | Error type derivation for particle system errors |
| `log` | `0.4` | Logging emitter lifecycle events |

All dependencies are declared in `[workspace.dependencies]` and consumed via `{ workspace = true }` in the `nebula-particles` crate's `Cargo.toml`. Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create a default emitter with a specific emission rate.
    fn emitter_with_rate(rate: f32) -> ParticleEmitter {
        ParticleEmitter {
            emission_rate: rate,
            ..Default::default()
        }
    }

    /// Helper: create an empty particle buffer.
    fn empty_buffer() -> ParticleBuffer {
        ParticleBuffer {
            positions: Vec::new(),
            velocities: Vec::new(),
            ages: Vec::new(),
            lifetimes: Vec::new(),
            colors: Vec::new(),
            sizes: Vec::new(),
            alive_count: 0,
        }
    }

    #[test]
    fn test_emitter_spawns_at_configured_rate() {
        // An emitter at 100 particles/sec over a 0.1s frame should spawn ~10 particles.
        let mut emitter = emitter_with_rate(100.0);
        let mut buffer = empty_buffer();
        let dt = 0.1;

        spawn_particles(&mut emitter, &mut buffer, dt);

        assert_eq!(buffer.alive_count, 10);
        assert_eq!(buffer.positions.len(), 10);
        assert_eq!(buffer.velocities.len(), 10);
    }

    #[test]
    fn test_max_particles_is_capped() {
        // An emitter with max_particles = 5 should never exceed 5,
        // even if the emission rate would produce more.
        let mut emitter = emitter_with_rate(1000.0);
        emitter.max_particles = 5;
        let mut buffer = empty_buffer();

        spawn_particles(&mut emitter, &mut buffer, 1.0);

        assert_eq!(buffer.alive_count, 5);
        assert!(buffer.positions.len() <= 5);
    }

    #[test]
    fn test_particle_lifetime_causes_removal() {
        // Spawn a particle with lifetime 0.5s, advance time by 0.6s,
        // then run the update — the particle should be removed.
        let mut buffer = empty_buffer();
        buffer.positions.push(Vec3::ZERO);
        buffer.velocities.push(Vec3::ZERO);
        buffer.ages.push(0.6);
        buffer.lifetimes.push(0.5);
        buffer.colors.push(Vec4::ONE);
        buffer.sizes.push(1.0);
        buffer.alive_count = 1;

        remove_expired_particles(&mut buffer);

        assert_eq!(buffer.alive_count, 0);
        assert!(buffer.positions.is_empty());
    }

    #[test]
    fn test_zero_emission_rate_spawns_nothing() {
        let mut emitter = emitter_with_rate(0.0);
        let mut buffer = empty_buffer();

        spawn_particles(&mut emitter, &mut buffer, 1.0);

        assert_eq!(buffer.alive_count, 0);
    }

    #[test]
    fn test_emitter_disabled_spawns_nothing() {
        let mut emitter = emitter_with_rate(100.0);
        emitter.enabled = false;
        let mut buffer = empty_buffer();

        spawn_particles(&mut emitter, &mut buffer, 1.0);

        assert_eq!(buffer.alive_count, 0);
    }

    #[test]
    fn test_emitter_can_be_toggled() {
        let mut emitter = emitter_with_rate(100.0);
        let mut buffer = empty_buffer();

        // Disabled — no spawning.
        emitter.enabled = false;
        spawn_particles(&mut emitter, &mut buffer, 0.1);
        assert_eq!(buffer.alive_count, 0);

        // Re-enable — particles spawn.
        emitter.enabled = true;
        spawn_particles(&mut emitter, &mut buffer, 0.1);
        assert!(buffer.alive_count > 0);
    }

    #[test]
    fn test_emission_accumulator_carries_fractional_particles() {
        // At 1 particle/sec with dt=0.3, three frames should yield:
        // Frame 1: accum=0.3 -> 0 spawned
        // Frame 2: accum=0.6 -> 0 spawned
        // Frame 3: accum=0.9 -> 0 spawned
        // Frame 4: accum=1.2 -> 1 spawned, accum=0.2
        let mut emitter = emitter_with_rate(1.0);
        let mut buffer = empty_buffer();

        for _ in 0..3 {
            spawn_particles(&mut emitter, &mut buffer, 0.3);
        }
        assert_eq!(buffer.alive_count, 0);

        spawn_particles(&mut emitter, &mut buffer, 0.3);
        assert_eq!(buffer.alive_count, 1);
    }

    #[test]
    fn test_color_gradient_sample_interpolates() {
        let gradient = ColorGradient {
            stops: vec![
                (0.0, Vec4::new(1.0, 0.0, 0.0, 1.0)),
                (1.0, Vec4::new(0.0, 0.0, 1.0, 1.0)),
            ],
        };

        let mid = gradient.sample(0.5);
        assert!((mid.x - 0.5).abs() < 0.01); // red channel at midpoint
        assert!((mid.z - 0.5).abs() < 0.01); // blue channel at midpoint
    }

    #[test]
    fn test_swap_remove_keeps_alive_particles_contiguous() {
        // Buffer with 3 particles: kill the middle one.
        // After swap-remove, the last particle takes its place.
        let mut buffer = ParticleBuffer {
            positions: vec![Vec3::X, Vec3::Y, Vec3::Z],
            velocities: vec![Vec3::ZERO; 3],
            ages: vec![0.0, 999.0, 0.0], // middle particle is expired
            lifetimes: vec![1.0, 0.5, 1.0],
            colors: vec![Vec4::ONE; 3],
            sizes: vec![1.0; 3],
            alive_count: 3,
        };

        remove_expired_particles(&mut buffer);

        assert_eq!(buffer.alive_count, 2);
        // The last particle (Z) should have been swapped into index 1.
        assert_eq!(buffer.positions[0], Vec3::X);
        assert_eq!(buffer.positions[1], Vec3::Z);
    }
}
```
