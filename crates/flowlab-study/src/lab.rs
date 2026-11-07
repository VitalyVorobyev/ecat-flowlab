//! Lab inputs for the simulator: the slave's PDI timing fitted from measured access times, the
//! master's mailbox policy read from a capture, and a file that carries both into a spec.
//!
//! The PDI model charges every access a fixed cost plus its bytes at an effective rate
//! (`flowlab_sim::PdiSpec`). [`fit_pdi`] fits exactly that line to timed reads and writes:
//! per access size it takes the median duration (one preempted call does not move it), then a
//! least-squares line through the medians. The intercept is the cost per access, the slope the
//! rate. Residuals per size show whether a line describes the driver at all.

use std::collections::BTreeMap;

use flowlab_analysis::ServicePattern;
use flowlab_core::{InputRef, Provenance, ReportMeta};
use flowlab_sim::{MailboxAccess, MailboxPolicy, SimSpec};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Why lab inputs could not be read or applied.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum LabError {
    /// A line of the samples file is not `direction,bytes,duration_ns`.
    #[error("line {line}: {message}")]
    Sample {
        /// 1-based line number.
        line: usize,
        /// What is wrong.
        message: String,
    },
    /// The samples cannot be fitted.
    #[error("cannot fit: {0}")]
    Fit(String),
    /// The lab inputs do not fit the spec.
    #[error("cannot apply lab inputs: {0}")]
    Apply(String),
}

/// Direction of a PDI access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PdiDirection {
    /// ESC → application.
    Read,
    /// Application → ESC.
    Write,
}

/// One timed ESC access by the slave application.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PdiSample {
    /// Read or write.
    pub direction: PdiDirection,
    /// Bytes moved by the access.
    pub bytes: u32,
    /// Wall time of the access, ns.
    pub duration_ns: f64,
}

/// Parse a samples file: a header line `direction,bytes,duration_ns`, then one access per
/// line (`read,128,7350`). Blank lines and lines starting with `#` are ignored; `r`/`w` are
/// accepted for the direction.
pub fn parse_samples(text: &str) -> Result<Vec<PdiSample>, LabError> {
    let mut out = Vec::new();
    let mut header = false;
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let err = |message: String| LabError::Sample { line: n + 1, message };
        let cols: Vec<&str> = line.split(',').map(str::trim).collect();
        if !header {
            if cols != ["direction", "bytes", "duration_ns"] {
                return Err(err(format!("expected the header `direction,bytes,duration_ns`, found `{line}`")));
            }
            header = true;
            continue;
        }
        let [d, b, t] = cols[..] else { return Err(err(format!("expected 3 columns, found {}", cols.len()))) };
        let direction = match d.to_ascii_lowercase().as_str() {
            "read" | "r" => PdiDirection::Read,
            "write" | "w" => PdiDirection::Write,
            _ => return Err(err(format!("direction `{d}` is neither read nor write"))),
        };
        let bytes: u32 = b.parse().map_err(|_| err(format!("bytes `{b}` is not a whole number")))?;
        let duration_ns: f64 = t.parse().map_err(|_| err(format!("duration `{t}` is not a number")))?;
        if bytes == 0 || !(duration_ns.is_finite() && duration_ns > 0.0) {
            return Err(err("bytes and duration must be positive".into()));
        }
        out.push(PdiSample { direction, bytes, duration_ns });
    }
    if !header {
        return Err(LabError::Sample { line: 1, message: "empty file".into() });
    }
    Ok(out)
}

/// Fit of one direction at one access size.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SizeResidual {
    /// Access size, bytes.
    pub bytes: u32,
    /// Accesses of this size.
    pub samples: u64,
    /// Median measured duration, ns.
    pub median_ns: f64,
    /// 95th percentile of the measured durations, ns.
    pub p95_ns: f64,
    /// Duration the fitted line gives, ns.
    pub fitted_ns: f64,
    /// (median − fitted) / fitted.
    pub relative_error: f64,
}

