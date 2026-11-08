# flowlab-server and flowlab-cli

`flowlab-server` is the local HTTP server behind the workbench: it keeps loaded captures in memory,
answers the UI's queries and serves the built UI. `flowlab-cli` builds the `flowlab` binary, which
exposes every pipeline stage — synthesis, analysis, ESI summary, planning, simulation, studies,
schemas — and starts the server. Both are thin: the work happens in the library crates.

API: [`flowlab_server`](../../api/flowlab_server/index.html) ·
source: [crates/flowlab-server](https://github.com/VitalyVorobyev/ecat-flowlab/blob/main/crates/flowlab-server),
[crates/flowlab-cli](https://github.com/VitalyVorobyev/ecat-flowlab/blob/main/crates/flowlab-cli)

## flowlab-server

### Position in the pipeline

Depends on `flowlab-core`, `-capture`, `-analysis`, `-synth`, `-sim`, `-devices`, `-study`, and
`axum`, `tokio`, `tower-http`. Used by `flowlab-cli` (`flowlab serve`).

| Item | What it is |
|---|---|
| `AppState` | The in-memory store; `AppState::new(AnalysisOptions)` sets options (mailbox hints, ENI) applied to every capture |
| `AppState::load_bytes`, `load_path`, `load_simulation` | Parse/index/analyse a capture (or run a simulation) and add it. Blocking |
| `AppState::set_network(id, Option<NetworkConfig>)` | Re-analyse one capture with or without an ENI |
| `CaptureInfo` | A loaded capture as listed: id, name, provenance, format, sizes, duration, streams, defect and warning counts, SHA-256, `has_simulation`, `eni` |
| `EsiImport` | An `EsiFile` plus one `PlanSlave` per device |
| `api(state)` / `app(state, ui_dir)` | The API router; the router plus static UI |
| `serve(state, addr, ui_dir)` | Bind and serve until Ctrl-C |
| `MAX_UPLOAD_BYTES`, `MAX_SIM_FRAMES` | Limits, below |

### Store

Each loaded capture is an entry holding its `CaptureInfo`, the `Trace` (shared `Arc`), the
`Analysis`, and the `SimResult` when it came from `POST /api/simulate`. Entries live behind an
`RwLock` until deleted or the process exits; nothing is written to disk. Ids count up from 1.
Attaching or removing an ENI re-analyses the same trace and replaces the entry under the same id.

### Endpoints

From the module documentation of `crates/flowlab-server/src/lib.rs`:

| Method | Path | Body / query | Response |
|---|---|---|---|
| GET | `/api/health` | | `{ version }` |
| GET | `/api/captures` | | `CaptureInfo[]` |
| POST | `/api/captures?name=` | raw file bytes | `CaptureInfo` |
| POST | `/api/captures/open` | `{ path }` | `CaptureInfo` |
| DELETE | `/api/captures/{id}` | | 204 |
| GET | `/api/captures/{id}/report` | | `CaptureReport` |
| GET | `/api/captures/{id}/frames` | `FrameFilter` + `offset`, `limit` | `FramePage` |
| GET | `/api/captures/{id}/frames/{no}` | | `FrameDetail` |
| GET | `/api/captures/{id}/eoe.pcapng` | | Ethernet frames reassembled from EoE (pcapng) |
| GET | `/api/captures/{id}/simulation` | | `SimResult` of a simulated capture |
| POST | `/api/captures/{id}/eni` | ENI XML | `CaptureInfo` (re-analysed with the configuration) |
| DELETE | `/api/captures/{id}/eni` | | `CaptureInfo` (re-analysed without) |
| POST | `/api/esi` | ESI XML | `EsiImport` |
| GET | `/api/plan/schema` | | JSON Schema of `PlanSpec` |
| GET | `/api/plan/example` | | `PlanSpec` |
| POST | `/api/plan` | `PlanSpec` | `PlanResult` |
| GET | `/api/study/schema` | | JSON Schema of `StudySpec` |
| GET | `/api/study/example` | | `StudySpec` |
| POST | `/api/study?capture=` | `StudySpec` | `StudyResult` (calibrated against capture `capture`, if given) |
| GET | `/api/synth/schema` | | JSON Schema of `SynthSpec` |
| GET | `/api/synth/example` | | `SynthSpec` |
| POST | `/api/synth` | `SynthSpec` | `CaptureInfo` |
| GET | `/api/sim/schema` | | JSON Schema of `SimSpec` |
| GET | `/api/sim/example` | | `SimSpec` |
| POST | `/api/simulate` | `SimSpec` | `CaptureInfo` (capture of the run; result via `/simulation`) |
| GET | `/api/sim/presets` | | `SimPreset[]` |
| POST | `/api/sim/run` | `SimSpec` | `SimResult` (no capture is kept) |
| POST | `/api/sim/what-if` | `SimSpec` | `WhatIfResult` |

Handler errors are `{ "error": "…" }` with status 400 (bad input, including a spec the simulator
or study rejects), 404 (unknown capture or frame) or 500. Requests axum rejects before a handler
runs (malformed query or JSON, wrong content type, unknown fields) get its plain-text 400, 415 or
422 responses; see the [HTTP API](../reference/api.md). The frame query's filter fields are `command`,
`direction`, `station`, `stream`, `problems_only`, `include_other` and `mailbox`; `limit` defaults
to 200 and is capped at 5000. Upload names are reduced to their file name. The full request and
response shapes are in [HTTP API](../reference/api.md).

### Threads and limits

Handlers do no analysis themselves. Parsing and analysing captures, frame queries, the EoE export,
ENI re-analysis, synthesis, simulation and studies run in `tokio::task::spawn_blocking`; a study
spreads its runs over its own scoped threads inside that task. Planning and ESI parsing, which are
arithmetic and a small XML read, run inline.

| Limit | Value |
|---|---|
| `MAX_UPLOAD_BYTES` (request body limit on every API route) | `2 << 30` = 2,147,483,648 bytes (2 GiB) |
| `MAX_SIM_FRAMES` (frames a UI-started simulation may write) | 500,000; `POST /api/simulate` also forces `capture.enabled` |
| Frames per page | default 200, at most 5000 |

### Security and static UI

The server has no authentication and `POST /api/captures/open` reads any file the user can, so it
belongs on loopback. `flowlab serve` binds 127.0.0.1 by default and logs a warning for any other
`--host`. `Ui::find(dir)` picks the UI: `dir` (`--ui-dir`, default `ui/dist`) when it holds an
`index.html`, else the UI built into the binary (feature `embed-ui`, through `rust-embed`), else
none. `app(state, Some(&ui))` serves it with `index.html` as the SPA fallback for unknown paths;
without a UI only the API is served.

## flowlab-cli

The binary is `flowlab` (`cargo run -p flowlab-cli -- <command>`). Logs go to stderr
(`RUST_LOG`, default `info`); summaries go to stdout. Every output path's parent directory is
created. Spec arguments that are omitted fall back to the built-in example.

| Command | Arguments and flags |
|---|---|
| `synth [SPEC] --out <OUT>` | `-o, --out` output capture (required); `--truth <TRUTH>` ground truth JSON; `--eni <ENI>` the network configuration a master would use, as ENI XML. Without `SPEC`, the built-in demo spec |
| `analyze <CAPTURE>` | `-o, --out <OUT>` JSON report; `--json` print the report instead of the summary; `--link-mbps <LINK_MBPS>` link rate for utilization, Mbit/s (default 100); `--min-stream-frames <MIN_STREAM_FRAMES>` minimum repetitions for a cyclic stream (default 20); `--mailbox <STATION=OUT,IN>` mailbox addresses for a station without SyncManager configuration in the capture, e.g. `0x1001=0x1000,0x1080`, repeatable; `--eoe-out <PCAPNG>` Ethernet frames reassembled from EoE; `--eni <XML>` network configuration |
| `esi <FILE>` | `--json` print JSON instead of the summary |
| `plan [SPEC]` | `--json` print JSON instead of the summary |
| `simulate [SPECS]...` | `-o, --out <OUT>` simulated frames (one spec only); `--result <RESULT>` result JSON (one spec) or a JSON array (several). Several specs print one table each, for comparison |
| `study [SPEC]` | `--capture <CAPTURE>` capture to calibrate against (targets with `from_capture`); `--eni <XML>` network configuration for analysing that capture; `--result <RESULT>` result JSON |
| `serve [CAPTURES]...` | `-p, --port <PORT>` (default 7878); `--host <HOST>` (default 127.0.0.1); `--ui-dir <UI_DIR>` built UI (default `ui/dist`); `--open` open the browser; `--mailbox <STATION=OUT,IN>` repeatable; `--eni <XML>` applied to every capture; `--live` allow live capture from interfaces; `--live-dir <DIR>` where live captures are saved (default `results/live`) |
| `capture [INTERFACE]` | live capture through dumpcap or tcpdump ([flowlab-live](flowlab-live.md)): `--list`; `--tool`; `-o, --out`; `--duration`; `--window`; `--all-traffic`; `--replay <FILE>` with `--speed`; `--report <JSON>` the last window's analysis; `--eni`, `--mailbox`, `--link-mbps` |
| `schema <WHICH>` | One of `synth`, `sim`, `plan`, `study`; prints the JSON Schema |
| `pdi-bench` | timed ESC accesses over a LAN9252's SPI ([flowlab-pdi](flowlab-pdi.md), `src/pdi_bench.rs`): `--device <PATH>` spidev device (Linux) or `--sim` a simulated LAN9252 (samples marked simulated); `-o, --out <CSV>` samples (required); `--clock-hz`; `--mode`; `--read-instruction read\|fast-read`; `--sizes`; `--reps`; `--warmup`; `--area <START+LEN>`; `--allow-writes`; `--no-al-event`; `--chip-id`; `--seed`; `--fit`, `--result <JSON>`, `--lab <JSON>` as `pdi-fit` (lab inputs from hardware runs only); `--sim-transaction-ns` |
| `master <scan\|run\|transfer>` | the lab master ([flowlab-master](flowlab-master.md)), compiled only with the feature `master` (`flowlab-master/hardware`); without it the command prints how to build it. Every subcommand: `-i, --interface` (required); `--isolated-lab-network` (required to send); `--listen <SECONDS>` (2), `--tool`, `--no-master-check` for the check for another master; `--pdu-timeout-ms` (30); `-o, --out` (default `results/master/<command>-UNIXTIME.json`). `scan --eni <XML>`; `run --cycle-us --duration --records`; `transfer --station` plus protocol, payload, policy and retry options ([Lab master](../guide/master.md#transfer)) |

`analyze` prints the capture's provenance in brackets (`[MEASURED]`, `[SYNTHETIC]`, `[SIMULATED]`),
`simulate` and `study` print `[SIMULATED]`, `master` prints `[MEASURED]`, and `synth` reports its output as synthetic. `study` prints
up to 15 Pareto-optimal runs sorted by the first objective (the best marked `*`), up to three
infeasible runs with their reasons, and the residuals of a calibration.

```bash
cargo run -p flowlab-cli -- synth configs/synth/demo.json -o results/demo.pcapng --truth results/demo.truth.json
cargo run -p flowlab-cli -- analyze results/demo.pcapng --out results/demo.json
cargo run --release -p flowlab-cli -- simulate configs/studies/bulk-transfer/{smoke,large-payload,mixed-load}.json
cargo run --release -p flowlab-cli -- study configs/studies/bulk-transfer/foe-sweep.json
cargo run -p flowlab-cli -- serve results/demo.pcapng      # UI from ui/dist on :7878
```

A walk-through of each command is in [Command line](../guide/cli.md).

## Tests

`crates/flowlab-server/src/lib.rs` drives the router in-process with `tower::ServiceExt::oneshot`:
synthesize, list, report, filtered frame pages (an empty numeric query parameter is a 400),
frame detail, EoE export, delete (`synth_then_query`); simulate and read the result, and a 400
for a spec over its ESC RAM (`simulate_then_read_result`); ENI attach/detach with `eni` WKC basis,
ESI import and plan (`eni_attach_esi_import_and_plan`); a study, a calibration without and with a
loaded capture (`study_runs_and_calibrates_against_a_loaded_capture`); garbage uploads and path
stripping (`upload_rejects_garbage_and_accepts_pcapng`); clean errors for a missing path. The CLI
is exercised by the CI steps listed in [Testing and validation](testing.md). `src/master.rs`
tests the policy, object, MAC and IP parsers and that nothing is sent without
`--isolated-lab-network`.
