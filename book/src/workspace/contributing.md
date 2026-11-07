# Contributing

## Setup

```bash
cargo build                                   # Rust ≥ 1.88
(cd ui && bun install)                        # Bun ≥ 1.4
```

For UI work, run `cargo run -p flowlab-cli -- serve results/demo.pcapng` (API on :7878) and
`bun run dev` in `ui/`. Vite proxies `/api` to the server and reloads on change.

## Before a pull request

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace
cd ui && bun run fmt:check && bun run lint && bun run typecheck && bun run test:run && bun run build
mdbook build book                              # if you changed the book
scripts/build-site.sh                          # the whole Pages site in _site/
```

CI runs all of these, plus the Wireshark cross-check, the licence check (`cargo deny`) and the
report regeneration below. See [Testing and validation](testing.md).

## Generated files

- **UI types.** `cargo test` exports the ts-rs types to `ui/src/api/gen/`. Commit them; CI fails
  if they differ from what the code generates.
- **UI fixtures.** If the report or result types change, regenerate the JSON fixtures in
  `ui/src/test/` as described in its README.
- **Study reports.** `python3 scripts/bulk-transfer-report.py` (after
  `cargo build --release -p flowlab-cli`) rewrites the specs in
  `configs/studies/bulk-transfer/report/` and the tables in
  `book/src/studies/bulk-transfer/generated/`, including `findings.json`, the data of the
  findings page. A change to the simulator that moves a number fails CI until the tables are
  regenerated. Then reread the report text and the findings page: both quote some of those
  numbers.

## Rules that reviews check

- **Provenance on everything.** Every result is `measured`, `simulated` or `synthetic`. Never
  combine kinds in one number or one unlabelled chart. No invented performance figures in code,
  docs or fixtures.
- **Assumptions are data.** Every heuristic goes into `meta.assumptions`; every threshold behind
  a finding is an option and appears in the finding's text.
- **Crate boundaries.** `ecat-wire` does no I/O, does not allocate while parsing, never panics on
  input, and gives every structure an encode path with a proptest round trip. Analysis is pure.
  The server holds no analysis logic and runs heavy work in `spawn_blocking`. The simulator never
  analyses its own frames.
- **Generators stay honest.** Synth and sim output must keep matching Wireshark's reading (CI)
  and behave like real ESCs wherever the analysis depends on it.
- **UI.** lab-ui components first; design tokens only (no raw palette classes or hex); verdict
  colours only for comparisons with declared thresholds; table columns as module-level constants
  or factories.
- **Dependencies.** MIT/Apache-2.0-compatible only; check before adopting. SOEM, for example, is
  GPLv3 or commercial.
- **Hardware safety.** Anything that sends frames needs explicit opt-in, an interface name and an
  isolated network, and must never become a second master on a running segment. An ordinary
  Ethernet MAC is not an ESC: do not label non-ESC traffic "EtherCAT".
- **Unknowns** become falsifiable questions in the [design decisions](decisions.md), not silent
  assumptions in code.

## Documentation

This book is the user-facing documentation; the README stays a short entry point. The published
site (<https://vitalyvorobyev.github.io/ecat-flowlab/>) wraps it: a start page and the
interactive findings page in `site/` (static HTML, Observable Plot from a CDN), the book under
`/book` and the rustdoc under `/api`. `scripts/build-site.sh` assembles it in `_site/`; the
Pages workflow runs the same script on every push to `main`. Write in the
present tense about what the code does, with units, and with British spelling as in the code.
Public Rust items need doc comments (`missing_docs`); a crate's top-level docs should say what it
is for and show a short example.
