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
use flowlab_core::pdi_samples::{Direction, Sample};
use flowlab_core::stats::quantile;
use flowlab_core::{InputRef, Provenance, ReportMeta};
use flowlab_sim::{MailboxAccess, MailboxPolicy, SimSpec};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Why lab inputs could not be read or applied.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum LabError {
    /// The samples cannot be fitted.
    #[error("cannot fit: {0}")]
    Fit(String),
    /// The lab inputs do not fit the spec.
    #[error("cannot apply lab inputs: {0}")]
    Apply(String),
    /// Only measured data becomes a lab input.
    #[error("{input} is {provenance}: only measured data becomes a lab input")]
    NotMeasured {
        /// The input's name.
        input: String,
        /// Its provenance.
        provenance: &'static str,
    },
}

fn measured(meta: &ReportMeta) -> Result<(), LabError> {
    match meta.source {
        Provenance::Measured => Ok(()),
        p => Err(LabError::NotMeasured { input: meta.input.name.clone(), provenance: p.as_str() }),
    }
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

/// PDI parameters for the simulator, as in `flowlab_sim::PdiSpec`, except that `mailbox_access`
/// is optional: a fit measures rates and costs, not how the stack accesses its mailbox, so a lab
/// file without it keeps the spec's choice.
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
    /// Provenance (`measured` unless the samples file is marked otherwise, see
    /// [`flowlab_core::pdi_samples::provenance`]), the samples file and the assumptions of the fit.
    pub meta: ReportMeta,
    /// Fit of the reads, if there were any.
    pub read: Option<LineFit>,
    /// Fit of the writes, if there were any.
    pub write: Option<LineFit>,
    /// Values for the simulator's `pdi` of the slave.
    pub pdi: PdiValues,
}

