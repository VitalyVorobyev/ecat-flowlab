# Hardware-free simulation model

> **Implemented in `flowlab-sim` (M4)** — engine with stable event order, link with full frame
> serialization, ESC mailboxes (1-buffer) and process data (3-buffer), ESC RAM gating, PDI with
> rates, per-access overhead and contention, master policies (fixed quota, round robin,
> opportunistic), EoE (UDP-like), FoE, CoE segmented and a PDO channel with go-back-N, frame
> loss, byte-for-byte and SHA-256 integrity, the physical bound and a TCP reference. **Not yet:**
> TCP dynamics over EoE, VoE, priority/weighted policies, replayed service timestamps, the
> application-level START/DATA/ACK session of `wire-protocol.md`, calibration. The validation
> rules below are tests in `crates/flowlab-sim/src/tests.rs` where implemented.

## Engine

A deterministic, single-process discrete-event simulator is the first deliverable. Use virtual monotonic time and a stable event ordering `(time_ns, priority, insertion_index)`. Seed pseudorandom fault/latency distributions. Avoid `sleep`, timers and platform performance assumptions.

## Events

`Generate`, `Enqueue`, `StartWireFrame`, `FinishWireFrame`, `MasterCycle`, `MailboxServiceRequest`, `MailboxServiceComplete`, `PdiAccess`, `SlaveApplicationProcess`, `DeliverChunk`, `Ack`, `Timeout`, `Retry`, `Drop`, `TransferComplete`. Keep event timeline optional to avoid per-fragment memory blowup during large sweeps.

## Actors

- Transfer source/sink; bounded queue/buffer and CPU delay model.
- Master: configurable PDO schedule, configurable mailbox service policy, optional competing mailbox devices, loop scheduling delay.
- EtherCAT link: physical 100 Mbit/s default (configurable for hypothetical experiments), full frame serialization, inter-frame gap, frame-level error injection.
- Slave ESC: actual configured SyncManager mailbox and process memory, coarse DPRAM accounting, a PDI interface with read/write rates and per-operation overhead.
- Slave host application: conversion of ESC messages to application receiver; processing/queue delays.

## Scheduling policies

- Fixed mailbox service budget per cycle — explicit **synthetic** policy.
- Opportunistic service during spare bus time, with configurable overhead.
- Round-robin across multiple slaves/mailboxes.
- Priority/weighted fairness between process data and mailboxes, subject to EtherCAT state and bus constraints.
- Replay-extracted mailbox service timestamps from PCAP later.

Do NOT assert a universal one-mailbox-per-cycle constraint. Actual scheduling can differ substantially between masters, device states, and traffic directions. The simulator should report sensitivity to the scheduling policy.

## Physical and memory constraints

- Standard EtherCAT rides Ethernet frames with standard Ethernet MTU/frame budget; avoid pretending a 10 MB EtherCAT telegram is possible.
- ESC DPRAM can be small (e.g. LAN9252 has 4 KiB). Memory occupied by SyncManagers/buffering constrains maximum PDO and mailbox configuration.
- SPI between Toradex and ESC can become dominant, especially if serviced via userspace transactions. Model per-transaction latency, transfer-rate asymmetry, contention and kernel scheduling.
- Model the virtual link as 100BASE-TX by default; use true frame lengths and protocol headers rather than only payload bytes.
- TCP/EoE submodel should explicitly include network transport properties (MSS, ACKs, window and loss) or be labeled simplified application tunnel.

## Example rate sanity checks (NOT measured targets)

- 100 Mbit/s *raw* capacity is 12.5 MB/s. A 10,000,000-byte payload cannot cross one 100 Mbit/s link in under 0.8 s even in an impossible zero-overhead case.
- A 10 MB payload at 5 Mbit/s of application goodput takes ~16 s. At 2 Mbit/s it takes ~40 s. These values are not predictions for the new target; only plausibility checks.
- Every experiment should report separately `raw_wire_rate`, `ethercat_frame_rate`, `mailbox_goodput`, `app_goodput`, `verified_transfer_seconds`.

## Validation

- Conservation of bytes/frames: all bytes accounted for in transmission, buffering, drop, completion or error.
- A zero-overhead link case approaches but never exceeds the physical bound.
- Increasing mailbox overhead cannot *improve* throughput in otherwise identical deterministic conditions.
- Model asymmetry and contention without violating memory allocations or wire-rate limits.
- Seed-reproducibility snapshot tests and property-based frame fragmentation/reassembly tests.
- Calibration plan: fit service-event distributions and PDI latencies to physical captures; publish fit residuals and uncertainty.
