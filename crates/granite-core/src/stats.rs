//! Small statistical helpers used by analysis and classification.

/// Arithmetic mean.
pub fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f64>() / values.len() as f64
    }
}

/// Sample variance (`n - 1` denominator).
pub fn variance(values: &[f64]) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    let m = mean(values);
    values.iter().map(|v| (v - m) * (v - m)).sum::<f64>() / (values.len() - 1) as f64
}

/// Sample standard deviation.
pub fn std_dev(values: &[f64]) -> f64 {
    variance(values).sqrt()
}

/// Median (sorts a copy).
pub fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    percentile_sorted(&sorted, 0.5)
}

/// Median absolute deviation.
pub fn mad(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let m = median(values);
    let deviations: Vec<f64> = values.iter().map(|v| (v - m).abs()).collect();
    median(&deviations)
}

/// Percentile in `[0, 1]` using linear interpolation.
pub fn percentile(values: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    percentile_sorted(&sorted, p)
}

fn percentile_sorted(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }
    let pos = p.clamp(0.0, 1.0) * (sorted.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    if lo == hi {
        sorted[lo]
    } else {
        let frac = pos - lo as f64;
        sorted[lo] * (1.0 - frac) + sorted[hi] * frac
    }
}

/// Root mean square.
pub fn rms(values: &[f64]) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        (values.iter().map(|v| v * v).sum::<f64>() / values.len() as f64).sqrt()
    }
}

/// Peak absolute value.
pub fn peak_abs(values: &[f64]) -> f64 {
    values.iter().fold(0.0, |acc, v| acc.max(v.abs()))
}

/// Result of a least-squares line fit.
#[derive(Clone, Copy, Debug)]
pub struct LinearFit {
    pub slope: f64,
    pub intercept: f64,
    pub r_squared: f64,
}

/// Ordinary least squares for `y = slope * x + intercept`.
pub fn linreg(xs: &[f64], ys: &[f64]) -> Option<LinearFit> {
    if xs.len() < 2 || xs.len() != ys.len() {
        return None;
    }
    let mean_x = mean(xs);
    let mean_y = mean(ys);
    let mut sxx = 0.0;
    let mut sxy = 0.0;
    let mut syy = 0.0;
    for (x, y) in xs.iter().zip(ys.iter()) {
        let dx = x - mean_x;
        let dy = y - mean_y;
        sxx += dx * dx;
        sxy += dx * dy;
        syy += dy * dy;
    }
    if sxx <= 0.0 {
        return None;
    }
    let slope = sxy / sxx;
    let intercept = mean_y - slope * mean_x;
    let r_squared = if syy <= 0.0 {
        1.0
    } else {
        (sxy * sxy / (sxx * syy)).clamp(0.0, 1.0)
    };
    Some(LinearFit {
        slope,
        intercept,
        r_squared,
    })
}

/// Coefficient of variation (`std / |mean|`).
pub fn coefficient_of_variation(values: &[f64]) -> f64 {
    let m = mean(values);
    if m.abs() < 1e-12 {
        0.0
    } else {
        std_dev(values) / m.abs()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_statistics() {
        let values = [1.0, 2.0, 3.0, 4.0, 5.0];
        assert!((mean(&values) - 3.0).abs() < 1e-12);
        assert!((median(&values) - 3.0).abs() < 1e-12);
        assert!((std_dev(&values) - 1.581_138_830_2).abs() < 1e-9);
        assert!((percentile(&values, 0.25) - 2.0).abs() < 1e-12);
        assert!((rms(&values) - 3.316_624_790_4).abs() < 1e-9);
    }

    #[test]
    fn line_fit_recovers_parameters() {
        let xs: Vec<f64> = (0..100).map(|i| i as f64).collect();
        let ys: Vec<f64> = xs.iter().map(|x| 2.5 * x + 1.25).collect();
        let fit = linreg(&xs, &ys).unwrap();
        assert!((fit.slope - 2.5).abs() < 1e-12);
        assert!((fit.intercept - 1.25).abs() < 1e-12);
        assert!((fit.r_squared - 1.0).abs() < 1e-12);
    }
}
