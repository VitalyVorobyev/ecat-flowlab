import {
  Badge,
  Empty,
  PageHeader,
  Panel,
  SegmentedControl,
  Select,
  Table,
  type Column,
} from "@vitavision/ui";

import type { CaptureReport } from "../api";
import type { WkcSlot } from "../api/gen/WkcSlot";
import { TimeHistogram, TimeSeries } from "../components/charts";
import { DistributionTable } from "../components/DistributionTable";
import { Metric, MetricGrid } from "../components/Metric";
import { captureTime, count, ns } from "../lib/format";

type Event = { frame: number; t_ns: number; kind: "lost" | "wkc" };

const EVENT_COLUMNS: Column<Event>[] = [
  { key: "t", header: "Time", numeric: true, cell: (e) => captureTime(e.t_ns) },
  { key: "f", header: "Frame", numeric: true, cell: (e) => e.frame },
  {
    key: "k",
    header: "Event",
    cell: (e) =>
      e.kind === "lost" ? (
        <Badge tone="defect">no return</Badge>
      ) : (
        <Badge tone="defect">WKC mismatch</Badge>
      ),
  },
];

const WKC_COLUMNS: Column<WkcSlot>[] = [
  { key: "p", header: "#", numeric: true, width: "2.5rem", cell: (w) => w.position },
  {
    key: "l",
    header: "Datagram",
    cell: (w) => <span className="font-mono text-xs">{w.label}</span>,
  },
  {
    key: "e",
    header: "Expected",
    numeric: true,
    cell: (w) =>
      w.mailbox ? (
        <span
          className="text-xs text-fg-subtle"
          title="Mailbox access: WKC 0 means the mailbox was full or empty, so it is not judged."
        >
          varies (mailbox)
        </span>
      ) : (
        <span
          title={
            w.expected_from === "eni"
              ? "Configured in the ENI"
              : "Most frequent value in the capture"
          }
        >
          {w.expected}
          <span className="ml-1 text-xs text-fg-subtle">
            {w.expected_from === "eni" ? "ENI" : "mode"}
          </span>
        </span>
      ),
  },
  {
    key: "m",
    header: "Mismatches",
    numeric: true,
    cell: (w) =>
      w.mailbox ? (
        "–"
      ) : (
        <span className={w.mismatches > 0 ? "text-defect" : undefined}>{count(w.mismatches)}</span>
      ),
  },
  { key: "z", header: "Zero", numeric: true, cell: (w) => count(w.zero) },
];

