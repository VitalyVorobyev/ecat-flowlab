import { LineChart, type Series } from "@vitavision/charts";
import {
  Badge,
  Button,
  Callout,
  ConfirmDialog,
  DensityProvider,
  ErrorBox,
  Field,
  Input,
  NumberInput,
  PageHeader,
  Panel,
  ReadoutStrip,
  SegmentedControl,
  Select,
  StatusDot,
  Table,
  ToggleChip,
  toast,
  type Column,
} from "@vitavision/ui";
import { Play, Square } from "lucide-react";
import { useState } from "react";

import {
  api,
  type Capabilities,
  errorMessage,
  type ReceivedPayload,
  type SlaveAlEvent,
  type SlaveRatePoint,
  type SlaveReadInstruction,
  type SlaveStatus,
} from "../api";
import { Metric, MetricGrid } from "../components/Metric";
import { ProvenanceBadge } from "../components/ProvenanceBadge";
import { byteRate, bytes, count, hex } from "../lib/format";

const AL_EVENT_COLUMNS: Column<SlaveAlEvent>[] = [
  {
    key: "t",
    header: "Time",
    numeric: true,
    width: "6rem",
    cell: (e) => <span className="font-mono text-xs">{e.t_s.toFixed(2)} s</span>,
  },
  {
    key: "st",
    header: "AL state",
    width: "7rem",
    cell: (e) =>
      e.error !== null ? (
        <Badge tone="defect">{`${e.state} (${hex(e.error)})`}</Badge>
      ) : (
        <Badge tone={e.state === "PREOP" ? "normal" : "neutral"}>{e.state}</Badge>
      ),
  },
  {
    key: "mbx",
    header: "Mailboxes",
    cell: (e) =>
      e.mailboxes ? (
        <span className="font-mono text-xs">
          SM0 {hex(e.mailboxes.out_start)}+{e.mailboxes.out_len} B · SM1 {hex(e.mailboxes.in_start)}
          +{e.mailboxes.in_len} B
        </span>
      ) : (
        <span className="text-xs text-fg-subtle">closed</span>
      ),
  },
];

const RECEIVED_COLUMNS: Column<ReceivedPayload>[] = [
  {
    key: "t",
    header: "Time",
    numeric: true,
    width: "6rem",
    cell: (r) => <span className="font-mono text-xs">{r.t_s.toFixed(2)} s</span>,
  },
  {
    key: "proto",
    header: "Protocol",
    width: "8rem",
    cell: (r) => <Badge tone="info">{r.protocol}</Badge>,
  },
  {
    key: "name",
    header: "Item",
    cell: (r) => <span className="font-mono text-xs">{r.name ?? "–"}</span>,
  },
  {
    key: "bytes",
    header: "Size",
    numeric: true,
    width: "7rem",
    cell: (r) => bytes(r.bytes),
  },
  {
    key: "sha",
    header: "SHA-256",
    cell: (r) => (
      <span className="font-mono text-xs text-fg-muted" title={r.sha256}>
        {r.sha256.slice(0, 16)}…
      </span>
    ),
  },
];

const UPLOAD_OPTIONS = [
  { value: "0", label: "None (0 B)" },
  { value: "65536", label: "64 KiB" },
  { value: "1000000", label: "1 MB" },
  { value: "4000000", label: "4 MB" },
];

type TuningPreset = "fast" | "stack" | "custom";

function tuningString(preset: TuningPreset, flags: readonly string[]): string {
  if (preset !== "custom") return preset;
  return flags.length > 0 ? flags.join(",") : "stack";
}

function seriesFromPoints(
  points: SlaveRatePoint[],
  lines: { name: string; pick: (p: SlaveRatePoint) => number }[],
): Series[] {
  return lines.map((l) => ({
    name: l.name,
    points: points.map((p) => ({ x: p.t_s, y: l.pick(p) })),
  }));
}

function statusTone(state: SlaveStatus["state"]): "normal" | "defect" | "neutral" | "unlabeled" {
  switch (state) {
    case "running":
      return "normal";
    case "failed":
      return "defect";
    case "stopped":
      return "neutral";
    default:
      return "unlabeled";
  }
}

/**
 * Configure, launch, and monitor FlowLab's PDI slave (`flowlab pdi-slave`) on a real LAN9252
 * over spidev or on the simulated LAN9252 backend.
 */