/// The line `duration = overhead + bytes × 8 / rate` for one direction.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LineFit {
    /// Accesses used.
    pub samples: u64,
    /// Fixed cost per access (the intercept), ns.
    pub overhead_ns: f64,
    /// Effective rate (from the slope), bit/s.
    pub rate_bps: f64,
    /// Per access size.
    pub sizes: Vec<SizeResidual>,
    /// Largest |relative error| over the sizes.
    pub max_relative_error: f64,
}

/// PDI parameters for the simulator, as in `flowlab_sim::PdiSpec`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PdiValues {
    /// Effective read rate (ESC → application), bit/s.
    pub read_bps: u64,
    /// Effective write rate (application → ESC), bit/s.
    pub write_bps: u64,
    /// Fixed cost per access, ns.
    pub access_overhead_ns: u32,
    /// How the application moves mailbox messages, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mailbox_access: Option<MailboxAccess>,
}

/// Result of [`fit_pdi`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PdiFit {
    /// Provenance (`measured`), the samples file and the assumptions of the fit.
    pub meta: ReportMeta,
    /// Fit of the reads, if there were any.
    pub read: Option<LineFit>,
    /// Fit of the writes, if there were any.
    pub write: Option<LineFit>,
    /// Values for the simulator's `pdi` of the slave.
    pub pdi: PdiValues,
}

fn quantile(sorted: &[f64], q: f64) -> f64 {
    let pos = q * (sorted.len() - 1) as f64;
    let (lo, hi) = (pos.floor() as usize, pos.ceil() as usize);
    sorted[lo] + (sorted[hi] - sorted[lo]) * (pos - lo as f64)
}

fn fit_line(samples: &[PdiSample]) -> Result<LineFit, LabError> {
    let mut by_size: BTreeMap<u32, Vec<f64>> = BTreeMap::new();
    for s in samples {
        by_size.entry(s.bytes).or_default().push(s.duration_ns);
    }
    if by_size.len() < 2 {
        return Err(LabError::Fit(format!(
            "{} needs accesses of at least two different sizes",
            if samples[0].direction == PdiDirection::Read { "read" } else { "write" }
        )));
    }
    let points: Vec<(u32, u64, f64, f64)> = by_size
        .into_iter()
        .map(|(b, mut v)| {
            v.sort_by(f64::total_cmp);
            (b, v.len() as u64, quantile(&v, 0.5), quantile(&v, 0.95))
        })
        .collect();
    let n = points.len() as f64;
    let mx = points.iter().map(|p| p.0 as f64).sum::<f64>() / n;
    let my = points.iter().map(|p| p.2).sum::<f64>() / n;
    let sxx: f64 = points.iter().map(|p| (p.0 as f64 - mx).powi(2)).sum();
    let sxy: f64 = points.iter().map(|p| (p.0 as f64 - mx) * (p.2 - my)).sum();
    let slope = sxy / sxx;
    if !(slope.is_finite() && slope > 0.0) {
        return Err(LabError::Fit("duration does not grow with size; no rate can be fitted".into()));
    }
    let intercept = my - slope * mx;
    let sizes: Vec<SizeResidual> = points
        .iter()
        .map(|&(bytes, samples, median_ns, p95_ns)| {
            let fitted_ns = intercept + slope * bytes as f64;
            SizeResidual {
                bytes,
                samples,
                median_ns,
                p95_ns,
                fitted_ns,
                relative_error: (median_ns - fitted_ns) / fitted_ns,
            }
        })
        .collect();
    Ok(LineFit {
        samples: samples.len() as u64,
        overhead_ns: intercept,
        rate_bps: 8e9 / slope,
        max_relative_error: sizes.iter().map(|s| s.relative_error.abs()).fold(0.0, f64::max),
        sizes,
    })
}

