/** Reading and editing a simulator scenario (`SimSpec`) in the units people think in. */

import type {
  MailboxPolicy,
  SimSlave,
  SimSpec,
  SpecEdit,
  TransferProtocol,
  TransferSpec,
} from "../api";
import { bitRate, bytes, ns } from "../lib/format";

export type ProtocolKind = TransferProtocol["kind"];
export type PolicyKind = MailboxPolicy["kind"];

/** Protocols the simulator models, plus the ones it does not (shown, not choosable). */
export const PROTOCOL_OPTIONS: {
  value: string;
  label: string;
  note?: string;
  disabled?: boolean;
}[] = [
  { value: "eoe", label: "EoE", note: "Ethernet over EtherCAT, UDP/IPv4" },
  { value: "foe", label: "FoE", note: "File access over EtherCAT" },
  { value: "coe_segmented", label: "CoE SDO", note: "segmented download / upload" },
  { value: "pdo_channel", label: "PDO channel", note: "chunks in cyclic process data" },
  { value: "tcp_eoe", label: "TCP over EoE", note: "not in the model", disabled: true },
  { value: "voe", label: "VoE", note: "not in the model", disabled: true },
];

export const POLICY_OPTIONS: { value: PolicyKind; label: string; hint: string }[] = [
  {
    value: "fixed_quota",
    label: "Fixed quota",
    hint: "Up to N mailbox datagrams per cycle, shared round-robin, in one frame after the cyclic frame.",
  },
  {
    value: "round_robin",
    label: "Round robin",
    hint: "Each cycle, N slaves with mailbox work get one write and one read.",
  },
  {
    value: "opportunistic",
    label: "Opportunistic",
    hint: "Mailbox frames back to back while the cycle has spare time, up to a share of the cycle.",
  },
];

export function defaultProtocol(kind: ProtocolKind): TransferProtocol {
  switch (kind) {
    case "eoe":
      return { kind, mtu_bytes: 1500 };
    case "foe":
      return { kind };
    case "coe_segmented":
      return { kind };
    case "pdo_channel":
      return { kind, bytes_per_cycle: 256, window: 2, retransmit_after_cycles: 8 };
  }
}

export function defaultPolicy(kind: PolicyKind): MailboxPolicy {
  switch (kind) {
    case "fixed_quota":
      return { kind, accesses_per_cycle: 2 };
    case "round_robin":
      return { kind, slaves_per_cycle: 1 };
    case "opportunistic":
      return { kind, bus_budget_percent: 60, turnaround_ns: 10_000 };
  }
}

export function protocolName(p: TransferProtocol): string {
  switch (p.kind) {
    case "eoe":
      return "EoE";
    case "foe":
      return "FoE";
    case "coe_segmented":
      return "CoE";
    case "pdo_channel":
      return "PDO channel";
  }
}

/** The protocol with its main parameter: `EoE, MTU 1500 B`, `PDO channel, 256 B/cycle`. */
export function protocolDetail(p: TransferProtocol): string {
  switch (p.kind) {
    case "eoe":
      return `EoE, MTU ${p.mtu_bytes} B`;
    case "pdo_channel":
      return `PDO channel, ${p.bytes_per_cycle} B/cycle`;
    default:
      return protocolName(p);
  }
}

export function arrow(t: TransferSpec): string {
  return t.direction === "master_to_slave" ? "↓" : "↑";
}

export function directionText(t: TransferSpec): string {
  return t.direction === "master_to_slave" ? "master → slave" : "slave → master";
}

/** One line for a transfer: `EoE ↓ 10.00 MB to bulk-endpoint`. */
export function transferLine(t: TransferSpec, spec: SimSpec): string {
  const slave = spec.slaves[t.slave]?.name ?? `slave ${t.slave}`;
  const where = t.direction === "master_to_slave" ? "to" : "from";
  return `${protocolName(t.protocol)} ${arrow(t)} ${payload(t.payload_bytes)} ${where} ${slave}`;
}

/** A payload in the unit it was most likely entered in: 10 MB, 64 KiB, 1500 B. */
export function payload(n: number): string {
  const unit = payloadUnit(n);
  const v = n / unit.factor;
  return `${Number.isInteger(v) ? v : v.toFixed(2)} ${unit.label}`;
}

