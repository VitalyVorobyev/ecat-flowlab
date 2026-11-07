# First sprint: concrete agent tasks

## Objective

Run a correct, self-contained simulated 10 MB transfer on a MacBook with no network, raw sockets, hardware or TwinCAT, and generate an auditable report. **Do not implement a real EtherCAT master in the first sprint.**

## Suggested increments

1. Initialize Cargo workspace with `flowlab-core`, `flowlab-sim`, `flowlab-cli` (or a single crate with clearly separated modules, whichever is leaner).
2. Create types for `ByteCount`, `Nanoseconds`, `BitsPerSecond`, `Direction`, `SourceKind`, `TransferOutcome` and config units; validate nonnegative finite parameters.
3. Implement a virtual clock + priority event queue; test order stability, tie handling and fixed seed reproducibility.
4. Implement synthetic `Link` with wire frame serialization and queue; test at zero overhead that upper bound is never violated.
5. Implement configurable `MailboxScheduler` (initial model fixed service quota *explicitly synthetic*); support asymmetry per direction and contention.
6. Create a transfer source generating deterministic pseudo-random binary content, fixed-size chunks and final digest, and a sink verifying every byte in bounded windows.
7. Define JSON scenario and report schemas with `schema_version`, `source:simulation`, `assumptions`, `scenario_id`, `seed`, `payload_bytes`, `stats`, `transfer_complete`, `integrity_verified`.
8. CLI:
   - `flowlab simulate --scenario configs/scenarios/smoke.json --out results/smoke.json`
   - `flowlab simulate --scenario configs/scenarios/large-payload.json --out results/bulk.json`
   - `flowlab compare results/smoke.json results/bulk.json`
9. Unit/property tests for segmentation boundaries, invariants, and report determinism; CI target on macOS/Linux when available.
10. Update README with running commands and a one-paragraph model limitation notice.

## Deliverables

- `cargo test` passing; formatted and linted.
- CLI runs 10 MB without allocating an uncontrolled number of events or a huge output log.
- A valid machine-readable simulation result with no fabricated measured throughput.
- Clear outstanding research items for M2, including transport-specific fragmentation and ESC memory mapping.

## Definition of done

A reviewer can clone repository on an Apple Silicon Mac, run a documented command and independently reproduce complete/integrity-verified simulated transfer and counters. Changing one mailbox parameter materially changes the predicted result in a scenario where that parameter is a known bottleneck. All assumptions are visible in the report.

## Anti-goals

- No speculative real EoE API adapter before checking master libraries.
- No root/admin, Npcap, IOCTL, Torizon BSP modifications or ESC driver now.
- No fictitious 100 Mbit/s EoE sustained goodput promises.
- No GUI-first work.
