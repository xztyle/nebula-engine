# Common Particle Effects

## Problem

Every game needs a baseline set of visual effects — dust clouds when objects land, sparks when metal strikes metal, smoke rising from fires, flames themselves, and debris scattering from explosions. Without a library of pre-configured emitter presets, every gameplay programmer who needs these effects must manually tune emission rate, lifetime, velocity, color gradients, gravity, and size curves from scratch. This leads to inconsistent visual quality (one person's "smoke" looks nothing like another's), duplicated work, and long iteration cycles. A standard preset library establishes the engine's visual vocabulary and gives teams a starting point they can tweak rather than build from zero.

The presets must be pure data — factory functions that return configured `ParticleEmitter` components — not separate systems or special-cased rendering paths. This keeps the architecture clean: a preset is just an emitter configuration, and the GPU simulation and rendering pipeline from Story 02 handles everything uniformly.

## Solution

Define a `presets` module in the `nebula-particles` crate containing factory functions that return fully configured `ParticleEmitter` instances for each common effect. Each function encodes the visual parameters that define the effect's look and feel.

### Preset: Dust

```rust
/// Small brown/tan particles drifting slowly outward and fading.
/// Use case: footsteps on dirt, object landing on sand, wind blowing across terrain.
pub fn dust() -> ParticleEmitter {
    ParticleEmitter {
        emission_rate: 20.0,
        lifetime_range: (0.3, 0.8),
        velocity_range: (
            Vec3::new(-0.5, 0.1, -0.5),
            Vec3::new(0.5, 0.5, 0.5),
        ),
        color_over_lifetime: ColorGradient {
            stops: vec![
                (0.0, Vec4::new(0.76, 0.70, 0.50, 0.6)),  // tan, semi-transparent
                (0.5, Vec4::new(0.76, 0.70, 0.50, 0.3)),
                (1.0, Vec4::new(0.76, 0.70, 0.50, 0.0)),  // fade to invisible
            ],
        },
        size_over_lifetime: Curve {
            keyframes: vec![(0.0, 0.05), (0.5, 0.1), (1.0, 0.15)],
        },
        gravity_influence: 0.1,       // slight settling
        max_particles: 200,
        enabled: true,
        emission_accumulator: 0.0,
    }
}
```

### Preset: Sparks

```rust
/// Bright, fast, tiny particles with strong gravity pull.
/// Use case: welding, metal impact, grinding, electrical discharge.
pub fn sparks() -> ParticleEmitter {
    ParticleEmitter {
        emission_rate: 80.0,
        lifetime_range: (0.1, 0.4),
        velocity_range: (
            Vec3::new(-3.0, 1.0, -3.0),
            Vec3::new(3.0, 5.0, 3.0),
        ),
        color_over_lifetime: ColorGradient {
            stops: vec![
                (0.0, Vec4::new(1.0, 1.0, 0.8, 1.0)),   // white-hot
                (0.3, Vec4::new(1.0, 0.8, 0.2, 1.0)),   // bright yellow
                (1.0, Vec4::new(1.0, 0.3, 0.0, 0.0)),   // red, fading out
            ],
        },
        size_over_lifetime: Curve {
            keyframes: vec![(0.0, 0.02), (1.0, 0.005)],
        },
        gravity_influence: 1.0,        // full gravity — sparks arc downward
        max_particles: 500,
        enabled: true,
        emission_accumulator: 0.0,
    }
}
```

### Preset: Smoke

```rust
/// Gray, rising, expanding, fading particles.
/// Use case: campfire smoke, chimney exhaust, burning wreckage aftermath.
pub fn smoke() -> ParticleEmitter {
    ParticleEmitter {
        emission_rate: 15.0,
        lifetime_range: (1.5, 3.0),
        velocity_range: (
            Vec3::new(-0.3, 0.5, -0.3),
            Vec3::new(0.3, 1.5, 0.3),
        ),
        color_over_lifetime: ColorGradient {
            stops: vec![
                (0.0, Vec4::new(0.3, 0.3, 0.3, 0.5)),   // dark gray
                (0.5, Vec4::new(0.5, 0.5, 0.5, 0.3)),   // lighter gray
                (1.0, Vec4::new(0.7, 0.7, 0.7, 0.0)),   // near-white, invisible
            ],
        },
        size_over_lifetime: Curve {
            keyframes: vec![(0.0, 0.1), (0.5, 0.3), (1.0, 0.6)],
        },
        gravity_influence: -0.1,       // negative = buoyancy, rises against gravity
        max_particles: 300,
        enabled: true,
        emission_accumulator: 0.0,
    }
}
```

