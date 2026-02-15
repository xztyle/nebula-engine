# Distant Planet Impostors

## Problem

A solar system contains multiple planets, and from any given planet's surface or orbit, the other planets should be visible in the sky as small points of light or, when close enough, as tiny discs showing a crescent phase based on the sun's direction. Rendering the full voxel geometry of a distant planet is absurdly expensive and unnecessary -- a planet millions of kilometers away occupies only a few pixels. The engine needs an impostor system that renders distant planets as billboards with a simple lit-sphere shader that computes the crescent shape from the sun angle. The impostors must update their positions over time as planets orbit, and their angular size and brightness must follow real optical scaling laws.

## Solution

Implement a `PlanetImpostorRenderer` in the `nebula-space` crate that manages and renders distant planets as camera-facing billboards with a procedural crescent shader. Each impostor is driven by orbital mechanics data (simplified Keplerian elements) that update the planet's position over time. The impostor system sits between the skybox and the main scene geometry in the render order.

### Planet Impostor Data

```rust
/// Data describing a distant planet for impostor rendering.
#[derive(Clone, Debug)]
pub struct DistantPlanet {
    /// Unique identifier for this planet.
    pub id: u64,
    /// Position in world space (128-bit coordinates).
    pub position: [i128; 3],
    /// Physical radius in engine units (meters).
    pub radius: f64,
    /// Albedo (reflectivity) in [0, 1]. Determines apparent brightness.
    pub albedo: f32,
    /// Surface color tint (linear RGB). Earth-like blue, Mars-like red, etc.
    pub color: [f32; 3],
    /// Whether the planet has an atmosphere (adds a thin colored rim).
    pub has_atmosphere: bool,
    /// Atmosphere color (linear RGB), only used if `has_atmosphere` is true.
    pub atmosphere_color: [f32; 3],
}

/// Orbital elements for simplified planetary motion.
#[derive(Clone, Debug)]
pub struct OrbitalElements {
    /// Semi-major axis in engine units.
    pub semi_major_axis: f64,
    /// Eccentricity [0, 1). 0 = circular orbit.
    pub eccentricity: f64,
    /// Inclination in radians relative to the ecliptic plane.
    pub inclination: f64,
    /// Longitude of ascending node in radians.
    pub longitude_ascending: f64,
    /// Argument of periapsis in radians.
    pub argument_periapsis: f64,
    /// Mean anomaly at epoch in radians.
    pub mean_anomaly_epoch: f64,
    /// Orbital period in seconds.
    pub orbital_period: f64,
}

impl OrbitalElements {
    /// Compute the planet's position at a given time using simplified Keplerian mechanics.
    /// Returns the position as a 3D offset from the star in engine units.
    pub fn position_at_time(&self, time_seconds: f64) -> glam::DVec3 {
        // Mean anomaly at current time.
        let mean_anomaly = self.mean_anomaly_epoch
            + std::f64::consts::TAU * (time_seconds / self.orbital_period);

        // Solve Kepler's equation: E - e*sin(E) = M (Newton-Raphson iteration).
        let mut eccentric_anomaly = mean_anomaly;
        for _ in 0..10 {
            let delta = eccentric_anomaly
                - self.eccentricity * eccentric_anomaly.sin()
                - mean_anomaly;
            let derivative = 1.0 - self.eccentricity * eccentric_anomaly.cos();
            eccentric_anomaly -= delta / derivative;
        }

        // True anomaly from eccentric anomaly.
        let true_anomaly = 2.0
            * ((1.0 + self.eccentricity).sqrt() * (eccentric_anomaly / 2.0).sin())
                .atan2((1.0 - self.eccentricity).sqrt() * (eccentric_anomaly / 2.0).cos());

        // Radius from the focus.
        let r = self.semi_major_axis * (1.0 - self.eccentricity * eccentric_anomaly.cos());

        // Position in the orbital plane.
        let x_orbital = r * true_anomaly.cos();
        let y_orbital = r * true_anomaly.sin();

        // Rotate into 3D space using orbital elements.
        let cos_o = self.longitude_ascending.cos();
        let sin_o = self.longitude_ascending.sin();
        let cos_i = self.inclination.cos();
        let sin_i = self.inclination.sin();
        let cos_w = self.argument_periapsis.cos();
        let sin_w = self.argument_periapsis.sin();

        let x = x_orbital * (cos_o * cos_w - sin_o * sin_w * cos_i)
            - y_orbital * (cos_o * sin_w + sin_o * cos_w * cos_i);
        let y = x_orbital * (sin_o * cos_w + cos_o * sin_w * cos_i)
            - y_orbital * (sin_o * sin_w - cos_o * cos_w * cos_i);
        let z = x_orbital * (sin_w * sin_i) + y_orbital * (cos_w * sin_i);

        glam::DVec3::new(x, y, z)
    }
}
```

