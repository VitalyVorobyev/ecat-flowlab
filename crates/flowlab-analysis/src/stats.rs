use serde::Serialize;
use ts_rs::TS;

/// Fixed-width histogram.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct Histogram {
    /// Lower edge of the first bin.
    pub lo: f64,
    /// Bin width.
    pub bin_width: f64,
    /// Count per bin.
    pub counts: Vec<u32>,
}

/// Summary statistics of a sample, in the unit of the sample (ns unless stated).
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct Distribution {
    /// Sample size.
    pub count: u64,
    /// Minimum.
    pub min: f64,
    /// Maximum.
    pub max: f64,
    /// Mean.
    pub mean: f64,
    /// Standard deviation (population).
    pub std: f64,
    /// Median.
    pub p50: f64,
    /// 1st percentile.
    pub p1: f64,
    /// 5th percentile.
    pub p5: f64,
    /// 95th percentile.
    pub p95: f64,
    /// 99th percentile.
    pub p99: f64,
    /// Histogram over [p0.1, p99.9] widened to include the extremes when few bins are lost.
    pub histogram: Histogram,
}

/// "Nice" step ≥ `raw`: 1, 2 or 5 × 10^k.
pub(crate) fn nice_step(raw: f64) -> f64 {
    if raw <= 0.0 || !raw.is_finite() {
        return 1.0;
    }
    let mag = 10f64.powf(raw.log10().floor());
    for m in [1.0, 2.0, 5.0, 10.0] {
        if m * mag >= raw {
            return m * mag;
        }
    }
    10.0 * mag
}

fn quantile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let pos = q * (sorted.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    sorted[lo] + (sorted[hi] - sorted[lo]) * (pos - lo as f64)
}

impl Distribution {
    /// Describe `values`. Returns `None` for an empty sample.
    pub fn of(values: &[f64], bins: usize) -> Option<Self> {
        if values.is_empty() {
            return None;
        }
        let mut s = values.to_vec();
        s.sort_by(f64::total_cmp);
        let n = s.len() as f64;
        let mean = s.iter().sum::<f64>() / n;
        let var = s.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / n;
        let (min, max) = (s[0], s[s.len() - 1]);
        // Histogram range: robust to a handful of outliers, which are clamped into the edge bins.
        let (mut lo, mut hi) = (quantile(&s, 0.001), quantile(&s, 0.999));
        if hi <= lo {
            lo = min;
            hi = max;
        }
        let bins = bins.max(1);
        let width = nice_step(((hi - lo) / bins as f64).max(f64::EPSILON));
        let lo = (lo / width).floor() * width;
        let nbins = (((hi - lo) / width).floor() as usize + 1).min(4 * bins);
        let mut counts = vec![0u32; nbins];
        for v in &s {
            let i = ((v - lo) / width).floor().clamp(0.0, (nbins - 1) as f64) as usize;
            counts[i] += 1;
        }
        Some(Self {
            count: s.len() as u64,
            min,
            max,
            mean,
            std: var.sqrt(),
            p50: quantile(&s, 0.5),
            p1: quantile(&s, 0.01),
            p5: quantile(&s, 0.05),
            p95: quantile(&s, 0.95),
            p99: quantile(&s, 0.99),
            histogram: Histogram { lo, bin_width: width, counts },
        })
    }
}

/// Reduce `(x, y)` points to at most `max_points` by keeping the min and max of each bucket,
/// so spikes survive.
pub(crate) fn decimate(points: &[(f64, f64)], max_points: usize) -> Vec<(f64, f64)> {
    if points.len() <= max_points || max_points < 4 {
        return points.to_vec();
    }
    let buckets = max_points / 2;
    let per = points.len().div_ceil(buckets);
    let mut out = Vec::with_capacity(buckets * 2);
    for chunk in points.chunks(per) {
        let (mut imin, mut imax) = (0, 0);
        for (i, p) in chunk.iter().enumerate() {
            if p.1 < chunk[imin].1 {
                imin = i;
            }
            if p.1 > chunk[imax].1 {
                imax = i;
            }
        }
        let (a, b) = if imin <= imax { (imin, imax) } else { (imax, imin) };
        out.push(chunk[a]);
        if b != a {
            out.push(chunk[b]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_and_histogram() {
        let v: Vec<f64> = (1..=100).map(f64::from).collect();
        let d = Distribution::of(&v, 10).unwrap();
        assert_eq!(d.count, 100);
        assert!((d.p50 - 50.5).abs() < 1e-9);
        assert!((d.mean - 50.5).abs() < 1e-9);
        assert_eq!(d.histogram.counts.iter().sum::<u32>(), 100);
        assert!(Distribution::of(&[], 10).is_none());
        let one = Distribution::of(&[5.0], 10).unwrap();
        assert_eq!(one.histogram.counts.iter().sum::<u32>(), 1);
    }

    #[test]
    fn nice_steps() {
        assert_eq!(nice_step(3.0), 5.0);
        assert_eq!(nice_step(0.012), 0.02);
        assert_eq!(nice_step(1000.0), 1000.0);
    }

    #[test]
    fn decimation_keeps_spikes() {
        let mut p: Vec<(f64, f64)> = (0..10_000).map(|i| (i as f64, 1.0)).collect();
        p[5_000].1 = 99.0;
        let d = decimate(&p, 200);
        assert!(d.len() <= 200);
        assert!(d.iter().any(|q| q.1 == 99.0));
    }
}
