# Testing and validation

FlowLab is validated in layers: unit and property tests on the wire format, analysis checked
against the ground truth of synthetic captures, invariants of the simulator, UI screens rendered
from real tool output, and an independent decoder — Wireshark's EtherCAT dissector — reading the
same captures. CI runs all of them on every push to `main` and every pull request.

## Commands

The full local check, the same as CI:

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
cd ui && bun run lint && bun run typecheck && bun run test:run && bun run build && bun run fmt:check
```

One crate, or one test:

```bash
cargo test -p flowlab-sim
cargo test -p flowlab-analysis recovers_mailbox_ground_truth
```

## Rust tests per crate

| Crate | Where | What |
|---|---|---|
| `ecat-wire` | `src/tests.rs` | Golden bytes (a BRD of AL status parsed, and rebuilt byte for byte by `FrameBuilder`, padded to 60 bytes); SyncManager, SDO command and mailbox protocol headers against golden bytes; UDP encapsulation; truncated input is an error, not a panic; property tests (below) |
| `flowlab-core` | `src/lib.rs`, `src/units.rs` | Provenance marker in a capture comment round trip; serialization time |
| `flowlab-capture` | `src/trace.rs`, `src/stream.rs` | pcapng write → read → index round trip; classic pcap with µs timestamps; WKC direction heuristic without a MAC split; garbage rejected; a stream fed in pieces of any size splits into the same blocks, and header blocks plus a run of packets parse as a file |
| `flowlab-analysis` | `src/tests.rs`, `src/stats.rs`, `src/mailbox.rs` | Against synth ground truth (below); percentiles, histogram steps, decimation keeping spikes; SDO toggle violations, superseded and orphan transfers, FoE errors and EoE gaps |
| `flowlab-synth` | `src/tests.rs` | Same spec ⇒ identical bytes and ground truth, another seed ⇒ different bytes; ground-truth counts equal what the capture contains; invalid specs rejected |
| `flowlab-devices` | `src/tests.rs` | Hand-written ESI/ENI fixtures, ENI writer round trip, malformed XML and DTDs rejected, planner against hand arithmetic |
| `flowlab-sim` | `src/tests.rs`, `src/protocol.rs` | Simulator invariants (below) |
| `flowlab-study` | `src/tests.rs` | Pareto consistency, constraints with reasons, sampling rules, calibration recovering hidden parameters |
| `flowlab-live` | `src/tests.rs`, `src/tool.rs` | Replays counted completely and saved byte for byte, the window's span, mailbox addresses kept beyond the window, `cat` as a stand-in capture tool, failing tools and non-capture output, interface lists |
| `flowlab-pdi` | `src/tests.rs` | Against a simulated LAN9252: SPI framing of READ and FAST READ; CSR reads at every alignment; process RAM round trips through the FIFOs (unaligned, not DWORD multiples, beyond the FIFO depth); refusals of a broken link, a wrong chip, an ESC outside INIT, writes without permission or outside process RAM, and the AL Control and SyncManager registers; a benchmark's CSV fitted back by `flowlab-study` to the simulated bus, labelled simulated |
| `flowlab-server` | `src/lib.rs` | The router driven in-process: synth, simulate, ENI, ESI, plan, study, live replay and the `--live` opt-in, uploads, errors |

Each crate page in this part of the book lists its tests in more detail.

### Property tests

`ecat-wire` uses `proptest` for every structure it encodes:

| Property | Checks |
|---|---|
| `encode_parse_round_trip` | 1–5 datagrams of any command, index, address, data (0–199 bytes), IRQ, WKC and circulating flag, built by `FrameBuilder` and parsed back field for field, with the `more` flag set on all but the last |
| `parser_never_panics`, `parser_never_panics_on_ethercat_type` | Random bytes, and random bytes carrying the EtherCAT EtherType |
| `sdo_command_decode_encode_is_canonical` | Every SDO command byte in both directions |
| `mailbox_headers_round_trip` | Mailbox header fields |
| `sm_slicing_never_panics` | SyncManager register slicing for any address and length near 0x0800 |

A new wire structure gets an encode path and a round-trip property.

### Analysis against synthetic ground truth

`flowlab-synth` writes a capture together with a `GroundTruth` (cycle time, jitter, lost frames,
WKC errors, skipped cycles, mailbox counts). `crates/flowlab-analysis/src/tests.rs` generates
captures in memory, analyses them and compares with that truth rather than with hand-copied
numbers: `recovers_cycle_timing` (period, phase jitter σ within ±15 %, interval σ ≈ √2 × phase σ,
missed cycles, round trip from frame size and slave delays), `recovers_faults` (unanswered frames
= lost frames, WKC mismatches = injected errors), `stations_and_states`,
`recovers_mailbox_ground_truth` (messages, rejected writes, empty polls, SDO/FoE/EoE),
`mailbox_without_configuration_uses_hints_or_heuristic`, the ENI tests, and
`report_is_deterministic`.

### Simulator invariants

`crates/flowlab-sim/src/tests.rs` keeps these passing:

| Invariant | Test |
|---|---|
| Determinism: same spec ⇒ identical capture bytes and result JSON | `example_completes_verified_and_deterministic` |
| Byte conservation: every transfer delivers exactly its payload, verified by SHA-256, under frame loss and at boundary sizes | `both_directions_survive_frame_loss`, `boundary_payloads` |
| Physical bound: `duration_ns ≥ physical_bound_ns`, goodput below the link rate | `example_completes_verified_and_deterministic` |
| Monotonicity: a slower PDI never speeds a transfer up | `slower_pdi_never_speeds_a_transfer_up` |
| ESC RAM gating | `esc_memory_gates_the_configuration` |
| Like-for-like: `flowlab-analysis` reads the simulated capture with the same message counts the simulator reports | `capture_is_analysed_like_a_measurement` |

`flowlab-study`'s calibration test closes the loop: a capture simulated with hidden PDI parameters
is analysed like a measurement, and the study recovers the parameters with residuals below 10⁻⁹.

## UI tests

`cd ui && bun run test:run` runs Vitest (happy-dom, Testing Library). The screen tests
(`Overview`, `Compare`, `Mailbox`, `Planner`, `Simulation`, `Study`) render real tool output from
`ui/src/test/*.fixture.json` and check provenance labels, headline numbers, findings,
threshold verdicts and assumptions; `format.test.ts` and `hexLayout.test.ts` cover the
formatting and hex-layout helpers. `bun run lint` (oxlint, type-aware, no warnings allowed),
`bun run typecheck` and `bun run fmt:check` are part of the same gate.

## Regenerating types and fixtures

`cargo test` exports the ts-rs types to `ui/src/api/gen/` (`TS_RS_EXPORT_DIR` in
`.cargo/config.toml`). After changing a serialized Rust type:

```bash
cargo test
git add ui/src/api/gen
```

If the report schema changed, regenerate `ui/src/test/report.fixture.json` (commands from
`ui/src/test/README.md`):

```bash
python3 -c "import json; s=json.load(open('configs/synth/demo.json')); s['cycles']=400; s['faults']={'lost_frame_probability':0.01,'wkc_error_probability':0.01}; json.dump(s,open('results/fixture-spec.json','w'))"
cargo run -p flowlab-cli -- synth results/fixture-spec.json -o results/fixture.pcapng
cargo run -p flowlab-cli -- analyze results/fixture.pcapng --out ui/src/test/report.fixture.json
```

and, for a `SimResult` change, `sim.fixture.json`:

```bash
python3 -c "import json; s=json.load(open('configs/sim/example.json')); [t.update(payload_bytes=8000) for t in s['transfers']]; s['capture']={'enabled':False,'max_frames':0}; json.dump(s,open('results/sim-fixture-spec.json','w'))"
cargo run -p flowlab-cli -- simulate results/sim-fixture-spec.json --result ui/src/test/sim.fixture.json
```

`plan.fixture.json` is `flowlab plan --json`; `study.fixture.json` is `flowlab study` output for
`foe-sweep.json` without its PDI-rate parameter.

## Wireshark cross-check

`scripts/crosscheck-tshark.py CAPTURE REPORT.json` compares FlowLab's mailbox analysis with
Wireshark's EtherCAT dissector, which decodes the capture independently:

1. No frame may be malformed or carry a Wireshark expert warning or error.
2. tshark decodes every mailbox datagram; the script counts, in returning datagrams with WKC > 0,
   messages per protocol and direction, CoE aborts and emergencies, completed EoE frames and FoE
   data bytes.
3. These counts must equal the report's. Any difference exits with status 1.

tshark runs with IP dissection disabled (it does not reassemble EoE fragments) and reads the
capture from stdin. The script assumes directions split by the source-MAC bit, as `flowlab-synth`
and `flowlab-sim` write them. Locally (needs `tshark`):

```bash
cargo run -p flowlab-cli -- synth configs/synth/demo.json -o results/demo.pcapng
cargo run -p flowlab-cli -- analyze results/demo.pcapng --out results/demo.json
python3 scripts/crosscheck-tshark.py results/demo.pcapng results/demo.json
```

New mailbox encodings in synth must keep this check passing.

## CI jobs

`.github/workflows/ci.yml`, with `RUSTFLAGS=-D warnings`:

| Job | Runs on | Steps |
|---|---|---|
| `rust` | ubuntu-latest, macos-latest | `cargo fmt --check`; `cargo clippy --all-targets -- -D warnings`; `cargo test`; `git diff --exit-code -- ui/src/api/gen` (generated types committed); synth → analyze round trip; synth with `--eni` → analyze with `--eni`, `esi` on the fixture, `plan`; `simulate` of `configs/sim/example.json` and the three bulk-transfer specs, `study` of `foe-sweep.json` (release) |
| `wireshark` | ubuntu-latest | Install tshark; synth → analyze → `crosscheck-tshark.py`; simulate the example with `-o` → analyze → `crosscheck-tshark.py` |
| `ui` | ubuntu-latest | `bun install --frozen-lockfile`; `fmt:check`, `lint`, `typecheck`, `test:run`, `build` |
