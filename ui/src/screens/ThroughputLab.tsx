import {
  extent,
  histogram,
  Histogram,
  LineChart,
  padDomain,
  type Series,
} from "@vitavision/charts";
import {
  Badge,
  Button,
  Callout,
  ErrorBox,
  Field,
  Input,
  NumberInput,
  PageHeader,
  Panel,
  Progress,
  SegmentedControl,
  Select,
  Table,
  toast,
  type Column,
} from "@vitavision/ui";
import { Activity, ArrowLeftRight, Download, Trash } from "lucide-react";
import { useRef, useState } from "react";

import { api, errorMessage, type UploadBenchResult } from "../api";
import { Metric, MetricGrid } from "../components/Metric";
import { ProvenanceBadge } from "../components/ProvenanceBadge";
import { downloadJson, downloadText } from "../lib/download";
import { bitRate, byteRate, bytes, count } from "../lib/format";
import { fillSeededBytes, Sha256Hasher, sha256Hex } from "../lib/sha256";

type BenchDirection = "both" | "download" | "upload";

interface BenchRunRow {
  id: number;
  label: string;
  direction: "download" | "upload";
  bytes: number;
  seconds: number;
  bytesPerS: number;
  detailSeconds: number;
  shaOk: boolean;
  sha256: string;
}

interface LatencyProbeResult {
  samplesMs: number[];
  minMs: number;
  p50Ms: number;
  p95Ms: number;
  maxMs: number;
}

const SIZE_OPTIONS = [
  { value: String(64 * 1024), label: "64 KiB" },
  { value: String(256 * 1024), label: "256 KiB" },
  { value: String(1024 * 1024), label: "1 MiB" },
  { value: String(4 * 1024 * 1024), label: "4 MiB" },
  { value: String(16 * 1024 * 1024), label: "16 MiB" },
];

const HISTORY_COLUMNS: Column<BenchRunRow>[] = [
  { key: "id", header: "#", numeric: true, width: "3rem", cell: (r) => r.id },
  {
    key: "lbl",
    header: "Label",
    cell: (r) => <span className="text-xs">{r.label || "–"}</span>,
  },
  {
    key: "dir",
    header: "Direction",
    width: "6.5rem",
    cell: (r) => (
      <Badge tone="neutral">{r.direction === "download" ? "Download ↓" : "Upload ↑"}</Badge>
    ),
  },
  {
    key: "size",
    header: "Payload",
    numeric: true,
    cell: (r) => bytes(r.bytes),
    sortValue: (r) => r.bytes,
  },
  {
    key: "dur",
    header: "Duration",
    numeric: true,
    cell: (r) => `${r.seconds.toFixed(3)} s`,
    sortValue: (r) => r.seconds,
  },
  {
    key: "rate",
    header: "Throughput",
    numeric: true,
    cell: (r) => (
      <span className="font-mono">
        {byteRate(r.bytesPerS)}{" "}
        <span className="text-xs text-fg-subtle">({bitRate(r.bytesPerS * 8)})</span>
      </span>
    ),
    sortValue: (r) => r.bytesPerS,
  },
  {
    key: "detail",
    header: "TTFB / Server",
    numeric: true,
    cell: (r) =>
      r.direction === "download"
        ? `${(r.detailSeconds * 1000).toFixed(1)} ms TTFB`
        : `${r.detailSeconds.toFixed(3)} s srv`,
  },
  {
    key: "sha",
    header: "SHA-256",
    width: "6.5rem",
    cell: (r) => <Badge tone={r.shaOk ? "normal" : "defect"}>{r.shaOk ? "OK" : "MISMATCH"}</Badge>,
  },
];

function quantile(sorted: readonly number[], p: number): number {
  if (sorted.length === 0) return 0;
  const idx = (sorted.length - 1) * p;
  const lo = Math.floor(idx);
  const hi = Math.ceil(idx);
  const a = sorted[lo] ?? 0;
  const b = sorted[hi] ?? a;
  return a + (b - a) * (idx - lo);
}

