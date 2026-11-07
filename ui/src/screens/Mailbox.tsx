import {
  Badge,
  ButtonLink,
  Callout,
  Empty,
  PageHeader,
  Panel,
  Table,
  Tabs,
  type Column,
} from "@vitavision/ui";
import { Download } from "lucide-react";
import { useState } from "react";

import {
  api,
  type CaptureReport,
  type ChannelSource,
  type EmergencyEvent,
  type EoeFrame,
  type FoeTransfer,
  type MailboxChannel,
  type MailboxErrorEvent,
  type ProtocolStats,
  type SdoOutcome,
  type SdoTransaction,
} from "../api";
import { TimeHistogram } from "../components/charts";
import { Metric, MetricGrid } from "../components/Metric";
import { bitRate, bytes, captureTime, count, hex, ns } from "../lib/format";

const SOURCE: Record<ChannelSource, string> = {
  sm_config: "SM config",
  eni: "ENI",
  hint: "hint",
  heuristic: "heuristic",
};

function sm(start: number | null, len: number | null): string {
  if (start === null) return "–";
  return len === null ? hex(start) : `${hex(start)} · ${len} B`;
}

const CHANNEL_COLUMNS: Column<MailboxChannel>[] = [
  {
    key: "st",
    header: "Station",
    width: "6rem",
    cell: (c) => <span className="font-mono">{hex(c.station)}</span>,
    sortValue: (c) => c.station,
  },
  {
    key: "src",
    header: "Source",
    width: "7rem",
    cell: (c) => (
      <Badge tone={c.source === "heuristic" ? "unlabeled" : "neutral"}>{SOURCE[c.source]}</Badge>
    ),
  },
  {
    key: "out",
    header: "Out (to slave)",
    cell: (c) => <span className="font-mono text-xs">{sm(c.out_start, c.out_len)}</span>,
  },
  {
    key: "in",
    header: "In (from slave)",
    cell: (c) => <span className="font-mono text-xs">{sm(c.in_start, c.in_len)}</span>,
  },
  {
    key: "mo",
    header: "Msgs ↓",
    numeric: true,
    cell: (c) => count(c.messages_out),
    sortValue: (c) => c.messages_out,
  },
  {
    key: "mi",
    header: "Msgs ↑",
    numeric: true,
    cell: (c) => count(c.messages_in),
    sortValue: (c) => c.messages_in,
  },
  { key: "bo", header: "Bytes ↓", numeric: true, cell: (c) => bytes(c.bytes_out) },
  { key: "bi", header: "Bytes ↑", numeric: true, cell: (c) => bytes(c.bytes_in) },
  {
    key: "rej",
    header: "Rejected",
    numeric: true,
    cell: (c) => count(c.writes_rejected),
    sortValue: (c) => c.writes_rejected,
  },
  {
    key: "empty",
    header: "Empty reads",
    numeric: true,
    cell: (c) => count(c.empty_reads),
    sortValue: (c) => c.empty_reads,
  },
  {
    key: "poll",
    header: "Poll p50",
    numeric: true,
    cell: (c) => (c.poll_interval ? ns(c.poll_interval.p50) : "–"),
  },
  {
    key: "p",
    header: "Protocols",
    cell: (c) => <span className="text-xs text-fg-muted">{c.protocols.join(" · ")}</span>,
  },
];

const PROTOCOL_COLUMNS: Column<ProtocolStats>[] = [
  { key: "p", header: "Protocol", width: "6rem", cell: (p) => p.protocol },
  { key: "mo", header: "Msgs ↓", numeric: true, cell: (p) => count(p.messages_out) },
  { key: "mi", header: "Msgs ↑", numeric: true, cell: (p) => count(p.messages_in) },
  { key: "bo", header: "Bytes ↓", numeric: true, cell: (p) => bytes(p.bytes_out) },
  { key: "bi", header: "Bytes ↑", numeric: true, cell: (p) => bytes(p.bytes_in) },
  {
    key: "span",
    header: "Active span",
    numeric: true,
    cell: (p) => ns(p.last_ns - p.first_ns),
  },
  {
    key: "rate",
    header: "Mean rate",
    numeric: true,
    cell: (p) => {
      const span = p.last_ns - p.first_ns;
      return span > 0 ? bitRate(((p.bytes_out + p.bytes_in) * 8) / (span / 1e9)) : "–";
    },
  },
];

