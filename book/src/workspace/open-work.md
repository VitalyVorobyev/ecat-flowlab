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
- Run live capture (`flowlab capture`, the Live view) on a segment with real ESCs, on Linux and
  Windows; on macOS it has captured the emulated-slave rig.
- Run the lab master (`flowlab master scan`, `run`, `transfer`) on the Verdin/LAN9252 rig with
  a real ESC, on Linux and then Windows, and answer its open questions in the
  [design decisions](decisions.md). On a cable to the [emulated slave](../guide/slave.md) it has
  run from macOS (scan, a cyclic run, FoE, CoE and EoE both ways); EtherCrab's Linux socket and
  Npcap have not run yet.
- Emulated slave: several slaves on one port (a line of ESC models, each incrementing ADP and
  adding to the working counter), distributed clocks, the three-buffer mode, a configurable
  processing delay, and a Windows build (Npcap).
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

- Tell a master's start-up register polling from cyclic streams. On a capture of `flowlab
  master run`, EtherCrab's EEPROM reads (FPRD/FPWR of the SII registers `0x0502` and `0x0508`)
  form three short "streams" with missed-cycle and jitter warnings.
- SDO Information and SoE transactions; AoE beyond the header.
- Mailbox status polled through a logically mapped SyncManager status bit.
- Distributed clocks: propagation delay, drift, sync error.
- Capture probes with their own headers and hardware timestamps (ET2000-style).

## Simulator and studies

- TCP over EoE (acknowledgements, windows, retransmission) and VoE.
- Priority and weighted mailbox policies.
- The bulk-transfer study's application session protocol.
- A `SimSpec` of an existing segment in one step: from the planner, an ENI, a `master scan` or a
  capture's stations and mailbox sizes.
- Local optimisation beyond grid and Latin hypercube sampling for large sweeps.

## UI

- Upstream `HexView` and `StateTimeline` to lab-ui.
- An item editor for arrays of objects in `@vitavision/forms`.
