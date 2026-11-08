# The UI

`ui/` is the workbench's React front end. It talks only to `flowlab serve` over the HTTP API,
renders analysis reports, simulation results, plans and studies, and builds its spec forms from
the JSON Schemas the server publishes, so the forms always match the Rust types. It contains no
analysis: every number it shows comes from the server.

Source: [ui/](https://github.com/VitalyVorobyev/ecat-flowlab/blob/main/ui) · user view:
[The workbench](../guide/ui.md)

## Stack

| Concern | Tool (from `ui/package.json`) |
|---|---|
| Framework | React 19, `react-dom/client` `createRoot`, `StrictMode` |
| Build and dev server | Vite (with `@vitejs/plugin-react`), TypeScript (`tsc --noEmit` before `vite build`) |
| Styling | Tailwind CSS 4 via `@tailwindcss/vite`, with the lab-ui token stylesheets |
| Package manager and runner | Bun (`bun.lock`) |
| Lint and format | oxlint (type-aware, `--max-warnings 0`, rules in `.oxlintrc.json`), oxfmt |
| Tests | Vitest with happy-dom, Testing Library (`@testing-library/react`, `jest-dom`, `user-event`) |
| Icons | `lucide-react` |

`src/styles.css` imports Tailwind and the stylesheets of all four lab-ui packages; `main.tsx`
calls `initTheme()` and wraps the app in `TooltipProvider` with a `Toaster`.

## lab-ui packages

| Package | Used here |
|---|---|
| `@vitavision/ui` | Primitives: `Badge`, `Button`, `ButtonLink`, `Callout`, `Dialog`, `Disclosure`, `Empty`, `ErrorBox`, `InfoHint`, `Input`, `PageHeader`, `Panel`, `SegmentedControl`, `Select`, `SkeletonRows`, `Table` (+ `Column`), `Tabs`, `ToggleChip`, `CountRun`, `DensityProvider`, `ThemeToggle`, `Toaster`, `toast`, `TooltipProvider`, `initTheme`, `cn` |
| `@vitavision/workbench` | Layout: `AppShell`, `NavRail`, `NavRailItem`, `StatusBar`, `SplitPane`, `FileDrop` |
| `@vitavision/charts` | `Histogram`, `LineChart`, `Legend`, `Frame`, `linearScale`, `extent`, `padDomain`, `areaFor`, `seriesColour`, `useFluidSize` |
| `@vitavision/forms` | `SchemaValueForm` (+ `JsonSchema`) for the synth, simulator, plan and study specs |

## Layout

`src/app/App.tsx` composes an `AppShell`:

- **Header**: a capture `Select` (each option noted with its provenance), a `ProvenanceBadge`,
  a *Generate* button (the synth dialog), a `FileDrop` overlay ("Open capture…", `.pcap`,
  `.pcapng`, `.cap`) and the `ThemeToggle`.
- **NavRail**: Simulate, Study, Planner, Captures, Overview, Timing, Frames, Stations, Mailbox
  (need a selected capture), Compare (needs two captures), Sim run (needs a simulated capture
  with a result). The view is mirrored in the URL hash (`#simulate`, `#frames`, …).
- **Scenario state** (`src/scenario/useScenario.ts`) lives in the shell, so the Simulate view
  keeps its setup, result and baseline while other views are open.
- **StatusBar**: provenance, capture name, frame count, span, direction mode, server version (or
  "server offline").

## Screens

`ui/src/screens/`:

| Screen | Shows |
|---|---|
| `Scenario` (*Simulate*) | `SetupEditor` (master, slaves, transfers, run options in display units), presets, open/save/undo; after each edit (350 ms debounce, older requests aborted) `POST /api/sim/run` and `POST /api/sim/what-if`; `ScenarioResult` (readout line, transfers with a timeline column, slaves), `WhatIfPanel` (single changes and pairs ranked for the run or the selected transfer, *Apply*), a pinned baseline with the changed parameters, *Open frames* |
| `Captures` | Drop or open a capture by path, generate one, the loaded captures with provenance |
| `Live` | Start a capture on an interface or a replay of a loaded capture (`POST /api/live/start`), poll `GET /api/live` every second while it runs: readouts of the latest second and the totals, jitter and round trip per second, problems per second, findings and stations of the window, assumptions; *Stop* loads the saved capture |
| `Overview` | Headline numbers (cycle period, send jitter, round trip, lost frames, WKC mismatches, missed cycles, link load, slaves responding), findings, link utilization over time, cycle-interval histogram, datagrams by command, attach/remove an ENI, how the numbers were computed |
| `Timing` | One cyclic stream: interval and phase-jitter histograms, interval over time, round trip histogram and over time, statistics table, working counters per datagram with their expectation basis |
| `Frames` | Server-side filtered frame list (pages of 1000) in a `SplitPane` beside `FrameDetailView`: header facts, decoded datagrams and the `HexView` |
| `Stations` | Stations addressed in the capture and the AL state over time (`StateTimeline`), with an event table |
| `Mailbox` | Channels with their address source, traffic by protocol, SDO transfers and durations, FoE transfers, EoE frames (with the reassembled-pcapng download), emergencies and mailbox errors |
| `Simulation` (*Sim run*) | A `SimResult`: the setup that was simulated (`SystemSummary`), the same result layout as Simulate, protocol work and the TCP reference, the model's assumptions; *Edit in Simulate*, result JSON download |
| `Planner` | A `PlanSpec` form (recomputed 250 ms after the last edit), ESI import adding devices to the plan, cyclic frames, idle time, mailbox headroom, checks, assumptions |
| `Study` | A `StudySpec` form, optional capture to calibrate against, runs table, `ScatterPlot` of the frontier, residuals; a frontier run can be simulated again with frames, which loads it as a capture |
| `Compare` | Two captures side by side, each column with its provenance badge, a fixed list of quantities and their ratio |

## Components built here

`ui/src/components/`:

| Component | Purpose |
|---|---|
| `HexView` (+ `hexLayout.ts`) | Hex dump with labelled byte ranges; hovering names the range, one datagram can be emphasized |
| `StateTimeline` | One lane per station with the AL state it reported, ticks where the master requested a state |
| `ScatterPlot` | Two-dimensional points from the charts primitives, highlighted ones in the first series colour |
| `SpecDialog` | Load a JSON Schema and example from the server, render them with `SchemaValueForm`, run the spec; `SynthDialog` is built on it |
| `Metric`, `MetricGrid` | Headline numbers with an optional `verdict` |
| `ProvenanceBadge` | `Measured` / `Simulated` / `Synthetic` badge with a hint on what that means |
| `TimeHistogram`, `TimeSeries` (`charts.tsx`) | ns distributions and series in a readable unit |
| `DistributionTable`, `Findings`, `FilePickButton` | Statistics rows, findings by severity, a one-file picker |

`src/scenario/` holds the Simulate view's parts: `spec.ts` (display units, labels, JSON-pointer
edits, spec diffs), `useScenario.ts`, `SetupEditor`, `ScenarioResult`, `WhatIfPanel` (and its
`rank`), `SystemSummary`.

