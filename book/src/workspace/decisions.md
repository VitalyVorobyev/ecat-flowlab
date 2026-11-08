# Design decisions

Architecture decisions, numbered in the order they were taken. A decision is changed by a new
entry that supersedes it. Open questions at the end are written so that a capture or an
experiment can answer them.

## Foundations

- **ADR-001** Name **ECAT FlowLab**, repository `ecat-flowlab`.
- **ADR-002** Rust first; everything runs offline on macOS without hardware or privileges.
- **ADR-003** Keep layers separate: application transfer, protocol transport, bus/ESC/PDI model, real drivers.
- **ADR-004** A bulk payload is a logical multi-frame transfer, never one EtherCAT frame.
- **ADR-005** Ordinary Ethernet is a mandatory baseline in transport comparisons.
- **ADR-006** A Toradex slave needs a real ESC (external, via PDI); the built-in MAC is not one.
- **ADR-007** No TwinCAT dependency; TwinCAT interoperability is an optional experiment.
- **ADR-008** Simulated, synthetic and measured results stay distinguishable everywhere.

## Scope and architecture

- **ADR-009 Scope: a general EtherCAT analysis tool.** The original design targeted one question
  (10 MB over EoE/PDO/FoE to a Toradex). That question is a *study*
  ([bulk transfer](../studies/bulk-transfer/index.md)) on top of a tool that analyses any
  EtherCAT traffic.
  Consequence: capture analysis comes first, because it is useful on any network today and
  needs the wire codec everything else builds on.
- **ADR-010 One wire codec for every source.** The analyzer decodes and the generator /
  simulator encode through `ecat-wire`. Simulated runs become captures and are analysed by
  the same code as real ones, so comparisons are like-for-like.
- **ADR-011 Own pcap/pcapng reader and writer.** The formats are small and well specified;
  the maintained `pcap-file` line is a 3.0 release candidate. Avoids libpcap for offline
  work. Live capture: see ADR-035.
- **ADR-012 Delivery: local web server.** `flowlab serve` (axum) serves the React UI and a
  JSON API on loopback. Works on a headless lab PC over an SSH tunnel and needs no native
  toolchain. A desktop wrapper could reuse the same API.
- **ADR-013 UI on the `@vitavision` lab-ui packages.** Instrument design language (neutral
  chrome, one accent, verdict colours only for verdicts), IBM Plex, studio shell. Missing
  pieces are built in-app and upstreamed once stable.
- **ADR-014 API types generated from Rust (ts-rs);** generator specs carry a JSON Schema
  (schemars) that `@vitavision/forms` renders. One source of truth for both.
- **ADR-015 No expected-WKC configuration yet.** Expected working counters are inferred as
  the most frequent value; the report states this. Refined by ADR-026.
- **ADR-016 Plain state and fetch hooks in the UI** (no router, no query library), matching
  the sibling lab-ui consumer apps; revisit when deep links are needed.
- **ADR-017 Licence: MIT OR Apache-2.0**, matching lab-ui. Dependencies must be compatible;
  GPL-only masters (e.g. SOEM) can only be optional, separately-licensed adapters.

## Mailbox analysis

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

## Simulator

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

## Device descriptions and planner

- **ADR-025 Read the ESI/ENI subset FlowLab uses, with `roxmltree`.** Read-only DOM, no
  dependencies, MIT OR Apache-2.0; DTDs are rejected. Unknown elements are ignored, so vendor
  extensions do not break import. No vendor ESI files are committed: the test fixtures are
  hand-written in ETG layout.
- **ADR-026 ENI before heuristics for expected working counters** (refines ADR-015). With an ENI,
  a datagram the ENI lists (same command, address, length) is judged against its configured
  `Cnt`; others still use the most frequent value. Every WKC slot reports which basis it used,
  and the finding text names it.
- **ADR-027 The planner is arithmetic.** Frame layout, wire time and bus-side headroom only; PDI
  and slave timing belong to the simulator. Its assumptions travel with each result.

## Studies, calibration and comparison

- **ADR-028 One study concept for sweeps and calibration.** Parameters are JSON pointers into a
  simulator spec, so any spec field can vary without study-specific code. Targets turn a sweep
  into a calibration whose first objective is the fit error.
- **ADR-029 Calibrate on what both sides observe.** The simulator reports each transfer's message
  span at the master, the same span `flowlab analyze` measures in a capture; end-to-end
  application times are not used for fitting.
- **ADR-030 A good fit is not identification.** Results say so: several parameter sets may
  reproduce the same measurements. Residuals are always shown.
