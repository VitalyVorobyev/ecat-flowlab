import { Badge, Button, Disclosure, PageHeader, Panel, Table, type Column } from "@vitavision/ui";
import { Download, FileCode, Unlink } from "lucide-react";

import type { CaptureReport, CyclicStream } from "../api";
import { TimeHistogram, TimeSeries } from "../components/charts";
import { FilePickButton } from "../components/FilePickButton";
import { Findings } from "../components/Findings";
import { Metric, MetricGrid } from "../components/Metric";
import { ProvenanceBadge } from "../components/ProvenanceBadge";
import { downloadJson } from "../lib/download";
import { count, ns, percent, utc } from "../lib/format";

const DIRECTIONS = {
  mac_local_bit: "from source MAC",
  wkc_heuristic: "from working counters",
  unknown: "unknown",
} as const;

function wkcMismatches(s: CyclicStream): number {
  return s.wkc_mismatch_frames;
}

const STREAM_COLUMNS: Column<CyclicStream>[] = [
  { key: "id", header: "#", numeric: true, width: "2.5rem", cell: (x) => x.id },
  {
    key: "sig",
    header: "Datagrams",
    cell: (x) => <span className="font-mono text-xs">{x.signature}</span>,
  },
  { key: "n", header: "Frames", numeric: true, cell: (x) => count(x.frames) },
  { key: "p", header: "Period", numeric: true, cell: (x) => ns(x.period_ns) },
  {
    key: "j",
    header: "|Jitter| p99",
    numeric: true,
    cell: (x) => (x.phase_jitter_abs_p99_ns === null ? "–" : ns(x.phase_jitter_abs_p99_ns)),
  },
  { key: "b", header: "Bytes", numeric: true, cell: (x) => x.frame_bytes },
];

type CommandRow = CaptureReport["summary"]["commands"][number];

function commandColumns(maxCmd: number): Column<CommandRow>[] {
  return [
    {
      key: "c",
      header: "Command",
      width: "6rem",
      cell: (r) => <span className="font-mono">{r.command}</span>,
    },
    {
      key: "bar",
      header: <span className="sr-only">Share</span>,
      cell: (r) => (
        <div className="h-1.5 rounded-full bg-raised">
          <div
            className="h-1.5 rounded-full bg-fg-subtle"
            style={{ width: `${(100 * r.count) / maxCmd}%` }}
          />
        </div>
      ),
    },
    { key: "n", header: "Datagrams", numeric: true, width: "7rem", cell: (r) => count(r.count) },
  ];
}

