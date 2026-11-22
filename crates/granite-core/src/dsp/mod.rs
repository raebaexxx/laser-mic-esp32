//! Signal-processing primitives used by the granite NDT pipeline.
//!
//! Everything here is dependency-free (`std` only) so the host, the digital
//! twin and the unit tests all use exactly the same numerical code.

pub mod complex;
pub mod envelope;
pub mod fft;
pub mod filters;
pub mod spectrum;
pub mod window;

pub use complex::Complex;
pub use filters::{filtfilt, filtfilt_cascade, Biquad};
pub use spectrum::Spectrum;
pub use window::WindowKind;

/// Smallest positive `f64` used to avoid `log10(0)`.
pub const TINY: f64 = 1e-15;

/// Convert a linear amplitude to decibels.
#[inline]
pub fn to_db(value: f64) -> f64 {
    20.0 * value.abs().max(TINY).log10()
}

/// Convert a power ratio to decibels.
#[inline]
pub fn power_to_db(value: f64) -> f64 {
    10.0 * value.abs().max(TINY).log10()
}

/// Round `value` to the nearest integer without relying on `f64::round` in
/// hot paths (and to keep behaviour obvious).
#[inline]
pub fn round_to_i64(value: f64) -> i64 {
    if value >= 0.0 {
        (value + 0.5) as i64
    } else {
        (value - 0.5) as i64
    }
}
