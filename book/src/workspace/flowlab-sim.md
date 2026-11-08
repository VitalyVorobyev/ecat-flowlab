# flowlab-sim

`flowlab-sim` is a deterministic discrete-event model of an EtherCAT segment: a master, the link,
each slave's ESC (SyncManagers, ESC RAM) and its PDI (the slave application's access to the ESC).
It runs bulk transfers over EoE, FoE, CoE or a process-data channel and writes every frame through
`ecat-wire` into a pcapng labelled `simulated`, so a simulated run is analysed by the same code as
a capture. Every result follows from the spec's parameters; nothing it produces is a measurement.

API: [`flowlab_sim`](../../api/flowlab_sim/index.html) ·
source: [crates/flowlab-sim](https://github.com/VitalyVorobyev/ecat-flowlab/blob/main/crates/flowlab-sim)

## Position in the pipeline

| | |
|---|---|
| Depends on | `ecat-wire` (frame building, mailbox/CoE/FoE/EoE headers), `flowlab-core` (`Provenance`, `ReportMeta`), `flowlab-capture` (`PcapNgWriter`), `flowlab-mailbox` (policies, payload, messages) |
| Dev-depends on | `flowlab-analysis` (to analyse its own captures in tests) |
| Used by | `flowlab-study` (studies and what-if, without frames), `flowlab-master` (`MailboxPolicy`, the payload generator for `transfer --like`), `flowlab-server` (`POST /api/simulate`, `/api/sim/run`), `flowlab-cli` (`flowlab simulate`) |

## Main types and functions

| Item | What it is |
|---|---|
| `simulate(spec, capture: Option<W: Write>) -> Result<SimResult, SimError>` | Validate, run, write the pcapng (if a writer is given and `capture.enabled`), return the result |
| `SimSpec` | Seed, link rate, cycle time, master jitter, slaves, mailbox policy, transfers, `max_duration_ms`, faults, capture output, start time |
| `SimSlave` | Process data sizes, `forwarding_delay_ns`, mailbox sizes, `dpram_bytes`, `pdi`, `app_processing_ns` |
| `PdiSpec` | `read_bps`, `write_bps`, `access_overhead_ns`, `mailbox_access` |
| `MailboxAccess` | `whole_buffer` (default) or `message_only` |
| `MailboxPolicy` | `fixed_quota`, `round_robin`, `opportunistic` (tagged by `kind`) |
| `TransferSpec` | `slave`, `direction`, `payload_bytes`, `protocol`, `start_ms` |
| `TransferProtocol` | `eoe { mtu_bytes }`, `foe`, `coe_segmented`, `pdo_channel { bytes_per_cycle, window, retransmit_after_cycles }` |
| `SimFaults` | `frame_loss_probability` |
| `CaptureSpec` | `enabled`, `max_frames` |
| `SimResult`, `TransferResult`, `SlaveResult` | Serialized results, exported to TypeScript by ts-rs |
| `spec_schema()` | JSON Schema (draft 2020-12) of `SimSpec`, served to the UI's form |
| `presets()` → `SimPreset[]` | the scenarios the Simulate view starts from (`GET /api/sim/presets`): the bulk-transfer study's specs, built in, and the example; a test checks that each is valid |
| `MailboxPolicy`, `TransferDirection`, `payload_bytes`, `payload_sha256`, `transfer_seed` | Re-exported from [flowlab-mailbox](flowlab-mailbox.md): the policies, and the deterministic payload generator with its hash and the seed of transfer `k` |
| `SimSpec::transfer_plan(k)` | Transfer `k` as a `TransferPlan` for the lab master (`flowlab master transfer --like` sends the same bytes) |
| `FIRST_STATION` (`0x1001`), `MBX_BASE` (`0x1000`), `MASTER_MAC` | Addressing used in every simulated capture |
| `PDO_CHUNK_HEADER` (8), `PDO_ACK_BYTES` (4) | PDO channel framing, bytes |

Defaults (from `spec.rs`): `link_bps` 100,000,000; `forwarding_delay_ns` 700; `dpram_bytes` 4096;
`app_processing_ns` 20,000; PDI 20,000,000 bit/s both ways with 5,000 ns per access;
`mailbox_policy` `fixed_quota` with 2 accesses per cycle; `mtu_bytes` 1500; `window` 2;
`retransmit_after_cycles` 8; `max_duration_ms` 60,000; `capture.max_frames` 200,000.
`SimSpec::example()` (identical to `configs/sim/example.json`) is a three-slave line with one
bulk endpoint running EoE, FoE, CoE and a PDO channel of 256 kB each.

## Validation

`SimSpec::validate()` runs before every simulation and rejects what the modelled devices cannot do:

| Rule | Limit |
|---|---|
| Slaves | 1 … 0xEFFF |
| `cycle_time_us` | > 0 |
| `link_bps` | ≥ 1 Mbit/s |
| `frame_loss_probability` | in [0, 1) |
| Policy | `accesses_per_cycle` > 0, `slaves_per_cycle` > 0, `bus_budget_percent` in 0..=100 |
| Mailboxes | none, or ≥ 32 bytes in both directions; each ≤ 1484 bytes (one frame); ≥ 42 bytes for EoE (one 32-byte fragment block) |
| PDI | `read_bps`, `write_bps` > 0 |
| ESC RAM | see below |
| Process image | ≤ 1400 bytes over all slaves, PDO channels included (one LRW in this model) |
| Transfers | at least one; `slave` exists; `start_ms` ≥ 0 and finite; `payload_bytes` ≤ 1,000,000,000 |
| Mailbox protocols | the slave has a mailbox; EoE `mtu_bytes` in 68..=1500 |
| PDO channel | `bytes_per_cycle` > 8 (the chunk header); `window` ≥ 1; one channel per slave |

**ESC RAM gating.** SyncManagers use one buffer per mailbox and three buffers per process-data
direction (3-buffer mode). A PDO channel adds `bytes_per_cycle` in its data direction and
`PDO_ACK_BYTES` in the other. The spec is refused when

```text
mailbox_out_bytes + mailbox_in_bytes
  + 3 × (outputs_bytes + channel_out)
  + 3 × (inputs_bytes + channel_in)   >   dpram_bytes
```

The same number is reported as `SlaveResult::dpram_used_bytes`.

## The engine

Virtual time is in ns. Events sit in a binary heap ordered by `(time, insertion index)`, so ties
resolve in scheduling order and a run is a pure function of the spec. The RNG (`flowlab_core::rng::SplitMix64`,
seeded by `seed`) drives only frame loss and master jitter; each transfer's payload has its own seed
derived from `seed` and the transfer index.

**Preamble.** Configuration frames are sent first, 50 µs apart, so that the capture describes
itself: a BRD slave count, APWR station addresses to 0x0010, FPWR SyncManager configuration at
0x0800 for each mailbox slave (control 0x26 for "mailbox out" at `MBX_BASE`, 0x22 for "mailbox
in" right after it), and an FPWR of OP to 0x0120 plus an FPRD of 0x0130 per slave. They are not
part of the timing model. Once they have returned, the first cycle (`t0`) starts on a multiple of
the cycle time at least one cycle after the last preamble frame; `start_ms` counts from `t0`.

**Master cycle.** Each cycle sends one cyclic frame — an LRW over the whole process image at
logical address 0x0001_0000 and a BRD of AL status (0x0130) — followed by the mailbox datagrams
the policy selects, packed into as few frames as fit 1498 bytes of EtherCAT payload. The next
cycle starts at `t0 + (k+1) × cycle` plus a normal deviate of `master_jitter_ns`, clamped to
±40 % of the cycle. The run stops when every transfer has finished or `max_duration_ms` has passed.

**Link serialization.** A frame starts at its send time or, if the link is busy, when the link
becomes free, then holds the link for `wire_bytes(frame) × 8 / link_bps` (preamble, FCS, padding to 60
bytes and inter-frame gap included). A lost frame (probability `frame_loss_probability`) still
occupies the link but reaches no slave and never returns.

**Forwarding delays.** A datagram reaches slave *p* when the bytes up to its end (preamble,
Ethernet and EtherCAT headers, preceding datagrams) are serialized, plus half of the forwarding
delay of each slave 0…*p*. The frame is back at the master after its full wire time (incl. FCS)
plus the sum of all forwarding delays.

**Mailbox policies.**

| Policy | Per cycle |
|---|---|
| `fixed_quota { accesses_per_cycle }` | Up to that many mailbox datagrams (writes and reads together), round-robin across slaves, in one frame after the cyclic frame |
| `round_robin { slaves_per_cycle }` | That many slaves with mailbox work each get one write and one read |
| `opportunistic { bus_budget_percent, turnaround_ns }` | After each mailbox frame returns, another is sent `turnaround_ns` later if it would start within `bus_budget_percent` of the cycle. A frame still in flight when the next cycle starts continues in that cycle's budget; one chain of mailbox frames runs per cycle |

The master writes a slave's "mailbox out" (FPWR at `MBX_BASE`, full SyncManager length) when it
has a message, and reads "mailbox in" (FPRD right after it) only while the protocol expects input.
It learns from the returning working counter whether a write was accepted (WKC 1) or must be
retried (WKC 0, counted in `rejected_writes`), and whether a read found a message (WKC 0 is an
`empty_reads`).

**ESC buffers.** A mailbox write is accepted only while "mailbox out" is empty; a read returns data
only while "mailbox in" is full and empties it. The LRW latches outputs (WKC +2) and returns the
latest inputs the application wrote (WKC +1). Process data is 3-buffered: the application reads
the newest outputs whenever its PDI is free, so intermediate updates can be overwritten unread.

**PDI access model.** The slave application uses its PDI one access at a time. An access of
*n* bytes takes `accesses × access_overhead_ns + n × 8 / rate` and starts when the PDI is free;
mailbox reads, mailbox writes and process-data reads and writes all queue on the same PDI, so a
slow PDI shows up as rejected writes, empty polls and stale process data. With `whole_buffer`
the application moves the whole SyncManager in one access; with `message_only` it moves the
message plus the last byte (which releases or fills the mailbox), in three accesses for a read
and two for a write. After a mailbox read or a process-data read the application needs
`app_processing_ns` before it reacts. `SlaveResult::pdi_busy_percent` is PDI busy time over
simulated time.

## Protocol endpoints

Each mailbox transfer is a pair of message-level state machines (`Endpoint`), one per side.
Mailbox transfers on the same slave run one after another in spec order; a PDO channel runs in
parallel. Receivers check every byte against `payload_bytes` and hash it with SHA-256.

| Protocol | Model |
|---|---|
| EoE | UDP/IPv4 datagrams of `mtu_bytes − 28` payload bytes in EoE fragments; fragments except the last carry a multiple of 32 bytes; frame numbers 1–15. No acknowledgements, no TCP, ARP or IP fragmentation. Documentation addresses (192.0.2.0/24, IANA MACs) |
| FoE | Write: WRQ, ACK 0, then DATA *n* / ACK *n* until a packet shorter than the mailbox capacity. Read: RRQ, then DATA / ACK. File name `flowlab.bin` |
| CoE segmented | SDO 0x2000:00, normal transfer with size indicated: initiate request/response carrying the first chunk, then segments with alternating toggle and the `last` flag; download (master → slave) and upload (slave → master) |
| PDO channel | Chunks with an 8-byte header (sequence u32, length u16, reserved u16) in `bytes_per_cycle` of process data; the receiver returns the next expected sequence (u32) in the opposite direction. At most `window` chunks unacknowledged; after `retransmit_after_cycles` sender updates without progress it goes back to the first unacknowledged chunk (go-back-N) |

## Results

`SimResult` carries `meta` (provenance `simulated`, the spec's SHA-256, the assumptions), the
policy as text, cycle time, link rate, `virtual_time_ns`, `cycles`, `frames`, `wire_bytes`,
`link_utilization_percent`, `all_completed`, the transfers, the slaves, `capture_frames`,
`capture_truncated`, and the `spec` itself, so a result file says what it is a result of. The
spec types derive `TS` as well; the UI edits `SimSpec` directly. Per transfer, `TransferResult` reports:

| Field | Meaning |
|---|---|
| `start_ns`, `first_byte_ns`, `end_ns`, `duration_ns` | Activation, first and last payload byte at the receiver, ns |
| `exchange_ns` | From the return of the frame with the first mailbox message to the return of the frame with the last — the span a capture at the master shows. Mailbox protocols only |
| `goodput_bps` | Payload bits / `duration_ns` |
| `delivered_bytes`, `verified`, `sha256`, `completed` | Integrity: every byte in order and the SHA-256 matches |
| `messages_to_slave`, `messages_from_slave`, `rejected_writes`, `empty_reads` | Mailbox work |
| `retransmitted_chunks`, `chunk_cycles` | PDO channel work |
| `physical_bound_ns` | Payload bits / link rate: no transfer can be faster |
| `ethernet_tcp_reference_ns` | `⌈payload / 1460⌉ × 1538 × 8 / link_bps`: TCP over a dedicated link, MSS 1460, steady state, no loss. A lower bound, not a prediction |

## Assumptions recorded in the result

`SimResult::meta.assumptions` lists, in the result itself: the run is a model; link serialization
with preamble, FCS, padding and gap and loss before the first slave; 1-buffer mailboxes and
3-buffer process data; the PDI access model and `mailbox_access`; a cycle-driven master that
retries on WKC 0 and has no CPU or OS delays beyond the configured jitter; EoE as a UDP/IPv4
tunnel; byte-for-byte and SHA-256 checking; the TCP reference's definition. The UI shows them on
the *Simulate* and *Sim run* views.

## Invariants and boundaries

- Deterministic: own RNG, stable event order. Same spec ⇒ byte-identical capture and result.
- Frames go through `ecat-wire`; the simulator never analyses its own frames. Message spans,
  WKC and timing seen in its capture come from `flowlab-analysis`.
- Every modelling choice that changes results is a spec parameter or an assumption string.

## Tests

`crates/flowlab-sim/src/tests.rs` and `protocol.rs`:

| Test | Checks |
|---|---|
| `example_completes_verified_and_deterministic` | Two runs give identical bytes and JSON; every transfer verified; `duration_ns ≥ physical_bound_ns`; goodput below the link rate |
| `both_directions_survive_frame_loss` | Each protocol, both directions, 2 % loss: completed, verified, `delivered_bytes` = payload (byte conservation) |
| `boundary_payloads` | 0, 1, 7, 8, 503–505, 1472, 1473 bytes for every protocol |
| `slower_pdi_never_speeds_a_transfer_up` | Monotonicity in `access_overhead_ns` |
| `mailbox_policy_matters_and_is_reported` | Quota, opportunistic and shared-quota orderings; policy text |
| `esc_memory_gates_the_configuration` | 1024 bytes of ESC RAM is refused with "ESC RAM" |
| `capture_is_analysed_like_a_measurement` | The pcapng analysed by `flowlab-analysis`: period, SM-config channels, message counts equal the result's, EoE/FoE/SDO byte counts |
| `message_only_pdi_access_is_not_slower_for_short_messages` | `message_only` beats `whole_buffer` on a slow PDI |
| `schema_and_example_agree`, `shipped_specs_parse_and_validate` | Schema draft, example round trip, shipped configs |
| `every_protocol_delivers_boundary_sizes`, `channel_survives_lost_updates` | Endpoint pairs in isolation |

## Example

```rust,ignore
use flowlab_sim::{SimSpec, simulate};

let mut spec = SimSpec::example();
spec.transfers.truncate(1); // the EoE transfer only
let mut pcapng = Vec::new();
let result = simulate(&spec, Some(&mut pcapng))?;
let t = &result.transfers[0];
assert!(t.verified);
assert!(t.duration_ns.unwrap() >= t.physical_bound_ns);
```

From the command line, `flowlab simulate` runs the built-in example; see
[Simulation](../guide/simulation.md) and [Input files](../reference/specs.md).
