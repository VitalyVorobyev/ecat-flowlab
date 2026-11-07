import { Badge, Panel, ReadoutStrip, Table, type Column } from "@vitavision/ui";
import type { ReactNode } from "react";

import type { SimResult, SlaveResult, TransferResult } from "../api";
import { bitRate, count, hex, ns, percent } from "../lib/format";
import { payload } from "./spec";

function finish(r: SimResult): number | null {
  if (!r.transfers.every((t) => t.verified)) return null;
  return r.transfers.reduce((m, t) => Math.max(m, t.end_ns ?? 0), 0);
}

/** `−37 %` / `+4.1 %` relative to a baseline, or null. */
export function relative(now: number | null, before: number | null | undefined): string | null {
  if (now === null || before === null || before === undefined || before <= 0) return null;
  const d = (now - before) / before;
  if (Math.abs(d) < 0.0005) return "±0 %";
  const digits = Math.abs(d) < 0.1 ? 1 : 0;
  return `${d < 0 ? "−" : "+"}${Math.abs(100 * d).toFixed(digits)} %`;
}

function Track({ start, end, span }: { start: number; end: number | null; span: number }) {
  const left = (100 * start) / span;
  const width = end === null ? 100 - left : Math.max(0.6, (100 * (end - start)) / span);
  return (
    <div className="relative h-2 w-full rounded-full bg-raised" aria-hidden="true">
      <div
        className="absolute top-0 h-2 rounded-full bg-fg-subtle"
        style={{ left: `${left}%`, width: `${width}%` }}
      />
    </div>
  );
}

function transferColumns(r: SimResult, baseline: SimResult | null): Column<TransferResult>[] {
  const span = Math.max(1, r.virtual_time_ns);
  const cols: Column<TransferResult>[] = [
    { key: "i", header: "#", numeric: true, width: "2.5rem", cell: (t) => t.index },
    {
      key: "t",
      header: "Transfer",
      cell: (t) => {
        const spec = r.spec.transfers[t.index];
        return (
          <span className="whitespace-nowrap">
            {t.protocol} {t.direction === "master_to_slave" ? "↓" : "↑"} {payload(t.payload_bytes)}{" "}
            <span className="text-fg-muted">
              {t.direction === "master_to_slave" ? "to" : "from"} {t.slave}
              {spec?.protocol.kind === "pdo_channel"
                ? `, ${spec.protocol.bytes_per_cycle} B/cycle`
                : ""}
            </span>
          </span>
        );
      },
    },
    {
      key: "d",
      header: "Time",
      numeric: true,
      width: "6.5rem",
      cell: (t) => (
        <span className="whitespace-nowrap">
          {t.duration_ns === null ? "unfinished" : ns(t.duration_ns)}
        </span>
      ),
      sortValue: (t) => t.duration_ns,
    },
  ];
  if (baseline) {
    cols.push({
      key: "vs",
      header: "vs. baseline",
      numeric: true,
      width: "6.5rem",
      cell: (t) => relative(t.duration_ns, baseline.transfers[t.index]?.duration_ns) ?? "–",
    });
  }
  cols.push(
    {
      key: "g",
      header: "Goodput",
      numeric: true,
      width: "7.5rem",
      cell: (t) => (
        <span className="whitespace-nowrap">
          {t.goodput_bps === null ? "–" : bitRate(t.goodput_bps)}
        </span>
      ),
      sortValue: (t) => t.goodput_bps,
    },
    {
      key: "b",
      header: "× link",
      numeric: true,
      width: "4.5rem",
      cell: (t) =>
        t.duration_ns === null || t.physical_bound_ns === 0
          ? "–"
          : `${(t.duration_ns / t.physical_bound_ns).toFixed(1)}×`,
    },
    {
      key: "tl",
      header: "When",
      width: "9rem",
      cell: (t) => <Track start={t.start_ns} end={t.end_ns} span={span} />,
    },
    {
      key: "v",
      header: "Check",
      width: "6rem",
      cell: (t) =>
        t.verified ? (
          <Badge tone="normal">verified</Badge>
        ) : (
          <Badge tone="defect">{t.completed ? "corrupt" : "incomplete"}</Badge>
        ),
    },
  );
  return cols;
}

const SLAVE_COLUMNS: Column<SlaveResult>[] = [
  {
    key: "st",
    header: "Station",
    width: "5.5rem",
    cell: (s) => <span className="font-mono">{hex(s.station)}</span>,
  },
  { key: "n", header: "Slave", cell: (s) => s.name },
  {
    key: "pdi",
    header: "PDI busy",
    numeric: true,
    cell: (s) => percent(s.pdi_busy_percent),
    sortValue: (s) => s.pdi_busy_percent,
  },
  {
    key: "ram",
    header: "ESC RAM used",
    numeric: true,
    cell: (s) => `${count(s.dpram_used_bytes)} / ${count(s.dpram_bytes)} B`,
  },
];

/** The run in one line, the transfers and the slaves' load. */
export function ScenarioResult({
  result,
  baseline,
  selected,
  onSelect,
  children,
}: {
  result: SimResult;
  baseline: SimResult | null;
  selected: number | null;
  onSelect: (transfer: number | null) => void;
  /** Shown between the transfers and the slaves. */
  children?: ReactNode;
}) {
  const r = result;
  const done = finish(r);
  const busiest = r.slaves.reduce<SlaveResult | null>(
    (m, s) => (m === null || s.pdi_busy_percent > m.pdi_busy_percent ? s : m),
    null,
  );
  const vs = baseline ? relative(done, finish(baseline)) : null;
  return (
    <div className="flex flex-col gap-3">
      <ReadoutStrip
        items={[
          {
            label: "all done after",
            value: done === null ? "not finished" : `${ns(done)}${vs ? ` (${vs})` : ""}`,
          },
          { label: "cycles", value: `${count(r.cycles)} × ${ns(r.cycle_time_ns)}` },
          { label: "link load", value: percent(r.link_utilization_percent) },
          {
            label: "busiest PDI",
            value: busiest ? `${percent(busiest.pdi_busy_percent)} ${busiest.name}` : null,
          },
          { label: "frames", value: count(r.frames) },
        ]}
      />
      <Panel
        title={
          <span className="flex items-baseline gap-2">
            Transfers
            <span className="text-xs font-normal text-fg-muted">
              select one to rank the changes for it
            </span>
          </span>
        }
      >
        <Table
          columns={transferColumns(r, baseline)}
          rows={r.transfers}
          rowKey={(t) => t.index}
          onRowClick={(t) => onSelect(selected === t.index ? null : t.index)}
          isRowActive={(t) => t.index === selected}
        />
      </Panel>
      {children}
      <Panel title="Slaves">
        <Table
          columns={SLAVE_COLUMNS}
          rows={r.slaves.filter((s) => s.pdi_busy_percent > 0 || s.dpram_used_bytes > 0)}
          rowKey={(s) => s.station}
        />
      </Panel>
    </div>
  );
}