export const PAYLOAD_UNITS = [
  { label: "B", factor: 1 },
  { label: "kB", factor: 1e3 },
  { label: "MB", factor: 1e6 },
  { label: "KiB", factor: 1024 },
  { label: "MiB", factor: 1024 * 1024 },
] as const;

export type PayloadUnit = (typeof PAYLOAD_UNITS)[number];

/** The largest unit the value is a whole multiple of, decimal first. */
export function payloadUnit(n: number): PayloadUnit {
  const [b, kb, mb, kib, mib] = PAYLOAD_UNITS;
  return [mb, mib, kb, kib].find((u) => n >= u.factor && n % u.factor === 0) ?? b;
}

/** One line for a slave's bulk-transfer resources. */
export function slaveLine(s: SimSlave): string {
  const parts: string[] = [];
  if (s.outputs_bytes || s.inputs_bytes) parts.push(`PD ${s.outputs_bytes}/${s.inputs_bytes} B`);
  parts.push(
    s.mailbox_out_bytes > 0 ? `MBX ${s.mailbox_out_bytes}/${s.mailbox_in_bytes} B` : "no mailbox",
  );
  if (s.mailbox_out_bytes > 0 || s.outputs_bytes > 0) {
    const r = s.pdi.read_bps / 1e6;
    const w = s.pdi.write_bps / 1e6;
    parts.push(`PDI ${r === w ? r : `${r}/${w}`} Mbit/s`);
  }
  return parts.join(" · ");
}

export function newSlave(index: number): SimSlave {
  return {
    name: `slave-${index}`,
    outputs_bytes: 0,
    inputs_bytes: 0,
    forwarding_delay_ns: 700,
    mailbox_out_bytes: 0,
    mailbox_in_bytes: 0,
    dpram_bytes: 4096,
    pdi: {
      read_bps: 20_000_000,
      write_bps: 20_000_000,
      access_overhead_ns: 5_000,
      mailbox_access: "whole_buffer",
    },
    app_processing_ns: 20_000,
  };
}

export function newTransfer(spec: SimSpec): TransferSpec {
  const withMailbox = spec.slaves.findIndex((s) => s.mailbox_out_bytes > 0);
  return {
    slave: withMailbox >= 0 ? withMailbox : Math.max(0, spec.slaves.length - 1),
    direction: "master_to_slave",
    payload_bytes: 1_000_000,
    protocol: defaultProtocol(withMailbox >= 0 ? "eoe" : "pdo_channel"),
    start_ms: 0,
  };
}

/** Remove slave `i`, its transfers, and renumber the transfers of the slaves after it. */
export function removeSlave(spec: SimSpec, i: number): SimSpec {
  return {
    ...spec,
    slaves: spec.slaves.filter((_, k) => k !== i),
    transfers: spec.transfers
      .filter((t) => t.slave !== i)
      .map((t) => (t.slave > i ? { ...t, slave: t.slave - 1 } : t)),
  };
}

function unescape(token: string): string {
  return token.replaceAll("~1", "/").replaceAll("~0", "~");
}

/** `spec` with each edit's value set at its JSON pointer. Unknown paths are skipped. */
export function applyEdits(spec: SimSpec, edits: readonly SpecEdit[]): SimSpec {
  const copy = structuredClone(spec) as unknown as Record<string, unknown>;
  for (const e of edits) {
    const keys = e.path.split("/").slice(1).map(unescape);
    const last = keys.pop();
    if (last === undefined) continue;
    let node: unknown = copy;
    for (const k of keys) node = (node as Record<string, unknown> | undefined)?.[k];
    if (node && typeof node === "object" && last in node) {
      (node as Record<string, unknown>)[last] = structuredClone(e.value);
    }
  }
  return copy as unknown as SimSpec;
}

export interface SpecChange {
  path: string;
  from: string;
  to: string;
}

function show(v: unknown): string {
  if (v === undefined) return "–";
  return typeof v === "string" ? v : JSON.stringify(v);
}

