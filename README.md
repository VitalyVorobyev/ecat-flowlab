# ECAT FlowLab

An EtherCAT analysis workbench: load a capture of EtherCAT traffic, and see the cycle, the
jitter, the round trip, the working counters, the bus load and the slaves' state machines —
in a browser UI or as a JSON report. Synthetic captures with known ground truth come built in,
so every number can be checked against what was put in. Simulation and transport studies
(EoE, FoE, PDO bulk transfer) build on the same pipeline.

**Status:** M0–M2 — capture analysis core, CLI, local server and UI. See [the roadmap](docs/roadmap.md).

## Quick start

Requirements: Rust ≥ 1.88 and [Bun](https://bun.sh) ≥ 1.4. No root, no network adapter, no
libpcap, no EtherCAT hardware.

```bash
# Build the UI once, then serve it with a demo capture loaded
(cd ui && bun install && bun run build)
cargo run --release -p flowlab-cli -- synth configs/synth/demo.json -o results/demo.pcapng
cargo run --release -p flowlab-cli -- serve results/demo.pcapng --open
```

The UI runs at <http://127.0.0.1:7878>. Drop any `.pcap` / `.pcapng` onto it, open a path on
this machine, or generate a synthetic capture from the header.

The same analysis on the command line:

```bash
cargo run -p flowlab-cli -- analyze results/demo.pcapng --out results/demo.json
```

```text
demo.pcapng  [SYNTHETIC]  sha256 361afcc447985a0b
  10157 frames (10157 EtherCAT, 0 other, 0 malformed), 30119 datagrams over 5.017 s
  directions: MacLocalBit; 5082 TX, 5075 RX, 5075 paired, 7 unanswered; slaves responding: 6
  stream 0: FRMW 0x1001:0x0910 · LRW 0x00010000 · BRD 0x0130
    4994 frames, period 1.000 ms, |jitter| p99 10.15 µs, missed 6, unanswered 7, rtt p50 16.52 µs
  [DEFECT] Stream 0 (1.000 ms): 7 frames did not return
  [DEFECT] Stream 0 (1.000 ms): 5 working-counter mismatches
  [WARN] Stream 0 (1.000 ms): 6 missed cycles
```

The generator's ground truth for that file (`--truth`) is 7 lost frames, 5 WKC errors and
6 skipped cycles.

## What it analyses

| View | From the capture |
|---|---|
| **Overview** | Headline numbers, findings against declared thresholds, link load over time, datagrams by command |
| **Timing** | Per cyclic stream: least-squares period, send-time jitter against the cycle grid, interval distribution, TX→RX round trip, gaps and missed cycles, working counters per datagram |
| **Frames** | Every frame, filterable by direction, command, stream, station or problems; decoded datagrams (register names, AL states, CoE mailbox heuristics) beside a hex view |
| **Stations** | Configured station addresses, ring positions, reads/writes, no-response counts, AL state over time |

How each number is inferred — and what it assumes — is written into every report
(`meta.assumptions`) and shown in the UI. Notably: expected working counters are the most
frequent observed value (no ENI/ESI is read yet), and TX/RX are told apart by the
locally-administered bit ESCs set in the source MAC of returning frames.

## Provenance

Every result says where its data came from: **Measured** (a capture file), **Simulated**
(the model) or **Synthetic** (`flowlab synth`). They are never mixed into one number.
FlowLab writes the marker into the pcapng section comment of everything it generates.

## Repository

```text
crates/
  ecat-wire/         EtherCAT codec: frames, datagrams, mailbox, CoE, registers. No I/O.
  flowlab-core/      provenance, report metadata, units
  flowlab-capture/   pcap/pcapng reader + writer, indexed trace, direction inference
  flowlab-analysis/  streams, timing, WKC, utilization, stations; frame queries
  flowlab-synth/     deterministic synthetic captures with ground truth
  flowlab-server/    local HTTP API + static UI (axum)
  flowlab-cli/       `flowlab synth | analyze | serve | schema`
ui/                  React 19 + Vite + Tailwind 4 on the @vitavision lab-ui packages
configs/synth/       example generator specs
docs/                architecture, roadmap, decisions, studies/
```

## Development

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
cd ui && bun run lint && bun run typecheck && bun run test:run && bun run build
```

`cargo test` also regenerates the UI's API types in `ui/src/api/gen/` (ts-rs). For UI work,
run `flowlab serve` on :7878 and `bun run dev` in `ui/` (Vite proxies `/api`).

## Safety

FlowLab only *reads* captures. Future live-capture and master backends will require explicit
opt-in, an isolated network and an interface name — never a second master on a running
production segment. `flowlab serve` binds to loopback: it can read any file you can and has
no authentication.

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your
option.
