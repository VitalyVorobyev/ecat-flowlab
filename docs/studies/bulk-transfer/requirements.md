# Requirements and success criteria

## Functional

- **F-01** Send arbitrary opaque binary payloads; default 10,000,000 bytes, configurable from 0 B up to at least 100 MiB in simulation (bounded chunk/memory strategy).
- **F-02** Both directions; model asymmetric mailboxes, PDI access and service rates.
- **F-03** Deterministic traffic sources: one-shot, periodic, sustained, burst, bidirectional, and concurrent with PDO background load.
- **F-04** Configurable EoE, custom PDO chunk transport and mailbox-based transfer (FoE/VoE/CoE as verified); plus plain Ethernet TCP reference.
- **F-05** A hardware-independent application transfer session with transfer ID, sequence, length, integrity check, acknowledgments, completion and cancellation.
- **F-06** Reproducible simulations with configurable virtual wire speed, frame-size budget, mailbox sizes, service policy, queue depths, PDI throughput, CPU delays, errors and retries.
- **F-07** Export JSON, CSV and human-readable reports with input config, code version, random seed, source=simulation/measured, payload integrity, throughput, wall/virtual time, latency statistics and counters.
- **F-08** Parameter exploration and constrained optimization with best candidate(s), Pareto frontier and reasons candidates failed constraints.
- **F-09** Playback/compare imported measurement traces (PCAP or aggregate CSV), fitting model parameters without hiding residual uncertainty.
- **F-10** Later: real Linux and Windows PC master backends and a Toradex app controlling an **external genuine ESC**, pending preflight.
- **F-11** Safety: isolate live master traffic from production networks, log interface selection and device identification, and require explicit opt-in to activate raw hardware.

## Non-functional

- macOS ARM64 offline operation first; no privileged access for simulation.
- Maintain repeatability and unit-tested framing with corruption/fault injection.
- Keep transfer/application logic independent of EoE/PDO/FoE implementation.
- Do not advertise hard real-time guarantees from desktop scheduling or simulation.
- Respect device EEPROM/ESI configurations, actual SyncManager/process RAM, available memory and endpoint CPU budget.
- Version all scenario/report schemas.
- Record license and platform limits before dependency adoption.

## Acceptance (not invented speed promises)

M1: 10 MB transfer passes integrity check and deterministic regression; test boundary sizes including 0, 1, MTU±1, mailbox±1, 10 MB and 10 MiB. M2: EoE and PDO modeled, test results stable/reproducible and clearly labeled simulated. M3: search algorithm finds best measured-simulation candidate subject to constraints and records baseline. M4: master/ESC preflight verified and physical lab endpoint enumerated. M5: real 10 MB transfer with exact checksum and bidirectional goodput, latency, CPU and network captures; compare against simulator. M6: optimized result compared against unoptimized baseline and ordinary Ethernet transfer with unchanged workload and hardware.

## Open product questions

- Is 10 MB transferred once, on a periodic schedule, or continuously? Required deadline?
- Sender is PC master or Toradex slave? Both?
- Must the new transport remain compatible with a **fixed TwinCAT EoE endpoint**, or is the whole EtherCAT bus replaceable?
- Is strict EtherCAT interoperability mandatory, or is direct Gigabit Ethernet an acceptable solution?
- What is the real number of EtherCAT slaves and the existing cyclic PDO control traffic?
- Do packet loss, exact resume, retransmissions or hard real-time coexistence have special requirements?
