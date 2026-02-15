# Engine Exhaust Particles

## Problem

Spaceship engine exhaust is one of the most prominent visual elements in a space game. Players spend the majority of their time looking at their ship or other ships, and the engine exhaust communicates thrust state, power level, and ship direction at a glance. A bad exhaust effect — one that pops in/out, doesn't align with the nozzle, or ignores zero-gravity physics — breaks immersion immediately.

Engine exhaust has unique requirements that generic particle presets do not cover: the emission must be conical (emanating from a nozzle with a spread angle), the particle velocity must be opposite to the thrust direction (exhaust pushes backward while the ship moves forward), the visual intensity must scale dynamically with throttle input (idle produces a faint flicker, full thrust produces a bright stream), zero-gravity environments mean exhaust trails persist far longer than in atmosphere, and an afterburner mode demands a visually distinct intensification.

## Solution

Implement an `EngineExhaustEmitter` configuration and factory functions in the `presets::exhaust` module of the `nebula-particles` crate. The exhaust system builds on the `ParticleEmitter` component from Story 01 and adds a wrapper that handles nozzle positioning, thrust-relative direction, and throttle scaling.

### Exhaust Configuration

```rust
/// Configuration for spaceship engine exhaust particles.
#[derive(Clone, Debug)]
pub struct EngineExhaustConfig {
    /// World-space position of the engine nozzle (set by the ship's transform).
    pub nozzle_position: Vec3,
    /// Normalized direction of thrust (ship forward). Exhaust emits opposite.
    pub thrust_direction: Vec3,
    /// Half-angle of the emission cone in radians.
    pub cone_half_angle: f32,
    /// Throttle level [0.0, 1.0] — scales emission rate and brightness.
    pub throttle: f32,
    /// Whether afterburner is engaged.
    pub afterburner: bool,
    /// Whether the environment is zero-g (affects particle lifetime).
    pub zero_gravity: bool,
}
```

### Conical Emission

Particles are spawned within a cone centered on the exhaust direction (opposite to `thrust_direction`). The spawn direction is computed by rotating the exhaust vector by a random angle within `cone_half_angle`:

```rust
fn sample_cone_direction(exhaust_dir: Vec3, half_angle: f32, rng: &mut impl Rng) -> Vec3 {
    // Generate a random direction within the cone.
    let cos_angle = rng.gen_range(half_angle.cos()..1.0);
    let sin_angle = (1.0 - cos_angle * cos_angle).sqrt();
    let phi = rng.gen_range(0.0..std::f32::consts::TAU);

    // Build a tangent frame from exhaust_dir.
    let (tangent, bitangent) = exhaust_dir.any_orthonormal_pair();

    exhaust_dir * cos_angle
        + tangent * sin_angle * phi.cos()
        + bitangent * sin_angle * phi.sin()
}
```

### Factory Functions

#### Standard Exhaust

```rust
/// Standard engine exhaust: blue-white core fading to transparent.
pub fn engine_exhaust(config: &EngineExhaustConfig) -> ParticleEmitter {
    let exhaust_dir = -config.thrust_direction.normalize();
    let base_speed = 8.0;
    let speed_scaled = base_speed * config.throttle.max(0.05); // minimum idle exhaust

    let base_rate = 60.0;
    let rate_scaled = base_rate * config.throttle.max(0.1);

    let base_lifetime = if config.zero_gravity { (0.8, 2.0) } else { (0.2, 0.6) };

    let brightness = config.throttle;

    ParticleEmitter {
        emission_rate: rate_scaled,
        lifetime_range: base_lifetime,
        velocity_range: (
            exhaust_dir * speed_scaled * 0.8,
            exhaust_dir * speed_scaled * 1.2,
        ),
        color_over_lifetime: ColorGradient {
            stops: vec![
                (0.0, Vec4::new(0.8, 0.9, 1.0, brightness)),          // blue-white core
                (0.3, Vec4::new(0.5, 0.7, 1.0, brightness * 0.7)),    // blue
                (0.7, Vec4::new(0.3, 0.4, 0.8, brightness * 0.3)),    // dim blue
                (1.0, Vec4::new(0.1, 0.1, 0.3, 0.0)),                 // transparent
            ],
        },
        size_over_lifetime: Curve {
            keyframes: vec![
                (0.0, 0.03),
                (0.3, 0.06 * config.throttle.max(0.2)),
                (1.0, 0.1),
            ],
        },
        gravity_influence: if config.zero_gravity { 0.0 } else { 0.05 },
        max_particles: 500,
        blend_mode: ParticleBlendMode::Additive,
        enabled: config.throttle > 0.01,
        emission_accumulator: 0.0,
    }
}
```

