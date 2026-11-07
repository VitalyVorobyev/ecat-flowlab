# Architecture

## One pipeline, three sources

```text
  capture file ──┐
  (measured)     │
                 ├─▶ flowlab-capture ──▶ Trace ──▶ flowlab-analysis ──▶ CaptureReport (JSON)
  flowlab-synth ─┤    pcap/pcapng         frame &      streams, timing,      │
  (synthetic)    │    reader, index,      datagram     WKC, utilization,     ├─▶ flowlab analyze
                 │    directions          index        stations, frames      └─▶ flowlab-server ─▶ UI
  simulator ─────┘
  (simulated, M4)
          all encode / decode through ecat-wire
```

Every source produces Ethernet frames that `ecat-wire` can parse. A simulated run, a synthetic
capture and a real capture are therefore analysed by the same code and compared like-for-like;
only their `Provenance` differs. This replaces the starter design's separate abstract
simulator model (see `decisions.md`, ADR-010).

## Crates

| Crate | Owns | Must not |
|---|---|---|
| `ecat-wire` | Ethernet / UDP encapsulation, EtherCAT header, datagram chain, mailbox / CoE / SDO headers, AL states, ESC register names; `FrameBuilder` | do I/O, allocate while parsing, panic on input |
| `flowlab-core` | `Provenance`, `ReportMeta`, `InputRef`, unit constants | depend on anything heavier than serde |
| `flowlab-capture` | pcap (µs/ns, both endians) and pcapng (multi-section, `if_tsresol`, `if_tsoffset`, EPB/SPB/PB) reading; pcapng writing; `Trace` index; TX/RX inference | interpret traffic beyond one frame |
| `flowlab-analysis` | cyclic-stream detection, period / jitter / round trip, WKC expectation, utilization, stations and AL timeline, findings; frame listing and decode | hold global state |
| `flowlab-synth` | deterministic master + slave-line capture generator, `GroundTruth`, JSON Schema of its spec | claim fidelity beyond its documented round-trip model |
| `flowlab-server` | in-memory capture store, REST API, static UI | contain analysis logic |
| `flowlab-cli` | `synth`, `analyze`, `serve`, `schema` | — |

## Analysis model

- **Directions.** ESCs set the locally-administered bit (0x02 of the first octet) in the
  source MAC of frames they return. If a source MAC appears with and without it, frames
  without are TX and with are RX. Otherwise, if some frames carry only zero working
  counters and others do not, all-zero frames are TX (masters send WKC 0). Otherwise
  directions are unknown and pairing, loss and WKC checks are switched off.
- **Pairing.** TX and RX match on (datagram layout, first datagram index). A TX whose key is
  re-sent before a reply arrives, or that has none by the end of the capture (outside a
  50 ms in-flight window), is *unanswered*.
- **Cyclic streams.** Group TX frames by datagram layout (command, address, length). A group
  with ≥ 20 frames and ≥ 60 % of intervals within ±50 % of the median is a stream.
- **Period and jitter.** Cycle numbers come from rounding each interval to the median; the
  period is the least-squares slope of send time over cycle number, so gaps do not bias it.
  *Phase jitter* is each send time's residual from that line; *interval* statistics exclude
  gaps (> 1.5 periods), which are counted as missed cycles instead. For independent send
  errors of σ, phase σ ≈ σ and interval σ ≈ √2 σ — the tests check both.
- **Working counters.** Without the network configuration, the expected value of each
  datagram position in a stream is the most frequent value on returning frames.
- **Utilization.** Wire bytes per frame = max(captured, 60) + 8 preamble + 4 FCS + 12 gap,
  binned over time against a configurable link rate (100 Mbit/s).
- **Stations.** From FP* datagrams; ring positions from APWR to 0x0010; AL requests from
  writes to 0x0120 and reports from reads of 0x0130 (changes only).
- **Mailbox channels.** Writes to the SyncManager registers (0x0800 + 8·n) in mailbox mode give
  each station's "mailbox out" (ECAT writes) and "mailbox in" (ECAT reads) start address;
  APWR-addressed configuration is mapped to stations through the APWR 0x0010 assignment.
  Without configuration in the capture a user hint is used, else a heuristic (an accepted
  FPWR at ≥ 0x1000 whose data parses as a mailbox header). The source is reported per station.
- **Mailbox messages.** A write is accepted when it returns with WKC > 0 (WKC 0: mailbox full);
  a read returns a message when WKC > 0 (WKC 0: empty). Messages are taken from returning
  datagrams; a counter equal to the previous one marks a repeat, which is dropped. Datagrams
  that access a mailbox are excluded from WKC verdicts, and frames made only of them are not
  cyclic streams.
- **Mailbox protocols.** CoE SDO transfers are followed per station as a state machine
  (initiate → segments with alternating toggle → last segment, or abort); FoE per station
  (request → DATA/ACK until a packet shorter than the mailbox); EoE per station and direction
  (fragment 0 → consecutive fragments with matching frame number and offset → last). Durations
  are measured at the capture point, between the frames that carried the first and last message.

Capture timestamps are whatever the capture tool recorded. Host software timestamps add their
own jitter; the UI and report say so rather than correcting for it.

## Server and UI

`flowlab serve` holds loaded captures in memory (`Trace` + `Analysis`), binds to 127.0.0.1
and serves `ui/dist`. The API is listed in `crates/flowlab-server/src/lib.rs`. Frames are
served in pages (default 1000) filtered server-side; the report (with decimated time series,
≤ 2000 points each) is served whole.

The UI (`ui/`) is a studio layout from `@vitavision/workbench` — `AppShell`, `NavRail`,
`StatusBar`, `SplitPane`, `FileDrop` — with `@vitavision/ui` primitives, `@vitavision/charts`
for histograms and time series, and `@vitavision/forms` rendering the synth spec's JSON
Schema (schemars) as the generator form. API types are generated from Rust by ts-rs.

## Operational modes (planned)

| Mode | Source | Status |
|---|---|---|
| ANALYZE | capture files | M1–M3 |
| SYNTH | `flowlab synth` | M0 |
| SIMULATE | discrete-event model emitting frames | M4 |
| PLAN | ESI/ENI + topology → frame layout and budget | M5 |
| OPTIMIZE / COMPARE | sweeps, Pareto, sim vs measured | M6 |
| LIVE | libpcap capture, master adapters, isolated lab only | M7 |
