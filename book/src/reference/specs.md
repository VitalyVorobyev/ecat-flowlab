# Input files

FlowLab reads four JSON specs (what to generate, simulate, plan or study), a mailbox hint on
the command line, and two kinds of ETG XML (network configuration and device descriptions).
This page lists their fields, defaults and variants. What each tool does with them is in the
[user guide](../guide/cli.md) and the workspace pages linked below.

## Overview

| Input | Read by | Schema | Example in the repo |
|---|---|---|---|
| `SynthSpec` | `flowlab synth`, `POST /api/synth` | `flowlab schema synth` | `configs/synth/*.json` |
| `SimSpec` | `flowlab simulate`, `what-if`, `study` (the base spec), `master transfer --like`, `POST /api/simulate`, `/api/sim/run`, `/api/sim/what-if` | `flowlab schema sim` | `configs/sim/example.json`, `configs/studies/bulk-transfer/{smoke,large-payload,mixed-load}.json` |
| `PlanSpec` | `flowlab plan`, `POST /api/plan` | `flowlab schema plan` | built in (`flowlab plan` without a file, `GET /api/plan/example`) |
| `StudySpec` | `flowlab study`, `POST /api/study` | `flowlab schema study` | `configs/studies/bulk-transfer/foe-sweep.json` |
| `LabInputs` | `scripts/bulk-transfer-report.py --lab`; written by `flowlab pdi-fit --lab`, `flowlab policy --lab` | `flowlab schema lab` | — |
| PDI timing samples (CSV) | `flowlab pdi-fit` (written by `flowlab pdi-bench`) | — | below |
| Mailbox hint | `analyze --mailbox`, `serve --mailbox` | — | `0x1001=0x1000,0x1080` |
| ENI XML | `analyze --eni`, `serve --eni`, `study --eni`, `POST /api/captures/{id}/eni` | — | written by `flowlab synth --eni` |
| ESI XML | `flowlab esi`, `POST /api/esi` | — | `crates/flowlab-devices/tests/fixtures/flowlab-demo.esi.xml` |

Rules common to the four JSON specs:

- `flowlab schema <synth|sim|plan|study>` prints a JSON Schema (draft 2020-12) generated from
  the Rust structs with `schemars`; the server returns the same from `GET /api/<name>/schema`
  (`synth`, `sim`, `plan`, `study`). The UI's forms are rendered from these schemas.
- Every struct and tagged enum is declared `#[serde(deny_unknown_fields)]`: a misspelt or
  unsupported field is an error, not silently ignored.

  ```text
  $ flowlab synth bad.json -o bad.pcapng
  error: unknown field `cycle_time_ns`, expected one of `seed`, `description`, `slaves`, `cycle_time_us`, `cycles`, `tx_jitter_ns`, `slave_delay_ns`, `include_startup`, `sdo_uploads_per_slave`, `distributed_clocks`, `start_unix_s`, `mailbox`, `faults` at line 1 column 83
  ```

- Fields without a default are required. Units are in the field names: `_us`, `_ns`, `_ms`,
  `_s` (time), `_bytes`, `_bps` (bit/s), `_percent`. Payload sizes are decimal: 10 MB is
  10,000,000 bytes, 10 MiB is 10,485,760 bytes.
- Enums with data are internally tagged by `kind` in `snake_case`, e.g.
  `{ "kind": "fixed_quota", "accesses_per_cycle": 2 }`. Plain enums are `snake_case` strings,
  e.g. `"master_to_slave"`.
- After parsing, `SynthSpec` and `SimSpec` are range-checked by their `validate()`; a study
  checks its own fields and turns an invalid run spec into an infeasible run. `PlanSpec` is not
  rejected; the planner reports problems as checks.

## SynthSpec

