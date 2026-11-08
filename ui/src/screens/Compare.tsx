import { Empty, PageHeader, Panel, Select, SkeletonRows, Table, type Column } from "@vitavision/ui";
import { useState } from "react";

import { api, type CaptureInfo, type CaptureReport } from "../api";
import { ProvenanceBadge } from "../components/ProvenanceBadge";
import { bitRate, count, ns, percent } from "../lib/format";
import { useAsync } from "../lib/useAsync";

type Quantity = {
  name: string;
  value: (r: CaptureReport) => number | null;
  show: (v: number) => string;
};

const sum = <T,>(xs: T[], f: (x: T) => number) => xs.reduce((n, x) => n + f(x), 0);

const QUANTITIES: Quantity[] = [
  { name: "Frames", value: (r) => r.summary.frames, show: count },
  { name: "Duration", value: (r) => r.summary.duration_ns, show: (v) => ns(v) },
  {
    name: "Cycle period (stream 0)",
    value: (r) => r.streams[0]?.period_ns ?? null,
    show: (v) => ns(v),
  },
  {
    name: "|Send jitter| p99",
    value: (r) => r.streams[0]?.phase_jitter_abs_p99_ns ?? null,
    show: (v) => ns(v),
  },
  {
    name: "Round trip p50",
    value: (r) => r.streams[0]?.round_trip?.p50 ?? null,
    show: (v) => ns(v),
  },
  { name: "Lost cyclic frames", value: (r) => sum(r.streams, (s) => s.unanswered), show: count },
  {
    name: "WKC mismatches",
    value: (r) => sum(r.streams, (s) => s.wkc_mismatch_frames),
    show: count,
  },
  { name: "Link load TX (mean)", value: (r) => r.utilization.mean_tx_percent, show: percent },
  {
    name: "Mailbox messages to slaves",
    value: (r) => sum(r.mailbox.channels, (c) => c.messages_out),
    show: count,
  },
  {
    name: "Mailbox messages from slaves",
    value: (r) => sum(r.mailbox.channels, (c) => c.messages_in),
    show: count,
  },
  {
    name: "Rejected mailbox writes",
    value: (r) => sum(r.mailbox.channels, (c) => c.writes_rejected),
    show: count,
  },
  {
    name: "Empty mailbox reads",
    value: (r) => sum(r.mailbox.channels, (c) => c.empty_reads),
    show: count,
  },
  { name: "SDO transfers", value: (r) => r.mailbox.coe.transactions_total, show: count },
  {
    name: "SDO duration p50",
    value: (r) => r.mailbox.coe.duration?.p50 ?? null,
    show: (v) => ns(v),
  },
  {
    name: "FoE goodput (first transfer)",
    value: (r) => r.mailbox.foe[0]?.goodput_bps ?? null,
    show: bitRate,
  },
  { name: "EoE frames reassembled", value: (r) => r.mailbox.eoe.frames_total, show: count },
];

type Row = { q: Quantity; a: number | null; b: number | null };

function columns(a: CaptureReport, b: CaptureReport): Column<Row>[] {
  const head = (r: CaptureReport) => (
    <span className="flex items-center gap-2">
      <span className="truncate">{r.meta.input.name}</span>
      <ProvenanceBadge provenance={r.meta.source} />
    </span>
  );
  return [
    { key: "q", header: "Quantity", cell: (x) => x.q.name },
    { key: "a", header: head(a), numeric: true, cell: (x) => (x.a === null ? "–" : x.q.show(x.a)) },
    { key: "b", header: head(b), numeric: true, cell: (x) => (x.b === null ? "–" : x.q.show(x.b)) },
    {
      key: "r",
      header: "B / A",
      numeric: true,
      cell: (x) => (x.a === null || x.b === null || x.a === 0 ? "–" : `${(x.b / x.a).toFixed(3)}×`),
    },
  ];
}

/** Two captures side by side — e.g. a measurement and a simulation of it — each column labelled. */
export function Compare({ captures }: { captures: CaptureInfo[] }) {
  const [a, setA] = useState<number | null>(captures[0]?.id ?? null);
  const [b, setB] = useState<number | null>(captures[1]?.id ?? null);
  const reports = useAsync(
    () =>
      a === null || b === null
        ? Promise.resolve(null)
        : Promise.all([api.report(a), api.report(b)]),
    [a, b],
  );
  const pick = (label: string, value: number | null, set: (v: number | null) => void) => (
    <Select
      aria-label={label}
      className="w-64"
      value={value === null ? "" : String(value)}
      placeholder={label}
      options={captures.map((c) => ({ value: String(c.id), label: c.name, note: c.provenance }))}
      onValueChange={(v) => set(v ? Number(v) : null)}
    />
  );
  if (captures.length < 2) {
    return (
      <div className="p-6">
        <Empty>
          Load two captures — for example a measurement and a simulation of the same network — to
          compare them.
        </Empty>
      </div>
    );
  }
  const data = reports.data;
  return (
    <div className="mx-auto flex max-w-[90rem] flex-col gap-5 p-6">
      <PageHeader
        title="Compare"
        meta="The same quantities from two analysed captures, side by side. Each column keeps its own provenance; nothing is merged."
        actions={
          <>
            {pick("Capture A", a, setA)}
            {pick("Capture B", b, setB)}
          </>
        }
      />
      <Panel title="Quantities">
        {data ? (
          <Table
            columns={columns(data[0], data[1])}
            rows={QUANTITIES.map((q) => ({ q, a: q.value(data[0]), b: q.value(data[1]) }))}
            rowKey={(x) => x.q.name}
          />
        ) : (
          <SkeletonRows rows={8} />
        )}
      </Panel>
    </div>
  );
}
