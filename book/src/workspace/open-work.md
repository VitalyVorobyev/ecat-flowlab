# Open work

What is not done yet, for contributors. The [limitations](../reference/limitations.md) describe
the same gaps from a user's point of view; the [design decisions](decisions.md) list the open
questions that real captures should answer.

## Real captures and hardware

- Validate the analysis, the ENI reader and direction inference on captures from several
  masters (TwinCAT, EtherCrab, SOEM/IgH) and from a tap, and answer the open questions in the
  [design decisions](decisions.md).
- Run the lab inputs (`flowlab pdi-fit`, `flowlab policy`) on the Verdin/LAN9252 rig and a real
  master, then commit a calibrated bulk-transfer report next to the placeholder one.
- Run live capture (`flowlab capture`, the Live view) on a real segment, on Linux and Windows.
- Run the lab master (`flowlab master scan`, `run`, `transfer`) on the Verdin/LAN9252 rig, on
  Linux and then Windows, and answer its open questions in the [design decisions](decisions.md).
  Until then the EtherCrab glue in `flowlab-master` has only been compiled.
- Lab master: load a `MeasuredTransfer` into the UI's Compare view next to a simulated run;
  several slaves and transfers at once (the policies' multi-slave behaviour); TCP over EoE and
  VoE; distributed clocks in `run`.
- Run `flowlab pdi-bench` on the Verdin/LAN9252 with both read instructions and several clocks,
  and answer the LAN9252 questions in the [design decisions](decisions.md#open-questions).
- PDI profiling of the real stack: a converter from kernel SPI trace events (ftrace/`trace-cmd`
  `spi:spi_transfer_start`/`stop`) of the running slave application to PDI timing samples for
  `flowlab pdi-fit`, grouping the transfers of one ESC access.
- Live capture: NIC hardware timestamps (`tcpdump -j`), ring files for long captures.

## Analysis

- SDO Information and SoE transactions; AoE beyond the header.
- Mailbox status polled through a logically mapped SyncManager status bit.
- Distributed clocks: propagation delay, drift, sync error.
- Capture probes with their own headers and hardware timestamps (ET2000-style).

## Simulator and studies

- TCP over EoE (acknowledgements, windows, retransmission) and VoE.
- Priority and weighted mailbox policies.
- The bulk-transfer study's application session protocol.
- Planner → simulator in one step.
- Local optimisation beyond grid and Latin hypercube sampling for large sweeps.

## Structure and duplication

Found in a review of the workspace; none changes a result today, each makes drift likely.

- **One mailbox message layer.** FoE and SDO message builders, the 7-byte SDO padding, counters
  1–7, EoE fragmentation (frame number `% 15 + 1`, 32-byte offsets) and reassembly, and the
  UDP/IPv4 frame exist four times: `flowlab-sim/src/protocol.rs`, `flowlab-master/src/protocol.rs`,
  `flowlab-master/src/virtual_slave.rs` and `flowlab-synth/src/mailbox.rs`. The master's copy
  already handles FoE BUSY and repeated DATA packets, the simulator's does not. Candidate home: a
  small crate below sim and master (`MailboxPolicy` with `Display`/`FromStr`, the payload
  generator, the message builders), which also removes `flowlab-master`'s dependency on the whole
  simulator; the analysis trackers stay separate as an independent reader.
- **Policy text and parsing.** `policy_text` is copied in `flowlab-sim/src/engine.rs` and
  `flowlab-master/src/lib.rs`, a third wording is in `ui/src/scenario/SystemSummary.tsx`; the
  CLI's `parse_policy` belongs on `MailboxPolicy`.
- **Mirrored result types.** `TransferResult` (sim) and `MeasuredTransfer` (master) share about
  20 fields, with two `TransferProtocol` enums, `String` directions and two computations of
  `physical_bound_ns`; `flowlab_study::lab::PdiValues` mirrors `flowlab_sim::PdiSpec`; the PDI
  samples CSV is written by `flowlab-pdi` and parsed by `flowlab-study` with separate types.
- **Small repeats.** Four SplitMix PRNGs (sim, study, pdi, synth's seeding); nine hex encoders
  and six hand-built `InputRef`s; four protocol-name tables; SyncManager register constants and
  the "mailbox full" bit in master and analysis (`ecat-wire`'s `sm` module is their home);
  capture → `Trace::build` → analyse three times in the CLI and once in the server.
- **`flowlab-analysis/src/mailbox.rs`** (about 1,500 lines) holds channel discovery, the CoE, FoE
  and EoE trackers, the description text, findings, assumptions and the EoE pcapng export: split
  along those lines.
- **Server presets** embed the study configs with `include_str!` and skip one that fails to
  parse; the catalogue belongs in the simulator or the study crate, with a test.
- **UI copies of Rust logic**: wire time (`+24` in `Timing.tsx`, `ecat_wire::wire_bytes`), the
  mean mailbox rate, and the `SimSpec` defaults in `scenario/spec.ts`.
- **ENI or capture → `SimSpec`.** A model of an existing segment is written by hand; a skeleton
  from an ENI, a `master scan` or a capture's stations and mailbox sizes would start it (with
  "Planner → simulator in one step" below).

## UI

- Upstream `HexView` and `StateTimeline` to lab-ui.
- An item editor for arrays of objects in `@vitavision/forms`.