#### Afterburner Exhaust

```rust
/// Afterburner mode: larger, brighter, more particles. Stacks on top of standard exhaust.
pub fn afterburner_exhaust(config: &EngineExhaustConfig) -> ParticleEmitter {
    let exhaust_dir = -config.thrust_direction.normalize();
    let speed = 12.0;

    let base_lifetime = if config.zero_gravity { (1.0, 2.5) } else { (0.3, 0.8) };

    ParticleEmitter {
        emission_rate: 120.0,
        lifetime_range: base_lifetime,
        velocity_range: (
            exhaust_dir * speed * 0.9,
            exhaust_dir * speed * 1.3,
        ),
        color_over_lifetime: ColorGradient {
            stops: vec![
                (0.0, Vec4::new(1.0, 1.0, 1.0, 1.0)),       // white-hot core
                (0.2, Vec4::new(0.9, 0.95, 1.0, 0.9)),      // near-white blue
                (0.5, Vec4::new(0.6, 0.8, 1.0, 0.6)),       // bright blue
                (1.0, Vec4::new(0.2, 0.3, 0.8, 0.0)),       // fading blue
            ],
        },
        size_over_lifetime: Curve {
            keyframes: vec![(0.0, 0.05), (0.3, 0.12), (1.0, 0.15)],
        },
        gravity_influence: if config.zero_gravity { 0.0 } else { 0.02 },
        max_particles: 800,
        blend_mode: ParticleBlendMode::Additive,
        enabled: config.afterburner,
        emission_accumulator: 0.0,
    }
}
```

### Throttle Scaling System

The `ExhaustThrottleSystem` runs each frame and updates the emitter parameters based on the current throttle value:

```rust
pub fn update_exhaust_for_throttle(
    emitter: &mut ParticleEmitter,
    config: &EngineExhaustConfig,
) {
    // Scale emission rate proportionally to throttle.
    let base_rate = 60.0;
    emitter.emission_rate = base_rate * config.throttle.max(0.1);

    // Scale velocity magnitude.
    let base_speed = 8.0;
    let speed = base_speed * config.throttle.max(0.05);
    let exhaust_dir = -config.thrust_direction.normalize();
    emitter.velocity_range = (
        exhaust_dir * speed * 0.8,
        exhaust_dir * speed * 1.2,
    );

    // Scale color brightness.
    let brightness = config.throttle;
    emitter.color_over_lifetime.stops[0].1.w = brightness;

    // Disable at zero throttle.
    emitter.enabled = config.throttle > 0.01;
}
```

### Multiple Nozzles

Ships with multiple engines (e.g., two wing-mounted thrusters) simply have multiple entities, each with its own `ParticleEmitter` and `EngineExhaustConfig`. The nozzle position is derived from the ship's transform plus a local offset per engine:

```rust
pub fn nozzle_world_position(ship_transform: &Transform, local_offset: Vec3) -> Vec3 {
    ship_transform.position + ship_transform.rotation * local_offset
}
```

## Outcome

A `presets::exhaust` module providing `engine_exhaust()` and `afterburner_exhaust()` factory functions that return throttle-responsive, direction-aware `ParticleEmitter` components. Exhaust emits conically from the nozzle position in the direction opposite to thrust. Intensity scales linearly with throttle. Zero-gravity environments extend particle lifetime for persistent trails. Afterburner mode produces larger, brighter, more numerous particles. Multiple nozzles are handled by multiple emitter entities. All exhaust particles render through the standard additive-blend GPU particle pipeline.

## Demo Integration

**Demo crate:** `nebula-demo`

The spaceship emits a blue-white exhaust trail from its engines. The trail fades behind the ship as it moves.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `glam` | `0.29` | Vec3 for nozzle positions, thrust direction, cone sampling |
| `rand` | `0.9` | Random cone angle and phi sampling for conical emission |