### Preset: Fire

```rust
/// Orange-to-red particles rising with medium lifetime.
/// Use case: torch flame, campfire, burning object.
pub fn fire() -> ParticleEmitter {
    ParticleEmitter {
        emission_rate: 40.0,
        lifetime_range: (0.3, 0.8),
        velocity_range: (
            Vec3::new(-0.5, 1.0, -0.5),
            Vec3::new(0.5, 3.0, 0.5),
        ),
        color_over_lifetime: ColorGradient {
            stops: vec![
                (0.0, Vec4::new(1.0, 1.0, 0.6, 1.0)),   // bright yellow-white core
                (0.2, Vec4::new(1.0, 0.7, 0.0, 0.9)),   // orange
                (0.6, Vec4::new(1.0, 0.3, 0.0, 0.6)),   // deep orange-red
                (1.0, Vec4::new(0.5, 0.0, 0.0, 0.0)),   // dark red, fading
            ],
        },
        size_over_lifetime: Curve {
            keyframes: vec![(0.0, 0.05), (0.3, 0.15), (1.0, 0.05)],
        },
        gravity_influence: -0.3,       // rises strongly
        max_particles: 400,
        enabled: true,
        emission_accumulator: 0.0,
    }
}
```

### Preset: Debris

```rust
/// Chunky particles with strong gravity and longer lifetime.
/// Use case: explosion aftermath, wall destruction, rock crumbling.
pub fn debris() -> ParticleEmitter {
    ParticleEmitter {
        emission_rate: 50.0,
        lifetime_range: (1.0, 3.0),
        velocity_range: (
            Vec3::new(-4.0, 2.0, -4.0),
            Vec3::new(4.0, 8.0, 4.0),
        ),
        color_over_lifetime: ColorGradient {
            stops: vec![
                (0.0, Vec4::new(0.5, 0.4, 0.3, 1.0)),   // brown/stone color
                (0.8, Vec4::new(0.4, 0.35, 0.25, 1.0)),  // slightly darker
                (1.0, Vec4::new(0.3, 0.25, 0.2, 0.5)),   // faded
            ],
        },
        size_over_lifetime: Curve {
            keyframes: vec![(0.0, 0.08), (1.0, 0.06)],
        },
        gravity_influence: 1.5,        // heavier than normal — chunks fall fast
        max_particles: 300,
        enabled: true,
        emission_accumulator: 0.0,
    }
}
```

### Module Organization

```
nebula-particles/
  src/
    presets/
      mod.rs          // pub mod common;
      common.rs       // dust(), sparks(), smoke(), fire(), debris()
    emitter.rs        // ParticleEmitter, ParticleBuffer (Story 01)
    gpu_sim.rs        // GpuParticleSimulator (Story 02)
    lib.rs
```

Each factory function is a standalone, pure function with no side effects. It returns a `ParticleEmitter` that can be attached to any ECS entity. The caller can modify any field after creation to customize the preset (e.g., `let mut emitter = presets::fire(); emitter.emission_rate = 100.0;`).

## Outcome

A `presets::common` module in the `nebula-particles` crate containing five factory functions — `dust()`, `sparks()`, `smoke()`, `fire()`, `debris()` — each returning a fully configured `ParticleEmitter` component. Gameplay code spawns a standard effect in one line: `commands.spawn((Transform::default(), presets::fire()))`. The presets establish consistent visual vocabulary across the engine and serve as documented examples of the emitter configuration API.

## Demo Integration

**Demo crate:** `nebula-demo`

Breaking dirt produces brown dust particles. Breaking stone produces grey shards. Walking kicks up small specks.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `glam` | `0.29` | `Vec3`, `Vec4` for velocity ranges and color values |

