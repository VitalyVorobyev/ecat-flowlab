# Simulation

`flowlab simulate` runs a discrete-event model of an EtherCAT segment and the bulk transfers
through it. It writes every frame as a capture labelled **Simulated**, which is analysed like
any other, and it returns a result per transfer.

```bash
flowlab simulate configs/sim/example.json -o sim.pcapng --result sim.json
flowlab simulate configs/studies/bulk-transfer/{smoke,large-payload,mixed-load}.json
flowlab schema sim                       # JSON Schema of the spec
```

In the UI, **Simulate…** in the header opens the spec form. The run appears as a capture, with
the **Sim run** view next to the usual analysis.

## What is modelled

- **Master**: a cycle with a cyclic frame (LRW over the process image, BRD of AL status) and
  Gaussian start jitter, plus mailbox datagrams chosen by a policy:
  - *fixed quota*: *n* mailbox datagrams per cycle, shared round robin across slaves;
  - *round robin*: *n* slaves per cycle, each with a write and a read;
  - *opportunistic*: mailbox frames back to back while within a share of the cycle.

  No policy is assumed to be what a real master does; compare them.
- **Link**: full frame serialization (preamble, FCS, padding, gap) at the link rate. A
  datagram reaches slave *p* after the bytes up to its end are on the wire, plus the forwarding
  delays of the slaves before it.
- **ESC**: one-buffer mailboxes (a write is accepted only while "mailbox out" is empty; a read
  returns data only while "mailbox in" is full), three-buffer process data, and ESC RAM that
  must hold all SyncManagers.
- **PDI and slave application**: one PDI access at a time, at a read and a write rate plus a
  fixed cost per access, for mailbox messages and process data alike, either over the whole
  SyncManager buffer or only the message. The application takes a fixed time per message.
- **Transfers**: EoE (Ethernet frames carrying UDP/IPv4, fragmented), FoE (write or read),
  CoE segmented SDO (download or upload), and a custom process-data channel (sequence-numbered
  chunks, cumulative acknowledgements, go-back-N).
- **Faults**: frame loss with a given probability.

The receiver checks every byte against the generated payload and its SHA-256.

## What a result reports

Per transfer:

- time, first-byte time and goodput;
- the *exchange* span as a capture at the master would see it;
- protocol work: messages each way, rejected writes, empty reads, resent chunks;
- integrity;
- two references: the time the payload alone needs on the link (no transfer can be faster),
  and TCP over a dedicated Ethernet link at the same rate (a lower bound for plain Ethernet,
  not a prediction).

Per slave: PDI busy share and ESC RAM use. For the run: cycles, frames, link utilization, and
the model's assumptions.

```text
example  [SIMULATED]  seed 1  One bulk slave: EoE, FoE, CoE and PDO channel, 256 kB each, 1 ms cycle
  2741 cycles of 1.000 ms, 5493 frames, link 8.84 %, fixed quota: 2 mailbox datagrams per cycle
  #   protocol     direction             payload         time        goodput   link bound     TCP ref.  integrity
  0   EoE          master to slave      256000 B   694.366 ms    2.95 Mbit/s    20.480 ms    21.655 ms  SHA-256 ok
  1   FoE          master to slave      256000 B      1.025 s    2.00 Mbit/s    20.480 ms    21.655 ms  SHA-256 ok
  2   CoE          master to slave      256000 B      1.017 s    2.01 Mbit/s    20.480 ms    21.655 ms  SHA-256 ok
  3   PDO channel  master to slave      256000 B      1.032 s    1.98 Mbit/s    20.480 ms    21.655 ms  SHA-256 ok
```

## Reading simulated numbers

A simulation is a pure function of its spec: the same spec gives the same capture and result.
Its numbers follow from the parameters, above all the PDI rate and per-access cost, the
application time and the master policy. Until these are calibrated against a capture of the real
system, a simulation answers *what the outcome depends on*, not *how fast it will be*. The
[bulk-transfer report](../studies/bulk-transfer/report.md) shows how far apart plausible
parameter values are.

The model in detail: [flowlab-sim](../workspace/flowlab-sim.md). Spec fields:
[Input files](../reference/specs.md).