No new external dependencies beyond what `nebula-particles` already requires. Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> EngineExhaustConfig {
        EngineExhaustConfig {
            nozzle_position: Vec3::new(0.0, 0.0, -5.0),
            thrust_direction: Vec3::new(0.0, 0.0, 1.0), // ship flies forward (+Z)
            cone_half_angle: 0.15, // ~8.6 degrees
            throttle: 1.0,
            afterburner: false,
            zero_gravity: false,
        }
    }

    #[test]
    fn test_exhaust_emits_from_nozzle_position() {
        let config = default_config();
        let emitter = engine_exhaust(&config);
        // Emitter should be active at full throttle.
        assert!(emitter.enabled);
        // The emitter will be attached to an entity at nozzle_position;
        // verify the configuration is non-trivial.
        assert!(emitter.emission_rate > 0.0);
    }

    #[test]
    fn test_exhaust_direction_matches_thrust() {
        let config = default_config();
        let emitter = engine_exhaust(&config);
        // Thrust direction is +Z, so exhaust direction should be -Z.
        // Velocity range should have negative Z components.
        assert!(
            emitter.velocity_range.0.z < 0.0,
            "exhaust min velocity z should be negative (opposite to thrust), got {}",
            emitter.velocity_range.0.z
        );
        assert!(
            emitter.velocity_range.1.z < 0.0,
            "exhaust max velocity z should be negative (opposite to thrust), got {}",
            emitter.velocity_range.1.z
        );
    }

    #[test]
    fn test_intensity_scales_with_throttle() {
        let full = {
            let mut c = default_config();
            c.throttle = 1.0;
            engine_exhaust(&c)
        };
        let half = {
            let mut c = default_config();
            c.throttle = 0.5;
            engine_exhaust(&c)
        };
        let idle = {
            let mut c = default_config();
            c.throttle = 0.1;
            engine_exhaust(&c)
        };

        assert!(
            full.emission_rate > half.emission_rate,
            "full throttle rate {} should exceed half throttle rate {}",
            full.emission_rate,
            half.emission_rate
        );
        assert!(
            half.emission_rate > idle.emission_rate,
            "half throttle rate {} should exceed idle rate {}",
            half.emission_rate,
            idle.emission_rate
        );
    }

    #[test]
    fn test_zero_throttle_disables_emitter() {
        let mut config = default_config();
        config.throttle = 0.0;
        let emitter = engine_exhaust(&config);
        assert!(!emitter.enabled, "zero throttle should disable the emitter");
    }

    #[test]
    fn test_zero_g_particles_last_longer() {
        let atmo = {
            let mut c = default_config();
            c.zero_gravity = false;
            engine_exhaust(&c)
        };
        let zero_g = {
            let mut c = default_config();
            c.zero_gravity = true;
            engine_exhaust(&c)
        };

        assert!(
            zero_g.lifetime_range.1 > atmo.lifetime_range.1,
            "zero-g max lifetime {} should exceed atmospheric {}",
            zero_g.lifetime_range.1,
            atmo.lifetime_range.1
        );
    }

    #[test]
    fn test_afterburner_increases_effect() {
        let config = default_config();
        let standard = engine_exhaust(&config);

        let mut ab_config = default_config();
        ab_config.afterburner = true;
        let afterburner = afterburner_exhaust(&ab_config);

        // Afterburner should have higher emission rate.
        assert!(
            afterburner.emission_rate > standard.emission_rate,
            "afterburner rate {} should exceed standard rate {}",
            afterburner.emission_rate,
            standard.emission_rate
        );

        // Afterburner should have larger particles.
        let ab_max_size = afterburner.size_over_lifetime.keyframes
            .iter()
            .map(|(_, s)| *s)
            .fold(0.0_f32, f32::max);
        let std_max_size = standard.size_over_lifetime.keyframes
            .iter()
            .map(|(_, s)| *s)
            .fold(0.0_f32, f32::max);
        assert!(
            ab_max_size > std_max_size,
            "afterburner max size {} should exceed standard max size {}",
            ab_max_size,
            std_max_size
        );
    }

    #[test]
    fn test_afterburner_disabled_when_not_engaged() {
        let mut config = default_config();
        config.afterburner = false;
        let emitter = afterburner_exhaust(&config);
        assert!(!emitter.enabled, "afterburner emitter should be disabled when not engaged");
    }

    #[test]
    fn test_cone_sampling_stays_within_half_angle() {
        let exhaust_dir = Vec3::new(0.0, 0.0, -1.0);
        let half_angle: f32 = 0.2; // ~11.5 degrees
        let mut rng = rand::rng();

        for _ in 0..1000 {
            let dir = sample_cone_direction(exhaust_dir, half_angle, &mut rng);
            let angle = exhaust_dir.dot(dir.normalize()).acos();
            assert!(
                angle <= half_angle + 0.001, // small epsilon for float precision
                "sampled angle {} exceeds half_angle {}",
                angle,
                half_angle
            );
        }
    }

    #[test]
    fn test_exhaust_uses_additive_blending() {
        let config = default_config();
        let emitter = engine_exhaust(&config);
        assert_eq!(emitter.blend_mode, ParticleBlendMode::Additive);
    }
}
```
