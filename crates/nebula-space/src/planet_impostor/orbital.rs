//! Simplified Keplerian orbital mechanics for planetary motion.

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
        let mean_anomaly =
            self.mean_anomaly_epoch + std::f64::consts::TAU * (time_seconds / self.orbital_period);

        // Solve Kepler's equation: E - e*sin(E) = M (Newton-Raphson).
        let mut e_anom = mean_anomaly;
        for _ in 0..10 {
            let delta = e_anom - self.eccentricity * e_anom.sin() - mean_anomaly;
            let derivative = 1.0 - self.eccentricity * e_anom.cos();
            e_anom -= delta / derivative;
        }

        // True anomaly from eccentric anomaly.
        let true_anomaly = 2.0
            * ((1.0 + self.eccentricity).sqrt() * (e_anom / 2.0).sin())
                .atan2((1.0 - self.eccentricity).sqrt() * (e_anom / 2.0).cos());

        // Radius from the focus.
        let r = self.semi_major_axis * (1.0 - self.eccentricity * e_anom.cos());

        // Position in the orbital plane.
        let x_orb = r * true_anomaly.cos();
        let y_orb = r * true_anomaly.sin();

        // Rotate into 3D space using orbital elements.
        let cos_o = self.longitude_ascending.cos();
        let sin_o = self.longitude_ascending.sin();
        let cos_i = self.inclination.cos();
        let sin_i = self.inclination.sin();
        let cos_w = self.argument_periapsis.cos();
        let sin_w = self.argument_periapsis.sin();

        let x = x_orb * (cos_o * cos_w - sin_o * sin_w * cos_i)
            - y_orb * (cos_o * sin_w + sin_o * cos_w * cos_i);
        let y = x_orb * (sin_o * cos_w + cos_o * sin_w * cos_i)
            - y_orb * (sin_o * sin_w - cos_o * cos_w * cos_i);
        let z = x_orb * (sin_w * sin_i) + y_orb * (cos_w * sin_i);

        glam::DVec3::new(x, y, z)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_planet_positions_update_over_time() {
        let orbit = OrbitalElements {
            semi_major_axis: 149_597_870_700.0,
            eccentricity: 0.0167,
            inclination: 0.0,
            longitude_ascending: 0.0,
            argument_periapsis: 0.0,
            mean_anomaly_epoch: 0.0,
            orbital_period: 365.25 * 24.0 * 3600.0,
        };

        let pos_t0 = orbit.position_at_time(0.0);
        let pos_quarter = orbit.position_at_time(orbit.orbital_period * 0.25);
        let distance_moved = (pos_quarter - pos_t0).length();
        assert!(distance_moved > 1e9, "moved {distance_moved}");

        let pos_full = orbit.position_at_time(orbit.orbital_period);
        let return_dist = (pos_full - pos_t0).length();
        assert!(
            return_dist < orbit.semi_major_axis * 0.001,
            "return dist = {return_dist}"
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

        for i in 0..20 {
            let t = (i as f64 / 20.0) * orbit.orbital_period;
            let pos = orbit.position_at_time(t);
            let r = pos.length();
            assert!(
                (r - orbit.semi_major_axis).abs() < orbit.semi_major_axis * 0.001,
                "at t={t}, r={r}"
            );
        }
    }
}
