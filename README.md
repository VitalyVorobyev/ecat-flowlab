# ECAT FlowLab

An EtherCAT analysis workbench. Load a capture of EtherCAT traffic and see the cycle, jitter,
round trips, working counters, bus load, the slaves' state machines and their mailbox
conversations (CoE, FoE, EoE). It runs in a browser UI or as a JSON report. A discrete-event
simulator, a capture generator with ground truth, a bus-budget planner and parameter studies
build on the same decoder, so measured and simulated traffic compare like for like.

**Documentation:** <https://vitalyvorobyev.github.io/ecat-flowlab/>: the
[book](https://vitalyvorobyev.github.io/ecat-flowlab/book/) (source in [`book/`](book/src/SUMMARY.md)),
the [API reference](https://vitalyvorobyev.github.io/ecat-flowlab/api/) and the interactive
[bulk-transfer findings](https://vitalyvorobyev.github.io/ecat-flowlab/findings/).

## Quick start

Requires Rust ≥ 1.88 and [Bun](https://bun.sh) ≥ 1.4. It needs no EtherCAT hardware, root rights
or libpcap.

```bash
cargo ui                      # builds the UI if needed, opens http://127.0.0.1:7878 on Simulate
cargo ui my-capture.pcapng    # …with your captures loaded (--demo: a synthetic one)
cargo ui-dev                  # UI with hot reload (Vite on :5173) against the API
cargo ci                      # the checks CI runs
```

On the command line:

```bash
flowlab analyze capture.pcapng --out report.json [--eni network.xml]
flowlab simulate configs/sim/example.json -o sim.pcapng --result sim.json
flowlab what-if configs/studies/bulk-transfer/large-payload.json   # which change helps most
flowlab study configs/studies/bulk-transfer/foe-sweep.json
flowlab plan
```

(`cargo install --path crates/flowlab-cli` installs `flowlab`.)

## Provenance

Every result is labelled **Measured** (a capture file), **Simulated** (the model) or
**Synthetic** (the generator), and the kinds are never mixed into one number. Every inference
is listed in the report's assumptions.

## Safety

FlowLab reads files and never sends frames. `flowlab serve` has no authentication and can read
any file you can, so keep it on loopback.

## Repository

| Path | Content |
|---|---|
| `crates/` | `ecat-wire` (codec), `flowlab-core`, `-capture`, `-analysis`, `-synth`, `-sim`, `-devices`, `-study`, `-server`, `-cli` |
| `ui/` | React UI on the `@vitavision` lab-ui packages |
| `configs/` | Example generator, simulator and study specs |
| `book/` | The documentation (mdBook) |
| `scripts/` | Wireshark cross-check, study report generator |

Contributing: see [the book's chapter](book/src/workspace/contributing.md).

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your
option.
