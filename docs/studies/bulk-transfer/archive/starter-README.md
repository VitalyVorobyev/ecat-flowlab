# ECAT FlowLab

**Repository:** `ecat-flowlab`  
**Project:** ECAT FlowLab — EtherCAT Payload Transport Laboratory  
**Status:** Design handoff / no implementation yet  
**Primary language:** Rust; macOS Apple Silicon for phase 1, Linux/Windows master and Toradex Verdin i.MX8M Plus target for later phases.

## Mission

Build a reproducible **simulation, configuration, measurement, and optimization system** for moving large binary payloads (nominally **10 MB per logical transfer**) over EtherCAT and comparing EtherCAT transports against conventional Ethernet. The simulator must run completely without hardware; the real backends must subsequently work with a PC master and a Toradex-hosted slave **only after slave-controller hardware feasibility is established**.

This is not initially a TwinCAT integration project. TwinCAT is an optional benchmark/interoperability target. The design must not assume TwinCAT, EtherCrab, SOEM, or any proprietary master is available in phase 1.

## Most important physical constraint

A Toradex Verdin i.MX8M Plus's ordinary Ethernet MAC is **not a standard EtherCAT Slave Controller** (ESC). Do not assume that connecting its RJ45 port to a PC running an EtherCAT master turns it into an EtherCAT slave. The credible planned lab configuration uses a compatible **external ESC** (e.g. a LAN9252 development board) accessed through an electrically compatible SPI/other peripheral interface, with an EtherCAT slave application stack. **Verify specific board/module revision, 1.8 V I/O levels, SPI path, firmware, ESI, and bandwidth before purchasing or wiring anything.** Ordinary Linux Ethernet can be used separately for baseline experiments.

## Payload objective

- Transfer opaque arbitrary binary buffers/files, including 10,000,000 bytes (`10 MB`) and 10,485,760 bytes (`10 MiB`). Never confuse the two.
- Send in both directions as independently measured workloads.
- Measure completed **application data** (not Ethernet wire bytes) with integrity checks and bounded memory.
- Explore one-shot transfers, repeated transfers and streams. **Desired transfer deadline and frequency are not yet specified**; do not invent performance targets.
- Compare at least **EoE, PDO-based custom transport, FoE** when supported; investigate **VoE and CoE block/segmented SDO** feasibility rather than promising support. Include plain TCP Ethernet as a critical baseline.

## First commands for Claude

1. Read `CLAUDE.md`, then `docs/system-design.md`, `docs/roadmap.md`, and `docs/hardware-lab.md`.
2. Implement **only Milestone M0–M1** to begin: a working offline simulator with deterministic seeds, report generation, and tests. Avoid premature master/slave drivers.
3. Treat `configs/scenarios/*.json` as *proposed illustrative scenario inputs*, not validated physical configurations.
4. Use sources in `docs/references.md` to check real-world capabilities and licensing; log any new assumptions.

## Proposed repository layout after bootstrap

```text
ecat-flowlab/
  README.md
  CLAUDE.md
  docs/
    system-design.md       # design and dependency boundaries
    requirements.md        # functional and non-functional goals
    roadmap.md             # milestones / decision gates
    transport-study.md     # EoE vs PDO vs mailbox alternatives
    simulation-model.md    # event / queue / physical budgets
    wire-protocol.md       # application transfer protocol
    benchmark-methodology.md
    optimization.md
    hardware-lab.md
    first-sprint.md
    risks-and-decisions.md
    references.md
  configs/scenarios/       # portable JSON scenarios
  crates/                  # proposed; create iteratively
  tests/                   # proposed
  results/                 # ignored by Git; generated data
```

## Key design principles

1. **Simulator results are predictions, not hardware measurements.** Every chart/report labels its source.
2. **Conformance first.** Custom protocols may run *over* EtherCAT PDOs/VoE; do not change the standard EtherCAT wire format.
3. **Hardware constraints are explicit.** Mailbox memory, process-data length, ESC SPI throughput and EtherCAT scheduling are modeled and eventually measured.
4. **Fair comparisons.** Same payloads, verification policy, traffic profiles, error rates, and measurement endpoints for each transport.
5. **Adapt, do not rewrite blindly.** Investigate existing master/slave libraries before implementing low-level EtherCAT control.
6. **Avoid unsafe automation.** Experimental master must never be attached to an operational production EtherCAT network without explicit isolation.
