use flowlab_sim::SimSpec;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A parameter study: variations of one simulator spec, what to optimize, what must hold, and
/// optionally measured values to fit (calibration).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StudySpec {
    /// Free text.
    #[serde(default)]
    pub description: String,
    /// The spec every run starts from. Its capture output is ignored: studies write no frames.
    pub base: SimSpec,
    /// What varies.
    pub parameters: Vec<Parameter>,
    /// How parameter values are combined.
    #[serde(default)]
    pub sampling: Sampling,
    /// What to optimize; the Pareto frontier is over these.
    #[serde(default)]
    pub objectives: Vec<Objective>,
    /// What every acceptable run must satisfy.
    #[serde(default)]
    pub constraints: Vec<Constraint>,
    /// Measured values to fit. When present, the study is a calibration: its first objective
    /// is the fit error (minimize), and the best run's residuals are reported.
    #[serde(default)]
    pub targets: Vec<Target>,
    /// Upper bound on runs; a grid larger than this is refused.
    #[serde(default = "default_max_runs")]
    pub max_runs: u32,
}

fn default_max_runs() -> u32 {
    400
}

/// One varied parameter: a JSON pointer into the base spec and its values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Parameter {
    /// JSON pointer into `base`, e.g. `/slaves/2/pdi/read_bps` or `/cycle_time_us`.
    pub path: String,
    /// Further pointers that take the same value (e.g. the PDI write rate with the read rate).
    #[serde(default)]
    pub also: Vec<String>,
    /// Explicit values (any JSON). Takes precedence over the range.
    #[serde(default)]
    pub values: Vec<serde_json::Value>,
    /// Range start (with `max` and `steps`).
    #[serde(default)]
    pub min: Option<f64>,
    /// Range end.
    #[serde(default)]
    pub max: Option<f64>,
    /// Points in the range for a grid (≥ 2).
    #[serde(default)]
    pub steps: Option<u32>,
    /// Round range values to integers.
    #[serde(default)]
    pub integer: bool,
    /// Space range values evenly on a log scale.
    #[serde(default)]
    pub log: bool,
}

/// How runs are drawn.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Sampling {
    /// Every combination of every parameter's values.
    #[default]
    Grid,
    /// `samples` runs; each parameter's range (or value list) is split into `samples` strata
    /// and each stratum is used once.
    LatinHypercube {
        /// Runs.
        samples: u32,
        /// Seed of the stratum permutations.
        seed: u64,
    },
}

/// A number read from a simulation result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Metric {
    /// Duration of one transfer, s.
    TransferTimeS {
        /// Transfer index in the spec.
        transfer: usize,
    },
    /// Span of one transfer's mailbox messages at the master (first to last message), s —
    /// comparable with the durations `flowlab analyze` reports from a capture.
    ExchangeTimeS {
        /// Transfer index.
        transfer: usize,
    },
    /// The longest transfer, s.
    MaxTransferTimeS,
    /// Goodput of one transfer, Mbit/s.
    GoodputMbps {
        /// Transfer index.
        transfer: usize,
    },
    /// Link utilization over the run, percent.
    LinkUtilizationPercent,
    /// The busiest slave PDI, percent.
    PdiBusyPercentMax,
    /// The fullest ESC RAM, percent of what is available.
    DpramUsedPercentMax,
}

impl Metric {
    /// Short label with unit.
    pub fn label(&self) -> String {
        match self {
            Self::TransferTimeS { transfer } => format!("transfer {transfer} time (s)"),
            Self::ExchangeTimeS { transfer } => format!("transfer {transfer} message span (s)"),
            Self::MaxTransferTimeS => "longest transfer (s)".into(),
            Self::GoodputMbps { transfer } => format!("transfer {transfer} goodput (Mbit/s)"),
            Self::LinkUtilizationPercent => "link utilization (%)".into(),
            Self::PdiBusyPercentMax => "busiest PDI (%)".into(),
            Self::DpramUsedPercentMax => "fullest ESC RAM (%)".into(),
        }
    }
}

/// Minimize or maximize.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Goal {
    /// Smaller is better.
    #[default]
    Minimize,
    /// Larger is better.
    Maximize,
}

/// One objective.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Objective {
    /// What.
    pub metric: Metric,
    /// Which way.
    #[serde(default)]
    pub goal: Goal,
}

/// A bound every acceptable run must respect.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Constraint {
    /// What.
    pub metric: Metric,
    /// Upper bound.
    #[serde(default)]
    pub max: Option<f64>,
    /// Lower bound.
    #[serde(default)]
    pub min: Option<f64>,
}

/// A number read from an analysed capture (the measurement side of a calibration).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CaptureMetric {
    /// Duration of the n-th FoE transfer in the capture, s.
    FoeDurationS {
        /// Index in the report's FoE list.
        index: usize,
    },
    /// Goodput of the n-th FoE transfer, Mbit/s.
    FoeGoodputMbps {
        /// Index.
        index: usize,
    },
    /// Duration of the n-th SDO transfer, s.
    SdoDurationS {
        /// Index in the report's SDO list.
        index: usize,
    },
    /// Duration of the n-th EoE frame's fragment sequence, s.
    EoeFrameDurationS {
        /// Index in the report's EoE frame list.
        index: usize,
    },
    /// Period of cyclic stream 0, µs.
    CyclePeriodUs,
}

/// A measured value the model should reproduce.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Target {
    /// The simulated quantity.
    pub metric: Metric,
    /// The measured value, given directly…
    #[serde(default)]
    pub measured: Option<f64>,
    /// …or read from the capture the study is run against.
    #[serde(default)]
    pub from_capture: Option<CaptureMetric>,
    /// Weight in the fit error.
    #[serde(default = "one")]
    pub weight: f64,
}

fn one() -> f64 {
    1.0
}
