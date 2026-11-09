# Introduction

ECAT FlowLab is a workbench for EtherCAT traffic. It reads captures of a running segment and
explains them: the cycle and its jitter, round trips, working counters, bus load, the slaves'
state machines and their mailbox conversations (CoE, FoE, EoE). It also simulates segments that
do not exist yet, and searches their parameter space. Simulated and measured traffic go through
the same decoder and the same analysis, so they can be compared view for view.

It runs on a laptop: a command-line tool (`flowlab`) and a local web UI (`flowlab serve`). Reading
captures, simulating and planning need no EtherCAT hardware, no network adapter, no root and no
libpcap. To watch a running segment, it reads what Wireshark's dumpcap or tcpdump captures, which
only listen. Three opt-in tools reach hardware on a lab rig: a benchmark of a slave's SPI to its
ESC, a lab master that sends frames on an isolated segment, and an emulated slave that answers
them on an ordinary Ethernet port, for a rig without EtherCAT hardware
([Safety](getting-started.md#safety)).

## What it does

| Task | Command | UI view |
|---|---|---|
| Analyse a pcap/pcapng capture | `flowlab analyze` | Overview, Timing, Frames, Stations, Mailbox |
| Watch a running segment, analysed every second | `flowlab capture` | Live |
| Judge working counters against the master's configuration | `flowlab analyze --eni` | Attach ENI… on the Overview |
| Generate a capture with known faults, to test the analysis | `flowlab synth` | Generate |
| Simulate a segment and bulk transfers through it, and find what limits them | `flowlab simulate`, `flowlab what-if` | Simulate, Sim run |
| Lay out the cyclic frames of a network and check its budget | `flowlab plan`, `flowlab esi` | Planner |
| Sweep simulator parameters, find the Pareto frontier, calibrate | `flowlab study` | Study |
| Compare two captures, e.g. a measurement and its simulation | — | Compare |
| Read the master's mailbox service pattern from a capture, as a simulator policy | `flowlab policy` | Mailbox |
| Put measured values (PDI timing, policy) into a simulator spec | `flowlab apply-lab` | — |
| Time a slave's SPI accesses to its ESC (LAN9252) and fit the simulator's PDI model | `flowlab pdi-bench`, `flowlab pdi-fit` | — |
| Scan a lab network and measure transfers on it with FlowLab's own master (opt-in build) | `flowlab master scan\|run\|transfer` | — |
| Answer a master's frames as an emulated ESC on an ordinary Ethernet port (opt-in build) | `flowlab slave` | — |
| Print the JSON Schema of an input file | `flowlab schema` | the spec forms |

## Three kinds of data, never mixed

Every capture, report and result carries its **provenance**:

- **Measured**: a capture file recorded from a real network.
- **Simulated**: produced by the discrete-event simulator from a model of master, link, ESCs
  and slave applications.
- **Synthetic**: produced by the capture generator, with an exact ground truth, to test the
  analysis.

FlowLab writes the marker into every capture it generates, shows it on every view and never
combines two kinds into one number. Every inference the analysis makes is listed in the report's
assumptions; see [Provenance and assumptions](reference/provenance.md).

## How this book is organised

- **User guide**: installing; [investigating your own network](guide/own-network.md), from the
  first capture to a model fitted to it and a change checked on the wire; the UI; and each task in
  the table above.
- **Reference**: input and output file formats, the HTTP API, limitations, terms and sources.
- **Workspace**: the crates, how they fit together, how they are tested, and the design
  decisions behind them, for anyone who wants to extend FlowLab.
- **Studies**: investigations made with the tool. The first is
  [bulk transfer over EtherCAT](studies/bulk-transfer/index.md), with a
  [report](studies/bulk-transfer/report.md) on optimising a 10 MB transfer through an ESC on SPI.

The Rust API documentation is published next to this book under [`api/`](api/ecat_wire/index.html).
