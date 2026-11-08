# Getting started

FlowLab is one program, `flowlab`, with the web UI built in. It needs no EtherCAT hardware,
network adapter, root rights or libpcap. Install a release binary, or build it from source.

## Install a release binary

Each [release](https://github.com/VitalyVorobyev/ecat-flowlab/releases) has one archive per
platform. It holds `flowlab` (with the UI inside), the README, the licences and the example
specs in `configs/`.

| Platform | Archive | |
|---|---|---|
| Linux x86-64 (lab PCs) | `flowlab-VERSION-x86_64-unknown-linux-musl.tar.gz` | static: runs on any distribution |
| Linux aarch64 (Toradex Verdin and other 64-bit Arm boards) | `flowlab-VERSION-aarch64-unknown-linux-musl.tar.gz` | static: independent of the image's glibc (Yocto, Torizon) |
| Windows x86-64 | `flowlab-VERSION-x86_64-pc-windows-msvc.zip` | needs no Visual C++ runtime |
| macOS, Apple silicon | `flowlab-VERSION-aarch64-apple-darwin.tar.gz` | |

The Linux binaries are linked statically against musl, so they need no shared library from the
system. They also hold the [lab master](guide/master.md) (`flowlab master`, which sends EtherCAT
frames only with `--isolated-lab-network` on a quiet segment, and needs `CAP_NET_RAW` or root).
The Windows and macOS binaries do not: on Windows the master links against Npcap's `wpcap.dll`,
which every user of the binary would then need. Build it there from source with the Npcap SDK
([Lab master](guide/master.md#building)).

**Linux and macOS**

```bash
tar xzf flowlab-VERSION-x86_64-unknown-linux-musl.tar.gz
cd flowlab-VERSION-x86_64-unknown-linux-musl
./flowlab serve --open                  # UI on http://127.0.0.1:7878, from any directory
./flowlab analyze run.pcapng --out run.json
```

Put `flowlab` on `PATH` (for example in `~/.local/bin`) to call it from anywhere. macOS marks
files downloaded with a browser as quarantined, and the binary is not signed: remove the mark with
`xattr -d com.apple.quarantine flowlab`.

**Toradex Verdin (aarch64)**

```bash
scp flowlab-VERSION-aarch64-unknown-linux-musl.tar.gz root@verdin:   # from the PC
ssh root@verdin
tar xzf flowlab-VERSION-aarch64-unknown-linux-musl.tar.gz
./flowlab-VERSION-aarch64-unknown-linux-musl/flowlab serve
```

`flowlab serve` listens on the module's loopback address only. To open the UI on the PC, forward
the port over SSH: `ssh -L 7878:127.0.0.1:7878 root@verdin`, then browse to
<http://127.0.0.1:7878> on the PC.

**Windows**

Unpack the zip and run, in PowerShell or `cmd`:

```text
.\flowlab.exe serve --open
.\flowlab.exe analyze run.pcapng --out run.json
```

Live capture on Windows uses Wireshark's `dumpcap.exe`: install
[Wireshark](https://www.wireshark.org) together with Npcap, which its installer offers. FlowLab
finds `dumpcap.exe` on `PATH` or in `C:\Program Files\Wireshark`. Interfaces have names like
`\Device\NPF_{GUID}`; `flowlab.exe capture --list` shows them with their descriptions
([Live capture](guide/live.md)). tcpdump does not exist on Windows.

**Builds of a branch.** Every run of the *Release* workflow (on a `v*` tag, on demand, or for a
pull request that changes it) keeps the same archives as workflow artifacts, named
`flowlab-dev-COMMIT-TARGET`.

## Build from source

### Requirements

- Rust ≥ 1.88 (edition 2024) — <https://rustup.rs>
- [Bun](https://bun.sh) ≥ 1.4, to build the UI
- Optional: Wireshark's `tshark`, to cross-check FlowLab's decoding

The workspace builds and its tests pass on Linux, macOS and Windows; CI runs all three.

### Build and start

```bash
git clone https://github.com/VitalyVorobyev/ecat-flowlab
cd ecat-flowlab
cargo ui
```

`cargo ui` installs the UI's dependencies and builds it when the sources changed, serves it on
<http://127.0.0.1:7878> and opens a browser on **Simulate**: pick a preset such as *10 MB
download over SPI*, change the setup, and read what limits the transfers
([Simulation](guide/simulation.md#the-simulate-view)). To analyse traffic, drop a `.pcap` or
`.pcapng` onto the page, open a path on this machine, or start with `cargo ui --demo`.

| Command | Does |
|---|---|
| `cargo ui [--no-open] [--demo] [CAPTURES…] [-- SERVE OPTIONS]` | build the UI if needed and serve it with the given captures, e.g. `cargo ui run.pcapng -- --eni network.xml --port 8080`; `--demo` adds the synthetic demo capture (generated once) |
| `cargo ui-dev [--no-open] [--demo] [CAPTURES…]` | the API server plus Vite's dev server on <http://localhost:5173>, reloading on UI changes |
| `cargo ci` | the checks CI runs: rustfmt, clippy, tests, generated types, rustdoc, UI lint/types/tests/build, the book |
| `cargo site` | the Pages site (start page, findings, book, API docs) in `_site/` |

These are cargo aliases (`.cargo/config.toml`) for the workspace's `xtask` crate. For a shorter
command, install the binary: `cargo install --path crates/flowlab-cli`, then use `flowlab …`.
Built this way, `flowlab serve` reads the UI from `ui/dist` (`--ui-dir`), so it shows the UI only
when started in the repository. To build the UI into the binary, as the releases do, build the UI
first:

```bash
(cd ui && bun install && bun run build)
cargo install --path crates/flowlab-cli --features embed-ui
```

A binary with `embed-ui` serves its built-in UI whenever `--ui-dir` holds no `index.html`.
Cross-compiling, for example for the Verdin, is described under
[Contributing](workspace/contributing.md#releases-and-cross-compiling).

## A first analysis

The commands below use the example specs in `configs/`, which are in the repository and in the
release archives. The demo capture is synthetic: a master with six slaves at 1 ms, with lost frames, working-counter
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

To watch the segment instead, while it runs: `flowlab capture eth1`, or the **Live** view of
`cargo ui -- --live` ([Live capture](guide/live.md)).

If the capture starts after the master configured the mailboxes, give their addresses:
`--mailbox 0x1001=0x1000,0x1080`. If you have the master's ENI file, add `--eni network.xml`, so
working counters are judged against the configuration rather than the most frequent value.

**Safety:** FlowLab only reads files and the output of capture tools. It never sends frames. Record on an isolated lab network, and
never run a second master on a segment that is in use.

## Next

- [The workbench](guide/ui.md): what each view shows.
- [Analysing a capture](guide/analysis.md): how each number is derived.
- [Simulation](guide/simulation.md) and [studies](guide/studies.md): model a segment before you build it.
