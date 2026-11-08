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
- Live capture and master adapters, only behind a feature with explicit opt-in, an interface
  name and an isolated network (see the hardware rules in [Contributing](contributing.md)).

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
