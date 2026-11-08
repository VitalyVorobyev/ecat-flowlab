# Contributing

## Setup

```bash
cargo build                                   # Rust ≥ 1.88; Bun ≥ 1.4 for the UI
cargo ui-dev                                  # API on :7878 + Vite on :5173 with hot reload
```

`cargo ui-dev` runs `flowlab serve` (with `--demo`, also the demo capture) and Vite's dev server, which proxies
`/api` to it; stopping one stops both. `cargo ui` serves the production build instead. The
aliases live in `.cargo/config.toml` and run the `xtask` crate (`publish = false`).

## Before a pull request

```bash
cargo ci                                       # all of the below except the site
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace
cd ui && bun run fmt:check && bun run lint && bun run typecheck && bun run test:run && bun run build
mdbook build book                              # if you changed the book
scripts/build-site.sh                          # the whole Pages site in _site/
```

CI runs all of these, plus the Wireshark cross-check, the licence check (`cargo deny`) and the
report regeneration below. See [Testing and validation](testing.md). The Rust checks and tests
run on Linux, macOS and Windows; a further job builds the static Linux binaries (below) for
x86-64 and aarch64. `.gitattributes` keeps text files at LF line endings in every checkout, so
generated files and fixtures compare equal on Windows too.

## Releases and cross-compiling

`.github/workflows/release.yml` builds the release archives. It runs on a `v*` tag, on demand
(`gh workflow run release.yml --ref BRANCH`) and for pull requests that change the workflow:

1. One job builds the UI with Bun (`ui/dist`).
2. One job per target builds `flowlab` with that UI built in (feature `embed-ui`) and packs it
   with the README, the licences and `configs/`:

   | Target | Runner | Linking |
   |---|---|---|
   | `x86_64-unknown-linux-musl` | Ubuntu, `cargo zigbuild` | static (musl) |
   | `aarch64-unknown-linux-musl` | Ubuntu, `cargo zigbuild` | static (musl) |
   | `x86_64-pc-windows-msvc` | Windows | static C runtime (`+crt-static`) |
   | `aarch64-apple-darwin` | macOS | system libraries |

   Each matrix entry names its cargo `features`, space-separated; a feature that only some
   targets support is added to those entries alone. The job fails if a Linux binary is not
   statically linked. `flowlab-pdi` links spidev on Linux only, so `flowlab pdi-bench` reaches
   hardware in the Linux binaries and runs `--sim` only in the Windows and macOS ones.
3. Every run uploads the archives as workflow artifacts. On a tag, a last job attaches them,
   with `SHA256SUMS.txt`, to a GitHub release of that name.

To release, push a tag: `git tag v0.2.0 && git push origin v0.2.0`.

**For the Verdin by hand.** The Verdin modules run aarch64 Linux. A static musl binary runs on
any of their images, whatever glibc the Yocto or Torizon build has.
[cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild) uses Zig as the cross linker and
needs no Docker:

```bash
rustup target add aarch64-unknown-linux-musl
cargo install --locked cargo-zigbuild         # and Zig: https://ziglang.org/download/
(cd ui && bun install && bun run build)       # the UI to build in
cargo zigbuild --release -p flowlab-cli --features embed-ui --target aarch64-unknown-linux-musl
file target/aarch64-unknown-linux-musl/release/flowlab   # ELF 64-bit, ARM aarch64, statically linked
```

The same works from Linux, macOS or Windows, and for `x86_64-unknown-linux-musl`. Without
`embed-ui` the UI build is not needed, and `flowlab serve` serves only the API unless it finds
`ui/dist`.

**`embed-ui`.** The feature of `flowlab-cli` (and `flowlab-server`) builds `ui/dist` into the
binary with `rust-embed`; the build fails with a message if `ui/dist/index.html` is missing.
Default builds leave it off and read `ui/dist` from disk, so `cargo ui` always serves the
current UI build and `cargo ui-dev` the Vite dev server.

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
  analyses its own frames; every modelling choice that changes results is a spec parameter or an
  assumption in the result, and its tests for byte conservation, the physical bound and
  monotonicity keep passing. Studies calibrate only on quantities both a capture and the
  simulator observe (message spans, not application end-to-end times) and always report
  residuals. Device fixtures are hand-written; no vendor ESI or ENI files are committed.
- **Mailbox reading.** Messages come only from returning datagrams with WKC > 0. Channel
  addresses come from SyncManager configuration, then hints, then heuristics, and the source is
  always reported.
- **Generators stay honest.** Synth and sim output must keep matching Wireshark's reading (CI)
  and behave like real ESCs wherever the analysis depends on it.
- **UI.** lab-ui components first; design tokens only (no raw palette classes or hex); verdict
  colours only for comparisons with declared thresholds; table columns as module-level constants
  or factories.
- **Dependencies.** MIT/Apache-2.0-compatible only; check before adopting. SOEM, for example, is
  GPLv3 or commercial.
- **Hardware safety.** Anything that sends frames needs explicit opt-in, an interface name and an
  isolated network, and must never become a second master on a running segment. Anything that
  accesses an ESC over its PDI (`flowlab-pdi`) checks the link first, needs the ESC in INIT, never
  touches AL Control or SyncManager registers, and writes only with explicit opt-in into a named
  area of process RAM. An ordinary
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
