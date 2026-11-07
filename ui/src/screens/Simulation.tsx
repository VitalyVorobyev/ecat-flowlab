import {
  Badge,
  Button,
  Callout,
  Disclosure,
  PageHeader,
  Panel,
  Table,
  type Column,
} from "@vitavision/ui";
import { Download } from "lucide-react";

import type { SimResult, SlaveResult, TransferResult } from "../api";
import { Metric, MetricGrid } from "../components/Metric";
import { ProvenanceBadge } from "../components/ProvenanceBadge";
import { downloadJson } from "../lib/download";
import { bitRate, bytes, count, hex, ns, percent } from "../lib/format";

function Bar({ share }: { share: number }) {
  return (
    <div className="h-1.5 w-full rounded-full bg-raised">
      <div
        className="h-1.5 rounded-full bg-fg-subtle"
        style={{ width: `${Math.min(100, Math.max(0, 100 * share))}%` }}
      />
    </div>
  );
}

function transferColumns(maxGoodput: number): Column<TransferResult>[] {
  return [
    { key: "i", header: "#", numeric: true, width: "2.5rem", cell: (t) => t.index },
    { key: "p", header: "Protocol", width: "7rem", cell: (t) => t.protocol },
    {
      key: "d",
      header: "Direction",
      width: "8rem",
      cell: (t) => (t.direction === "master_to_slave" ? "master → slave" : "slave → master"),
    },
    {
      key: "st",
      header: "Slave",
      width: "8rem",
      cell: (t) => (
        <span className="font-mono text-xs" title={t.slave}>
          {hex(t.station)}
        </span>
      ),
    },
    { key: "b", header: "Payload", numeric: true, cell: (t) => bytes(t.payload_bytes) },
    {
      key: "t",
      header: "Time",
      numeric: true,
      cell: (t) => (t.duration_ns === null ? "unfinished" : ns(t.duration_ns)),
      sortValue: (t) => t.duration_ns,
    },
    {
      key: "g",
      header: "Goodput",
      numeric: true,
      cell: (t) => (t.goodput_bps === null ? "–" : bitRate(t.goodput_bps)),
      sortValue: (t) => t.goodput_bps,
    },
    {
      key: "bar",
      header: <span className="sr-only">Goodput share</span>,
      width: "8rem",
      cell: (t) => <Bar share={(t.goodput_bps ?? 0) / maxGoodput} />,
    },
    {
      key: "bound",
      header: "× link bound",
      numeric: true,
      cell: (t) =>
        t.duration_ns === null || t.physical_bound_ns === 0
          ? "–"
          : `${(t.duration_ns / t.physical_bound_ns).toFixed(1)}×`,
    },
    {
      key: "tcp",
      header: "TCP ref.",
      numeric: true,
      cell: (t) => ns(t.ethernet_tcp_reference_ns),
    },
    {
      key: "v",
      header: "Integrity",
      cell: (t) =>
        t.verified ? (
          <Badge tone="normal">SHA-256 ok</Badge>
        ) : (
          <Badge tone="defect">{t.completed ? "corrupt" : "incomplete"}</Badge>
        ),
    },
  ];
}

const DETAIL_COLUMNS: Column<TransferResult>[] = [
  { key: "i", header: "#", numeric: true, width: "2.5rem", cell: (t) => t.index },
  { key: "p", header: "Protocol", width: "7rem", cell: (t) => t.protocol },
  { key: "mo", header: "Msgs ↓", numeric: true, cell: (t) => count(t.messages_to_slave) },
  { key: "mi", header: "Msgs ↑", numeric: true, cell: (t) => count(t.messages_from_slave) },
  { key: "rej", header: "Rejected writes", numeric: true, cell: (t) => count(t.rejected_writes) },
  { key: "emp", header: "Empty reads", numeric: true, cell: (t) => count(t.empty_reads) },
  { key: "ch", header: "Chunk cycles", numeric: true, cell: (t) => count(t.chunk_cycles) },
  {
    key: "re",
    header: "Resent chunks",
    numeric: true,
    cell: (t) => count(t.retransmitted_chunks),
  },
  {
    key: "fb",
    header: "First byte",
    numeric: true,
    cell: (t) => (t.first_byte_ns === null ? "–" : ns(t.first_byte_ns - t.start_ns)),
  },
];

const SLAVE_COLUMNS: Column<SlaveResult>[] = [
  {
    key: "st",
    header: "Station",
    width: "6rem",
    cell: (s) => <span className="font-mono">{hex(s.station)}</span>,
  },
  { key: "n", header: "Name", cell: (s) => <span className="whitespace-nowrap">{s.name}</span> },
  {
    key: "pdi",
    header: "PDI busy",
    numeric: true,
    width: "7rem",
    cell: (s) => percent(s.pdi_busy_percent),
    sortValue: (s) => s.pdi_busy_percent,
  },
  {
    key: "bar",
    header: <span className="sr-only">PDI share</span>,
    width: "10rem",
    cell: (s) => <Bar share={s.pdi_busy_percent / 100} />,
  },
  {
    key: "ram",
    header: "ESC RAM used",
    numeric: true,
    cell: (s) => (
      <span className="whitespace-nowrap">
        {count(s.dpram_used_bytes)} / {count(s.dpram_bytes)} B
      </span>
    ),
  },
];

