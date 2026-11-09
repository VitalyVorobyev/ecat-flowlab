# Emulated slave

`flowlab slave` turns an ordinary Ethernet port into an EtherCAT slave: it answers a master's
frames as an ESC with FoE, CoE and EoE behind its mailbox would. With it, the
[lab master](master.md), [live capture](live.md) and the analysis run on a real cable between two
computers when no EtherCAT hardware is at hand.

```bash
# On the slave's host (Linux or macOS), the port cabled to the master:
flowlab slave -i eth0 --isolated-lab-network --upload-bytes 20000 --seed 7

# On the master's host:
flowlab master scan     -i en7 --isolated-lab-network
flowlab master run      -i en7 --isolated-lab-network --cycle-us 1000 --duration 5
flowlab master transfer -i en7 --isolated-lab-network --station 0x1000 --protocol foe --bytes 20000 --seed 3
flowlab master transfer -i en7 --isolated-lab-network --station 0x1000 --protocol foe --direction from-slave --bytes 20000 --seed 7
```

> **This sends frames.** It answers every EtherCAT frame on the interface. Use it only on a cable
> or switch that connects it to the master under test, with no production equipment and no
> other EtherCAT slaves: on a real segment it would answer a real master's frames a second time.

## What it tells you, and what not

The frames on the cable are real, and so are the master's state machine, its socket, the capture
and the analysis that read them. The slave is software: an ESC forwards a frame while it
arrives, the emulated slave answers once the host's network stack has delivered the frame to it
and taken the answer back. Every round trip and transfer time measured against it describes the
rig, not an EtherCAT slave.

Its EEPROM names it `FlowLab emulated ESC`. The lab master recognises the name, prints it in
`scan`, and adds to the assumptions of every result that involves it that the slave is the
emulator and its timing is the host's. Provenance stays `measured`: the frames were measured;
the assumption says what was on the other end. Such results are not lab inputs for the
simulator.

## The slave

| What | Behaviour |
|---|---|
| Identity | vendor ID 0 (none assigned), product code 1, revision 1, `--serial` (0); name `FlowLab emulated ESC` |
| Mailbox | SyncManager 0 ("mailbox out") at `0x1000` and 1 ("mailbox in") right after it, `--mailbox-bytes` each (128); protocols FoE, CoE, EoE |
| Process data | SyncManager 2: `--output-bytes` (4), SyncManager 3: `--input-bytes` (4); the inputs echo the outputs of the previous exchange; mapped in 32-bit entries (`0x1600`/`0x7000`, `0x1A00`/`0x6000`), readable over CoE (`0x1C12`, `0x1C13`) and in the EEPROM |
| FoE | write: stores the file (its size and SHA-256 are printed); read: sends the upload |
| CoE | SDO download into the transfer object `--sdo` (`0x2000:0`); upload of the upload from it; the mapping objects and `0x1018` (identity) are read-only |
| EoE | reassembles frames; with `--eoe-send`, sends the upload as UDP/IPv4 frames of `--eoe-mtu` (1500) bytes, from `--eoe-start-after-ms` (1000) after it entered PRE-OP |
| Faults | `--busy-every N`: every n-th FoE DATA packet is answered BUSY once; a message with the previous message's counter (not 0) is dropped, as SOES drops it |

The upload is `--upload FILE`, or `--upload-bytes N` of the simulator's payload for `--seed S`:
the bytes `flowlab master transfer --direction from-slave --bytes N --seed S` expects, so the
master verifies them by SHA-256.

An EoE send starts after a delay because a master configures the slave over CoE right after
PRE-OP (EtherCrab reads the PDO mapping on the way to SAFE-OP): EoE fragments waiting in
"mailbox in" then would be the answer to its requests. The send starts over whenever the slave
goes back to INIT, which every `flowlab master` command does first. A transfer's `duration_ns`
therefore includes the wait; `exchange_ns` starts with the first message.

The slave prints its AL state changes and, once a second while it answers frames, what arrived:

