# Getting started

## Requirements

- Rust ≥ 1.88 (edition 2024) — <https://rustup.rs>
- [Bun](https://bun.sh) ≥ 1.4, to build the UI
- Optional: Wireshark's `tshark`, to cross-check FlowLab's decoding

No EtherCAT hardware, network adapter, root rights or libpcap are needed.

## Build and start

```bash
git clone https://github.com/VitalyVorobyev/ecat-flowlab
cd ecat-flowlab
(cd ui && bun install && bun run build)          # once; the server serves ui/dist
cargo run --release -p flowlab-cli -- synth configs/synth/demo.json -o results/demo.pcapng
cargo run --release -p flowlab-cli -- serve results/demo.pcapng --open
```

The UI runs at <http://127.0.0.1:7878>. Drop a `.pcap` or `.pcapng` onto it, open a path on this
machine, or generate a capture from the header. For a shorter command, install the binary:
`cargo install --path crates/flowlab-cli`, then use `flowlab …`.

## A first analysis

The demo capture is synthetic: a master with six slaves at 1 ms, with lost frames, working-counter
errors, skipped cycles and mailbox traffic put in on purpose.

```bash
flowlab synth configs/synth/demo.json -o results/demo.pcapng --truth results/demo.truth.json
flowlab analyze results/demo.pcapng --out results/demo.json
```

```text
demo.pcapng  [SYNTHETIC]  sha256 95d8517efcb8543b
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

The ground truth in `demo.truth.json` lists what the generator put in: 7 lost frames, 5 WKC
errors, 6 skipped cycles, 12 completed and 3 aborted SDO transfers, one 4,096-byte FoE write,
16 EoE frames and 5 rejected mailbox writes. The analysis finds each of them. The FoE goodput
reflects the generator's polling model (one mailbox access per slave per cycle). It is not a
property of any device.

## Your own capture

Record at the master's network interface, or with a tap between master and first slave, for
example with Wireshark or `tcpdump -i eth1 -w run.pcapng ether proto 0x88a4`. Then:

```bash
flowlab analyze run.pcapng --out run.json
flowlab serve run.pcapng --open
```

If the capture starts after the master configured the mailboxes, give their addresses:
`--mailbox 0x1001=0x1000,0x1080`. If you have the master's ENI file, add `--eni network.xml`, so
working counters are judged against the configuration rather than the most frequent value.

**Safety:** FlowLab only reads files. It never sends frames. Record on an isolated lab network, and
never run a second master on a segment that is in use.

## Next

- [The workbench](guide/ui.md): what each view shows.
- [Analysing a capture](guide/analysis.md): how each number is derived.
- [Simulation](guide/simulation.md) and [studies](guide/studies.md): model a segment before you build it.
