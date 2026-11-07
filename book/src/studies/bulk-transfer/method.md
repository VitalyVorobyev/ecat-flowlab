# Method

## Requirements

- Opaque binary payloads; 10,000,000 B by default; also 0 B, 1 B, around MTU, mailbox and
  fragment boundaries, and 10 MiB.
- Both directions, measured separately, since mailboxes, PDI and service rates can be
  asymmetric.
- One-shot, periodic, sustained and burst sources, alone and next to cyclic control traffic.
- A transfer counts only when every byte has arrived and its SHA-256 matches. Time runs from
  the sender's start to verified completion.
- Reproducible: simulations record seed and spec; lab runs record hardware and firmware
  revisions, master library and version, interface and timestamp source.

**Open product questions.** These decide the outcome more than any tuning:

- Is the payload moved once (configuration, firmware), periodically, or continuously? What is
  the deadline?
- Who sends: the PC, the Verdin, or both?
- Must the solution stay compatible with an existing TwinCAT EoE endpoint, or may the bus
  design change? Is a separate Gigabit Ethernet link acceptable?
- How many other slaves share the segment, with what cycle time and process data?
- What happens on loss: resume, retransmit, abort?

## Transport candidates

A transport must be supported by **both** master and slave. Replacing the master cannot add a
protocol the slave lacks.

| Transport | Path | Strength | Main risks |
|---|---|---|---|
| EoE | Ethernet frames fragmented into mailbox messages | IP applications work unchanged | fragment overhead, mailbox scheduling, TCP over the tunnel |
| FoE | mailbox file protocol, DATA/ACK | file semantics, widely implemented | stop-and-wait, availability by state and device |
| VoE | vendor-defined mailbox messages | minimal encapsulation | master API support, own reliability needed |
| CoE segmented / block SDO | object dictionary access | standard, interoperable | request/response per segment |
| PDO channel | reserved process data, own sequence/ACK protocol | fixed rate, independent of mailbox scheduling | ESC RAM, process image size, PDI load every cycle; both ends must be custom |
| Plain Ethernet TCP | separate link | throughput baseline | not through the EtherCAT segment |

An application-level session (start with length and digest, chunks with offsets, cumulative
acknowledgements, end with verification) makes transports comparable. FoE's own semantics are
used where FoE is native.

## Fairness rules

- Same payload bytes and the same integrity check on every transport.
- Background process-data traffic stays the same across candidates; it is never removed to make
  a protocol look faster.
- Control messages and retries count toward link utilization but not toward goodput.
- Directions, payload definitions and master settings are never averaged together.
- Plain-Ethernet TCP between PC and Verdin is the baseline in every comparison.

## Metrics

Transfer time and verified goodput (decimal MB and MiB stated), first-byte latency, link
utilization, frames and mailbox messages, rejected writes and empty polls, retransmissions,
PDI load and ESC RAM use. In the lab, also master and Verdin CPU load and the impact on
the cyclic deadline.

## The search problem

Find configurations that minimise transfer time, or maximise sustained goodput, subject to:

- the control cycle meets its deadline;
- the ESC RAM holds every SyncManager;
- mailbox sizes are ones the device supports;
- the protocol is genuinely supported by both ends.

The variables are:

- protocol and cycle time;
- mailbox sizes and the master's mailbox policy and budget;
- PDO-channel bytes per cycle and window;
- PDI rate and per-access cost, which are measured rather than tuned.

The result is a Pareto frontier over time, link load and PDI load, with infeasible candidates
listed with their reasons.

Before trusting the frontier, calibrate the model against lab captures. The PDI and the master's
scheduling are the parameters the outcome hinges on, and they are the least known. If the
model's advantages disappear after calibration, the study says so and stops optimising a model
that is wrong.

Statistics in the lab: warm-up, at least five trials per configuration, report median and range;
never claim precision beyond the timestamp source.
