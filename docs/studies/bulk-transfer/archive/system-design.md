# System design

## 1. Scope

ECAT FlowLab is a research-grade engineering tool for **large binary transfers over EtherCAT**, with simulation, protocol comparison, reproducible benchmarks and constrained optimization. The first version runs on a MacBook without hardware; the next version runs against an isolated lab with Linux/Windows PC master and a Toradex Verdin i.MX8M Plus **using supported EtherCAT Slave Controller (ESC) hardware**. TwinCAT can be compared later but is not a dependency.

An opaque 10 MB logical payload must be partitioned: neither a standard EtherCAT frame nor an ESC mailbox holds 10 MB. The optimizer must compare completed logical transfers, not fragment speed alone.

## 2. Layered architecture

```text
          Configuration CLI / experiments / optimizer / reports
                             |
            Workload + Transfer Session Protocol
           (chunking / ACK / integrity / retry / flow control)
                             |
                   Transport Strategy trait
            +---------------+---------------+
            |               |               |
      EoE / IP path    Mailbox path      Cyclic PDO path
       (real option)   (FoE/VoE/CoE)     (custom protocol)
            +---------------+---------------+
                             |
                 EtherCAT service model
            (mailbox slots / PDO cycles / state / ESC RAM)
                             |
                 Abstract Link / Backend
           +---------------+---------------+
           |                               |
  Virtual discrete-event model     Real master + slave adapters
  (macOS, hardware-free)           (Linux/Windows; ESC on target)
```

Separately implement **ordinary Ethernet baseline** with TCP/IP and no EtherCAT; do not sneak it into an EoE simulation.

## 3. Major components and interfaces

1. `flowlab-core`: message/session state machine and typed units (bytes, seconds, bps, direction), chunking, ACK, bounds, checksum policy, transfer outcomes.
2. `flowlab-sim`: priority-queue discrete-event engine, virtual clock, simulated ESC/mailbox/process image, master service policies, link serialization and slave-side processing delays. No OS timing assumptions.
3. `flowlab-models`: EoE fragmentation model; FoE, VoE, CoE and PDO models gated by capabilities; ordinary Ethernet reference model.
4. `flowlab-bench`: deterministic payload generators, virtual runs and later real transfer runner; normalized report schema.
5. `flowlab-opt`: parameter sweeps, constraint checker, reproducible ranking, Pareto explorer.
6. `flowlab-cli`: `simulate`, `compare`, `sweep`, `report` first; `inspect-interface`, `discover`, `transfer` in hardware phase.
7. `flowlab-real`: Linux/Windows master backend adapters; Linux ARM64 Toradex application adapter; ESC driver and firmware dependencies isolated here.
8. `flowlab-capture`: optional packet/trace import, timeline reconstruction and calibration of model parameters.

Do not force all these crates immediately. Start with `flowlab-core`, `flowlab-sim`, `flowlab-cli` and split based on actual needs.

## 4. Operational modes

- **SIMULATE**: synthetic event-driven runs; no hardware/root.
- **REPLAY**: derive transport events and timing from trace/CSV; no live traffic.
- **LAB-REAL**: isolated Ethernet adapter, explicit master/ESC selection and safe opt-in; physically transmits messages.
- **COMPARE**: runs and reports across distinct modes while preserving source labels and confidence.
- **OPTIMIZE**: evaluates bounded candidates against declared constraints, not magical auto-tuning of an uninstrumented production bus.

## 5. Models must include

- EtherCAT physical medium typically 100BASE-TX; serialize *full frames* and mandatory gaps rather than equating PHY rate with application goodput.
- Master cyclic PDO allocation (including 1500-byte standard Ethernet payload budget / multiple frames), mailbox service and arbitration; output/input directions independently.
- ESC SyncManager memory/buffer restrictions; supported PDO lengths; ESI and configured mailbox sizes; ESC PDI path (SPI/HBI) bandwidth and burst latency.
- EoE fragments grouped by logical Ethernet frame; reassembly and error handling.
- Optional TCP/IP overhead, ACKs, window behavior and burst sensitivity if simulating EoE over TCP; don't misrepresent a bare EoE payload as equivalent to a TCP application transfer.
- Transfer protocol buffers, sequence numbers, retransmission policy, timeouts and checksums.
- Background cyclic process traffic and competing mailbox use.
- Optional CPU capacity/queue drain and timestamp uncertainty.

## 6. Ownership and security

- The EtherCAT master owns bus control; **never attach a second master to an active TwinCAT segment**.
- ESI and EEPROM content must match actual device identity, topology and memory configuration.
- Master's raw network interface must not be managed by ordinary IP networking during EtherCAT runs when the selected stack requires exclusive access.
- Lab mode is opt-in, requires interface name, slave identity, and explicit acknowledgement of isolated test bus.
- Data transport is not inherently authenticated or confidential. Expose a security profile for application integrity/authentication separately when appropriate.

## 7. Why not a complete master immediately?

The project's research contribution is measurement and optimization across transports. A full master, PLC and ESC are huge side projects. Prefer adapter integration with EtherCrab or another adequately licensed stack after examining feature support. The virtual engine should emulate the relevant behavior faithfully while retaining explicit adjustable assumptions.
