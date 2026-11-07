# Analysing a capture

`flowlab analyze CAPTURE` decodes every frame, infers what it can, and writes a report: a
summary on the terminal, JSON with `--out` or `--json`. The UI shows the same report. This
page explains how each part is derived. The full list of a report's inferences is in its
`meta.assumptions`.

```bash
flowlab analyze run.pcapng --out run.json [--eni network.xml] [--mailbox 0x1001=0x1000,0x1080] \
                [--link-mbps 100] [--min-stream-frames 20] [--eoe-out eoe.pcapng]
```

## Inputs

FlowLab reads classic pcap (µs and ns resolution, both byte orders) and pcapng (several sections
and interfaces, any `if_tsresol`, `if_tsoffset`). It decodes EtherCAT in raw Ethernet frames
(EtherType 0x88A4, with or without one VLAN tag) and over UDP/IPv4 (port 34980). Frames
that are not EtherCAT are counted as *other* and otherwise ignored.

Timestamps are whatever the capture tool recorded. Software timestamps on a PC add their own
jitter. FlowLab reports what is in the file and does not correct for it.

## Directions and round trips

A capture at the master contains every frame twice: as sent (TX) and as returned by the ring
(RX). ESCs set the locally administered bit (0x02 of the first octet) of the source MAC
in frames they forward, so a source MAC that appears both with and without that bit separates
TX from RX. If not, frames whose working counters are all zero are taken as TX (masters send
WKC 0). If neither rule applies, directions stay unknown. Pairing, loss and working-counter
checks are then switched off, and the report says so.

A TX frame is paired with the next RX frame that has the same datagram layout and the same
first datagram index. The ADP field of position-addressed and broadcast datagrams is ignored in
this comparison, because every slave increments it. A TX frame that is sent again before an
answer arrives, or that has none by the end of the capture (outside a 50 ms in-flight window),
is *unanswered*. Round trip = RX time − TX time.

## Cyclic streams, period and jitter

TX frames with an identical datagram layout (command, address, length of every datagram) form
a group. A group with at least 20 frames (`--min-stream-frames`), of which at least 60 % of the
intervals lie within ±50 % of the median, is a **cyclic stream**.

- **Period**: each interval is rounded to a whole number of median periods to give a cycle
  number. The period is the least-squares slope of send time over cycle number, so gaps do
  not bias it.
- **Phase jitter**: each send time's residual from that line. This is what a slave
  synchronised to the master's cycle sees.
- **Interval**: the difference between consecutive send times, excluding gaps (> 1.5 periods),
  which are counted as **missed cycles** instead. For independent send errors of σ, phase σ ≈ σ
  and interval σ ≈ √2 σ.

A warning is raised when |phase jitter| p99 exceeds 10 % of the period.

## Working counters

Each datagram of a stream gets an expected working counter:

- with an ENI (`--eni`), the `Cnt` the master configured for that command, address and length;
- otherwise, and for datagrams the ENI does not list, the most frequent value on returning
  frames.

Every returning frame with a different value is a *working-counter mismatch*: a slave missed the
datagram, is in the wrong state or is absent. The basis of each expectation (`eni` or
`most_frequent`) is reported per datagram. Datagrams that access a mailbox are excluded:
a WKC of 0 there means "mailbox full" or "empty", which is normal.

## Utilization

Each frame occupies max(captured length, 60) + 8 (preamble) + 4 (FCS) + 12 (gap) bytes of link
time. Utilization is binned over time against the link rate (`--link-mbps`, default 100) and
reported per direction.

## Stations and state machines

Station addresses come from configured-address (FPxx) datagrams. Ring positions come from
APWR writes to register 0x0010. AL state requests are taken from writes to AL control
(0x0120), and reported states from reads of AL status (0x0130); only changes are recorded.

## Findings

Findings summarise what deserves attention, each with a severity (*info*, *warn*, *defect*) and
the threshold it was judged against: unanswered frames, working-counter mismatches, missed
cycles, jitter, SDO aborts, emergencies, mailbox error replies and rejected writes. Defaults are
fields of `AnalysisOptions`. See [Report and result files](../reference/reports.md).

## In the UI

**Overview** shows the summary and findings. **Timing** shows one stream at a time. **Frames**
lists every frame with server-side filters; selecting one shows its datagrams decoded (register
names, AL states, mailbox messages) beside the raw bytes. **Stations** shows each station's
accesses and its AL state as a timeline.