/// Fit the simulator's PDI model to timed accesses. `input` names the samples file (for the
/// result's provenance). A direction without samples takes the other direction's values.
pub fn fit_pdi(samples: &[PdiSample], input_name: &str, input_bytes: &[u8]) -> Result<PdiFit, LabError> {
    let of = |d: PdiDirection| samples.iter().copied().filter(|s| s.direction == d).collect::<Vec<_>>();
    let (reads, writes) = (of(PdiDirection::Read), of(PdiDirection::Write));
    let read = (!reads.is_empty()).then(|| fit_line(&reads)).transpose()?;
    let write = (!writes.is_empty()).then(|| fit_line(&writes)).transpose()?;
    let mut assumptions = vec![
        "Each sample is one ESC read or write by the slave application, timed around the driver call.".to_owned(),
        "Per access size the median duration is taken; the line is a least-squares fit through the medians. \
         Intercept = cost per access, slope = 8 / effective rate."
            .into(),
        "The simulator charges one cost per access for reads and writes alike: the access-weighted mean of the \
         two intercepts."
            .into(),
    ];
    let (r, w) = match (&read, &write) {
        (Some(r), Some(w)) => (r, w),
        (Some(r), None) => {
            assumptions.push("No write samples: writes take the read values.".into());
            (r, r)
        }
        (None, Some(w)) => {
            assumptions.push("No read samples: reads take the write values.".into());
            (w, w)
        }
        (None, None) => return Err(LabError::Fit("no samples".into())),
    };
    let overhead =
        (r.overhead_ns * r.samples as f64 + w.overhead_ns * w.samples as f64) / (r.samples + w.samples) as f64;
    if overhead < 0.0 {
        assumptions.push(format!("The fitted cost per access is negative ({overhead:.0} ns) and is set to 0."));
    }
    let pdi = PdiValues {
        read_bps: r.rate_bps.round() as u64,
        write_bps: w.rate_bps.round() as u64,
        access_overhead_ns: overhead.max(0.0).round().min(u32::MAX as f64) as u32,
        mailbox_access: None,
    };
    let input = InputRef {
        name: input_name.to_owned(),
        bytes: input_bytes.len() as u64,
        sha256: Sha256::digest(input_bytes).iter().map(|b| format!("{b:02x}")).collect(),
    };
    Ok(PdiFit { meta: ReportMeta::new(Provenance::Measured, input, assumptions), read, write, pdi })
}

/// The simulator policy for a service pattern read from a capture.
pub fn mailbox_policy(pattern: &ServicePattern) -> MailboxPolicy {
    match *pattern {
        ServicePattern::FixedQuota { accesses_per_cycle } => MailboxPolicy::FixedQuota { accesses_per_cycle },
        ServicePattern::Opportunistic { bus_budget_percent, turnaround_ns } => MailboxPolicy::Opportunistic {
            bus_budget_percent,
            turnaround_ns: turnaround_ns.clamp(0.0, u32::MAX as f64) as u32,
        },
    }
}

/// Measured values that replace the simulator's placeholders for one slave and the master.
/// Every field is optional; `sources` says where each value came from.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LabInputs {
    /// What was measured, on what.
    #[serde(default)]
    pub description: String,
    /// PDI timing of the slave (from `flowlab pdi-fit`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pdi: Option<PdiValues>,
    /// Time the slave application takes to handle a mailbox message, ns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_processing_ns: Option<u32>,
    /// The master's mailbox policy (from `flowlab policy`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mailbox_policy: Option<MailboxPolicy>,
    /// Where each value came from: file, date, setup.
    #[serde(default)]
    pub sources: Vec<String>,
}

/// JSON Schema of [`LabInputs`] (draft 2020-12).
pub fn schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(LabInputs)).unwrap_or_default()
}

