# ECAT FlowLab

An EtherCAT analysis workbench: load a capture of EtherCAT traffic, and see the cycle, the
jitter, the round trip, the working counters, the bus load and the slaves' state machines —
in a browser UI or as a JSON report. Synthetic captures with known ground truth come built in,
so every number can be checked against what was put in. Simulation and transport studies
(EoE, FoE, PDO bulk transfer) build on the same pipeline.

**Status:** M0–M3 — capture analysis core, mailbox analysis, CLI, local server and UI. See
[the roadmap](docs/roadmap.md).

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
demo.pcapng  [SYNTHETIC]  sha256 8adbd44c3ea96bc3
  10859 frames (10859 EtherCAT, 0 other, 0 malformed), 30823 datagrams over 5.035 s
  directions: MacLocalBit; 5433 TX, 5426 RX, 5426 paired, 7 unanswered; slaves responding: 6
  utilization TX mean 1.33 %, peak 7.20 % (bins of 10.000 ms)
  stream 0: FRMW 0x1001:0x0910 · LRW 0x00010000 · BRD 0x0130
    4994 frames, period 1.000 ms, |jitter| p99 10.15 µs, missed 6, unanswered 7, rtt p50 16.52 µs
  stations: 0x1001 0x1002 0x1003 0x1004 0x1005 0x1006
  mailbox: 3 stations; messages CoE 24↓/25↑, EoE 56↓/56↑, FoE 37↓/37↑
    SDO: 15 transfers, 12 ok, 3 aborted, 0 unfinished, 0 protocol errors; duration p50 3.000 ms
    FoE write 'firmware.bin' station 0x1004: 4096 B in 36 packets, ok, 219.9 kbit/s
    EoE: 16 frames reassembled (4720 B to slaves, 4720 B from slaves), 0 errors
  [DEFECT] Stream 0 (1.000 ms): 7 frames did not return
  [DEFECT] Stream 0 (1.000 ms): 5 working-counter mismatches
  [WARN] Stream 0 (1.000 ms): 6 missed cycles
  [info] Synthetic capture
  [WARN] 3 SDO transfers aborted
  [WARN] 1 CoE emergency
  [info] 5 mailbox writes rejected
```

The generator's ground truth for that file (`--truth`) is 7 lost frames, 5 WKC errors,
6 skipped cycles, 12 completed and 3 aborted SDO transfers, one 4096-byte FoE write, 16 EoE
frames and 5 rejected mailbox writes. The FoE goodput is a property of the generator's
polling model (one mailbox access per slave per 1 ms cycle, response after two polls), not
a measurement of any device.

`--eoe-out eoe.pcapng` writes the Ethernet frames tunnelled by EoE as their own capture.
When a capture starts after the mailboxes were configured, give their addresses with
`--mailbox 0x1001=0x1000,0x1080` (repeatable; also on `flowlab serve`).

## What it analyses

| View | From the capture |
|---|---|
| **Overview** | Headline numbers, findings against declared thresholds, link load over time, datagrams by command |
| **Timing** | Per cyclic stream: least-squares period, send-time jitter against the cycle grid, interval distribution, TX→RX round trip, gaps and missed cycles, working counters per datagram |
| **Frames** | Every frame, filterable by direction, command, stream, station, mailbox protocol or problems; decoded datagrams (register names, AL states, mailbox messages) beside a hex view |
| **Stations** | Configured station addresses, ring positions, reads/writes, no-response counts, AL state over time |
| **Mailbox** | Per-slave mailbox SyncManagers and their use (accepted/rejected writes, empty reads, polling cadence); traffic per protocol; CoE SDO transfers with outcome, abort codes and duration; emergencies; FoE transfers with goodput; EoE frames reassembled (downloadable as pcapng) |

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

`cargo test` also regenerates the UI's API types in `ui/src/api/gen/` (ts-rs). With Wireshark's
`tshark` installed, `scripts/crosscheck-tshark.py CAPTURE REPORT.json` compares FlowLab's mailbox
decoding with Wireshark's EtherCAT dissector (CI runs it on the demo capture). For UI work,
run `flowlab serve` on :7878 and `bun run dev` in `ui/` (Vite proxies `/api`).

## Safety

FlowLab only *reads* captures. Future live-capture and master backends will require explicit
opt-in, an isolated network and an interface name — never a second master on a running
production segment. `flowlab serve` binds to loopback: it can read any file you can and has
no authentication.

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your
option.