/** Cycle timing, round trip and working counters of one cyclic stream. */
export function Timing({
  report,
  streamId,
  onStreamChange,
  onOpenFrame,
}: {
  report: CaptureReport;
  streamId: number;
  onStreamChange: (id: number) => void;
  onOpenFrame: (no: number) => void;
}) {
  const streams = report.streams;
  const s = streams.find((x) => x.id === streamId) ?? streams[0];
  if (!s) {
    return (
      <div className="p-6">
        <Empty>No cyclic stream: no datagram layout repeats at a regular interval.</Empty>
      </div>
    );
  }
  const options = streams.map((x) => ({
    value: String(x.id),
    label: `Stream ${x.id} · ${ns(x.period_ns)}`,
  }));
  const picker =
    streams.length <= 4 ? (
      <SegmentedControl
        aria-label="Stream"
        value={String(s.id)}
        options={options}
        onValueChange={(v) => onStreamChange(Number(v))}
      />
    ) : (
      <Select
        aria-label="Stream"
        value={String(s.id)}
        options={options}
        onValueChange={(v) => onStreamChange(Number(v))}
      />
    );

  const events: Event[] = [
    ...s.unanswered_events.map((e) => ({ ...e, kind: "lost" as const })),
    ...s.wkc_events.map((e) => ({ ...e, kind: "wkc" as const })),
  ].sort((a, b) => a.t_ns - b.t_ns);

  return (
    <div className="mx-auto flex max-w-[110rem] flex-col gap-5 p-6">
      <PageHeader
        title="Timing"
        meta={<span className="font-mono text-xs">{s.signature}</span>}
        actions={picker}
      />
      <MetricGrid>
        <Metric label="Period" value={ns(s.period_ns)} sub={`${count(s.frames)} TX frames`} />
        <Metric
          label="Interval σ"
          value={s.interval ? ns(s.interval.std) : "–"}
          sub={s.interval ? `range ${ns(s.interval.min)} – ${ns(s.interval.max)}` : undefined}
        />
        <Metric
          label="Send jitter |p99|"
          value={s.phase_jitter_abs_p99_ns != null ? ns(s.phase_jitter_abs_p99_ns) : "–"}
          sub={s.phase_jitter ? `σ ${ns(s.phase_jitter.std)}` : undefined}
        />
        <Metric
          label="Round trip p50"
          value={s.round_trip ? ns(s.round_trip.p50) : "–"}
          sub={s.round_trip ? `max ${ns(s.round_trip.max)}` : "needs directions"}
        />
        <Metric
          label="Gaps"
          value={count(s.gaps)}
          verdict={s.gaps > 0 ? "warn" : "normal"}
          sub={`${count(s.missed_cycles)} cycles missing`}
        />
        <Metric
          label="Lost"
          value={count(s.unanswered)}
          verdict={s.unanswered > 0 ? "defect" : "normal"}
        />
        <Metric
          label="Frame size"
          value={`${s.frame_bytes} B`}
          sub={`${ns(s.frame_wire_ns)} on the wire`}
        />
      </MetricGrid>

      <div className="grid gap-5 xl:grid-cols-2">
        <Panel title="Interval over time">
          <TimeSeries
            label="Cycle interval over capture time"
            yLabel="interval"
            series={[{ name: "interval", points: s.interval_series }]}
          />
        </Panel>
        <Panel title="Round trip over time">
          {s.round_trip_series.length > 0 ? (
            <TimeSeries
              label="Round trip over capture time"
              yLabel="round trip"
              series={[{ name: "round trip", points: s.round_trip_series }]}
            />
          ) : (
            <Empty>Round trip needs TX and RX frames to be told apart.</Empty>
          )}
        </Panel>
        {s.phase_jitter ? (
          <Panel title="Send-time deviation from the cycle grid">
            <TimeHistogram
              d={s.phase_jitter}
              label="Histogram of phase jitter"
              xLabel="deviation"
              markers={[{ position: 0, label: "ideal" }]}
            />
          </Panel>
        ) : null}
        {s.interval ? (
          <Panel title="Cycle-to-cycle interval">
            <TimeHistogram
              d={s.interval}
              label="Histogram of intervals"
              xLabel="interval"
              markers={[{ position: s.period_ns, label: "period" }]}
              logY
            />
          </Panel>
        ) : null}
        {s.round_trip ? (
          <Panel title="Round trip">
            <TimeHistogram
              d={s.round_trip}
              label="Histogram of round trip"
              xLabel="round trip"
              logY
            />
          </Panel>
        ) : null}
        <Panel title="Working counters">
          <Table
            columns={WKC_COLUMNS}
            rows={s.wkc}
            rowKey={(w) => w.position}
            empty="Working counters need returning frames."
          />
        </Panel>
      </div>

      <Panel title="Statistics">
        <DistributionTable
          rows={[
            { name: "Cycle interval (no gaps)", d: s.interval },
            { name: "Send-time deviation", d: s.phase_jitter },
            { name: "Round trip", d: s.round_trip },
          ]}
        />
      </Panel>

      <Panel title={`Events (${count(events.length)})`}>
        <Table
          columns={EVENT_COLUMNS}
          rows={events}
          rowKey={(e) => `${e.kind}${e.frame}`}
          onRowClick={(e) => onOpenFrame(e.frame)}
          empty="No lost frames and no working-counter mismatches."
          windowed={events.length > 30 ? { rowHeight: 36, height: 360 } : undefined}
        />
      </Panel>
    </div>
  );
}
