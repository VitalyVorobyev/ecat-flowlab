# Lab master

`flowlab master` is FlowLab's own EtherCAT master, built on
[EtherCrab](https://github.com/ethercrab-rs/ethercrab) (MIT OR Apache-2.0). It measures what the
simulator predicts: a transfer is set up like a simulated one (protocol, payload, cycle time,
mailbox policy), and its result has the simulator's field names wherever a field means the same,
labelled **measured**.

```bash
flowlab master scan     -i enp3s0 --isolated-lab-network --eni results/master/lab.eni.xml
flowlab master run      -i enp3s0 --isolated-lab-network --cycle-us 1000 --duration 10
flowlab master transfer -i enp3s0 --isolated-lab-network --station 0x1000 --protocol foe --file firmware.bin
```

> **This sends EtherCAT frames.** Use it only on an isolated lab segment, on a network card
> dedicated to it, with no other master and no production equipment. It refuses to start when it
> hears another master ([Safety checks](#safety-checks)); never start it on a segment that is in
> use.

Its status on hardware is listed under [Limitations](../reference/limitations.md#lab-master): the
protocol engine is tested end to end against a simulated ESC, and the EtherCrab layer against
the [emulated slave](slave.md), in one process in CI and over a cable between two computers
([flowlab-master](../workspace/flowlab-master.md#tests)). Without EtherCAT hardware, the emulated
slave on a second computer is a complete rig; a slave that names itself `FlowLab emulated ESC`
in its EEPROM puts an assumption into every result saying that its timing is the host's.

## Building

The master is a cargo feature of the command line, off by default: a build without it has no
code that sends frames, and only the master needs the Npcap SDK on Windows.

```bash
cargo build --release -p flowlab-cli --features master
```

The Linux (x86-64 and aarch64, static musl) and macOS
[release binaries](../getting-started.md#install-a-release-binary) are built with it; the Windows
one is not. A build without the feature
answers `flowlab master` with these instructions.

| System | Setup |
|---|---|
| **Linux** (the lab PC) | EtherCrab opens a raw socket, which needs `CAP_NET_RAW`: run as root, or `sudo setcap cap_net_raw,cap_net_admin=eip target/release/flowlab` once per build. Take the card out of NetworkManager's hands (`nmcli device set enp3s0 managed no`) and bring the link up (`ip link set enp3s0 up`), so the host sends nothing of its own on the segment. |
| **Windows** | Install [Npcap](https://npcap.com/#download), download the Npcap SDK and add its `Lib\x64` folder to the `LIB` environment variable before building (the `pcap` crate links against `wpcap.lib`). Interfaces are named `\Device\NPF_{…}`; `flowlab capture --list` shows them. Windows networking is not deterministic; expect wider timing distributions than on Linux. |
| **macOS** | BPF: read and write access to `/dev/bpf*` (Wireshark's ChmodBPF, or `sudo chgrp admin /dev/bpf* && sudo chmod g+rw /dev/bpf*` until the next reboot). FlowLab passes EtherCrab's frames through its own BPF socket and hands EtherCrab only frames that passed an ESC (bit 1 of the source MAC set): EtherCrab's own macOS socket reads the adapter's MAC through `getifaddrs`, which for some USB adapters returns a wrong address, and then takes its own outgoing frame for the answer. A Mac is a good master for an [emulated-slave](slave.md) rig; for measurements that feed the simulator, the lab plan uses a Linux PC ([lab plan](../studies/bulk-transfer/lab.md#master)). |

The listening check below runs Wireshark's `dumpcap` or `tcpdump`, as [live capture](live.md)
does; install one of them and give it capture permissions.

## Safety checks

Every command, before it sends anything:

1. requires `--interface` (no default) and `--isolated-lab-network`, the user's statement that
   the interface is on an isolated lab segment;
2. listens on the interface for `--listen` seconds (default 2, including the capture tool's
   start-up) with the EtherCAT capture filter, and refuses to start if any EtherCAT frame
   arrives: another master, or something else sending EtherCAT, is active;
3. refuses if no capture tool is installed or the tool cannot capture, unless
   `--no-master-check` is given.

The check only hears a master that sends during those seconds; it is a guard, not a proof. FlowLab
never writes a slave's EEPROM and never changes alias addresses. EtherCrab assigns station
addresses from `0x1000` in ring order.

While a command runs, a capture on the same port shows exactly what the master sent:

```bash
flowlab capture enp3s0 -o results/master/foe.pcapng      # in a second terminal
```

Capturing your own master's traffic is fine; the check above only runs before the master starts.

## scan

Initialises the network (EtherCrab assigns addresses, reads each slave's EEPROM and configures
its mailbox SyncManagers) and reports, per slave in ring order: station and alias address, name,
vendor id, product code, revision and serial number, AL state and status code, the mailbox
protocols from the EEPROM, every enabled SyncManager read back from registers `0x0800 + 8·n`
(start, length, mailbox or buffered, direction), DC support and the propagation delay. The
slaves stay in PRE-OP.

```text
1 slave(s) on enp3s0  [MEASURED]
  #0 0x1000  LAN9252-SOES   vendor 0x00001337 product 0x00009252 rev 0x1  PREOP  mailbox [SM0 0x1000+128, SM1 0x1080+128] CoE/FoE/EoE  DC 64 bit
```

(The values above show the format; they are not a measurement.) `--eni FILE` also writes the
network as ENI XML: slaves, identities and mailbox SyncManagers, without cyclic commands. With it,
`flowlab analyze CAPTURE --eni FILE` names the slaves and takes the mailbox addresses from the
configuration (see [Network configuration](devices.md)).

## run

Moves the network PRE-OP → SAFE-OP → OP, exchanges process data every `--cycle-us` for
`--duration` seconds, then returns the slaves to PRE-OP. For every cycle it records when the
cycle started against the plan, the round trip of EtherCrab's process data exchange (one LRW
over the process image, plus EtherCrab's AL status reads) and its working counter. The summary
gives the round trip and start lateness percentiles, WKC mismatches against the expected value
(2 per slave with outputs plus 1 per slave with inputs), lost frames, overruns (a cycle's work
ending after the next cycle's planned start) and skipped cycles.

All times are the **host's**: `std::time::Instant` around each call, including the operating
system's scheduling and the socket. They are not the wire's. A capture with hardware timestamps,
or a tap, shows the wire; the round trip of the same frames in `flowlab analyze` of that capture
is the comparison.

## transfer

Moves a payload to (`--direction to-slave`, the default) or from (`from-slave`) the slave at
`--station`, with FoE, CoE, EoE or [FlowLab VoE](../reference/voe.md), at a cycle time and under
a mailbox policy. The mailbox comes
from the slave's SyncManager 0 and 1 registers.

```bash
# A file to the slave over FoE, two mailbox datagrams per 1 ms cycle
flowlab master transfer -i enp3s0 --isolated-lab-network --station 0x1000 \
  --protocol foe --file firmware.bin --foe-file firmware.bin --policy fixed-quota:2 --cycle-us 1000

# 1 MB of the simulator's payload as a segmented SDO download to 0x2000:00, opportunistic policy
flowlab master transfer -i enp3s0 --isolated-lab-network --station 0x1000 \
  --protocol coe --sdo 0x2000:0 --bytes 1000000 --policy opportunistic:60,10000

# Exactly what transfer 1 of a simulated scenario moved: same payload bytes, cycle time and policy
flowlab master transfer -i enp3s0 --isolated-lab-network --station 0x1000 \
  --like configs/sim/example.json --transfer 1 -o results/master/foe.json
```

| Option | Meaning |
|---|---|
| `--protocol foe\|coe\|eoe\|voe`, `--direction to-slave\|from-slave` | what moves where |
| `--file FILE` or `--bytes N [--seed S]` | the payload to send: a file, or N bytes of the simulator's generated payload |
| `--bytes N`, `--expect-sha256 HEX` or `--seed S`, `--save-received FILE` | from the slave: how much to wait for (required for EoE; FlowLab VoE announces it), what it should hash to, where to keep it |
| `--like SIM_JSON --transfer K` | everything from transfer K of a simulator spec (below) |
| `--cycle-us US` (1000) | cycle time |
| `--policy` (`fixed-quota:2`) | `fixed-quota:N`, `round-robin:N` or `opportunistic:PERCENT[,TURNAROUND_NS]`, the simulator's [policies](simulation.md#what-is-modelled) |
| `--poll read\|status` | read "mailbox in" directly (WKC 0 when empty, as the simulator does), or read SyncManager 1's status first and the buffer only when it is full |
| `--op` | move the network to OP and exchange process data at the start of every cycle (as the simulator does); otherwise the slaves stay in PRE-OP and cycles carry only mailbox traffic |
| `--foe-file NAME`, `--foe-password N` | FoE request (default `flowlab.bin`, 0) |
| `--sdo INDEX:SUB` | CoE object (default `0x2000:0`) |
| `--eoe-mtu`, `--eoe-slave-mac`, `--eoe-slave-ip`, `--eoe-master-mac`, `--eoe-master-ip`, `--eoe-port` | the UDP/IPv4 frames EoE carries |
| `--voe-window N` (16) | FlowLab VoE: DATA messages per acknowledgement the master asks for |
| `--reply-timeout-ms` (1000), `--max-resends` (3), `--max-duration` (600 s), `--first-counter` (1; 0 with `--op`) | retries and limits |
| `-o FILE` | result JSON (default `results/master/transfer-UNIXTIME.json`) |

### How a cycle runs

At each planned cycle start, the master first runs the cyclic exchange (with `--op`), then the
policy decides the mailbox frames:

| Policy | Mailbox frames per cycle (one slave) |
|---|---|
| `fixed-quota:N` | one frame: a write when a message is waiting (N ≥ 1), a read when the protocol expects an answer and the quota has room |
| `round-robin:N` | one frame with a write and a read (N ≥ 1): with one slave, the same as a quota of 2 |
| `opportunistic:P,T` | frames back to back: after each returns, another is sent T ns later if that is within P % of the cycle |

The master writes only when the protocol has a message, and reads only when it expects the slave
to answer, as in the simulator. EtherCrab sends every datagram as its own Ethernet frame; a write
and a read in the same policy frame are in flight together, in two frames (the simulator packs
them into one).

### Retries and counters

| Event | What the master does | Counted as |
|---|---|---|
| Write returns WKC 0 | "Mailbox out" is still full: the same message goes again in a later frame | `rejected_writes` |
| Read returns WKC 0 (or the status shows empty) | nothing to read yet | `empty_reads` |
| A write's frame is lost | sent again with the same counter, so the slave can drop it if it already took it | `lost_frames` |
| A read's frame is lost | the message may be gone: the master toggles the repeat request (SyncManager 1, activate bit 1), waits for the slave's acknowledge bit and reads again | `lost_frames`, `repeat_requests` |
| No answer within `--reply-timeout-ms`, or FoE BUSY | the last message goes again with a new counter, up to `--max-resends` times in a row | `resent_messages` |
| A message from the slave with the previous one's counter | dropped as a repeat | `duplicate_messages` |
| A message for another protocol (e.g. a CoE emergency during FoE) | ignored | `other_messages` |
| An FoE error, SDO abort or mailbox error reply | the transfer ends with its text in `error` | |

Mailbox counters run 1–7. Before a transfer, messages waiting in "mailbox in" are read and
discarded (`drained_messages`), except for an EoE receive, whose stream may already have started.
On the way to OP, EtherCrab reads the PDO mapping over CoE, so with `--op` the slave's last
counter is unknown. If it equalled the first message's counter, a slave that drops repeated
counters (SOES does) would ignore that message until the reply timeout resent it. With `--op`
the first message therefore carries 0, the counter's start value (ETG.1000.6), which a slave does
not compare with the previous one; without `--op`, EtherCrab has not used the mailbox and the
first counter is 1.

### Protocols

- **FoE**: write request (`--foe-file`, `--foe-password`), then DATA packets filling "mailbox
  out", each acknowledged; a short packet (empty if the file fills the last one) ends the file. A
  read expects packets that fill "mailbox in".
- **CoE**: SDO normal transfer: an initiate request with the size and the first data, then
  segments with alternating toggle bits, each answered. The slave needs a writable (or readable)
  object of that size at `--sdo`, for example a domain or octet-string object in its SOES
  object dictionary. An upload accepts an expedited answer too.
- **EoE**: Ethernet frames carrying UDP/IPv4 datagrams of up to `--eoe-mtu` bytes, fragmented to
  fit the mailbox, without acknowledgements. The defaults are the simulator's documentation
  addresses; set `--eoe-slave-mac` and `--eoe-slave-ip` to what the slave's EoE interface accepts.
  From the slave, the master reassembles frames and keeps the payload of UDP datagrams to
  `--eoe-port`; something on the slave must send them (for example a UDP sender bound to its EoE
  interface), and `--bytes` says when the transfer is complete.
- **FlowLab VoE**: FlowLab's own protocol in VoE messages ([reference](../reference/voe.md)).
  DATA messages fill the mailbox after an 8-byte header and are acknowledged once per
  `--voe-window` messages, so the master writes whenever "mailbox out" is free and reads while
  messages are unacknowledged. To the slave, the slave ends the transfer with DONE, carrying the
  length and SHA-256 of what it received: `verified` is then true or false for a write as well.
  From the slave, READY announces the length, and a gap in the DATA numbers (a lost read) makes
  the slave send again from the missing message. The slave must implement it: the
  [emulated slave](slave.md) does.

The PDO channel of the simulator is not a mailbox protocol; the lab master does not run it.

## Comparing with the simulator

`--like SPEC --transfer K` sets up the transfer as the simulator ran transfer K of that spec:
protocol and direction, the spec's cycle time and policy (unless `--cycle-us` or `--policy` say
otherwise) and the **same payload bytes**, generated as the simulator generates them, so `sha256`
equals the simulated result's. For a transfer from the slave, the slave must send that payload;
the expected SHA-256 is checked.

Before comparing, make the simulated slave match the real one: its mailbox sizes from `scan`,
its PDI and application time from the [lab inputs](../studies/bulk-transfer/lab.md). Then:

| Measured (`MeasuredTransfer`) | Simulated (`TransferResult`) | Compare |
|---|---|---|
| `exchange_ns` | `exchange_ns` | the same definition: first to last mailbox message, as a capture at the master sees it |
| `messages_to_slave`, `messages_from_slave`, `rejected_writes`, `empty_reads` | the same | the protocol work and how often the master met a full or empty mailbox |
| `duration_ns`, `goodput_bps` | `duration_ns`, `goodput_bps` | close, not equal: to the slave, the measured end is the return of the last acknowledgement, the simulated one when the slave got the last byte |
| `payload_bytes`, `sha256`, `physical_bound_ns` | the same | the same payload |
| `timing` (cycle start lateness, round trips) | — | the host's timing, which the simulator models only as `master_jitter_ns` |

In the [Simulate view](simulation.md#the-simulate-view), open the same spec, *Pin as baseline*,
and change the parameters that differ from the rig; the table then shows what each change does
to the transfer.

## Comparing with a capture

A capture taken during a transfer is read like any other: `flowlab analyze foe.pcapng --eni
lab.eni.xml` finds the FoE transfer, its messages, rejected writes and empty reads in the Mailbox
view, with the capture tool's timestamps. `flowlab policy foe.pcapng` reads the master's service
pattern back as a simulator policy, which checks that the policy FlowLab set is the one on the
wire. Compare the capture's FoE span with the result's `exchange_ns`: the difference is the
host's timing against the capture tool's.

## Results

`scan`, `run` and `transfer` write `ScanResult`, `RunResult` and `MeasuredTransfer` JSON with
`meta.source = "measured"` and their assumptions (see [Report and result files](../reference/reports.md#lab-master-results)).
The command prints a summary tagged `[MEASURED]`; a transfer that did not complete still writes
its result and exits with an error.
