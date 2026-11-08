import { LineChart, type Series } from "@vitavision/charts";
import {
  Badge,
  Button,
  Callout,
  Disclosure,
  Field,
  NumberInput,
  PageHeader,
  Panel,
  ReadoutStrip,
  SegmentedControl,
  Select,
  Table,
  toast,
} from "@vitavision/ui";
import { Circle, Play, Square } from "lucide-react";
import { type ReactNode, useCallback, useEffect, useState } from "react";

import {
  api,
  type CaptureInfo,
  errorMessage,
  type LivePoint,
  type LiveSnapshot,
  type LiveSourceSpec,
} from "../api";
import { Findings } from "../components/Findings";
import { ProvenanceBadge } from "../components/ProvenanceBadge";
import { count, ns } from "../lib/format";
import { useAsync } from "../lib/useAsync";
import { STATION_COLUMNS } from "./Stations";

const POLL_MS = 1000;
/** The station columns that matter while watching: who answers, and in which state. */
const LIVE_STATION_COLUMNS = STATION_COLUMNS.filter((c) =>
  ["st", "name", "dg", "nr", "req", "rep"].includes(c.key),
);
const SPEEDS = ["1", "2", "5", "10"];

type Kind = "interface" | "replay";

/** One line per series, skipping ticks without a value. */
function series(
  points: LivePoint[],
  lines: { name: string; pick: (p: LivePoint) => number | null }[],
) {
  return lines.map<Series>((l) => ({
    name: l.name,
    points: points.flatMap((p) => {
      const v = l.pick(p);
      return v === null ? [] : [{ x: p.t_s, y: v }];
    }),
  }));
}

function LiveCharts({ points }: { points: LivePoint[] }) {
  const timing = series(points, [
    { name: "jitter p99", pick: (p) => (p.jitter_p99_ns === null ? null : p.jitter_p99_ns / 1e3) },
    {
      name: "round trip p99",
      pick: (p) => (p.round_trip_p99_ns === null ? null : p.round_trip_p99_ns / 1e3),
    },
  ]);
  const events = series(points, [
    { name: "WKC mismatches", pick: (p) => p.wkc_mismatches },
    { name: "unanswered", pick: (p) => p.unanswered },
    { name: "missed cycles", pick: (p) => p.missed_cycles },
  ]);
  const maxEvents = Math.max(1, ...events.flatMap((s) => s.points.map((p) => p.y)));
  return (
    <div className="grid gap-4 lg:grid-cols-2">
      <Panel title="Main cyclic stream, per second">
        <LineChart
          label="Phase jitter and round trip, 99th percentile per second"
          series={timing}
          xLabel="time since start (s)"
          yLabel="µs"
          variant="fluid"
          height={200}
          showLegend
        />
      </Panel>
      <Panel title="Problems per second">
        <LineChart
          label="WKC mismatches, unanswered frames and missed cycles per second"
          series={events}
          xLabel="time since start (s)"
          yLabel="count"
          yDomain={[0, maxEvents * 1.1]}
          variant="fluid"
          height={200}
          showLegend
        />
      </Panel>
    </div>
  );
}

function Readouts({ snap }: { snap: LiveSnapshot }) {
  const last = snap.series[snap.series.length - 1];
  const opt = (v: number | null | undefined) => (v === null || v === undefined ? null : ns(v));
  const t = snap.totals;
  return (
    <ReadoutStrip
      items={[
        { label: "frames last s", value: last ? count(last.frames) : null },
        { label: "cycle", value: opt(last?.period_ns) },
        { label: "jitter p99", value: opt(last?.jitter_p99_ns) },
        { label: "round trip p99", value: opt(last?.round_trip_p99_ns) },
        { label: "WKC mismatches", value: count(t.wkc_mismatches) },
        { label: "unanswered", value: count(t.unanswered) },
        { label: "missed cycles", value: count(t.missed_cycles) },
        { label: "mailbox messages", value: count(t.mailbox_messages) },
      ]}
    />
  );
}