Input of the synthetic capture generator ([flowlab-synth](../workspace/flowlab-synth.md)).
Every value is chosen, not measured; the capture is labelled `synthetic`.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `seed` | u64 | required | PRNG seed; the same spec gives a byte-identical capture |
| `description` | string | `""` | stored in the pcapng section comment |
| `slaves` | `SlaveSpec[]` | required | slaves in ring order, at least one |
| `cycle_time_us` | u32 | required | nominal cycle time of the cyclic frame, µs (> 0) |
| `cycles` | u32 | required | cycles in the operational phase |
| `tx_jitter_ns` | u32 | 0 | standard deviation of the master's send-time jitter, ns (clamped to ±40 % of a cycle) |
| `slave_delay_ns` | u32 | 700 | forwarding delay per slave, both directions together, ns |
| `include_startup` | bool | `true` | emit the start-up sequence before the cyclic phase |
| `sdo_uploads_per_slave` | u32 | 2 | expedited SDO uploads per mailbox slave in PREOP |
| `distributed_clocks` | bool | `true` | add a DC system-time FRMW to every cycle |
| `start_unix_s` | u64 | 1767225600 (2026-01-01T00:00:00Z) | Unix time of the first frame, s |
| `mailbox` | `MailboxSpec` | see below | mailbox traffic beyond the expedited uploads |
| `faults` | `FaultSpec` | all 0 | fault probabilities per cycle |

`SlaveSpec`:

| Field | Type | Default | Meaning |
|---|---|---|---|
| `name` | string | required | display name |
| `outputs_bytes` | u16 | 0 | output process data (master → slave), bytes |
| `inputs_bytes` | u16 | 0 | input process data (slave → master), bytes |
| `mailbox` | bool | `true` | the slave has a CoE mailbox |

`MailboxSpec` (PREOP items run per mailbox slave during start-up; OP items on the first or last
mailbox slave):

| Field | Type | Default | Meaning |
|---|---|---|---|
| `response_delay_polls` | u32 | 2 | empty polls before a slave's response is readable (≤ 1000) |
| `segmented_upload_bytes` | u32 | 0 | PREOP upload of this many bytes, segmented beyond 112 bytes; 0 = none |
| `download_bytes` | u32 | 0 | PREOP download, expedited up to 4 bytes, else normal/segmented; 0 = none |
| `aborted_uploads` | u32 | 0 | PREOP uploads of a missing object, aborted by the slave |
| `foe_write_bytes` | u32 | 0 | OP FoE write of this many bytes (≤ 16,000,000); 0 = none |
| `eoe_frames_to_slave` | u32 | 0 | OP Ethernet frames tunnelled by EoE to the slave |
| `eoe_frames_from_slave` | u32 | 0 | OP Ethernet frames tunnelled by EoE from the slave |
| `eoe_frame_bytes` | u32 | 590 | size of each tunnelled frame, 60–1514 bytes |
| `emergencies` | u32 | 0 | OP CoE emergencies |

`FaultSpec` (probabilities in [0, 1]):

| Field | Type | Default | Meaning |
|---|---|---|---|
| `lost_frame_probability` | f64 | 0 | the cyclic frame does not come back |
| `wkc_error_probability` | f64 | 0 | the cyclic frame returns with a too-low LRW working counter |
| `skipped_cycle_probability` | f64 | 0 | the master sends nothing in that cycle |
| `mailbox_write_reject_probability` | f64 | 0 | a mailbox write finds the mailbox full and is retried (must be < 1) |

`validate()` also rejects more than 0xEFFF slaves, a process image above 1400 bytes and SDO
sizes above 1,000,000 bytes. `configs/synth/healthy-250us.json`, trimmed to four of its twelve
slaves:

```json
{
  "seed": 1,
  "description": "Fault-free 250 us cycle with 12 compact I/O slaves",
  "slaves": [
    { "name": "coupler", "mailbox": false },
    { "name": "di-1", "inputs_bytes": 2, "mailbox": false },
    { "name": "ai-1", "inputs_bytes": 8 },
    { "name": "drive", "inputs_bytes": 20, "outputs_bytes": 16 }
  ],
  "cycle_time_us": 250,
  "cycles": 20000,
  "tx_jitter_ns": 1500
}
```

`configs/synth/demo.json` adds `mailbox` and `faults` objects; `jittery-host.json` sets a large
`tx_jitter_ns` and frequent skipped cycles.

## SimSpec