/** The capture at a glance: headline numbers, findings, load and cycle shape. */
export function Overview({
  report,
  onOpenStream,
  eni = null,
  onAttachEni,
  onDetachEni,
}: {
  report: CaptureReport;
  onOpenStream: (id: number) => void;
  /** Master name of the attached network configuration, if any. */
  eni?: string | null;
  onAttachEni?: (file: File) => void;
  onDetachEni?: () => void;
}) {
  const { meta, summary: s, utilization: u } = report;
  const main = report.streams[0] ?? null;
  const lost = report.streams.reduce((n, x) => n + x.unanswered, 0);
  const wkc = report.streams.reduce((n, x) => n + wkcMismatches(x), 0);
  const missed = report.streams.reduce((n, x) => n + x.missed_cycles, 0);
  const known = s.direction_mode !== "unknown";

  const commandRows = s.commands.slice(0, 10);
  const maxCmd = commandRows[0]?.count ?? 1;

  return (
    <div className="mx-auto flex max-w-[110rem] flex-col gap-5 p-6">
      <PageHeader
        title={meta.input.name}
        meta={
          <span className="flex flex-wrap items-center gap-x-3 gap-y-1">
            <ProvenanceBadge provenance={meta.source} />
            <span className="font-mono text-xs">{s.format}</span>
            <span className="font-mono text-xs">{utc(s.start_unix_ms)}</span>
            <span className="font-mono text-xs">{ns(s.duration_ns)}</span>
            <span className="font-mono text-xs" title={meta.input.sha256}>
              sha256 {meta.input.sha256.slice(0, 12)}…
            </span>
          </span>
        }
        actions={
          <>
            {eni !== null ? (
              <span className="flex items-center gap-1">
                <span title="Expected working counters, mailboxes and names come from this network configuration.">
                  <Badge tone="info">ENI: {eni}</Badge>
                </span>
                {onDetachEni ? (
                  <Button
                    variant="ghost"
                    size="sm"
                    icon={<Unlink className="size-3.5" />}
                    onClick={onDetachEni}
                    aria-label="Remove the network configuration"
                  >
                    Remove
                  </Button>
                ) : null}
              </span>
            ) : onAttachEni ? (
              <FilePickButton
                accept=".xml,application/xml,text/xml"
                icon={<FileCode className="size-4" />}
                onFile={onAttachEni}
              >
                Attach ENI…
              </FilePickButton>
            ) : null}
            <Button
              variant="secondary"
              icon={<Download className="size-4" />}
              onClick={() =>
                downloadJson(meta.input.name.replace(/\.[^.]+$/, "") + ".report.json", report)
              }
            >
              Report JSON
            </Button>
          </>
        }
      />

      <MetricGrid>
        <Metric
          label="Cycle period"
          value={main ? ns(main.period_ns) : "–"}
          sub={main ? `stream 0 · ${count(main.frames)} frames` : "no cyclic stream"}
          hint="Least-squares slope of send time over cycle number for the most frequent cyclic frame."
        />
        <Metric
          label="Send jitter |p99|"
          value={main?.phase_jitter_abs_p99_ns != null ? ns(main.phase_jitter_abs_p99_ns) : "–"}
          sub={main?.phase_jitter ? `σ ${ns(main.phase_jitter.std)}` : undefined}
          hint="Deviation of each send time from the ideal grid, as timestamped by the capture tool."
        />
        <Metric
          label="Round trip p50"
          value={main?.round_trip ? ns(main.round_trip.p50) : "–"}
          sub={
            main?.round_trip
              ? `p99 ${ns(main.round_trip.p99)}`
              : known
                ? undefined
                : "needs directions"
          }
          hint="TX to returning RX at the capture point: serialization plus slave forwarding."
        />
        <Metric
          label="Lost frames"
          value={known ? count(lost) : "–"}
          verdict={known ? (lost > 0 ? "defect" : "normal") : undefined}
          sub={known ? `of ${count(main?.frames ?? 0)} cyclic` : "needs directions"}
          hint="Cyclic TX frames that never came back. Threshold: any."
        />
        <Metric
          label="WKC mismatches"
          value={known ? count(wkc) : "–"}
          verdict={known ? (wkc > 0 ? "defect" : "normal") : undefined}
          sub={
            main?.wkc.some((w) => w.expected_from === "eni")
              ? "vs. configured (ENI)"
              : "vs. most frequent value"
          }
          hint="Returning cyclic frames whose working counter differs from the usual value at that datagram. Threshold: any."
        />
        <Metric
          label="Missed cycles"
          value={count(missed)}
          verdict={missed > 0 ? "warn" : "normal"}
          sub={`${count(report.streams.reduce((n, x) => n + x.gaps, 0))} gaps`}
          hint="Cycles missing in gaps longer than 1.5 periods of the master's send times."
        />
        <Metric
          label="Link load (TX)"
          value={percent(u.mean_tx_percent)}
          sub={`peak ${percent(u.peak_tx_percent)} / ${ns(u.bin_ns, 0)}`}
          hint={`Wire bits incl. preamble, FCS, padding and gap, over a ${u.link_bps / 1e6} Mbit/s link.`}
        />
        <Metric
          label="Slaves responding"
          value={s.slaves_responding ?? "–"}
          sub={`${report.stations.length} stations addressed`}
          hint="Highest broadcast-read working counter on a returning frame."
        />
      </MetricGrid>

      <div className="grid gap-5 xl:grid-cols-[minmax(0,5fr)_minmax(0,7fr)]">
        <Panel title="Findings">
          <Findings findings={report.findings} />
        </Panel>
        <div className="flex flex-col gap-5">
          <Panel title="Link utilization">
            <TimeSeries
              label="Link utilization over time"
              yLabel="utilization (%)"
              yIsTime={false}
              yFromZero
              series={[
                { name: "TX", points: u.tx_series },
                ...(known ? [{ name: "RX", points: u.rx_series }] : []),
              ]}
            />
          </Panel>
          {main?.interval ? (
            <Panel title="Cycle intervals, stream 0">
              <TimeHistogram
                d={main.interval}
                label="Histogram of cycle-to-cycle intervals"
                xLabel="interval"
                markers={[{ position: main.period_ns, label: "period" }]}
                logY
              />
            </Panel>
          ) : null}
        </div>
      </div>

      <div className="grid gap-5 xl:grid-cols-[minmax(0,7fr)_minmax(0,5fr)]">
        <Panel title={`Cyclic streams (${report.streams.length})`}>
          <Table
            columns={STREAM_COLUMNS}
            rows={report.streams}
            rowKey={(x) => x.id}
            onRowClick={(x) => onOpenStream(x.id)}
            empty="No datagram layout repeats at a regular interval."
          />
        </Panel>
        <Panel title="Datagrams by command">
          <Table columns={commandColumns(maxCmd)} rows={commandRows} rowKey={(r) => r.command} />
        </Panel>
      </div>

      <Disclosure summary="How these numbers were computed" count={meta.assumptions.length}>
        <ul className="ml-4 list-disc space-y-1 text-sm text-fg-muted">
          {meta.assumptions.map((a) => (
            <li key={a}>{a}</li>
          ))}
          <li>
            Directions: {DIRECTIONS[s.direction_mode]} — {count(s.tx_frames)} TX,{" "}
            {count(s.rx_frames)} RX, {count(s.paired_frames)} paired.
          </li>
          <li>
            Report schema v{meta.schema_version}, {meta.tool} {meta.tool_version}.
          </li>
        </ul>
      </Disclosure>
    </div>
  );
}
