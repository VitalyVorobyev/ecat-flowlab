# ECAT FlowLab — agent guide

EtherCAT analysis workbench: Rust core, local axum server, React UI on the `@vitavision/*`
lab-ui packages. The book in `book/src/` is the reference. Start with `workspace/overview.md`
(crates and conventions), `workspace/contributing.md` (commands, generated files, review rules)
and `workspace/decisions.md`. Open work is in `workspace/open-work.md`.

## Commands

```bash
cargo ci                  # everything CI checks except the site
cargo ui / cargo ui-dev   # built UI / Vite hot reload + API (--demo loads a demo capture)
cargo run -p flowlab-cli -- --help
```

## Must not forget

- After changing a serialized Rust type: `cargo test` regenerates `ui/src/api/gen/`; commit it.
  Regenerate the UI fixtures if needed (`ui/src/test/README.md`).
- After a simulator change: `cargo build --release -p flowlab-cli && python3 scripts/bulk-transfer-report.py`,
  then reread the report prose and the findings page; CI fails on stale tables.
- Provenance (`measured | simulated | synthetic`) on every result; never mix them unlabelled; no
  invented performance figures.
- Crate boundaries and the rules in `workspace/contributing.md` ("Rules that reviews check"),
  including the hardware-safety rule: nothing sends frames without explicit opt-in, an interface
  name and an isolated network.
- Docs: the user guide and reference describe what the code does now, in the present tense, with
  no plans or "will". Update the matching book page with every user-visible change. Future work
  goes into `workspace/open-work.md`; unknowns become questions in `workspace/decisions.md`.
