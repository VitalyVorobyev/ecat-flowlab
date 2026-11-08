/** Typed client for the `flowlab serve` API. Types are generated from Rust (ts-rs). */
import type { CaptureInfo } from "./gen/CaptureInfo";
import type { CaptureReport } from "./gen/CaptureReport";
import type { FrameDetail } from "./gen/FrameDetail";
import type { FrameFilter } from "./gen/FrameFilter";
import type { FramePage } from "./gen/FramePage";
import type { EsiImport } from "./gen/EsiImport";
import type { PlanResult } from "./gen/PlanResult";
import type { SimPreset } from "./gen/SimPreset";
import type { SimResult } from "./gen/SimResult";
import type { SimSpec } from "./gen/SimSpec";
import type { WhatIfResult } from "./gen/WhatIfResult";
import type { StudyResult } from "./gen/StudyResult";
import type { LiveInterfaces } from "./gen/LiveInterfaces";
import type { LiveSnapshot } from "./gen/LiveSnapshot";
import type { LiveStart } from "./gen/LiveStart";
import type { LiveStopped } from "./gen/LiveStopped";

export type { CaptureInfo } from "./gen/CaptureInfo";
export type { CaptureReport } from "./gen/CaptureReport";
export type { CyclicStream } from "./gen/CyclicStream";
export type { DatagramDetail } from "./gen/DatagramDetail";
export type { Distribution } from "./gen/Distribution";
export type { Finding } from "./gen/Finding";
export type { FrameDetail } from "./gen/FrameDetail";
export type { FrameFilter } from "./gen/FrameFilter";
export type { FramePage } from "./gen/FramePage";
export type { FrameRow } from "./gen/FrameRow";
export type { Provenance } from "./gen/Provenance";
export type { Severity } from "./gen/Severity";
export type { StationSummary } from "./gen/StationSummary";
export type { AlEvent } from "./gen/AlEvent";
export type { ByteRange } from "./gen/ByteRange";
export type { Point } from "./gen/Point";
export type { Direction } from "./gen/Direction";
export type { DirectionMode } from "./gen/DirectionMode";
export type { ChannelSource } from "./gen/ChannelSource";
export type { CoeReport } from "./gen/CoeReport";
export type { EmergencyEvent } from "./gen/EmergencyEvent";
export type { EoeFrame } from "./gen/EoeFrame";
export type { FoeTransfer } from "./gen/FoeTransfer";
export type { MailboxChannel } from "./gen/MailboxChannel";
export type { MailboxService } from "./gen/MailboxService";
export type { ServicePattern } from "./gen/ServicePattern";
export type { MailboxErrorEvent } from "./gen/MailboxErrorEvent";
export type { MailboxInfo } from "./gen/MailboxInfo";
export type { MailboxReport } from "./gen/MailboxReport";
export type { ProtocolStats } from "./gen/ProtocolStats";
export type { SdoOutcome } from "./gen/SdoOutcome";
export type { SdoTransaction } from "./gen/SdoTransaction";
export type { SimResult } from "./gen/SimResult";
export type { SlaveResult } from "./gen/SlaveResult";
export type { TransferResult } from "./gen/TransferResult";
export type { CaptureSpec } from "./gen/CaptureSpec";
export type { Lever } from "./gen/Lever";
export type { MailboxAccess } from "./gen/MailboxAccess";
export type { MailboxPolicy } from "./gen/MailboxPolicy";
export type { PdiSpec } from "./gen/PdiSpec";
export type { SimPreset } from "./gen/SimPreset";
export type { SimSlave } from "./gen/SimSlave";
export type { SimSpec } from "./gen/SimSpec";
export type { SpecEdit } from "./gen/SpecEdit";
export type { TransferDirection } from "./gen/TransferDirection";
export type { TransferProtocol } from "./gen/TransferProtocol";
export type { TransferSpec } from "./gen/TransferSpec";
export type { WhatIf } from "./gen/WhatIf";
export type { WhatIfResult } from "./gen/WhatIfResult";
export type { WhatIfRun } from "./gen/WhatIfRun";
export type { DeviceDescription } from "./gen/DeviceDescription";
export type { EsiImport } from "./gen/EsiImport";
export type { PlanCheck } from "./gen/PlanCheck";
export type { PlannedFrame } from "./gen/PlannedFrame";
export type { PlanResult } from "./gen/PlanResult";
export type { PlanSlave } from "./gen/PlanSlave";
export type { Residual } from "./gen/Residual";
export type { StudyResult } from "./gen/StudyResult";
export type { StudyRun } from "./gen/StudyRun";
export type { CaptureInterface } from "./gen/CaptureInterface";
export type { LiveInterfaces } from "./gen/LiveInterfaces";
export type { LivePoint } from "./gen/LivePoint";
export type { LiveSnapshot } from "./gen/LiveSnapshot";
export type { LiveSourceSpec } from "./gen/LiveSourceSpec";
export type { LiveStart } from "./gen/LiveStart";
export type { LiveState } from "./gen/LiveState";
export type { LiveStopped } from "./gen/LiveStopped";
export type { LiveTotals } from "./gen/LiveTotals";
export type { Tool } from "./gen/Tool";

/** An error the server answered with `{ "error": "..." }`, or a transport failure. */
export class ApiError extends Error {
  readonly status: number;
  constructor(status: number, message: string) {
    super(message);
    this.name = "ApiError";
    this.status = status;
  }
}

