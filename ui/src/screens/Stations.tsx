import {
  Badge,
  Empty,
  PageHeader,
  Panel,
  SegmentedControl,
  Table,
  type Column,
} from "@vitavision/ui";
import { useState } from "react";

import type { AlEvent, CaptureReport, StationSummary } from "../api";
import { StateTimeline } from "../components/StateTimeline";
import { captureTime, count, hex } from "../lib/format";

export const STATION_COLUMNS: Column<StationSummary>[] = [
  {
    key: "st",
    header: "Station",
    width: "6rem",
    cell: (s) => <span className="font-mono">{hex(s.station)}</span>,
    sortValue: (s) => s.station,
  },
  {
    key: "name",
    header: "Name",
    cell: (s) => <span className="text-xs">{s.name ?? "–"}</span>,
  },
  {
    key: "pos",
    header: "Position",
    numeric: true,
    cell: (s) => s.position ?? "–",
    sortValue: (s) => s.position ?? null,
  },
  {
    key: "dg",
    header: "Datagrams",
    numeric: true,
    cell: (s) => count(s.datagrams),
    sortValue: (s) => s.datagrams,
  },
  { key: "r", header: "Reads", numeric: true, cell: (s) => count(s.reads) },
  { key: "w", header: "Writes", numeric: true, cell: (s) => count(s.writes) },
  {
    key: "nr",
    header: "No response",
    numeric: true,
    cell: (s) => (
      <span className={s.no_response > 0 ? "text-defect" : undefined}>{count(s.no_response)}</span>
    ),
    sortValue: (s) => s.no_response,
  },
  {
    key: "req",
    header: "Requested",
    cell: (s) => <span className="font-mono text-xs">{s.requested_state ?? "–"}</span>,
  },
  {
    key: "rep",
    header: "Reported",
    cell: (s) =>
      s.reported_state === null ? (
        "–"
      ) : s.al_error ? (
        <Badge tone="defect">{s.reported_state} + error</Badge>
      ) : (
        <span className="font-mono text-xs">{s.reported_state}</span>
      ),
  },
  {
    key: "areas",
    header: "Register areas",
    cell: (s) => (
      <span className="text-xs text-fg-muted">
        {s.areas
          .slice(0, 3)
          .map((a) => `${a.area} ${count(a.count)}`)
          .join(" · ")}
      </span>
    ),
  },
];

const EVENT_COLUMNS: Column<AlEvent>[] = [
  { key: "t", header: "Time", numeric: true, width: "8rem", cell: (e) => captureTime(e.t_ns) },
  { key: "f", header: "Frame", numeric: true, width: "5rem", cell: (e) => e.frame },
  {
    key: "s",
    header: "Station",
    width: "6rem",
    cell: (e) => <span className="font-mono">{e.station === null ? "all" : hex(e.station)}</span>,
  },
  {
    key: "k",
    header: "Event",
    width: "7rem",
    cell: (e) => (e.kind === "requested" ? "requested" : "reported"),
  },
  {
    key: "st",
    header: "State",
    cell: (e) =>
      e.error ? (
        <Badge tone={e.kind === "reported" ? "defect" : "neutral"}>
          {e.state} + {e.kind === "reported" ? "error" : "ack"}
        </Badge>
      ) : (
        <span className="font-mono text-xs">{e.state}</span>
      ),
  },
];

/** Stations addressed in the capture and the AL state machine over time. */
export function Stations({
  report,
  onOpenFrame,
}: {
  report: CaptureReport;
  onOpenFrame: (no: number) => void;
}) {
  const [range, setRange] = useState<"startup" | "all">("startup");
  const lastEvent = report.al_events.reduce((m, e) => Math.max(m, e.t_ns), 0);
  const end = range === "startup" && lastEvent > 0 ? lastEvent * 1.15 : report.summary.duration_ns;

  if (report.stations.length === 0) {
    return (
      <div className="p-6">
        <Empty>No configured-address (FP*) datagrams in this capture.</Empty>
      </div>
    );
  }

  return (
    <div className="mx-auto flex max-w-[110rem] flex-col gap-5 p-6">
      <PageHeader
        title="Stations"
        meta={`${report.stations.length} stations addressed · ${report.summary.slaves_responding ?? "?"} slaves answered the broadcast count`}
      />
      <Panel title="Stations">
        <Table columns={STATION_COLUMNS} rows={report.stations} rowKey={(s) => s.station} />
      </Panel>
      <Panel
        title="AL state over time"
        actions={
          <SegmentedControl
            aria-label="Time range"
            size="sm"
            value={range}
            options={[
              { value: "startup", label: "Start-up" },
              { value: "all", label: "Whole capture" },
            ]}
            onValueChange={(v) => setRange(v as "startup" | "all")}
          />
        }
      >
        <StateTimeline
          stations={report.stations.map((s) => s.station)}
          events={report.al_events}
          end={end}
        />
      </Panel>
      <Panel title={`AL events (${count(report.al_events.length)})`}>
        <Table
          columns={EVENT_COLUMNS}
          rows={report.al_events}
          rowKey={(e) => `${e.frame}-${e.station ?? "all"}-${e.kind}`}
          onRowClick={(e) => onOpenFrame(e.frame)}
          windowed={report.al_events.length > 40 ? { rowHeight: 36, height: 480 } : undefined}
          empty="No AL control writes or AL status reads."
        />
      </Panel>
    </div>
  );
}
