# Command line

```text
flowlab <COMMAND>

  analyze   Analyze a pcap/pcapng capture
  capture   Capture EtherCAT traffic live and analyse it as it arrives
  synth     Generate a synthetic pcapng with known ground truth from a JSON spec
  simulate  Run the discrete-event simulator
  what-if   Rank one-change variants of a scenario by how much they shorten its transfers
  study     Run a parameter study (sweep, Pareto frontier, calibration) over the simulator
  plan      Lay out the cyclic frames of a network and check its budget
  esi       Summarize the devices of an ESI file
  serve     Serve the web UI and API on localhost
  schema    Print a JSON Schema (synth, sim, plan, study, lab)
  pdi-fit   Fit the simulator's PDI model (cost per access + effective rate) to timed ESC accesses
  policy    Read the master's mailbox service pattern from a capture, as a simulator policy
```

`flowlab <command> --help` prints every option. The most used:

| Command | Options |
|---|---|
| `analyze CAPTURE` | `-o/--out FILE` JSON report · `--json` to stdout · `--eni XML` · `--mailbox STATION=OUT,IN` (repeatable) · `--eoe-out PCAPNG` · `--link-mbps N` (100) · `--min-stream-frames N` (20) |
| `capture [INTERFACE]` | `--list` · `--tool dumpcap\|tcpdump` · `-o/--out FILE` · `--duration S` · `--window S` (10) · `--all-traffic` · `--replay FILE` `--speed X` · `--report JSON` · `--eni XML` · `--mailbox …` · `--link-mbps N` ([Live capture](live.md)) |
| `synth [SPEC]` | `-o/--out FILE` (required) · `--truth FILE` · `--eni FILE` |
| `simulate [SPECS…]` | `-o/--out FILE` frames (one spec) · `--result FILE` JSON (an array for several specs) |
| `what-if [SPEC]` | `--result FILE` JSON: the scenario, every single change and pair, best first |
| `study [SPEC]` | `--capture FILE` to calibrate against · `--eni XML` and `--mailbox …` for that capture · `--result FILE` |
| `plan [SPEC]` | `--json` |
| `esi FILE` | `--json` |
| `serve [CAPTURES…]` | `-p/--port` (7878) · `--host` (127.0.0.1) · `--ui-dir` (ui/dist) · `--open` · `--eni XML` · `--mailbox …` · `--live` · `--live-dir DIR` (results/live) |
| `schema WHICH` | `synth`, `sim`, `plan`, `study` or `lab` |
| `pdi-fit SAMPLES.csv` | `--result FILE` the fit with residuals · `--lab JSON` store the values in a lab-inputs file |
| `policy CAPTURE` | `--station N` · `--mailbox …` · `--eni XML` · `--lab JSON` store the policy in a lab-inputs file |

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

# What limits a scenario, and which change would help most
flowlab what-if spec.json

# Fit the simulator to a lab capture and inspect the residuals
flowlab study fit.json --capture lab.pcapng --result fit-result.json

# Turn lab measurements into simulator inputs, then re-run the bulk-transfer report with them
flowlab pdi-fit spi-timing.csv --lab lab.json
flowlab policy master-foe.pcapng --lab lab.json
python3 scripts/bulk-transfer-report.py --lab lab.json --out results/report-lab
```
