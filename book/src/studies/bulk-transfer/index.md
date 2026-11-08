# Bulk transfer over EtherCAT

**Question.** How fast and how reliably can an opaque binary payload of 10,000,000 bytes (10 MB;
separately 10,485,760 B, 10 MiB) cross an EtherCAT segment between a PC master and a slave
application? The candidates are EoE, FoE, VoE, CoE segmented SDO and a custom process-data
channel. How does each compare with plain Ethernet?

**Target system.** A Linux or Windows PC as master. As slave, a Toradex Verdin iMX8M Plus
application behind an **external EtherCAT slave controller** (a LAN9252-class ESC) on SPI. The
Verdin's own Ethernet port is an ordinary MAC, not an ESC, and cannot be an EtherCAT slave; it
remains available for management and as the plain-Ethernet baseline.

```text
PC master ──[100BASE-TX EtherCAT]── LAN9252 ESC board ──SPI / IRQ / reset── Verdin iMX8M Plus
                                                                            slave application
```

| Page | Content |
|---|---|
| [Report](report.md) | Findings and recommendations from the simulator: what limits a 10 MB transfer, what helps, and what to measure first |
| [Method](method.md) | Requirements, transport candidates, fairness rules, metrics, the search problem |
| [Lab plan](lab.md) | Hardware gate, wiring checklist, master choice, lab sequence, stop/go decisions, risks |

## How FlowLab serves the study

- The simulator models the master's mailbox scheduling, the ESC (one-buffer mailboxes,
  three-buffer process data, 4 KiB RAM) and the PDI, and runs EoE, FoE, CoE and process-data
  transfers with SHA-256 integrity. Its frames are analysed like a lab capture.
- Studies sweep the parameters, report Pareto frontiers, and fit the model to lab captures
  (FoE/SDO message spans) to calibrate the PDI and master parameters.
- The same capture views apply to simulated transfers and to lab captures, so model and
  measurement are compared view for view.

Specs: `configs/studies/bulk-transfer/`. The report's sweeps are in its `report/` folder. For a
network other than this study's, the same tools are described step by step in
[Investigating your own network](../../guide/own-network.md).

## Physical sanity bound

10,000,000 B cannot cross one 100 Mbit/s link in less than 0.8 s, even with zero overhead. That
is a bound, not a target. The real transfer's deadline and repetition rate are open questions;
see [Method](method.md).
