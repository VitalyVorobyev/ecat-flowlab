# flowlab-study

`flowlab-study` runs parameter studies over the simulator. A `StudySpec` varies fields of one
`SimSpec` by JSON pointer, on a grid or a Latin hypercube, runs every variant without writing
frames, checks constraints and finds the Pareto frontier of the objectives. With measured targets
— given as numbers, or read from an analysed capture — the same study is a calibration: the best
run is the one whose simulated values come closest, and the residuals are reported. Every number
in a study result is `simulated`. `what_if` answers a narrower question without a study spec:
which single change (or pair) to one scenario shortens its transfers most.

API: [`flowlab_study`](../../api/flowlab_study/index.html) ·
source: [crates/flowlab-study](https://github.com/VitalyVorobyev/ecat-flowlab/blob/main/crates/flowlab-study)

## Position in the pipeline

| | |
|---|---|
| Depends on | `flowlab-sim` (`simulate`, `SimSpec`, `SimResult`), `flowlab-analysis` (`CaptureReport` for capture targets), `flowlab-core` |
| Dev-depends on | `flowlab-capture` (the calibration test builds a capture) |
| Used by | `flowlab-server` (`POST /api/study`, `POST /api/sim/what-if`), `flowlab-cli` (`flowlab study`, `flowlab what-if`) |

## Main types and functions

| Item | What it is |
|---|---|
| `run_study(&StudySpec, Option<&CaptureReport>) -> Result<StudyResult, StudyError>` | Draw, run and judge every run |
| `what_if(&SimSpec) -> Result<WhatIfResult, StudyError>` | The scenario and its one-change and two-change variants, best first |
| `WhatIfResult`, `WhatIf`, `WhatIfRun`, `SpecEdit`, `Lever` | Serialized what-if results, exported to TypeScript |
| `StudySpec` | `description`, `base: SimSpec`, `parameters`, `sampling`, `objectives`, `constraints`, `targets`, `max_runs` (default 400) |
| `Parameter` | `path` (JSON pointer), `also`, `values`, `min`, `max`, `steps`, `integer`, `log` |
| `Sampling` | `grid` (default) or `latin_hypercube { samples, seed }` |
| `Metric` | A number read from a `SimResult` |
| `Objective` | `metric` + `goal` (`minimize` default, or `maximize`) |
| `Constraint` | `metric` + optional `max` / `min` |
| `lab::parse_samples`, `lab::fit_pdi` → `PdiFit` | PDI timing samples (CSV) → the PDI model's cost per access and effective rates, with residuals per access size; provenance `measured` |
| `lab::mailbox_policy(&ServicePattern) -> MailboxPolicy` | the analysis's service pattern as a simulator policy |
| `lab::LabInputs` (+ `apply(&mut SimSpec, slave)`, `lab::schema()`) | measured values replacing the placeholders of one slave and the master |
| `Target`, `CaptureMetric` | A measured value to fit: `measured` directly, or `from_capture` |
| `StudyResult`, `StudyRun`, `Residual` | Serialized results, exported to TypeScript |
| `sim_metric(&SimResult, &Metric)`, `capture_metric(&CaptureReport, &CaptureMetric)` | The metric readers |
| `spec_schema()` | JSON Schema (draft 2020-12) of `StudySpec` |
| `StudySpec::example()` | A 100 kB FoE write: mailbox size × PDI rate × policy (24 runs); identical to `configs/studies/bulk-transfer/foe-sweep.json` |

## Parameters and sampling

A `Parameter`'s `path` is a JSON pointer into `base`, e.g. `/slaves/2/pdi/read_bps`,
`/cycle_time_us` or `/mailbox_policy` (any JSON value, so a whole policy object can vary). Every
pointer in `also` receives the same value, e.g. the PDI write rate together with the read rate.
`run_study` refuses a study whose pointers are not in the base spec.

Values come from `values` (explicit, any JSON) when given; otherwise from the range `min`–`max`,
spaced linearly or, with `log` and positive bounds, logarithmically, and rounded with `integer`.

| Sampling | Runs |
|---|---|
| `grid` | Every combination of every parameter's values. A range needs `steps` ≥ 2 (consecutive duplicates after rounding are dropped). A grid with more than `max_runs` runs is refused |
| `latin_hypercube { samples, seed }` | `samples` runs (1..=`max_runs`). Each parameter's range or value list is split into `samples` strata, each stratum used once, in a permutation drawn from `seed`; `steps` is not used |

## Metrics, objectives, constraints

| `Metric` | Read from `SimResult` |
|---|---|
| `transfer_time_s { transfer }` | `duration_ns` / 10⁹ |
| `exchange_time_s { transfer }` | `exchange_ns` / 10⁹ — the message span at the master, comparable with a capture |
| `max_transfer_time_s` | The longest transfer |
| `goodput_mbps { transfer }` | `goodput_bps` / 10⁶ |
| `link_utilization_percent` | `link_utilization_percent` |
| `pdi_busy_percent_max` | The busiest slave PDI |
| `dpram_used_percent_max` | The fullest ESC RAM, `dpram_used_bytes / dpram_bytes` |

**Feasibility.** A run is feasible when its spec deserializes and validates, every transfer
finishes verified within `max_duration_ms`, every constraint holds, and every objective is
available. Otherwise `feasible` is false and `reason` says why, e.g. the simulator's validation
error ("… ESC RAM …"), `"transfer 0 did not finish verified within max_duration_ms"` or
`"<metric label> = <value> exceeds <max>"`. An invalid variant is an infeasible run, not a
failed study.

**Pareto frontier.** A feasible run is Pareto-optimal when no other feasible run is at least as
good on every objective and better on one. `best` is the frontier run with the best first
objective (ties to the lower index); for a calibration the first objective is the fit error. With
no objectives and no targets there is no frontier and no best run.

Every run keeps its parameter `values` (JSON text), their `numeric` form for plots, and its
`objectives`. Only Pareto-optimal runs and the best run keep their full simulator `spec`, so they
can be simulated again (the UI's *Study* page does this with frames enabled).

## Calibration

`Target` names a simulated `metric` and the measured value: `measured`, or `from_capture` read
from the capture passed to `run_study`:

| `CaptureMetric` | Read from `CaptureReport` |
|---|---|
| `foe_duration_s { index }` | The *n*-th FoE transfer's end − start |
| `foe_goodput_mbps { index }` | Its goodput |
| `sdo_duration_s { index }` | The *n*-th SDO transfer's end − start |
| `eoe_frame_duration_s { index }` | The *n*-th EoE frame's fragment sequence |
| `cycle_period_us` | Period of cyclic stream 0 |

A target that reads a capture without one given, a metric missing from the capture, or a
measured value of 0 is a `StudyError`. Calibrate on quantities both sides observe: pair a
capture's mailbox durations with `exchange_time_s`, not with `transfer_time_s` (which ends at the
receiving application).

**Fit error.** With targets, the first objective is the weighted RMS relative error

```text
fit = sqrt( Σ wᵢ ((simᵢ − measᵢ) / measᵢ)² / Σ wᵢ )
```

(`weight` defaults to 1). A run where any target is unavailable has no fit error.
`StudyResult::residuals` lists, for the best run, each target's label, source (`"given"` or the
capture metric), measured and simulated values and `(sim − meas) / meas`.
`calibrated_against` names the capture when a target read it. The assumptions state that a small
error shows the model can reproduce these numbers, not that its parameters are the physical ones.

