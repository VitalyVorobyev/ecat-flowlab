# Study: large binary transfers over EtherCAT

**Question.** How fast, and how reliably, can an opaque binary payload of nominally
10,000,000 B (10 MB — and separately 10,485,760 B, 10 MiB) cross an EtherCAT segment between
a PC master and a slave, by EoE, FoE, VoE, CoE segmented/block SDO or a custom PDO channel —
and how does that compare with plain Ethernet?

**Lab target (later).** Linux/Windows PC master; Toradex Verdin i.MX8M Plus slave application
behind an **external ESC** (e.g. LAN9252) on SPI. The Verdin's own Ethernet MAC is not an
ESC. See [hardware-lab.md](hardware-lab.md) for the feasibility gate.

**Status.** The simulator (M4) and mailbox analysis (M3) exist; the study's specs run with
`flowlab simulate configs/studies/bulk-transfer/*.json`. Device descriptions (M5), sweeps (M6)
and the lab work (M7) are still to come; until the model is calibrated against captures, its
results answer "what does the outcome depend on", not "how fast will it be".

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
- The scenarios in `configs/studies/bulk-transfer/` are `SimSpec`s (JSON Schema:
  `flowlab schema sim`; editable in the UI's Simulate dialog). The starter's versions are in
  `archive/configs/`; `configs/studies/bulk-transfer/README.md` lists what changed.
- Each simulated run reports per transfer: time, goodput, the time the payload alone needs on
  the link, a plain-Ethernet TCP reference, protocol work (messages, rejected writes, empty
  polls, resent chunks), PDI load and ESC RAM use, and a SHA-256 check of the received bytes.
- Not modelled yet: TCP over EoE (EoE carries UDP-like datagrams), VoE, master CPU and OS
  scheduling beyond cycle jitter, several masters' real mailbox schedulers (see the policies).

## Physical sanity bound (not a target)

10,000,000 B cannot cross one 100 Mbit/s link in under 0.8 s even with zero overhead
(`flowlab_core::units::serialization_ns`). The deadline and repetition rate of the real
transfer are still unknown and must not be invented.