- **ADR-031 Comparison keeps provenance per column.** The Compare view shows ratios between
  labelled columns; it never averages or merges measured and simulated values.

## Documentation and validation

- **ADR-032 One book for users and contributors.** User-facing documentation is this mdBook plus
  a short README; it describes what the tool does, not plans. Rust API docs are generated by
  rustdoc (warnings denied in CI) and published next to the book.
- **ADR-033 Study reports are generated.** A report's tables are written by a script from the
  committed specs (`scripts/bulk-transfer-report.py`); CI regenerates them and fails on any
  difference, so a model change cannot leave stale numbers in the text.
- **ADR-034 Generators behave like ESCs where the analysis depends on it.** Returning frames from
  `flowlab-synth` and `flowlab-sim` carry the ADP incremented by every slave for position and
  broadcast datagrams, as real ESCs do, and TX/RX pairing ignores that field. An analysis that
  only ever saw its own generator's frames had paired on the raw address and would have failed
  on real captures.

## Live capture and hardware

- **ADR-035 Live capture through an external capture tool.** `flowlab capture` and the Live view
  run Wireshark's `dumpcap` (or `tcpdump`) as a child process and read pcap/pcapng from its
  standard output; FlowLab never opens an interface. The tools already handle capture
  permissions (the `wireshark` group, ChmodBPF, Npcap), timestamp precision and filters on every
  platform. FlowLab gains no native dependency, the offline build stays free of libpcap
  (ADR-011), and the stream is read by the same parser and analysis as a file. Capturing only
  listens, so it needs no cargo feature. The server still asks for `--live` before it starts a
  capture from an HTTP request, and the interface is always named explicitly.
  Consequence: the timestamps are the tool's. NIC hardware timestamps need the tool's own option.
- **ADR-037 PDI timing measured by a benchmark that speaks the ESC's protocol.** `flowlab
  pdi-bench` performs complete ESC accesses through the LAN9252's SPI protocol, the same paths a
  slave stack's hardware layer takes (CSR interface below 0x1000, process RAM FIFOs above, AL
  Event read after each access as in SOES), and times each one; the samples go through
  `flowlab pdi-fit` unchanged. Instrumenting the real stack stays possible but needs a patched
  build per stack; the benchmark needs only the binary on the target. It talks to `/dev/spidevB.C`
  through the `spidev` crate (MIT/Apache-2.0, Linux only; other systems compile without it). The
  LAN9252 logic sits behind a small SPI trait, so a simulated LAN9252 tests it and gives `--sim`
  dry runs, whose samples carry `# flowlab:provenance=simulated`; `pdi-fit` reads the marker and
  refuses to store such a fit as lab inputs. Hardware safety: the link check (`BYTE_TEST`,
  `HW_CFG`, `ID_REV`) runs first, the ESC must be in INIT (and still be at the end), AL Control and
  SyncManager registers are never accessed, and writes go only into process RAM with
  `--allow-writes` and an explicit `--area`. SOES (GPLv2) and Microchip's application notes were
  read for protocol facts only; no code was copied.
  Consequence: the samples time user-space spidev, one driver variant; a kernel driver or
  another stack's access pattern needs its own measurement.

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
  *Test:* `flowlab pdi-bench` on the rig, then `flowlab pdi-fit`; check the fit against a
  capture of mailbox turnaround.
- Does the LAN9252's `ID_REV` read 0x9252 in bits 31:16, and does it accept the `FAST READ`
  dummy byte value 0x01 that SOES sends? `pdi-bench` assumes both (`--chip-id` overrides the
  first). *Test:* the first `pdi-bench` run on the Verdin, with both read instructions.
- Does the access pattern of `pdi-bench` (one access at a fixed address per size, polling the
  PRAM command register before each FIFO burst) time the same as the real stack's accesses?
  *Test:* compare its fit with timing logged inside SOES on the same board.
- Which mailbox policy does each candidate master implement (EtherCrab, SOEM, IgH, TwinCAT), and
  does it poll mailboxes every cycle? *Test:* captures of an FoE write per master; compare with
  the policies' signatures (accesses per cycle, frames per cycle).
- ENI `<Cyclic><CycleTime>` is read as µs. Is that what TwinCAT and other configurators write,
  and do they all address cyclic datagrams with `<Addr>` / `<Adp>`+`<Ado>` as assumed?
  *Test:* ENI exports from TwinCAT and one other tool against their captures (the expected-WKC
  basis should be "eni" for every cyclic datagram).

