//! Drawing, running and judging the runs of a study.

use flowlab_analysis::CaptureReport;
use flowlab_core::{InputRef, Provenance, ReportMeta};
use flowlab_sim::{SimResult, SimSpec};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use ts_rs::TS;

use crate::StudyError;
use crate::spec::{CaptureMetric, Goal, Metric, Parameter, Sampling, StudySpec};

/// One run of a study.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct StudyRun {
    /// Run number.
    pub index: u32,
    /// Parameter values, as JSON text, in parameter order.
    pub values: Vec<String>,
    /// The same values where they are numbers (for plots).
    pub numeric: Vec<Option<f64>>,
    /// Objective values in objective order (the fit error first for a calibration).
    pub objectives: Vec<Option<f64>>,
    /// Ran, finished, and met every constraint.
    pub feasible: bool,
    /// Why not, if not.
    pub reason: Option<String>,
    /// Not dominated by another feasible run.
    pub pareto: bool,
    /// The run's full simulator spec (Pareto and best runs only), to simulate it again.
    #[ts(type = "Record<string, unknown> | null")]
    pub spec: Option<Value>,
}

/// Measured vs simulated for one calibration target, at the best run.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct Residual {
    /// The simulated quantity.
    pub label: String,
    /// Where the measurement came from.
    pub source: String,
    /// Measured value.
    pub measured: f64,
    /// Simulated value at the best run.
    pub simulated: Option<f64>,
    /// (simulated − measured) / measured.
    pub relative_error: Option<f64>,
}

/// Result of a study.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct StudyResult {
    /// Provenance (`simulated`), the study spec's identity, assumptions.
    pub meta: ReportMeta,
    /// Study description.
    pub description: String,
    /// Parameter paths.
    pub parameters: Vec<String>,
    /// Objective labels.
    pub objectives: Vec<String>,
    /// `"minimize"` / `"maximize"`, per objective.
    pub goals: Vec<String>,
    /// Every run.
    pub runs: Vec<StudyRun>,
    /// Feasible runs.
    pub feasible: u32,
    /// Indices of the Pareto-optimal runs.
    pub pareto: Vec<u32>,
    /// Best run, chosen among the Pareto-optimal runs: smallest fit error for a calibration, else
    /// the best on the first objective. `None` without objectives and targets.
    pub best: Option<u32>,
    /// Calibration residuals at the best run.
    pub residuals: Vec<Residual>,
    /// The capture a calibration was fitted to.
    pub calibrated_against: Option<InputRef>,
}

/// A simulator result read as one metric.
pub fn sim_metric(r: &SimResult, m: &Metric) -> Option<f64> {
    let t = |i: usize| r.transfers.get(i);
    match m {
        Metric::TransferTimeS { transfer } => t(*transfer)?.duration_ns.map(|d| d / 1e9),
        Metric::ExchangeTimeS { transfer } => t(*transfer)?.exchange_ns.map(|d| d / 1e9),
        Metric::MaxTransferTimeS => {
            r.transfers.iter().map(|t| t.duration_ns.map(|d| d / 1e9)).try_fold(0.0, |m: f64, d| d.map(|d| m.max(d)))
        }
        Metric::GoodputMbps { transfer } => t(*transfer)?.goodput_bps.map(|g| g / 1e6),
        Metric::LinkUtilizationPercent => Some(r.link_utilization_percent),
        Metric::PdiBusyPercentMax => r.slaves.iter().map(|s| s.pdi_busy_percent).reduce(f64::max),
        Metric::DpramUsedPercentMax => r
            .slaves
            .iter()
            .filter(|s| s.dpram_bytes > 0)
            .map(|s| 100.0 * s.dpram_used_bytes as f64 / s.dpram_bytes as f64)
            .reduce(f64::max),
    }
}

/// An analysed capture read as one measurement.
pub fn capture_metric(r: &CaptureReport, m: &CaptureMetric) -> Option<f64> {
    let span = |s: f64, e: Option<f64>| e.map(|e| (e - s) / 1e9);
    match m {
        CaptureMetric::FoeDurationS { index } => r.mailbox.foe.get(*index).and_then(|t| span(t.start_ns, t.end_ns)),
        CaptureMetric::FoeGoodputMbps { index } => r.mailbox.foe.get(*index)?.goodput_bps.map(|g| g / 1e6),
        CaptureMetric::SdoDurationS { index } => {
            r.mailbox.coe.transactions.get(*index).and_then(|t| span(t.start_ns, t.end_ns))
        }
        CaptureMetric::EoeFrameDurationS { index } => {
            r.mailbox.eoe.frames.get(*index).and_then(|f| span(f.start_ns, Some(f.end_ns)))
        }
        CaptureMetric::CyclePeriodUs => r.streams.first().map(|s| s.period_ns / 1e3),
    }
}

