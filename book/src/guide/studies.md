# Studies, calibration and comparison

A **study** runs the simulator many times over variations of one spec. It checks constraints,
and reports the Pareto frontier of its objectives. With measured targets it becomes a
**calibration**: the runs that best reproduce a measurement. **Compare** puts two captures
side by side.

```bash
flowlab study configs/studies/bulk-transfer/foe-sweep.json --result study.json
flowlab study calibrate.json --capture lab.pcapng [--eni lab.eni.xml] --result fit.json
flowlab schema study
```

In the UI, the **Study** view has the spec form, a scatter of the runs (Pareto-optimal ones
highlighted), the table of all runs with reasons for rejection, the residuals of a calibration
and *Simulate this run*.

## Parameters

A parameter is a JSON pointer into the base simulator spec, for example `/cycle_time_us`,
`/slaves/2/pdi/read_bps` or `/mailbox_policy`, with explicit values (any JSON, so whole
policies or protocols can vary) or a range (`min`, `max`, `steps`, optionally `integer` and
`log`). `also` lists pointers that take the same value, such as the PDI write rate with the
read rate.

Runs are every combination of the values (**grid**) or a **Latin hypercube** of a given size and
seed. `max_runs` (default 400) guards against an accidental huge grid. Runs execute in
parallel. A spec the simulator rejects, for example because the ESC RAM is too small, is an
infeasible run with its reason, not an error.

## Objectives, constraints, best run

- **Metrics**: transfer time, exchange time (the span a capture shows), the slowest transfer,
  goodput, link utilization, the busiest PDI, the fullest ESC RAM.
- **Objectives**: each a metric to minimise or maximise. The **Pareto frontier** holds the
  feasible runs that no other run beats on every objective. There is no single best
  configuration unless a weighting is chosen, so the frontier is reported in full.
- **Constraints**: minimum and maximum on any metric. Violations make a run infeasible, with the
  reason given.
- **Best run**: the Pareto-optimal run best on the first objective. Pareto-optimal and best runs
  keep their full spec, so they can be simulated again with frames.

## Calibration

A target pairs a simulated metric with a measured value. The value is either given directly,
or read from an analysed capture (`--capture`):

| From the capture | Meaning |
|---|---|
| `foe_duration_s`, `foe_goodput_mbps` | the *n*-th FoE transfer |
| `sdo_duration_s` | the *n*-th SDO transfer |
| `eoe_frame_duration_s` | the *n*-th EoE frame's fragment sequence |
| `cycle_period_us` | the period of cyclic stream 0 |

The fit error, the weighted RMS of the relative errors, becomes the first objective. The best
run's residuals are reported per target. Compare like with like: capture durations are message
spans at the master, so match them with the simulator's `exchange_time_s`, not with
application-level transfer times.

A good fit shows that the model *can* reproduce the measurement with those parameters. It does
not show that the parameters are physically right, because other combinations may fit as well.
The study says so in its assumptions.

## Compare

**Compare** shows the same quantities from two analysed captures:

- frames, duration, cycle period, jitter and round trip;
- losses and WKC mismatches;
- link load;
- mailbox messages, rejected writes and empty reads;
- SDO and FoE figures, EoE frames;
- the ratio B / A.

Each column keeps its provenance badge. A typical pair is a lab measurement and a simulation of
the same network.