### Angular Size and Visibility

```rust
impl DistantPlanet {
    /// Compute the angular diameter in radians as seen from a given distance.
    pub fn angular_diameter(&self, distance: f64) -> f64 {
        if distance <= 0.0 {
            return std::f64::consts::PI; // degenerate: full hemisphere
        }
        2.0 * (self.radius / distance).atan()
    }

    /// Compute the apparent brightness as a fraction of full illumination.
    /// Uses the phase angle (angle between sun and observer as seen from the planet).
    /// Returns a value in [0, 1].
    pub fn phase_brightness(&self, phase_angle: f64) -> f32 {
        // Lambertian phase function: (1 + cos(phase)) / 2.
        let lambert = ((1.0 + phase_angle.cos()) / 2.0) as f32;
        lambert * self.albedo
    }

    /// Compute the phase angle given the sun direction and observer direction
    /// (both as unit vectors from the planet's position).
    pub fn compute_phase_angle(
        to_sun: glam::DVec3,
        to_observer: glam::DVec3,
    ) -> f64 {
        let cos_phase = to_sun.normalize().dot(to_observer.normalize());
        cos_phase.clamp(-1.0, 1.0).acos()
    }

    /// Determine if this planet should be rendered as an impostor at a given distance.
    /// If the angular diameter is above a threshold, the planet should be rendered
    /// with full geometry instead.
    pub fn should_render_as_impostor(&self, distance: f64) -> bool {
        // If angular diameter is smaller than ~0.5 degrees, use impostor.
        self.angular_diameter(distance) < 0.0087 // ~0.5 degrees in radians
    }
}
```

### Crescent Shader

The impostor billboard uses a procedural shader that renders a lit sphere with correct phase angle:

```wgsl
struct ImpostorUniforms {
    planet_color: vec3<f32>,
    brightness: f32,
    sun_direction_local: vec3<f32>, // Sun direction in billboard-local space.
    angular_radius: f32,
    has_atmosphere: u32,
    atmosphere_color: vec3<f32>,
};

@group(0) @binding(0)
var<uniform> impostor: ImpostorUniforms;

@fragment
fn fs_planet_impostor(in: ImpostorVertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv; // [-1, 1] centered on the billboard.
    let dist_sq = dot(uv, uv);

    // Discard fragments outside the planet disk.
    if dist_sq > 1.0 {
        discard;
    }

    // Reconstruct normal on a unit sphere.
    let z = sqrt(1.0 - dist_sq);
    let normal = vec3<f32>(uv.x, uv.y, z);

    // Lambertian shading: dot(normal, sun_direction).
    let ndotl = max(dot(normal, impostor.sun_direction_local), 0.0);

    // The dark side of the planet.
    let lit_color = impostor.planet_color * ndotl * impostor.brightness;

    // Atmospheric rim glow on the lit limb.
    var atmo = vec3<f32>(0.0);
    if impostor.has_atmosphere != 0u {
        let rim = 1.0 - z; // Stronger at edges.
        let rim_factor = pow(rim, 2.0) * max(ndotl + 0.2, 0.0);
        atmo = impostor.atmosphere_color * rim_factor * 0.5;
    }

    let final_color = lit_color + atmo;

    // Alpha: solid on the disk, transparent outside (already discarded).
    return vec4<f32>(final_color, 1.0);
}
```

