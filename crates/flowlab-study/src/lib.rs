//! Parameter studies over the simulator.
//!
//! A [`StudySpec`] varies parameters of one [`flowlab_sim::SimSpec`] (by JSON pointer), on a
//! grid or a Latin hypercube, runs every variant, checks constraints and finds the Pareto
//! frontier of the objectives. With measured targets — given, or read from an analysed
//! capture — it is a calibration: the best run is the one whose simulated values come closest,
//! and the residuals are reported.
//!
//! Every number is a simulation result; a calibration says how well the model can reproduce
//! a measurement, not that its parameters are the physical ones.
//!
//! ```no_run
//! use flowlab_study::{StudySpec, run_study};
//!
//! // Mailbox size × PDI rate × master policy for a 100 kB FoE write (24 simulations).
//! let result = run_study(&StudySpec::example(), None).unwrap();
//! for &i in &result.pareto {
//!     let run = &result.runs[i as usize];
//!     println!("{:?} → {:?}", run.values, run.objectives);
//! }
//! ```

pub mod lab;
mod run;
mod spec;
mod whatif;

pub use lab::{LabInputs, PdiFit, fit_pdi, mailbox_policy, parse_samples, samples_provenance};
pub use run::{Residual, StudyResult, StudyRun, capture_metric, run_study, sim_metric};
pub use spec::{CaptureMetric, Constraint, Goal, Metric, Objective, Parameter, Sampling, StudySpec, Target};
pub use whatif::{Lever, SpecEdit, WhatIf, WhatIfResult, WhatIfRun, what_if};

/// Why a study could not run.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum StudyError {
    /// The study spec is inconsistent.
    #[error("invalid study: {0}")]
    Spec(String),
}

/// JSON Schema of [`StudySpec`] (draft 2020-12).
pub fn spec_schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(StudySpec)).unwrap_or_default()
}

impl StudySpec {
    /// FoE write of 100 kB: how mailbox size, PDI rate and policy trade off transfer time
    /// against link load and ESC RAM.
    pub fn example() -> Self {
        let mut base = flowlab_sim::SimSpec::example();
        base.description = "Study base: one FoE write of 100 kB at 1 ms".into();
        base.transfers.truncate(2);
        base.transfers.remove(0);
        base.transfers[0].payload_bytes = 100_000;
        let p = |path: &str, values: Vec<serde_json::Value>| Parameter {
            path: path.into(),
            also: Vec::new(),
            values,
            min: None,
            max: None,
            steps: None,
            integer: false,
            log: false,
        };
        Self {
            description: "FoE write: mailbox size × PDI rate × policy".into(),
            base,
            parameters: vec![
                Parameter {
                    also: vec!["/slaves/2/mailbox_in_bytes".into()],
                    ..p("/slaves/2/mailbox_out_bytes", [128, 256, 512, 1024].map(serde_json::Value::from).to_vec())
                },
                Parameter {
                    also: vec!["/slaves/2/pdi/write_bps".into()],
                    ..p(
                        "/slaves/2/pdi/read_bps",
                        [5_000_000, 20_000_000, 80_000_000].map(serde_json::Value::from).to_vec(),
                    )
                },
                p(
                    "/mailbox_policy",
                    vec![
                        serde_json::json!({ "kind": "fixed_quota", "accesses_per_cycle": 2 }),
                        serde_json::json!({ "kind": "opportunistic", "bus_budget_percent": 60.0, "turnaround_ns": 10000 }),
                    ],
                ),
            ],
            sampling: Sampling::Grid,
            objectives: vec![
                Objective { metric: Metric::TransferTimeS { transfer: 0 }, goal: Goal::Minimize },
                Objective { metric: Metric::LinkUtilizationPercent, goal: Goal::Minimize },
            ],
            constraints: vec![Constraint { metric: Metric::DpramUsedPercentMax, max: Some(100.0), min: None }],
            targets: Vec::new(),
            max_runs: 400,
        }
    }
}

#[cfg(test)]
mod tests;
