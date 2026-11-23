# FlowLab VoE

FlowLab VoE is a bulk transfer protocol of FlowLab's own, carried in VoE messages (vendor
specific over EtherCAT, mailbox type 15). The lab master (`flowlab master transfer --protocol
voe`) and the [emulated slave](../guide/slave.md) implement it; `flowlab_mailbox::voe` holds the
message format both use. A slave stack can implement it to move files or data with less mailbox
traffic than FoE or EoE need.

## What it is for

| | FoE | EoE (UDP) | FlowLab VoE |
|---|---|---|---|
| Bytes per message, 128-byte mailbox | 116 B | 96 B of the tunnelled frame (fragments in 32-byte steps) | 114 B |
| Bytes per message, 1024-byte mailbox | 1012 B | 992 B of the tunnelled frame | 1010 B |
| Further headers | none | Ethernet, IPv4, UDP per tunnelled frame | none |
| Answers per message | one ACK per DATA | none | one ACK per window of DATA messages |
| Next message may be sent | after the previous one's ACK | when "mailbox out" is free | when "mailbox out" is free and fewer than a window are unacknowledged |
| The receiver confirms the payload | no | no | yes: DONE with length and SHA-256 |
| A lost message from the slave | the master asks with the mailbox repeat request | the frame is lost | the master acknowledges what it still expects; the slave goes back to it |

The slave writes one acknowledgement per window instead of one per message, so a slave whose
PDI (its SPI to the ESC) is slow spends fewer accesses on answers. The master needs no read
between two writes while the window has room.

## Messages

Every message is a VoE message: the mailbox header (type 15), the VoE header (vendor ID
`0x00000000`, FlowLab has no ETG vendor ID; vendor type `0x464C`, "FL"), then a little-endian
word with the operation in bits 15–13 and a sequence number in bits 12–0. Numbers are
little-endian.

| Operation (code) | Sent by | Sequence number | Then |
|---|---|---|---|
| OPEN (0) | master | 0 | direction (u8: 0 to the slave, 1 from it), 0 (u8), window (u16), length (u64; 0 from the slave) |
| READY (1) | slave | 0 | window granted (u16, at most the one asked for), length (u64) |
| DATA (2) | sender | the message's number, 0, 1, 2, … modulo 8192 | payload bytes, as many as the mailbox holds |
| ACK (3) | receiver | the number of the next DATA message expected | — |
| DONE (4) | slave | 0 | length (u64), SHA-256 of the payload received (32 bytes) |
| ABORT (5) | either | 0 | reason (u16: 1 refused, 2 protocol), text |

A window is 1–4096 messages. Mailboxes must hold at least 62 bytes, so that DONE fits.

## A transfer to the slave

```text
master                                   slave
OPEN (to slave, window W, length L)  ──▶
                                     ◀──  READY (window W', length L)
DATA 0, DATA 1, …, DATA W'-1         ──▶  (each as soon as "mailbox out" is free)
                                     ◀──  ACK W'
DATA W', …                           ──▶
…
DATA n (the last bytes)              ──▶
                                     ◀──  DONE (L, SHA-256)
```

The slave answers DATA only after every W'-th message, and DONE after the last. The master may
have at most W' DATA messages unacknowledged. A transfer ends when DONE arrives; the master
compares length and SHA-256 with its payload and reports `verified`.

## A transfer from the slave

```text
master                                   slave
OPEN (from slave, window W)          ──▶
                                     ◀──  READY (window W', length L), DATA 0 … DATA W'-1
ACK W'                               ──▶
                                     ◀──  DATA W' … DATA 2W'-1
…
ACK n+1 (after the last DATA)        ──▶
```

The slave puts each message into "mailbox in" as soon as the master has read the previous one.
The master acknowledges after every W'-th message and after the last; the transfer ends when it
has received L bytes and written the last ACK.

## Losses

- **To the slave.** A write to "mailbox out" is confirmed by its working counter and repeated
  until accepted, with the same mailbox counter, so the slave can drop a repeat. DATA therefore
  arrives in order.
- **From the slave.** If the master's read of "mailbox in" does not come back, the slave may
  already have written the next message, and the mailbox repeat request returns that one. The
  master notices the gap in the DATA numbers and sends one ACK with the number it still
  expects; the slave goes back to that message and sends again from there.
- **Lost answers.** If an ACK or DONE does not reach the master, the master writes its last
  message again after its reply timeout. The slave answers a repeated DATA message with an ACK
  of what it expects, or with its DONE again once the payload is complete.