Input of the discrete-event simulator ([flowlab-sim](../workspace/flowlab-sim.md)). Every value
is a model parameter; results are labelled `simulated`.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `seed` | u64 | required | seed for frame loss and master jitter |
| `description` | string | `""` | stored in the capture comment and the result |
| `link_bps` | u64 | 100000000 | link rate, bit/s (≥ 1,000,000) |
| `cycle_time_us` | u32 | required | master cycle time, µs (> 0) |
| `master_jitter_ns` | u32 | 0 | standard deviation of the cycle start jitter, ns (clamped to ±40 % of a cycle) |
| `slaves` | `SimSlave[]` | required | slaves in ring order |
| `mailbox_policy` | `MailboxPolicy` | `fixed_quota`, 2 accesses | how the master schedules mailbox datagrams |
| `transfers` | `TransferSpec[]` | required | at least one; transfers on the same slave run one after another in this order |
| `max_duration_ms` | u64 | 60000 | stop after this much virtual time, ms |
| `faults` | `SimFaults` | no loss | fault injection |
| `capture` | `CaptureSpec` | enabled, 200,000 frames | what to write into the capture |
| `start_unix_s` | u64 | 1767225600 | Unix time of the first frame, s |

`SimSlave`:

| Field | Type | Default | Meaning |
|---|---|---|---|
| `name` | string | required | display name |
| `outputs_bytes` | u16 | 0 | cyclic output process data, bytes, besides any PDO channel |
| `inputs_bytes` | u16 | 0 | cyclic input process data, bytes, besides any PDO channel |
| `forwarding_delay_ns` | u32 | 700 | forwarding delay, both directions together, ns |
| `mailbox_out_bytes` | u16 | 0 | "mailbox out" SyncManager size, bytes; 0 = no mailbox |
| `mailbox_in_bytes` | u16 | 0 | "mailbox in" SyncManager size, bytes |
| `dpram_bytes` | u32 | 4096 | ESC RAM available to SyncManagers, bytes |
| `pdi` | `PdiSpec` | see below | the slave application's access to the ESC |
| `app_processing_ns` | u32 | 20000 | application time per message or process-data update, ns |

`PdiSpec` — when `pdi` is omitted, all four defaults apply; when it is given, the first three
fields are required:

| Field | Type | Default | Meaning |
|---|---|---|---|
| `read_bps` | u64 | 20000000 | effective read rate ESC → application, bit/s (> 0) |
| `write_bps` | u64 | 20000000 | effective write rate application → ESC, bit/s (> 0) |
| `access_overhead_ns` | u32 | 5000 | fixed cost per access, ns |
| `mailbox_access` | `"whole_buffer"` \| `"message_only"` | `"whole_buffer"` | one access over the whole SyncManager, or only the message (three accesses to read, two to write) |

`MailboxPolicy` (`kind`):

| `kind` | Fields | Per cycle |
|---|---|---|
| `fixed_quota` | `accesses_per_cycle: u32` (> 0) | up to that many mailbox datagrams, round-robin across slaves, in one frame after the cyclic frame |
| `round_robin` | `slaves_per_cycle: u32` (> 0) | that many slaves with mailbox work get one write and one read |
| `opportunistic` | `bus_budget_percent: f64` (0–100), `turnaround_ns: u32` | after each mailbox frame returns, another one if it would start within that share of the cycle |

`TransferSpec`:

| Field | Type | Default | Meaning |
|---|---|---|---|
| `slave` | usize | required | index into `slaves` |
| `direction` | `"master_to_slave"` \| `"slave_to_master"` | required | direction of the payload |
| `payload_bytes` | u64 | required | payload, bytes (≤ 1,000,000,000) |
| `protocol` | `TransferProtocol` | required | how the payload crosses the bus |
| `start_ms` | f64 | 0 | earliest start after the first cycle, ms |

`TransferProtocol` (`kind`):

| `kind` | Fields | Meaning |
|---|---|---|
| `eoe` | `mtu_bytes: u16` (default 1500, 68–1500) | UDP/IPv4 datagrams tunnelled by EoE (no TCP); mailboxes ≥ 42 bytes |
| `foe` | — | FoE write (master → slave) or read (slave → master) |
| `coe_segmented` | — | CoE SDO download or upload, normal transfer with segments |
| `pdo_channel` | `bytes_per_cycle: u16` (> 8), `window: u32` (default 2, ≥ 1), `retransmit_after_cycles: u32` (default 8) | sequence-numbered chunks in cyclic process data with cumulative acknowledgements and go-back-N; one per slave |

