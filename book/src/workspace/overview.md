# Overview

FlowLab is a Cargo workspace of ten crates and a React UI. Every source of traffic produces
real Ethernet frames, and one analysis reads them all.

```text
  capture file ───┐
  (measured)      │
                  ├─▶ flowlab-capture ─▶ Trace ─▶ flowlab-analysis ─▶ CaptureReport ─┬─▶ flowlab analyze (JSON)
  flowlab-synth ──┤    pcap/pcapng        frame and   streams, timing, WKC,           └─▶ flowlab-server ─▶ UI
  (synthetic)     │    reader, index,     datagram    utilization, stations,
                  │    directions         index       mailbox, findings
  flowlab-sim ────┘
  (simulated)          all frames are encoded and decoded by ecat-wire

  flowlab-devices: ESI/ENI → expected WKCs, names, mailboxes (analysis) · bus budget (planner)
  flowlab-study:   many flowlab-sim runs → Pareto frontier, calibration against a CaptureReport
```

Because the simulator and the generator write frames through the same codec the analysis
decodes, a simulated run, a synthetic capture and a lab capture are compared like for like.
Only their provenance differs.

## Crates

| Crate | Role | Depends on |
|---|---|---|
| [`ecat-wire`](ecat-wire.md) | EtherCAT codec: frames, datagrams, mailbox protocols, registers. No I/O, no allocation while parsing | — |
| [`flowlab-core`](flowlab-core.md) | Provenance, report metadata, units | serde |
| [`flowlab-capture`](flowlab-capture.md) | pcap/pcapng reader and writer, trace index, direction inference | ecat-wire, core |
| [`flowlab-devices`](flowlab-devices.md) | ESI/ENI readers, ENI writer, bus-budget planner | ecat-wire |
| [`flowlab-analysis`](flowlab-analysis.md) | Capture analysis and frame queries | capture, devices |
| [`flowlab-synth`](flowlab-synth.md) | Deterministic synthetic captures with ground truth | capture, devices |
| [`flowlab-sim`](flowlab-sim.md) | Discrete-event simulator emitting frames | capture |
| [`flowlab-study`](flowlab-study.md) | Parameter studies, Pareto frontier, calibration | sim, analysis |
| [`flowlab-server`](server-cli.md) | Local HTTP API and static UI (axum) | all of the above |
| [`flowlab-cli`](server-cli.md) | The `flowlab` binary | all of the above |
| `xtask` (not published) | Developer commands behind the cargo aliases `cargo ui`, `ui-dev`, `ci`, `site` ([Contributing](contributing.md)) | std only |

The [UI](ui.md) in `ui/` talks only to the server's API. Its TypeScript types are generated from
the Rust result types by ts-rs.

## Conventions

- **Edition 2024**, `unsafe_code = "forbid"`, `missing_docs` and `unreachable_pub` warnings;
  CI denies all warnings, including rustdoc's.
- **Units in field names** (`_ns`, `_bytes`, `_bps`) rather than newtypes, so JSON stays
  self-describing. Times sent to the UI are relative to capture start and stay below 2⁵³ ns.
- **Determinism.** Generator and simulator use their own PRNGs and a stable event order: the
  same spec gives byte-identical output.
- **Specs are schemas.** Every input (`SynthSpec`, `SimSpec`, `PlanSpec`, `StudySpec`) derives a
  JSON Schema (schemars, field order preserved) that the UI renders as a form and that
  `flowlab schema` prints. Unknown fields are rejected.
- **Analysis is pure.** `flowlab-analysis` functions take a `Trace` and options, and return
  values; every heuristic lands in `meta.assumptions`, every threshold in `AnalysisOptions`.
- **The server is thin.** Handlers parse, call a library function off the async runtime,
  and serialise.

## API documentation

The rustdoc of every crate is published with this book: start at
[`ecat_wire`](../../api/ecat_wire/index.html), [`flowlab_analysis`](../../api/flowlab_analysis/index.html),
[`flowlab_sim`](../../api/flowlab_sim/index.html) or [`flowlab_study`](../../api/flowlab_study/index.html).
Locally: `cargo doc --no-deps --open`.
