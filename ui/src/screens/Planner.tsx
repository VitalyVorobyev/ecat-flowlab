import { SchemaValueForm, type JsonSchema } from "@vitavision/forms";
import {
  Badge,
  Button,
  Disclosure,
  ErrorBox,
  PageHeader,
  Panel,
  SkeletonRows,
  Table,
  type Column,
} from "@vitavision/ui";
import { FileCode, Plus, RotateCcw } from "lucide-react";
import { useEffect, useState } from "react";

import {
  api,
  type DeviceDescription,
  errorMessage,
  type Finding,
  type PlannedFrame,
  type PlanResult,
  type PlanSlave,
} from "../api";
import { FilePickButton } from "../components/FilePickButton";
import { Findings } from "../components/Findings";
import { Metric, MetricGrid } from "../components/Metric";
import { bytes, count, hex, ns, percent } from "../lib/format";

type Spec = Record<string, unknown> & { slaves?: unknown[] };
type Imported = { device: DeviceDescription; slave: PlanSlave };

const FRAME_COLUMNS: Column<PlannedFrame & { i: number }>[] = [
  { key: "i", header: "#", numeric: true, width: "2.5rem", cell: (f) => f.i },
  {
    key: "d",
    header: "Datagrams",
    cell: (f) => (
      <span className="font-mono text-xs">
        {f.datagrams.map((d) => `${d.command} ${d.data_bytes} B`).join(" · ")}
      </span>
    ),
  },
  { key: "b", header: "Frame", numeric: true, cell: (f) => `${f.frame_bytes} B` },
  { key: "w", header: "Wire", numeric: true, cell: (f) => `${f.wire_bytes} B` },
  { key: "t", header: "Time", numeric: true, cell: (f) => ns(f.wire_ns) },
];

function deviceColumns(onAdd: (i: Imported) => void): Column<Imported>[] {
  return [
    {
      key: "t",
      header: "Type",
      cell: (r) => <span className="font-mono text-xs">{r.device.type_name}</span>,
    },
    {
      key: "id",
      header: "Product",
      cell: (r) => <span className="font-mono text-xs">{hex(r.device.product_code, 8)}</span>,
    },
    { key: "o", header: "Out", numeric: true, cell: (r) => `${r.slave.outputs_bytes} B` },
    { key: "in", header: "In", numeric: true, cell: (r) => `${r.slave.inputs_bytes} B` },
    {
      key: "m",
      header: "Mailbox",
      cell: (r) =>
        r.slave.mailbox_out_bytes > 0 ? (
          <span className="text-xs">
            {r.slave.mailbox_out_bytes}/{r.slave.mailbox_in_bytes} B ·{" "}
            {(r.slave.supports ?? []).join(" ")}
          </span>
        ) : (
          <span className="text-xs text-fg-subtle">none</span>
        ),
    },
    {
      key: "add",
      header: <span className="sr-only">Add</span>,
      width: "5rem",
      cell: (r) => (
        <Button
          size="sm"
          variant="ghost"
          icon={<Plus className="size-3.5" />}
          onClick={() => onAdd(r)}
        >
          Add
        </Button>
      ),
    },
  ];
}

function checksAsFindings(r: PlanResult): Finding[] {
  return r.checks.map((c) => ({
    severity: c.severity === "defect" ? "defect" : c.severity === "warn" ? "warn" : "info",
    title: c.subject,
    detail: c.message,
  }));
}