const OUTCOME: Record<SdoOutcome, { label: string; tone: "neutral" | "warning" | "defect" }> = {
  ok: { label: "ok", tone: "neutral" },
  aborted: { label: "aborted", tone: "warning" },
  superseded: { label: "superseded", tone: "neutral" },
  incomplete: { label: "incomplete", tone: "neutral" },
  protocol_error: { label: "protocol error", tone: "defect" },
};

const SDO_COLUMNS: Column<SdoTransaction>[] = [
  {
    key: "t",
    header: "Time",
    numeric: true,
    width: "7.5rem",
    cell: (t) => captureTime(t.start_ns),
    sortValue: (t) => t.start_ns,
  },
  {
    key: "st",
    header: "Station",
    width: "5.5rem",
    cell: (t) => <span className="font-mono">{hex(t.station)}</span>,
    sortValue: (t) => t.station,
  },
  { key: "k", header: "Kind", width: "6rem", cell: (t) => t.kind },
  {
    key: "obj",
    header: "Object",
    width: "7rem",
    cell: (t) => (
      <span className="font-mono text-xs">
        {hex(t.index)}:{t.subindex.toString(16).toUpperCase().padStart(2, "0")}
        {t.complete_access ? " CA" : ""}
      </span>
    ),
  },
  { key: "tr", header: "Transfer", width: "6.5rem", cell: (t) => t.transfer },
  {
    key: "b",
    header: "Bytes",
    numeric: true,
    cell: (t) => count(t.bytes),
    sortValue: (t) => t.bytes,
  },
  { key: "seg", header: "Segments", numeric: true, cell: (t) => count(t.segments) },
  {
    key: "d",
    header: "Duration",
    numeric: true,
    cell: (t) => (t.end_ns === null ? "–" : ns(t.end_ns - t.start_ns)),
    sortValue: (t) => (t.end_ns === null ? null : t.end_ns - t.start_ns),
  },
  {
    key: "o",
    header: "Outcome",
    cell: (t) => {
      const o = OUTCOME[t.outcome];
      const detail =
        t.abort_code !== null
          ? `${hex(t.abort_code, 8)} ${t.abort_text ?? ""} (${t.aborted_by ?? "?"})`
          : t.note;
      return (
        <span className="flex min-w-0 items-center gap-2">
          {t.outcome === "ok" ? (
            <span className="text-xs text-fg-muted">ok</span>
          ) : (
            <Badge tone={o.tone}>{o.label}</Badge>
          )}
          {detail ? <span className="truncate text-xs text-fg-muted">{detail}</span> : null}
        </span>
      );
    },
  },
];

const FOE_COLUMNS: Column<FoeTransfer>[] = [
  {
    key: "t",
    header: "Time",
    numeric: true,
    width: "7.5rem",
    cell: (t) => captureTime(t.start_ns),
  },
  {
    key: "st",
    header: "Station",
    width: "5.5rem",
    cell: (t) => <span className="font-mono">{hex(t.station)}</span>,
  },
  { key: "k", header: "Kind", width: "5rem", cell: (t) => t.kind },
  {
    key: "f",
    header: "File",
    cell: (t) => <span className="font-mono text-xs">{t.file_name}</span>,
  },
  { key: "b", header: "Bytes", numeric: true, cell: (t) => count(t.bytes) },
  { key: "p", header: "Packets", numeric: true, cell: (t) => count(t.packets) },
  { key: "busy", header: "Busy", numeric: true, cell: (t) => count(t.busy) },
  {
    key: "d",
    header: "Duration",
    numeric: true,
    cell: (t) => (t.end_ns === null ? "–" : ns(t.end_ns - t.start_ns)),
  },
  {
    key: "g",
    header: "Goodput",
    numeric: true,
    cell: (t) => (t.goodput_bps === null ? "–" : bitRate(t.goodput_bps)),
  },
  {
    key: "o",
    header: "Outcome",
    cell: (t) =>
      t.outcome === "ok" ? (
        <span className="text-xs text-fg-muted">ok</span>
      ) : (
        <span className="flex items-center gap-2">
          <Badge tone={t.outcome === "incomplete" ? "neutral" : "warning"}>
            {t.outcome.replace("_", " ")}
          </Badge>
          {t.error_text ? <span className="text-xs text-fg-muted">{t.error_text}</span> : null}
        </span>
      ),
  },
];