```text
emulated ESC on eth0: mailbox 128 B at 0x1000/0x1080, outputs 4 B, inputs 4 B; FoE, CoE, EoE. …
AL INIT, station 0x1000
AL PREOP, station 0x1000
FoE file `flowlab.bin`: 20000 B, SHA-256 0606fc6f…
```

## The ESC model

- **Commands**: position (`APxx`, `ARMW`), configured address (`FPxx`, `FRMW`) and broadcast
  reads, writes and read-writes, and logical `LRD`/`LWR`/`LRW` through three FMMUs, with the
  ESC's working counter rules (read 1, write 1, read-write 3; LRW 1 for reading, 2 for writing).
  Position and broadcast datagrams leave with their ADP incremented; the answer has bit 1 of the
  source MAC set, as an ESC sets it, and frames that already have it are not answered.
- **Registers**: station address, DL status (link and communication on port 0, ports 1–3
  closed), AL control, status and status code, the SII interface over the EEPROM image (8-byte
  reads; writes only with the write-enable bit), FMMUs and SyncManagers. Read-only registers
  ignore writes (the write still counts).
- **AL states**: INIT → PRE-OP once SyncManagers 0 and 1 are configured as the EEPROM says
  (otherwise AL status code `0x0016`), PRE-OP → SAFE-OP once 2 and 3 are (`0x001D`, `0x001E`),
  SAFE-OP → OP; invalid transitions `0x0011`, BOOT `0x0013`. An error stays until the master
  acknowledges it. Going back to INIT empties the mailboxes and restarts the application.
- **Mailbox SyncManagers**: a write to a full "mailbox out" or a read of an empty "mailbox in"
  gets no working counter; a message is the slave's when its last byte is written, and "mailbox
  in" is free again when its last byte is read; the status register shows "mailbox full"; the
  repeat request puts the last message back and toggles the acknowledge bit.

Not modelled: distributed clocks (the slave reports no DC support), the three-buffer mode of the
process data SyncManagers (plain memory here), watchdogs, interrupts and events, more than one
port, and an ESC's forwarding delay.

## Building and running

The emulated slave is a cargo feature of the command line, off by default:

```bash
cargo build --release -p flowlab-cli --features slave            # or master,slave
```

The Linux and macOS [release binaries](../getting-started.md#install-a-release-binary) are built
with it. For an aarch64 Linux board such as a Verdin, the static binary from
[Contributing](../workspace/contributing.md#releases-and-cross-compiling) runs on any image.

| System | Setup |
|---|---|
| **Linux** | A raw socket, which needs `CAP_NET_RAW`: run as root, or `sudo setcap cap_net_raw,cap_net_admin=eip flowlab` (the port goes into promiscuous mode). The port can be a bridge member or carry IP traffic (SSH, for example): the slave answers only EtherCAT frames. |
| **macOS** | BPF: read and write access to `/dev/bpf*` (Wireshark's ChmodBPF, or `sudo chgrp admin /dev/bpf* && sudo chmod g+rw /dev/bpf*` until the next reboot). |

`--duration SECONDS` stops it; otherwise Ctrl-C does, and the slave prints how many frames,
datagrams and messages it handled.

## A rig with two computers

A Mac with a USB Ethernet adapter cabled to a Verdin iMX95 (Torizon OS):

1. On the Verdin, `ethernet0` carries the cable (here as the only member of bridge `br0`, which
   holds the IP address used for SSH). Copy the static aarch64 binary and start
   `./flowlab slave -i ethernet0 --isolated-lab-network …`.
2. On the Mac, the adapter is `en10`; give your user BPF access (above), then run
   `flowlab master` on `en10`. Its check for another master hears nothing, because the slave
   only answers.
3. `flowlab capture en10` in a second terminal records the conversation; `flowlab analyze`
   reads it like any capture. The slave sets the source MAC bit, so directions are inferred as
   on a real segment.

SSH to the Verdin over the same cable keeps working: the master and the slave ignore IP traffic,
and the capture filter takes only EtherCAT frames.
