# PDI slave

`flowlab pdi-slave` runs FlowLab's slave application on a real ESC: a LAN9252 on the slave's
host, reached over SPI (Linux spidev). The ESC answers the master's frames as always; FlowLab
takes the place of the slave's own application behind it and serves AL Control and the
mailbox through the PDI, with the same responder as the [emulated slave](slave.md) (FoE, CoE,
EoE, [FlowLab VoE](../reference/voe.md)).

With it, the mailbox sizes and the PDI access pattern are FlowLab's, not those of the slave's
own application: a transfer measured against it shows what the ESC, its SPI and the master can
carry, independent of the application that normally runs there. [On a Verdin
board](verdin.md) walks through a whole run; [LAN9252 on a Verdin
iMX8MP](../studies/bulk-transfer/lab-results.md) has the measured results.

```bash
# On the slave's host: stop the slave's own application first (here a container).
docker stop <container>
flowlab pdi-slave --device /dev/spidev1.0 --clock-hz 20000000 --replace-slave-application \
    --max-mailbox-bytes 1024 --upload-bytes 1000000 --seed 7

# On the master's host:
flowlab master transfer -i en7 --isolated-lab-network --station 0x1000 --protocol voe \
    --bytes 1000000 --seed 3
flowlab master transfer -i en7 --isolated-lab-network --station 0x1000 --protocol voe \
    --direction from-slave --bytes 1000000 --seed 7
```

> **This drives a real ESC.** It writes AL Status, AL Status Code, the PDI Control bytes of
> SyncManagers 0 and 1 and the mailbox buffers; it never writes AL Control or the master's part
> of the SyncManager registers. If the slave's own application still runs, both answer the
> master: stop it first. The command refuses to start without `--replace-slave-application`.
> On Ctrl-C (or SIGTERM) it sets AL Status back to INIT; start the slave's own application
> again afterwards.

## What it does

| What | Behaviour |
|---|---|
| Link | checks `BYTE_TEST`, `HW_CFG` and `ID_REV` (`--chip-id`, 0x9252) first, as `pdi-bench` does |
| INIT | always accepted; the mailboxes close and the application starts over |
| SyncManager PDI Control | bit 0 of SyncManagers 0 and 1 (0x0807, 0x080F) is cleared on PRE-OP and set again on INIT and at the end. A stack sets it outside PRE-OP, and a stopped stack leaves it set: while it is set, the ESC refuses the master's mailbox writes |
| PRE-OP | from INIT, when SyncManager 0 is a mailbox the master writes and SyncManager 1 one it reads: enabled, in process RAM, apart, 34 B to `--max-mailbox-bytes` (1024) each. The sizes are the master's: the EEPROM's, or others set with `flowlab master … --mailbox-bytes N`. Otherwise AL Status gets the error flag and code `0x0016` |
| SAFE-OP, OP | refused with `0x0011`: there is no process data |
| BOOT | refused with `0x0013` |
| Error acknowledge | with the error flag set, a request without the acknowledge bit is ignored, as on an ESC |
| Polling | one read of AL Event Request (0x0220) per poll: bit 0 for AL Control, bit 8 for "mailbox out" written, bit 9 for "mailbox in" read, when the master enabled the SyncManagers' AL Event (control bit 5). Without it, and every 64th poll as a fallback, the SyncManager status registers (0x0805, 0x080D) |
| "Mailbox out" | a full mailbox is read up to the message's end, then its last DWORD (reading the last byte frees it), and handed to the responder. `--whole-buffers` reads the whole buffer instead |
| "Mailbox in" | when the mailbox is empty and an answer waits, the message is written, then the buffer's last DWORD (writing the last byte hands it to the master); the counter runs 1–7. `--whole-buffers` writes the whole buffer instead |
| Upload | `--upload FILE` or `--upload-bytes N --seed S`: what FoE reads, SDO uploads and VoE from the slave send |

Every second with traffic it prints the polls per second and the messages and bytes per second
in each direction; payloads it received are printed with their size and SHA-256, as the
emulated slave prints them.

It polls without pause: one SPI-heavy loop on one CPU core. It does not serve the SyncManager
repeat request (the master's own resend, a new message with a new counter, still works),
process data, interrupts or the watchdogs.

## EoE to a network interface

With `--eoe-bridge PEER --eoe-host HOST`, EoE frames cross to a Linux network interface, so
TCP and anything else IP runs through FlowLab's application:

```bash
# On the slave's host, as root: a veth pair without offloads (the frames must carry
# finished checksums and be at most one MTU long).
ip link add fleoe0 type veth peer name fleoe0b
for i in fleoe0 fleoe0b; do ethtool -K $i tx off rx off gso off tso off gro off sg off; done
ip link set fleoe0b up; ip link set fleoe0 up
flowlab pdi-slave --device /dev/spidev1.0 --clock-hz 20000000 --replace-slave-application \
    --eoe-bridge fleoe0b --eoe-host fleoe0

# On the master's host: the EoE gateway sets the slave's address and forwards a TCP port.
flowlab master gateway -i en7 --isolated-lab-network --station 0x1000 --mailbox-bytes 1024 \
    --slave-ip 10.0.0.2 --master-ip 10.0.0.1 --set-ip --slave-mac 02:00:00:00:00:02 \
    --forward 18000:8000
```

Frames the master sends go out on `PEER` (raw socket) and arrive at `HOST`; frames `HOST` sends
(source MAC `HOST`'s) are read from `PEER` and sent to the master as EoE fragments. While eight
messages wait for "mailbox in", no more frames are taken from `PEER`. A Set IP Parameter request
is answered with success and applied to `HOST` (`ip link set … address`, `ip addr flush`, `ip
addr add`, `ip link set … up`; the subnet mask gives the prefix, /24 without one). Without the
bridge, EoE frames are counted and Set IP requests go to the responder, which does not answer
them. [TCP over EoE](tcp-bench.md) measures the result with `flowlab tcp-bench`. The bridge loop
is `flowlab_slave::bridge::EoeBridge` ([flowlab-slave](../workspace/flowlab-slave.md)).

## SPI options

`--clock-hz` (10 MHz), `--mode` (0), `--read-instruction read|fast-read`: as for
[`pdi-bench`](../studies/bulk-transfer/lab.md#measuring-the-spi). Registers go through the CSR
interface, the mailbox buffers through the process RAM FIFOs, in bursts of up to 16 DWORDs.

`--spi-tuning` (`fast`) sets how the SPI bus is used; the bytes on the wire per LAN9252
transaction stay the same:

| Value | Effect |
|---|---|
| `split` | a write's instruction and address go as their own transfer ahead of the data, so no transfer exceeds 64 bytes: the i.MX ECSPI driver moves longer transfers by DMA, which costs more than it saves at these sizes |
| `noabort` | no abort (and its poll) before a process RAM command when the previous one in that direction finished |
| `batch` | several LAN9252 transactions per spidev call (`cs_change` between them): a CSR command with its status and data reads, a FIFO burst with the status read after it |
| `fifo` | the write FIFO takes as many DWORDs as it has room for, instead of a whole burst |
| `fast` | `split,noabort` |
| `stack` | none: one spidev call per transaction, an abort before every command, as the slave stacks' hardware layers do |

A comma-separated list combines them (`split,batch`).
