# Synthetic captures

`flowlab synth` writes a capture of a master and a line of slaves whose every property is known:
cycle time, send jitter, lost frames, working-counter errors, skipped cycles, AL state changes
and mailbox traffic (SDO transfers and aborts, FoE writes, EoE frames, emergencies, rejected
writes). It exists to test the analysis, and to have realistic material when no capture is at
hand.

```bash
flowlab synth configs/synth/demo.json -o demo.pcapng --truth demo.truth.json [--eni demo.eni.xml]
flowlab synth -o demo.pcapng            # built-in demo spec
flowlab schema synth                    # JSON Schema of the spec
```

| Example spec | Content |
|---|---|
| `configs/synth/demo.json` | Six slaves at 1 ms; lost frames, WKC errors, skipped cycles; CoE, FoE and EoE traffic |
| `configs/synth/healthy-250us.json` | A clean 250 µs cycle |
| `configs/synth/jittery-host.json` | A master with large send jitter |

The capture is labelled **Synthetic** in its pcapng section comment. The same spec always gives
a byte-identical file, because the generator uses its own pseudo-random generator. `--truth`
writes what was put in: frame counts, faults, SDO outcomes, FoE and EoE transfers. The analysis
tests compare the report with this file rather than with hand-copied numbers.

## What it is not

The generator writes what a capture at the master would show under a simple round-trip
model: serialization plus a fixed delay per slave. It does not model slave behaviour or
bus timing. Its mailbox timing follows a fixed polling scheme. For behaviour that depends on
ESCs, PDIs and master scheduling, use the [simulator](simulation.md).
