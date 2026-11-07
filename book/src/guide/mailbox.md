# Mailbox traffic

Acyclic communication with a slave (parameters, files, tunnelled Ethernet) goes through its
mailbox: two SyncManagers in ESC memory, "mailbox out" (master → slave) and "mailbox in"
(slave → master), each holding one message. The master writes into the first and polls the
second. FlowLab finds these channels, extracts the messages and follows the protocols.

## Finding the channels

For each station, the mailbox addresses come from the first source available. The source is
reported per station:

1. **SyncManager configuration in the capture**: writes to the SM registers (0x0800 + 8·n) in
   mailbox mode. ECAT-write SMs are "mailbox out", ECAT-read SMs are "mailbox in".
   Position-addressed configuration is mapped to stations through the APWR 0x0010 assignment.
2. **The ENI** (`--eni`), for stations whose configuration the capture does not contain.
3. **A hint**: `--mailbox STATION=OUT,IN`, for example `--mailbox 0x1001=0x1000,0x1080`
   (repeatable; also on `flowlab serve`).
4. **A heuristic**: an accepted FPWR at ≥ 0x1000 whose data parses as a mailbox header.

A capture that starts after the master configured the slaves, without an ENI or hint, may only
have the heuristic, which cannot find the "mailbox in" address until the slave answers.

## Messages

Messages are taken from **returning** datagrams with a working counter > 0:

| Datagram | WKC > 0 | WKC = 0 |
|---|---|---|
| Write to mailbox out | accepted message | rejected: the mailbox was still full |
| Read of mailbox in | message from the slave | empty: nothing to read |
| Read of the SM status | status poll | — |

A message whose mailbox counter equals the previous one is a repeat and is dropped. Mailbox
datagrams are excluded from working-counter verdicts, and frames made only of them are not
cyclic streams.

## Protocols

- **CoE**: SDO transfers are followed per station as a state machine: initiate, then segments
  with alternating toggle bits, then the last segment, or an abort. Each transfer records index,
  sub-index, direction, size, outcome, abort code with its text, and duration. Emergencies are
  listed. SDO Information messages are counted.
- **FoE**: request, then DATA/ACK until a packet shorter than the mailbox. Each transfer records
  file name, size, packets, errors, busy replies and goodput.
- **EoE**: fragments per station and direction are reassembled into Ethernet frames, checking
  frame number and offset. `--eoe-out eoe.pcapng` (or the download in the Mailbox view) writes
  the reassembled frames as their own capture, for Wireshark.
- **SoE, VoE, AoE**: headers are decoded and counted.

Durations are measured at the capture point, between the frames that carried the first and
the last message of a transfer. This is the span a calibration compares with the simulator (see
[Studies](studies.md)).

## The Mailbox view

Per channel: addresses and their source, accepted and rejected writes, empty reads, polling
cadence. **How the master serves the mailbox** shows the service pattern per busy slave (below). Traffic per protocol, and tables of SDO transfers, emergencies, FoE transfers and EoE
frames. In **Frames**, the *Mailbox* filter selects a protocol, and the detail pane shows each
message's decoded fields.

## How the master serves the mailbox

Masters differ most in how often they touch a slave's mailbox, and the simulator has to be told
which behaviour to model. When directions are known and the capture has a cyclic stream, the
analysis measures, per slave whose mailbox buffers were read or written in at least 10 cycles of
stream 0:

| Field | Meaning |
|---|---|
| `cycles` | cycles with at least one read or write of the slave's mailbox buffers |
| `accesses_per_cycle` | those reads and writes per cycle (distribution) |
| `frames_per_cycle` | frames carrying them per cycle |
| `in_cyclic_frames` | accesses that rode in a frame of a cyclic stream |
| `turnaround` | time from a mailbox frame's return to the next mailbox frame's send in the same cycle |
| `last_send_percent` | when the cycle's last mailbox frame was sent, as a share of the period |
| `pattern` | the simulator policy that produces this pattern |

The pattern is read like this: a median above 1.5 mailbox frames per cycle means back-to-back
polling, `opportunistic` with the 95th percentile of `last_send_percent` as its budget and the
median turnaround; otherwise a `fixed_quota` of the 95th percentile of accesses per cycle.
Status-register polls are not counted. The quota is per slave: a master that shares one quota
among several slaves needs the sum. The rule is written into the report's assumptions.

`flowlab policy CAPTURE --lab lab.json` prints the pattern and stores the policy in a
lab-inputs file for the simulator (see [Input files](../reference/specs.md#lab-inputs)).

## Agreement with Wireshark

FlowLab's mailbox decoding is checked against Wireshark's EtherCAT dissector:

```bash
python3 scripts/crosscheck-tshark.py results/demo.pcapng results/demo.json
```

The script fails on any frame Wireshark marks malformed or warns about, and compares message and
transfer counts with the report. CI runs it on generated and simulated captures.
