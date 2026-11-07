# Roadmap

No dates. Each milestone ends with a running, demonstrable artifact. Studies (e.g.
[bulk transfer](studies/bulk-transfer/README.md)) draw on the milestones they need.

## M0 — Foundation ✅

- [x] Restructure the starter design around a general analysis tool; preserve the bulk-transfer study.
- [x] Cargo workspace, lints (`unsafe_code = forbid`), rustfmt.
- [x] `ecat-wire` parse + encode with golden bytes and proptest round trips.
- [x] `flowlab-synth`: deterministic captures with ground truth.
- [x] CI: fmt, clippy `-D warnings`, tests; UI lint, typecheck, test, build.

## M1 — Capture analysis core ✅

- [x] pcap/pcapng reader and writer (no libpcap).
- [x] Trace index, direction inference, TX/RX pairing.
- [x] Cyclic streams, period, phase jitter, intervals, gaps, round trip, WKC, utilization,
      stations, AL timeline, findings with declared thresholds.
- [x] `flowlab analyze` → JSON report with provenance and assumptions; tests against synth ground truth.

## M2 — Server and UI ✅

- [x] `flowlab serve`: upload / open path / synth, report, paged frame queries, frame decode.
- [x] UI on lab-ui: Captures, Overview, Timing, Frames (filters, decode, hex), Stations (AL timeline).
- [x] ts-rs API types; render tests against a real report fixture.

## M3 — Mailbox analysis ✅

- [x] Mailbox channels: SyncManager addresses from observed SM configuration writes, else a user hint
      (`--mailbox STATION=OUT,IN`), else a header heuristic — the source is reported.
- [x] Messages from returning datagrams (WKC > 0); rejected writes, empty reads, status polls, repeats.
- [x] CoE: SDO transactions (expedited, normal, segmented, complete access flag), toggle/size checks,
      abort codes with text, duration; emergencies; SDO Information counted.
- [x] FoE file transfers (goodput, errors, busy); EoE fragment reassembly with pcapng export; SoE/VoE/AoE headers.
- [x] Per-protocol traffic; polling cadence; mailbox datagrams excluded from WKC verdicts and stream detection.
- [x] UI Mailbox view; frame list filter by protocol; decoded mailbox fields in the frame detail.
- [x] Synth: polling, rejected writes, segmented SDO, aborts, FoE, EoE, emergencies with ground truth.
- [x] Cross-check against Wireshark's EtherCAT dissector (CI).
- [ ] Validate on real captures from at least two masters (none available yet).
- [ ] SDO Information transactions, SoE transactions, AoE; logically mapped mailbox status.

## M4 — Simulator ✅

- [x] Discrete-event engine (virtual time, stable event order, seeded faults) emitting frames via `ecat-wire`;
      simulated captures go through the same analysis (and match Wireshark in CI).
- [x] Master scheduling policies (fixed quota, round robin, opportunistic) — none assumed universal.
- [x] ESC model: mailbox and process-data SyncManagers, ESC RAM gating, PDI rates, per-access overhead,
      whole-buffer or message-only mailbox access, contention.
- [x] Bulk-transfer models: EoE (UDP-like tunnel), FoE, CoE segmented, PDO channel with go-back-N;
      physical bound and plain-Ethernet TCP reference; SHA-256 integrity.
- [x] `flowlab simulate` (comparison table), `POST /api/simulate`, Simulate dialog (SchemaForm) and Sim run view.
- [x] Study scenarios migrated to the schema.
- [ ] TCP over EoE, VoE, priority/weighted mailbox policies, the application session protocol of the study.
- [ ] Calibration of PDI and master parameters against lab captures (M6/M7).

## M5 — Planner and device descriptions ✅

- [x] ESI import (identity, SyncManagers, PDOs and default assignment, mailbox protocols, DC) — `flowlab esi`, `POST /api/esi`.
- [x] ENI import: expected WKCs per cyclic datagram (falling back to the most frequent value, reported per
      datagram), mailboxes for stations without SM configuration in the capture, station names —
      `analyze --eni`, `serve --eni`, Attach ENI in the UI. `synth --eni` writes the generator's network as ENI.
- [x] Bus budget: frame layout, wire time, ring delay, idle time, bus-side mailbox headroom — `flowlab plan`, Planner view.
- [x] Capability gating: cycle overrun, ESC RAM, mailbox size limits, missing mailbox protocols, each with its reason.
- [ ] Validate the ENI reader on exports from TwinCAT and at least one other configurator (none available yet).
- [ ] Planner → simulator: run a plan with a workload in one step.

## M6 — Compare and optimize ✅

- [x] Studies (`flowlab-study`): parameters by JSON pointer (linked pointers, ranges, log spacing), grid or
      Latin-hypercube sampling, objectives, constraints with reasons, Pareto frontier — `flowlab study`, Study view.
- [x] Calibration: targets measured directly or read from an analysed capture; best fit and residuals. The
      simulator reports message spans at the master so model and capture are compared like for like.
- [x] Compare view: two captures (e.g. measured and simulated) side by side, each column with its provenance.
- [ ] Calibrate against real lab captures (needs M7 hardware); local optimization beyond grid/LHS if sweeps get large.

## M7 — Live and lab

- [ ] Feature-gated live capture (libpcap/Npcap), explicit opt-in and interface selection.
- [ ] Master adapters after API and licence review (EtherCrab, SOEM, IgH).
- [ ] Distributed-clock analysis (propagation delay, drift, sync error).
- [ ] Bulk-transfer lab per its hardware gates (external ESC, Toradex Verdin).

## Upstream to lab-ui

- [ ] `HexView` (byte ranges, emphasis) and `StateTimeline` (state lanes over time), once stable.
- [ ] `@vitavision/forms`: arrays of objects (slaves, transfers) render as JSON text; an item editor would help.