No additional dependencies beyond what `nebula-particles` already depends on from Stories 01 and 02. The presets are pure data construction — no I/O, no GPU access, no randomness at the factory level. Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that all presets produce valid emitters (non-zero rate,
    /// sane lifetime, positive max_particles).
    fn assert_valid_emitter(emitter: &ParticleEmitter) {
        assert!(emitter.max_particles > 0, "max_particles must be positive");
        assert!(emitter.lifetime_range.0 > 0.0, "min lifetime must be positive");
        assert!(
            emitter.lifetime_range.1 >= emitter.lifetime_range.0,
            "max lifetime must be >= min lifetime"
        );
        assert!(
            !emitter.color_over_lifetime.stops.is_empty(),
            "color gradient must have at least one stop"
        );
        assert!(
            !emitter.size_over_lifetime.keyframes.is_empty(),
            "size curve must have at least one keyframe"
        );
    }

    #[test]
    fn test_dust_creates_valid_emitter() {
        let emitter = dust();
        assert_valid_emitter(&emitter);
    }

    #[test]
    fn test_sparks_creates_valid_emitter() {
        let emitter = sparks();
        assert_valid_emitter(&emitter);
    }

    #[test]
    fn test_smoke_creates_valid_emitter() {
        let emitter = smoke();
        assert_valid_emitter(&emitter);
    }

    #[test]
    fn test_fire_creates_valid_emitter() {
        let emitter = fire();
        assert_valid_emitter(&emitter);
    }

    #[test]
    fn test_debris_creates_valid_emitter() {
        let emitter = debris();
        assert_valid_emitter(&emitter);
    }

    #[test]
    fn test_dust_particles_are_slow() {
        let emitter = dust();
        // Max velocity magnitude should be small (< 1.0 per axis).
        let max_vel = emitter.velocity_range.1;
        assert!(max_vel.x.abs() <= 1.0);
        assert!(max_vel.y.abs() <= 1.0);
        assert!(max_vel.z.abs() <= 1.0);
    }

    #[test]
    fn test_sparks_have_gravity() {
        let emitter = sparks();
        assert!(
            emitter.gravity_influence >= 1.0,
            "sparks should have full gravity influence, got {}",
            emitter.gravity_influence
        );
    }

    #[test]
    fn test_smoke_rises() {
        let emitter = smoke();
        // Smoke velocity should have a positive y-component (upward).
        assert!(
            emitter.velocity_range.0.y > 0.0,
            "smoke min velocity y should be positive (upward)"
        );
        // Negative gravity influence = buoyancy.
        assert!(
            emitter.gravity_influence < 0.0,
            "smoke gravity_influence should be negative (buoyant)"
        );
    }

    #[test]
    fn test_fire_has_color_gradient() {
        let emitter = fire();
        let stops = &emitter.color_over_lifetime.stops;
        assert!(stops.len() >= 3, "fire should have at least 3 color stops for a gradient");

        // First stop should be bright (high RGB values).
        let start_color = stops[0].1;
        assert!(start_color.x > 0.8, "fire start should be bright (r > 0.8)");

        // Last stop should be dark/faded.
        let end_color = stops.last().unwrap().1;
        assert!(end_color.w < 0.1, "fire end should be nearly transparent");
    }

    #[test]
    fn test_debris_has_gravity() {
        let emitter = debris();
        assert!(
            emitter.gravity_influence > 1.0,
            "debris should have strong gravity (> 1.0), got {}",
            emitter.gravity_influence
        );
    }

    #[test]
    fn test_presets_are_enabled_by_default() {
        assert!(dust().enabled);
        assert!(sparks().enabled);
        assert!(smoke().enabled);
        assert!(fire().enabled);
        assert!(debris().enabled);
    }

    #[test]
    fn test_preset_can_be_customized_after_creation() {
        let mut emitter = fire();
        emitter.emission_rate = 200.0;
        emitter.max_particles = 2000;
        assert_eq!(emitter.emission_rate, 200.0);
        assert_eq!(emitter.max_particles, 2000);
    }
}
```
