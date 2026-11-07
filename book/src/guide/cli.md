# Command line

```text
flowlab <COMMAND>

  analyze   Analyze a pcap/pcapng capture
  synth     Generate a synthetic pcapng with known ground truth from a JSON spec
  simulate  Run the discrete-event simulator
  study     Run a parameter study (sweep, Pareto frontier, calibration) over the simulator
  plan      Lay out the cyclic frames of a network and check its budget
  esi       Summarize the devices of an ESI file
  serve     Serve the web UI and API on localhost
  schema    Print a JSON Schema (synth, sim, plan, study)
```

`flowlab <command> --help` prints every option. The most used:

| Command | Options |
|---|---|
| `analyze CAPTURE` | `-o/--out FILE` JSON report · `--json` to stdout · `--eni XML` · `--mailbox STATION=OUT,IN` (repeatable) · `--eoe-out PCAPNG` · `--link-mbps N` (100) · `--min-stream-frames N` (20) |
| `synth [SPEC]` | `-o/--out FILE` (required) · `--truth FILE` · `--eni FILE` |
| `simulate [SPECS…]` | `-o/--out FILE` frames (one spec) · `--result FILE` JSON (an array for several specs) |
| `study [SPEC]` | `--capture FILE` to calibrate against · `--eni XML` for that capture · `--result FILE` |
| `plan [SPEC]` | `--json` |
| `esi FILE` | `--json` |
| `serve [CAPTURES…]` | `-p/--port` (7878) · `--host` (127.0.0.1) · `--ui-dir` (ui/dist) · `--open` · `--eni XML` · `--mailbox …` |
| `schema WHICH` | `synth`, `sim`, `plan` or `study` |

Commands that take a spec use a built-in example when it is omitted. Errors go to stderr with
a non-zero exit code. Specs with unknown fields are rejected, so a typo cannot silently fall
back to a default.

## Recipes

```bash
# Analyse with the master's configuration and export the EoE traffic for Wireshark
flowlab analyze run.pcapng --eni network.xml --eoe-out eoe.pcapng --out run.json

# Simulate, then analyse the simulated frames like a capture
flowlab simulate spec.json -o sim.pcapng --result sim.json
flowlab analyze sim.pcapng --out sim-report.json

# Fit the simulator to a lab capture and inspect the residuals
flowlab study fit.json --capture lab.pcapng --result fit-result.json
```
