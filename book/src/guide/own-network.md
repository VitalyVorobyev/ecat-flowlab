# Investigating your own network

This page is a path through FlowLab for a network that exists: a master, a line of slaves and a
question about them. Is the cycle healthy? Why is a transfer slow? What would a change do? It
names the decisions to take first, then the tool for each step, with links to the pages that
describe each one in full.

```text
 decide ─▶ capture ─▶ read the analysis ─▶ model the segment ─▶ fit the model ─▶ change one thing ─▶ capture again
 (below)   any master   findings, mailbox     SimSpec + lab        to the capture     in the model,      and compare
                        service pattern       inputs               (residuals)        then on the wire
```

Every result on the way keeps its provenance. A capture of your network is **measured**. The
simulator's runs, what-if rankings and fits are **simulated**, even when their inputs were
measured. FlowLab never mixes the two in one number ([Provenance](../reference/provenance.md)).

## Decide first

| Question | If yes | If no |
|---|---|---|
| Is the segment in use (production, or a machine someone depends on)? | Observe only: `capture`, `analyze`, `policy`, the Live view. These only listen. | You may also run measurements on it (below), on an isolated segment. |
| Can you capture where both directions are seen: the master's port, a tap, or a mirror port between master and first slave? | Directions, round trips, lost frames and mailbox messages are all available. | Behind a switch, or with one direction only, directions stay unknown, and pairing, loss and WKC checks are off. Move the capture point first. |
| Does the capture tool timestamp in hardware (a tap, or a NIC with hardware timestamps)? | Jitter and round trips are the wire's. | Timestamps are the capturing host's. On the master PC they add its scheduling jitter to the master's ([Live capture § Where to capture](live.md#where-to-capture)). |
| Can you start the capture before the master starts? | The capture holds the SyncManager configuration and the state changes, so mailbox addresses and AL states are known. | Give the mailbox addresses (`--mailbox`) or the ENI. |
| Do you have the master's ENI (e.g. a TwinCAT export)? | Working counters are judged against the configuration, stations get names, mailboxes are known ([Network configuration](devices.md)). | Expected working counters are the most frequent value, which is wrong if a slave was missing for most of the capture. |
| Can you stop the master and isolate a rig with the same slaves? | `flowlab master` can measure transfers set up like simulated ones ([below](#measure-on-an-isolated-rig)). | Work from captures of the real master. |
| Do you develop the slave (its ESC, PDI and stack)? | Measure its PDI and application time ([below](#your-own-slave-device)). | Those values come from a fit to captures, or stay placeholders. |

Then name the question. *Cyclic health* (period, jitter, lost frames, working counters) needs a
capture and the analysis. *Acyclic throughput* (a file over FoE, parameters over CoE, Ethernet
over EoE) needs a capture of a transfer, and usually the model too.

## Capture

Record at the master's port or at a tap, keeping only EtherCAT, for as long as the effect you are
looking for needs to show:

```bash
flowlab capture eth1 --duration 60 -o run.pcapng       # through dumpcap or tcpdump; or the Live view
tcpdump -i eth1 -w run.pcapng ether proto 0x88a4        # any tool that writes pcap or pcapng works
```

`flowlab capture` prints one line per second while it records, and its file is an ordinary
capture ([Live capture](live.md)). For a transfer question, record while the master moves a file
of a known size, so the transfer stands out in the Mailbox view.

## Read the analysis

```bash
flowlab analyze run.pcapng --eni network.xml --out run.json    # or: flowlab serve run.pcapng --open
```

Start with the findings on the **Overview**. Each states the threshold it was judged against
([Analysing a capture](analysis.md)).

| Finding | What it means | What to check |
|---|---|---|
| Frame directions unknown | The capture does not show which frames were sent and which came back | the capture point: master port or tap, not behind a switch |
| Directions inferred from working counters | No source-MAC split was found; frames with all-zero working counters are taken as sent | the capture point; a frame no slave answered is then misread |
| *n* frames did not return | Sent frames without a returning one: lost on the ring, or not captured | cabling and connectors; the **Stations** view (datagrams without response, per station); whether the capture tool dropped packets |
| *n* working-counter mismatches | A slave did not process a datagram as the master expected: it is absent, in the wrong state, or failed | the basis (`eni` or `most_frequent`) per datagram on **Timing**; the AL state timeline on **Stations** at those times |
| *n* missed cycles | The master did not send in some cycles | the master host's real-time setup and load |
| Send jitter p99 above 10 % of the period | The master's send times scatter around its cycle grid | capture at a tap as well: if the jitter shrinks there, part of it was the capturing host's |
| Station reports an AL error | The slave set the error indication bit in AL status | when, and in which state, on the **Stations** timeline; the *AL status code* register (0x0134) in a **Frames** detail, if the master reads it |
| SDO transfers aborted, CoE emergencies | The slave refused an object access, or reported an error | the abort code and its text, the emergency codes, on **Mailbox** |
| Mailbox writes rejected | The slave's "mailbox out" was still full when the master wrote again | the slave reads its mailbox slowly: its application and PDI ([below](#your-own-slave-device)) |
| FoE transfers failed, EoE frames not reassembled, mailbox error replies | A file or tunnelled frame did not get through | the error text on **Mailbox**; the slave's protocol support |
| No cyclic stream detected | No frame layout repeats regularly | a longer capture, or `--min-stream-frames` for a slow cycle |

The **Mailbox** view adds the throughput side: per channel, accepted and rejected writes and empty
reads, and every SDO, FoE and EoE transfer with its duration and goodput, measured between the
frames that carried its first and last message ([Mailbox traffic](mailbox.md)).

## How the master serves the mailbox

Masters differ most in how often they touch a slave's mailbox, and that often decides throughput.
FlowLab reads the pattern from a capture with mailbox traffic and expresses it as one of the
simulator's policies:

```bash
flowlab policy run.pcapng --eni network.xml --lab lab.json
```

It prints accesses and frames per cycle, the turnaround and the policy. `--lab` stores the policy
in a lab-inputs file for the model, from measured captures only
([How the master serves the mailbox](mailbox.md#how-the-master-serves-the-mailbox)).

## Model the segment

The simulator answers *what limits this* and *what would a change do*. It needs a `SimSpec` that
describes your segment. Start from the Simulate view (open a preset, edit, save the setup as JSON)
or from `configs/sim/example.json`, and fill it in from what the steps above found:

| `SimSpec` field | Where the value comes from |
|---|---|
| `slaves[]` in ring order, `name` | **Stations** (ring positions), the ENI, or `flowlab master scan` |
| `outputs_bytes`, `inputs_bytes` | the ENI or the slaves' ESI (`flowlab esi`, the Planner's *Import ESI…*) |
| `mailbox_out_bytes`, `mailbox_in_bytes` | the channels on **Mailbox** (SyncManager configuration), or `master scan` |
| `dpram_bytes` | the ESC's data sheet |
| `cycle_time_us`, `master_jitter_ns` | the stream's period and phase jitter on **Timing** |
| `mailbox_policy` | `flowlab policy` |
| `pdi`, `app_processing_ns` | measured ([below](#your-own-slave-device)), fitted ([below](#fit-the-model-to-a-capture)), or placeholders until then |
| `transfers[]` | what you want to move: protocol, direction, slave, size |

`flowlab apply-lab` puts a lab-inputs file into the spec: the PDI timing and application time go
to the slave you name, the policy to the master. The spec's description records the sources:

```bash
flowlab apply-lab my-segment.json --lab lab.json --slave 2 -o my-segment.lab.json
flowlab simulate my-segment.lab.json                     # or open it in the Simulate view
flowlab what-if my-segment.lab.json                      # which single change helps most
```

**What would help** in the Simulate view, or `flowlab what-if`, reruns the scenario with one change
at a time and ranks the changes ([Simulation](simulation.md#the-simulate-view)). The change at the
top limits the scenario; a change with no effect does not. For the cyclic frames alone, the
Planner (`flowlab plan`) checks the bus budget and ESC RAM of a planned line
([Network configuration and planning](devices.md#planning-a-bus-budget)).

A model with placeholder values says *what the outcome depends on*, not *how fast it will be*.

## Fit the model to a capture

A calibration runs the model over a grid of the unknown parameters and keeps the runs that
reproduce the capture's measured message spans. `configs/calibration/foe-span.json` is an
example: it fits slave 2's PDI rate, per-access overhead and application time to the first FoE
write and the first SDO transfer in a capture.

```bash
flowlab study configs/calibration/foe-span.json --capture run.pcapng --eni network.xml --result fit.json
```

Edit its `base` to match your segment first (the table above), and set its transfers to the sizes
the capture shows. A capture span is compared with the simulator's `exchange_time_s`, the same
span seen from the master, never with an application's end-to-end time. Read the residuals: a
fit within a few percent shows that the model *can* reproduce the capture with those values. It
does not show that they are the physical ones, because several sets may fit as well
([Studies, calibration and comparison](studies.md#calibration)).

## Your own slave device

The simulator charges every PDI access a fixed cost plus its bytes at an effective rate, and the
application a fixed time per message. On a slave you develop, measure them:

- **PDI timing.** For a LAN9252 on SPI under Linux, `flowlab pdi-bench` times complete ESC reads
  and writes and `flowlab pdi-fit --lab lab.json` stores the fitted rate and overhead
  ([Measuring the SPI](../studies/bulk-transfer/lab.md#measuring-the-spi)). For another ESC or
  driver, wrap your stack's ESC read and write functions with a monotonic clock and log one line
  per call in the [PDI timing samples](../reference/specs.md#pdi-timing-samples) format; `pdi-fit`
  reads it the same way.
- **Application time.** Measure the time from the mailbox interrupt to the reply being written,
  and set `app_processing_ns` in `lab.json`.
- **Mailbox access.** Whether the stack moves the whole SyncManager buffer or only the message is a
  property of the stack: set `pdi.mailbox_access` to `whole_buffer` or `message_only`.
- **ESC RAM.** The Planner checks that mailboxes and three buffers of process data fit the ESC's
  RAM.

Then `apply-lab` and `what-if` show whether the PDI, the application or the master limits a
transfer, and how much a faster SPI clock or a message-only stack would change.

## Measure on an isolated rig

With the slaves on a segment of their own, FlowLab's lab master measures transfers set up like
simulated ones: the same payload bytes, cycle time and mailbox policy
([Lab master](master.md)). It sends frames, so it exists only in a build with the feature
`master`, and it refuses without `--isolated-lab-network` or when it hears another master.

```bash
flowlab master scan -i enp3s0 --isolated-lab-network --eni rig.eni.xml
flowlab capture enp3s0 -o transfer.pcapng                       # in a second terminal
flowlab master transfer -i enp3s0 --isolated-lab-network --station 0x1000 --like my-segment.lab.json --transfer 0
```

The result, `MeasuredTransfer`, uses the simulator's field names wherever they mean the same; the
capture shows the same transfer with the capture tool's timestamps
([Comparing with the simulator](master.md#comparing-with-the-simulator)).

## Change one thing and check it

1. Pick the change from the what-if ranking, and try it in the model first.
2. Make it on the network, or on the rig, and capture again under the same conditions: the same
   capture point, tool and duration.
3. Put the captures before and after side by side in **Compare**: both are measured. To set the
   prediction against the measurement, compare the simulated capture of the changed model
   (*Open frames* in Simulate) with the new measurement in a second comparison. Each column keeps
   its badge; ratios are taken between columns, never merged into one number
   ([Compare](studies.md#compare)).
4. If the measured change differs from the predicted one, the model is missing something: fit it
   again with the new capture as a second target.
