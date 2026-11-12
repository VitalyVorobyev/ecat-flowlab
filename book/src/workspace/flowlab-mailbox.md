# flowlab-mailbox

`flowlab-mailbox` is the mailbox vocabulary the simulator, the lab master and the capture
generator share, so that a measured transfer is set up, described and put on the wire exactly
like a simulated one.

API reference: [`flowlab_mailbox`](../../api/flowlab_mailbox/index.html) ·
source: [`crates/flowlab-mailbox`](https://github.com/VitalyVorobyev/ecat-flowlab/blob/main/crates/flowlab-mailbox)

## Position in the pipeline

| | |
|---|---|
| Depends on | `ecat-wire` (the headers), `serde`, `schemars`, `ts-rs`, `sha2` |
| Used by | `flowlab-sim` (re-exports its types as part of `SimSpec`), `flowlab-master` (engine and virtual slave), `flowlab-slave` (the responder), `flowlab-synth`, `flowlab-cli` |

## Items

| Item | What it is |
|---|---|
| `MailboxPolicy` | `fixed_quota { accesses_per_cycle }`, `round_robin { slaves_per_cycle }`, `opportunistic { bus_budget_percent, turnaround_ns }`; default a fixed quota of 2. `Display` gives the wording of `SimResult::policy` and `MeasuredTransfer::policy`; `FromStr` reads the command line's `fixed-quota:N`, `round-robin:N`, `opportunistic:PERCENT[,TURNAROUND_NS]` |
| `TransferDirection` | `master_to_slave`, `slave_to_master` |
| `MailboxProtocol` | `Foe`, `CoeSegmented`, `Eoe { mtu_bytes }`: the protocols both a simulation and a lab transfer run |
| `payload_bytes(seed, offset, out)`, `payload(seed, len)`, `payload_sha256(seed, len)` | the generated payload: a SplitMix64 mix of seed and position, so any slice is computed without the rest |
| `transfer_seed(spec_seed, k)` | the payload seed of transfer `k` of a simulator spec |
| `TransferPlan` | one transfer of a `SimSpec` as a lab run repeats it: protocol, direction, payload size and seed, cycle time, policy and the slave's mailbox sizes (`SimSpec::transfer_plan(k)`; the lab master's `setup_like` turns it into a job) |

## Messages (`msg`)

The layer above `ecat-wire`'s headers. The simulator's endpoints, the lab master's endpoints and
its virtual slave, and the capture generator's conversations build every message here.

| Item | What it is |
|---|---|
| `mailbox_buffer(kind, counter, address, body, len)` | header, data and zero padding to the SyncManager length, so a write reaches the buffer's last byte |
| `next_counter`, `next_eoe_frame_no` | the mailbox counter 1–7 and the EoE frame number 1–15 |
| `foe(opcode, value, tail)`, `foe_capacity(len)` | an FoE packet; data bytes per packet |
| `sdo(service, command, index, subindex, data, tail)`, `sdo_segment(service, command, data)`, `sdo_unused(n)`, `sdo_initiate_capacity(len)`, `sdo_segment_capacity(len)` | CoE SDO initiate and segment messages (segments padded to seven data bytes) and how much payload each carries |
| `FOE_FILE_NAME` (`flowlab.bin`), `SDO_INDEX`, `SDO_SUBINDEX` (`0x2000:00`) | the request defaults |
| `EoeAddresses`, `udp_frame`, `udp_payload`, `EOE_HEADERS` | Ethernet II / IPv4 / UDP frames for EoE, with documentation addresses as defaults |
| `eoe_capacity(len)`, `eoe_fragments(frame, capacity, frame_no)`, `EoeReassembler` | EoE fragments (all but the last a multiple of 32 bytes) and their reassembly, counting broken frames |

FlowLab VoE has its own module, `voe` ([reference](../reference/voe.md)):

| Item | What it is |
|---|---|
| `encode(op, seq, tail)`, `parse(body)` → `Message { op, seq, tail }` | a message: VoE header (vendor `VENDOR_ID`, type `VENDOR_TYPE`), the operation/sequence word, the rest; `parse` returns `None` for another vendor's VoE |
| `open`, `ready`, `done`, `abort` and their `parse_*` | the control messages and their fields |
| `capacity(len)`, `HEADER`, `MIN_MAILBOX`, `DEFAULT_WINDOW`, `MAX_WINDOW`, `SEQ_MOD` | payload bytes per DATA message and the protocol's limits |
| `unwrap_seq(base, seq)`, `classify(expected, seq)` → `Arrival` | 13-bit sequence numbers as message counts; whether a DATA message is the next, a later one (a gap) or a repeat |

The analysis does not use `msg`: its trackers read messages on their own, so that a misreading
here cannot hide behind the same misreading there.

## The slave application (`responder`)

`Responder` is a slave's side of the mailbox at message level: it takes each "mailbox out"
buffer and returns the messages the slave answers with, knowing nothing of time or
SyncManagers. The lab master's virtual slave runs it in virtual time; the emulated ESC
(`flowlab-slave`) runs it as soon as a message arrives.

| Item | What it is |
|---|---|
| `Responder::new(ResponderConfig, out_len, in_len)`, `take_message(buf)` → `Vec<Message>` | FoE write and read, CoE SDO download (normal, segmented) into the transfer object and upload (expedited, normal, segmented) from it, writes and reads of settings, EoE reassembly, FlowLab VoE both ways (ACK per window, DONE with SHA-256, go-back-N after a gap); mailbox error reply for other protocols; a repeated counter (not 0) is dropped |
| `eoe_pending()`, `next_eoe_frame()` | an EoE send of `upload` in UDP frames, one frame's fragments at a time |
| `restart()` | back to INIT: open transfers abandoned, the counter forgotten, the EoE send from its start |
| `reconfigure(out_len, in_len, eoe_send)`, `setting(index, sub)`, `put_setting` | new mailbox sizes and EoE send for a reconfigured slave (log and settings kept); a setting's value |
| `ResponderConfig` | BUSY every n-th FoE packet, counter handling, the transfer object, read-only `objects`, writable `settings`, `upload`, EoE send, MTU and addresses, the largest VoE window |
| `ResponderLog` | files, downloaded objects, EoE frames, broken fragments, VoE payloads received and uploads sent, settings written, messages taken and dropped |

The JSON forms of `MailboxPolicy` and `TransferDirection` are those of `SimSpec`, and their
TypeScript types are exported to the UI like the simulator's.