impl LabInputs {
    /// Apply to `spec`: PDI and application time to slave `slave`, the policy to the master.
    pub fn apply(&self, spec: &mut SimSpec, slave: usize) -> Result<(), LabError> {
        let n = spec.slaves.len();
        let s =
            spec.slaves.get_mut(slave).ok_or_else(|| LabError::Apply(format!("no slave {slave} in a line of {n}")))?;
        if let Some(p) = &self.pdi {
            s.pdi.read_bps = p.read_bps;
            s.pdi.write_bps = p.write_bps;
            s.pdi.access_overhead_ns = p.access_overhead_ns;
            if let Some(a) = p.mailbox_access {
                s.pdi.mailbox_access = a;
            }
        }
        if let Some(t) = self.app_processing_ns {
            s.app_processing_ns = t;
        }
        if let Some(p) = &self.mailbox_policy {
            spec.mailbox_policy = p.clone();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples(overhead: f64, rate: f64, sizes: &[u32], dir: &str) -> String {
        let mut s = String::new();
        for &b in sizes {
            for k in 0..9 {
                // Spread around the true value, plus one preempted and one early call per size.
                let t = overhead + b as f64 * 8e9 / rate + (k as f64 - 4.0) * 50.0;
                s += &format!("{dir},{b},{t}\n");
            }
            let t = overhead + b as f64 * 8e9 / rate;
            s += &format!("{dir},{b},{}\n{dir},{b},{}\n", 10.0 * t, 0.9 * t);
        }
        s
    }

    #[test]
    fn fit_recovers_rate_and_overhead_despite_outliers() {
        let text = format!(
            "# lab, 2026\ndirection,bytes,duration_ns\n{}{}",
            samples(12_000.0, 18e6, &[6, 64, 128, 512, 1024], "read"),
            samples(8_000.0, 24e6, &[6, 128, 1024], "w"),
        );
        let s = parse_samples(&text).unwrap();
        assert_eq!(s.len(), 88);
        let fit = fit_pdi(&s, "spi.csv", text.as_bytes()).unwrap();
        let r = fit.read.as_ref().unwrap();
        assert!((r.rate_bps / 18e6 - 1.0).abs() < 1e-6, "{}", r.rate_bps);
        assert!((r.overhead_ns - 12_000.0).abs() < 1e-3);
        assert!(r.max_relative_error < 1e-9);
        assert_eq!(r.sizes.len(), 5);
        assert_eq!(fit.pdi.read_bps, 18_000_000);
        assert_eq!(fit.pdi.write_bps, 24_000_000);
        // 55 reads at 12 µs and 33 writes at 8 µs.
        assert_eq!(fit.pdi.access_overhead_ns, 10_500);
        assert_eq!(fit.meta.source, Provenance::Measured);
        assert_eq!(fit.meta.input.name, "spi.csv");
    }

    #[test]
    fn bad_samples_are_rejected_with_their_line() {
        assert!(matches!(parse_samples("bytes,duration\n"), Err(LabError::Sample { line: 1, .. })));
        let e = parse_samples("direction,bytes,duration_ns\nread,12\n").unwrap_err();
        assert_eq!(e, LabError::Sample { line: 2, message: "expected 3 columns, found 2".into() });
        assert!(parse_samples("direction,bytes,duration_ns\nsideways,1,2\n").is_err());
        let one_size = parse_samples("direction,bytes,duration_ns\nread,64,1000\nread,64,1100\n").unwrap();
        assert!(matches!(fit_pdi(&one_size, "x", b""), Err(LabError::Fit(_))));
    }

    #[test]
    fn lab_inputs_replace_placeholders() {
        let lab: LabInputs = serde_json::from_str(
            r#"{ "pdi": { "read_bps": 9000000, "write_bps": 7000000, "access_overhead_ns": 30000,
                          "mailbox_access": "message_only" },
                 "app_processing_ns": 50000,
                 "mailbox_policy": { "kind": "fixed_quota", "accesses_per_cycle": 1 },
                 "sources": ["spi.csv"] }"#,
        )
        .unwrap();
        let mut spec = SimSpec::example();
        lab.apply(&mut spec, 2).unwrap();
        let s = &spec.slaves[2];
        assert_eq!((s.pdi.read_bps, s.pdi.write_bps, s.pdi.access_overhead_ns), (9_000_000, 7_000_000, 30_000));
        assert_eq!(s.pdi.mailbox_access, MailboxAccess::MessageOnly);
        assert_eq!(s.app_processing_ns, 50_000);
        assert_eq!(spec.mailbox_policy, MailboxPolicy::FixedQuota { accesses_per_cycle: 1 });
        assert!(lab.apply(&mut spec, 9).is_err());
    }
}