/** Leaf differences between two specs, as JSON pointers. Arrays of different length differ as a whole. */
export function diffSpecs(a: unknown, b: unknown, path = ""): SpecChange[] {
  if (Object.is(a, b)) return [];
  const isObj = (v: unknown): v is Record<string, unknown> =>
    v !== null && typeof v === "object" && !Array.isArray(v);
  if (Array.isArray(a) && Array.isArray(b)) {
    if (a.length !== b.length) {
      return [{ path, from: `${a.length} entries`, to: `${b.length} entries` }];
    }
    return a.flatMap((x, i) => diffSpecs(x, b[i], `${path}/${i}`));
  }
  if (isObj(a) && isObj(b)) {
    if (a.kind !== b.kind && ("kind" in a || "kind" in b)) {
      return [{ path, from: show(a.kind), to: show(b.kind) }];
    }
    const keys = [...new Set([...Object.keys(a), ...Object.keys(b)])];
    return keys.flatMap((k) => diffSpecs(a[k], b[k], `${path}/${k}`));
  }
  return [{ path, from: show(a), to: show(b) }];
}

/** A pointer in words: `/slaves/1/pdi/read_bps` → `bulk-endpoint: PDI read rate`. */
export function pathLabel(path: string, spec: SimSpec): string {
  const k = path.split("/").slice(1);
  const field = (name: string | undefined): string =>
    ({
      cycle_time_us: "cycle time",
      link_bps: "link rate",
      master_jitter_ns: "master jitter",
      mailbox_policy: "mailbox policy",
      accesses_per_cycle: "mailbox datagrams per cycle",
      slaves_per_cycle: "slaves served per cycle",
      bus_budget_percent: "mailbox share of the cycle",
      turnaround_ns: "master turnaround",
      read_bps: "PDI read rate",
      write_bps: "PDI write rate",
      access_overhead_ns: "PDI access overhead",
      mailbox_access: "mailbox access",
      mailbox_out_bytes: "mailbox out",
      mailbox_in_bytes: "mailbox in",
      outputs_bytes: "outputs",
      inputs_bytes: "inputs",
      dpram_bytes: "ESC RAM",
      app_processing_ns: "application processing",
      forwarding_delay_ns: "forwarding delay",
      payload_bytes: "payload",
      protocol: "protocol",
      mtu_bytes: "MTU",
      bytes_per_cycle: "bytes per cycle",
      window: "window",
      retransmit_after_cycles: "resend after",
      direction: "direction",
      start_ms: "start",
      slave: "slave",
      seed: "seed",
      max_duration_ms: "time limit",
      frame_loss_probability: "frame loss probability",
      description: "description",
    })[name ?? ""] ??
    name ??
    "";
  if (k[0] === "slaves" && k[1] !== undefined) {
    const s = spec.slaves[Number(k[1])];
    return `${s?.name ?? `slave ${k[1]}`}: ${field(k.at(-1))}`;
  }
  if (k[0] === "transfers" && k[1] !== undefined) {
    const kind = k.length === 2 ? "transfers" : field(k.at(-1));
    return `transfer ${k[1]}: ${kind}`;
  }
  return field(k.at(-1)) || path;
}

/** A value at `path` in its unit: `40 Mbit/s`, `250 µs`, `1024 B`. */
export function valueText(path: string, raw: string): string {
  const n = Number(raw);
  if (raw === "" || !Number.isFinite(n)) return raw.replaceAll("_", " ");
  const key = path.split("/").at(-1) ?? "";
  if (key.endsWith("_bps")) return bitRate(n);
  if (key.endsWith("_ns")) return ns(n);
  if (key.endsWith("_us")) return ns(n * 1e3);
  if (key.endsWith("_ms")) return ns(n * 1e6);
  if (key === "payload_bytes") return payload(n);
  if (key.endsWith("_bytes") || key === "bytes_per_cycle") return `${n} B`;
  if (key.endsWith("_percent")) return `${n} %`;
  if (key === "frame_loss_probability") return `${100 * n} %`;
  return raw;
}

/** A change in words: `bulk-endpoint: PDI read rate 20.00 Mbit/s → 40.00 Mbit/s`. */
export function changeText(c: SpecChange, spec: SimSpec): string {
  return `${pathLabel(c.path, spec)} ${valueText(c.path, c.from)} → ${valueText(c.path, c.to)}`;
}

/** All transfers' payload together. */
export function totalPayload(spec: SimSpec): string {
  return bytes(spec.transfers.reduce((n, t) => n + t.payload_bytes, 0));
}
