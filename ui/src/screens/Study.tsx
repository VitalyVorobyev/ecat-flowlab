import { SchemaValueForm, type JsonSchema } from "@vitavision/forms";
import {
  Badge,
  Button,
  Callout,
  Disclosure,
  ErrorBox,
  PageHeader,
  Panel,
  Select,
  SkeletonRows,
  Table,
  type Column,
} from "@vitavision/ui";
import { FlaskConical, Play, RotateCcw } from "lucide-react";
import { useEffect, useState } from "react";

import {
  api,
  type CaptureInfo,
  errorMessage,
  type Residual,
  type StudyResult,
  type StudyRun,
} from "../api";
import { Metric, MetricGrid } from "../components/Metric";
import { ProvenanceBadge } from "../components/ProvenanceBadge";
import { ScatterPlot } from "../components/ScatterPlot";
import { downloadJson } from "../lib/download";
import { count } from "../lib/format";

function short(value: string): string {
  // Policies and other objects: their "kind" says enough in a table cell.
  const kind = /"kind":"([a-z_]+)"/.exec(value);
  return kind ? (kind[1] ?? value).replaceAll("_", " ") : value;
}

function num(v: number | null | undefined): string {
  if (v === null || v === undefined) return "–";
  return Math.abs(v) >= 1000 ? v.toFixed(0) : v.toPrecision(4);
}

function runColumns(r: StudyResult): Column<StudyRun>[] {
  return [
    {
      key: "i",
      header: "Run",
      numeric: true,
      width: "3.5rem",
      cell: (x) => x.index,
      sortValue: (x) => x.index,
    },
    ...r.parameters.map((p, k) => ({
      key: `p${k}`,
      header: <span title={p}>{p.split("/").filter(Boolean).slice(-2).join("/")}</span>,
      cell: (x: StudyRun) => <span className="font-mono text-xs">{short(x.values[k] ?? "")}</span>,
      sortValue: (x: StudyRun) => x.numeric[k] ?? null,
    })),
    ...r.objectives.map((o, k) => ({
      key: `o${k}`,
      header: <span title={`${r.goals[k]} ${o}`}>{o}</span>,
      numeric: true,
      cell: (x: StudyRun) => num(x.objectives[k]),
      sortValue: (x: StudyRun) => x.objectives[k] ?? null,
    })),
    {
      key: "s",
      header: "Status",
      cell: (x) =>
        !x.feasible ? (
          <span className="text-xs text-fg-muted" title={x.reason ?? undefined}>
            infeasible: {x.reason}
          </span>
        ) : x.index === r.best ? (
          <Badge tone="info">best</Badge>
        ) : x.pareto ? (
          <Badge tone="neutral">Pareto</Badge>
        ) : null,
    },
  ];
}

const RESIDUAL_COLUMNS: Column<Residual>[] = [
  { key: "l", header: "Simulated quantity", cell: (x) => x.label },
  { key: "s", header: "Measured from", cell: (x) => <span className="text-xs">{x.source}</span> },
  { key: "m", header: "Measured", numeric: true, cell: (x) => num(x.measured) },
  { key: "v", header: "Simulated", numeric: true, cell: (x) => num(x.simulated) },
  {
    key: "e",
    header: "Error",
    numeric: true,
    cell: (x) =>
      x.relative_error === null
        ? "–"
        : `${x.relative_error >= 0 ? "+" : "−"}${Math.abs(100 * x.relative_error).toFixed(2)} %`,
  },
];

/** Axes of the scatter: two objectives, or the first numeric parameter against the first objective. */
function axes(r: StudyResult): {
  x: (x: StudyRun) => number | null;
  y: (x: StudyRun) => number | null;
  xl: string;
  yl: string;
} | null {
  if (r.objectives.length >= 2) {
    return {
      x: (x) => x.objectives[0] ?? null,
      y: (x) => x.objectives[1] ?? null,
      xl: r.objectives[0] ?? "",
      yl: r.objectives[1] ?? "",
    };
  }
  const p = r.runs[0]?.numeric.findIndex((v) => v !== null) ?? -1;
  if (r.objectives.length === 1 && p >= 0) {
    return {
      x: (x) => x.numeric[p] ?? null,
      y: (x) => x.objectives[0] ?? null,
      xl: r.parameters[p] ?? "",
      yl: r.objectives[0] ?? "",
    };
  }
  return null;
}