## Lab inputs

The PDI model charges every access a fixed cost plus its bytes at an effective rate.
`fit_pdi` fits exactly that line: per access size the median duration (robust to preempted
calls), then least squares through the medians. Intercept = cost per access, slope = 8 / rate;
the simulator's single cost per access is the access-weighted mean of the read and write
intercepts. Each direction needs two or more sizes; a slope ≤ 0 is an error. A direction without
samples takes the other's values, and the assumptions say so. The fit's provenance is
`measured` unless the samples file carries a `flowlab:provenance=` marker in a comment
(`samples_provenance`); `flowlab pdi-bench --sim` ([flowlab-pdi](flowlab-pdi.md)) writes
`simulated`, and the CLI stores only measured fits as lab inputs.

`mailbox_policy` maps the analysis's `ServicePattern` (see
[flowlab-analysis § Mailbox](flowlab-analysis.md#mailbox)) one to one onto `MailboxPolicy`.
`LabInputs::apply` writes `pdi`, `app_processing_ns` and `mailbox_policy` into a spec; fields
left out keep the spec's values. CLI: `flowlab pdi-fit`, `flowlab policy`, `flowlab schema lab`;
`scripts/bulk-transfer-report.py --lab` uses the file.

## Running

`run_study` copies `base` to JSON, sets `/capture/enabled` to false (studies write no frames),
draws the points, and evaluates them on `std::thread::available_parallelism()` scoped threads in
contiguous chunks. Runs are independent and deterministic and keep their order, so a study result
does not depend on the thread count. `meta` has provenance `simulated`, the study spec's SHA-256
and the assumptions.

## What-if

`what_if` derives its variants from the scenario, so it needs no study spec. For the master: the
cycle halved, and the mailbox policy's parameter doubled or a switch to the opportunistic
policy (60 % of the cycle, 10 µs turnaround). For each slave that carries a transfer: PDI read
and write rate doubled, overhead per access halved, message-only mailbox access, mailboxes
doubled (capped at `MAX_MAILBOX_BYTES`, one frame), application time halved. For each transfer:
every other protocol (a PDO channel at 256 B per cycle), EoE MTU 1500 B, PDO channel bytes per
cycle and window doubled. Pairs of the first eight changes that are not protocol switches and
set different values are run too, because two limits can coincide: then neither change helps
alone and the pair does.

Every variant is a `SpecEdit` list (JSON pointer + value) applied to the scenario as JSON, run
on the same thread pool as a study, and summarised as a `WhatIfRun`: feasibility with its
reason (an invalid spec, or a transfer not finished verified), duration per transfer, when the
last finished, link load, busiest PDI and fullest ESC RAM. Feasible variants come first, by
finish time. The edits are what the UI applies, so applying them reproduces the run.

## Invariants and boundaries

- Runs simulate without captures.
- Parameters are JSON pointers into the spec; no study-specific code per field.
- Calibration only on quantities both a capture and the simulator observe; residuals are always
  reported.

## Tests

`crates/flowlab-study/src/tests.rs`:

| Test | Checks |
|---|---|
| `example_grid_has_a_consistent_pareto_frontier` | 24 runs, all feasible; no frontier run is dominated; frontier runs keep their spec; the fastest run is best; larger mailboxes and a faster PDI never slow the FoE write |
| `constraints_reject_runs_with_their_reason` | An ESC RAM constraint excludes 1024-byte mailboxes with the reason; an invalid variant is an infeasible run |
| `sampling_rules` | `max_runs` refusal; Latin hypercube uses each value once and is deterministic; unknown pointer refused; log/integer range gives 10⁶, 10⁷, 10⁸ |
| `calibration_recovers_hidden_pdi_parameters_from_a_capture` | A "measurement" simulated with hidden PDI rate and overhead, analysed as a capture; the grid's best run recovers both, fit error and residuals < 10⁻⁹, `calibrated_against` is the capture |
| `shipped_study_is_the_example` | `foe-sweep.json` equals `StudySpec::example()` |
| `what_if_ranks_one_change_runs_and_its_edits_reproduce_them` | Feasible variants first, by finish; the best one is faster than the scenario and its edits alone reproduce its run; every lever is tried; slaves without transfers are left alone |
| `what_if_reports_why_a_change_does_not_fit` | Doubled mailboxes that overflow the ESC RAM are an infeasible variant with the reason |
| `analysis_reads_the_master_policy_back_from_a_simulated_capture` | A simulated FoE transfer with `fixed_quota` 2, then with `opportunistic` 60 % / 10 µs, captured and analysed: the service pattern gives back fixed quota 2, and back-to-back polling with a budget of 50–60 % and a 10 µs turnaround |
| `lab::tests::fit_recovers_rate_and_overhead_despite_outliers` | Samples on known lines with spread and one preempted and one early call per size: rates and costs recovered exactly; combined cost is the access-weighted mean |
| `lab::tests::bad_samples_are_rejected_with_their_line`, `lab_inputs_replace_placeholders` | Header, column and direction errors name the line; one size cannot be fitted; `apply` sets the slave's PDI, application time and the policy |
| `lab::tests::a_provenance_marker_labels_the_fit` | A `# flowlab:provenance=simulated` comment makes the fit simulated, with an assumption saying so; no marker means measured |

## Example

```rust,ignore
use flowlab_study::{StudySpec, run_study};

let result = run_study(&StudySpec::example(), None)?;
for &i in &result.pareto {
    let run = &result.runs[i as usize];
    println!("{:?} -> {:?}", run.values, run.objectives);
}
```

```bash
cargo run --release -p flowlab-cli -- study configs/studies/bulk-transfer/foe-sweep.json
```

See [Studies, calibration and comparison](../guide/studies.md) for the workflow.
