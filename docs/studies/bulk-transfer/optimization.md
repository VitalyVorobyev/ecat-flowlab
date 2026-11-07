# Configuration search and optimization

## Problem statement

Find a transport and settings that minimize verified 10 MB transfer completion time or maximize sustainable useful goodput subject to constraints: EtherCAT control traffic must meet its latency/cycle requirement, ESC memory allocation is valid, CPU/queue budgets hold, and the candidate uses genuinely supported master/slave protocol features.

## Search variables

- `transport` (EoE, FoE, VoE, CoE block, custom PDO, ordinary Ethernet reference).
- `cycle_time_us` and background PDO data allocation.
- `mailbox_out_bytes` / `mailbox_in_bytes` (only allowed from device-supported sizes).
- `service_policy`, mailbox polling frequency/budget, queue depths, priority/fairness.
- `app_chunk_bytes`, transmit window, ACK frequency/retry timeout, send pacing.
- `pdo_bytes_per_cycle`, process image configuration and update budget.
- `pdi_access_delay_us`, SPI throughput (model parameter, not automatically tunable).
- `direction` and number of contending devices.

## Objectives (report full vector)

- Maximize **verified application goodput** (byte/s).
- Minimize transfer completion time (`s`) for N=10,000,000 bytes.
- Minimize p95/p99 control-cycle or command latency impact.
- Minimize link utilization, CPU utilization, drop/retry rate.

Compute a Pareto frontier rather than claiming a unique universal 'best'. Require configurable weighted ranking as a secondary convenience.

## Algorithm roadmap

1. Grid sweep over a small matrix and Pareto filtering (deterministic and transparent).
2. Random/Latin-hypercube sampling with reproducible seed if combinatorial explosion.
3. Optional model-guided search only after model validation; compare to sweep.
4. `calibrate` mode uses measured traces to revise distributions and uncertainty.
5. For robust optimization, evaluate best candidates across multiple jitter/error seeds and show confidence intervals.

## Essential validity gates

- Reject configurations that exceed frame payload, mailbox, or ESC memory constraints.
- Reject protocols unsupported by slave firmware, master library or hardware.
- Reject candidates violating background deterministic workload conditions.
- Report feasibility separately from speed; hypothetical research candidates may be tagged `simulation-only`.
- Never silently optimize by reducing data integrity checks or declaring truncated output a success.

## Output example (schema to refine in code)

```json
{
  "schema_version": 1,
  "mode": "simulation",
  "payload_bytes": 10000000,
  "objective": "minimize_completion_time",
  "feasible": true,
  "assumptions": ["synthetic mailbox scheduler"],
  "metrics": {"completion_seconds": null, "app_goodput_bps": null},
  "constraints": {"max_control_jitter_us": null},
  "candidate": {"transport": "eoe", "mailbox_bytes": 1024}
}
```

`null` metric values here mean example schema placeholders, not observations. Do not store invented performance data as if observed.