fn capture_label(m: &CaptureMetric) -> String {
    match m {
        CaptureMetric::FoeDurationS { index } => format!("capture: FoE transfer {index} duration (s)"),
        CaptureMetric::FoeGoodputMbps { index } => format!("capture: FoE transfer {index} goodput (Mbit/s)"),
        CaptureMetric::SdoDurationS { index } => format!("capture: SDO transfer {index} duration (s)"),
        CaptureMetric::EoeFrameDurationS { index } => format!("capture: EoE frame {index} duration (s)"),
        CaptureMetric::CyclePeriodUs => "capture: cycle period (µs)".into(),
    }
}

// ---------------------------------------------------------------------------------------------
// Sampling
// ---------------------------------------------------------------------------------------------

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn range_value(p: &Parameter, f: f64) -> Value {
    let (lo, hi) = (p.min.unwrap_or(0.0), p.max.unwrap_or(0.0));
    let v = if p.log && lo > 0.0 && hi > 0.0 { (lo.ln() + f * (hi.ln() - lo.ln())).exp() } else { lo + f * (hi - lo) };
    if p.integer { Value::from(v.round() as i64) } else { Value::from(v) }
}

fn grid_values(p: &Parameter) -> Result<Vec<Value>, StudyError> {
    if !p.values.is_empty() {
        return Ok(p.values.clone());
    }
    match (p.min, p.max, p.steps) {
        (Some(_), Some(_), Some(steps)) if steps >= 2 => {
            let mut v: Vec<Value> = (0..steps).map(|i| range_value(p, i as f64 / (steps - 1) as f64)).collect();
            v.dedup();
            Ok(v)
        }
        _ => Err(StudyError::Spec(format!("{}: give `values`, or `min`, `max` and `steps` ≥ 2", p.path))),
    }
}

