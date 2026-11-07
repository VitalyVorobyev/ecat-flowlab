# flowlab-synth

`flowlab-synth` generates deterministic synthetic captures: it plays a master talking to a
line of slaves and writes what a capture at the master's NIC would show, together with a
`GroundTruth` of what it produced. Its job is to give the analysis captures whose cycle time,
jitter, faults, topology and mailbox traffic are known exactly. Every capture it writes is
labelled `synthetic`.

API reference: [`flowlab_synth`](../../api/flowlab_synth/index.html) ·
source: [`crates/flowlab-synth`](https://github.com/VitalyVorobyev/ecat-flowlab/blob/main/crates/flowlab-synth) ·
user guide: [Synthetic captures](../guide/synth.md)

## Position in the pipeline

| | |
|---|---|
| Depends on | `ecat-wire` (`FrameBuilder`), `flowlab-core`, `flowlab-capture` (`PcapNgWriter`), `flowlab-devices` (`NetworkConfig`), `serde`, `schemars`, `thiserror` |
| Used by | `flowlab-cli` (`synth`, `schema synth`), `flowlab-server`, and the tests of `flowlab-analysis` |

It is not a simulator of slave behaviour or bus timing: the round trip is a fixed model
(below). The discrete-event model lives in [flowlab-sim](flowlab-sim.md).

## Main items

| Item | Role |
|---|---|
| `SynthSpec` (+ `SlaveSpec`, `MailboxSpec`, `FaultSpec`) | the input; `deny_unknown_fields` |
| `SynthSpec::example()` / `validate()` | the demo line; value range checks |
| `generate(&spec, out) -> Result<GroundTruth, SynthError>` | writes a pcapng into any `io::Write` |
| `GroundTruth`, `MailboxTruth` | what was generated |
| `network_config(&spec) -> NetworkConfig` | the ENI subset a master would use for the spec |
| `spec_schema()` | JSON Schema (draft 2020-12) of `SynthSpec`, rendered as the UI's generator form |
| `MASTER_MAC`, `FIRST_STATION`, `LOGICAL_BASE`, `MBX_LEN`, `SM0_START`, `SM1_START`, `MISSING_OBJECT`, `ABORT_NO_OBJECT`, `EOE_DST`, `EOE_SRC`, `EOE_ETHERTYPE` | constants the generated network uses, so tests can refer to them |

## The spec

| Field | Default | Meaning |
|---|---|---|
| `seed` | required | PRNG seed |
| `description` | `""` | stored in the capture comment |
| `slaves` | required | ring order; each `name`, `outputs_bytes`, `inputs_bytes`, `mailbox` (default `true`) |
| `cycle_time_us` | required | nominal cycle, µs |
| `cycles` | required | cycles in the operational phase |
| `tx_jitter_ns` | 0 | σ of the master's send-time jitter (normal, clamped to ±40 % of a cycle) |
| `slave_delay_ns` | 700 | forwarding delay per slave, both directions together |
| `include_startup` | `true` | emit the start-up sequence |
| `sdo_uploads_per_slave` | 2 | expedited uploads per mailbox slave in PREOP |
| `distributed_clocks` | `true` | add an FRMW of the DC system time to every cycle |
| `start_unix_s` | 1 767 225 600 (2026-01-01T00:00:00Z) | time of the first frame |
| `mailbox` | see below | mailbox traffic beyond the expedited uploads |
| `faults` | all 0 | fault probabilities per cycle |

`validate` rejects an empty or oversized slave list, a zero cycle time, a process image above
1400 bytes, probabilities outside [0, 1] (and a write-reject probability of 1), EoE frame sizes
outside 60–1514 bytes, more than 1000 response-delay polls, SDO sizes above 1 MB and FoE sizes
above 16 MB (decimal).

## What a capture contains

**Start-up** (when `include_startup`), one exchange every 200 µs: BRD of 0x0000 to count slaves;
APWR of 0x0010 assigning station `FIRST_STATION + position` (0x1001, 0x1002, …); FPRD of the ESC
information; for mailbox slaves an FPWR at 0x0800 configuring SM0 (mailbox out, 0x1000,
128 bytes) and SM1 (mailbox in, 0x1080, 128 bytes); INIT → PREOP (FPWR of 0x0120, then two FPRD
rounds of 0x0130 reporting the old and the new state); the PREOP mailbox conversations, one
frame per access; FMMU 0 per slave (FPWR at 0x0600); PREOP → SAFEOP → OP.

**Cyclic phase**, starting one cycle after start-up: every cycle a frame of an FRMW of the DC
system time (0x0910) at the first station (WKC = slave count), an LRW at `LOGICAL_BASE` over the
process image (outputs, then inputs; WKC = Σ over slaves of 2 with outputs + 1 with inputs) and
a BRD of AL status (WKC = slave count, state OP). Returning frames carry the master MAC with the
locally-administered bit set, as ESCs do. Their datagrams differ from the sent ones in working
counter and data, and, for position and broadcast commands, in the ADP field, which every
slave increments (`Command::address_after`).

**Round trip.** An RX frame follows its TX after
`serialization_ns(wire_bytes(tx_len), 100 Mbit/s) + slaves × slave_delay_ns`.

**Mailbox conversations.** Each mailbox slave has one conversation, a queue of steps, for the
whole capture, so its mailbox counters (1–7) continue from PREOP into OP. A *send* is an FPWR of
the 128-byte buffer at SM0; with `mailbox_write_reject_probability` it returns WKC 0 (mailbox
full) and is retried. A *receive* is an FPRD at SM1 that returns WKC 0 (empty poll) until the
slave's message is ready, `response_delay_polls` ticks after the request was accepted;
consecutive messages from the slave are one tick apart. In OP, one acyclic frame half a cycle
after each cyclic frame carries one access per busy slave; work left at the end of the cyclic
phase continues at the same cadence.

| `MailboxSpec` field | Default | Generated |
|---|---|---|
| `response_delay_polls` | 2 | empty polls before a response |
| `segmented_upload_bytes` | 0 | PREOP upload of 0x2000: normal up to 112 bytes, segmented beyond |
| `download_bytes` | 0 | PREOP download to 0x2001: expedited up to 4 bytes, else normal + segments |
| `aborted_uploads` | 0 | PREOP uploads of `MISSING_OBJECT` (0x5FFF), aborted with `ABORT_NO_OBJECT` (0x0602_0000) |
| `foe_write_bytes` | 0 | OP FoE write of `firmware.bin` (first mailbox slave), 116-byte DATA packets, ended by a shorter or empty packet |
| `eoe_frames_to_slave`, `eoe_frames_from_slave` | 0 | OP EoE frames (first mailbox slave), `EOE_DST`/`EOE_SRC`, EtherType 0x88B5 |
| `eoe_frame_bytes` | 590 | size of each tunnelled frame |
| `emergencies` | 0 | OP CoE emergencies 0xFF00 + n from the last mailbox slave |

## Faults

| `FaultSpec` field | Effect |
|---|---|
| `lost_frame_probability` | the cyclic frame does not return (TX without RX) |
| `wkc_error_probability` | the cyclic frame returns with the LRW WKC 1 or 2 too low |
| `skipped_cycle_probability` | the master sends nothing in that cycle |
| `mailbox_write_reject_probability` | a mailbox write finds the mailbox full and is retried |

Mailbox faults draw from a separate PRNG stream, so changing mailbox settings does not move the
cyclic faults.

## Determinism and provenance

The generator uses its own PRNG (xoshiro256\*\* seeded by splitmix64, Box–Muller for normal
draws) and no clock, so the same spec gives a byte-identical capture on every run, independent
of dependency upgrades. The pcapng section comment is
`flowlab:provenance=synthetic; generator=flowlab-synth <version>; seed=<seed>; <description>`,
on an interface named `synthetic0`; `Trace::build` reads it back as `Provenance::Synthetic`.

## Ground truth and the ENI

`GroundTruth` records the seed, slave count, cycle time and configured jitter, frame counts
(total, TX, RX, start-up, cyclic TX), skipped cycles, lost frames, WKC errors, the healthy LRW
WKC, process image size, SDO uploads, `first_cyclic_ns`, and `MailboxTruth` (messages each way,
empty polls, rejected writes, SDO ok/aborted/segmented/segments, FoE transfers and bytes, EoE
frames and bytes each way, emergencies).

`network_config(&spec)` describes the same network as a `flowlab_devices::NetworkConfig`:
stations and positions, process data sizes, SM0/SM1 mailboxes (CoE, FoE, EoE) and the cyclic
commands with their expected working counters. `flowlab synth --eni FILE` writes it with
`flowlab_devices::write_eni`.

## How analysis tests use it

The tests of `flowlab-analysis` generate a capture, analyse it and compare against the truth
the generator returns, never against hand-copied numbers: `streams[0].frames ==
truth.cyclic_tx_frames`, `unanswered == truth.lost_frames`, WKC mismatches against
`truth.wkc_errors`, mailbox counts against `MailboxTruth`. Some tests change the spec to
probe one rule: no start-up (no SyncManager configuration, so hints, heuristic or ENI must
supply the mailboxes), or 70 % WKC errors (the most frequent WKC is wrong, the ENI from
`network_config` is right).

The crate's own tests check byte-identical output for equal specs and different output for a
different seed, that frame counts in the truth match the parsed capture, that bad specs are
rejected, and that the schema is draft 2020-12.

## Example

```bash
cargo run -p flowlab-cli -- synth configs/synth/demo.json -o results/demo.pcapng \
    --truth results/demo.truth.json --eni results/demo.eni.xml
```

For `configs/synth/demo.json` it reports
`wrote 10859 frames … (synthetic, seed 42): 4994 cyclic, 7 lost, 5 WKC errors, 6 skipped cycles`,
and the truth file contains, among others:

```json
{ "cycle_time_ns": 1000000, "cyclic_tx_frames": 4994, "lost_frames": 7, "wkc_errors": 5,
  "expected_lrw_wkc": 10, "process_image_bytes": 68, "first_cyclic_ns": 35800000 }
```

```rust,ignore
let spec = flowlab_synth::SynthSpec::example();
let mut bytes = Vec::new();
let truth = flowlab_synth::generate(&spec, &mut bytes)?;
let eni_xml = flowlab_devices::write_eni(&flowlab_synth::network_config(&spec));
```