export function SlaveControl({
  capabilities,
  status,
  onStatusChange,
}: {
  capabilities: Capabilities | null;
  status: SlaveStatus;
  onStatusChange: (status: SlaveStatus) => void;
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
  const [maxMailbox, setMaxMailbox] = useState(1024);
  const [messageOnly, setMessageOnly] = useState("message_only");
  const [uploadBytes, setUploadBytes] = useState("1000000");
  const [uploadSeed, setUploadSeed] = useState(7);
  const [eoeBridge, setEoeBridge] = useState("");
  const [eoeHost, setEoeHost] = useState("");

  const [confirmOpen, setConfirmOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const running = status.state === "running";
  const canStart = sim ? Boolean(hw?.sim_pdi) : Boolean(hw?.allow_replace_app);

  const toggleFlag = (flag: string) => {
    setCustomFlags((prev) =>
      prev.includes(flag) ? prev.filter((f) => f !== flag) : [...prev, flag],
    );
  };

  const doStart = async () => {
    setBusy(true);
    setError(null);
    try {
      const next = await api.slaveStart({
        device: sim ? null : device,
        sim,
        clock_hz: Math.round(clockMhz * 1e6),
        mode: Number(mode),
        read_instruction: readInst,
        spi_tuning: tuningString(tuningPreset, customFlags),
        max_mailbox_bytes: maxMailbox,
        message_only: messageOnly === "message_only",
        upload_bytes: Number(uploadBytes),
        upload_seed: uploadSeed,
        eoe_bridge: eoeBridge.trim() || null,
        eoe_host: eoeHost.trim() || null,
        chip_id: null,
      });
      onStatusChange(next);
      setConfirmOpen(false);
      toast({
        tone: "success",
        title: "PDI slave started",
        description: next.target ?? undefined,
      });
    } catch (e) {
      const msg = errorMessage(e);
      setError(msg);
      toast({ tone: "error", title: "Could not start PDI slave", description: msg });
    } finally {
      setBusy(false);
    }
  };

  const doStop = async () => {
    setBusy(true);
    setError(null);
    try {
      const next = await api.slaveStop();
      onStatusChange(next);
      toast({ tone: "info", title: "PDI slave stopped", description: "AL Status set to INIT" });
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const latest = status.series.at(-1);
  const throughputSeries = seriesFromPoints(status.series, [
    { name: "to slave (SM0)", pick: (p) => p.bytes_in_s / 1e3 },
    { name: "from slave (SM1)", pick: (p) => p.bytes_out_s / 1e3 },
  ]);
  const msgSeries = seriesFromPoints(status.series, [
    { name: "msg in/s", pick: (p) => p.msg_in_s },
    { name: "msg out/s", pick: (p) => p.msg_out_s },
  ]);

  const spidevOptions = hw?.spidev_devices ?? [];
  const netOptions = hw?.net_interfaces ?? [];

  return (
    <div className="mx-auto flex max-w-[110rem] flex-col gap-5 p-6">
      <PageHeader
        title="PDI Slave"
        meta={
          <span className="flex flex-wrap items-center gap-x-3 gap-y-1">
            <StatusDot tone={statusTone(status.state)}>
              {status.state === "running" ? `running (${status.al_state ?? "INIT"})` : status.state}
            </StatusDot>
            {status.provenance ? <ProvenanceBadge provenance={status.provenance} /> : null}
            {status.target ? <span className="font-mono text-xs">{status.target}</span> : null}
            {status.elapsed_s > 0 ? (
              <span className="font-mono text-xs">{status.elapsed_s.toFixed(1)} s</span>
            ) : null}
          </span>
        }
        actions={
          running ? (
            <Button
              variant="danger"
              icon={<Square className="size-4" />}
              loading={busy}
              onClick={() => void doStop()}
            >
              Stop PDI slave
            </Button>
          ) : (
            <Button
              variant="primary"
              icon={<Play className="size-4" />}
              loading={busy}
              disabled={!canStart}
              onClick={() => {
                if (sim) {
                  void doStart();
                } else {
                  setConfirmOpen(true);
                }
              }}
            >
              Start PDI slave
            </Button>
          )
        }
      />

      {!running && hw && !canStart ? (
        <Callout
          tone={sim ? "info" : "warning"}
          title={sim ? "Simulated LAN9252 mode is off" : "Real ESC takeover requires opt-in"}
        >
          {sim ? (
            <>
              Start the server with <code>--sim-pdi</code> to run the PDI slave against a simulated
              LAN9252, or switch Target to Hardware spidev.
            </>
          ) : (
            <>
              Stop the slave device&apos;s own EtherCAT application first, then start{" "}
              <code>flowlab serve</code> with <code>--replace-slave-application</code>.
            </>
          )}
        </Callout>
      ) : null}

      {error ? <ErrorBox>{error}</ErrorBox> : null}
      {status.error ? <ErrorBox>{status.error}</ErrorBox> : null}

      <div className="grid gap-5 xl:grid-cols-[minmax(0,4.5fr)_minmax(0,7.5fr)]">
        <Panel title="Configuration">
          <DensityProvider value="compact">
            <div className="flex flex-col gap-3">
              <Field label="Target" as="group">
                <SegmentedControl
                  aria-label="Target"
                  value={sim ? "sim" : "hardware"}
                  disabled={running}
                  options={[
                    { value: "hardware", label: "Hardware spidev" },
                    { value: "sim", label: "Simulated LAN9252" },
                  ]}
                  onValueChange={(v) => setSimChoice(v === "sim")}
                />
              </Field>

              {!sim ? (
                <Field
                  label="SPI device"
                  as="group"
                  annotation={
                    spidevOptions.length > 0
                      ? `${spidevOptions.length} detected`
                      : "enter path or enable --sim-pdi"
                  }
                >
                  <div className="flex flex-col gap-1.5">
                    {spidevOptions.length > 0 ? (
                      <Select
                        aria-label="Detected spidev"
                        disabled={running}
                        value={spidevOptions.includes(device) ? device : ""}
                        placeholder="Custom path…"
                        options={spidevOptions.map((p) => ({ value: p, label: p }))}
                        onValueChange={(v) => v && setDeviceChoice(v)}
                      />
                    ) : null}
                    <Input
                      aria-label="spidev path"
                      className="font-mono"
                      disabled={running}
                      value={device}
                      onChange={(e) => setDeviceChoice(e.target.value)}
                      placeholder="/dev/spidev1.0"
                    />
                  </div>
                </Field>
              ) : null}

              <div className="grid gap-3 sm:grid-cols-2">
                <Field label="SPI clock" annotation="READ ≤ 30 MHz">
                  <NumberInput
                    aria-label="SPI clock MHz"
                    value={clockMhz}
                    unit="MHz"
                    min={1}
                    max={80}
                    disabled={running}
                    onValueChange={(v) => setClockMhz(Math.max(1, Math.min(80, v)))}
                  />
                </Field>
                <Field label="SPI mode" as="group">
                  <SegmentedControl
                    aria-label="SPI mode"
                    value={mode}
                    disabled={running}
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
                    disabled={running}
                    options={[
                      { value: "read", label: "READ" },
                      { value: "fast_read", label: "FAST READ" },
                    ]}
                    onValueChange={(v) => v && setReadInst(v as SlaveReadInstruction)}
                  />
                </Field>
                <Field label="Max mailbox">
                  <NumberInput
                    aria-label="Max mailbox bytes"
                    value={maxMailbox}
                    unit="B"
                    min={34}
                    max={2048}
                    disabled={running}
                    onValueChange={(v) =>
                      setMaxMailbox(Math.max(34, Math.min(2048, Math.round(v))))
                    }
                  />
                </Field>
              </div>

              <Field label="SPI bus tuning" as="group">
                <div className="flex flex-col gap-2">
                  <SegmentedControl
                    aria-label="SPI tuning preset"
                    value={tuningPreset}
                    disabled={running}
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
                          disabled={running}
                          onCheckedChange={() => toggleFlag(flag)}
                        >
                          {flag}
                        </ToggleChip>
                      ))}
                    </div>
                  ) : null}
                </div>
              </Field>

              <Field label="Mailbox buffer access" as="group">
                <SegmentedControl
                  aria-label="Mailbox buffer access"
                  value={messageOnly}
                  disabled={running}
                  options={[
                    { value: "message_only", label: "Message-only + last DWORD" },
                    { value: "whole", label: "Whole buffer" },
                  ]}
                  onValueChange={(v) => v && setMessageOnly(v)}
                />
              </Field>

              <div className="grid gap-3 sm:grid-cols-2">
                <Field label="Upload payload (from slave)">
                  <Select
                    aria-label="Upload payload"
                    value={uploadBytes}
                    disabled={running}
                    options={UPLOAD_OPTIONS}
                    onValueChange={(v) => v && setUploadBytes(v)}
                  />
                </Field>
                <Field label="Payload seed">
                  <NumberInput
                    aria-label="Payload seed"
                    value={uploadSeed}
                    min={0}
                    disabled={running || uploadBytes === "0"}
                    onValueChange={(v) => setUploadSeed(Math.max(0, Math.round(v)))}
                  />
                </Field>
              </div>

              <Field
                label="EoE bridge (veth pair)"
                as="group"
                annotation={
                  hw?.eoe_bridge_supported
                    ? "optional, needs CAP_NET_RAW / root"
                    : "Linux + --features slave only"
                }
              >
                <div className="grid gap-2 sm:grid-cols-2">
                  <Input
                    aria-label="EoE bridge peer"
                    className="font-mono"
                    disabled={running || !hw?.eoe_bridge_supported}
                    value={eoeBridge}
                    onChange={(e) => setEoeBridge(e.target.value)}
                    placeholder="peer (e.g. fleoe0b)"
                    list="flowlab-net-ifaces"
                  />
                  <Input
                    aria-label="EoE host interface"
                    className="font-mono"
                    disabled={running || !hw?.eoe_bridge_supported}
                    value={eoeHost}
                    onChange={(e) => setEoeHost(e.target.value)}
                    placeholder="host (e.g. fleoe0)"
                    list="flowlab-net-ifaces"
                  />
                  <datalist id="flowlab-net-ifaces">
                    {netOptions.map((n) => (
                      <option key={n} value={n} />
                    ))}
                  </datalist>
                </div>
              </Field>
            </div>
          </DensityProvider>
        </Panel>

        <div className="flex flex-col gap-5">
          <ReadoutStrip
            items={[
              { label: "AL", value: status.al_state ?? "–" },
              {
                label: "chip",
                value: status.link
                  ? `${hex(status.link.chip_id)} rev ${status.link.revision}`
                  : null,
              },
              { label: "polls/s", value: latest ? count(Math.round(latest.polls_s)) : null },
              { label: "in", value: latest ? byteRate(latest.bytes_in_s) : null },
              { label: "out", value: latest ? byteRate(latest.bytes_out_s) : null },
              {
                label: "SM0/SM1",
                value: status.mailboxes
                  ? `${status.mailboxes.out_len}/${status.mailboxes.in_len} B`
                  : null,
              },
            ]}
          />

          <MetricGrid compact>
            <Metric
              label="AL state"
              value={status.al_state ?? "–"}
              sub={
                status.al_error !== null
                  ? `error ${hex(status.al_error)}`
                  : status.mailboxes
                    ? `SM0 ${hex(status.mailboxes.out_start)} · SM1 ${hex(status.mailboxes.in_start)}`
                    : "mailboxes closed"
              }
              verdict={status.al_error !== null ? "defect" : undefined}
            />
            <Metric
              label="Polls"
              value={count(status.stats.polls)}
              sub={latest ? `${count(Math.round(latest.polls_s))} polls/s` : undefined}
            />
            <Metric
              label="Messages in (SM0)"
              value={count(status.stats.messages_in)}
              sub={bytes(status.stats.bytes_in)}
            />
            <Metric
              label="Messages out (SM1)"
              value={count(status.stats.messages_out)}
              sub={bytes(status.stats.bytes_out)}
            />
          </MetricGrid>

          {status.bridge ? (
            <Panel
              title={`EoE bridge (${status.bridge.bridge_iface} ↔ ${status.bridge.host_iface})`}
            >
              <MetricGrid compact>
                <Metric
                  label="Frames to host"
                  value={count(status.bridge.to_host)}
                  sub={bytes(status.bridge.to_host_bytes)}
                />
                <Metric
                  label="Frames to master"
                  value={count(status.bridge.to_master)}
                  sub={bytes(status.bridge.to_master_bytes)}
                />
                <Metric label="EoE Set IP" value={status.bridge.last_set_ip ?? "waiting"} />
              </MetricGrid>
            </Panel>
          ) : null}

          <div className="grid gap-4 lg:grid-cols-2">
            <Panel title="Mailbox throughput">
              <LineChart
                label="Mailbox throughput over time"
                series={throughputSeries}
                xLabel="elapsed (s)"
                yLabel="kB/s"
                variant="fluid"
                height={190}
                showLegend
              />
            </Panel>
            <Panel title="Mailbox message rate">
              <LineChart
                label="Mailbox messages per second"
                series={msgSeries}
                xLabel="elapsed (s)"
                yLabel="msg/s"
                variant="fluid"
                height={190}
                showLegend
              />
            </Panel>
          </div>

          <div className="grid gap-4 lg:grid-cols-2">
            <Panel title={`AL state history (${status.al_events.length})`}>
              <Table
                columns={AL_EVENT_COLUMNS}
                rows={status.al_events}
                rowKey={(e) => `${e.t_s}-${e.state}-${e.error ?? "ok"}`}
                empty="Start the PDI slave to record AL state transitions."
              />
            </Panel>
            <Panel title={`Received payloads (${status.received.length})`}>
              <Table
                columns={RECEIVED_COLUMNS}
                rows={status.received}
                rowKey={(r) => `${r.t_s}-${r.protocol}-${r.sha256}-${r.bytes}`}
                empty="No FoE, CoE, VoE, or EoE payloads received yet."
              />
            </Panel>
          </div>
        </div>
      </div>

      <ConfirmDialog
        open={confirmOpen}
        onOpenChange={setConfirmOpen}
        title="Take over EtherCAT Slave Controller?"
        description={
          <>
            FlowLab will open <code>{device}</code> and serve AL Status, SyncManagers 0/1, and the
            mailbox buffers in place of the device&apos;s own slave application. Ensure the product
            slave application is stopped before continuing.
          </>
        }
        confirmLabel="Start PDI slave"
        loading={busy}
        onConfirm={() => void doStart()}
      />
    </div>
  );
}
