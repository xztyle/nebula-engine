/// 1 meter = 1,000 millimeters
pub const UNITS_PER_METER: i128 = 1_000;

/// 1 kilometer = 1,000,000 millimeters
pub const UNITS_PER_KILOMETER: i128 = 1_000_000;

/// 1 Astronomical Unit = 149,597,870,700 meters = 149,597,870,700,000 mm
/// (IAU 2012 exact definition)
pub const UNITS_PER_AU: i128 = 149_597_870_700_000;

/// 1 light-year = 9,460,730,472,580,800 meters
/// = 9,460,730,472,580,800,000 mm
/// (IAU definition: exactly 9,460,730,472,580,800 m)
pub const UNITS_PER_LIGHT_YEAR: i128 = 9_460_730_472_580_800_000;

/// 1 parsec ≈ 3.26156 light-years
/// = 30,856,775,814,913,673,000 mm
pub const UNITS_PER_PARSEC: i128 = 30_856_775_814_913_673_000;

/// 1 centimeter = 10 mm
pub const UNITS_PER_CENTIMETER: i128 = 10;

/// 1 inch = 25.4 mm (exact by definition), rounded to 25 for integer
pub const UNITS_PER_INCH: i128 = 25;

/// Earth radius (mean) ≈ 6,371 km = 6,371,000,000 mm
pub const EARTH_RADIUS_UNITS: i128 = 6_371_000_000;

/// Solar radius ≈ 695,700 km = 695,700,000,000 mm
pub const SOLAR_RADIUS_UNITS: i128 = 695_700_000_000;

/// Convert meters (f64) to internal units (i128).
/// Rounds to nearest millimeter.
pub fn meters_to_units(meters: f64) -> i128 {
    (meters * UNITS_PER_METER as f64).round() as i128
}

/// Convert internal units (i128) to meters (f64).
pub fn units_to_meters(units: i128) -> f64 {
    units as f64 / UNITS_PER_METER as f64
}

/// Convert kilometers (f64) to internal units (i128).
pub fn kilometers_to_units(km: f64) -> i128 {
    (km * UNITS_PER_KILOMETER as f64).round() as i128
}

/// Convert internal units (i128) to kilometers (f64).
pub fn units_to_kilometers(units: i128) -> f64 {
    units as f64 / UNITS_PER_KILOMETER as f64
}

/// Convert astronomical units (f64) to internal units (i128).
pub fn au_to_units(au: f64) -> i128 {
    (au * UNITS_PER_AU as f64).round() as i128
}

/// Convert internal units (i128) to astronomical units (f64).
pub fn units_to_au(units: i128) -> f64 {
    units as f64 / UNITS_PER_AU as f64
}

/// Convert light-years (f64) to internal units (i128).
pub fn light_years_to_units(ly: f64) -> i128 {
    (ly * UNITS_PER_LIGHT_YEAR as f64).round() as i128
}

/// Convert internal units (i128) to light-years (f64).
pub fn units_to_light_years(units: i128) -> f64 {
    units as f64 / UNITS_PER_LIGHT_YEAR as f64
}

