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

None of these changes a result; each makes drift likely.

- **UI copies of Rust logic**: wire time (`+24` in `Timing.tsx`, `ecat_wire::wire_bytes`), the
  mean mailbox rate, the `SimSpec` defaults in `scenario/spec.ts`, and the policy wording in
  `scenario/SystemSummary.tsx` (`MailboxPolicy`'s `Display`).
- **ENI or capture → `SimSpec`.** A model of an existing segment is written by hand; a skeleton
  from an ENI, a `master scan` or a capture's stations and mailbox sizes would start it (with
  "Planner → simulator in one step" below).

## UI

- Upstream `HexView` and `StateTimeline` to lab-ui.
- An item editor for arrays of objects in `@vitavision/forms`.
