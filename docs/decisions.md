# Decisions

Architecture decisions, newest last. A decision is changed by a new entry that supersedes it.

## Carried over from the starter design

- **ADR-001** Name **ECAT FlowLab**, repository `ecat-flowlab`.
- **ADR-002** Rust first; everything runs offline on macOS without hardware or privileges.
- **ADR-003** Keep layers separate: application transfer, protocol transport, bus/ESC/PDI model, real drivers.
- **ADR-004** A bulk payload is a logical multi-frame transfer, never one EtherCAT frame.
- **ADR-005** Ordinary Ethernet is a mandatory baseline in transport comparisons.
- **ADR-006** A Toradex slave needs a real ESC (external, via PDI); the built-in MAC is not one.
- **ADR-007** No TwinCAT dependency; TwinCAT interoperability is an optional later experiment.
- **ADR-008** Simulated, synthetic and measured results stay distinguishable everywhere.

## 2026-11-07 — reframing

- **ADR-009 Scope: a general EtherCAT analysis tool.** The starter targeted one question
  (10 MB over EoE/PDO/FoE to a Toradex). That question is now a *study*
  (`docs/studies/bulk-transfer/`) on top of a tool that analyses any EtherCAT traffic.
  Consequence: capture analysis comes first, because it is useful on any network today and
  needs the wire codec everything else builds on.
- **ADR-010 One wire codec for every source.** The analyzer decodes and the generator /
  simulator encode through `ecat-wire`. Simulated runs become captures and are analysed by
  the same code as real ones, so comparisons are like-for-like.
- **ADR-011 Own pcap/pcapng reader and writer.** The formats are small and well specified;
  the maintained `pcap-file` line is a 3.0 release candidate. Avoids libpcap for offline
  work. Live capture (M7) will use libpcap/Npcap behind a feature.
- **ADR-012 Delivery: local web server.** `flowlab serve` (axum) serves the React UI and a
  JSON API on loopback. Works on a headless lab PC over an SSH tunnel and needs no native
  toolchain. A Tauri wrapper can reuse the same API later.
- **ADR-013 UI on the `@vitavision` lab-ui packages.** Instrument design language (neutral
  chrome, one accent, verdict colours only for verdicts), IBM Plex, studio shell. Missing
  pieces are built in-app and upstreamed once stable.
- **ADR-014 API types generated from Rust (ts-rs);** generator specs carry a JSON Schema
  (schemars) that `@vitavision/forms` renders. One source of truth for both.
- **ADR-015 No expected-WKC configuration yet.** Expected working counters are inferred as
  the most frequent value until ENI/ESI import (M5); the report states this.
- **ADR-016 Plain state and fetch hooks in the UI** (no router, no query library), matching
  the sibling lab-ui consumer apps; revisit when deep links are needed.
- **ADR-017 Licence: MIT OR Apache-2.0**, matching lab-ui. Dependencies must be compatible;
  GPL-only masters (e.g. SOEM) can only be optional, separately-licensed adapters.

## Open questions

- Do TwinCAT / other masters' captures always show the ESC-set source-MAC bit, or are
  captures taken behind a switch / with a tap different? *Test:* collect captures from at
  least two masters and a tap; check `direction_mode`.
- Beckhoff ET2000-style capture probes add their own header and hardware timestamps. Worth
  supporting before M3? *Test:* obtain a sample; measure share of user captures using it.
- Is a 50 ms in-flight window right for "unanswered" at capture end on slow cycles
  (≥ 50 ms)? *Test:* synth with a 100 ms cycle and a truncated end.