`SimFaults` has `frame_loss_probability` (f64, default 0, in [0, 1)). `CaptureSpec` has
`enabled` (bool, default `true`) and `max_frames` (u64, default 200,000; the simulation goes on
after the capture stops). The full list of validation rules, including the ESC RAM check, is in
[flowlab-sim § Validation](../workspace/flowlab-sim.md#validation).

`configs/sim/example.json` with the defaults left out and two of its four transfers:

```json
{
  "seed": 1,
  "description": "One bulk slave: FoE and a PDO channel, 256 kB each, 1 ms cycle",
  "cycle_time_us": 1000,
  "master_jitter_ns": 2000,
  "slaves": [
    { "name": "coupler" },
    { "name": "io-16", "outputs_bytes": 2, "inputs_bytes": 2 },
    { "name": "bulk-endpoint", "outputs_bytes": 8, "inputs_bytes": 8,
      "mailbox_out_bytes": 512, "mailbox_in_bytes": 512 }
  ],
  "mailbox_policy": { "kind": "fixed_quota", "accesses_per_cycle": 2 },
  "transfers": [
    { "slave": 2, "direction": "master_to_slave", "payload_bytes": 256000,
      "protocol": { "kind": "foe" } },
    { "slave": 2, "direction": "master_to_slave", "payload_bytes": 256000,
      "protocol": { "kind": "pdo_channel", "bytes_per_cycle": 256 } }
  ]
}
```

## PlanSpec

Input of the bus-budget planner ([flowlab-devices § The planner](../workspace/flowlab-devices.md#the-planner)),
which does arithmetic only.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `cycle_time_us` | u32 | required | cycle time, µs |
| `link_bps` | u64 | 100000000 | link rate, bit/s |
| `distributed_clocks` | bool | `true` | the cyclic frame carries a DC system-time FRMW |
| `min_idle_percent` | f64 | 25.0 | warn when less than this share of the cycle is left after the cyclic frames, percent |
| `slaves` | `PlanSlave[]` | required | slaves in ring order |

`PlanSlave`:

| Field | Type | Default | Meaning |
|---|---|---|---|
| `name` | string | required | name, used as the subject of checks |
| `outputs_bytes`, `inputs_bytes` | u16 | 0 | process data, bytes |
| `forwarding_delay_ns` | u32 | 700 | forwarding delay, both directions together, ns |
| `mailbox_out_bytes` | u16 | 0 | "mailbox out" size, bytes; 0 = no mailbox |
| `mailbox_in_bytes` | u16 | 0 | "mailbox in" size, bytes |
| `dpram_bytes` | u32 | 4096 | ESC RAM available to SyncManagers, bytes |
| `supports` | string[] \| null | `null` | mailbox protocols the device declares in its ESI; `null` = unknown |
| `requires` | string[] | `[]` | mailbox protocols the application needs, compared case-insensitively with `supports` |
| `max_mailbox_bytes` | u16 \| null | `null` | largest mailbox the device allows (ESI `MaxSize`), bytes |

`POST /api/esi` returns one `PlanSlave` per ESI device with `supports` and `max_mailbox_bytes`
filled in. This file gives the same result as the built-in example (`flowlab plan`):

```json
{
  "cycle_time_us": 1000,
  "slaves": [
    { "name": "coupler" },
    { "name": "DI-16", "inputs_bytes": 2 },
    { "name": "DO-16", "outputs_bytes": 2 },
    { "name": "Servo-1", "outputs_bytes": 12, "inputs_bytes": 16,
      "mailbox_out_bytes": 128, "mailbox_in_bytes": 128, "requires": ["CoE"] },
    { "name": "Servo-2", "outputs_bytes": 12, "inputs_bytes": 16,
      "mailbox_out_bytes": 128, "mailbox_in_bytes": 128, "requires": ["CoE"] },
    { "name": "bulk-endpoint", "outputs_bytes": 8, "inputs_bytes": 8,
      "mailbox_out_bytes": 512, "mailbox_in_bytes": 512, "requires": ["EoE", "FoE"] }
  ]
}
```

## StudySpec

A parameter study over the simulator ([flowlab-study](../workspace/flowlab-study.md)): variations
of one `SimSpec`, objectives, constraints and, for a calibration, measured targets.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `description` | string | `""` | free text |
| `base` | `SimSpec` | required | the spec every run starts from; its `capture` is ignored (studies write no frames) |
| `parameters` | `Parameter[]` | required | what varies; at least one |
| `sampling` | `Sampling` | `grid` | how values are combined |
| `objectives` | `Objective[]` | `[]` | what to optimize; the Pareto frontier is over these |
| `constraints` | `Constraint[]` | `[]` | what every acceptable run must satisfy |
| `targets` | `Target[]` | `[]` | measured values to fit; when present the first objective is the fit error |
| `max_runs` | u32 | 400 | upper bound on runs; a larger grid is refused |

`Parameter`:

| Field | Type | Default | Meaning |
|---|---|---|---|
| `path` | string | required | JSON pointer into `base`, e.g. `/slaves/2/pdi/read_bps`, `/mailbox_policy` |
| `also` | string[] | `[]` | further pointers that take the same value |
| `values` | JSON[] | `[]` | explicit values of any JSON type; take precedence over the range |
| `min`, `max` | f64 \| null | `null` | range bounds |
| `steps` | u32 \| null | `null` | grid points in the range (≥ 2) |
| `integer` | bool | `false` | round range values to integers |
| `log` | bool | `false` | space range values evenly on a log scale |

`Sampling` (`kind`): `grid` (every combination) or `latin_hypercube` with `samples: u32` and
`seed: u64`.

`Metric` (`kind`), read from each run's `SimResult`:

| `kind` | Fields | Value |
|---|---|---|
| `transfer_time_s` | `transfer` | duration of one transfer, s |
| `exchange_time_s` | `transfer` | span of the transfer's mailbox messages at the master, s (comparable with a capture) |
| `max_transfer_time_s` | — | the longest transfer, s |
| `goodput_mbps` | `transfer` | goodput of one transfer, Mbit/s |
| `link_utilization_percent` | — | link utilization over the run, percent |
| `pdi_busy_percent_max` | — | the busiest slave PDI, percent |
| `dpram_used_percent_max` | — | the fullest ESC RAM, percent of what is available |

`Objective` is `{ metric, goal }` with `goal` `"minimize"` (default) or `"maximize"`.
`Constraint` is `{ metric, max, min }` with optional bounds. `Target` is
`{ metric, measured, from_capture, weight }`: the measured value given directly (`measured`)
or read from the capture the study runs against (`from_capture`), with `weight` (default 1.0)
in the fit error.

`CaptureMetric` (`kind`), read from the analysed capture:

| `kind` | Fields | Value |
|---|---|---|
| `foe_duration_s` | `index` | duration of the n-th FoE transfer in the report, s |
| `foe_goodput_mbps` | `index` | goodput of the n-th FoE transfer, Mbit/s |
| `sdo_duration_s` | `index` | duration of the n-th SDO transfer, s |
| `eoe_frame_duration_s` | `index` | duration of the n-th EoE frame's fragment sequence, s |
| `cycle_period_us` | — | period of cyclic stream 0, µs |

A shortened form of `configs/studies/bulk-transfer/foe-sweep.json` (defaults left out, the PDI
rate given as a log range instead of a value list, the policy parameter dropped):

```json
{
  "description": "FoE write: mailbox size × PDI rate",
  "base": {
    "seed": 1,
    "cycle_time_us": 1000,
    "master_jitter_ns": 2000,
    "slaves": [
      { "name": "coupler" },
      { "name": "io-16", "outputs_bytes": 2, "inputs_bytes": 2 },
      { "name": "bulk-endpoint", "outputs_bytes": 8, "inputs_bytes": 8,
        "mailbox_out_bytes": 512, "mailbox_in_bytes": 512 }
    ],
    "transfers": [
      { "slave": 2, "direction": "master_to_slave", "payload_bytes": 100000,
        "protocol": { "kind": "foe" } }
    ]
  },
  "parameters": [
    { "path": "/slaves/2/mailbox_out_bytes", "also": ["/slaves/2/mailbox_in_bytes"],
      "values": [128, 256, 512, 1024] },
    { "path": "/slaves/2/pdi/read_bps", "also": ["/slaves/2/pdi/write_bps"],
      "min": 5000000, "max": 80000000, "steps": 3, "log": true, "integer": true }
  ],
  "objectives": [
    { "metric": { "kind": "transfer_time_s", "transfer": 0 } },
    { "metric": { "kind": "link_utilization_percent" } }
  ],
  "constraints": [
    { "metric": { "kind": "dpram_used_percent_max" }, "max": 100.0 }
  ]
}
```

A calibration adds targets and is run against a capture
(`flowlab study SPEC --capture CAPTURE`, or `POST /api/study?capture=ID`). Pair a capture's
mailbox durations with `exchange_time_s`, which both sides observe:

```json
{
  "targets": [
    { "metric": { "kind": "exchange_time_s", "transfer": 0 },
      "from_capture": { "kind": "foe_duration_s", "index": 0 } }
  ]
}
```

## Lab inputs

`LabInputs` carries measured values that replace the simulator's placeholders for one slave and
the master. Every field is optional; `flowlab pdi-fit --lab` and `flowlab policy --lab` create
the file or update their part of it and append to `sources`. Both take only measured data: samples
marked `simulated` and synthetic or simulated captures are refused, and the file stays as it was.

| Field | Meaning |
|---|---|
| `description` | what was measured, on what |
| `pdi` | `read_bps`, `write_bps`, `access_overhead_ns`, optionally `mailbox_access` (`"whole_buffer"` or `"message_only"`), as in `SimSpec` |
| `app_processing_ns` | time the slave application takes per mailbox message |
| `mailbox_policy` | a `SimSpec` mailbox policy, e.g. `{ "kind": "fixed_quota", "accesses_per_cycle": 2 }` |
| `sources` | where each value came from: file, checksum, date, setup |

```json
{
  "description": "Verdin iMX8M Plus, LAN9252 on SPI, kernel driver, 2026-11",
  "pdi": { "read_bps": 16002244, "write_bps": 21991594, "access_overhead_ns": 11492 },
  "mailbox_policy": { "kind": "fixed_quota", "accesses_per_cycle": 2 },
  "sources": ["pdi: flowlab pdi-fit spi.csv (sha256 2ae72dab51e921c8)"]
}
```

## PDI timing samples

One timed ESC access by the slave application per line, after the header
`direction,bytes,duration_ns`. `direction` is `read` or `write` (`r`, `w`); `bytes` is what the
access moved; `duration_ns` is the wall time around the driver call. Blank lines and lines
starting with `#` are ignored, except a provenance marker: a comment containing
`flowlab:provenance=simulated` (or `synthetic`) makes the fit carry that provenance, and
`pdi-fit --lab` then refuses to store it. Files without a marker are taken as measured.
`flowlab pdi-bench` writes this format, with the setup in comment lines and the marker
(`measured` on hardware, `simulated` with `--sim`).

```text
# Verdin, LAN9252, spidev at 30 MHz
direction,bytes,duration_ns
read,6,17010
read,1024,525970
write,128,55400
```

Each direction needs at least two different access sizes; spread them over what the application
does (mailbox headers, whole mailboxes, process data). `flowlab pdi-fit` takes the median duration
per size, so occasional preempted calls do not move the fit, and fits a line through the
medians: the intercept is `access_overhead_ns`, the slope gives the effective rate. Its result
(`--result`) lists, per size, the median, the 95th percentile, the fitted value and the relative
error. A large error at some sizes means a straight line does not describe the driver, for
example because long accesses are split into chunks; then time the driver at the sizes the
application actually uses.

## Mailbox hints

A hint gives the mailbox start addresses of a station whose SyncManager configuration is not in
the capture (for example a capture started after the master configured the network):

```text
--mailbox STATION=OUT,IN
--mailbox 0x1001=0x1000,0x1080
```

`STATION` is the configured station address, `OUT` the start of the master → slave mailbox
(SM0 by convention), `IN` the start of the slave → master mailbox (SM1 by convention). Each
number is a 16-bit value, decimal or `0x` hex (the `#x` form of ETG XML is not accepted). The
option is repeatable on `flowlab analyze` and `flowlab serve`; `serve` applies it to every
capture. A hint carries no lengths. It is used only for a station whose mailbox is not already
known from SyncManager configuration in the capture or from an ENI, and the affected channels
report `source: "hint"`. A malformed hint is rejected by the argument parser:

```text
$ flowlab analyze demo.pcapng --mailbox 0x1001=0x1000
error: invalid value '0x1001=0x1000' for '--mailbox <STATION=OUT,IN>': expected STATION=OUT,IN
```

## ENI and ESI XML

Both are read by subset readers in [flowlab-devices](../workspace/flowlab-devices.md), which
lists every element they use. Common rules:

- XML is parsed with `roxmltree`; documents with a DTD are rejected. The root element must be
  `<EtherCATConfig>` (ENI) or `<EtherCATInfo>` (ESI).
- Only the elements FlowLab uses are read; unknown elements are ignored.
- Numbers may be decimal, `#x` hex (the ETG convention) or `0x` hex.
- Over HTTP the body must be UTF-8.

**ENI** (EtherCAT Network Information, ETG.2100) describes how a master configured one network.
FlowLab reads, under `<Config>`: the master's name; each `<Slave>`'s `<Info>` (`PhysAddr`
required; `AutoIncAddr`, `VendorId`, `ProductCode`, `RevisionNo`, `Name`), the process-data bit
lengths, the `<Mailbox>` `Send`/`Recv` start and length with its `<Protocol>` list; and each
`<Cyclic>` with `CycleTime` (µs) and the `<Frame><Cmd>` entries with their expected working
counter `Cnt`. With an ENI, the analysis judges working counters against `Cnt`, takes mailbox
addresses for stations without SyncManager configuration in the capture, and names stations.
`flowlab synth --eni FILE` writes the configuration of a synthetic network; an excerpt of the
file it writes for `configs/synth/demo.json`:

```xml
<EtherCATConfig Version="1.3">
  <Config>
    <Master>
      <Info>
        <Name>flowlab-synth 0.1.0</Name>
      </Info>
    </Master>
    <Slave>
      <Info>
        <Name>AI-4</Name>
        <PhysAddr>4100</PhysAddr>
        <AutoIncAddr>65533</AutoIncAddr>
      </Info>
      <ProcessData>
        <Recv>
          <BitLength>64</BitLength>
        </Recv>
      </ProcessData>
      <Mailbox DataLinkLayer="true">
        <Send>
          <Start>4096</Start>
          <Length>128</Length>
        </Send>
        <Recv>
          <Start>4224</Start>
          <Length>128</Length>
        </Recv>
        <Protocol>CoE</Protocol>
        <Protocol>FoE</Protocol>
        <Protocol>EoE</Protocol>
      </Mailbox>
    </Slave>
    <Cyclic>
      <CycleTime>1000</CycleTime>
      <Frame>
        <Cmd>
          <State>SAFEOP</State>
          <State>OP</State>
          <Cmd>12</Cmd>
          <Addr>65536</Addr>
          <DataLength>68</DataLength>
          <Cnt>10</Cnt>
        </Cmd>
      </Frame>
    </Cyclic>
  </Config>
</EtherCATConfig>
```

**ESI** (EtherCAT Slave Information, ETG.2000) describes what a device type can do. FlowLab
reads the vendor id and name and, for each `<Descriptions><Devices><Device>` (each needs a
`<Type>`): product code, revision, type name, name, SyncManagers, RxPdo/TxPdo with entries,
the mailbox protocols listed under `<Mailbox>`, and whether `<Dc>` is present. The default
process-data size is the sum of the PDOs with an `Sm` attribute. `flowlab esi FILE` prints one
line per device, `--json` the parsed `EsiFile`:

```text
$ flowlab esi crates/flowlab-devices/tests/fixtures/flowlab-demo.esi.xml
FlowLab Test Devices (vendor 0x00000FFF): 2 devices
  FL-DI16        0x00010001 rev 0x00020000  out    0 B  in    2 B  mailbox    0/0    B  no mailbox  FL-DI16 16-channel digital input
  FL-BULK        0x00020001 rev 0x00010000  out    8 B  in    8 B  mailbox  512/512  B  CoE/FoE/EoE, DC  FL-BULK bulk endpoint
```

The repository holds no vendor ESI or ENI files; the fixtures under
`crates/flowlab-devices/tests/fixtures/` are hand-written.
