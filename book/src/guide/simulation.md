# Simulation

`flowlab simulate` runs a discrete-event model of an EtherCAT segment and the bulk transfers
through it. It writes every frame as a capture labelled **Simulated**, which is analysed like
any other, and it returns a result per transfer.

```bash
flowlab simulate configs/sim/example.json -o sim.pcapng --result sim.json
flowlab simulate configs/studies/bulk-transfer/{smoke,large-payload,mixed-load}.json
flowlab schema sim                       # JSON Schema of the spec
```

## The Simulate view

**Simulate**, the first view of the UI, is where a scenario is set up and tuned. The left column
is the setup, in the units people use:

- **Master and link**: cycle time (µs), link rate (Mbit/s), cycle jitter, and the mailbox policy
  with its parameter.
- **Slaves**, in ring order: cyclic outputs and inputs, mailbox sizes, ESC RAM, forwarding
  delay, and the PDI (read and write rate in Mbit/s, overhead per access in µs, whole-buffer or
  message-only mailbox access) and the application's time per message.
- **Transfers**: protocol (EoE, FoE, CoE SDO, [FlowLab VoE](../reference/voe.md), PDO channel),
  direction, slave, payload (B, kB, MB, KiB or MiB), start time, and the protocol's own
  parameters (EoE MTU; VoE window; PDO channel bytes per cycle, window, resend timeout). TCP
  over EoE is listed but not modelled.
- **Run**: time limit, seed, frame loss.

Start from a preset (the bulk-transfer study's scenarios), or open and save the setup as a
`SimSpec` JSON file. The setup is kept in the browser between visits; *Undo* steps back through
the edits.

About a third of a second after each edit, the scenario is simulated again, without a capture.
The result shows when all transfers are done, the cycles, link load and the busiest PDI on one
line; a table of the transfers (time, goodput, multiple of the link bound, when each ran,
integrity); and the PDI load and ESC RAM of each slave that carries a transfer.

**What would help** reruns the scenario with one change at a time and ranks the changes by how
much they shorten the run, or the transfer selected in the table:

| Lever | Change tried |
|---|---|
| master | cycle time halved |
| policy | twice the mailbox datagrams or slaves per cycle; the opportunistic policy (60 % of the cycle, 10 µs turnaround); a larger share or shorter turnaround if already opportunistic |
| PDI | read and write rate doubled; overhead per access halved; message-only mailbox access |
| mailbox | mailboxes doubled, up to one frame (1484 B) |
| application | time per message halved |
| protocol | each other protocol for the transfer; EoE MTU 1500 B; PDO channel bytes per cycle or window doubled |

Only slaves and transfers that carry a transfer are changed. What tops the list limits the
scenario; a change with no effect (within 1 %) does not. When two limits coincide, for example
a PDI read that takes most of a cycle when the cycle also allows one message, no single change
helps: pairs of the first eight non-protocol changes are simulated too, and those that beat
both of their parts alone, and the scenario, by more than 1 % are shown as *two changes
together*. `flowlab what-if` prints the same ranking (`--no-effect-percent` sets the 1 %). Changes that make the spec invalid
(ESC RAM, frame size, one PDO channel per slave) or leave a transfer unfinished are listed with
the reason. *Apply* puts a change into the setup.

*Pin as baseline* keeps the current result: later results show their difference per transfer,
and a line lists the parameters changed since, with *Back to baseline*. *Open frames* runs the
scenario again with a capture and opens it in the analysis views; its **Sim run** view shows the
setup that was simulated, with *Edit in Simulate*.

From the command line:

```bash
flowlab what-if configs/studies/bulk-transfer/large-payload.json --result what-if.json
```

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
