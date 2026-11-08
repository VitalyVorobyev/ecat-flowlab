# flowlab-capture

`flowlab-capture` turns capture files into a `Trace`: an index of every frame and datagram,
decoded once with `ecat-wire`, with frame directions (TX/RX) inferred. It owns pcap and pcapng
reading and writing; there is no libpcap dependency.

API reference: [`flowlab_capture`](../../api/flowlab_capture/index.html) ·
source: [`crates/flowlab-capture`](https://github.com/VitalyVorobyev/ecat-flowlab/blob/main/crates/flowlab-capture)

## Position in the pipeline

| | |
|---|---|
| Depends on | `ecat-wire`, `flowlab-core`, `sha2`, `serde`, `ts-rs`, `thiserror` |
| Used by | `flowlab-analysis`, `flowlab-synth` and `flowlab-sim` (writing), `flowlab-live` (the pcap/pcapng stream of the capture tool), `flowlab-study`, `flowlab-server`, `flowlab-cli` |

```text
bytes ──CaptureFile::parse──▶ CaptureFile ──Trace::build──▶ Trace ──▶ flowlab-analysis
                              packets, interfaces,          frames, datagrams,
                              comments                      direction_mode, provenance
```

The crate interprets nothing beyond one frame: streams, pairing and timing belong to
`flowlab-analysis`.

## Main types

| Item | Role |
|---|---|
| `CaptureFile::parse(bytes)` / `CaptureFile::read(path)` | read a capture held in memory / on disk |
| `CaptureFile` | `format`, the whole file in `bytes`, `packets: Vec<PacketRecord>`, `interfaces: Vec<InterfaceInfo>`, `comments`, `user_application` |
| `PacketRecord` | absolute `ts_ns` (since the Unix epoch), offset and captured length into `bytes`, original length, interface index |
| `CaptureFormat` | `Pcap` or `Pcapng` |
| `CaptureError` | `UnknownFormat`, `Malformed { format, offset, reason }`, `Io` |
| `PcapNgWriter` | writes pcapng (see below) |
| `Trace::build(file, name)` | decode every packet into the index |
| `Trace::read(path)` | read a file and decode it, named after the file (the CLI and the server's *Open path*) |
| `FrameRecord`, `DatagramRecord` | the per-frame and per-datagram index entries |
| `FrameKind` | `Ethercat`, `Other`, `Malformed` |
| `Direction`, `DirectionMode` | per-frame direction, and how directions were inferred |

## Reading pcap and pcapng

| Format | Supported |
|---|---|
| pcap | magic `0xA1B2C3D4` (µs) and `0xA1B23C4D` (ns), either byte order; one interface with the file's link type |
| pcapng | multiple sections, each with its own byte order and interface numbering; Interface Description Blocks with `if_name`, `if_description`, `if_tsresol` (powers of 10 and of 2) and `if_tsoffset` (seconds); Enhanced Packet Blocks, Simple Packet Blocks (no timestamp: `ts_ns` = 0) and the obsolete Packet Block; `opt_comment` of section and interface blocks; `shb_userappl` of the first section |

Unknown block types are skipped. Structural errors (truncated headers, a bad block length, a
bad byte-order magic, a packet on an undeclared interface) are `CaptureError::Malformed`
with the byte offset. Timestamps are converted to ns with integer arithmetic.

## Writing pcapng and the provenance marker

`PcapNgWriter::new(out, user_application, comment, interface)` writes a little-endian section
header carrying `comment` and `user_application`, and one Ethernet interface named
`interface` with `if_tsresol = 9` (ns). `packet(ts_ns, data)` writes one Enhanced Packet Block;
`finish()` flushes and returns the writer.

The writer puts `comment` into the section header as given. FlowLab's generators start it
with the provenance marker (`flowlab:provenance=synthetic`, `…=simulated`), and
`flowlab_analysis::mailbox::write_eoe_pcapng` copies the source capture's provenance into the
EoE export. `Trace::build` sets `trace.provenance` from the first comment (section or
interface) that carries a marker, else `Provenance::Measured`.

## The trace index

`Trace` holds the `CaptureFile`, an `InputRef` (name, size, SHA-256), the provenance,
`start_ts_ns` (earliest packet), `frames`, `datagrams`, `direction_mode` and `errors`
(parse errors by 0-based frame index).

| `FrameRecord` field | Meaning |
|---|---|
| `packet` | index into `file.packets` |
| `t_ns` | time since the earliest packet, ns |
| `captured_len`, `orig_len` | bytes (no FCS) |
| `kind`, `direction` | classification and direction |
| `encapsulation`, `src` | for EtherCAT frames: `Ethernet` or `Udp`, and the source MAC |
| `first_datagram`, `datagram_count` | the frame's slice of `trace.datagrams` |

| `DatagramRecord` field | Meaning |
|---|---|
| `frame` | owning frame index |
| `command`, `index`, `address_raw`, `len`, `wkc`, `irq`, `circulating` | the decoded header and trailer |
| `data_offset` | offset of the datagram *data* within the frame bytes |

Frame number *n* as shown in Wireshark and the UI is `trace.frames[n - 1]`; times in the
index are relative to capture start, which keeps them below 2^53 ns for the TypeScript side.
Accessors: `datagrams_of(&frame)`, `frame_bytes(&frame)`, `datagram_data(&datagram)`,
`duration_ns()`, and `DatagramRecord::address()` for the interpreted address.

Only packets on an Ethernet interface (`LINKTYPE_ETHERNET` = 1) are decoded. A frame whose
datagram chain breaks part-way is `Malformed`; the datagrams before the break stay in the
index.

## Direction inference

`Trace::build` decides one `DirectionMode` for the whole capture:

| Mode | Rule |
|---|---|
| `MacLocalBit` | some source MAC appears both with and without the locally-administered bit (ESCs set it on returning frames). Frames from such a MAC are TX without the bit and RX with it; frames from other MACs stay `Unknown`. |
| `WkcHeuristic` | no MAC split, but some EtherCAT frames carry only zero working counters and others do not. Masters send WKC 0, so all-zero frames are TX and the rest RX. A returning frame that no slave answered is then misread as TX. |
| `Unknown` | neither rule applies, e.g. a capture of one direction only. Every frame's direction is `Unknown`, and the analysis switches off pairing, loss and WKC checks. |

The chosen mode is reported in the capture summary and in the report's assumptions.

## Tests

The unit tests in `trace.rs` build captures with `ecat-wire` and `PcapNgWriter`:

- `pcapng_round_trip_and_index`: written timestamps, provenance from the section comment,
  `MacLocalBit` directions, datagram data slices, a non-EtherCAT frame classified `Other`,
  `duration_ns` and the SHA-256 length;
- `classic_pcap_microseconds`: a hand-built µs pcap reads as `Measured` with `Unknown`
  directions;
- `wkc_heuristic_without_mac_split`;
- `rejects_garbage`: an unknown magic and a truncated pcapng are errors.

Every synthetic and simulated capture in the other crates' tests also passes through this
reader.

## Example

```rust,ignore
use flowlab_capture::{CaptureFile, Direction, Trace};

let file = CaptureFile::read("results/demo.pcapng".as_ref())?;
let trace = Trace::build(file, "demo.pcapng");
println!("{:?}, {:?}, {} frames", trace.provenance, trace.direction_mode, trace.frames.len());
for f in trace.frames.iter().filter(|f| f.direction == Direction::Tx).take(3) {
    for d in trace.datagrams_of(f) {
        println!("{} {:?} wkc={}", d.command, d.address(), d.wkc);
    }
}
```
