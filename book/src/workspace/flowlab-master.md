# flowlab-master

The lab master behind `flowlab master` ([Lab master](../guide/master.md)): a mailbox transfer
engine without hardware dependencies, a virtual slave to test it, and, behind the cargo feature
`hardware`, EtherCrab on a network interface ([ADR-036](decisions.md#live-capture-and-hardware)).

## Design

```text
 TransferJob + TransferOptions (policy = flowlab_sim::MailboxPolicy)
        │
        ▼
 run_transfer ── per cycle: cyclic() ─ policy ─▶ frames of FPWR "mailbox out" / FPRD "mailbox in"
        │                                         (+ SM1 status, repeat request)
        │            Endpoint: FoeWrite, FoeRead, SdoDownload, SdoUpload, EoeSend, EoeReceive
        ▼
 MailboxBus (trait) ──┬── VirtualSlave: ESC SyncManagers + FoE/CoE/EoE responders, virtual time
                      └── hardware::Session (feature `hardware`): EtherCrab Command::fprd/fpwr,
                          SubDeviceGroup::tx_rx as the cyclic exchange, host clock
        │
        ▼
 MeasuredTransfer (provenance measured; the simulator's field names) + per-cycle records
```

- **`MailboxBus`** is the whole contract with the hardware: `fprd(station, register, len)` and
  `fpwr(station, register, data)` returning data and working counter (or `BusError::Lost`), a
  monotonic clock, `sleep_until_ns`, and `cyclic()` for the process data at a cycle's start.
  Helpers on top write "mailbox out", read "mailbox in", and read a SyncManager's register block
  or status byte. Methods take `&self`, so a write and a read run concurrently
  (`futures_lite::future::zip`) as one policy frame.
- **Endpoints** are the master's side of each protocol as message-level state machines, with the
  simulator's message formats (FoE file name `flowlab.bin`, CoE object `0x2000:00`, EoE UDP frames
  with documentation addresses as defaults). The engine adds the mailbox header and counter and
  pads to the SyncManager length, so a write always reaches the buffer's last byte, and a read
  always reads the whole buffer, which is what hands a buffer over on an ESC.
- **Policies** are `flowlab_mailbox::MailboxPolicy`, the type `SimSpec` uses; results name them
  with its `Display`, as `SimResult::policy` does. With one slave, round robin serves it every
  cycle.
- **Retries**: a rejected write is repeated, a lost write keeps its counter, a lost read of
  "mailbox in" triggers the repeat request, a silent slave gets the last message again with a
  new counter, and a repeated counter from the slave is dropped ([Lab
  master](../guide/master.md#retries-and-counters)).
- **`setup_like(&TransferPlan)`** builds the job for a transfer of a simulator spec
  (`SimSpec::transfer_plan(k)`, [flowlab-mailbox](flowlab-mailbox.md)) with the generated
  payload bytes, so measured and simulated SHA-256s match. The crate does not depend on the
  simulator; its tests use it to check the match.
- **Safety** (`safety` module, feature `hardware`): `check_segment(&SegmentCheck, progress)`
  refuses without an interface name and the isolated-network acknowledgement. It then listens
  through `flowlab-live` (dumpcap or tcpdump, EtherCAT filter) and refuses if any EtherCAT frame
  arrives, or if no tool can check unless `no_master_check` is set. On success it returns a
  `QuietSegment`, the only way to obtain one, and `Session::open` requires it. The CLI only maps
  its flags onto `SegmentCheck`.
- **Hardware** (`hardware` module): `Session::open(&QuietSegment, &SessionOptions)` splits EtherCrab's static PDU storage (one
  session per process), starts the TX/RX thread (`tx_rx_task` on a raw socket under Linux and
  macOS, `tx_rx_task_blocking` with Npcap on Windows) and creates the `MainDevice` with
  `RetryBehaviour::None` (a lost frame must reach the engine, not be resent behind its back) and
  no DC static sync. `scan`, `run` and `transfer` initialise the network with
  `init_single_group`; mailbox accesses are `Command::fprd/fpwr` with EtherCrab's WKC check
  mapped back to a working counter. Cycles are paced with a sleep-then-yield wait on the host
  clock. The crate has `unsafe_code = forbid`, so EtherCrab's `release_all` (unsafe) is not
  called: the TX/RX thread ends with the process.

## API

| Item | Purpose |
|---|---|
| `run_transfer(&bus, TransferJob, &TransferOptions)` | run and measure one transfer |
| `TransferJob` | slave name, `MailboxLayout`, `TransferProtocol` (`Foe`, `Coe`, `Eoe`), `Payload` (`Send(bytes)` or `Receive { expected_bytes, expected_sha256 }`) |
| `TransferOptions` | cycle time, policy, `PollMode` (`Read`, `Status`), reply timeout, resends, time limit, first counter, draining, record limit, link rate for the bound, extra assumptions |
| `setup_like(&TransferPlan)` → `SimulatedSetup` | a transfer set up like a simulated one |
| `MailboxBus`, `MailboxLayout`, `Reply`, `BusError`; `write_mailbox`, `read_mailbox`, `read_sm`, `read_sm_status` | the bus |
| `VirtualSlave`, `VirtualSlaveConfig`, `VirtualSlaveLog` | the simulated slave and what it received |
| `MeasuredTransfer`, `RunResult`, `ScanResult` (`network_config()` → ENI), `CycleTiming`, `CycleRecord`, `Summary` | results ([reports](../reference/reports.md#lab-master-results)) |
| `udp_frame`, `udp_payload`, `eoe_fragments`, `EoeReassembler` | EoE helpers |
| `safety::{check_segment, SegmentCheck, QuietSegment}` | the checks before anything is sent (feature `hardware`) |
| `hardware::Session::{open, scan, run, transfer}`, `SessionOptions` | EtherCrab (feature `hardware`); `open` needs a `QuietSegment` |

## The virtual slave

`VirtualSlave` implements `MailboxBus` with an ESC model in virtual time: each datagram advances
the clock by `access_ns`, and the slave application takes a message `processing_ns` after it
arrived and answers at that moment.

- "Mailbox out" accepts a write (WKC 1) only while empty; the buffer becomes the slave's when the
  write reaches its last byte. "Mailbox in" answers a read (WKC 1) only while full and is
  released when the read reaches its last byte.
- SyncManager 0 and 1 register blocks read back with the status bit "mailbox full"; toggling the
  repeat request bit makes the slave put its last message back and toggle its acknowledge bit.
- Responders: FoE write and read (with BUSY on every n-th packet if asked), an SDO server for
  one object (segmented download and upload, expedited for up to 4 bytes, aborts for other
  objects), EoE reassembly and an EoE sender.
- Faults: every n-th datagram lost on its way back (after the slave processed it), BUSY answers,
  and the counter of a message seen before the transfer, which the slave drops a repeat of, as
  SOES does.

## Tests

`crates/flowlab-master/src/tests.rs`, end to end against the virtual slave:

- FoE write byte-identical at the slave for sizes around the packet boundaries (0, one packet,
  exact multiples); FoE read and SDO upload verified by SHA-256 (expedited and segmented);
  segmented SDO download whole at the slave with the expected message counts; an SDO abort ends
  the transfer with its text.
- EoE: frames reassembled at the slave carry exactly the payload; a slave's EoE stream is
  reassembled by the master; an EoE receive needs a byte count.
- A slow slave makes writes rejected and reads empty; fixed quota 1 and 2, round robin and
  opportunistic give different accesses per cycle and cycle counts (opportunistic within its
  budget); status polling reads the buffer only when full.
- Lost frames every 5th, 7th, 11th or 13th datagram: FoE, SDO download and upload still complete
  byte-identical, with repeat requests.
- A first counter the slave saw last is resent with a new one; FoE BUSY resends the packet; a
  silent slave fails after its resends, and the time limit stops a transfer.
- Cycles start on plan in virtual time, record the cyclic exchange, and overrun and skip when
  the work does not fit; stale messages are drained first.
- A result names its policy as the simulator's does; a scan becomes an ENI that `parse_eni` reads
  back; every mailbox transfer of `SimSpec::example()` set up with `setup_like` moves the bytes
  with the simulated result's SHA-256.

`src/protocol.rs` tests EoE fragmentation and reassembly; `src/hardware.rs` (feature
`hardware`) tests the EEPROM protocol word, the WKC mapping and the cycle wait. CI builds the
`master` feature of the CLI with clippy and runs the `hardware` tests on Linux. The EtherCrab
calls themselves need a slave and are not covered.
