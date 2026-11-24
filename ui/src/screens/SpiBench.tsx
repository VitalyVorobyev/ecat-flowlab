import { LineChart, type Series } from "@vitavision/charts";
import {
  Badge,
  Button,
  Callout,
  ConfirmDialog,
  DensityProvider,
  Disclosure,
  ErrorBox,
  Field,
  Input,
  NumberInput,
  PageHeader,
  Panel,
  SegmentedControl,
  Table,
  ToggleChip,
  toast,
  type Column,
} from "@vitavision/ui";
import { Download, FlaskConical, Play } from "lucide-react";
import { useState } from "react";

import {
  api,
  type BenchSizeSummaryView,
  type Capabilities,
  errorMessage,
  type PdiBenchResult,
  type PdiValuesView,
  type SlaveReadInstruction,
} from "../api";
import { Metric, MetricGrid } from "../components/Metric";
import { ProvenanceBadge } from "../components/ProvenanceBadge";
import { downloadJson, downloadText } from "../lib/download";
import { bitRate, ns, percent } from "../lib/format";

interface BenchRow extends BenchSizeSummaryView {
  fitted_ns: number | null;
  relative_error: number | null;
}

const BENCH_COLUMNS: Column<BenchRow>[] = [
  {
    key: "dir",
    header: "Direction",
    width: "6rem",
    cell: (r) => <Badge tone="neutral">{r.direction}</Badge>,
  },
  { key: "bytes", header: "Size", numeric: true, cell: (r) => `${r.bytes} B` },
  { key: "n", header: "Samples", numeric: true, cell: (r) => r.samples },
  { key: "med", header: "Median", numeric: true, cell: (r) => ns(r.median_ns) },
  { key: "p95", header: "p95", numeric: true, cell: (r) => ns(r.p95_ns) },
  { key: "min", header: "Min", numeric: true, cell: (r) => ns(r.min_ns) },
  {
    key: "fit",
    header: "Fitted",
    numeric: true,
    cell: (r) => (r.fitted_ns === null ? "–" : ns(r.fitted_ns)),
  },
  {
    key: "err",
    header: "Residual",
    numeric: true,
    cell: (r) =>
      r.relative_error === null
        ? "–"
        : `${r.relative_error >= 0 ? "+" : "−"}${Math.abs(100 * r.relative_error).toFixed(2)} %`,
  },
];

type TuningPreset = "fast" | "stack" | "custom";

function tuningString(preset: TuningPreset, flags: readonly string[]): string {
  if (preset !== "custom") return preset;
  return flags.length > 0 ? flags.join(",") : "stack";
}

function parseSizes(raw: string): number[] | null {
  const parts = raw
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean)
    .map(Number);
  if (parts.length === 0 || parts.some((n) => !Number.isInteger(n) || n < 1 || n > 4096)) {
    return null;
  }
  return parts;
}

function buildRows(result: PdiBenchResult): BenchRow[] {
  const resMap = new Map<string, { fitted_ns: number; relative_error: number }>();
  for (const s of result.fit?.read?.sizes ?? []) {
    resMap.set(`read:${s.bytes}`, { fitted_ns: s.fitted_ns, relative_error: s.relative_error });
  }
  for (const s of result.fit?.write?.sizes ?? []) {
    resMap.set(`write:${s.bytes}`, { fitted_ns: s.fitted_ns, relative_error: s.relative_error });
  }
  return result.summaries.map((s) => {
    const f = resMap.get(`${s.direction}:${s.bytes}`);
    return {
      ...s,
      fitted_ns: f?.fitted_ns ?? null,
      relative_error: f?.relative_error ?? null,
    };
  });
}

function buildChartSeries(result: PdiBenchResult): Series[] {
  const series: Series[] = [];
  const reads = result.summaries.filter((s) => s.direction === "read");
  const writes = result.summaries.filter((s) => s.direction === "write");

  if (reads.length > 0) {
    series.push({
      name: "Read median",
      points: reads.map((s) => ({ x: s.bytes, y: s.median_ns / 1e3 })),
    });
  }
  if (result.fit?.read) {
    series.push({
      name: "Read fitted",
      points: result.fit.read.sizes.map((s) => ({ x: s.bytes, y: s.fitted_ns / 1e3 })),
    });
  }
  if (writes.length > 0) {
    series.push({
      name: "Write median",
      points: writes.map((s) => ({ x: s.bytes, y: s.median_ns / 1e3 })),
    });
  }
  if (result.fit?.write) {
    series.push({
      name: "Write fitted",
      points: result.fit.write.sizes.map((s) => ({ x: s.bytes, y: s.fitted_ns / 1e3 })),
    });
  }
  return series;
}

/**
 * Timed ESC read/write benchmark over LAN9252 SPI (`flowlab pdi-bench`) and linear PDI
 * timing fit (`flowlab pdi-fit`), with one-click export into Simulate.
 */