function summarizeLatency(samplesMs: number[]): LatencyProbeResult {
  const sorted = [...samplesMs].sort((a, b) => a - b);
  return {
    samplesMs,
    minMs: sorted[0] ?? 0,
    p50Ms: quantile(sorted, 0.5),
    p95Ms: quantile(sorted, 0.95),
    maxMs: sorted[sorted.length - 1] ?? 0,
  };
}

function uploadWithProgress(
  url: string,
  payload: Uint8Array,
  signal: AbortSignal,
  onProgress: (loadedBytes: number, elapsedS: number) => void,
): Promise<UploadBenchResult> {
  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    const t0 = performance.now();
    const onAbort = () => xhr.abort();
    signal.addEventListener("abort", onAbort, { once: true });

    xhr.upload.onprogress = (ev) => {
      if (ev.lengthComputable) {
        onProgress(ev.loaded, Math.max((performance.now() - t0) / 1000, 1e-3));
      }
    };
    xhr.onload = () => {
      signal.removeEventListener("abort", onAbort);
      if (xhr.status >= 200 && xhr.status < 300) {
        try {
          resolve(JSON.parse(xhr.responseText) as UploadBenchResult);
        } catch (err) {
          reject(err instanceof Error ? err : new Error(String(err)));
        }
      } else {
        reject(new Error(`Upload failed (${xhr.status} ${xhr.statusText})`));
      }
    };
    xhr.onerror = () => {
      signal.removeEventListener("abort", onAbort);
      reject(new Error("Upload network error"));
    };
    xhr.onabort = () => {
      signal.removeEventListener("abort", onAbort);
      reject(new DOMException("Aborted", "AbortError"));
    };
    xhr.open("POST", url);
    xhr.setRequestHeader("content-type", "application/octet-stream");
    xhr.send(new Blob([payload.slice().buffer]));
  });
}

function toCsv(rows: readonly BenchRunRow[]): string {
  const head = "id,label,direction,bytes,seconds,bytes_per_s,ttfb_or_server_s,sha256_ok,sha256";
  const body = rows.map((r) =>
    [
      r.id,
      JSON.stringify(r.label),
      r.direction,
      r.bytes,
      r.seconds.toFixed(6),
      r.bytesPerS.toFixed(2),
      r.detailSeconds.toFixed(6),
      r.shaOk,
      r.sha256,
    ].join(","),
  );
  return [head, ...body].join("\n") + "\n";
}

/**
 * Interactive browser-to-slave TCP/HTTP throughput and latency lab with streaming SHA-256
 * verification over the active link (management LAN or EtherCAT EoE via `flowlab master gateway`).
 */
