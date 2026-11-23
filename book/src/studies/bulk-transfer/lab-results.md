# LAN9252 on a Verdin iMX8MP

All figures on this page are **measured**, on one board in November 2026:
- a Toradex Verdin iMX8M Plus (Torizon OS, Linux spidev) on a carrier board with an on-board
  Microchip LAN9252, on SPI at 20 MHz;
- a Mac with a USB 3 Ethernet dock as the master, cabled directly to the slave's EtherCAT IN
  port (no coupler);
- FlowLab's own master on the Mac ([`flowlab master`](../../guide/master.md));
- on the board, either FlowLab's slave application
  ([`flowlab pdi-slave`](../../guide/pdi-slave.md)) or the board's original slave application.

Only FlowLab's own application's figures are given here.

They answer one question: how much payload can TCP over EoE and a bulk mailbox protocol carry
through this ESC, and what limits it?

## Summary

| What | Result |
|---|---|
| TCP over EoE, 1 MiB each way, 1024-byte mailboxes | **407 kB/s down, 388 kB/s up**, SHA-256 checked |
| FlowLab VoE, 1 MB each way, 1024-byte mailboxes | 340 kB/s to the slave, 398 kB/s from it |
| 1 KiB process RAM write over SPI | 4.67 ms → **1.94 ms** with split writes |
| Cost per spidev transfer | about 40 µs, more than its bytes at 20 MHz |
| Highest working SPI clock | 25 MHz; from 30 MHz on, reads come back shifted by one bit |

What limits it is not the 100 Mbit/s line and not the ESC: it is the number of SPI transfers
the slave's application spends per mailbox message, each of which costs the Linux SPI driver
about 40 µs. The access pattern of the slave application therefore matters more than the
mailbox size.

## SPI access

`flowlab pdi-bench --allow-writes --area 0x1800+1024 --sizes 64,1024 --no-al-event`, 30
repetitions per size and direction, 20 MHz, READ instruction, ESC in INIT. Medians; the
`--spi-tuning` values are explained under [SPI options](../../guide/pdi-slave.md#spi-options).

| `--spi-tuning` | read 64 B | read 1 KiB | write 64 B | write 1 KiB |
|---|---|---|---|---|
| `stack` (as slave stacks access the LAN9252) | 175 µs | 2.02 ms | 329 µs | 4.67 ms |
| `split` | 180 µs | 2.00 ms | 193 µs | 1.97 ms |
| `noabort` | 148 µs | 1.97 ms | 301 µs | 4.19 ms |
| `batch` | 188 µs | 2.12 ms | 348 µs | 4.33 ms |
| `fifo` | 174 µs | 2.00 ms | 328 µs | 4.20 ms |
| `split,noabort` (`fast`) | 151 µs | 1.96 ms | 164 µs | 1.94 ms |

At 25 MHz the `fast` figures are the same (1.97 ms and 1.95 ms for 1 KiB); the clock is not
what limits a 1 KiB access, the number of transfers is. At 30 MHz and above (READ or FAST READ)
`BYTE_TEST` reads 0xC3322110 instead of 0x87654321: the expected value shifted right by one
bit, a sampling-edge problem of the board's SPI path, not of the LAN9252, which specifies READ
up to 30 MHz and FAST READ up to 80 MHz.

Why the tuning matters:

- **Split writes.** The i.MX ECSPI driver moves a transfer longer than 64 bytes by DMA. For a
  process RAM write, a slave stack sends the 3-byte instruction and address and up to 64 bytes
  of FIFO data in one transfer, which then goes by DMA and costs more than it saves. Sending
  the header as its own transfer keeps every transfer at 64 bytes or less (programmed I/O) and
  more than halves a 1 KiB write, with the same bytes on the wire.
- **No abort.** Slave stacks abort the previous FIFO command (and poll for it) before each new
  one. When the previous command in that direction finished, the abort only costs transfers.
- **Batching gives nothing here.** Several transactions in one spidev call need `cs_change`,
  and the kernel's default delay after it (10 µs) uses up the time saved.
- **Partial FIFO fills give nothing.** The write FIFO is never the bottleneck at 20 MHz.

## Mailbox access

FlowLab's slave application, measured with FlowLab VoE (1 MB, SHA-256 checked) and with TCP over
EoE, moves a mailbox message in fewer accesses than a whole-buffer stack does:

- **AL Event Request drives the poll.** SyncManager events (AL Event bits 8 and 9, enabled by
  the master in SyncManager control bit 5) say that "mailbox out" was written or "mailbox in"
  read, so an idle poll is one register read. The SyncManager status registers are the
  fallback, and are read every 64th poll.
- **Message-only buffer access.** The mailbox header gives the message length. The
  application reads the message and then only the buffer's last DWORD, whose read frees the
  buffer. It writes an answer the same way: the message, then the last DWORD, whose write
  hands it to the master. A 6-byte acknowledgement then costs two short writes instead of a
  whole 1 KiB buffer.

## TCP over EoE

[TCP over EoE](../../guide/tcp-bench.md) through `flowlab master gateway` (MTU 1500), FlowLab's
slave application bridged to a veth pair on the board, and a server on the board. Each line is
one run; 1 MiB down and 1 MiB up, every byte checked by SHA-256.

| Slave application settings | Mailbox | Down | Up |
|---|---|---|---|
| `--spi-tuning stack --whole-buffers` | 256 B | 122 kB/s | 227 kB/s |
| `--spi-tuning stack --whole-buffers` | 1024 B | 108 kB/s | 153 kB/s |
| `--spi-tuning split --whole-buffers` | 256 B | 222 kB/s | 256 kB/s |
| `--spi-tuning split --whole-buffers` | 1024 B | 215 kB/s | 205 kB/s |
| defaults (`fast`, message-only) | 256 B | 323 kB/s | 335 kB/s |
| defaults | 512 B | 362 kB/s | 372 kB/s |
| defaults | 1024 B | 407 kB/s | 388 kB/s |

Reading the table:

- With whole-buffer access, a larger mailbox is *slower*. Every TCP acknowledgement then costs
  a full 1 KiB write, while a 1514-byte frame needs only two messages instead of seven.
- With message-only access, the mailbox size helps as expected. At the same 256 B, the access
  pattern alone gives 2.6× (down) and 1.5× (up) over stack-style access.
- Repeated runs (1024 B, defaults) agreed within 1 %: 407 and 407 kB/s down, 390 and 388 kB/s
  up.

## FlowLab VoE

`flowlab master transfer --protocol voe`, 1 MB, SHA-256 checked, 1024-byte mailboxes: 340 kB/s
to the slave and 398 kB/s from it with the defaults. VoE acknowledges once per window instead of once per message
([FlowLab VoE](../../reference/voe.md)), so it is close to what the mailbox itself carries;
TCP over EoE reaches the same range with its own acknowledgements.

## What would go further

The remaining cost is the Linux SPI driver's time per transfer, not the wire: a 1 KiB message
spends about 0.4 ms on the wire at 20 MHz and about 2 ms in total. Polling the LAN9252's IRQ
pin instead of AL Event, a kernel-side driver, or ESC access from the i.MX8M Plus's Cortex-M7
core would each remove part of it ([Open work](../../workspace/open-work.md)).

## Limits of these figures

- One board, one carrier, one SPI clock, one master host. The master's USB adapter and macOS
  add their own round trip per mailbox frame.
- FlowLab's slave application serves mailboxes only (PRE-OP): no process data, distributed
  clocks or watchdogs, which a product's application also has to serve.
- TCP figures depend on the gateway's TCP stack (smoltcp) and its settings; they are the
  throughput of this rig, not a model of another master's.