const EOE_COLUMNS: Column<EoeFrame>[] = [
  {
    key: "t",
    header: "Time",
    numeric: true,
    width: "7.5rem",
    cell: (f) => captureTime(f.start_ns),
  },
  {
    key: "st",
    header: "Station",
    width: "5.5rem",
    cell: (f) => <span className="font-mono">{hex(f.station)}</span>,
  },
  {
    key: "dir",
    header: "Direction",
    width: "7rem",
    cell: (f) => (f.direction === "to_slave" ? "to slave" : "from slave"),
  },
  { key: "b", header: "Bytes", numeric: true, cell: (f) => count(f.bytes) },
  { key: "fr", header: "Fragments", numeric: true, cell: (f) => f.fragments },
  { key: "d", header: "Duration", numeric: true, cell: (f) => ns(f.end_ns - f.start_ns) },
  {
    key: "eth",
    header: "Tunnelled frame",
    cell: (f) => (
      <span className="font-mono text-xs">
        {f.src ?? "?"} → {f.dst ?? "?"}
        {f.ethertype !== null ? ` · ${hex(f.ethertype)}` : ""}
      </span>
    ),
  },
];

const EMERGENCY_COLUMNS: Column<EmergencyEvent>[] = [
  { key: "t", header: "Time", numeric: true, width: "7.5rem", cell: (e) => captureTime(e.t_ns) },
  { key: "f", header: "Frame", numeric: true, width: "5rem", cell: (e) => e.frame },
  {
    key: "st",
    header: "Station",
    width: "5.5rem",
    cell: (e) => <span className="font-mono">{hex(e.station)}</span>,
  },
  {
    key: "c",
    header: "Error code",
    cell: (e) => <span className="font-mono">{hex(e.error_code)}</span>,
  },
  {
    key: "r",
    header: "Register",
    cell: (e) => <span className="font-mono">{hex(e.error_register, 2)}</span>,
  },
  { key: "d", header: "Data", cell: (e) => <span className="font-mono text-xs">{e.data}</span> },
];

const ERROR_COLUMNS: Column<MailboxErrorEvent>[] = [
  { key: "t", header: "Time", numeric: true, width: "7.5rem", cell: (e) => captureTime(e.t_ns) },
  { key: "f", header: "Frame", numeric: true, width: "5rem", cell: (e) => e.frame },
  {
    key: "st",
    header: "Station",
    width: "5.5rem",
    cell: (e) => <span className="font-mono">{hex(e.station)}</span>,
  },
  { key: "c", header: "Code", cell: (e) => <span className="font-mono">{hex(e.detail)}</span> },
  { key: "x", header: "Meaning", cell: (e) => e.text },
];

type Tab = "sdo" | "emergencies" | "foe" | "eoe" | "errors";

function plural(n: number, word: string): string {
  return `${count(n)} ${word}${n === 1 ? "" : "s"}`;
}

function windowed(n: number) {
  return n > 40 ? { rowHeight: 36, height: 520 } : undefined;
}