export function ThroughputLab() {
  const [direction, setDirection] = useState<BenchDirection>("both");
  const [sizeBytes, setSizeBytes] = useState(String(1024 * 1024));
  const [seed, setSeed] = useState(1);
  const [label, setLabel] = useState("");
  const [pingCount, setPingCount] = useState("20");

  const [running, setRunning] = useState(false);
  const [probing, setProbing] = useState(false);
  const [phase, setPhase] = useState<string | null>(null);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const [liveSeries, setLiveSeries] = useState<Series[]>([]);
  const [rows, setRows] = useState<BenchRunRow[]>([]);
  const [latency, setLatency] = useState<LatencyProbeResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  const nextIdRef = useRef(1);
  const abortRef = useRef<AbortController | null>(null);

  const runTransfer = async () => {
    const size = Number(sizeBytes);
    const ctl = new AbortController();
    abortRef.current = ctl;
    setRunning(true);
    setError(null);
    setLiveSeries([]);

    const dlPoints: { x: number; y: number }[] = [];
    const ulPoints: { x: number; y: number }[] = [];
    const updateSeries = () => {
      const next: Series[] = [];
      if (dlPoints.length > 0) next.push({ name: "Download (kB/s)", points: [...dlPoints] });
      if (ulPoints.length > 0) next.push({ name: "Upload (kB/s)", points: [...ulPoints] });
      setLiveSeries(next);
    };

    try {
      if (direction === "both" || direction === "download") {
        setPhase("Downloading…");
        setProgress({ done: 0, total: size });
        const t0 = performance.now();
        const res = await api.benchDownload(size, seed, ctl.signal);
        if (!res.ok) {
          throw new Error(`Download failed (${res.status} ${res.statusText})`);
        }
        const serverSha = res.headers.get("x-sha256");
        const hasher = new Sha256Hasher();
        let received = 0;
        let ttfbS = 0;
        let lastPointMs = t0;

        if (res.body) {
          const reader = res.body.getReader();
          while (true) {
            const { done, value } = await reader.read();
            if (done) break;
            if (value) {
              if (received === 0) {
                ttfbS = Math.max(0, (performance.now() - t0) / 1000);
              }
              hasher.update(value);
              received += value.byteLength;
              const now = performance.now();
              if (now - lastPointMs >= 120) {
                const elapsedS = Math.max((now - t0) / 1000, 1e-3);
                dlPoints.push({ x: elapsedS, y: received / elapsedS / 1000 });
                setProgress({ done: received, total: size });
                updateSeries();
                lastPointMs = now;
              }
            }
          }
        } else {
          const buf = new Uint8Array(await res.arrayBuffer());
          ttfbS = Math.max(0, (performance.now() - t0) / 1000);
          hasher.update(buf);
          received = buf.byteLength;
        }

        const seconds = Math.max((performance.now() - t0) / 1000, 1e-4);
        const bytesPerS = received / seconds;
        dlPoints.push({ x: seconds, y: bytesPerS / 1000 });
        setProgress({ done: received, total: size });
        updateSeries();

        const digest = hasher.digestHex();
        const shaOk = Boolean(serverSha) && received === size && digest === serverSha;
        const row: BenchRunRow = {
          id: nextIdRef.current++,
          label: label.trim(),
          direction: "download",
          bytes: received,
          seconds,
          bytesPerS,
          detailSeconds: ttfbS,
          shaOk,
          sha256: digest,
        };
        setRows((prev) => [row, ...prev]);
        if (!shaOk) {
          toast({
            tone: "error",
            title: "Download SHA-256 mismatch",
            description: `Expected ${serverSha ?? "none"}, got ${digest}`,
          });
        }
      }

      if (direction === "both" || direction === "upload") {
        setPhase("Preparing upload payload…");
        const payload = new Uint8Array(size);
        fillSeededBytes(seed + 1, payload);
        const clientSha = sha256Hex(payload);

        setPhase("Uploading…");
        setProgress({ done: 0, total: size });
        const t0 = performance.now();
        let lastPointMs = t0;

        const res = await uploadWithProgress(
          api.benchUploadUrl(),
          payload,
          ctl.signal,
          (loaded, elapsedS) => {
            const now = performance.now();
            if (now - lastPointMs >= 120) {
              ulPoints.push({ x: elapsedS, y: loaded / elapsedS / 1000 });
              setProgress({ done: loaded, total: size });
              updateSeries();
              lastPointMs = now;
            }
          },
        );

        const seconds = Math.max((performance.now() - t0) / 1000, 1e-4);
        const bytesPerS = res.bytes / seconds;
        ulPoints.push({ x: seconds, y: bytesPerS / 1000 });
        setProgress({ done: res.bytes, total: size });
        updateSeries();

        const shaOk = res.bytes === size && res.sha256 === clientSha;
        const row: BenchRunRow = {
          id: nextIdRef.current++,
          label: label.trim(),
          direction: "upload",
          bytes: res.bytes,
          seconds,
          bytesPerS,
          detailSeconds: res.server_seconds,
          shaOk,
          sha256: res.sha256,
        };
        setRows((prev) => [row, ...prev]);
        if (!shaOk) {
          toast({
            tone: "error",
            title: "Upload SHA-256 mismatch",
            description: `Expected ${clientSha}, server reported ${res.sha256}`,
          });
        }
      }
    } catch (e) {
      if (!(e instanceof DOMException && e.name === "AbortError")) {
        const msg = errorMessage(e);
        setError(msg);
        toast({ tone: "error", title: "Throughput benchmark failed", description: msg });
      }
    } finally {
      abortRef.current = null;
      setRunning(false);
      setPhase(null);
      setProgress(null);
    }
  };

  const runProbe = async () => {
    const n = Number(pingCount);
    const ctl = new AbortController();
    abortRef.current = ctl;
    setProbing(true);
    setError(null);
    const samples: number[] = [];
    try {
      for (let i = 0; i < n; i++) {
        const t0 = performance.now();
        await api.benchPing(ctl.signal);
        samples.push(Math.max(0.01, performance.now() - t0));
      }
      setLatency(summarizeLatency(samples));
    } catch (e) {
      if (!(e instanceof DOMException && e.name === "AbortError")) {
        setError(errorMessage(e));
      }
    } finally {
      abortRef.current = null;
      setProbing(false);
    }
  };

  const latestDown = rows.find((r) => r.direction === "download") ?? null;
  const latestUp = rows.find((r) => r.direction === "upload") ?? null;

  const latencySeries: Series[] = latency
    ? [
        {
          name: "RTT (ms)",
          points: latency.samplesMs.map((y, i) => ({ x: i + 1, y })),
        },
      ]
    : [];
  const latencyHist = latency
    ? (() => {
        const [lo, hi] = extent(latency.samplesMs);
        const dom = padDomain(lo, hi === lo ? lo + 1 : hi);
        return { counts: histogram(latency.samplesMs, dom, 14), domain: dom };
      })()
    : null;

  return (
    <div className="mx-auto flex max-w-[110rem] flex-col gap-5 p-6">
      <PageHeader
        title="Throughput Lab"
        meta={
          <span className="flex flex-wrap items-center gap-x-3 gap-y-1">
            <ProvenanceBadge provenance="measured" />
            <span className="text-xs">
              Browser-to-server TCP/HTTP transfer &amp; RTT benchmark with SHA-256 verification
            </span>
          </span>
        }
        actions={
          <>
            {running || probing ? (
              <Button variant="danger" onClick={() => abortRef.current?.abort()}>
                Abort
              </Button>
            ) : (
              <>
                <Button
                  variant="secondary"
                  icon={<Activity className="size-4" />}
                  onClick={() => void runProbe()}
                >
                  Probe RTT ({pingCount}×)
                </Button>
                <Button
                  variant="primary"
                  icon={<ArrowLeftRight className="size-4" />}
                  onClick={() => void runTransfer()}
                >
                  Run transfer
                </Button>
              </>
            )}
          </>
        }
      />

      <Callout tone="info" title="End-to-end over EoE or management LAN">
        This view streams uncompressed byte payloads over the active HTTP connection: point your
        browser at the slave&apos;s management IP for baseline LAN speed, or open{" "}
        <code>http://127.0.0.1:18080</code> through{" "}
        <code>flowlab master gateway --forward 18080:8080</code> to measure live TCP over EtherCAT
        EoE.
      </Callout>

      {error ? <ErrorBox>{error}</ErrorBox> : null}

      <Panel title="Test configuration">
        <div className="flex flex-wrap items-end gap-4">
          <Field label="Direction" as="group">
            <SegmentedControl
              aria-label="Direction"
              value={direction}
              disabled={running || probing}
              options={[
                { value: "both", label: "Download + Upload" },
                { value: "download", label: "Download ↓" },
                { value: "upload", label: "Upload ↑" },
              ]}
              onValueChange={(v) => v && setDirection(v as BenchDirection)}
            />
          </Field>
          <Field label="Payload size">
            <Select
              aria-label="Payload size"
              className="w-36"
              value={sizeBytes}
              disabled={running || probing}
              options={SIZE_OPTIONS}
              onValueChange={(v) => v && setSizeBytes(v)}
            />
          </Field>
          <Field label="Seed">
            <NumberInput
              aria-label="Seed"
              className="w-24"
              value={seed}
              min={0}
              disabled={running || probing}
              onValueChange={(v) => setSeed(Math.max(0, Math.round(v)))}
            />
          </Field>
          <Field label="RTT pings" as="group">
            <SegmentedControl
              aria-label="RTT pings"
              value={pingCount}
              disabled={running || probing}
              options={[
                { value: "20", label: "20" },
                { value: "50", label: "50" },
                { value: "100", label: "100" },
              ]}
              onValueChange={(v) => v && setPingCount(v)}
            />
          </Field>
          <Field label="Run label (optional)">
            <Input
              aria-label="Run label"
              className="w-48"
              value={label}
              disabled={running || probing}
              onChange={(e) => setLabel(e.target.value)}
              placeholder="e.g. 1024 B fast EoE"
            />
          </Field>
        </div>

        {running && progress ? (
          <div className="mt-4 flex flex-col gap-1.5">
            <div className="flex justify-between text-xs text-fg-muted">
              <span>{phase}</span>
              <span className="font-mono">
                {bytes(progress.done)} / {bytes(progress.total)}
              </span>
            </div>
            <Progress value={progress.done} max={progress.total} aria-label="Transfer progress" />
          </div>
        ) : null}
      </Panel>

      {latestDown || latestUp || latency ? (
        <MetricGrid>
          <Metric
            label="Latest download ↓"
            value={latestDown ? byteRate(latestDown.bytesPerS) : "–"}
            sub={
              latestDown
                ? `${bytes(latestDown.bytes)} in ${latestDown.seconds.toFixed(2)} s · SHA-256 ${latestDown.shaOk ? "ok" : "MISMATCH"}`
                : undefined
            }
            verdict={latestDown ? (latestDown.shaOk ? "normal" : "defect") : undefined}
          />
          <Metric
            label="Latest upload ↑"
            value={latestUp ? byteRate(latestUp.bytesPerS) : "–"}
            sub={
              latestUp
                ? `${bytes(latestUp.bytes)} in ${latestUp.seconds.toFixed(2)} s · SHA-256 ${latestUp.shaOk ? "ok" : "MISMATCH"}`
                : undefined
            }
            verdict={latestUp ? (latestUp.shaOk ? "normal" : "defect") : undefined}
          />
          <Metric
            label="RTT median (p50)"
            value={latency ? `${latency.p50Ms.toFixed(2)} ms` : "–"}
            sub={latency ? `min ${latency.minMs.toFixed(2)} ms` : undefined}
          />
          <Metric
            label="RTT p95 / max"
            value={latency ? `${latency.p95Ms.toFixed(2)} ms` : "–"}
            sub={latency ? `max ${latency.maxMs.toFixed(2)} ms` : undefined}
          />
        </MetricGrid>
      ) : null}

      {liveSeries.length > 0 ? (
        <Panel title="Live transfer throughput">
          <LineChart
            label="Transfer throughput over elapsed time"
            series={liveSeries}
            xLabel="elapsed (s)"
            yLabel="kB/s"
            variant="fluid"
            height={200}
            showLegend
          />
        </Panel>
      ) : null}

      {latency && latencyHist ? (
        <div className="grid gap-4 lg:grid-cols-2">
          <Panel title={`RTT per request (${count(latency.samplesMs.length)} pings)`}>
            <LineChart
              label="HTTP round-trip time per probe"
              series={latencySeries}
              xLabel="request #"
              yLabel="ms"
              variant="fluid"
              height={190}
            />
          </Panel>
          <Panel title="RTT distribution">
            <Histogram
              label="RTT histogram"
              xLabel="RTT"
              unit="ms"
              yLabel="count"
              counts={latencyHist.counts}
              domain={latencyHist.domain}
              variant="fluid"
              height={190}
            />
          </Panel>
        </div>
      ) : null}

      <Panel
        title={`Recorded transfers (${rows.length})`}
        actions={
          rows.length > 0 ? (
            <div className="flex items-center gap-1.5">
              <Button
                size="sm"
                variant="ghost"
                icon={<Download className="size-3.5" />}
                onClick={() => downloadText("throughput-bench.csv", toCsv(rows), "text/csv")}
              >
                CSV
              </Button>
              <Button
                size="sm"
                variant="ghost"
                icon={<Download className="size-3.5" />}
                onClick={() => downloadJson("throughput-bench.json", rows)}
              >
                JSON
              </Button>
              <Button
                size="sm"
                variant="ghost"
                icon={<Trash className="size-3.5" />}
                onClick={() => setRows([])}
              >
                Clear
              </Button>
            </div>
          ) : undefined
        }
      >
        <Table
          columns={HISTORY_COLUMNS}
          rows={rows}
          rowKey={(r) => r.id}
          empty="Run a download or upload transfer to record measured throughput and SHA-256 integrity."
        />
      </Panel>
    </div>
  );
}
