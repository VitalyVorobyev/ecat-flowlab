import {
  Badge,
  Button,
  Callout,
  ConfirmDialog,
  Disclosure,
  ErrorBox,
  Field,
  Input,
  NumberInput,
  PageHeader,
  Panel,
  SegmentedControl,
  Select,
  StatusDot,
  Table,
  toast,
  type Column,
} from "@vitavision/ui";
import { RefreshCw, Wrench } from "lucide-react";
import { useState } from "react";

import {
  api,
  type Capabilities,
  errorMessage,
  type PhyPortView,
  type PhyRegisterView,
  type PhyReport,
} from "../api";
import { ProvenanceBadge } from "../components/ProvenanceBadge";
import { hex } from "../lib/format";

function binary16(v: number): string {
  const s = (v & 0xffff).toString(2).padStart(16, "0");
  return `${s.slice(0, 4)} ${s.slice(4, 8)} ${s.slice(8, 12)} ${s.slice(12, 16)}`;
}

function decodeRegHints(r: PhyRegisterView): string {
  const v = r.value;
  switch (r.reg) {
    case 0:
      return [
        v & (1 << 15) ? "reset" : null,
        v & (1 << 12) ? "autoneg on" : "autoneg off",
        v & (1 << 13) ? "100M" : "10M",
        v & (1 << 8) ? "full duplex" : "half duplex",
      ]
        .filter(Boolean)
        .join(" · ");
    case 1:
      return [
        v & (1 << 2) ? "link up" : "link down",
        v & (1 << 5) ? "autoneg done" : "autoneg incomplete",
      ].join(" · ");
    case 18:
      return `mode ${(v >> 5) & 0x7} · addr ${v & 0x1f}`;
    case 27:
      return [
        v & (1 << 15) ? "reg override" : "strap",
        v & (1 << 14) ? "auto-MDIX" : "fixed MDI",
        v & (1 << 13) ? "MDI-X" : "MDI",
      ].join(" · ");
    case 31: {
      const sp = (v >> 2) & 0x7;
      return `speed/duplex code 0b${sp.toString(2).padStart(3, "0")}`;
    }
    default:
      return "–";
  }
}

const REG_COLUMNS: Column<PhyRegisterView>[] = [
  {
    key: "reg",
    header: "Reg",
    numeric: true,
    width: "4rem",
    cell: (r) => <span className="font-mono">r{r.reg}</span>,
  },
  { key: "name", header: "Register", cell: (r) => r.name },
  {
    key: "hex",
    header: "Hex",
    width: "6.5rem",
    cell: (r) => <span className="font-mono font-medium">{hex(r.value)}</span>,
  },
  {
    key: "bin",
    header: "Bits [15:0]",
    cell: (r) => <span className="font-mono text-xs text-fg-muted">{binary16(r.value)}</span>,
  },
  {
    key: "notes",
    header: "Decoded",
    cell: (r) => <span className="text-xs text-fg-muted">{decodeRegHints(r)}</span>,
  },
];

function parseHexOrDec(text: string): number | null {
  const s = text.trim();
  if (!s) return null;
  const n = /^0x/i.test(s) ? Number.parseInt(s.slice(2), 16) : Number(s);
  if (!Number.isInteger(n) || n < 0 || n > 0xffff) return null;
  return n;
}

type PendingAction =
  | { kind: "autoneg"; phy: number }
  | { kind: "write"; phy: number; reg: number; value: number };

/**
 * Inspect and configure the LAN9252 Ethernet PHYs over the ESC's MII management interface
 * (`flowlab pdi-phy`).
 */
