//! Elastic material presets.

#[derive(Clone, Copy, Debug)]
pub struct Material {
    pub name: &'static str,
    pub youngs_modulus_pa: f64,
    pub density_kg_m3: f64,
    pub poisson_ratio: f64,
    /// Baseline modal loss factor (damping ratio) at low frequency.
    pub loss_factor: f64,
}

pub const GRANITE: Material = Material {
    name: "granite",
    youngs_modulus_pa: 50.0e9,
    density_kg_m3: 2700.0,
    poisson_ratio: 0.25,
    loss_factor: 0.0022,
};

pub const MARBLE: Material = Material {
    name: "marble",
    youngs_modulus_pa: 70.0e9,
    density_kg_m3: 2700.0,
    poisson_ratio: 0.27,
    loss_factor: 0.003,
};

pub const CONCRETE: Material = Material {
    name: "concrete",
    youngs_modulus_pa: 30.0e9,
    density_kg_m3: 2400.0,
    poisson_ratio: 0.20,
    loss_factor: 0.006,
};

impl Material {
    /// Flexural rigidity of a plate with thickness `h`.
    pub fn flexural_rigidity(&self, thickness_m: f64) -> f64 {
        self.youngs_modulus_pa * thickness_m.powi(3)
            / (12.0 * (1.0 - self.poisson_ratio * self.poisson_ratio))
    }

    /// Damping ratio at a given frequency; granite damping rises slowly with
    /// frequency in the audio band.
    pub fn damping_at(&self, frequency_hz: f64) -> f64 {
        let octaves = (frequency_hz.max(50.0) / 1000.0).log2();
        (self.loss_factor * (1.0 + 0.08 * octaves.max(-2.0))).clamp(1e-5, 0.2)
    }
}