/** Frame layout and bus budget of a network, from numbers or imported ESI files. */
export function Planner() {
  const [schema, setSchema] = useState<JsonSchema | null>(null);
  const [spec, setSpec] = useState<Spec | null>(null);
  const [result, setResult] = useState<PlanResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [devices, setDevices] = useState<Imported[]>([]);

  useEffect(() => {
    let live = true;
    Promise.all([api.planSchema(), api.planExample()]).then(
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

  // Recompute shortly after the last edit.
  useEffect(() => {
    if (!spec) return;
    let live = true;
    const t = setTimeout(() => {
      api.plan(spec).then(
        (r) => {
          if (!live) return;
          setResult(r);
          setError(null);
        },
        (e: unknown) => live && setError(errorMessage(e)),
      );
    }, 250);
    return () => {
      live = false;
      clearTimeout(t);
    };
  }, [spec]);

  const importEsi = (file: File) => {
    api.importEsi(file).then(
      (imp) => {
        setDevices((list) => [
          ...list,
          ...imp.file.devices.flatMap((device, i) => {
            const slave = imp.slaves[i];
            return slave ? [{ device, slave }] : [];
          }),
        ]);
        setError(null);
      },
      (e: unknown) => setError(`${file.name}: ${errorMessage(e)}`),
    );
  };

  const add = (r: Imported) =>
    setSpec((s) => (s ? { ...s, slaves: [...(s.slaves ?? []), r.slave] } : s));

  return (
    <div className="mx-auto flex max-w-[110rem] flex-col gap-5 p-6">
      <PageHeader
        title="Planner"
        meta="Frame layout, wire time and headroom of a cycle; ESC RAM and mailbox capabilities. Arithmetic, not simulation."
        actions={
          <>
            <FilePickButton
              accept=".xml,application/xml,text/xml"
              icon={<FileCode className="size-4" />}
              onFile={importEsi}
            >
              Import ESI…
            </FilePickButton>
            <Button
              variant="ghost"
              icon={<RotateCcw className="size-4" />}
              onClick={() => void api.planExample().then((e) => setSpec(e as Spec))}
            >
              Example
            </Button>
          </>
        }
      />
      {error ? <ErrorBox>{error}</ErrorBox> : null}
      {devices.length > 0 ? (
        <Panel title={`Devices from ESI (${devices.length})`}>
          <Table
            columns={deviceColumns(add)}
            rows={devices}
            rowKey={(r) =>
              `${r.device.vendor_id}-${r.device.product_code}-${r.device.revision}-${r.device.type_name}`
            }
          />
        </Panel>
      ) : null}
      <div className="grid gap-5 xl:grid-cols-[minmax(0,5fr)_minmax(0,7fr)]">
        <Panel title="Network">
          {schema && spec ? (
            <SchemaValueForm
              schema={schema}
              value={spec}
              onValueChange={(v) => setSpec(v as Spec)}
            />
          ) : (
            <SkeletonRows rows={8} />
          )}
        </Panel>
        <div className="flex flex-col gap-5">
          {result ? (
            <>
              <MetricGrid compact>
                <Metric
                  label="Cyclic frames back"
                  value={ns(result.cyclic_done_ns)}
                  sub={`${result.frames.length} frame${result.frames.length === 1 ? "" : "s"}`}
                  verdict={result.cyclic_done_ns > result.cycle_ns ? "defect" : undefined}
                  hint="Wire time of the cyclic frames plus the ring delay, from cycle start."
                />
                <Metric
                  label="Idle per cycle"
                  value={percent(result.idle_percent)}
                  sub={ns(result.idle_ns)}
                  hint="Cycle time left for mailbox and acyclic traffic and for master jitter."
                />
                <Metric
                  label="Process image"
                  value={bytes(result.image_out_bytes + result.image_in_bytes)}
                  sub={`${count(result.image_out_bytes)} B out · ${count(result.image_in_bytes)} B in`}
                />
                <Metric
                  label="Mailbox frames"
                  value={result.mailbox_frames_per_cycle ?? "–"}
                  sub={
                    result.mailbox_bytes_per_s === null
                      ? "no mailbox"
                      : `≤ ${bytes(result.mailbox_bytes_per_s)}/s per direction`
                  }
                  hint="Mailbox frames (write + read of the largest mailbox) that fit in the idle time when the master waits for each. A bus-side bound: slave PDI time is not included."
                />
              </MetricGrid>
              <Panel title="Checks">
                <Findings findings={checksAsFindings(result)} />
              </Panel>
              <Panel title="Cyclic frames">
                <Table
                  columns={FRAME_COLUMNS}
                  rows={result.frames.map((f, i) => ({ ...f, i }))}
                  rowKey={(f) => f.i}
                />
                <p className="mt-3 flex flex-wrap items-center gap-2 text-xs text-fg-muted">
                  <Badge tone="neutral">
                    {percent(result.cyclic_percent)} of the cycle on the wire
                  </Badge>
                  ring delay {ns(result.ring_delay_ns)}
                </p>
              </Panel>
              <Disclosure summary="What the arithmetic assumes" count={result.assumptions.length}>
                <ul className="ml-4 list-disc space-y-1 text-sm text-fg-muted">
                  {result.assumptions.map((a) => (
                    <li key={a}>{a}</li>
                  ))}
                </ul>
              </Disclosure>
            </>
          ) : (
            <SkeletonRows rows={8} />
          )}
        </div>
      </div>
    </div>
  );
}