`HexView`, `StateTimeline` and `ScatterPlot` are candidates to upstream into lab-ui.
`src/lib/` holds `format.ts` (units always shown, decimal and binary units never mixed),
`useAsync.ts` and `download.ts`.

## API client and generated types

`src/api/index.ts` is a typed client (`api.report`, `api.frames`, `api.simulate`, `api.study`
…) over `fetch`; a non-2xx answer becomes an `ApiError` carrying the server's `error` text, and a
transport failure says that `flowlab serve` is unreachable.

The types under `src/api/gen/` are generated from the Rust types marked `#[ts(export)]`
(`CaptureReport`, `FrameDetail`, `SimResult`, `PlanResult`, `StudyResult` and the rest) by ts-rs
during `cargo test`. `.cargo/config.toml` sets `TS_RS_EXPORT_DIR = ui/src/api/gen` and
`TS_RS_LARGE_INT = "number"`: u64 values become `number`, which is exact because times sent to
the UI are relative to capture start and below 2⁵³ ns. The directory is generated — do not edit
it; oxlint ignores it and CI fails if `cargo test` changes it.

## Fixtures

`src/test/` holds real tool output, described with its regeneration commands in
`ui/src/test/README.md`:

| Fixture | Source |
|---|---|
| `report.fixture.json` | `flowlab analyze` of a small synthetic capture (demo spec, 400 cycles, 1 % loss and WKC errors) |
| `sim.fixture.json` | `flowlab simulate` of the example spec with 8 kB per transfer |
| `whatif.fixture.json` | `flowlab what-if` of the same spec |
| `plan.fixture.json` | `flowlab plan --json` (built-in example) |
| `study.fixture.json` | `flowlab study` of `foe-sweep.json` without its PDI-rate parameter (8 runs) |

The screen tests (`Overview`, `Compare`, `Mailbox`, `Planner`, `Scenario`, `Simulation`, `Study`) render
against these files, so a report schema change that breaks a screen fails a test.

## Design rules

- lab-ui components first; tokens only (`text-fg-muted`, `border-line`, `bg-surface` …), no raw
  palette classes or hex colours.
- Verdict colours (`normal` / `warn` / `defect`) only where the app compared a value with a
  declared threshold — e.g. lost cyclic frames > 0, SDO aborts > 0, transfers not verified, cyclic
  frames longer than the cycle. Magnitudes such as periods, rates and percentages stay neutral.
- Provenance is shown wherever results are shown; Compare keeps it per column.
- Table columns are module-level constants (`CHANNEL_COLUMNS`, `SLAVE_COLUMNS` …) or factories
  (`transferColumns(maxGoodput)`, `runColumns(result)`), never components defined during render;
  oxlint's `react/no-unstable-nested-components` enforces it.
- React 19 idioms are linted: no `forwardRef`, `useContext` or `flushSync`.

## Development workflow

```bash
cd ui && bun install
# terminal 1: the API (and a capture to look at)
cargo run -p flowlab-cli -- serve results/demo.pcapng
# terminal 2: Vite on :5173, proxying /api to http://127.0.0.1:7878
cd ui && bun run dev
```

Before committing:

```bash
cd ui && bun run lint && bun run typecheck && bun run test:run && bun run build && bun run fmt:check
```

`bun run build` writes `ui/dist`, which `flowlab serve` serves by default.
