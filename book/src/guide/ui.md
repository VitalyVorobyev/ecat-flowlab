# The workbench

`flowlab serve` starts a local server on <http://127.0.0.1:7878> that holds captures in memory,
analyses them, and serves the UI. Start it with captures to load, or empty:

```bash
flowlab serve [CAPTURES…] [--eni network.xml] [--mailbox STATION=OUT,IN] [--open]
```

## Layout

The header holds the capture selector, **Synthesize…** (generator form) and **Simulate…**
(simulator form). The status bar shows the selected capture's provenance, frame count and
span. The navigation rail on the left leads to the views:

| View | Shows |
|---|---|
| **Captures** | Loaded captures with provenance, size and duration; drop files here or open a local path |
| **Overview** | Headline numbers, findings, link load over time, datagrams by command; Attach ENI… |
| **Timing** | Per cyclic stream: period, phase jitter against the cycle grid, interval distribution, round trip, gaps and missed cycles, working counters per datagram |
| **Frames** | Every frame, filtered by direction, command, stream, station, mailbox protocol or problems; decoded datagrams next to a hex view |
| **Stations** | Station addresses, ring positions, reads and writes, no-response counts, AL state over time |
| **Mailbox** | Mailbox channels and their use, traffic per protocol, SDO transfers, emergencies, FoE transfers, EoE frames |
| **Planner** | Frame layout and bus budget of a network; ESI import |
| **Study** | Parameter sweeps over the simulator: Pareto scatter, all runs, calibration residuals |
| **Compare** | Two captures side by side, each column with its provenance |
| **Sim run** | The simulator's result for a simulated capture: transfers, protocol work, PDI and ESC RAM per slave |

Views that need an analysed capture are disabled until one is selected. **Sim run** is
available for simulated captures. Light and dark themes follow the system and can be switched
in the header.

## Reading the views

- **Colours** mark judgements only. *Defect* and *warning* colours appear where a value was
  compared with a declared threshold, such as a working counter that differs from the expected
  value or jitter above 10 % of the period. Magnitudes such as load or duration are neutral.
- **Assumptions** are listed at the bottom of the Overview, Mailbox, Sim run and Study views.
  They are the same list that the JSON report carries.
- **Provenance badges** (*Measured*, *Simulated*, *Synthetic*) sit next to every capture name.

## Generating captures from the UI

**Synthesize…** and **Simulate…** open a form generated from the JSON Schema of the
[generator](synth.md) or [simulator](simulation.md) spec, pre-filled with an example. The
resulting capture is added to the list and analysed like a file. **Study** has the same form for
study specs, plus *Simulate this run* for any Pareto-optimal or best run.

## Security

The server has no authentication and can read any file the user can (`Open path`). It binds to
loopback by default. Do not bind it to a public address with `--host`.