/**
 * Live capture: the traffic on an interface (through dumpcap or tcpdump), or a loaded capture
 * replayed at its recorded pace, analysed every second.
 */
export function Live({
  captures,
  onSaved,
  onOpen,
}: {
  captures: CaptureInfo[];
  /** A finished interface capture was saved and loaded. */
  onSaved: (info: CaptureInfo) => void;
  /** Open a loaded capture's analysis. */
  onOpen: (id: number) => void;
}) {
  const ifaces = useAsync(() => api.liveInterfaces(), []);
  const [snap, setSnap] = useState<LiveSnapshot | null>(null);
  const [saved, setSaved] = useState<CaptureInfo | null>(null);
  const [kind, setKind] = useState<Kind | null>(null);
  const [iface, setIface] = useState("");
  const [replayId, setReplayId] = useState("");
  const [speed, setSpeed] = useState("1");
  const [windowS, setWindowS] = useState(10);
  const [busy, setBusy] = useState(false);
  const running = snap?.state === "running";
  const enabled = ifaces.data?.enabled ?? false;
  const source: Kind = kind ?? (enabled ? "interface" : "replay");

  const poll = useCallback(() => {
    api.live().then(setSnap, () => undefined);
  }, []);
  useEffect(poll, [poll]);
  useEffect(() => {
    if (!running) return undefined;
    const t = window.setInterval(poll, POLL_MS);
    return () => window.clearInterval(t);
  }, [running, poll]);

  const start = async () => {
    let spec: LiveSourceSpec;
    if (source === "interface") {
      spec = { kind: "interface", interface: iface, tool: null, all_traffic: false };
    } else {
      spec = { kind: "replay", capture: Number(replayId), speed: Number(speed) };
    }
    setBusy(true);
    try {
      setSaved(null);
      setSnap(await api.liveStart({ source: spec, window_s: windowS }));
    } catch (e) {
      toast({ tone: "error", title: "Could not start", description: errorMessage(e) });
    } finally {
      setBusy(false);
    }
  };

  const stop = async () => {
    setBusy(true);
    try {
      const r = await api.liveStop();
      setSnap(r.snapshot);
      if (r.capture) {
        setSaved(r.capture);
        onSaved(r.capture);
      }
    } catch (e) {
      toast({ tone: "error", title: "Could not stop", description: errorMessage(e) });
    } finally {
      setBusy(false);
    }
  };

  const ready = source === "interface" ? enabled && iface !== "" : replayId !== "";
  const interfaceNote: ReactNode = !ifaces.data ? null : !enabled ? (
    <>
      Capturing from interfaces is off. Start the server with <code>flowlab serve --live</code> (or{" "}
      <code>cargo ui -- --live</code>).
    </>
  ) : (
    ifaces.data.error
  );

  return (
    <div className="mx-auto flex max-w-[110rem] flex-col gap-4 p-5">
      <PageHeader
        title="Live"
        meta={
          snap ? (
            <span className="flex flex-wrap items-center gap-x-3 gap-y-1">
              <Badge tone={snap.state === "failed" ? "defect" : "neutral"}>
                {running ? (
                  <Circle className="mr-1 inline size-2.5 fill-current" aria-hidden="true" />
                ) : null}
                {snap.state}
              </Badge>
              {snap.provenance ? <ProvenanceBadge provenance={snap.provenance} /> : null}
              <span className="text-xs">{snap.source}</span>
              <span className="font-mono text-xs">
                {snap.elapsed_s.toFixed(0)} s · {count(snap.packets)} packets
              </span>
              {snap.saved_to ? (
                <span className="font-mono text-xs">saving to {snap.saved_to}</span>
              ) : null}
            </span>
          ) : (
            "Watch EtherCAT traffic as it happens: cycle timing, working counters, lost frames and mailbox traffic, analysed every second."
          )
        }
        actions={
          running ? (
            <Button
              variant="primary"
              icon={<Square className="size-4" />}
              onClick={() => void stop()}
              disabled={busy}
            >
              Stop
            </Button>
          ) : saved ? (
            <Button variant="primary" onClick={() => onOpen(saved.id)}>
              Open the saved capture
            </Button>
          ) : null
        }
      />
      {running ? null : (
        <Panel title="Start">
          <div className="flex flex-wrap items-end gap-3">
            <Field label="Source">
              <SegmentedControl
                aria-label="Source"
                value={source}
                options={[
                  { value: "interface", label: "Interface" },
                  { value: "replay", label: "Replay a capture" },
                ]}
                onValueChange={(v) => setKind(v as Kind)}
              />
            </Field>
            {source === "interface" ? (
              <Field label={`Interface${ifaces.data?.tool ? ` (${ifaces.data.tool})` : ""}`}>
                <Select
                  aria-label="Interface"
                  className="w-64"
                  value={iface}
                  placeholder="Choose the master's port"
                  disabled={!enabled}
                  options={(ifaces.data?.interfaces ?? []).map((i) => ({
                    value: i.name,
                    label: i.name,
                    ...(i.description ? { note: i.description } : {}),
                  }))}
                  onValueChange={(v) => setIface(v ?? "")}
                />
              </Field>
            ) : (
              <>
                <Field label="Capture">
                  <Select
                    aria-label="Capture to replay"
                    className="w-64"
                    value={replayId}
                    placeholder={
                      captures.length ? "Choose a loaded capture" : "Load a capture first"
                    }
                    disabled={captures.length === 0}
                    options={captures.map((c) => ({
                      value: String(c.id),
                      label: c.name,
                      note: c.provenance,
                    }))}
                    onValueChange={(v) => setReplayId(v ?? "")}
                  />
                </Field>
                <Field label="Speed">
                  <Select
                    aria-label="Replay speed"
                    className="w-24"
                    value={speed}
                    options={SPEEDS.map((s) => ({ value: s, label: `${s}×` }))}
                    onValueChange={(v) => setSpeed(v ?? "1")}
                  />
                </Field>
              </>
            )}
            <Field label="Analysis window">
              <NumberInput
                className="w-28"
                value={windowS}
                unit="s"
                min={1}
                max={600}
                onValueChange={(v) => setWindowS(Math.min(600, Math.max(1, v)))}
              />
            </Field>
            <Button
              variant="primary"
              icon={<Play className="size-4" />}
              onClick={() => void start()}
              disabled={busy || !ready}
            >
              Start
            </Button>
          </div>
          {interfaceNote ? <p className="mt-2 text-xs text-fg-muted">{interfaceNote}</p> : null}
          <p className="mt-2 text-xs text-fg-muted">
            FlowLab only listens: dumpcap or tcpdump captures EtherCAT frames (EtherType 0x88A4 and
            UDP port 34980) on the port you choose, and nothing is sent. Capture on the master's
            port or through a tap so both directions are seen.
          </p>
        </Panel>
      )}
      {snap?.message ? (
        <Callout
          tone={snap.state === "failed" ? "error" : "info"}
          title={snap.state === "failed" ? "Capture failed" : "Capture ended"}
        >
          <span className="whitespace-pre-wrap">{snap.message}</span>
        </Callout>
      ) : null}
      {snap && snap.series.length > 0 ? (
        <>
          <Readouts snap={snap} />
          <LiveCharts points={snap.series} />
          {snap.window ? (
            <div className="grid gap-4 lg:grid-cols-2">
              <Panel title={`Findings in the last ${(snap.window_ns / 1e9).toFixed(0)} s`}>
                <Findings findings={snap.window.findings} />
              </Panel>
              <Panel title="Stations in the window">
                <Table
                  columns={LIVE_STATION_COLUMNS}
                  rows={snap.window.stations}
                  rowKey={(s) => s.station}
                />
              </Panel>
            </div>
          ) : null}
          <Disclosure summary="How the live figures are made" count={snap.assumptions.length}>
            <ul className="ml-4 list-disc space-y-1 text-sm text-fg-muted">
              {snap.assumptions.map((a) => (
                <li key={a}>{a}</li>
              ))}
            </ul>
          </Disclosure>
        </>
      ) : null}
    </div>
  );
}
