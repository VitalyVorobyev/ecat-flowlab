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

- **Header**: a capture `Select` (each option noted with its provenance), a `ProvenanceBadge`, a
  `FileDrop` overlay ("Open capture…", `.pcap`, `.pcapng`, `.cap`), a live PDI slave status
  pill (`StatusDot`), and the `ThemeToggle`.
- **NavRail**: seven always-enabled workspaces — **Slave**, **Throughput**, **PHY**, **SPI Bench**,
  **Analysis**, **Simulate**, and **Planner**. The active route is mirrored in the URL hash
  (`#slave`, `#throughput`, `#phy`, `#spi-bench`, `#analysis`, `#simulate`, `#planner`, with
  legacy capture hashes such as `#frames` and `#overview` opening the matching tab inside
  **Analysis**).
- **Shell state**: `src/scenario/useScenario.ts` keeps the **Simulate** setup, result, and pinned
  baseline across workspace switches; `App.tsx` polls `GET /api/slave/status` every 1 s while the
  slave runs (or while on **Slave**) and wires **Apply to Simulate** from **SPI Bench**.
- **StatusBar**: provenance, capture name, frame count, span, direction mode, server version (or
  "server offline").

## Screens

`ui/src/screens/`:

| Screen | Shows |
|---|---|
| `SlaveControl` (*Slave*) | Configure, launch, monitor, and stop FlowLab's PDI slave (`flowlab pdi-slave`) on Linux `/dev/spidev*` or simulated LAN9252: AL state, SyncManagers, 1 Hz polls/msg/throughput charts, EoE bridge stats, and received payloads |
| `ThroughputLab` (*Throughput*) | Browser-to-server streaming HTTP/TCP download/upload (`fetch` + `XMLHttpRequest`) and RTT probe against `/api/bench/*` with pure-TS SHA-256 verification and CSV/JSON export |
| `PhyInspector` (*PHY*) | Read LAN9252 PHY 0–3 port cards, decoded registers, Auto-Neg & Auto-MDIX action, and raw MII register writes |
| `SpiBench` (*SPI Bench*) | Timed SPI read/write benchmarks and linear regression (`pdi-bench` + `pdi-fit`), residual table, CSV/JSON export, and one-click **Apply to Simulate** |
| `Analysis` (*Analysis*) | Consolidated 9-tab workspace wrapping `Captures`, `Live`, `Overview`, `Timing`, `Frames` (+ `FrameDetailView`), `Stations`, `Mailbox`, `Compare` (including FlowLab VoE goodput), and `Simulation` (*Sim run*) |
| `Scenario` (*Simulate*) | `SetupEditor` (master, slaves, transfers, run options in display units; a new slave starts from `GET /api/sim/slave`), presets, open/save/undo; after each edit (350 ms debounce, older requests aborted) `POST /api/sim/run` and `POST /api/sim/what-if`; `ScenarioResult`, `WhatIfPanel`, a pinned baseline, *Open frames* |
| `Planner` | A `PlanSpec` form (recomputed 250 ms after the last edit), ESI import adding devices to the plan, cyclic frames, idle time, mailbox headroom, checks, assumptions |

## Components built here

`ui/src/components/`:

| Component | Purpose |
|---|---|
| `HexView` (+ `hexLayout.ts`) | Hex dump with labelled byte ranges; hovering names the range, one datagram can be emphasized |
| `StateTimeline` | One lane per station with the AL state it reported, ticks where the master requested a state |
| `Metric`, `MetricGrid` | Headline numbers with an optional `verdict` |
| `ProvenanceBadge` | `Measured` / `Simulated` / `Synthetic` badge with a hint on what that means |
| `TimeHistogram`, `TimeSeries` (`charts.tsx`) | ns distributions and series in a readable unit |
| `DistributionTable`, `Findings`, `FilePickButton` | Statistics rows, findings by severity, a one-file picker |

`src/scenario/` holds the Simulate view's parts: `spec.ts` (display units, labels, JSON-pointer
edits, spec diffs), `useScenario.ts`, `SetupEditor`, `ScenarioResult`, `WhatIfPanel` (and its
`rank`), `SystemSummary`.

`HexView` and `StateTimeline` are candidates to upstream into lab-ui.
`src/lib/` holds `format.ts` (units always shown, decimal and binary units never mixed),
`useAsync.ts`, `download.ts` (`downloadJson`, `downloadText`), and `sha256.ts` (pure-TS streaming
SHA-256 and `xorshift64` pattern generator for plain HTTP origins without `crypto.subtle`).

## API client and generated types

`src/api/index.ts` is a typed client (`api.capabilities`, `api.report`, `api.frames`,
`api.simulate`, `api.slaveStatus`, `api.slaveStart`, `api.slaveStop`, `api.phyRead`, `api.phyAutoneg`,
`api.phyWrite`, `api.pdiBenchRun`, `api.benchPing`, `api.benchDownload`, `api.benchUploadUrl` …)
over `fetch`; a non-2xx answer becomes an `ApiError` carrying the server's `error` text, and a
transport failure says that `flowlab serve` is unreachable.

The types under `src/api/gen/` are generated from the Rust types marked `#[ts(export)]`
(`Capabilities`, `CaptureReport`, `FrameDetail`, `SimResult`, `PlanResult`, `SlaveStatus`,
`PhyReport`, `PdiBenchResult`, `UploadBenchResult` and the rest) by ts-rs during `cargo test`.
`.cargo/config.toml` sets `TS_RS_EXPORT_DIR = ui/src/api/gen` and `TS_RS_LARGE_INT = "number"`:
u64 values become `number`, which is exact because times sent to the UI are relative to capture
start and below 2⁵³ ns. The directory is generated — do not edit it; oxlint ignores it and CI
fails if `cargo test` changes it.

## Fixtures

`src/test/` holds real tool output, described with its regeneration commands in
`ui/src/test/README.md`:

| Fixture | Source |
|---|---|
| `report.fixture.json` | `flowlab analyze` of a small synthetic capture (demo spec, 400 cycles, 1 % loss and WKC errors) |
| `sim.fixture.json` | `flowlab simulate` of the example spec with 8 kB per transfer |
| `whatif.fixture.json` | `flowlab what-if` of the same spec |
| `plan.fixture.json` | `flowlab plan --json` (built-in example) |

The screen tests (`Overview`, `Compare`, `Mailbox`, `Planner`, `Scenario`, `Simulation`, `Live`,
`SlaveControl`, `ThroughputLab`, `PhyInspector`, `SpiBench`) render against these fixtures or typed
API mocks, so a report or API schema change that breaks a screen fails a test.

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
# terminal 1: the API (with simulated LAN9252 hardware views enabled)
cargo run -p flowlab-cli -- serve --sim-pdi results/demo.pcapng
# terminal 2: Vite on :5173, proxying /api, /bytes and /upload to http://127.0.0.1:7878
cd ui && bun run dev
```

Before committing:

```bash
cd ui && bun run lint && bun run typecheck && bun run test:run && bun run build && bun run fmt:check
```

`bun run build` writes `ui/dist`, which `flowlab serve` serves by default.