/** Mailbox channels, protocol traffic and the transfers followed through them. */
export function Mailbox({
  captureId,
  report,
  onOpenFrame,
}: {
  captureId: number;
  report: CaptureReport;
  onOpenFrame: (no: number) => void;
}) {
  const m = report.mailbox;
  const coe = m.coe;
  const counts: Record<Tab, number> = {
    sdo: Number(coe.transactions_total),
    emergencies: Number(coe.emergencies_total),
    foe: m.foe.length,
    eoe: Number(m.eoe.frames_total) + Number(m.eoe.reassembly_errors),
    errors: m.errors.length,
  };
  const first = (Object.keys(counts) as Tab[]).find((t) => counts[t] > 0) ?? "sdo";
  const [tab, setTab] = useState<Tab>(first);

  if (m.channels.length === 0) {
    return (
      <div className="p-6">
        <Empty>
          No mailbox traffic. No SyncManager was configured as a mailbox in this capture, and no
          FPWR/FPRD at 0x1000 or above carried a mailbox header. If the capture starts after
          configuration, give the addresses with{" "}
          <span className="font-mono">flowlab serve --mailbox STATION=OUT,IN</span>.
        </Empty>
      </div>
    );
  }

  const sum = (f: (c: MailboxChannel) => number) => m.channels.reduce((n, c) => n + f(c), 0);
  const msgsOut = sum((c) => c.messages_out);
  const msgsIn = sum((c) => c.messages_in);
  const goodputs = m.foe.flatMap((t) => (t.goodput_bps === null ? [] : [t.goodput_bps]));
  const heuristic = m.channels.some((c) => c.source === "heuristic");

  return (
    <div className="mx-auto flex max-w-[110rem] flex-col gap-5 p-6">
      <PageHeader
        title="Mailbox"
        meta={`${plural(m.channels.length, "station")} with a mailbox · ${m.protocols.map((p) => p.protocol).join(", ") || "no messages"}`}
        actions={
          m.eoe.frames_total > 0 ? (
            <ButtonLink
              href={api.eoeUrl(captureId)}
              download
              icon={<Download className="size-4" />}
            >
              EoE frames (pcapng)
            </ButtonLink>
          ) : null
        }
      />
      {heuristic ? (
        <Callout tone="info" title="Some mailboxes were inferred">
          The capture has no SyncManager configuration for stations marked “heuristic”; their
          mailbox addresses come from datagrams whose data parses as a mailbox header.
        </Callout>
      ) : null}

      <MetricGrid>
        <Metric
          label="Messages to slaves"
          value={count(msgsOut)}
          sub={`${bytes(sum((c) => c.bytes_out))} payload`}
          hint="Writes into a slave's mailbox that the slave accepted (WKC > 0)."
        />
        <Metric
          label="Messages from slaves"
          value={count(msgsIn)}
          sub={`${bytes(sum((c) => c.bytes_in))} payload`}
          hint="Reads of a slave's mailbox that returned a message (WKC > 0)."
        />
        <Metric
          label="SDO transfers"
          value={count(coe.transactions_total)}
          sub={`${count(coe.ok)} ok · ${count(coe.unfinished)} unfinished`}
        />
        <Metric
          label="SDO aborts"
          value={count(coe.aborted)}
          verdict={coe.aborted > 0 ? "warn" : "normal"}
          sub={
            coe.protocol_errors > 0 ? `${count(coe.protocol_errors)} protocol errors` : undefined
          }
          hint="Transfers ended by an abort from master or slave. Threshold: any."
        />
        <Metric
          label="SDO duration p50"
          value={coe.duration ? ns(coe.duration.p50) : "–"}
          sub={coe.duration ? `p99 ${ns(coe.duration.p99)}` : undefined}
          hint="From the initiate request to the final response of completed transfers, at the capture point."
        />
        <Metric
          label="Emergencies"
          value={count(coe.emergencies_total)}
          verdict={coe.emergencies_total > 0 ? "warn" : "normal"}
          hint="CoE emergency messages from slaves. Threshold: any."
        />
        <Metric
          label="FoE goodput"
          value={goodputs.length > 0 ? bitRate(Math.max(...goodputs)) : "–"}
          sub={plural(m.foe.length, "transfer")}
          hint="Data bytes of a completed FoE transfer over its duration (best transfer shown)."
        />
        <Metric
          label="EoE frames"
          value={count(m.eoe.frames_total)}
          verdict={m.eoe.reassembly_errors > 0 ? "warn" : undefined}
          sub={
            m.eoe.reassembly_errors > 0
              ? `${count(m.eoe.reassembly_errors)} not reassembled`
              : `${bytes(Number(m.eoe.bytes_to_slaves) + Number(m.eoe.bytes_from_slaves))}`
          }
          hint="Ethernet frames reassembled from EoE fragments."
        />
      </MetricGrid>

      <Panel title="Channels">
        <Table columns={CHANNEL_COLUMNS} rows={m.channels} rowKey={(c) => c.station} />
      </Panel>

      <div className="grid gap-5 xl:grid-cols-[minmax(0,7fr)_minmax(0,5fr)]">
        <Panel title="Traffic by protocol">
          <Table
            columns={PROTOCOL_COLUMNS}
            rows={m.protocols}
            rowKey={(p) => p.protocol}
            empty="No mailbox messages."
          />
        </Panel>
        <Panel title="SDO transfer duration">
          {coe.duration ? (
            <TimeHistogram
              d={coe.duration}
              label="Histogram of SDO transfer durations"
              xLabel="duration"
            />
          ) : (
            <Empty>No completed SDO transfer.</Empty>
          )}
        </Panel>
      </div>

      <Panel>
        <Tabs
          label="Mailbox transfers"
          active={tab}
          onSelect={setTab}
          items={[
            { id: "sdo", label: "CoE SDO", count: counts.sdo, disabled: counts.sdo === 0 },
            {
              id: "emergencies",
              label: "Emergencies",
              count: counts.emergencies,
              disabled: counts.emergencies === 0,
            },
            { id: "foe", label: "FoE", count: counts.foe, disabled: counts.foe === 0 },
            {
              id: "eoe",
              label: "EoE",
              count: Number(m.eoe.frames_total),
              disabled: counts.eoe === 0,
            },
            {
              id: "errors",
              label: "Mailbox errors",
              count: counts.errors,
              disabled: counts.errors === 0,
            },
          ]}
        />
        <div className="pt-3">
          {tab === "sdo" ? (
            <Table
              columns={SDO_COLUMNS}
              rows={coe.transactions}
              rowKey={(t) => `${t.first_frame}-${t.station}`}
              onRowClick={(t) => onOpenFrame(t.first_frame)}
              windowed={windowed(coe.transactions.length)}
              empty="No SDO transfers."
            />
          ) : tab === "emergencies" ? (
            <Table
              columns={EMERGENCY_COLUMNS}
              rows={coe.emergencies}
              rowKey={(e) => `${e.frame}-${e.station}`}
              onRowClick={(e) => onOpenFrame(e.frame)}
              windowed={windowed(coe.emergencies.length)}
            />
          ) : tab === "foe" ? (
            <Table
              columns={FOE_COLUMNS}
              rows={m.foe}
              rowKey={(t) => `${t.first_frame}-${t.station}`}
              onRowClick={(t) => onOpenFrame(t.first_frame)}
              windowed={windowed(m.foe.length)}
            />
          ) : tab === "eoe" ? (
            <div className="flex flex-col gap-3">
              {m.eoe.reassembly_errors > 0 ? (
                <Callout tone="warning" title="Incomplete EoE frames">
                  {count(m.eoe.reassembly_errors)} fragment sequences broke off or arrived out of
                  order. Threshold: any.
                </Callout>
              ) : null}
              <Table
                columns={EOE_COLUMNS}
                rows={m.eoe.frames}
                rowKey={(f) => `${f.first_frame}-${f.station}-${f.direction}`}
                onRowClick={(f) => onOpenFrame(f.first_frame)}
                windowed={windowed(m.eoe.frames.length)}
              />
            </div>
          ) : (
            <Table
              columns={ERROR_COLUMNS}
              rows={m.errors}
              rowKey={(e) => `${e.frame}-${e.station}`}
              onRowClick={(e) => onOpenFrame(e.frame)}
            />
          )}
        </div>
      </Panel>
    </div>
  );
}
