import { seriesColour } from "@vitavision/charts";
import { Badge, Button, cn, ErrorBox } from "@vitavision/ui";
import { ArrowRight } from "lucide-react";
import { useState, type ReactNode } from "react";

import type { DatagramDetail, FrameDetail, MailboxInfo } from "../api";
import { HexView } from "../components/HexView";
import { captureTime, hex, ns } from "../lib/format";

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <>
      <dt className="text-fg-muted">{label}</dt>
      <dd className="min-w-0 truncate font-mono text-fg">{children}</dd>
    </>
  );
}

const ROLE: Record<string, string> = {
  write: "write accepted",
  "write rejected": "write rejected: mailbox full",
  "write attempt": "write (as sent)",
  read: "read",
  "empty read": "read: mailbox empty",
  "read attempt": "read (as sent)",
  "status poll": "status poll",
};

function MailboxPanel({ m }: { m: MailboxInfo }) {
  const h = m.header;
  return (
    <div className="mt-2 rounded border border-line bg-raised p-2 text-xs">
      <div className="mb-1 flex flex-wrap items-center gap-2">
        <span className="font-medium text-fg">Mailbox{h ? ` · ${h.protocol}` : ""}</span>
        <span className="text-fg-muted">{ROLE[m.role] ?? m.role}</span>
        {m.source === "heuristic" ? (
          <span
            className="text-fg-subtle"
            title="The capture has no SyncManager configuration for this station; the mailbox address was inferred from the data layout."
          >
            (inferred address)
          </span>
        ) : m.source === "hint" ? (
          <span className="text-fg-subtle">(address from hint)</span>
        ) : null}
        {m.repeat ? <Badge tone="neutral">repeat</Badge> : null}
      </div>
      {m.summary ? <div className="mb-1 font-mono text-fg">{m.summary}</div> : null}
      {h ? (
        <dl className="grid grid-cols-[7rem_minmax(0,1fr)] gap-x-3 gap-y-1">
          <Field label="Header">
            {h.length} B · counter {h.counter} · channel {h.channel} · priority {h.priority}
          </Field>
          {m.fields.map((f) => (
            <Field key={f.name} label={f.name}>
              {f.value}
            </Field>
          ))}
        </dl>
      ) : null}
    </div>
  );
}

function DatagramCard({
  d,
  hovered,
  onHover,
}: {
  d: DatagramDetail;
  hovered: boolean;
  onHover: (p: number | null) => void;
}) {
  const a = d.address;
  const wkcOk = d.expected_wkc === null ? null : d.wkc === d.expected_wkc;
  return (
    <div
      onMouseEnter={() => onHover(d.position)}
      onMouseLeave={() => onHover(null)}
      className={cn(
        "rounded-md border bg-surface p-3 transition-colors",
        hovered ? "border-line-strong" : "border-line",
      )}
      style={{ borderLeft: `3px solid ${seriesColour(d.position)}` }}
    >
      <div className="mb-2 flex flex-wrap items-center gap-2">
        <span className="font-mono text-sm font-medium text-fg">
          #{d.position} {d.command}
        </span>
        <span className="font-mono text-xs text-fg-muted">idx {hex(d.index, 2)}</span>
        {wkcOk === null ? null : wkcOk ? (
          <Badge tone="normal">WKC {d.wkc} = expected</Badge>
        ) : (
          <Badge tone="defect">
            WKC {d.wkc} ≠ {d.expected_wkc}
          </Badge>
        )}
        {d.circulating ? <Badge tone="warning">circulating</Badge> : null}
      </div>
      <dl className="grid grid-cols-[7rem_minmax(0,1fr)] gap-x-3 gap-y-1 text-xs">
        <Field label="Address">{a.text}</Field>
        {a.register ? (
          <Field label="Register">
            {a.register} <span className="text-fg-subtle">· {a.register_area}</span>
          </Field>
        ) : null}
        <Field label="Length">{d.len} B</Field>
        <Field label="WKC">{d.wkc}</Field>
        {d.irq !== 0 ? <Field label="IRQ">{hex(d.irq)}</Field> : null}
        {d.al_state ? <Field label="AL state">{d.al_state}</Field> : null}
      </dl>
      {d.mailbox ? <MailboxPanel m={d.mailbox} /> : null}
    </div>
  );
}

/** Everything about one frame: header facts, decoded datagrams and the bytes. */
export function FrameDetailView({
  detail,
  onOpenFrame,
}: {
  detail: FrameDetail;
  onOpenFrame: (no: number) => void;
}) {
  const [hover, setHover] = useState<number | null>(null);
  const r = detail.row;
  return (
    <div className="flex flex-col gap-4 p-4">
      <div className="flex flex-wrap items-center gap-2">
        <h2 className="text-base font-medium text-fg">Frame {r.no}</h2>
        <Badge tone={r.direction === "unknown" ? "unlabeled" : "neutral"}>
          {r.direction.toUpperCase()}
        </Badge>
        {r.stream !== null ? <Badge tone="neutral">stream {r.stream}</Badge> : null}
        {r.unanswered ? <Badge tone="defect">no return</Badge> : null}
        {r.wkc_mismatch ? <Badge tone="defect">WKC mismatch</Badge> : null}
        {r.paired !== null ? (
          <Button
            variant="ghost"
            size="sm"
            icon={<ArrowRight className="size-3.5" />}
            onClick={() => onOpenFrame(r.paired ?? 0)}
          >
            {r.direction === "tx" ? "Reply" : "Request"}: frame {r.paired}
          </Button>
        ) : null}
      </div>
      {detail.error ? <ErrorBox>{detail.error}</ErrorBox> : null}
      <dl className="grid grid-cols-[7rem_minmax(0,1fr)] gap-x-3 gap-y-1 text-xs">
        <Field label="Time">{captureTime(r.t_ns)}</Field>
        {r.rtt_ns !== null ? <Field label="Round trip">{ns(r.rtt_ns)}</Field> : null}
        <Field label="Length">
          {r.len} B captured · {detail.wire_bytes} B on the wire
        </Field>
        {detail.src ? (
          <Field label="Ethernet">
            {detail.src} → {detail.dst}
            {detail.vlan !== null ? ` · VLAN ${detail.vlan & 0xfff}` : ""}
          </Field>
        ) : null}
        {detail.encapsulation ? (
          <Field label="EtherCAT">
            {detail.encapsulation === "udp" ? "UDP/IP" : "EtherType 0x88A4"} · length{" "}
            {detail.ecat_length} · type {detail.ecat_type}
          </Field>
        ) : null}
      </dl>
      {detail.datagrams.length > 0 ? (
        <div className="flex flex-col gap-2">
          {detail.datagrams.map((d) => (
            <DatagramCard
              key={d.position}
              d={d}
              hovered={hover === d.position}
              onHover={setHover}
            />
          ))}
        </div>
      ) : null}
      <HexView
        hex={detail.hex}
        ranges={detail.ranges}
        emphasis={hover}
        onHoverDatagram={setHover}
      />
    </div>
  );
}
