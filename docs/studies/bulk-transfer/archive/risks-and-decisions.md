# Risks, open questions, and architecture decisions

## Decisions made for startup

- **ADR-001:** Name is **ECAT FlowLab**, repository `ecat-flowlab`.
- **ADR-002:** Rust-first, macOS offline deterministic simulator before real hardware.
- **ADR-003:** Separate application transfer layer, protocol transport, bus scheduler/ESC/PDI model and real drivers.
- **ADR-004:** Benchmark 10 MB as a *logical multi-frame transfer*, not a single EtherCAT frame.
- **ADR-005:** Ordinary Ethernet is a mandatory alternative baseline.
- **ADR-006:** Toradex slave requires verification of ESC hardware; external ESC via PDI is default hypothesis, not present hardware.
- **ADR-007:** No TwinCAT software dependency, but optional TwinCAT interoperability experiment later.
- **ADR-008:** Simulator estimates and real hardware measurements must remain distinguishable in outputs.

## High risks

| ID | Risk | Consequence | Mitigation |
|---|---|---|---|
| R1 | Toradex has no suitable EtherCAT ESC | No real slave from native RJ45 | Select/validate external ESC and stack first |
| R2 | SPI/PDI too slow for desired bulk throughput | Transfer performance far below wire limit | Benchmark PDI; consider HBI or alternate ESC/architecture |
| R3 | Slave DPRAM/mapping too small for large PDO/mailbox | Unsupported configuration | Model actual ESC memory; verify ESI, SII/EEPROM |
| R4 | EtherCAT 100 Mbit/s theoretical and practical bounds | 10 MB transfer deadline missed | Calculate lower bounds and compare Gigabit Ethernet |
| R5 | Master stack lacks EoE/FoE/VoE APIs | Adapter impossible without extension | Verify versions/APIs; choose compatible stack |
| R6 | Third-party stack license conflicts | Cannot distribute desired artifact | Review exact versions/licenses before embedding |
| R7 | Simulated mailbox scheduling unrealistic | Misleading optimization conclusions | Multiple policies; fit to PCAP/scheduler measurements |
| R8 | Bulk traffic interferes with control PDOs | Production safety / determinism | Isolated lab, control traffic constraints, priority and instrumentation |
| R9 | Windows/master user-space timing limits | Unstable throughput/latency | Linux first; measure separately on Windows |
| R10 | Confusing 10 MB with transfer cadence | Wrong technology selection | Ask for deadline and frequency in requirements |

## Critical unknowns

- Exact ESC eval kit + wiring; SPI port and available voltage translation.
- Availability of appropriate LAN9252 Linux driver and SOES EoE/FoE implementations on selected revision.
- Actual master library version and supported protocols (don’t infer from “EtherCAT master” label).
- Is 10 MB a single trigger, every 100 ms, every second, or only once during configuration?
- Must application run inside TwinCAT ecosystem in production?
- If data sent from Toradex to PC, memory/ESC queue asymmetry and backpressure behavior.
- Exact coexistence demands with other slaves and cycle times.

## Decision gates

- D1: Require sustained >10 MB/s? EtherCAT 100 Mb/s physical bound already leaves almost no overhead room. Evaluate alternative dedicated network immediately.
- D2: Slave cannot add ESC or ESI? Keep simulation research, but don't promise real Toradex EtherCAT slave.
- D3: Simulation advantages disappear after calibration? Report that and stop optimizing a false model.
- D4: EoE best measured option? Keep it; optimizing research should not force a custom protocol.
