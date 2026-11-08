# Limitations

What FlowLab does not do, or does only partly. Each item is a property of the current code.

## Captures and analysis

- **No own capture or master.** FlowLab never sends frames. Live capture runs Wireshark's
  dumpcap or tcpdump and reads what they write; their timestamps are the capturing host's. It has
  been tested with replays and stand-in tools, not yet on a real segment, and not on Windows.
- **Live figures** come from one second of frames each (cycles slower than about 50 ms show no
  per-second period) and from a rolling window ([Live capture](../guide/live.md#how-the-live-figures-are-made)).
- **Validated on generated traffic.** The analysis is tested against synthetic captures with
  known ground truth, against simulated captures, and against Wireshark's dissector. It has not
  yet been checked against captures from real masters. The [design decisions](../workspace/decisions.md)
  list the questions such captures should answer.
- **Capture probes.** Captures with probe-specific headers (for example ET2000-style hardware
  timestamps) are not decoded; frames must be plain Ethernet II or EtherCAT over UDP.
- **Timestamps** are taken as recorded; host software timestamps add jitter that FlowLab
  cannot separate from the master's.
- **Distributed clocks** are not analysed beyond decoding the datagrams (FRMW, system time
  registers).
- **Mailbox.** Status polls through a logically mapped SyncManager status bit are seen as
  process data. SDO Information and SoE are counted but not followed as transactions; AoE
  headers are decoded only.
- **Expected working counters** without an ENI are the most frequent value per datagram,
  which is wrong if a slave is missing for most of the capture.

## ESI and ENI

- Only the subset FlowLab uses is read; the ENI reader has been checked against FlowLab's own
  writer and hand-written fixtures, not against exports from configuration tools.

## Simulator

- **EoE carries UDP-like datagrams**; TCP over EoE (acknowledgements, windows, retransmission)
  is not modelled. VoE is not modelled.
- **Master policies are idealised** (fixed quota, round robin, opportunistic); none is a model
  of a particular product. Priority or weighted schedulers are not modelled.
- **No operating system.** Master and slave scheduling latency appear only as cycle jitter
  and a fixed application time per message.
- **Process image ≤ 1,400 bytes** in one cyclic frame; one PDO channel per slave.
- **Parameters are not calibrated** until a study fits them to a capture of the real system.

## PDI benchmark

- **LAN9252 over SPI only**, through Linux spidev (`flowlab pdi-bench`); other ESCs, the
  LAN9252's host bus interface, dual/quad SPI and kernel drivers are not covered. On macOS and
  Windows only `--sim` runs.
- **Untested on hardware.** The protocol follows the data sheet, Microchip's application notes
  and the SOES hardware layer, and is tested against FlowLab's own simulated LAN9252, not yet
  against a real one.
- **Its own access pattern.** It times one access at a fixed address per size; a stack that
  splits or merges accesses differently can time differently.

## Server

- Captures are held in memory; there is no persistence between runs.
- No authentication; the server must stay on loopback.