export function PhyInspector({
  capabilities,
  slaveRunning,
  onStopSlave,
}: {
  capabilities: Capabilities | null;
  slaveRunning: boolean;
  onStopSlave: () => Promise<void>;
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

  const [clockMhz, setClockMhz] = useState(10);
  const [mode, setMode] = useState("0");
  const [selectedPhy, setSelectedPhy] = useState<number>(0);

  const [writePhy, setWritePhy] = useState(0);
  const [writeReg, setWriteReg] = useState(0);
  const [writeVal, setWriteVal] = useState("0x3300");

  const [report, setReport] = useState<PhyReport | null>(null);
  const [busy, setBusy] = useState(false);
  const [stopping, setStopping] = useState(false);
  const [pending, setPending] = useState<PendingAction | null>(null);
  const [error, setError] = useState<string | null>(null);

  const canWrite = sim ? Boolean(hw?.sim_pdi) : Boolean(hw?.allow_phy_writes);
  const canRead = sim ? Boolean(hw?.sim_pdi) : Boolean(device.trim());

  const readPhys = async () => {
    setBusy(true);
    setError(null);
    try {
      const r = await api.phyRead({
        device: sim ? null : device,
        sim,
        clock_hz: Math.round(clockMhz * 1e6),
        mode: Number(mode),
        chip_id: null,
        phys: [0, 1, 2, 3],
      });
      setReport(r);
      const firstPresent = r.ports.find((p) => p.present);
      if (firstPresent && !r.ports.some((p) => p.phy === selectedPhy && p.present)) {
        setSelectedPhy(firstPresent.phy);
      }
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const executeWrite = async (action: PendingAction) => {
    setBusy(true);
    setError(null);
    try {
      const base = {
        device: sim ? null : device,
        sim,
        clock_hz: Math.round(clockMhz * 1e6),
        mode: Number(mode),
        chip_id: null,
        phys: [0, 1, 2, 3],
      };
      const r =
        action.kind === "autoneg"
          ? await api.phyAutoneg({ ...base, target_phys: [action.phy] })
          : await api.phyWrite({ ...base, phy: action.phy, reg: action.reg, value: action.value });
      setReport(r);
      setPending(null);
      toast({
        tone: "success",
        title:
          action.kind === "autoneg"
            ? `PHY ${action.phy} auto-negotiation restarted`
            : `PHY ${action.phy} r${action.reg} := ${hex(action.value)}`,
      });
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

  const triggerWrite = () => {
    const parsed = parseHexOrDec(writeVal);
    if (parsed === null) {
      setError(`Invalid register value "${writeVal}" (expected 0–65535 or 0x0000–0xFFFF)`);
      return;
    }
    const action: PendingAction = { kind: "write", phy: writePhy, reg: writeReg, value: parsed };
    if (sim) {
      void executeWrite(action);
    } else {
      setPending(action);
    }
  };

  const triggerAutoneg = (phy: number) => {
    const action: PendingAction = { kind: "autoneg", phy };
    if (sim) {
      void executeWrite(action);
    } else {
      setPending(action);
    }
  };

  const presentPorts = report?.ports.filter((p) => p.present) ?? [];
  const absentPorts = report?.ports.filter((p) => !p.present) ?? [];
  const activePort: PhyPortView | null =
    presentPorts.find((p) => p.phy === selectedPhy) ?? presentPorts[0] ?? null;

  return (
    <div className="mx-auto flex max-w-[110rem] flex-col gap-5 p-6">
      <PageHeader
        title="PHY Inspector"
        meta={
          report ? (
            <span className="flex flex-wrap items-center gap-x-3 gap-y-1">
              <ProvenanceBadge provenance={report.provenance} />
              <span className="font-mono text-xs">{report.target}</span>
              <span className="font-mono text-xs">
                LAN9252 rev {report.link.revision} · AL {report.link.al_state}
              </span>
            </span>
          ) : (
            "Read and configure the LAN9252 Ethernet PHYs over the ESC MII management interface."
          )
        }
        actions={
          <Button
            variant="primary"
            icon={<RefreshCw className="size-4" />}
            loading={busy}
            disabled={slaveRunning || !canRead}
            onClick={() => void readPhys()}
          >
            Read PHYs
          </Button>
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
          PHY MII reads share the LAN9252 SPI bus. Stop the running PDI slave before inspecting or
          modifying PHY registers.
        </Callout>
      ) : null}

      {!sim && hw && !hw.allow_phy_writes ? (
        <Callout tone="info" title="PHY writes are read-only">
          Start <code>flowlab serve</code> with <code>--allow-phy-writes</code> to enable
          auto-negotiation and register writes on real hardware.
        </Callout>
      ) : null}

      {error ? <ErrorBox>{error}</ErrorBox> : null}

      <Panel title="Bus target">
        <div className="flex flex-wrap items-end gap-4">
          <Field label="Target" as="group">
            <SegmentedControl
              aria-label="PHY target"
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
                className="w-48 font-mono"
                value={device}
                onChange={(e) => setDeviceChoice(e.target.value)}
                placeholder="/dev/spidev1.0"
                list="phy-spidev-list"
              />
              <datalist id="phy-spidev-list">
                {(hw?.spidev_devices ?? []).map((p) => (
                  <option key={p} value={p} />
                ))}
              </datalist>
            </Field>
          ) : null}
          <Field label="SPI clock">
            <NumberInput
              aria-label="SPI clock MHz"
              className="w-28"
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
      </Panel>

      {report ? (
        <>
          <div className="grid gap-4 md:grid-cols-2">
            {presentPorts.map((p) => (
              <Panel
                key={p.phy}
                title={`PHY ${p.phy}`}
                actions={
                  <Button
                    size="sm"
                    variant="secondary"
                    icon={<Wrench className="size-3.5" />}
                    disabled={slaveRunning || busy || !canWrite}
                    onClick={() => triggerAutoneg(p.phy)}
                  >
                    Enable Auto-Neg &amp; Auto-MDIX
                  </Button>
                }
              >
                <div className="flex flex-col gap-3">
                  <div className="flex flex-wrap items-center gap-3">
                    <StatusDot tone={p.link_up ? "normal" : "defect"}>
                      {p.link_up ? "Link up" : "Link down"}
                    </StatusDot>
                    <Badge tone="neutral">
                      {p.speed_100 ? "100 Mbit/s" : "10 Mbit/s"}{" "}
                      {p.full_duplex ? "full duplex" : "half duplex"}
                    </Badge>
                    <Badge tone={p.autoneg_enabled && p.autoneg_done ? "normal" : "warning"}>
                      {!p.autoneg_enabled
                        ? "Auto-neg off"
                        : p.autoneg_done
                          ? "Auto-neg done"
                          : "Auto-neg pending"}
                    </Badge>
                    <Badge tone="neutral">
                      {p.auto_mdix ? "Auto-MDIX" : p.mdix ? "MDI-X" : "MDI"}
                      {p.mdix_override ? " (reg 27)" : ""}
                    </Badge>
                    <Badge tone="neutral">mode {p.mode.toString(2).padStart(3, "0")}</Badge>
                  </div>
                  <p className="font-mono text-xs text-fg-muted">{p.summary}</p>
                </div>
              </Panel>
            ))}
          </div>

          {absentPorts.length > 0 ? (
            <p className="text-xs text-fg-muted">
              No PHY answered at MII address{absentPorts.length > 1 ? "es" : ""}{" "}
              {absentPorts.map((p) => p.phy).join(", ")}.
            </p>
          ) : null}

          {activePort ? (
            <Panel
              title={`PHY ${activePort.phy} registers`}
              actions={
                presentPorts.length > 1 ? (
                  <Select
                    aria-label="Inspect PHY registers"
                    className="w-32"
                    value={String(activePort.phy)}
                    options={presentPorts.map((p) => ({
                      value: String(p.phy),
                      label: `PHY ${p.phy}`,
                    }))}
                    onValueChange={(v) => v && setSelectedPhy(Number(v))}
                  />
                ) : undefined
              }
            >
              <Table columns={REG_COLUMNS} rows={activePort.registers} rowKey={(r) => r.reg} />
            </Panel>
          ) : null}

          <Disclosure summary="Write raw PHY register">
            <div className="flex flex-wrap items-end gap-3 pt-2">
              <Field label="PHY (0–31)">
                <NumberInput
                  aria-label="Write PHY address"
                  className="w-24"
                  value={writePhy}
                  min={0}
                  max={31}
                  onValueChange={(v) => setWritePhy(Math.max(0, Math.min(31, Math.round(v))))}
                />
              </Field>
              <Field label="Register (0–31)">
                <NumberInput
                  aria-label="Write register index"
                  className="w-24"
                  value={writeReg}
                  min={0}
                  max={31}
                  onValueChange={(v) => setWriteReg(Math.max(0, Math.min(31, Math.round(v))))}
                />
              </Field>
              <Field label="Value (hex or dec)">
                <Input
                  aria-label="Write register value"
                  className="w-32 font-mono"
                  value={writeVal}
                  onChange={(e) => setWriteVal(e.target.value)}
                  placeholder="0x3300"
                />
              </Field>
              <Button
                variant="secondary"
                disabled={slaveRunning || busy || !canWrite}
                onClick={triggerWrite}
              >
                Write register
              </Button>
            </div>
          </Disclosure>
        </>
      ) : (
        <Callout tone="info" title="No PHY readout yet">
          Click <strong>Read PHYs</strong> to inspect link state, speed, duplex, auto-negotiation,
          and crossover strap/override registers on the LAN9252.
        </Callout>
      )}

      <ConfirmDialog
        open={pending !== null}
        onOpenChange={(open) => {
          if (!open) setPending(null);
        }}
        title="Modify LAN9252 PHY registers?"
        description={
          pending?.kind === "autoneg" ? (
            <>
              Enable auto-negotiation and auto-MDIX on PHY {pending.phy} (registers 27, 18, and 0)
              and wait 3 s for link renegotiation. Changes remain until reset or slave stack
              re-initialization.
            </>
          ) : pending ? (
            <>
              Write {hex(pending.value)} to PHY {pending.phy} register {pending.reg}.
            </>
          ) : null
        }
        confirmLabel="Apply PHY change"
        loading={busy}
        onConfirm={() => pending && void executeWrite(pending)}
      />
    </div>
  );
}