fn fit_line(samples: &[Sample]) -> Result<LineFit, LabError> {
    let mut by_size: BTreeMap<u32, Vec<f64>> = BTreeMap::new();
    for s in samples {
        by_size.entry(s.bytes).or_default().push(s.duration_ns);
    }
    if by_size.len() < 2 {
        return Err(LabError::Fit(format!(
            "{} needs accesses of at least two different sizes",
            samples[0].direction.as_str()
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

/// Fit the simulator's PDI model to timed accesses. `input_name` and `input_bytes` identify the
/// samples file; its provenance marker, if any, sets the result's provenance
/// ([`flowlab_core::pdi_samples::provenance`]). A direction without samples takes the other direction's values.
pub fn fit_pdi(samples: &[Sample], input_name: &str, input_bytes: &[u8]) -> Result<PdiFit, LabError> {
    let source = flowlab_core::pdi_samples::provenance(&String::from_utf8_lossy(input_bytes));
    let of = |d: Direction| samples.iter().copied().filter(|s| s.direction == d).collect::<Vec<_>>();
    let (reads, writes) = (of(Direction::Read), of(Direction::Write));
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
    if source != Provenance::Measured {
        assumptions.push(format!(
            "The samples file is marked {}: the durations come from a model, not from hardware, and are not lab inputs.",
            source.as_str()
        ));
    }
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
    let input = InputRef::of(input_name, input_bytes);
    Ok(PdiFit { meta: ReportMeta::new(source, input, assumptions), read, write, pdi })
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

fn short(meta: &ReportMeta) -> &str {
    meta.input.sha256.get(..16).unwrap_or(&meta.input.sha256)
}

/// JSON Schema of [`LabInputs`] (draft 2020-12).
pub fn schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(LabInputs)).unwrap_or_default()
}

impl LabInputs {
    /// Store the PDI values of a fit (keeping a `mailbox_access` set by hand) and name the
    /// samples in `sources`. Refuses a fit of samples that are not measured.
    pub fn set_pdi_fit(&mut self, fit: &PdiFit) -> Result<(), LabError> {
        measured(&fit.meta)?;
        let access = self.pdi.as_ref().and_then(|x| x.mailbox_access);
        self.pdi = Some(PdiValues { mailbox_access: access, ..fit.pdi.clone() });
        self.sources.push(format!("pdi: flowlab pdi-fit {} (sha256 {})", fit.meta.input.name, short(&fit.meta)));
        Ok(())
    }

    /// Store the master's mailbox policy read from the analysis `report_meta` of a capture
    /// (`flowlab policy`), for `station`. Refuses a capture that is not measured.
    pub fn set_mailbox_policy(
        &mut self,
        policy: MailboxPolicy,
        station: u16,
        report_meta: &ReportMeta,
    ) -> Result<(), LabError> {
        measured(report_meta)?;
        self.mailbox_policy = Some(policy);
        self.sources.push(format!(
            "mailbox_policy: flowlab policy {}, station 0x{station:04X} (sha256 {})",
            report_meta.input.name,
            short(report_meta)
        ));
        Ok(())
    }

    /// Apply to `spec`: PDI and application time to slave `slave`, the policy to the master.
    /// The spec's description gains the [`sources`](Self::sources), so the result says what it
    /// was simulated with. Fails if `slave` is not in the spec or the file holds no value.
    pub fn apply(&self, spec: &mut SimSpec, slave: usize) -> Result<(), LabError> {
        if self.pdi.is_none() && self.app_processing_ns.is_none() && self.mailbox_policy.is_none() {
            return Err(LabError::Apply("the lab inputs hold no value".into()));
        }
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
        let from = if self.sources.is_empty() { self.description.clone() } else { self.sources.join("; ") };
        let note =
            format!("lab inputs on slave {slave}: {}", if from.is_empty() { "(no sources given)" } else { &from });
        spec.description = if spec.description.is_empty() { note } else { format!("{} ({note})", spec.description) };
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
        let s = flowlab_core::pdi_samples::parse(&text).unwrap();
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
    fn one_size_cannot_be_fitted() {
        let one_size =
            flowlab_core::pdi_samples::parse("direction,bytes,duration_ns\nread,64,1000\nread,64,1100\n").unwrap();
        assert!(matches!(fit_pdi(&one_size, "x", b""), Err(LabError::Fit(_))));
    }

    #[test]
    fn a_provenance_marker_labels_the_fit() {
        let body = "direction,bytes,duration_ns\nread,4,1000\nread,64,2000\n";
        let sim = format!("# simulated LAN9252\n# flowlab:provenance=simulated\n{body}");
        let fit = fit_pdi(&flowlab_core::pdi_samples::parse(&sim).unwrap(), "sim.csv", sim.as_bytes()).unwrap();
        assert_eq!(fit.meta.source, Provenance::Simulated);
        assert!(fit.meta.assumptions.iter().any(|a| a.contains("marked simulated")));
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
        assert!(spec.description.ends_with("(lab inputs on slave 2: spi.csv)"), "{}", spec.description);
        assert!(lab.apply(&mut spec, 9).is_err());
        assert!(LabInputs::default().apply(&mut spec, 2).is_err(), "an empty file applies nothing");
    }

    #[test]
    fn only_measured_data_becomes_a_lab_input() {
        let text = "direction,bytes,duration_ns\nread,4,1000\nread,64,2000\n";
        let mut fit = fit_pdi(&flowlab_core::pdi_samples::parse(text).unwrap(), "spi.csv", text.as_bytes()).unwrap();
        let mut lab = LabInputs {
            pdi: Some(PdiValues { mailbox_access: Some(MailboxAccess::MessageOnly), ..fit.pdi.clone() }),
            ..LabInputs::default()
        };
        lab.set_pdi_fit(&fit).unwrap();
        assert_eq!(lab.pdi.as_ref().unwrap().mailbox_access, Some(MailboxAccess::MessageOnly));
        assert!(lab.sources[0].starts_with("pdi: flowlab pdi-fit spi.csv"), "{:?}", lab.sources);

        let policy = MailboxPolicy::FixedQuota { accesses_per_cycle: 3 };
        lab.set_mailbox_policy(policy.clone(), 0x1001, &fit.meta).unwrap();
        assert_eq!(lab.mailbox_policy, Some(policy.clone()));
        assert!(lab.sources[1].contains("station 0x1001"));

        fit.meta.source = Provenance::Simulated;
        let before = lab.clone();
        assert!(matches!(lab.set_pdi_fit(&fit), Err(LabError::NotMeasured { provenance: "simulated", .. })));
        fit.meta.source = Provenance::Synthetic;
        assert!(lab.set_mailbox_policy(policy, 0x1001, &fit.meta).is_err());
        assert_eq!(lab, before);
    }
}