### Impostor Renderer

```rust
pub struct PlanetImpostorRenderer {
    pipeline: wgpu::RenderPipeline,
    instance_buffer: wgpu::Buffer,
    uniform_buffer: wgpu::Buffer,
    max_impostors: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct ImpostorInstance {
    /// Screen-space position of the billboard center.
    position: [f32; 3],
    /// Billboard scale (angular radius mapped to screen units).
    scale: f32,
    /// Planet color.
    color: [f32; 3],
    /// Apparent brightness.
    brightness: f32,
    /// Sun direction in billboard-local space for crescent calculation.
    sun_dir_local: [f32; 3],
    /// Whether this planet has an atmosphere.
    has_atmosphere: u32,
    /// Atmosphere color.
    atmosphere_color: [f32; 3],
    _padding: f32,
}

impl PlanetImpostorRenderer {
    /// Update all impostor instances for the current frame.
    pub fn update(
        &self,
        queue: &wgpu::Queue,
        camera: &Camera,
        camera_world_pos: &[i128; 3],
        sun_world_pos: &[i128; 3],
        planets: &[(DistantPlanet, OrbitalElements)],
        time_seconds: f64,
    ) {
        let mut instances = Vec::new();

        for (planet, orbit) in planets {
            // Compute planet position from orbital elements.
            let orbital_offset = orbit.position_at_time(time_seconds);

            // Convert to relative position from camera (128-bit subtraction, then to f64).
            // This uses the engine's origin-rebasing strategy.
            let relative_pos = compute_relative_position(
                camera_world_pos,
                &planet.position,
                orbital_offset,
            );

            let distance = relative_pos.length();

            if !planet.should_render_as_impostor(distance) {
                continue; // Too close; will be rendered with full geometry.
            }

            let direction = (relative_pos / distance).as_vec3();
            let angular_radius = (planet.angular_diameter(distance) * 0.5) as f32;

            // Compute sun direction relative to the planet for crescent shading.
            let planet_to_sun = compute_sun_direction(
                &planet.position,
                sun_world_pos,
                orbital_offset,
            );

            // Transform sun direction into billboard-local space.
            let sun_dir_local = billboard_local_sun_dir(
                direction,
                planet_to_sun.as_vec3(),
                camera,
            );

            let phase_angle = DistantPlanet::compute_phase_angle(
                planet_to_sun,
                -relative_pos,
            );
            let brightness = planet.phase_brightness(phase_angle);

            instances.push(ImpostorInstance {
                position: direction.into(),
                scale: angular_radius,
                color: planet.color,
                brightness,
                sun_dir_local: sun_dir_local.into(),
                has_atmosphere: planet.has_atmosphere as u32,
                atmosphere_color: planet.atmosphere_color,
                _padding: 0.0,
            });
        }

        queue.write_buffer(
            &self.instance_buffer,
            0,
            bytemuck::cast_slice(&instances),
        );
    }
}
```

### Render Integration

The impostor renderer is invoked after the skybox pass and before the main scene geometry pass. Impostors are rendered with depth write disabled (they are infinitely distant visual indicators) and alpha blending enabled for smooth edges.

## Outcome

A `PlanetImpostorRenderer`, `DistantPlanet`, and `OrbitalElements` in `nebula-space` that render distant planets as lit-sphere billboards with correct crescent phase, atmospheric rim glow, and distance-based angular scaling. Orbital positions update over time using simplified Keplerian mechanics. Running `cargo test -p nebula-space` passes all impostor tests. The impostor system integrates with the skybox pipeline and transitions to full geometry rendering at close range.

## Demo Integration

**Demo crate:** `nebula-demo`

