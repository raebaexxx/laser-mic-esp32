//! Reconstructed, mono capture ready for analysis.

use crate::stream::PacketStats;

/// A gap in the reconstructed sample stream, expressed in absolute device
/// sample indices.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SampleGap {
    pub start_sample: u64,
    pub len: u64,
}

/// A mono capture plus its provenance and transport statistics.
#[derive(Clone, Debug)]
pub struct Capture {
    pub sample_rate_hz: f64,
    pub samples: Vec<f32>,
    pub start_sample_index: u64,
    pub gaps: Vec<SampleGap>,
    pub stats: Option<PacketStats>,
    pub source: String,
}

impl Capture {
    pub fn new(sample_rate_hz: f64, samples: Vec<f32>, source: impl Into<String>) -> Self {
        Self {
            sample_rate_hz,
            samples,
            start_sample_index: 0,
            gaps: Vec::new(),
            stats: None,
            source: source.into(),
        }
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn duration_s(&self) -> f64 {
        if self.sample_rate_hz > 0.0 {
            self.samples.len() as f64 / self.sample_rate_hz
        } else {
            0.0
        }
    }

    pub fn rms(&self) -> f64 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let sum: f64 = self.samples.iter().map(|v| (*v as f64) * (*v as f64)).sum();
        (sum / self.samples.len() as f64).sqrt()
    }

    pub fn peak(&self) -> f64 {
        self.samples
            .iter()
            .fold(0.0f64, |acc, v| acc.max((*v as f64).abs()))
    }

    /// Fraction of the capture occupied by zero-filled gaps.
    pub fn gap_fraction(&self) -> f64 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let gap: u64 = self.gaps.iter().map(|g| g.len).sum();
        (gap as f64 / self.samples.len() as f64).clamp(0.0, 1.0)
    }

    /// Extract a time window, clamped to the capture boundaries.
    pub fn window(&self, start_s: f64, end_s: f64) -> Vec<f32> {
        let fs = self.sample_rate_hz.max(1.0);
        let start = ((start_s * fs).floor() as usize).min(self.samples.len());
        let end = ((end_s * fs).ceil() as usize).min(self.samples.len());
        if end <= start {
            Vec::new()
        } else {
            self.samples[start..end].to_vec()
        }
    }

    /// Convert the capture to `f64` for the numerical pipeline.
    pub fn samples_f64(&self) -> Vec<f64> {
        self.samples.iter().map(|v| *v as f64).collect()
    }
}
