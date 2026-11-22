//! Defect models that perturb the plate's modal parameters.
//!
//! These are phenomenological, not finite-element simulations.  Each defect
//! reproduces the qualitative signature reported in impact-modal testing:
//!
//! * a crack lowers stiffness, raises damping and can split a mode;
//! * a delamination adds a low-frequency "breathing" mode and splits modes;
//! * a void lowers stiffness and raises damping without strong splitting;
//! * a thickness variation shifts all modes while leaving damping similar.

use crate::material::Material;
use crate::plate::{PlateGeometry, PlateMode, Point2};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DefectKind {
    Healthy,
    Crack,
    Delamination,
    Void,
    ThicknessVariation,
}

impl DefectKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Crack => "crack",
            Self::Delamination => "delamination",
            Self::Void => "void",
            Self::ThicknessVariation => "thickness variation",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::Healthy => "No defect; baseline slab.",
            Self::Crack => {
                "A surface-breaking crack reduces local bending stiffness, increases damping and splits closely spaced modes."
            }
            Self::Delamination => {
                "A near-surface delamination adds a low-frequency layer mode and breaks modal symmetry."
            }
            Self::Void => {
                "An internal void reduces effective stiffness and increases modal damping."
            }
            Self::ThicknessVariation => {
                "The slab is thicker than the reference, shifting all modes upward without adding damping."
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct Defect {
    pub kind: DefectKind,
    /// 0 = no effect, 1 = severe.
    pub severity: f64,
    pub location: Point2,
}

impl Defect {
    pub fn healthy() -> Self {
        Self {
            kind: DefectKind::Healthy,
            severity: 0.0,
            location: Point2::new(0.3, 0.2),
        }
    }

    pub fn new(kind: DefectKind, severity: f64, location: Point2) -> Self {
        Self {
            kind,
            severity: severity.clamp(0.0, 1.0),
            location,
        }
    }

    pub fn is_healthy(&self) -> bool {
        self.kind == DefectKind::Healthy || self.severity <= 0.0
    }

    /// Apply the defect to a set of modes in place.
    pub fn apply(&self, modes: &mut Vec<PlateMode>, geometry: &PlateGeometry, material: Material) {
        if self.is_healthy() || modes.is_empty() {
            return;
        }
        let s = self.severity.clamp(0.0, 1.0);
        let (stiffness_factor, damping_factor, split_count, split_fraction) = match self.kind {
            DefectKind::Healthy => (1.0, 1.0, 0, 0.0),
            DefectKind::Crack => (1.0 - 0.055 * s, 1.0 + 4.0 * s, 3, 0.025 * s),
            DefectKind::Delamination => (1.0 - 0.020 * s, 1.0 + 2.5 * s, 2, 0.020 * s),
            DefectKind::Void => (1.0 - 0.040 * s, 1.0 + 2.6 * s, 0, 0.0),
            DefectKind::ThicknessVariation => (1.0 + 0.100 * s, 1.0, 0, 0.0),
        };
        let frequency_scale = stiffness_factor.sqrt();
        for mode in modes.iter_mut() {
            mode.frequency_hz *= frequency_scale;
            mode.damping_ratio = (mode.damping_ratio * damping_factor).clamp(1e-5, 0.2);
        }

        // Mode splitting: duplicate the most strongly coupled low-order modes.
        if split_count > 0 && split_fraction > 0.0 {
            let mut ranked: Vec<usize> = (0..modes.len())
                .filter(|index| modes[*index].amplitude.abs() > 1e-6)
                .collect();
            ranked.sort_by(|a, b| {
                modes[*a]
                    .frequency_hz
                    .partial_cmp(&modes[*b].frequency_hz)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let mut copies = Vec::new();
            for &index in ranked.iter().take(split_count) {
                let original = modes[index].clone();
                for sign in [-1.0, 1.0] {
                    let mut copy = original.clone();
                    copy.frequency_hz = original.frequency_hz * (1.0 + sign * split_fraction);
                    copy.damping_ratio = (original.damping_ratio * 1.25).clamp(1e-5, 0.2);
                    copy.amplitude *= 0.52;
                    copy.coupling *= 0.52;
                    copies.push(copy);
                }
                // Keep the original but make it slightly less dominant so the
                // three lines are comparable.
                modes[index].amplitude *= 0.85;
            }
            modes.extend(copies);
        }

        // A delamination introduces a low-frequency layer mode.
        if self.kind == DefectKind::Delamination {
            let lowest = modes
                .iter()
                .map(|mode| mode.frequency_hz)
                .fold(f64::INFINITY, f64::min);
            let strongest = modes
                .iter()
                .map(|mode| mode.amplitude.abs())
                .fold(0.0, f64::max)
                .max(1e-12);
            let frequency_hz = (0.34 * lowest).max(45.0);
            let damping_ratio = material.damping_at(frequency_hz) * (2.5 + 2.0 * s);
            modes.push(PlateMode {
                m: 0,
                n: 0,
                frequency_hz,
                damping_ratio,
                coupling: 0.35,
                amplitude: strongest * (0.35 + 0.35 * s),
            });
        }

        modes.sort_by(|a, b| {
            a.frequency_hz
                .partial_cmp(&b.frequency_hz)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let _ = geometry;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::material::GRANITE;
    use crate::plate::PlateModel;

    fn model() -> PlateModel {
        PlateModel::new(
            PlateGeometry::nominal_granite(),
            GRANITE,
            Point2::new(0.18, 0.14),
            Point2::new(0.33, 0.22),
            6000.0,
        )
    }

    #[test]
    fn crack_lowers_frequency_and_raises_damping() {
        let base = model();
        let mut modes = base.modes.clone();
        let before = modes[0].frequency_hz;
        let before_damping = modes[0].damping_ratio;
        Defect::new(DefectKind::Crack, 0.6, Point2::new(0.3, 0.2)).apply(
            &mut modes,
            &base.geometry,
            base.material,
        );
        let first = modes
            .iter()
            .min_by(|a, b| a.frequency_hz.partial_cmp(&b.frequency_hz).unwrap())
            .unwrap();
        assert!(first.frequency_hz < before * 0.99);
        assert!(first.damping_ratio > before_damping);
    }

    #[test]
    fn delamination_adds_low_mode() {
        let base = model();
        let mut modes = base.modes.clone();
        let lowest = modes
            .iter()
            .map(|m| m.frequency_hz)
            .fold(f64::INFINITY, f64::min);
        Defect::new(DefectKind::Delamination, 0.8, Point2::new(0.3, 0.2)).apply(
            &mut modes,
            &base.geometry,
            base.material,
        );
        let new_lowest = modes
            .iter()
            .map(|m| m.frequency_hz)
            .fold(f64::INFINITY, f64::min);
        assert!(new_lowest < lowest * 0.6);
    }
}
