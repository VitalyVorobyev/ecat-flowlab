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

## 2026-11-07 — M3 mailbox analysis

- **ADR-018 Mailbox messages come from returning datagrams.** The working counter of the
  returning copy is the only evidence that a write was accepted or a read carried data.
  With unknown directions, datagrams with WKC > 0 are used and rejected/empty counts are
  not reported.
- **ADR-019 Mailbox addresses from SyncManager configuration first.** A data-layout heuristic
  is a fallback only, and every channel states its source, because process data can look
  like a mailbox header by chance.
- **ADR-020 Wireshark as an independent reference.** Generated captures are decoded by
  Wireshark's EtherCAT dissector in CI and compared message for message with FlowLab's counts
  (`scripts/crosscheck-tshark.py`). Ground truth checks the generator; Wireshark checks that
  generator and analyzer do not share a misreading of the specification.

## 2026-11-07 — M4 simulator

- **ADR-021 The simulator writes frames, not summaries.** Its pcapng goes through the capture
  analysis like a measurement; its own result reports only what the analysis cannot see
  (integrity, PDI load, ESC RAM, protocol-internal retries). Like-for-like comparison with lab
  captures (ADR-010) follows.
- **ADR-022 No default "one mailbox per cycle".** Policies are explicit spec choices (fixed
  quota, round robin, opportunistic) and every result names its policy.
- **ADR-023 References, not targets.** Each transfer reports the time the payload alone needs on
  the link and a TCP-over-dedicated-Ethernet lower bound. Neither is a prediction for a device.
- **ADR-024 PDI access is a parameter.** Whether a slave stack moves whole mailbox buffers or only
  messages changes PDI-bound results severalfold; both are modelled and the spec says which.

## Open questions

- Do TwinCAT / other masters' captures always show the ESC-set source-MAC bit, or are
  captures taken behind a switch / with a tap different? *Test:* collect captures from at
  least two masters and a tap; check `direction_mode`.
- Beckhoff ET2000-style capture probes add their own header and hardware timestamps. Worth
  supporting before validating on real captures? *Test:* obtain a sample; measure the share
  of user captures using it.
- Is a 50 ms in-flight window right for "unanswered" at capture end on slow cycles
  (≥ 50 ms)? *Test:* synth with a 100 ms cycle and a truncated end.
- Do masters mix mailbox datagrams into the cyclic frame (IgH packs datagrams of several
  domains/slaves into one Ethernet frame)? Stream detection groups by full frame layout and
  would then split one cycle into several layouts. *Test:* IgH capture; count streams vs
  configured domains.
- How common is mailbox status polling via a logically mapped SyncManager status bit (FMMU on
  0x080D) instead of FPRD? Such polls are counted as process data today. *Test:* TwinCAT capture.
- Is "counter equal to the previous" a reliable repeat test when a master resets counters
  after a state change? *Test:* captures across INIT → PREOP transitions on real slaves.
- What are a LAN9252-class ESC's effective SPI read/write rates and per-access overhead from a
  Linux userspace driver on the Verdin? The simulator's PDI-bound results hinge on them.
  *Test:* timestamped SPI transactions or a capture of mailbox turnaround on the lab rig.
- Which mailbox policy does each candidate master implement (EtherCrab, SOEM, IgH, TwinCAT), and
  does it poll mailboxes every cycle? *Test:* captures of an FoE write per master; compare with
  the policies' signatures (accesses per cycle, frames per cycle).

