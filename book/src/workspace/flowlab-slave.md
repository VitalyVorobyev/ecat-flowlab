# flowlab-slave

An emulated EtherCAT slave: an ESC register model with FoE, CoE and EoE behind its mailbox. Used
by `flowlab slave` ([Emulated slave](../guide/slave.md)) on a network interface, and by
`flowlab-master`'s tests, which run EtherCrab against it in one process
([ADR-038](decisions.md#live-capture-and-hardware)).

| | |
|---|---|
| Depends on | `ecat-wire`, `flowlab-core` (the emulator's name), `flowlab-mailbox` (the responder); `pnet_datalink` with the feature `wire` |
| Used by | `flowlab-cli` (feature `slave`), `flowlab-master` (tests) |

## Design

```text
 Ethernet frame ─▶ EmulatedSlave::handle ─▶ Esc::process ─ per datagram ─┬─ physical: registers, SII, AL, SyncManagers
   (EtherType 0x88A4,                         WKC, ADP++                  └─ logical: FMMUs ─▶ process RAM
    source MAC bit 1 clear,
    or set behind an ESC)                                │
                                                           ▼ "mailbox out" full
                                              flowlab_mailbox::responder::Responder ─▶ outbox ─▶ "mailbox in"
 ◀─ the same frame, source MAC bit 1 set
```

- **`EmulatedSlave::handle(frame, now_ns)`** returns the answer to an Ethernet frame (VLAN-tagged
  or not), or `None` for other EtherTypes, malformed frames and frames whose source MAC has bit
  1 set: an ESC sets that bit, so such a frame has passed a slave. With
  `SlaveConfig::behind_esc` those frames are answered, because an upstream ESC passes the
  master's frames on with the bit set. Nothing in it does I/O; `now_ns` drives only the EoE
  send's start.
- **`Esc`** holds 64 KiB of memory: registers below `0x1000`, 8 KiB of process RAM from
  `0x1000`. A datagram is applied in place: its data, working counter and (position, broadcast)
  ADP are rewritten in the frame. Writes go through a read-only filter (identity, DL status, AL
  status, SyncManager status and PDI control bytes) and then trigger the register's behaviour:
  AL control runs the state machine, SII control executes the EEPROM command at once (the
  interface is never busy), a SyncManager write recomputes the mailbox areas and serves the
  repeat request. Mailbox SyncManagers are found from the registers the master wrote (enabled,
  mailbox mode, direction), as an ESC does.
- **The application** is `flowlab_mailbox::responder::Responder`, the one the lab master's
  virtual slave runs in virtual time. It takes each message as soon as its last byte is written;
  answers wait in an outbox and enter "mailbox in" whenever it is free, with the counter cycling
  1–7. The inputs echo the outputs after every logical write.
- **Settings.** `SETTING_MAILBOX_BYTES` (`0x2001:0`) and `SETTING_EOE_SEND` (`0x2002:0`) are
  writable responder settings. After a frame that leaves the ESC in INIT, `EmulatedSlave`
  compares them with the configuration it runs: a change builds a new `Layout` and EEPROM and
  hands them to `Esc::reconfigure`, which keeps the registers (station address) and the
  application's log. A size that does not fit is written back.
- **`sii`** builds the EEPROM (ETG.2010: identity, standard mailbox, protocol word with EoE,
  CoE, FoE and VoE, the
  checksum of words 0–6 with the CRC-8 that EtherCrab checks, and the categories Strings,
  General, FMMU, SyncManager, TxPDO, RxPDO) and the CoE objects of the same mapping, so a master
  maps the process data from either.
- **`wire::serve`** (feature `wire`) opens the interface with `pnet_datalink` (Linux
  `AF_PACKET`, macOS BPF; MIT OR Apache-2.0, safe API, so the crate keeps `unsafe_code =
  forbid`), refuses without the isolated-network acknowledgement, and answers frame by frame on
  one thread, reporting AL changes and a once-a-second tick to a callback. Where the socket
  hands back the host's own frames (BPF, not `AF_PACKET`), `echo::Echoes` keeps each answer for
  100 ms and drops the first received frame with the same bytes, so a slave behind an ESC does
  not answer itself.

## API

| Item | Purpose |
|---|---|
| `EmulatedSlave::new(SlaveConfig)`, `handle`, `al_state`, `station`, `layout`, `log`, `stats`, `responder` | the slave |
| `SlaveConfig` | identity, mailbox and process data sizes, the EoE send's start, whether it sits behind an ESC, the `ResponderConfig` |
| `SETTING_MAILBOX_BYTES`, `SETTING_EOE_SEND` | the settings a master writes over CoE; they apply at the next INIT |
| `Layout` | where the SyncManager buffers are |
| `SlaveStats` | frames, datagrams, messages each way, repeats, AL changes |
| `SlaveError` | `Invalid`, `Refused`, `Io` |
| `EMULATED_ESC_NAME` | the name in its EEPROM (from `flowlab-core`) |
| `wire::{serve, ServeOptions, Event, interface_names}` | on a network interface (feature `wire`) |

## Tests

`src/tests.rs`, frame by frame with `ecat-wire`'s `FrameBuilder`: counting with BRD, station
address by APWR and ADP increment, read-only registers; returning and non-EtherCAT frames
ignored; the EEPROM through the SII interface (identity, mailbox words, protocol word, name
string, erased words, write protection); AL transitions with their status codes and the error
acknowledge; the mailbox one-buffer rules, partial reads, the repeat request; the CoE mapping
objects; LRW/LRD/LWR working counters and the echoed inputs; the EoE send's start and restart;
layout limits. `src/sii.rs` checks the CRC and the PDO entries. EtherCrab against the slave:
[flowlab-master](flowlab-master.md#tests).
