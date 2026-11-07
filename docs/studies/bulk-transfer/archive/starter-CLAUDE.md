# Claude implementation handoff — ECAT FlowLab

You are the primary coding agent for a new Rust project. Implement the attached plan, **not a generic EtherCAT driver from scratch**.

## Immediate assignment

Start from an empty repository with the handoff files. Write an implementation plan grounded in `docs/first-sprint.md`, then create an idiomatic Rust workspace supporting deterministic offline simulation on Apple Silicon macOS. The first runnable target should simulate a 10 MB binary transfer using configurable mailbox service events, output a machine-readable JSON report, and verify full-payload integrity. Later add EoE/VoE/FoE/PDO models and real backends only after tests and capability checks.

## Priorities and prohibitions

- Code must compile and run without hardware, root permissions, virtual machines, TwinCAT, network adapters, or any proprietary SDK.
- Preserve clean interfaces between payload generation, transfer-session protocol, transport model, link/ESC model, scheduler, optimizer, benchmark, and platform backend.
- No invented throughput measurements; simulated results must contain `source: "simulation"` and explicit assumptions/limits.
- Do not assume the Toradex device can function as a real EtherCAT slave using only its built-in Ethernet MAC. Require actual ESC evaluation/selection before enabling real slave mode.
- Never assume one mailbox action per EtherCAT cycle as a protocol guarantee. It is one selectable simplified scheduling scenario among several.
- Do not assume EtherCrab exposes EoE/FoE/VoE; verify version and APIs before adopting dependencies. Use feature-gated adapters.
- Do not assume VoE/FoE available or efficiently supported for every slave. Obtain and validate ESI/firmware capabilities.
- Do not label a custom raw Ethernet protocol "EtherCAT". Real EtherCAT requires proper master/ESC semantics.
- Do not run a second master against an active TwinCAT bus. Lab tests must be isolated.
- Prefer safe Rust; minimize unsafe code; clearly isolate platform FFI.
- Avoid copying GPL code into permissively licensed modules without reviewing compatibility.
- Use stable configuration schema with explicit units and consistent decimal MB vs binary MiB definitions.

## Implementation guidance

Use std-only deterministic discrete-event simulation at first; no need for Tokio unless justified. `serde`/`serde_json`, `clap`, `tracing`, `thiserror`, and property-based testing are reasonable starting points. Avoid complex GUIs and network drivers during milestone M0. Design interfaces for future experiment runners and optimizers, but don't build empty abstraction hierarchies for unimplemented features.

When confronting unknown details: write a falsifiable question in `docs/risks-and-decisions.md`, isolate the assumption behind a feature or configuration parameter, and implement tests proving the local behavior. Prefer small demonstrable milestones over broad placeholders.

## Definition of first useful demo

On macOS, one command can simulate a reproducible 10 MB application transfer with a mailbox-based or synthetic direct link, emit fragmentation/queue/service counters, estimated completion time, application goodput and integrity status. A second command or flag compares two scenarios differing in mailbox size or service frequency. A README shows how to reproduce the same results and lists what they cannot predict about TwinCAT/ESC hardware.

## How to work

1. Read all docs. Summarize key unknowns, do not silently resolve hardware assumptions.
2. Implement M0/M1 only with build, test, lint, and runnable CLI.
3. Review the roadmap and update checkboxes and ADRs based on evidence.
4. Report simulated vs measured separately.
5. Prepare interfaces for Linux/Windows master and Toradex slave only after passing the lab preflight gate.
