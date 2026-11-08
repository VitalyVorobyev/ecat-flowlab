# flowlab-analysis

`flowlab-analysis` turns a `Trace` into a `CaptureReport` (the JSON report and the UI's data)
plus per-frame side tables used to list, filter and explain individual frames. Its functions
are pure functions of the trace and the options: the same capture gives the same report,
whether the capture is measured, synthetic or simulated.

API reference: [`flowlab_analysis`](../../api/flowlab_analysis/index.html) ·
source: [`crates/flowlab-analysis`](https://github.com/VitalyVorobyev/ecat-flowlab/blob/main/crates/flowlab-analysis) ·
user guide: [Analysing a capture](../guide/analysis.md), [Mailbox traffic](../guide/mailbox.md)

## Position in the pipeline

| | |
|---|---|
| Depends on | `ecat-wire`, `flowlab-core`, `flowlab-capture`, `flowlab-devices` (ENI `NetworkConfig`); `flowlab-synth` for tests only |
| Used by | `flowlab-cli` (`analyze`, `policy`), `flowlab-server`, `flowlab-live` (each second and the window), `flowlab-study` (capture targets, the service pattern behind `policy`); the tests of `flowlab-sim` |

## Main items

| Item | Role |
|---|---|
| `analyze(&Trace, &AnalysisOptions) -> Analysis` | the whole pass |
| `AnalysisOptions` | link rate, thresholds, mailbox hints, ENI |
| `Analysis` | `report: CaptureReport` plus side tables (below) |
| `CaptureReport` | `meta`, `summary`, `streams`, `utilization`, `stations`, `al_events`, `mailbox`, `findings` |
| `Distribution`, `Histogram` | count, min, max, mean, population σ, p1/p5/p50/p95/p99 and a histogram of a sample |
| `frames::{query, detail}` | frame list pages and single-frame decode |
| `mailbox::{MailboxHint, Channels, Access, MailboxAnalysis, MessageRecord, describe, write_eoe_pcapng}` | mailbox channels, messages and the EoE export |

## Options and thresholds

| `AnalysisOptions` field | Default | Used for |
|---|---|---|
| `link_bps` | 100 000 000 bit/s | utilization |
| `min_stream_frames` | 20 | minimum TX frames of a cyclic stream |
| `jitter_warn_fraction` | 0.10 | Warn when the p99 of \|phase jitter\| exceeds this fraction of the period |
| `gap_factor` | 1.5 | an interval longer than this many periods is a gap (missed cycles, Warn) |
| `regular_share`, `regular_tolerance` | 0.6, 0.5 | a cyclic stream needs this share of its intervals within ± this fraction of the median interval |
| `max_series_points` | 2000 | points per time series (min/max decimation keeps spikes) |
| `histogram_bins` | 40 | target bins per histogram |
| `in_flight_window_ns` | 50 000 000 ns (50 ms) | TX frames this close to the capture end are not counted as unanswered |
| `mailbox_hints` | empty | mailbox addresses for stations without SyncManager configuration in the capture |
| `network` | `None` | the master's ENI: expected working counters, mailboxes, station names |

The stream and gap thresholds are stated in the report's assumptions and finding text. Fixed
in the code: event and transaction lists hold at most 2000 entries; utilization bins are at
least 100 µs wide; the mailbox service pattern needs 10 cycles and reads more than 1.5 frames
per cycle as back-to-back polling (both stated in the assumptions).

## The pass

`analyze` works through the trace in time order:

1. **Layouts.** Each EtherCAT frame's datagram layout, the list of (command, address, length),
   gets an id.
2. **Pairing.** A TX frame waits under the key (layout, first datagram index); the next RX with
   that key is its reply. The pairing layout uses `Command::stable_address`, which drops the
   ADP of position and broadcast datagrams: every slave increments it, so the returning copy
   differs from the sent one. A TX whose key is sent again before a reply, or that has none by the
   end of the capture (outside `in_flight_window_ns`), is *unanswered*.
3. **Mailbox** (see below), first, because mailbox datagrams are excluded from what follows.
4. **Cyclic streams.** Sent frames grouped by layout; a group is a stream when it has
   ≥ `min_stream_frames` frames, its intervals are regular, and it is not made only of mailbox
   accesses. Streams are numbered by frame count, largest first.
5. **Period and jitter.** Each interval is rounded to a whole number of median intervals,
   giving a cycle number per frame; the period is the least-squares slope of send time over
   cycle number, so gaps do not bias it. *Phase jitter* is each send time's residual from that
   line; *interval* statistics exclude gaps (> `gap_factor` periods, default 1.5), which are counted as gaps and
   missed cycles. For independent send errors of σ, phase σ ≈ σ and interval σ ≈ √2 σ.
6. **Round trip.** RX time minus TX time for every paired frame of a stream.
7. **Working counters.** Per datagram position of a stream, the expected value is the ENI's
   `Cnt` when the ENI lists a cyclic command with the same command, address and length, else
   the most frequent value on returning frames. Each `WkcSlot` records its basis
   (`expected_from`: `eni` or `most_frequent`). Mailbox positions are never counted as
   mismatches.
8. **Utilization.** Each frame occupies `wire_bytes(max(orig_len, captured_len))` bytes
   (padding to the 60-byte minimum, 8 bytes preamble and SFD, 4 bytes FCS, 12-byte gap),
   binned over time against `link_bps`, TX and RX separately.
9. **Stations and AL.** Configured-address datagrams give per-station counts (datagrams,
   reads, writes, register areas, RX with WKC 0); APWR to 0x0010 gives ring positions; writes
   to 0x0120 are AL requests and returning reads of 0x0130 with WKC > 0 are AL reports, recorded
   only when the state changes. The largest WKC of a returning BRD is `slaves_responding`.
   With an ENI, stations get their names.
10. **Findings and assumptions**, then `ReportMeta::new(trace.provenance, …)`.

When directions are `Unknown`, every EtherCAT frame is treated as sent: streams and timing
are still computed, pairing, loss and WKC checks are not.

## Findings

Each `Finding` has a `Severity`, a title and a detail that states the evidence and the
threshold. `Warn` and `Defect` are verdicts against a declared threshold; `Info` is context.

| Severity | Finding | Threshold |
|---|---|---|
| Defect | stream frames did not return | any |
| Defect | working-counter mismatches | any, against the stated basis |
| Defect | station reports an AL error | error bit in the last AL status read |
| Defect | SDO transfers violated the protocol | any |
| Warn | missed cycles | any gap > `gap_factor` periods (1.5) |
| Warn | send jitter p99 | `jitter_warn_fraction` × period |
| Warn | malformed EtherCAT frames; aborted SDO transfers; CoE emergencies; failed FoE transfers; EoE frames not reassembled; mailbox error replies; malformed mailbox messages | any |
| Info | no EtherCAT frames; directions unknown or inferred from WKC; no cyclic stream; unfinished SDO transfers; rejected mailbox writes; synthetic capture | — |

## Mailbox

**Channels.** `Channels` maps each station to its "mailbox out" (master → slave) and
"mailbox in" (slave → master) start addresses. The source is reported per station as
`ChannelSource`, in this priority:

| Source | From |
|---|---|
| `SmConfig` | sent writes to the SyncManager registers (0x0800 + 8·n) in mailbox mode with non-zero length: `EcatWrite` is mailbox out, `EcatRead` mailbox in. APWR-addressed configuration is mapped to stations through the APWR 0x0010 assignment. Overrides every other source. |
| `Eni` | the ENI's mailbox of a station without SyncManager configuration in the capture; SM1 is taken to be mailbox in for status polls |
| `Hint` | `MailboxHint`, parsed from `STATION=OUT,IN` (decimal or `0x` hex; CLI `--mailbox`) |
| `Heuristic` | stations still unknown: an accepted FPWR at ≥ 0x1000 whose data parses as a mailbox header defines mailbox out; an FPRD at ≥ 0x1000 of that station returning one defines mailbox in |

`Channels::access(datagram)` classifies a configured-address datagram as `Access::Out` (a write
at the out start), `Access::In` (a read at the in start) or `Access::Status` (a read covering
the mailbox-in SyncManager's status register).

**Messages.** Messages come only from returning datagrams with WKC > 0. A returning write with
WKC 0 is a rejected write (mailbox full); a returning read with WKC 0 an empty read. The header
must fit the datagram and name a known type, else the message counts as malformed. A non-zero
counter equal to the previous one of the same station and direction marks a repeat, which is
counted and dropped. Each kept message becomes a `MessageRecord` with a one-line summary from
`describe`.

**Protocols.** Durations are measured at the capture point, between the frames that carried
the first and last message.

- *CoE*: one open SDO transfer per station: initiate request → initiate response → segments
  with alternating toggle → last segment, or abort. A wrong step, a toggle that does not
  alternate, a response for another index/subindex, or a final size different from the
  announced one is `ProtocolError`; a new initiate request supersedes an open transfer
  (`Superseded`); still open at the end is `Incomplete`. Emergencies and SDO Information
  messages are counted.
- *FoE*: per station, RRQ/WRQ → DATA/ACK. Packet numbers must increase by one. A DATA packet
  that does not fill the mailbox is the last; the ACK of it ends the transfer `ok`, with
  `goodput_bps` = bytes × 8 / duration. ERR ends it `error`; BUSY is counted.
- *EoE*: per station and direction, fragment 0 → consecutive fragments with the same frame
  number and an offset (× 32 bytes) equal to the bytes so far → last fragment. Any break is a
  reassembly error. Reassembled frames are kept with their absolute time in
  `MailboxAnalysis::eoe_packets`.
- *SoE* error responses and mailbox error replies are counted and listed.

`mailbox::write_eoe_pcapng(&analysis.mailbox, provenance, source_name, out)` writes the
reassembled EoE frames as a pcapng that carries the source capture's provenance
(`flowlab analyze --eoe-out`).

**Service pattern** (`service.rs`). With known directions and a cyclic stream, every station
whose mailbox buffers were read or written in at least 10 cycles of stream 0 gets a
`MailboxService`: accesses and frames per cycle, accesses inside cyclic frames, the turnaround
from a mailbox frame's return to the next send within a cycle, and the send time of the cycle's
last mailbox frame as a percentage of the period. Only sent reads and writes of the buffers
count, not status polls. `pattern` names the simulator policy: a median above 1.5 frames per
cycle is `Opportunistic` (budget = 95th percentile of the last send, rounded up; turnaround =
median), anything else `FixedQuota` (95th percentile of accesses per cycle, at least 1). The rule
is added to the assumptions when a pattern is reported.

## Side tables and frames

| `Analysis` field | Per frame / key |
|---|---|
| `pair_of` | paired TX↔RX frame index, or `u32::MAX` |
| `stream_of` | stream id, or `u16::MAX` |
| `wkc_mismatch` | set on a mismatching RX frame and its TX partner |
| `unanswered` | unanswered TX frame |
| `expected_wkc` | (stream id, datagram position) → expected WKC |
| `mailbox` | `MailboxAnalysis`: channels, messages, (frame, datagram) → message, per-frame protocol flags, EoE packets |

The `frames` module reads them. `query(trace, analysis, &FrameFilter, offset, limit)` returns a
`FramePage` (`total`, `offset`, `rows`) of `FrameRow`s in capture order. `FrameFilter`
conditions all have to hold: command mnemonic, direction (`"tx"`, `"rx"`, `"unknown"`),
station, stream (65535 = in no stream), `problems_only` (WKC mismatch, unanswered or
malformed), `include_other`, and `mailbox` (`"any"` or a protocol such as `"coe"`, `"foe"`,
`"eoe"`). `detail(trace, analysis, no)` returns a `FrameDetail` for frame number `no`
(1-based): Ethernet and EtherCAT header fields, labelled byte ranges for the hex view, and a
`DatagramDetail` per datagram with its address and register name, expected WKC, AL state at
0x0120/0x0130 and mailbox role and decoded fields.

## Invariants

- No global state; no I/O except `write_eoe_pcapng` into a caller's writer.
- Every heuristic is written into `ReportMeta.assumptions`; every threshold behind a `Warn` or
  `Defect` is stated in the finding's detail.
- Mailbox channel addresses come from SyncManager configuration before ENI, hints and
  heuristics, and the source is always reported.
- Times in the report are ns since capture start.

## Tests

[`src/tests.rs`](https://github.com/VitalyVorobyev/ecat-flowlab/blob/main/crates/flowlab-analysis/src/tests.rs)
generates captures with `flowlab-synth` and asserts against its `GroundTruth`: period, phase
and interval jitter (σ and √2 σ), missed cycles, round trip (wire time plus slave delays),
lost frames, WKC errors, stations, positions and AL states, mailbox counts, SDO, FoE and EoE
results, hint and heuristic channels without start-up, ENI-based WKC when errors dominate, and
byte-identical JSON across runs. Unit tests in `mailbox.rs` cover a toggle violation,
superseded and orphan aborts, an FoE error and an EoE gap; `stats.rs` covers percentiles,
histogram bins and decimation.

## Example

```rust,ignore
use flowlab_analysis::{AnalysisOptions, analyze};
use flowlab_capture::{CaptureFile, Trace};

// A synthetic capture with known ground truth stands in for a measurement.
let mut bytes = Vec::new();
let truth = flowlab_synth::generate(&flowlab_synth::SynthSpec::example(), &mut bytes).unwrap();
let trace = Trace::build(CaptureFile::parse(bytes).unwrap(), "example.pcapng");
let report = analyze(&trace, &AnalysisOptions::default()).report;

let stream = &report.streams[0];
assert!((stream.period_ns - truth.cycle_time_ns as f64).abs() < 100.0);
assert_eq!(report.meta.source, flowlab_core::Provenance::Synthetic);
```