export function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  let res: Response;
  try {
    res = await fetch(path, init);
  } catch (error) {
    throw new ApiError(
      0,
      `Cannot reach the FlowLab server (${errorMessage(error)}). Is \`flowlab serve\` running?`,
    );
  }
  if (!res.ok) {
    let message = `${res.status} ${res.statusText}`;
    try {
      const body = (await res.json()) as { error?: string };
      if (body.error) message = body.error;
    } catch {
      // Not JSON; keep the status line.
    }
    throw new ApiError(res.status, message);
  }
  if (res.status === 204) return undefined as T;
  return (await res.json()) as T;
}

function json(method: string, body: unknown): RequestInit {
  return { method, headers: { "content-type": "application/json" }, body: JSON.stringify(body) };
}

/** Query string for a frame filter, leaving out unset fields. */
export function frameQuery(filter: FrameFilter, offset: number, limit: number): string {
  const q = new URLSearchParams({ offset: String(offset), limit: String(limit) });
  if (filter.command) q.set("command", filter.command);
  if (filter.direction) q.set("direction", filter.direction);
  if (filter.station !== null) q.set("station", String(filter.station));
  if (filter.stream !== null) q.set("stream", String(filter.stream));
  if (filter.problems_only) q.set("problems_only", "true");
  if (filter.include_other) q.set("include_other", "true");
  if (filter.mailbox) q.set("mailbox", filter.mailbox);
  return q.toString();
}

export const NO_FILTER: FrameFilter = {
  command: null,
  direction: null,
  station: null,
  stream: null,
  problems_only: false,
  include_other: false,
  mailbox: null,
};

/** Mailbox protocol filters the server understands (`FrameFilter.mailbox`). */
export const MAILBOX_FILTERS = [
  { value: "any", label: "Any mailbox access" },
  { value: "coe", label: "CoE" },
  { value: "foe", label: "FoE" },
  { value: "eoe", label: "EoE" },
  { value: "soe", label: "SoE" },
  { value: "voe", label: "VoE" },
  { value: "aoe", label: "AoE" },
  { value: "error", label: "Mailbox error" },
] as const;

export const api = {
  health: () => request<{ version: string }>("/api/health"),
  listCaptures: () => request<CaptureInfo[]>("/api/captures"),
  upload: (file: File) =>
    request<CaptureInfo>(`/api/captures?name=${encodeURIComponent(file.name)}`, {
      method: "POST",
      headers: { "content-type": "application/octet-stream" },
      body: file,
    }),
  openPath: (path: string) => request<CaptureInfo>("/api/captures/open", json("POST", { path })),
  remove: (id: number) => request<void>(`/api/captures/${id}`, { method: "DELETE" }),
  report: (id: number) => request<CaptureReport>(`/api/captures/${id}/report`),
  frames: (id: number, filter: FrameFilter, offset: number, limit: number) =>
    request<FramePage>(`/api/captures/${id}/frames?${frameQuery(filter, offset, limit)}`),
  frame: (id: number, no: number) => request<FrameDetail>(`/api/captures/${id}/frames/${no}`),
  /** URL of the Ethernet frames reassembled from EoE, as a pcapng download. */
  eoeUrl: (id: number) => `/api/captures/${id}/eoe.pcapng`,
  synthSchema: () => request<Record<string, unknown>>("/api/synth/schema"),
  synthExample: () => request<Record<string, unknown>>("/api/synth/example"),
  synth: (spec: unknown) => request<CaptureInfo>("/api/synth", json("POST", spec)),
  simSchema: () => request<Record<string, unknown>>("/api/sim/schema"),
  simExample: () => request<Record<string, unknown>>("/api/sim/example"),
  simulate: (spec: unknown) => request<CaptureInfo>("/api/simulate", json("POST", spec)),
  simulation: (id: number) => request<SimResult>(`/api/captures/${id}/simulation`),
  simPresets: () => request<SimPreset[]>("/api/sim/presets"),
  /** Simulate without keeping a capture. */
  simRun: (spec: SimSpec, signal?: AbortSignal) =>
    request<SimResult>("/api/sim/run", { ...json("POST", spec), signal: signal ?? null }),
  /** The scenario with one change at a time, best first. */
  whatIf: (spec: SimSpec, signal?: AbortSignal) =>
    request<WhatIfResult>("/api/sim/what-if", { ...json("POST", spec), signal: signal ?? null }),
  attachEni: (id: number, file: File) =>
    request<CaptureInfo>(`/api/captures/${id}/eni`, {
      method: "POST",
      headers: { "content-type": "application/xml" },
      body: file,
    }),
  detachEni: (id: number) => request<CaptureInfo>(`/api/captures/${id}/eni`, { method: "DELETE" }),
  importEsi: (file: File) =>
    request<EsiImport>("/api/esi", {
      method: "POST",
      headers: { "content-type": "application/xml" },
      body: file,
    }),
  planSchema: () => request<Record<string, unknown>>("/api/plan/schema"),
  planExample: () => request<Record<string, unknown>>("/api/plan/example"),
  plan: (spec: unknown) => request<PlanResult>("/api/plan", json("POST", spec)),
  studySchema: () => request<Record<string, unknown>>("/api/study/schema"),
  studyExample: () => request<Record<string, unknown>>("/api/study/example"),
  study: (spec: unknown, captureId: number | null) =>
    request<StudyResult>(
      captureId === null ? "/api/study" : `/api/study?capture=${captureId}`,
      json("POST", spec),
    ),
  liveInterfaces: () => request<LiveInterfaces>("/api/live/interfaces"),
  liveStart: (start: LiveStart) => request<LiveSnapshot>("/api/live/start", json("POST", start)),
  /** The running or last live session; null when there has been none. */
  live: () => request<LiveSnapshot | null>("/api/live"),
  liveStop: () => request<LiveStopped>("/api/live/stop", { method: "POST" }),
};