/** Parameter sweeps over the simulator: Pareto frontier, constraints, calibration. */
export function Study({
  captures,
  onCreated,
}: {
  captures: CaptureInfo[];
  onCreated: (info: CaptureInfo) => void;
}) {
  const [schema, setSchema] = useState<JsonSchema | null>(null);
  const [spec, setSpec] = useState<unknown>(null);
  const [capture, setCapture] = useState<number | null>(null);
  const [result, setResult] = useState<StudyResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [selected, setSelected] = useState<number | null>(null);

  useEffect(() => {
    let live = true;
    Promise.all([api.studySchema(), api.studyExample()]).then(
      ([s, example]) => {
        if (!live) return;
        setSchema(s);
        setSpec(example);
      },
      (e: unknown) => live && setError(errorMessage(e)),
    );
    return () => {
      live = false;
    };
  }, []);

  const run = async () => {
    setBusy(true);
    setError(null);
    try {
      const r = await api.study(spec, capture);
      setResult(r);
      setSelected(r.best);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const simulateRun = async (r: StudyRun) => {
    if (!r.spec) return;
    setBusy(true);
    try {
      onCreated(await api.simulate({ ...r.spec, capture: { enabled: true, max_frames: 200000 } }));
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const ax = result ? axes(result) : null;
  const chosen = result && selected !== null ? result.runs[selected] : undefined;

  return (
    <div className="mx-auto flex max-w-[110rem] flex-col gap-5 p-6">
      <PageHeader
        title="Study"
        meta="Vary simulator parameters, find the Pareto frontier of the objectives, or fit the model to a measured capture."
        actions={
          <>
            <Select
              aria-label="Calibrate against"
              className="w-60"
              value={capture === null ? "" : String(capture)}
              placeholder="No capture (sweep only)"
              unsetLabel="No capture (sweep only)"
              options={captures.map((c) => ({
                value: String(c.id),
                label: c.name,
                note: c.provenance,
              }))}
              onValueChange={(v) => setCapture(v ? Number(v) : null)}
            />
            <Button
              variant="ghost"
              icon={<RotateCcw className="size-4" />}
              onClick={() => void api.studyExample().then(setSpec)}
            >
              Example
            </Button>
            <Button
              variant="primary"
              icon={<Play className="size-4" />}
              loading={busy}
              disabled={!schema}
              onClick={() => void run()}
            >
              Run study
            </Button>
          </>
        }
      />
      {error ? <ErrorBox>{error}</ErrorBox> : null}
      <div className="grid gap-5 xl:grid-cols-[minmax(0,5fr)_minmax(0,7fr)]">
        <Panel title="Study spec">
          {schema && spec ? (
            <SchemaValueForm schema={schema} value={spec} onValueChange={setSpec} />
          ) : (
            <SkeletonRows rows={8} />
          )}
        </Panel>
        <div className="flex flex-col gap-5">
          {result ? (
            <>
              <div className="flex flex-wrap items-center gap-2 text-xs text-fg-muted">
                <ProvenanceBadge provenance={result.meta.source} />
                <span>{result.description}</span>
                {result.calibrated_against ? (
                  <span>· fitted to {result.calibrated_against.name}</span>
                ) : null}
                <Button
                  variant="ghost"
                  size="sm"
                  className="ml-auto"
                  onClick={() => downloadJson("study.result.json", result)}
                >
                  Result JSON
                </Button>
              </div>
              <MetricGrid compact>
                <Metric label="Runs" value={count(result.runs.length)} />
                <Metric
                  label="Feasible"
                  value={count(result.feasible)}
                  sub={`${count(result.runs.length - result.feasible)} rejected`}
                />
                <Metric
                  label="Pareto-optimal"
                  value={count(result.pareto.length)}
                  hint="No other feasible run is at least as good on every objective and better on one."
                />
                <Metric
                  label="Best run"
                  value={result.best ?? "–"}
                  sub={
                    result.residuals.length > 0
                      ? "smallest fit error"
                      : (result.objectives[0] ?? "")
                  }
                />
              </MetricGrid>
              {result.residuals.length > 0 ? (
                <Panel title="Calibration residuals (best run)">
                  <Table
                    columns={RESIDUAL_COLUMNS}
                    rows={result.residuals}
                    rowKey={(x) => x.label}
                  />
                  <p className="mt-3 text-xs text-fg-muted">
                    A small error shows the model can reproduce these measurements; other parameter
                    sets may fit as well.
                  </p>
                </Panel>
              ) : null}
              {ax ? (
                <Panel title="Runs">
                  <ScatterPlot
                    label="Study runs"
                    xLabel={ax.xl}
                    yLabel={ax.yl}
                    highlightLabel="Pareto-optimal"
                    otherLabel="other feasible runs"
                    selected={selected}
                    onSelect={setSelected}
                    points={result.runs
                      .filter((x) => x.feasible)
                      .flatMap((x) => {
                        const px = ax.x(x);
                        const py = ax.y(x);
                        return px === null || py === null
                          ? []
                          : [{ id: x.index, x: px, y: py, highlight: x.pareto }];
                      })}
                  />
                </Panel>
              ) : null}
              {chosen ? (
                <Callout
                  tone="info"
                  title={`Run ${chosen.index}`}
                  actions={
                    chosen.spec ? (
                      <Button
                        size="sm"
                        icon={<FlaskConical className="size-4" />}
                        loading={busy}
                        onClick={() => void simulateRun(chosen)}
                      >
                        Simulate this run
                      </Button>
                    ) : undefined
                  }
                >
                  {result.parameters
                    .map((p, k) => `${p} = ${short(chosen.values[k] ?? "")}`)
                    .join(" · ")}
                  {chosen.spec ? "" : " — only Pareto-optimal and best runs keep their full spec."}
                </Callout>
              ) : null}
            </>
          ) : (
            <Callout tone="info" title="No study run yet">
              Edit the spec and press Run study. Runs are simulations without a capture; pick a
              capture above to fit targets that read it.
            </Callout>
          )}
        </div>
      </div>
      {result ? (
        <>
          <Panel title={`All runs (${count(result.runs.length)})`}>
            <Table
              columns={runColumns(result)}
              rows={result.runs}
              rowKey={(x) => x.index}
              onRowClick={(x) => setSelected(x.index)}
              windowed={result.runs.length > 40 ? { rowHeight: 36, height: 520 } : undefined}
            />
          </Panel>
          <Disclosure summary="What a study assumes" count={result.meta.assumptions.length}>
            <ul className="ml-4 list-disc space-y-1 text-sm text-fg-muted">
              {result.meta.assumptions.map((a) => (
                <li key={a}>{a}</li>
              ))}
            </ul>
          </Disclosure>
        </>
      ) : null}
    </div>
  );
}
