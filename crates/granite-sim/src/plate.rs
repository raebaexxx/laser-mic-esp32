//! Simply-supported rectangular plate modal model.

use std::f64::consts::PI;

use crate::material::Material;

/// Rectangular slab dimensions.
#[derive(Clone, Copy, Debug)]
pub struct PlateGeometry {
    pub length_m: f64,
    pub width_m: f64,
    pub thickness_m: f64,
}

impl PlateGeometry {
    pub const fn new(length_m: f64, width_m: f64, thickness_m: f64) -> Self {
        Self {
            length_m,
            width_m,
            thickness_m,
        }
    }

    pub fn nominal_granite() -> Self {
        Self::new(0.600, 0.400, 0.020)
    }

    pub fn volume_m3(&self) -> f64 {
        self.length_m * self.width_m * self.thickness_m
    }
}

/// A point on the slab surface, in metres from one corner.
#[derive(Clone, Copy, Debug)]
pub struct Point2 {
    pub x_m: f64,
    pub y_m: f64,
}

impl Point2 {
    pub const fn new(x_m: f64, y_m: f64) -> Self {
        Self { x_m, y_m }
    }
}

/// One bending mode of the plate.
#[derive(Clone, Debug)]
pub struct PlateMode {
    pub m: u32,
    pub n: u32,
    pub frequency_hz: f64,
    pub damping_ratio: f64,
    /// Product of the mode shape at impact and sensor positions, signed.
    pub coupling: f64,
    /// Relative velocity response amplitude for an impulse.
    pub amplitude: f64,
}

/// Modal model of a simply-supported rectangular plate.
#[derive(Clone, Debug)]
pub struct PlateModel {
    pub geometry: PlateGeometry,
    pub material: Material,
    pub sensor: Point2,
    pub impact: Point2,
    pub modes: Vec<PlateMode>,
}

impl PlateModel {
    pub fn new(
        geometry: PlateGeometry,
        material: Material,
        sensor: Point2,
        impact: Point2,
        max_frequency_hz: f64,
    ) -> Self {
        let a = geometry.length_m.max(1e-6);
        let b = geometry.width_m.max(1e-6);
        let h = geometry.thickness_m.max(1e-6);
        let rigidity = material.flexural_rigidity(h);
        let wave_factor = (PI / 2.0) * (rigidity / (material.density_kg_m3 * h)).sqrt();
        let modal_mass = material.density_kg_m3 * h * a * b / 4.0;
        let max_frequency_hz = max_frequency_hz.clamp(100.0, 24_000.0);
        let max_order = 32u32;
        let mut modes = Vec::new();

        for m in 1..=max_order {
            let fundamental_m = wave_factor * (m as f64 / a).powi(2);
            if fundamental_m > max_frequency_hz {
                break;
            }
            for n in 1..=max_order {
                let frequency_hz = wave_factor * ((m as f64 / a).powi(2) + (n as f64 / b).powi(2));
                if frequency_hz > max_frequency_hz {
                    break;
                }
                let shape_impact =
                    (m as f64 * PI * impact.x_m / a).sin() * (n as f64 * PI * impact.y_m / b).sin();
                let shape_sensor =
                    (m as f64 * PI * sensor.x_m / a).sin() * (n as f64 * PI * sensor.y_m / b).sin();
                let coupling = shape_impact * shape_sensor;
                if coupling.abs() < 1e-4 {
                    continue;
                }
                let damping_ratio = material.damping_at(frequency_hz);
                // Velocity response to an impulse scales with omega and the
                // modal participation.  The absolute scale is normalised
                // after synthesis; only relative amplitudes matter.
                let amplitude = coupling * (2.0 * PI * frequency_hz) / modal_mass
                    * (frequency_hz / 1000.0).powf(-0.08);
                modes.push(PlateMode {
                    m,
                    n,
                    frequency_hz,
                    damping_ratio,
                    coupling,
                    amplitude,
                });
                if modes.len() >= 180 {
                    break;
                }
            }
        }
        modes.sort_by(|a, b| {
            a.frequency_hz
                .partial_cmp(&b.frequency_hz)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Self {
            geometry,
            material,
            sensor,
            impact,
            modes,
        }
    }

    pub fn modal_frequencies_hz(&self) -> Vec<f64> {
        self.modes.iter().map(|mode| mode.frequency_hz).collect()
    }

    pub fn modal_damping(&self) -> Vec<f64> {
        self.modes.iter().map(|mode| mode.damping_ratio).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::material::GRANITE;

    #[test]
    fn first_mode_is_in_expected_range() {
        let model = PlateModel::new(
            PlateGeometry::nominal_granite(),
            GRANITE,
            Point2::new(0.18, 0.14),
            Point2::new(0.33, 0.22),
            8000.0,
        );
        assert!(!model.modes.is_empty());
        let f1 = model.modes[0].frequency_hz;
        assert!(f1 > 250.0 && f1 < 500.0, "f1 {f1}");
        // Modes must be sorted.
        for pair in model.modes.windows(2) {
            assert!(pair[0].frequency_hz <= pair[1].frequency_hz);
        }
    }
}
