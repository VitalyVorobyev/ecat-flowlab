# ecat-wire

`ecat-wire` is the EtherCAT codec: it parses and encodes the wire layer (Ethernet framing,
the EtherCAT header, the datagram chain, mailbox headers and the protocols they carry,
SyncManager registers and AL states). Every frame FlowLab reads or writes, whether
measured, synthetic or simulated, goes through this crate.

API reference: [`ecat_wire`](../../api/ecat_wire/index.html) ·
source: [`crates/ecat-wire`](https://github.com/VitalyVorobyev/ecat-flowlab/blob/main/crates/ecat-wire)

## Position in the pipeline

| | |
|---|---|
| Depends on | `thiserror`; `serde` behind the default `serde` feature |
| Used by | `flowlab-capture` (decode into the trace index), `flowlab-analysis` (decode mailbox content and frame details), `flowlab-synth` and `flowlab-sim` (encode frames), `flowlab-devices`, `flowlab-mailbox` (messages above the headers), `flowlab-master` (mailbox headers, SyncManager and AL status registers) |

The crate does no I/O and knows nothing about captures, time or analysis.

## Encapsulations

[`parse_frame`](../../api/ecat_wire/fn.parse_frame.html) takes the bytes of one Ethernet II frame
(without FCS) and returns `Ok(None)` for well-formed Ethernet that does not carry EtherCAT,
`Ok(Some(ParsedFrame))` for EtherCAT, and an error when the frame claims EtherCAT but is
malformed.

| Encapsulation | Recognised by | `Encapsulation` |
|---|---|---|
| Raw Ethernet | EtherType `ETHERTYPE_ETHERCAT` = `0x88A4` | `Ethernet` |
| 802.1Q VLAN | one tag (`ETHERTYPE_VLAN` = `0x8100`) before either of the above; the TCI is kept in `EthernetHeader::vlan_tci` | as the inner frame |
| UDP/IPv4 | EtherType `0x0800`, IPv4, protocol 17, source or destination port `UDP_PORT_ETHERCAT` = 34980 (`0x88A4`) | `Udp` |

`ParsedFrame` holds the `EthernetHeader`, the encapsulation, `pdu_offset` (where the EtherCAT
header starts in the frame) and the `EcatPdu`. `MacAddr::locally_administered()` tests bit
`0x02` of the first octet, which ESCs set in the source address of returning frames;
`flowlab-capture` uses it to tell TX from RX.

## EtherCAT header and datagrams

| Item | Layout |
|---|---|
| `EcatHeader` (`ECAT_HEADER_LEN` = 2) | 11-bit length, 1 reserved bit, 4-bit type (`EcatPduType`: `Datagrams` = 1, `NetworkVariables` = 4, `MailboxGateway` = 5, `Reserved(n)`) |
| `Datagram` header (`DATAGRAM_HEADER_LEN` = 10) | command, index, 32-bit address, length word (11-bit length, 3 reserved bits, circulating bit 14, more bit 15), IRQ |
| Trailer (`WKC_LEN` = 2) | working counter |

`EcatPdu::parse` slices the header's `length` bytes as `body` and keeps whatever follows as
`trailing` (minimum-frame padding). `EcatPdu::datagrams()` returns a `DatagramIter` that follows
the `more` flag and yields `Result<Datagram, WireError>`; it stops after the first error. Only
type 1 PDUs are iterated. Each `Datagram` borrows its `data` from the frame and records its
`offset` within the PDU body.

### Commands and address modes

`Command` covers codes 0–14 (`Nop`, `Aprd` … `Frmw`) plus `Unknown(u8)`. It serializes as its
mnemonic (`"LRW"`, or `"CMD15"` for an unknown code) and answers `reads()`, `writes()` and
`address_mode()`. Each slave increments the ADP of position and broadcast datagrams as they
pass (`increments_adp()`). `address_after(address, slaves)` gives the address field of the
returning copy, and `stable_address(address)` the part that is the same in both directions.

| `AddressMode` | Commands | `DatagramAddress` variant |
|---|---|---|
| `None` | NOP, unknown | `Raw { value }` |
| `Position` | APRD, APWR, APRW, ARMW | `Position { adp, ado }`; `position()` = `adp.wrapping_neg()` |
| `Fixed` | FPRD, FPWR, FPRW, FRMW | `Fixed { station, ado }` |
| `Broadcast` | BRD, BWR, BRW | `Broadcast { adp, ado }` |
| `Logical` | LRD, LWR, LRW | `Logical { address }` |

`DatagramAddress::raw()` gives back the 32-bit field; `ado()` the physical offset for the three
physical modes.

## Mailbox

`MailboxHeader` (`MAILBOX_HEADER_LEN` = 6) holds length, address, channel (6 bits), priority
(2 bits), `MailboxType` (4 bits) and counter (3 bits, 0 reserved). The protocol headers that
follow it:

| `MailboxType` | Types in this crate |
|---|---|
| `Error` (0) | `MailboxErrorReply` (service, detail; `detail_text()` for codes 1–9) |
| `Aoe` (1) | type code only |
| `Eoe` (2) | `EoeHeader` (4 bytes): `EoeType`, port, last fragment, time appended / requested, fragment number, `offset_blocks` (size of the whole frame in fragment 0, otherwise offset, both in `EoeHeader::BLOCK` = 32-byte units), frame number |
| `Coe` (3) | `CoeHeader` (2 bytes, `CoeService`); `SdoHeader` (8 bytes: command, index, subindex, 4 data bytes); `SdoCommand`; `CoeEmergency` (error code, error register, 5 data bytes); `SdoInfoHeader` (op code, incomplete, fragments left) |
| `Foe` (4) | `FoeHeader` (6 bytes): `FoeOpcode` (RRQ, WRQ, DATA, ACK, ERR, BUSY) and a 32-bit value; `foe_error_text()` |
| `Soe` (5) | `SoeHeader` (4 bytes): op code, incomplete, error, drive, elements, IDN |
| `Voe` (15) | `VoeHeader` (6 bytes): vendor id, vendor type |

`SdoCommand::decode(byte, request)` needs to know which side sent the byte, because client
and server command specifiers share codes; the CoE service (request or response) says so.
`segment_data_len(mailbox_length, unused)` gives the data bytes of a segment frame (segments of
up to 7 bytes are padded to 7 and declare the unused count). `sdo_abort_text()` names the CiA
301 / ETG.1000.6 abort codes.

## SyncManagers, AL states and registers

- `SyncManagerConfig` is the 8-byte block at `SM_REGISTER_BASE + n · SM_REGISTER_LEN`
  (`0x0800 + 8·n`, `SM_COUNT` = 16): start, length, control, status, activate, PDI control.
  `mode()` returns `SmMode::{Buffered, Mailbox, Reserved}` and `direction()` returns
  `SmDirection::{EcatRead, EcatWrite, Reserved}`. `sync_managers_in(ado, data)` yields the
  blocks wholly contained in one write; the mailbox analysis uses it to learn mailbox
  addresses.
- `AlState` (`Init`, `PreOp`, `Boot`, `SafeOp`, `Op`, `Other`) and `AlStatus` (state plus the
  error bit 4) decode the low byte of AL control (0x0120) and AL status (0x0130).
- `register_info(ado)` names common ESC registers and returns a `RegisterInfo` with start,
  length, name and area. FMMU (0x0600–0x06FF) and SyncManager (0x0800–0x087F) channels are
  named by number; addresses ≥ 0x1000 are "Process/mailbox RAM".

## Encoding

`FrameBuilder::new(dst, src)` starts a raw-Ethernet frame; `push(DatagramSpec)` appends a
datagram and sets `more` on the previous one; `finish()` patches the EtherCAT header and pads
to the 60-byte Ethernet minimum. `push` fails with `WireError::FieldOverflow` when the datagram
would not fit a 1500-byte payload (`remaining()` reports the room left). `DatagramSpec::new`
sets IRQ, WKC and the circulating flag to zero; `with_wkc` sets the counter. Every header
type above has an `encode` that is the inverse of its `parse`.

`wire_bytes(captured_len)` is the link occupancy of one frame:
`max(captured_len, MIN_FRAME_LEN_NO_FCS) + WIRE_OVERHEAD_BYTES`, i.e. padding to 60 bytes plus
8 bytes preamble and SFD, 4 bytes FCS and a 12-byte inter-frame gap. Utilization in
`flowlab-analysis` and link time in `flowlab-synth` both use it.

## Invariants

- **No panics on input.** Every length is checked before it is read; malformed input maps to
  `WireError::{Truncated, LengthMismatch, FieldOverflow}`.
- **No allocation on the parse path.** Parsed structures are `Copy` values or borrow slices of
  the input buffer. Only `FrameBuilder` allocates.
- **Encode/parse symmetry.** Every new structure gets an encode path and a property-test round
  trip.
- **No I/O, no time, no interpretation** beyond one frame or one header.

## Tests

The unit tests in
[`src/tests.rs`](https://github.com/VitalyVorobyev/ecat-flowlab/blob/main/crates/ecat-wire/src/tests.rs)
combine golden bytes with property tests:

| Kind | What it checks |
|---|---|
| Golden | a hand-assembled BRD of AL status (from the ETG.1000.4 layout) parses field by field, and `FrameBuilder` reproduces it byte for byte, padded to 60 bytes; SyncManager, SDO command and mailbox protocol headers against hand-written bytes; UDP encapsulation; `wire_bytes(30) == 84` |
| Errors | a truncated frame gives `LengthMismatch`, a header shorter than its datagram gives `Truncated`; ARP frames return `Ok(None)` |
| Property | `encode_parse_round_trip` (1–5 random datagrams); `parser_never_panics` and `parser_never_panics_on_ethercat_type` on random bytes; `sdo_command_decode_encode_is_canonical`; `mailbox_headers_round_trip` for every mailbox-level header; `sm_slicing_never_panics` |

Mailbox encodings that `flowlab-synth` emits are also checked against Wireshark's EtherCAT
dissector in CI (`scripts/crosscheck-tshark.py`).

## Example

```rust,ignore
use ecat_wire::{Command, DatagramSpec, FrameBuilder, MacAddr, parse_frame};

// A broadcast read of AL status (register 0x0130), as a master sends it every cycle.
let mut frame = FrameBuilder::new(MacAddr([0xff; 6]), MacAddr([0x02, 0, 0, 0, 0, 1]));
frame.push(DatagramSpec::new(Command::Brd, 7, 0x0130 << 16, &[0, 0]).with_wkc(3)).unwrap();
let bytes = frame.finish();

let parsed = parse_frame(&bytes).unwrap().expect("an EtherCAT frame");
let d = parsed.pdu.datagrams().next().unwrap().unwrap();
assert_eq!((d.command, d.index, d.wkc, d.data), (Command::Brd, 7, 3, &[0, 0][..]));
```
