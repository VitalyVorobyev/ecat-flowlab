# ECAT FlowLab — agent guide

EtherCAT analysis workbench: Rust core + local axum server + React UI on the `@vitavision/*`
lab-ui packages. Read `README.md`, then the book in `book/src/` (start with
`workspace/overview.md`, `workspace/contributing.md` and `workspace/decisions.md`).

## Commands

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
cd ui && bun run lint && bun run typecheck && bun run test:run && bun run build && bun run fmt:check
cargo run -p flowlab-cli -- synth configs/synth/demo.json -o results/demo.pcapng --truth results/demo.truth.json
cargo run -p flowlab-cli -- analyze results/demo.pcapng --out results/demo.json
cargo run --release -p flowlab-cli -- simulate configs/studies/bulk-transfer/{smoke,large-payload,mixed-load}.json
cargo run -p flowlab-cli -- plan                     # bus budget; `esi FILE` summarizes an ESI
cargo run --release -p flowlab-cli -- study configs/studies/bulk-transfer/foe-sweep.json
cargo run -p flowlab-cli -- serve results/demo.pcapng      # UI from ui/dist on :7878
python3 scripts/crosscheck-tshark.py results/demo.pcapng results/demo.json   # needs tshark
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace && mdbook build book
scripts/build-site.sh                        # Pages site in _site/: site/ + book at /book + rustdoc at /api
python3 scripts/bulk-transfer-report.py      # after a release build; regenerates the report's tables
cargo run -p flowlab-cli -- pdi-fit spi.csv --lab lab.json && cargo run -p flowlab-cli -- policy lab.pcapng --lab lab.json
python3 scripts/bulk-transfer-report.py --lab lab.json --out results/report-lab   # report with measured inputs
```

`cargo test` exports ts-rs types to `ui/src/api/gen/` (configured in `.cargo/config.toml`).
After changing a serialized Rust type, run `cargo test` and commit the regenerated files; if
the report schema changed, regenerate `ui/src/test/report.fixture.json` (see `ui/src/test/README.md`).

## Boundaries

- `ecat-wire`: wire format only. No I/O, no allocation on the parse path, never panics on
  input. Every new structure gets an encode path and a proptest round trip.
- `flowlab-capture`: files → `Trace` index. Owns pcap/pcapng (own reader/writer, no libpcap)
  and direction inference.
- `flowlab-analysis`: pure functions of a `Trace`. Every heuristic is written into
  `ReportMeta.assumptions`; every threshold behind a `Warn`/`Defect` finding is in
  `AnalysisOptions` and stated in the finding's text.
- `flowlab-synth`: deterministic (own PRNG). Same spec ⇒ byte-identical capture. Analysis
  tests assert against its `GroundTruth`, not against hand-copied numbers.
- `flowlab-server`: thin. No analysis logic in handlers; heavy work in `spawn_blocking`.
- `flowlab-devices`: ESI/ENI subset readers (roxmltree, DTDs rejected), ENI writer, planner
  (arithmetic only). No vendor ESI/ENI files in the repo — fixtures are hand-written.
- `flowlab-study`: runs simulations without captures; parameters are JSON pointers into the
  spec. Calibrate only on quantities both a capture and the simulator observe (message spans,
  not application end-to-end times); always report residuals.
- `flowlab-sim`: deterministic (own RNG, stable event order). Emits frames through `ecat-wire`
  into the same analysis pipeline; never analyses its own frames. Every modelling choice that
  changes results is a spec parameter or an entry in the result's assumptions. Keep its tests
  for byte conservation, the physical bound and monotonicity passing.
- Mailbox: messages only from returning datagrams with WKC > 0; channel addresses from SM
  configuration before hints before heuristics, and the source is always reported. New
  mailbox encodings in synth must stay identical to Wireshark's reading (CI `wireshark` job).

## Rules

- **Provenance.** Every result carries `measured | simulated | synthetic`; never combine them
  into one number or chart without labelling. No invented performance figures anywhere.
- **Units** in field names (`_ns`, `_bytes`, `_bps`); decimal MB ≠ binary MiB.
- **Times sent to the UI** are relative to capture start (below 2^53 ns) — the TS types use
  `number` for u64.
- **UI:** lab-ui components first (`@vitavision/ui`, `workbench`, `charts`, `forms`); tokens
  only, no raw palette classes or hex. Verdict colours (`normal`/`warn`/`defect`) only where
  the app compared a value with a declared threshold; magnitudes stay neutral. Table columns
  are module-level constants or factories (lint: `no-unstable-nested-components`). Components
  built here that lab-ui lacks (`HexView`, `StateTimeline`) are candidates to upstream.
- **Hardware safety.** Never run a second master on an active segment; live capture/master
  features need explicit opt-in, interface name and an isolated network. An ordinary Ethernet
  MAC is not an ESC — do not label non-ESC traffic "EtherCAT".
- **Unknowns** become falsifiable questions in `book/src/workspace/decisions.md` or the relevant study, not
  silent assumptions in code.
- Check licences before adopting a dependency (SOEM is GPLv3/commercial).

## Documentation

- `book/` (mdBook) is the user-facing documentation; `README.md` stays a short entry point.
  Both describe what the code does now: no milestones, plans or "will". Update the matching
  book page with every user-visible change.
- `site/` holds the start page and the findings page (reads `generated/findings.json`); same
  rules as the book.
- `book/src/studies/bulk-transfer/generated/` and `configs/studies/bulk-transfer/report/` are
  written by `scripts/bulk-transfer-report.py`; CI fails if they are stale. After a model change,
  regenerate and reread the report's prose, which quotes some of the numbers.

## Open work (not user-facing)

- Validate analysis, ENI reader and direction inference on real captures from several masters
  (TwinCAT, EtherCrab, SOEM/IgH) and a tap; answer the open questions in `workspace/decisions.md`.
- SDO Information and SoE transactions, AoE; logically mapped mailbox status.
- Simulator: TCP over EoE, VoE, priority/weighted mailbox policies, the study's application
  session protocol. Lab inputs (`pdi-fit`, `policy`) are untested on real hardware: run them on
  the Verdin/LAN9252 and a real master, then commit a calibrated report next to the placeholder one.
- Planner → simulator in one step; local optimisation beyond grid/LHS for large sweeps.
- Distributed-clock analysis (propagation delay, drift, sync error).
- Live capture and master adapters only behind a feature with explicit opt-in (hardware rules).
- Upstream `HexView` and `StateTimeline` to lab-ui; an item editor for arrays of objects in
  `@vitavision/forms`.