/// Format a distance in internal units as a human-readable string,
/// automatically choosing the most appropriate unit.
///
/// Examples:
/// - 500 -> "500 mm"
/// - 1_500 -> "1.500 m"
/// - 5_000_000 -> "5.000 km"
/// - 200_000_000_000_000 -> "1.337 AU"
/// - 10_000_000_000_000_000_000 -> "1.057 ly"
pub fn format_distance(units: i128) -> String {
    let abs = units.unsigned_abs();
    let sign = if units < 0 { "-" } else { "" };

    if abs >= UNITS_PER_LIGHT_YEAR as u128 {
        format!("{}{:.3} ly", sign, units_to_light_years(units.abs()))
    } else if abs >= UNITS_PER_AU as u128 {
        format!("{}{:.3} AU", sign, units_to_au(units.abs()))
    } else if abs >= UNITS_PER_KILOMETER as u128 {
        format!("{}{:.3} km", sign, units_to_kilometers(units.abs()))
    } else if abs >= UNITS_PER_METER as u128 {
        format!("{}{:.3} m", sign, units_to_meters(units.abs()))
    } else {
        format!("{}{} mm", sign, units.abs())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_one_meter_is_1000_units() {
        assert_eq!(UNITS_PER_METER, 1000);
        assert_eq!(meters_to_units(1.0), 1000);
    }

    #[test]
    fn test_one_km_is_1m_units() {
        assert_eq!(UNITS_PER_KILOMETER, 1_000_000);
        assert_eq!(kilometers_to_units(1.0), 1_000_000);
    }

    #[test]
    fn test_roundtrip_meters() {
        let meters = 42.567;
        let units = meters_to_units(meters);
        let back = units_to_meters(units);
        assert!((back - meters).abs() < 0.001); // within 1mm
    }

    #[test]
    fn test_roundtrip_kilometers() {
        let km = 123.456;
        let units = kilometers_to_units(km);
        let back = units_to_kilometers(units);
        assert!((back - km).abs() < 0.000001); // within 1mm = 10⁻⁶ km
    }

    #[test]
    fn test_au_constant_astronomically_correct() {
        // 1 AU = 149,597,870,700 meters (IAU 2012)
        assert_eq!(UNITS_PER_AU, 149_597_870_700 * UNITS_PER_METER);
    }

    #[test]
    fn test_light_year_constant_astronomically_correct() {
        // 1 ly = 9,460,730,472,580,800 meters (IAU)
        assert_eq!(
            UNITS_PER_LIGHT_YEAR,
            9_460_730_472_580_800 * UNITS_PER_METER
        );
    }

    #[test]
    fn test_au_to_units_and_back() {
        let au = 1.0;
        let units = au_to_units(au);
        let back = units_to_au(units);
        assert!((back - au).abs() < 1e-10);
    }

    #[test]
    fn test_light_year_to_units_and_back() {
        let ly = 1.0;
        let units = light_years_to_units(ly);
        let back = units_to_light_years(units);
        assert!((back - ly).abs() < 1e-6);
    }

    #[test]
    fn test_au_in_light_years() {
        // 1 AU ≈ 1.581×10⁻⁵ light-years
        let au_in_ly = units_to_light_years(UNITS_PER_AU);
        assert!((au_in_ly - 1.5812507e-5).abs() < 1e-8);
    }

    #[test]
    fn test_format_distance_mm() {
        assert_eq!(format_distance(500), "500 mm");
    }

    #[test]
    fn test_format_distance_meters() {
        let s = format_distance(1_500);
        assert!(s.contains("m"));
        assert!(s.contains("1.500"));
    }

    #[test]
    fn test_format_distance_km() {
        let s = format_distance(5_000_000);
        assert!(s.contains("km"));
    }

    #[test]
    fn test_earth_radius_constant() {
        // Earth mean radius ≈ 6,371 km
        let km = units_to_kilometers(EARTH_RADIUS_UNITS);
        assert!((km - 6371.0).abs() < 0.001);
    }

    #[test]
    fn test_solar_radius_constant() {
        // Solar radius ≈ 695,700 km
        let km = units_to_kilometers(SOLAR_RADIUS_UNITS);
        assert!((km - 695_700.0).abs() < 0.001);
    }

    #[test]
    fn test_format_distance_au() {
        // Test AU formatting - about 1 AU distance
        let s = format_distance(UNITS_PER_AU);
        assert!(s.contains("AU"));
        assert!(s.contains("1.000"));
    }

    #[test]
    fn test_format_distance_ly() {
        // Test light-year formatting - about 1 ly distance
        let s = format_distance(UNITS_PER_LIGHT_YEAR);
        assert!(s.contains("ly"));
        assert!(s.contains("1.000"));
    }

    #[test]
    fn test_format_distance_negative() {
        let s = format_distance(-1500);
        assert!(s.starts_with("-"));
        assert!(s.contains("m"));
    }

    #[test]
    fn test_centimeter_constant() {
        assert_eq!(UNITS_PER_CENTIMETER, 10);
    }

    #[test]
    fn test_inch_constant() {
        assert_eq!(UNITS_PER_INCH, 25);
    }

    #[test]
    fn test_parsec_constant() {
        // 1 parsec ≈ 3.26156 light-years
        let parsec_in_ly = units_to_light_years(UNITS_PER_PARSEC);
        assert!((parsec_in_ly - 3.26156).abs() < 0.001);
    }
}
