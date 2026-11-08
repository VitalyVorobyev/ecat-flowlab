# flowlab-mailbox

`flowlab-mailbox` is the mailbox vocabulary the simulator and the lab master share, so that a
measured transfer is set up, and described, exactly like a simulated one.

API reference: [`flowlab_mailbox`](../../api/flowlab_mailbox/index.html) ·
source: [`crates/flowlab-mailbox`](https://github.com/VitalyVorobyev/ecat-flowlab/blob/main/crates/flowlab-mailbox)

## Position in the pipeline

| | |
|---|---|
| Depends on | `serde`, `schemars`, `ts-rs`, `sha2` |
| Used by | `flowlab-sim` (re-exports its types as part of `SimSpec`), `flowlab-master`, `flowlab-cli` |

## Items

| Item | What it is |
|---|---|
| `MailboxPolicy` | `fixed_quota { accesses_per_cycle }`, `round_robin { slaves_per_cycle }`, `opportunistic { bus_budget_percent, turnaround_ns }`; default a fixed quota of 2. `Display` gives the wording of `SimResult::policy` and `MeasuredTransfer::policy`; `FromStr` reads the command line's `fixed-quota:N`, `round-robin:N`, `opportunistic:PERCENT[,TURNAROUND_NS]` |
| `TransferDirection` | `master_to_slave`, `slave_to_master` |
| `MailboxProtocol` | `Foe`, `CoeSegmented`, `Eoe { mtu_bytes }`: the protocols both a simulation and a lab transfer run |
| `payload_bytes(seed, offset, out)`, `payload(seed, len)`, `payload_sha256(seed, len)` | the generated payload: a SplitMix64 mix of seed and position, so any slice is computed without the rest |
| `transfer_seed(spec_seed, k)` | the payload seed of transfer `k` of a simulator spec |
| `TransferPlan` | one transfer of a `SimSpec` as a lab run repeats it: protocol, direction, payload size and seed, cycle time, policy and the slave's mailbox sizes (`SimSpec::transfer_plan(k)`; the lab master's `setup_like` turns it into a job) |

The JSON forms of `MailboxPolicy` and `TransferDirection` are those of `SimSpec`, and their
TypeScript types are exported to the UI like the simulator's.
