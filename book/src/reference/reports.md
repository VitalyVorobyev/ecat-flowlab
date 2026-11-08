# Report and result files

Every FlowLab tool writes JSON: the analysis report of a capture, the ground truth of a
synthetic capture, simulation, planning and study results, and parsed ESI files. This page
lists what each file holds. The tables show the fields a reader needs most; the full
definitions are the Rust structs, exported as TypeScript by ts-rs into
[`ui/src/api/gen/`](https://github.com/VitalyVorobyev/ecat-flowlab/tree/main/ui/src/api/gen)
whenever `cargo test` runs.

## Overview

| File | Written by | Rust type | Provenance | TypeScript |
|---|---|---|---|---|
| Analysis report | `flowlab analyze CAPTURE --out FILE` (or `--json` to stdout); `GET /api/captures/{id}/report` | `flowlab_analysis::CaptureReport` | `meta.source`: from the capture | `CaptureReport.ts` |
| Ground truth | `flowlab synth SPEC -o CAPTURE --truth FILE` | `flowlab_synth::GroundTruth` | synthetic (no `meta`) | not exported |
| Simulation result | `flowlab simulate SPEC --result FILE` (an array for several specs); `GET /api/captures/{id}/simulation` | `flowlab_sim::SimResult` | `meta.source`: `simulated` | `SimResult.ts` |
| Plan result | `flowlab plan [SPEC] --json`; `POST /api/plan` | `flowlab_devices::PlanResult` | arithmetic on the spec (no `meta`) | `PlanResult.ts` |
| What-if result | `flowlab what-if SPEC --result FILE`; `POST /api/sim/what-if` | `flowlab_study::WhatIfResult` | `meta.source`: `simulated` | `WhatIfResult.ts` |
| Study result | `flowlab study SPEC --result FILE`; `POST /api/study` | `flowlab_study::StudyResult` | `meta.source`: `simulated` | `StudyResult.ts` |
| ESI summary | `flowlab esi FILE --json`; `POST /api/esi` (as `EsiImport`) | `flowlab_devices::EsiFile` | device description, not a result | `EsiFile.ts`, `EsiImport.ts` |
| PDI fit | `flowlab pdi-fit SAMPLES --result FILE`, `flowlab pdi-bench … --result FILE` | `flowlab_study::PdiFit` | `meta.source`: `measured`, or the samples file's marker (`simulated` from `pdi-bench --sim`) | not exported |
| Lab master scan | `flowlab master scan -o FILE` (`--eni` also writes ENI XML) | `flowlab_master::ScanResult` | `meta.source`: `measured` | not exported |
| Lab master run | `flowlab master run -o FILE` | `flowlab_master::RunResult` | `meta.source`: `measured` | not exported |
| Lab master transfer | `flowlab master transfer -o FILE` | `flowlab_master::MeasuredTransfer` | `meta.source`: `measured` | not exported |

`flowlab analyze --eoe-out FILE` and `GET /api/captures/{id}/eoe.pcapng` also write the
Ethernet frames reassembled from EoE as a pcapng; it carries the provenance of the capture it
came from. The API's own types (`CaptureInfo`, `FramePage`, `FrameDetail`) are in
[HTTP API](api.md).

## Conventions

- **Times.** Every time in a report is in ns relative to the first frame of the capture
  (`t_ns`, `start_ns`, `first_ns` …), so it stays below 2^53 and fits a JavaScript `number`.
  The one absolute time is `summary.start_unix_ms`, ms since the Unix epoch. Simulation times
  are ns since the first simulated frame.
- **Units** are in field names: `_ns`, `_bytes`, `_bps` (bit/s), `_percent`; study metrics in
  s, Mbit/s or µs as their names say.
- **Frame numbers** are 1-based, as in Wireshark.
- **Long lists** (events, SDO transactions, FoE transfers, EoE frames, mailbox errors) hold at
  most 2000 entries; a `_total` field or the counters next to them count all.
- **Distributions.** Every statistic of a sample is a `Distribution` in the unit of the sample
  (ns unless stated): `count`, `min`, `max`, `mean`, `std` (population), `p1`, `p5`, `p50`,
  `p95`, `p99` and a `histogram` (`lo`, `bin_width`, `counts`). It is `null` when there is no
  sample.
- **Percentiles** everywhere (distributions, the lab master's summaries, PDI benchmark and fit)
  interpolate linearly between the two closest ranks, at position *q* · (*n* − 1), so a
  measured, a simulated and a captured p99 are computed the same way.

## ReportMeta

`CaptureReport`, `SimResult` and `StudyResult` begin with `meta` (`flowlab_core::ReportMeta`;
see [Provenance and assumptions](provenance.md)):

| Field | Meaning |
|---|---|
| `schema_version` | `SCHEMA_VERSION` of the report format, currently 1 |
| `tool` | `"ecat-flowlab"` |
| `tool_version` | crate version of the producing tool |
| `source` | `"measured"`, `"simulated"` or `"synthetic"` |
| `input` | `InputRef`: `name`, `bytes`, `sha256` (lower-case hex) of what the result was computed from |
| `assumptions` | sentences a reader must know to interpret the numbers |

For a report, `input` is the capture file (name without directories, size and SHA-256 of its
contents) and `source` comes from the capture: `synthetic` or `simulated` when the pcapng
carries FlowLab's provenance marker, else `measured`. For a simulation, `input` is the spec
serialized as JSON, named `sim-seed<seed>.json`; for a study, the study spec, named
`study.json`. From `flowlab analyze` of the demo capture
(`flowlab synth configs/synth/demo.json -o demo.pcapng`), assumptions shortened to two:

```json
{
  "schema_version": 1,
  "tool": "ecat-flowlab",
  "tool_version": "0.1.0",
  "source": "synthetic",
  "input": {
    "name": "demo.pcapng",
    "bytes": 1787928,
    "sha256": "95d8517efcb8543b965a4543dcd0a8c2fdb1594de799677253a80dae396f813c"
  },
  "assumptions": [
    "Utilization assumes a 100 Mbit/s link; wire bytes include padding to 64 bytes, preamble, FCS and the 12-byte inter-frame gap.",
    "Expected working counters are the most frequent value per datagram position; no ENI network configuration was given."
  ]
}
```

## CaptureReport

The output of `analyze` ([flowlab-analysis](../workspace/flowlab-analysis.md)); the UI's
capture views read the same object.

| Section | Type | Contents |
|---|---|---|
| `meta` | `ReportMeta` | above |
| `summary` | `CaptureSummary` | frame and datagram counts, directions, command counts |
| `streams` | `CyclicStream[]` | detected cyclic streams, most frequent first |
| `utilization` | `Utilization` | link utilization over time |
| `stations` | `StationSummary[]` | per configured station address |
| `al_events` | `AlEvent[]` | AL state requests and reports, changes only |
| `mailbox` | `MailboxReport` | mailbox channels, protocols and transfers |
| `findings` | `Finding[]` | judgements against declared thresholds, and notes |

**`summary`**

| Field | Meaning |
|---|---|
| `format` | `"pcap"` or `"pcapng"` |
| `frames`, `ethercat_frames`, `other_frames`, `malformed_frames` | all frames; well-formed EtherCAT; non-EtherCAT; claiming EtherCAT but not parsing |
| `datagrams` | datagrams in EtherCAT frames |
| `duration_ns`, `start_unix_ms` | first to last frame; absolute time of the first frame |
| `direction_mode` | how TX and RX were told apart: `"mac_local_bit"`, `"wkc_heuristic"` or `"unknown"` |
| `tx_frames`, `rx_frames`, `paired_frames`, `unanswered_frames` | direction counts; TX frames with and without their returning frame |
| `commands` | `{ command, count }` per mnemonic, most frequent first |
| `slaves_responding` | highest broadcast-read WKC on a returning frame: a lower bound on the slaves that answered |

**`streams[]`** (`CyclicStream`)

| Field | Meaning |
|---|---|
| `id`, `signature` | 0 = most frequent; the layout, e.g. `"FRMW 0x1001:0x0910 · LRW 0x00010000 · BRD 0x0130"` |
| `frames`, `frame_bytes`, `first_ns`, `last_ns` | TX frames in the stream, bytes per frame, first and last send time |
| `period_ns` | least-squares slope of send time over cycle number |
| `interval`, `phase_jitter`, `phase_jitter_abs_p99_ns` | cycle-to-cycle intervals (gaps excluded); deviation from the ideal grid `t0 + k·period`; p99 of its absolute value |
| `gaps`, `missed_cycles` | intervals longer than `gap_factor` periods (1.5); cycles missing inside them |
| `round_trip`, `unanswered` | TX → RX time at the capture point; TX frames that did not return |
| `wkc` | `WkcSlot` per datagram position: `label`, `expected`, `expected_from` (`"eni"` or `"most_frequent"`), `mismatches`, `zero`, `mailbox` (mailbox positions are not counted as mismatches) |
| `wkc_mismatch_frames` | returning frames with at least one working-counter mismatch, all of them |
| `wkc_events`, `unanswered_events` | `{ frame, t_ns }` of the affected frames (first 2000) |
| `interval_series`, `round_trip_series` | `{ t_ns, v }` points, decimated |

**`utilization`**: `link_bps` (assumed rate), `bin_ns`, `mean_tx_percent`, `peak_tx_percent`,
`mean_rx_percent`, and `tx_series`/`rx_series` in percent per bin.

**`stations[]`**: `station`, `name` (from the ENI), `position` (when assigned by APWR 0x0010),
`datagrams`, `reads`, `writes`, `no_response` (returning datagrams with WKC 0), `areas`
(register areas accessed), `requested_state`, `reported_state`, `al_error`.

**`al_events[]`**: `t_ns`, `frame`, `station` (`null` for broadcast), `kind` (`"requested"` or
`"reported"`), `state` (`"INIT"`, `"PREOP"`, `"BOOT"`, `"SAFEOP"`, `"OP"`), `error`.

**`mailbox`** (`MailboxReport`; see [Mailbox traffic](../guide/mailbox.md))

| Field | Meaning |
|---|---|
| `channels[]` | per station: `source` (`"sm_config"`, `"eni"`, `"hint"`, `"heuristic"`), `out_start`/`out_len`, `in_start`/`in_len`, accepted and rejected writes, reads and empty reads, status polls, messages and payload bytes each way, `repeats`, `poll_interval`, `protocols`, `service` (`MailboxService` or `null`: how the master served this mailbox per cycle, and the simulator policy that produces the pattern; see [Mailbox traffic](../guide/mailbox.md#how-the-master-serves-the-mailbox)) |
| `protocols[]` | per protocol (`"CoE"`, `"FoE"`, `"EoE"`, `"SoE"`, `"VoE"`, `"AoE"`, `"Error"`, `"Reserved"`): messages and bytes each way, `first_ns`, `last_ns` |
| `coe` | `transactions[]` (`SdoTransaction`: station, `kind` upload/download, index, subindex, `transfer` expedited/normal/segmented, `bytes`, `start_ns`, `end_ns`, `outcome` `ok`/`aborted`/`superseded`/`incomplete`/`protocol_error`, abort code and text), counts (`transactions_total`, `ok`, `aborted`, `unfinished`, `protocol_errors`), `duration`, `emergencies[]`, `sdo_info_messages` |
| `foe[]` | `FoeTransfer`: station, `kind` (`"write"`/`"read"`), `file_name`, `bytes`, `packets`, `busy`, `start_ns`, `end_ns`, `goodput_bps`, `outcome` (`"ok"`, `"error"`, `"incomplete"`, `"protocol_error"`), error code and text |
| `eoe` | `frames[]` (station, direction `to_slave`/`from_slave`, `start_ns`, `end_ns`, fragments, `bytes`, MACs, EtherType), `frames_total`, `bytes_to_slaves`, `bytes_from_slaves`, `reassembly_errors`, `service_messages` |
| `soe_errors`, `errors[]`, `malformed` | SoE error responses; mailbox error replies; data at a mailbox address that is not a valid mailbox header |

Durations of mailbox transfers are measured at the capture point, between the frames carrying
the first and the last message.

**`findings[]`** (`Finding`): `severity`, `title`, `detail`. `severity` is `"defect"` or
`"warn"` only where a value was compared with a declared threshold, which `detail` states;
`"info"` is context. The thresholds are listed in
[flowlab-analysis § Findings](../workspace/flowlab-analysis.md#findings). From the demo capture:

```json
[
  {
    "severity": "defect",
    "title": "Stream 0 (1.000 ms): 7 frames did not return",
    "detail": "7 of 4994 TX frames (0.140 %) have no returning frame. Threshold: any."
  },
  {
    "severity": "warn",
    "title": "Stream 0 (1.000 ms): 6 missed cycles",
    "detail": "6 gaps longer than 1.5 periods in the master's send times. Threshold: any."
  }
]
```

## GroundTruth

What `flowlab synth` actually generated, written by `--truth`. Analysis tests compare reports
with it. It has no `meta`: it describes a synthetic capture by construction.

| Field | Meaning |
|---|---|
| `seed`, `slaves`, `cycle_time_ns`, `tx_jitter_ns` | from the spec |
| `frames`, `tx_frames`, `rx_frames` | frames written |
| `startup_frames`, `cyclic_tx_frames` | start-up frames (TX + RX); cyclic TX frames |
| `skipped_cycles`, `lost_frames`, `wkc_errors` | injected faults that occurred |
| `expected_lrw_wkc`, `process_image_bytes` | LRW WKC of a healthy cycle; outputs + inputs, bytes |
| `sdo_uploads` | expedited uploads of the identity object |
| `first_cyclic_ns` | first cyclic TX frame relative to the first frame, ns |
| `mailbox` | `MailboxTruth`: `messages_to_slaves`, `messages_from_slaves`, `empty_polls`, `rejected_writes`, `sdo_ok`, `sdo_aborted`, `sdo_segmented`, `sdo_segments`, `foe_transfers`, `foe_bytes`, `eoe_frames_to_slave`, `eoe_frames_from_slave`, `eoe_bytes_to_slave`, `eoe_bytes_from_slave`, `emergencies` |

From `flowlab synth configs/synth/demo.json -o demo.pcapng --truth demo.truth.json`, without
`mailbox`:

```json
{
  "seed": 42,
  "slaves": 6,
  "cycle_time_ns": 1000000,
  "tx_jitter_ns": 4000,
  "frames": 10859,
  "tx_frames": 5433,
  "rx_frames": 5426,
  "startup_frames": 348,
  "cyclic_tx_frames": 4994,
  "skipped_cycles": 6,
  "lost_frames": 7,
  "wkc_errors": 5,
  "expected_lrw_wkc": 10,
  "process_image_bytes": 68,
  "sdo_uploads": 6,
  "first_cyclic_ns": 35800000
}
```

## SimResult

One simulation run ([flowlab-sim § Results](../workspace/flowlab-sim.md#results)). With several
spec files, `--result` writes a JSON array of them. In the server, a run started with
`POST /api/simulate` keeps its result next to its capture.

| Field | Meaning |
|---|---|
| `meta` | provenance `simulated`, spec hash, the model's assumptions |
| `description`, `seed`, `policy` | from the spec; the mailbox policy as text |
| `cycle_time_ns`, `link_bps` | from the spec |
| `virtual_time_ns`, `cycles`, `frames` | simulated time; master cycles; frames sent by the master |
| `wire_bytes`, `link_utilization_percent` | bytes on the wire master → slaves (preamble, FCS, padding, gap included); their share of link capacity |
| `all_completed` | every transfer finished within `max_duration_ms` |
| `transfers[]` | `TransferResult`, below |
| `slaves[]` | `SlaveResult`: `station`, `name`, `pdi_busy_percent`, `dpram_used_bytes`, `dpram_bytes` |
| `capture_frames`, `capture_truncated` | frames written (TX + RX); whether `capture.max_frames` stopped the capture |
| `spec` | the `SimSpec` that was simulated, so the result says what it is a result of |

`TransferResult`:

| Field | Meaning |
|---|---|
| `index`, `station`, `slave`, `protocol`, `direction`, `payload_bytes` | identity (`protocol` is `"EoE"`, `"FoE"`, `"CoE"` or `"PDO channel"`) |
| `delivered_bytes`, `verified`, `sha256`, `completed` | bytes the receiver got in order; whether they hash to the sender's SHA-256; whether the protocol finished |
| `start_ns`, `first_byte_ns`, `end_ns`, `duration_ns` | activation; first and last payload byte at the receiver; `end − start` |
| `exchange_ns` | first to last mailbox message as a capture at the master shows it (mailbox protocols only) |
| `goodput_bps` | payload bits / duration |
| `messages_to_slave`, `messages_from_slave`, `rejected_writes`, `empty_reads` | mailbox traffic |
| `retransmitted_chunks`, `chunk_cycles` | PDO channel only |
| `physical_bound_ns` | payload on the link at zero overhead: no transfer can be faster |
| `ethernet_tcp_reference_ns` | TCP over a dedicated link at the same rate (MSS 1460, steady state, no loss): a lower bound, not a prediction |

From `flowlab simulate configs/sim/example.json --result sim.json`, transfer 1 with some
fields left out:

```json
{
  "index": 1,
  "station": 4099,
  "slave": "bulk-endpoint",
  "protocol": "FoE",
  "direction": "master_to_slave",
  "payload_bytes": 256000,
  "delivered_bytes": 256000,
  "verified": true,
  "completed": true,
  "start_ns": 696366333.0,
  "end_ns": 1721368167.0,
  "duration_ns": 1025001834.0,
  "exchange_ns": 1026960110.0,
  "goodput_bps": 1998045.2054488715,
  "messages_to_slave": 514,
  "messages_from_slave": 514,
  "rejected_writes": 0,
  "empty_reads": 514,
  "physical_bound_ns": 20480000.0,
  "ethernet_tcp_reference_ns": 21655040.0
}
```

These are simulated figures of a model with the example's parameters, not measurements.

## WhatIfResult

The scenario rerun with one change at a time, and with pairs of changes
([Simulation § The Simulate view](../guide/simulation.md#the-simulate-view)).

| Field | Meaning |
|---|---|
| `meta` | provenance `simulated`, hash of the scenario, the method's assumptions |
| `base` | `WhatIfRun` of the scenario as given |
| `changes[]` | `WhatIf`: feasible first, by when the last transfer finished |
| `no_effect` | relative change below which a change has no effect (0.01 = 1 %, `--no-effect-percent`) |
| `rankings[]` | `WhatIfRanking`: first by when all transfers are done (`transfer` `null`), then one per transfer |

`WhatIf`: `label` (what changes, old → new), `lever` (`master`, `mailbox_policy`, `pdi`,
`mailbox`, `application`, `protocol`, or `combined` for a pair), `slave` and `transfer` (indices
it touches, or `null`), `edits[]` (`{ path, value }`: JSON pointers into the spec and the values
they get; applying them reproduces the run), `parts[]` (for a pair, the labels of its two
changes), `run`.

`WhatIfRun`: `feasible` and `reason` (an invalid spec, or a transfer that did not finish
verified), `transfer_ns[]` (duration per transfer), `finish_ns` (when the last transfer
finished), `link_utilization_percent`, `pdi_busy_percent_max`, `dpram_used_percent_max`.

`WhatIfRanking`: `transfer`, and lists of `{ change, time_ns, delta }` (`change` indexes
`changes`, `delta` is the relative change against the scenario as given), best first. `helps`,
`flat` and `hurts` sort the single changes by `delta` against ±`no_effect`; `impossible` holds the
ones whose run is not feasible. `together` holds up to five pairs that beat both of their parts
alone, and the scenario, by more than `no_effect`. A ranking for one transfer leaves out the
other transfers' protocol switches. The Simulate view and `flowlab what-if` show these rankings.

## LiveSnapshot

The state of a live session ([Live capture](../guide/live.md)), from `GET /api/live`.

| Field | Meaning |
|---|---|
| `state` | `running`, `finished` or `failed` |
| `source`, `message` | where the packets come from (`dumpcap on eth1`); why it ended or failed, or the tool's error output |
| `provenance` | `measured` for an interface; a replay keeps its file's |
| `elapsed_s`, `packets`, `bytes`, `saved_to` | wall-clock seconds since the start; what arrived; the save file |
| `window_ns`, `window` | the window's span and its full `CaptureReport` (times relative to the window's first frame) |
| `series[]` | `LivePoint` per second: `t_s`, `frames`, `tx_percent`, `period_ns`, `jitter_p99_ns`, `round_trip_p99_ns` (main cyclic stream, or `null`), `wkc_mismatches`, `unanswered`, `missed_cycles`, `mailbox_messages`, `malformed` |
| `totals` | sums of the points: `ethercat_frames`, `wkc_mismatches`, `unanswered`, `missed_cycles`, `mailbox_messages`, `malformed` |
| `assumptions` | how the live figures are made |

## Lab master results

Written by `flowlab master` ([Lab master](../guide/master.md)); `meta.input` names the payload
(transfer) or the interface (scan, run), and `meta.assumptions` say where the timing comes from.
Times are ns relative to the start of the transfer or run, on the host's clock.

`MeasuredTransfer` uses the names of the simulator's [`TransferResult`](#simresult) wherever a
field means the same:

| Field | Meaning |
|---|---|
| `station`, `slave`, `protocol`, `direction`, `payload_bytes`, `sha256` | as in `TransferResult` (`protocol` is `"EoE"`, `"FoE"` or `"CoE"`) |
| `policy`, `poll`, `cycle_time_ns`, `mailbox_out_bytes`, `mailbox_in_bytes` | how it ran: the policy in the simulator's words, `read` or `status` polling |
| `delivered_bytes` | to the slave: payload in writes the slave accepted; from the slave: payload received in order |
| `verified` | from the slave: the received bytes hash to the expected SHA-256; `null` to the slave (the master cannot see the slave's copy) or without an expected hash |
| `completed`, `error` | the protocol finished; why it stopped otherwise (FoE error, SDO abort, mailbox error, no reply, time limit) |
| `start_ns` (0), `first_byte_ns`, `end_ns`, `duration_ns`, `goodput_bps` | return of the access with the first payload byte; of the access that completed the transfer (to the slave: the last acknowledgement read) |
| `exchange_ns` | first to last mailbox message, defined as in `TransferResult` |
| `messages_to_slave`, `messages_from_slave`, `rejected_writes`, `empty_reads` | as in `TransferResult` |
| `status_reads`, `lost_frames`, `repeat_requests`, `resent_messages`, `duplicate_messages`, `other_messages`, `drained_messages` | the master's own work ([Retries and counters](../guide/master.md#retries-and-counters)) |
| `physical_bound_ns`, `link_bps` | the payload at the link rate, as in the simulator |
| `timing` | `CycleTiming`: `cycles`, `skipped_cycles`, `overruns`, `max_accesses_per_cycle`, and summaries (`count`, `min_ns`, `p50_ns`, `p99_ns`, `max_ns`, `mean_ns`) of `start_late`, `period`, `cyclic`, `mailbox_access` and `busy` |
| `cycle_records[]`, `cycle_records_truncated` | per cycle: `index`, `start_ns`, `late_ns`, `cyclic_ns`, `cyclic_wkc`, `mailbox_accesses`, `busy_ns` |

`RunResult`: `interface`, `slaves`, `cycle_time_ns`, `process_image_bytes`, `expected_wkc`,
`wkc_mismatches`, `lost_frames`, `final_states`, `timing` and `cycle_records[]` as above.

`ScanResult`: `interface` and `slaves[]` with `position`, `station`, `alias`, `name`,
`vendor_id`, `product_code`, `revision`, `serial`, `al_state`, `al_status_code`,
`mailbox_protocols`, `sync_managers[]` (`index`, `start`, `length`, `control`, `mode`,
`direction`), `dc_support` and `propagation_delay_ns`.

## PlanResult

The bus budget of a `PlanSpec` ([flowlab-devices § The planner](../workspace/flowlab-devices.md#the-planner)).
It is arithmetic on the spec and carries its assumptions but no `meta`.

| Field | Meaning |
|---|---|
| `cycle_ns`, `link_bps` | from the spec |
| `image_out_bytes`, `image_in_bytes` | process image |
| `frames[]` | `PlannedFrame`: `datagrams` (`label`, `command`, `data_bytes`), `frame_bytes` (without FCS, padded to 60), `wire_bytes` (preamble, FCS, gap included), `wire_ns` |
| `cyclic_wire_ns`, `cyclic_percent` | wire time of all cyclic frames; as a share of the cycle |
| `ring_delay_ns`, `cyclic_done_ns` | sum of forwarding delays; when the last cyclic frame is back |
| `idle_ns`, `idle_percent` | cycle time left after that |
| `mailbox_frame_ns`, `mailbox_frames_per_cycle`, `mailbox_bytes_per_s` | the largest mailbox write + read pair as one frame; how many fit in the idle time when each waits for the previous; the payload bound per direction for one slave, bytes/s (`null` without a mailbox slave) |
| `checks[]` | `PlanCheck`: `severity` (`"defect"`, `"warn"`, `"info"`), `subject` (a slave name, `"cycle"`, `"process image"`), `message` with its threshold |
| `assumptions` | what the arithmetic assumes |

From `flowlab plan --json` (the built-in example), with the assumptions and three of the four
checks left out:

```json
{
  "cycle_ns": 1000000.0,
  "link_bps": 100000000.0,
  "image_out_bytes": 34,
  "image_in_bytes": 42,
  "frames": [
    {
      "datagrams": [
        { "label": "DC system time", "command": "FRMW", "data_bytes": 8 },
        { "label": "process image part 0", "command": "LRW", "data_bytes": 76 },
        { "label": "AL status", "command": "BRD", "data_bytes": 2 }
      ],
      "frame_bytes": 138,
      "wire_bytes": 162,
      "wire_ns": 12960.0
    }
  ],
  "cyclic_wire_ns": 12960.0,
  "cyclic_percent": 1.296,
  "ring_delay_ns": 4200.0,
  "cyclic_done_ns": 17160.0,
  "idle_ns": 982840.0,
  "idle_percent": 98.284,
  "mailbox_frame_ns": 87040.0,
  "mailbox_frames_per_cycle": 10,
  "mailbox_bytes_per_s": 5060000.0,
  "checks": [
    { "severity": "info", "subject": "Servo-1", "message": "CoE support is unknown (no ESI)." }
  ]
}
```

## StudyResult

A parameter study or calibration ([flowlab-study](../workspace/flowlab-study.md)). Every run is a
simulation.

| Field | Meaning |
|---|---|
| `meta` | provenance `simulated`, the study spec's hash, the study's assumptions |
| `description` | from the spec |
| `parameters`, `objectives`, `goals` | parameter paths; objective labels with units (the fit error first for a calibration); `"minimize"`/`"maximize"` |
| `runs[]` | `StudyRun`, below |
| `feasible` | feasible runs |
| `pareto` | indices of the Pareto-optimal runs |
| `best` | the frontier run with the best first objective (smallest fit error for a calibration); `null` without objectives and targets |
| `residuals[]` | calibration only, at the best run: `label`, `source` (`"given"` or the capture metric), `measured`, `simulated`, `relative_error` = (simulated − measured) / measured |
| `calibrated_against` | `InputRef` of the capture a target read, else `null` |

`StudyRun`: `index`, `values` (parameter values as JSON text), `numeric` (the same where they
are numbers), `objectives`, `feasible`, `reason` (why not), `pareto`, and `spec`, the run's full
`SimSpec`, kept for Pareto-optimal and best runs only so they can be simulated again. From
`flowlab study configs/studies/bulk-transfer/foe-sweep.json --result study.json`, the best run
with its `spec` left out:

```json
{
  "index": 23,
  "values": [
    "1024",
    "80000000",
    "{\"kind\":\"opportunistic\",\"bus_budget_percent\":60.0,\"turnaround_ns\":10000}"
  ],
  "numeric": [1024.0, 80000000.0, null],
  "objectives": [0.04969571, 57.81065499507884],
  "feasible": true,
  "reason": null,
  "pareto": true
}
```

## PdiFit

Written by `flowlab pdi-fit SAMPLES --result FILE` (or `flowlab pdi-bench … --result FILE`).
`meta.source` is `measured` unless the samples file carries a provenance marker
(`flowlab pdi-bench --sim` writes `simulated`; see
[PDI timing samples](specs.md#pdi-timing-samples)), and `meta.input` names the samples file
with its SHA-256.

| Field | Meaning |
|---|---|
| `read`, `write` | `LineFit` or `null`: `samples`, `overhead_ns` (intercept), `rate_bps` (from the slope), `sizes[]` (`bytes`, `samples`, `median_ns`, `p95_ns`, `fitted_ns`, `relative_error`), `max_relative_error` |
| `pdi` | the values for the simulator: `read_bps`, `write_bps`, `access_overhead_ns` (the access-weighted mean of the two intercepts) |

A direction without samples takes the other direction's values, and the assumptions say so.

## EsiFile and EsiImport

`flowlab esi FILE --json` prints the parsed `EsiFile`: `vendor_id`, `vendor_name` and
`devices[]`, each a `DeviceDescription` with `product_code`, `revision`, `type_name`, `name`,
`sync_managers` (`index`, `kind` such as `"MBoxOut"`, `start`, `default_size`, `min_size`,
`max_size`, `control`, `enable`), `rx_pdos`, `tx_pdos`, `mailbox` (booleans `coe`, `foe`, `eoe`,
`soe`, `voe`, `aoe`, `coe_sdo_info`, `coe_complete_access`), `dc`, `default_outputs_bits` and
`default_inputs_bits`. `POST /api/esi` returns an `EsiImport`: the same `file` plus `slaves`, one
`PlanSlave` per device for its default configuration, ready to put into a `PlanSpec`.
