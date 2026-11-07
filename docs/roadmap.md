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

## M3 — Mailbox analysis

- [ ] Mailbox stream reassembly: SyncManager addresses from observed SM configuration writes, else a user hint.
- [ ] CoE: SDO transactions (expedited, segmented, complete access), abort codes, latency; SDO Information; emergencies.
- [ ] FoE file transfers, EoE fragment reassembly (frame → Ethernet packet), VoE, SoE headers.
- [ ] Per-protocol throughput and latency; mailbox polling cadence; UI Mailbox view.
- [ ] Validate against public sample captures and Wireshark's dissector where licences allow.

## M4 — Simulator

- [ ] Discrete-event engine (virtual time, stable event order, seeded faults) emitting frames via `ecat-wire`.
- [ ] Master scheduling policies (fixed quota, opportunistic, round robin) — none assumed universal.
- [ ] ESC model: SyncManagers, DPRAM budget, PDI (SPI) throughput and latency.
- [ ] Bulk-transfer study models (EoE, FoE, VoE, CoE, PDO channel, TCP baseline); scenario editor via `SchemaForm`.

## M5 — Planner and device descriptions

- [ ] ESI XML import (PDOs, SyncManagers, mailbox sizes, supported protocols); ENI import for expected WKCs.
- [ ] Bus budget: frame layout, wire time, utilization and mailbox headroom for a cycle time.
- [ ] Capability gating: a configuration a device cannot support is rejected with the reason.

## M6 — Compare and optimize

- [ ] Simulated vs measured comparison on the same views; calibration of model parameters with residuals.
- [ ] Grid / Latin-hypercube sweeps, constraints, Pareto frontier.

## M7 — Live and lab

- [ ] Feature-gated live capture (libpcap/Npcap), explicit opt-in and interface selection.
- [ ] Master adapters after API and licence review (EtherCrab, SOEM, IgH).
- [ ] Distributed-clock analysis (propagation delay, drift, sync error).
- [ ] Bulk-transfer lab per its hardware gates (external ESC, Toradex Verdin).

## Upstream to lab-ui

- [ ] `HexView` (byte ranges, emphasis) and `StateTimeline` (state lanes over time), once stable.
