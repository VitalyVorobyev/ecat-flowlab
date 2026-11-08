//! The one percentile definition every FlowLab result uses, so that a percentile of a capture,
//! of a simulation and of a lab measurement mean the same thing.

/// The `q`-quantile (0 ≤ `q` ≤ 1) of `sorted` (ascending), by linear interpolation between
/// the two closest ranks: position `q · (n − 1)`. The same definition as NumPy's default
/// (`linear`). Returns 0 for an empty slice; `q` outside 0–1 is clamped.
///
/// ```
/// use flowlab_core::stats::quantile;
/// assert_eq!(quantile(&[1.0, 2.0, 3.0, 4.0], 0.5), 2.5);
/// assert_eq!(quantile(&[10.0, 20.0], 1.0), 20.0);
/// assert_eq!(quantile(&[], 0.5), 0.0);
/// ```
pub fn quantile(sorted: &[f64], q: f64) -> f64 {
    let Some(last) = sorted.len().checked_sub(1) else {
        return 0.0;
    };
    let pos = q.clamp(0.0, 1.0) * last as f64;
    let (lo, hi) = (pos.floor() as usize, pos.ceil() as usize);
    sorted[lo] + (sorted[hi] - sorted[lo]) * (pos - lo as f64)
}

#[cfg(test)]
mod tests {
    use super::quantile;

    #[test]
    fn interpolates_between_ranks() {
        let v: Vec<f64> = (1..=101).map(f64::from).collect();
        assert_eq!(quantile(&v, 0.0), 1.0);
        assert_eq!(quantile(&v, 0.99), 100.0);
        assert_eq!(quantile(&v, 0.995), 100.5);
        assert_eq!(quantile(&[7.0], 0.3), 7.0);
        assert_eq!(quantile(&[1.0, 3.0], 2.0), 3.0);
    }
}