Distant planets appear as lit-sphere billboards with correct crescent phases and atmospheric rim glow. Their orbital positions update over time using Keplerian mechanics.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `wgpu` | `24.0` | Render pipeline, instanced billboard drawing |
| `bytemuck` | `1.21` | Instance and uniform buffer serialization |
| `glam` | `0.29` | Vec3/DVec3 for positions, directions, and orbital math |

The impostor renderer lives in the `nebula-space` crate. Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;

    #[test]
    fn test_distant_planet_visible_as_dot() {
        let planet = DistantPlanet {
            id: 1,
            position: [0; 3],
            radius: 6_371_000.0, // Earth-like radius in meters
            albedo: 0.3,
            color: [0.3, 0.5, 0.8],
            has_atmosphere: true,
            atmosphere_color: [0.4, 0.6, 1.0],
        };

        // At 1 AU distance, Earth would subtend a tiny angle.
        let distance = 149_597_870_700.0; // 1 AU in meters
        let angular = planet.angular_diameter(distance);

        assert!(
            angular > 0.0,
            "Planet should have positive angular diameter"
        );
        assert!(
            angular < 0.001, // less than ~0.06 degrees
            "Planet at 1 AU should be a tiny dot, angular diameter = {angular} rad"
        );
    }

    #[test]
    fn test_crescent_shape_matches_sun_direction() {
        // When the sun is to the right (+X), the right side of the planet should be lit.
        let sun_dir = DVec3::X;
        let observer_dir = DVec3::NEG_Z;

        let phase_angle = DistantPlanet::compute_phase_angle(sun_dir, observer_dir);

        // Sun at right angle to observer: phase angle should be ~90 degrees.
        assert!(
            (phase_angle - std::f64::consts::FRAC_PI_2).abs() < 0.01,
            "Phase angle should be ~90 deg when sun is perpendicular, got {} rad",
            phase_angle
        );

        // When sun is behind the observer (full illumination): phase angle ~0.
        let phase_full = DistantPlanet::compute_phase_angle(DVec3::NEG_Z, DVec3::NEG_Z);
        assert!(
            phase_full < 0.01,
            "Phase angle should be ~0 when sun is behind observer, got {phase_full}"
        );

        // When sun is behind the planet (new moon): phase angle ~PI.
        let phase_new = DistantPlanet::compute_phase_angle(DVec3::Z, DVec3::NEG_Z);
        assert!(
            (phase_new - std::f64::consts::PI).abs() < 0.01,
            "Phase angle should be ~PI for new phase, got {phase_new}"
        );
    }

    #[test]
    fn test_planet_angular_size_decreases_with_distance() {
        let planet = DistantPlanet {
            id: 1,
            position: [0; 3],
            radius: 6_371_000.0,
            albedo: 0.3,
            color: [0.3, 0.5, 0.8],
            has_atmosphere: false,
            atmosphere_color: [0.0; 3],
        };

        let angular_near = planet.angular_diameter(1_000_000.0);
        let angular_far = planet.angular_diameter(10_000_000.0);

        assert!(
            angular_near > angular_far,
            "Closer planet should have larger angular diameter: {angular_near} vs {angular_far}"
        );

        // At 10x distance, angular diameter should be roughly 10x smaller.
        let ratio = angular_near / angular_far;
        assert!(
            (ratio - 10.0).abs() < 0.5,
            "Angular diameter ratio should be ~10 for 10x distance, got {ratio}"
        );
    }

    #[test]
    fn test_multiple_planets_render_simultaneously() {
        let planets: Vec<DistantPlanet> = (0..8)
            .map(|i| DistantPlanet {
                id: i as u64,
                position: [0; 3],
                radius: 5_000_000.0 + i as f64 * 1_000_000.0,
                albedo: 0.3,
                color: [0.5, 0.5, 0.5],
                has_atmosphere: i % 2 == 0,
                atmosphere_color: [0.3, 0.5, 0.8],
            })
            .collect();

        // All 8 planets should be representable as impostors.
        let distance = 500_000_000_000.0;
        for planet in &planets {
            assert!(
                planet.should_render_as_impostor(distance),
                "Planet {} should render as impostor at distance {distance}",
                planet.id
            );
        }
    }

    #[test]
    fn test_planet_positions_update_over_time() {
        let orbit = OrbitalElements {
            semi_major_axis: 149_597_870_700.0,
            eccentricity: 0.0167,
            inclination: 0.0,
            longitude_ascending: 0.0,
            argument_periapsis: 0.0,
            mean_anomaly_epoch: 0.0,
            orbital_period: 365.25 * 24.0 * 3600.0, // ~1 year in seconds
        };

        let pos_t0 = orbit.position_at_time(0.0);
        let pos_t1 = orbit.position_at_time(orbit.orbital_period * 0.25); // quarter orbit

        let distance_moved = (pos_t1 - pos_t0).length();
        assert!(
            distance_moved > 1e9,
            "Planet should move significantly over a quarter orbit: moved {distance_moved}"
        );

        // After a full period, the planet should return near its starting position.
        let pos_full = orbit.position_at_time(orbit.orbital_period);
        let return_distance = (pos_full - pos_t0).length();
        assert!(
            return_distance < orbit.semi_major_axis * 0.001,
            "Planet should return near start after full orbit: distance = {return_distance}"
        );
    }

    #[test]
    fn test_orbital_circular_orbit_is_constant_radius() {
        let orbit = OrbitalElements {
            semi_major_axis: 100_000.0,
            eccentricity: 0.0,
            inclination: 0.0,
            longitude_ascending: 0.0,
            argument_periapsis: 0.0,
            mean_anomaly_epoch: 0.0,
            orbital_period: 1000.0,
        };

        // For a circular orbit, the radius should be constant at all times.
        for i in 0..20 {
            let t = (i as f64 / 20.0) * orbit.orbital_period;
            let pos = orbit.position_at_time(t);
            let r = pos.length();
            assert!(
                (r - orbit.semi_major_axis).abs() < orbit.semi_major_axis * 0.001,
                "Circular orbit radius at t={t} should be ~{}, got {r}",
                orbit.semi_major_axis
            );
        }
    }

    #[test]
    fn test_phase_brightness_full_illumination() {
        let planet = DistantPlanet {
            id: 1,
            position: [0; 3],
            radius: 6_371_000.0,
            albedo: 0.5,
            color: [1.0; 3],
            has_atmosphere: false,
            atmosphere_color: [0.0; 3],
        };

        // Phase angle 0 = full illumination.
        let brightness = planet.phase_brightness(0.0);
        assert!(
            (brightness - 0.5).abs() < 1e-6,
            "Full illumination brightness should equal albedo (0.5), got {brightness}"
        );

        // Phase angle PI = no illumination.
        let dark_brightness = planet.phase_brightness(std::f64::consts::PI);
        assert!(
            dark_brightness < 0.01,
            "Dark side brightness should be near zero, got {dark_brightness}"
        );
    }

    #[test]
    fn test_impostor_threshold_transition() {
        let planet = DistantPlanet {
            id: 1,
            position: [0; 3],
            radius: 6_371_000.0,
            albedo: 0.3,
            color: [0.5; 3],
            has_atmosphere: false,
            atmosphere_color: [0.0; 3],
        };

        // Very far: should be impostor.
        assert!(planet.should_render_as_impostor(1e12));
        // Very close: should NOT be impostor (render full geometry).
        assert!(!planet.should_render_as_impostor(planet.radius * 10.0));
    }

    #[test]
    fn test_instance_data_alignment() {
        // ImpostorInstance must be properly aligned for GPU instancing.
        let size = std::mem::size_of::<ImpostorInstance>();
        assert_eq!(
            size % 16,
            0,
            "ImpostorInstance size ({size} bytes) must be 16-byte aligned"
        );
    }
}
```