/** A simulation run: what each transfer took, against the physical bound and plain Ethernet. */
export function Simulation({ result }: { result: SimResult }) {
  const r = result;
  const maxGoodput = Math.max(1, ...r.transfers.map((t) => t.goodput_bps ?? 0));
  const verified = r.transfers.filter((t) => t.verified).length;
  const busiest = r.slaves.reduce<SlaveResult | null>(
    (m, s) => (m === null || s.pdi_busy_percent > m.pdi_busy_percent ? s : m),
    null,
  );
  return (
    <div className="mx-auto flex max-w-[110rem] flex-col gap-5 p-6">
      <PageHeader
        title="Simulation"
        meta={
          <span className="flex flex-wrap items-center gap-x-3 gap-y-1">
            <ProvenanceBadge provenance={r.meta.source} />
            <span className="text-xs">{r.description}</span>
            <span className="font-mono text-xs">seed {r.seed}</span>
            <span className="font-mono text-xs" title={r.meta.input.sha256}>
              spec {r.meta.input.sha256.slice(0, 12)}…
            </span>
          </span>
        }
        actions={
          <Button
            variant="secondary"
            icon={<Download className="size-4" />}
            onClick={() => downloadJson(`sim-seed${r.seed}.result.json`, r)}
          >
            Result JSON
          </Button>
        }
      />
      <Callout tone="info" title="Model output">
        Every number here follows from the spec's parameters — link rate, cycle, mailbox policy, ESC
        memory, PDI rate and overhead. It is a prediction to compare with measurements, not a
        measurement. The capture of this run is analysed in the other views like any capture.
      </Callout>
      {r.capture_truncated ? (
        <Callout tone="warning" title="Capture truncated">
          The run wrote its first {count(r.capture_frames)} frames; the other views show only those.
          The results on this page cover the whole run.
        </Callout>
      ) : null}

      <MetricGrid>
        <Metric
          label="Transfers verified"
          value={`${verified} / ${r.transfers.length}`}
          verdict={verified === r.transfers.length ? "normal" : "defect"}
          hint="Every payload byte arrived in order and the receiver's SHA-256 equals the sender's."
        />
        <Metric
          label="Simulated time"
          value={ns(r.virtual_time_ns)}
          sub={`${count(r.cycles)} cycles`}
        />
        <Metric label="Cycle" value={ns(r.cycle_time_ns)} sub={`link ${bitRate(r.link_bps)}`} />
        <Metric label="Frames" value={count(r.frames)} sub={`${bytes(r.wire_bytes)} on the wire`} />
        <Metric
          label="Link load (TX)"
          value={percent(r.link_utilization_percent)}
          hint="Wire bits incl. preamble, FCS, padding and gap over the simulated time."
        />
        <Metric
          label="Busiest PDI"
          value={busiest ? percent(busiest.pdi_busy_percent) : "–"}
          sub={busiest ? `${hex(busiest.station)} ${busiest.name}` : undefined}
          hint="Share of the simulated time the slave application spent accessing its ESC. Near 100 % the PDI is the bottleneck."
        />
        <Metric
          label="Mailbox policy"
          value={r.policy.split(":")[0] ?? r.policy}
          sub={r.policy.split(":")[1]}
        />
        <Metric
          label="Capture"
          value={count(r.capture_frames)}
          sub={r.capture_truncated ? "truncated" : "frames written"}
        />
      </MetricGrid>

      <Panel title="Transfers">
        <Table columns={transferColumns(maxGoodput)} rows={r.transfers} rowKey={(t) => t.index} />
        <p className="mt-3 text-xs text-fg-muted">
          <span className="font-medium">× link bound</span>: time over the time the payload alone
          needs at the link rate. <span className="font-medium">TCP ref.</span>: TCP over a
          dedicated link at the same rate, steady state, no loss — a lower bound for plain Ethernet,
          not a prediction.
        </p>
      </Panel>

      <div className="grid gap-5 xl:grid-cols-[minmax(0,7fr)_minmax(0,5fr)]">
        <Panel title="Protocol work">
          <Table columns={DETAIL_COLUMNS} rows={r.transfers} rowKey={(t) => t.index} />
        </Panel>
        <Panel title="Slaves">
          <Table columns={SLAVE_COLUMNS} rows={r.slaves} rowKey={(s) => s.station} />
        </Panel>
      </div>

      <Disclosure summary="Model assumptions" count={r.meta.assumptions.length}>
        <ul className="ml-4 list-disc space-y-1 text-sm text-fg-muted">
          {r.meta.assumptions.map((a) => (
            <li key={a}>{a}</li>
          ))}
        </ul>
      </Disclosure>
    </div>
  );
}
