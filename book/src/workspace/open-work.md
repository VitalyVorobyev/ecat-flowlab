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
- A lab master on EtherCrab: scan (identity, EEPROM, SyncManagers, an ENI for the analysis), a
  cyclic run with per-cycle round trips, and mailbox transfer benchmarks (FoE, CoE, EoE) under
  the simulator's policies. Only with explicit opt-in, an interface name and an isolated network
  (see the hardware rules in [Contributing](contributing.md)); refuse to start when another
  master's frames are seen.
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

## UI

- Upstream `HexView` and `StateTimeline` to lab-ui.
- An item editor for arrays of objects in `@vitavision/forms`.