export function SpiBench({
  capabilities,
  slaveRunning,
  onStopSlave,
  onApplyPdiFit,
}: {
  capabilities: Capabilities | null;
  slaveRunning: boolean;
  onStopSlave: () => Promise<void>;
  onApplyPdiFit: (pdi: PdiValuesView) => void;
}) {
  const hw = capabilities?.slave ?? null;
  const defaultSim = hw
    ? hw.spidev_devices.length === 0 && !hw.default_device && hw.sim_pdi
    : false;
  const [simChoice, setSimChoice] = useState<boolean | null>(null);
  const sim = simChoice ?? defaultSim;

  const defaultDevice = hw?.default_device ?? hw?.spidev_devices[0] ?? "/dev/spidev1.0";
  const [deviceChoice, setDeviceChoice] = useState<string | null>(null);
  const device = deviceChoice ?? defaultDevice;

  const [clockMhz, setClockMhz] = useState(20);
  const [mode, setMode] = useState("0");
  const [readInst, setReadInst] = useState<SlaveReadInstruction>("read");
  const [tuningPreset, setTuningPreset] = useState<TuningPreset>("fast");
  const [customFlags, setCustomFlags] = useState<string[]>(["split", "noabort"]);
  const [sizesText, setSizesText] = useState("1,2,4,8,16,32,64,128,256,512,1024");
  const [reps, setReps] = useState(30);
  const [warmup, setWarmup] = useState(3);
  const [alEvent, setAlEvent] = useState(true);
  const [appStopped, setAppStopped] = useState(false);
  const [allowWrites, setAllowWrites] = useState(false);
  const [area, setArea] = useState("0x1800+1024");

  const [result, setResult] = useState<PdiBenchResult | null>(null);
  const [busy, setBusy] = useState(false);
  const [stopping, setStopping] = useState(false);
  const [confirmWriteOpen, setConfirmWriteOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const writesPermitted = sim ? Boolean(hw?.sim_pdi) : Boolean(hw?.allow_writes);
  const canRun =
    !slaveRunning &&
    (sim ? Boolean(hw?.sim_pdi) : Boolean(device.trim())) &&
    !(allowWrites && !sim && !hw?.allow_writes);

  const toggleFlag = (flag: string) => {
    setCustomFlags((prev) =>
      prev.includes(flag) ? prev.filter((f) => f !== flag) : [...prev, flag],
    );
  };

  const executeRun = async () => {
    const sizes = parseSizes(sizesText);
    if (!sizes) {
      setError("Sizes must be comma-separated integers between 1 and 4096 bytes.");
      return;
    }
    setBusy(true);
    setError(null);
    setConfirmWriteOpen(false);
    try {
      const r = await api.pdiBenchRun({
        device: sim ? null : device,
        sim,
        clock_hz: Math.round(clockMhz * 1e6),
        mode: Number(mode),
        read_instruction: readInst,
        spi_tuning: tuningString(tuningPreset, customFlags),
        sizes,
        reps,
        warmup,
        area: allowWrites || appStopped ? area.trim() || null : null,
        allow_writes: allowWrites,
        app_stopped: appStopped,
        no_al_event: !alEvent,
        chip_id: null,
        seed: 1,
        sim_transaction_ns: 5000,
      });
      setResult(r);
      toast({ tone: "success", title: "SPI benchmark complete", description: r.target });
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const handleStopSlave = async () => {
    setStopping(true);
    try {
      await onStopSlave();
    } finally {
      setStopping(false);
    }
  };

  const maxResidual = result?.fit
    ? Math.max(result.fit.read?.max_relative_error ?? 0, result.fit.write?.max_relative_error ?? 0)
    : null;

  return (
    <div className="mx-auto flex max-w-[110rem] flex-col gap-5 p-6">
      <PageHeader
        title="SPI Bench"
        meta={
          result ? (
            <span className="flex flex-wrap items-center gap-x-3 gap-y-1">
              <ProvenanceBadge provenance={result.provenance} />
              <span className="font-mono text-xs">{result.target}</span>
              <span className="font-mono text-xs">
                LAN9252 rev {result.link.revision} · AL {result.link.al_state} · tuning{" "}
                {result.spi_tuning}
              </span>
            </span>
          ) : (
            "Measure complete LAN9252 SPI read/write latency across sizes and fit the simulator PDI model."
          )
        }
        actions={
          <>
            {result?.fit ? (
              <Button
                variant="secondary"
                icon={<FlaskConical className="size-4" />}
                onClick={() => onApplyPdiFit(result.fit!.pdi)}
              >
                Apply to Simulate
              </Button>
            ) : null}
            {result ? (
              <>
                <Button
                  variant="ghost"
                  icon={<Download className="size-4" />}
                  onClick={() => downloadText("pdi-bench.csv", result.csv, "text/csv")}
                >
                  CSV
                </Button>
                <Button
                  variant="ghost"
                  icon={<Download className="size-4" />}
                  onClick={() => downloadJson("pdi-bench.json", result)}
                >
                  JSON
                </Button>
              </>
            ) : null}
            <Button
              variant="primary"
              icon={<Play className="size-4" />}
              loading={busy}
              disabled={!canRun}
              onClick={() => {
                if (allowWrites && !sim) {
                  setConfirmWriteOpen(true);
                } else {
                  void executeRun();
                }
              }}
            >
              Run benchmark
            </Button>
          </>
        }
      />

      {slaveRunning ? (
        <Callout
          tone="warning"
          title="PDI slave is currently using the SPI bus"
          actions={
            <Button
              size="sm"
              variant="danger"
              loading={stopping}
              onClick={() => void handleStopSlave()}
            >
              Stop PDI slave
            </Button>
          }
        >
          Stop the active PDI slave session before benchmarking raw SPI transfers.
        </Callout>
      ) : null}

      {allowWrites && !sim && hw && !hw.allow_writes ? (
        <Callout tone="warning" title="Hardware writes require --allow-writes">
          Writing into ESC process RAM requires starting <code>flowlab serve</code> with{" "}
          <code>--allow-writes</code>.
        </Callout>
      ) : null}

      {error ? <ErrorBox>{error}</ErrorBox> : null}

      <div className="grid gap-5 xl:grid-cols-[minmax(0,4.5fr)_minmax(0,7.5fr)]">
        <Panel title="Benchmark parameters">
          <DensityProvider value="compact">
            <div className="flex flex-col gap-3">
              <Field label="Target" as="group">
                <SegmentedControl
                  aria-label="SPI bench target"
                  value={sim ? "sim" : "hardware"}
                  options={[
                    { value: "hardware", label: "Hardware spidev" },
                    { value: "sim", label: "Simulated LAN9252" },
                  ]}
                  onValueChange={(v) => setSimChoice(v === "sim")}
                />
              </Field>
              {!sim ? (
                <Field label="SPI device">
                  <Input
                    aria-label="SPI device"
                    className="font-mono"
                    value={device}
                    onChange={(e) => setDeviceChoice(e.target.value)}
                    placeholder="/dev/spidev1.0"
                    list="bench-spidev-list"
                  />
                  <datalist id="bench-spidev-list">
                    {(hw?.spidev_devices ?? []).map((p) => (
                      <option key={p} value={p} />
                    ))}
                  </datalist>
                </Field>
              ) : null}

              <div className="grid gap-3 sm:grid-cols-2">
                <Field label="SPI clock">
                  <NumberInput
                    aria-label="SPI clock MHz"
                    value={clockMhz}
                    unit="MHz"
                    min={1}
                    max={80}
                    onValueChange={(v) => setClockMhz(Math.max(1, Math.min(80, v)))}
                  />
                </Field>
                <Field label="SPI mode" as="group">
                  <SegmentedControl
                    aria-label="SPI mode"
                    value={mode}
                    options={[
                      { value: "0", label: "Mode 0" },
                      { value: "3", label: "Mode 3" },
                    ]}
                    onValueChange={(v) => v && setMode(v)}
                  />
                </Field>
              </div>

              <div className="grid gap-3 sm:grid-cols-2">
                <Field label="Read opcode" as="group">
                  <SegmentedControl
                    aria-label="Read opcode"
                    value={readInst}
                    options={[
                      { value: "read", label: "READ" },
                      { value: "fast_read", label: "FAST READ" },
                    ]}
                    onValueChange={(v) => v && setReadInst(v as SlaveReadInstruction)}
                  />
                </Field>
                <Field label="Repetitions / warmup" as="group">
                  <div className="grid grid-cols-2 gap-2">
                    <NumberInput
                      aria-label="Repetitions"
                      value={reps}
                      min={1}
                      max={500}
                      onValueChange={(v) => setReps(Math.max(1, Math.min(500, Math.round(v))))}
                    />
                    <NumberInput
                      aria-label="Warmup"
                      value={warmup}
                      min={0}
                      max={50}
                      onValueChange={(v) => setWarmup(Math.max(0, Math.min(50, Math.round(v))))}
                    />
                  </div>
                </Field>
              </div>

              <Field label="SPI bus tuning" as="group">
                <div className="flex flex-col gap-2">
                  <SegmentedControl
                    aria-label="SPI tuning"
                    value={tuningPreset}
                    options={[
                      { value: "fast", label: "fast (split,noabort)" },
                      { value: "stack", label: "stack" },
                      { value: "custom", label: "custom" },
                    ]}
                    onValueChange={(v) => v && setTuningPreset(v as TuningPreset)}
                  />
                  {tuningPreset === "custom" ? (
                    <div className="flex flex-wrap gap-1.5">
                      {["split", "noabort", "batch", "fifo"].map((flag) => (
                        <ToggleChip
                          key={flag}
                          checked={customFlags.includes(flag)}
                          onCheckedChange={() => toggleFlag(flag)}
                        >
                          {flag}
                        </ToggleChip>
                      ))}
                    </div>
                  ) : null}
                </div>
              </Field>

              <Field label="Access sizes (bytes)">
                <Input
                  aria-label="Access sizes"
                  className="font-mono text-xs"
                  value={sizesText}
                  onChange={(e) => setSizesText(e.target.value)}
                />
              </Field>

              <div className="flex flex-wrap gap-2 pt-1">
                <ToggleChip checked={alEvent} onCheckedChange={setAlEvent}>
                  AL Event read per access
                </ToggleChip>
                <ToggleChip
                  checked={allowWrites}
                  disabled={!sim && !writesPermitted}
                  onCheckedChange={(on) => {
                    setAllowWrites(on);
                    if (on) setAppStopped(false);
                  }}
                >
                  Benchmark writes
                </ToggleChip>
                <ToggleChip
                  checked={appStopped}
                  disabled={allowWrites}
                  onCheckedChange={setAppStopped}
                >
                  App stopped (reads only)
                </ToggleChip>
              </div>

              {allowWrites || appStopped ? (
                <Field label="Process RAM area (START+LEN)">
                  <Input
                    aria-label="Process RAM area"
                    className="font-mono"
                    value={area}
                    onChange={(e) => setArea(e.target.value)}
                    placeholder="0x1800+1024"
                  />
                </Field>
              ) : null}
            </div>
          </DensityProvider>
        </Panel>

        <div className="flex flex-col gap-5">
          {result ? (
            <>
              <MetricGrid compact>
                <Metric
                  label="Fitted read rate"
                  value={result.fit ? bitRate(result.fit.pdi.read_bps) : "–"}
                  sub={result.fit?.read ? `overhead ${ns(result.fit.read.overhead_ns)}` : undefined}
                />
                <Metric
                  label="Fitted write rate"
                  value={result.fit ? bitRate(result.fit.pdi.write_bps) : "–"}
                  sub={
                    result.fit?.write
                      ? `overhead ${ns(result.fit.write.overhead_ns)}`
                      : "from read model"
                  }
                />
                <Metric
                  label="Access overhead"
                  value={result.fit ? ns(result.fit.pdi.access_overhead_ns) : "–"}
                  sub={`read area ${result.read_area}`}
                />
                <Metric
                  label="Max fit residual"
                  value={maxResidual === null ? "–" : percent(maxResidual * 100)}
                  sub={result.write_area ? `write area ${result.write_area}` : "reads only"}
                />
              </MetricGrid>

              {result.fit_error ? (
                <Callout tone="warning" title="Model fit skipped">
                  {result.fit_error}
                </Callout>
              ) : null}

              <Panel title="Access latency vs. size">
                <LineChart
                  label="Median and fitted SPI access duration by size"
                  series={buildChartSeries(result)}
                  xLabel="access size (bytes)"
                  yLabel="µs"
                  variant="fluid"
                  height={220}
                  showLegend
                />
              </Panel>

              <Panel title="Per-size timings &amp; linear fit residuals">
                <Table
                  columns={BENCH_COLUMNS}
                  rows={buildRows(result)}
                  rowKey={(r) => `${r.direction}-${r.bytes}`}
                />
              </Panel>

              {result.fit ? (
                <Disclosure summary="Fit assumptions" count={result.fit.assumptions.length}>
                  <ul className="ml-4 list-disc space-y-1 text-sm text-fg-muted">
                    {result.fit.assumptions.map((a) => (
                      <li key={a}>{a}</li>
                    ))}
                  </ul>
                </Disclosure>
              ) : null}
            </>
          ) : (
            <Callout tone="info" title="No benchmark run yet">
              Run the benchmark to measure SPI access times across payload sizes, inspect linear fit
              residuals, export <code>pdi-fit</code> CSV samples, or push the fitted PDI timings
              directly into <strong>Simulate</strong>.
            </Callout>
          )}
        </div>
      </div>

      <ConfirmDialog
        open={confirmWriteOpen}
        onOpenChange={setConfirmWriteOpen}
        title="Run SPI write benchmark on ESC RAM?"
        description={
          <>
            Writes will overwrite process RAM area <code>{area}</code> on <code>{device}</code>.
            Confirm the ESC is in <code>INIT</code> and no slave stack uses that range.
          </>
        }
        confirmLabel="Run read + write benchmark"
        loading={busy}
        onConfirm={() => void executeRun()}
      />
    </div>
  );
}