fn points(spec: &StudySpec) -> Result<Vec<Vec<Value>>, StudyError> {
    match &spec.sampling {
        Sampling::Grid => {
            let lists: Vec<Vec<Value>> = spec.parameters.iter().map(grid_values).collect::<Result<_, _>>()?;
            let total = lists.iter().map(Vec::len).try_fold(1usize, |a, n| a.checked_mul(n)).unwrap_or(usize::MAX);
            if total > spec.max_runs as usize {
                return Err(StudyError::Spec(format!(
                    "the grid has {total} runs, more than max_runs = {}; use fewer steps or Latin hypercube sampling",
                    spec.max_runs
                )));
            }
            let mut out = vec![Vec::new()];
            for list in &lists {
                out = out
                    .into_iter()
                    .flat_map(|prefix| list.iter().map(move |v| [prefix.clone(), vec![v.clone()]].concat()))
                    .collect();
            }
            Ok(out)
        }
        Sampling::LatinHypercube { samples, seed } => {
            let n = *samples as usize;
            if n == 0 || n > spec.max_runs as usize {
                return Err(StudyError::Spec(format!("samples must be within 1..={}", spec.max_runs)));
            }
            let mut rng = Rng(*seed);
            let mut columns = Vec::with_capacity(spec.parameters.len());
            for p in &spec.parameters {
                let mut strata: Vec<usize> = (0..n).collect();
                for i in (1..n).rev() {
                    strata.swap(i, rng.below(i + 1));
                }
                let col: Vec<Value> = strata
                    .into_iter()
                    .map(|j| {
                        let f = (j as f64 + rng.unit()) / n as f64;
                        if p.values.is_empty() {
                            if p.min.is_none() || p.max.is_none() {
                                return Err(StudyError::Spec(format!("{}: give `values` or `min` and `max`", p.path)));
                            }
                            Ok(range_value(p, f))
                        } else {
                            Ok(p.values[((f * p.values.len() as f64) as usize).min(p.values.len() - 1)].clone())
                        }
                    })
                    .collect::<Result<_, _>>()?;
                columns.push(col);
            }
            Ok((0..n).map(|i| columns.iter().map(|c| c[i].clone()).collect()).collect())
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Running
// ---------------------------------------------------------------------------------------------

struct Outcome {
    objectives: Vec<Option<f64>>,
    targets: Vec<Option<f64>>,
    feasible: bool,
    reason: Option<String>,
    spec: Option<Value>,
}

fn apply(base: &Value, params: &[Parameter], values: &[Value]) -> Result<Value, String> {
    let mut v = base.clone();
    for (p, val) in params.iter().zip(values) {
        for path in std::iter::once(&p.path).chain(&p.also) {
            *v.pointer_mut(path).ok_or_else(|| format!("{path} is not in the spec"))? = val.clone();
        }
    }
    Ok(v)
}

fn evaluate(spec: &StudySpec, base: &Value, values: &[Value], measured: &[f64]) -> Outcome {
    let fail = |reason: String| Outcome {
        objectives: vec![None; spec.objectives.len() + usize::from(!spec.targets.is_empty())],
        targets: vec![None; spec.targets.len()],
        feasible: false,
        reason: Some(reason),
        spec: None,
    };
    let json = match apply(base, &spec.parameters, values) {
        Ok(j) => j,
        Err(e) => return fail(e),
    };
    let sim: SimSpec = match serde_json::from_value(json.clone()) {
        Ok(s) => s,
        Err(e) => return fail(format!("not a valid spec: {e}")),
    };
    let r = match flowlab_sim::simulate(&sim, None::<std::io::Sink>) {
        Ok(r) => r,
        Err(e) => return fail(e.to_string()),
    };
    let mut reason = None;
    if let Some(t) = r.transfers.iter().find(|t| !t.verified) {
        reason = Some(format!("transfer {} did not finish verified within max_duration_ms", t.index));
    }
    let targets: Vec<Option<f64>> = spec.targets.iter().map(|t| sim_metric(&r, &t.metric)).collect();
    let mut objectives = Vec::with_capacity(spec.objectives.len() + 1);
    if !spec.targets.is_empty() {
        let mut sum = 0.0;
        let mut weights = 0.0;
        let mut ok = true;
        for ((t, sim), meas) in spec.targets.iter().zip(&targets).zip(measured) {
            match sim {
                Some(s) if *meas != 0.0 => {
                    sum += t.weight * ((s - meas) / meas).powi(2);
                    weights += t.weight;
                }
                _ => ok = false,
            }
        }
        objectives.push((ok && weights > 0.0).then(|| (sum / weights).sqrt()));
    }
    for o in &spec.objectives {
        objectives.push(sim_metric(&r, &o.metric));
    }
    for c in &spec.constraints {
        let Some(v) = sim_metric(&r, &c.metric) else {
            reason.get_or_insert_with(|| format!("{} is unavailable", c.metric.label()));
            continue;
        };
        if let Some(max) = c.max
            && v > max
        {
            reason.get_or_insert_with(|| format!("{} = {v:.4} exceeds {max}", c.metric.label()));
        }
        if let Some(min) = c.min
            && v < min
        {
            reason.get_or_insert_with(|| format!("{} = {v:.4} is below {min}", c.metric.label()));
        }
    }
    if reason.is_none() && objectives.iter().any(Option::is_none) {
        reason = Some("an objective is unavailable for this run".into());
    }
    Outcome { objectives, targets, feasible: reason.is_none(), reason, spec: Some(json) }
}

fn dominates(a: &[Option<f64>], b: &[Option<f64>], goals: &[Goal]) -> bool {
    let mut better = false;
    for ((x, y), g) in a.iter().zip(b).zip(goals) {
        let (Some(x), Some(y)) = (x, y) else { return false };
        let (x, y) = if *g == Goal::Maximize { (-x, -y) } else { (*x, *y) };
        if x > y {
            return false;
        }
        if x < y {
            better = true;
        }
    }
    better
}

/// Run `spec`. Targets with `from_capture` read `capture` (an analysed capture's report).
pub fn run_study(spec: &StudySpec, capture: Option<&CaptureReport>) -> Result<StudyResult, StudyError> {
    if spec.parameters.is_empty() {
        return Err(StudyError::Spec("at least one parameter is required".into()));
    }
    let mut measured = Vec::with_capacity(spec.targets.len());
    let mut sources = Vec::with_capacity(spec.targets.len());
    for t in &spec.targets {
        let (v, src) = match (&t.measured, &t.from_capture) {
            (Some(v), _) => (*v, "given".to_owned()),
            (None, Some(m)) => {
                let r =
                    capture.ok_or_else(|| StudyError::Spec("a target reads the capture, but none was given".into()))?;
                let v = capture_metric(r, m)
                    .ok_or_else(|| StudyError::Spec(format!("{} is not in the capture", capture_label(m))))?;
                (v, capture_label(m))
            }
            (None, None) => return Err(StudyError::Spec("a target needs `measured` or `from_capture`".into())),
        };
        if v == 0.0 {
            return Err(StudyError::Spec("a measured value of 0 cannot be fitted relatively".into()));
        }
        measured.push(v);
        sources.push(src);
    }
    let mut base = serde_json::to_value(&spec.base).map_err(|e| StudyError::Spec(e.to_string()))?;
    if let Some(c) = base.pointer_mut("/capture/enabled") {
        *c = Value::Bool(false);
    }
    for p in &spec.parameters {
        for path in std::iter::once(&p.path).chain(&p.also) {
            if base.pointer(path).is_none() {
                return Err(StudyError::Spec(format!("{path} is not in the base spec")));
            }
        }
    }
    let pts = points(spec)?;

    // Runs are independent and deterministic; spread them over threads, keep their order.
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(pts.len().max(1));
    let mut outcomes: Vec<Option<Outcome>> = (0..pts.len()).map(|_| None).collect();
    std::thread::scope(|s| {
        for (k, chunk) in outcomes.chunks_mut(pts.len().div_ceil(threads).max(1)).enumerate() {
            let start = k * pts.len().div_ceil(threads).max(1);
            let (pts, base, measured) = (&pts, &base, &measured);
            s.spawn(move || {
                for (i, slot) in chunk.iter_mut().enumerate() {
                    *slot = Some(evaluate(spec, base, &pts[start + i], measured));
                }
            });
        }
    });
    let outcomes: Vec<Outcome> = outcomes.into_iter().map(|o| o.expect("every run evaluated")).collect();

    let mut goals: Vec<Goal> = Vec::new();
    let mut labels: Vec<String> = Vec::new();
    if !spec.targets.is_empty() {
        goals.push(Goal::Minimize);
        labels.push("fit error (weighted RMS relative)".into());
    }
    for o in &spec.objectives {
        goals.push(o.goal);
        labels.push(o.metric.label());
    }
    let pareto: Vec<u32> = (0..outcomes.len())
        .filter(|&i| {
            outcomes[i].feasible
                && !goals.is_empty()
                && !(0..outcomes.len()).any(|j| {
                    j != i
                        && outcomes[j].feasible
                        && dominates(&outcomes[j].objectives, &outcomes[i].objectives, &goals)
                })
        })
        .map(|i| i as u32)
        .collect();
    let best = pareto.iter().copied().min_by(|&a, &b| {
        let key = |i: u32| {
            let v = outcomes[i as usize].objectives.first().copied().flatten().unwrap_or(f64::INFINITY);
            if goals.first() == Some(&Goal::Maximize) { -v } else { v }
        };
        key(a).total_cmp(&key(b)).then(a.cmp(&b))
    });
    let residuals = match best {
        Some(b) if !spec.targets.is_empty() => spec
            .targets
            .iter()
            .zip(&outcomes[b as usize].targets)
            .zip(measured.iter().zip(&sources))
            .map(|((t, sim), (meas, src))| Residual {
                label: t.metric.label(),
                source: src.clone(),
                measured: *meas,
                simulated: *sim,
                relative_error: sim.map(|s| (s - meas) / meas),
            })
            .collect(),
        _ => Vec::new(),
    };
    let runs = outcomes
        .into_iter()
        .zip(pts)
        .enumerate()
        .map(|(i, (o, vals))| {
            let keep = pareto.contains(&(i as u32)) || best == Some(i as u32);
            StudyRun {
                index: i as u32,
                numeric: vals.iter().map(Value::as_f64).collect(),
                values: vals.iter().map(Value::to_string).collect(),
                objectives: o.objectives,
                feasible: o.feasible,
                reason: o.reason,
                pareto: pareto.contains(&(i as u32)),
                spec: if keep { o.spec } else { None },
            }
        })
        .collect::<Vec<_>>();
    let json = serde_json::to_vec(spec).map_err(|e| StudyError::Spec(e.to_string()))?;
    let input = InputRef {
        name: "study.json".into(),
        bytes: json.len() as u64,
        sha256: Sha256::digest(&json).iter().map(|b| format!("{b:02x}")).collect(),
    };
    let mut assumptions = vec![
        "Every run is a simulation of the base spec with the listed parameters changed; the simulator's own assumptions apply to each.".to_owned(),
        "A run is feasible when its spec is valid, every transfer finishes verified, and every constraint holds.".into(),
        "Pareto-optimal: no other feasible run is at least as good on every objective and better on one.".into(),
    ];
    if !spec.targets.is_empty() {
        assumptions.push("Fit error: square root of the weighted mean of squared relative errors between simulated and measured values. A small error shows that the model can reproduce these numbers, not that its parameters are the physical ones — several parameter sets may fit equally.".into());
    }
    Ok(StudyResult {
        meta: ReportMeta::new(Provenance::Simulated, input, assumptions),
        description: spec.description.clone(),
        parameters: spec.parameters.iter().map(|p| p.path.clone()).collect(),
        objectives: labels,
        goals: goals.iter().map(|g| if *g == Goal::Maximize { "maximize" } else { "minimize" }.to_owned()).collect(),
        feasible: runs.iter().filter(|r| r.feasible).count() as u32,
        runs,
        pareto,
        best,
        residuals,
        calibrated_against: if spec.targets.iter().any(|t| t.from_capture.is_some()) {
            capture.map(|c| c.meta.input.clone())
        } else {
            None
        },
    })
}
