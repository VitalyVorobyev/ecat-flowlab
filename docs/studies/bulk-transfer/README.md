# Study: large binary transfers over EtherCAT

**Question.** How fast, and how reliably, can an opaque binary payload of nominally
10,000,000 B (10 MB — and separately 10,485,760 B, 10 MiB) cross an EtherCAT segment between
a PC master and a slave, by EoE, FoE, VoE, CoE segmented/block SDO or a custom PDO channel —
and how does that compare with plain Ethernet?

**Lab target (later).** Linux/Windows PC master; Toradex Verdin i.MX8M Plus slave application
behind an **external ESC** (e.g. LAN9252) on SPI. The Verdin's own Ethernet MAC is not an
ESC. See [hardware-lab.md](hardware-lab.md) for the feasibility gate.

**Status.** Design only. It needs the simulator (M4), mailbox analysis (M3) and device
descriptions (M5) of the main [roadmap](../../roadmap.md); its lab work is M7.

## Documents

These come from the starter design hand-off and are kept as the study's specification. Where
they describe the *tool* (architecture, milestones), `docs/architecture.md` and
`docs/roadmap.md` supersede them.

| File | Content |
|---|---|
| [requirements.md](requirements.md) | Functional / non-functional requirements and open product questions |
| [transport-study.md](transport-study.md) | EoE, FoE, VoE, CoE, PDO channel, TCP baseline: strengths and risks |
| [wire-protocol.md](wire-protocol.md) | Transport-independent application transfer protocol (START/DATA/ACK/END) |
| [simulation-model.md](simulation-model.md) | Discrete-event model, scheduling policies, ESC/PDI constraints, sanity bounds |
| [benchmark-methodology.md](benchmark-methodology.md) | Workloads, metrics, statistics, fairness |
| [optimization.md](optimization.md) | Search variables, objectives, Pareto, validity gates |
| [hardware-lab.md](hardware-lab.md) | ESC gate, wiring checklist, master choice, lab sequence L0–L6, stop/go |
| [risks.md](risks.md) | Risks, unknowns, decision gates D1–D4 |
| [archive/](archive/) | The original starter README, agent guide, system design, first sprint and ADR file, verbatim |

## How the tool serves the study

- The same `ecat-wire` frames and analysis views apply to simulated transfers and to real
  lab captures (ADR-010), so "model vs measurement" is one comparison, not two pipelines.
- Starter scenarios live in `configs/studies/bulk-transfer/`. They predate the schema: on
  M4 they become Rust config structs with a JSON Schema. Known issues to fix then:
  `mode` mixes input with result provenance; `mailbox_services_per_cycle: 0.5` has no
  defined semantics; there is no ESC/slave memory model.

## Physical sanity bound (not a target)

10,000,000 B cannot cross one 100 Mbit/s link in under 0.8 s even with zero overhead
(`flowlab_core::units::serialization_ns`). The deadline and repetition rate of the real
transfer are still unknown and must not be invented.
