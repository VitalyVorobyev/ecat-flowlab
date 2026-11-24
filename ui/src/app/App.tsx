import { ErrorBox, StatusDot, ThemeToggle, toast } from "@vitavision/ui";
import { AppShell, FileDrop, NavRail, NavRailItem, StatusBar } from "@vitavision/workbench";
import { Activity, ArrowLeftRight, Cable, Cpu, FlaskConical, Ruler, Timer } from "lucide-react";
import { useCallback, useEffect, useState } from "react";

import {
  api,
  type Capabilities,
  type CaptureInfo,
  errorMessage,
  type FrameFilter,
  NO_FILTER,
  type PdiValuesView,
  type SlaveStatus,
} from "../api";
import { count, ns } from "../lib/format";
import { useAsync } from "../lib/useAsync";
import { useScenario } from "../scenario/useScenario";
import { ANALYSIS_TABS, Analysis, type AnalysisTab, needsCapture } from "../screens/Analysis";
import { CAPTURE_ACCEPT } from "../screens/Captures";
import { type FramesTarget } from "../screens/Frames";
import { PhyInspector } from "../screens/PhyInspector";
import { Planner } from "../screens/Planner";
import { Scenario } from "../screens/Scenario";
import { SlaveControl } from "../screens/SlaveControl";
import { SpiBench } from "../screens/SpiBench";
import { ThroughputLab } from "../screens/ThroughputLab";

export type View =
  | "slave"
  | "throughput"
  | "phy"
  | "spi-bench"
  | "analysis"
  | "simulate"
  | "planner";

const VIEWS: readonly View[] = [
  "slave",
  "throughput",
  "phy",
  "spi-bench",
  "analysis",
  "simulate",
  "planner",
];

const INITIAL_SLAVE_STATUS: SlaveStatus = {
  state: "idle",
  provenance: null,
  target: null,
  spi_tuning: null,
  clock_hz: null,
  max_mailbox_bytes: null,
  message_only: null,
  elapsed_s: 0,
  link: null,
  al_state: null,
  al_error: null,
  mailboxes: null,
  stats: {
    polls: 0,
    al_requests: 0,
    messages_in: 0,
    messages_out: 0,
    bytes_in: 0,
    bytes_out: 0,
    status_reads: 0,
  },
  bridge: null,
  series: [],
  al_events: [],
  received: [],
  error: null,
};

const DIRECTIONS = {
  mac_local_bit: "directions: MAC bit",
  wkc_heuristic: "directions: WKC heuristic",
  unknown: "directions: unknown",
} as const;

function parseHash(): { view: View; tab: AnalysisTab | null } | null {
  const raw = window.location.hash.replace(/^#\/?/, "");
  if (!raw) return null;
  if (raw.startsWith("analysis/")) {
    const sub = raw.slice("analysis/".length) as AnalysisTab;
    return { view: "analysis", tab: ANALYSIS_TABS.includes(sub) ? sub : "captures" };
  }
  if (VIEWS.includes(raw as View)) {
    return { view: raw as View, tab: null };
  }
  if (ANALYSIS_TABS.includes(raw as AnalysisTab)) {
    return { view: "analysis", tab: raw as AnalysisTab };
  }
  return null;
}

function writeHash(view: View, tab: AnalysisTab): void {
  const target = view === "analysis" ? `#analysis/${tab}` : `#${view}`;
  try {
    window.history.replaceState(null, "", target);
  } catch {
    // Sandboxed frames disallow replaceState.
  }
}

export function App() {
  const initialHash = parseHash();
  const [capabilities, setCapabilities] = useState<Capabilities | null>(null);
  const [captures, setCaptures] = useState<CaptureInfo[]>([]);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [view, setViewState] = useState<View>(() => initialHash?.view ?? "slave");
  const [analysisTab, setAnalysisTabState] = useState<AnalysisTab>(
    () => initialHash?.tab ?? "captures",
  );
  const [slaveStatus, setSlaveStatus] = useState<SlaveStatus>(INITIAL_SLAVE_STATUS);
  const [busy, setBusy] = useState(false);
  const scenario = useScenario();
  const [streamId, setStreamId] = useState(0);
  const [frameFilter, setFrameFilter] = useState<FrameFilter>(NO_FILTER);
  const [frameTarget, setFrameTarget] = useState<FramesTarget | null>(null);
  const [version, setVersion] = useState<string | null>(null);
  const [reloads, setReloads] = useState(0);

  const navigate = useCallback((nextView: View, nextTab?: AnalysisTab) => {
    setViewState(nextView);
    setAnalysisTabState((prevTab) => {
      const resolved = nextTab ?? prevTab;
      writeHash(nextView, resolved);
      return resolved;
    });
  }, []);

  const setAnalysisTab = useCallback(
    (tab: AnalysisTab) => {
      navigate("analysis", tab);
    },
    [navigate],
  );

  useEffect(() => {
    const hasExplicitHash = parseHash() !== null;
    api.capabilities().then(
      (caps) => {
        setCapabilities(caps);
        setVersion(caps.version);
        if (!hasExplicitHash) {
          setViewState((cur) => {
            if (cur === "analysis") return cur;
            const next = caps.default_view === "slave" ? "slave" : "analysis";
            writeHash(next, "captures");
            return next;
          });
        }
      },
      (e: unknown) =>
        toast({
          tone: "error",
          title: "FlowLab server unreachable",
          description: errorMessage(e),
        }),
    );
    api.slaveStatus().then(setSlaveStatus, () => undefined);
    api.listCaptures().then(
      (list) => {
        setCaptures(list);
        const last = list[list.length - 1];
        if (last) {
          setSelectedId(last.id);
          if (!hasExplicitHash) {
            navigate("analysis", "overview");
          }
        }
      },
      () => undefined,
    );
  }, [navigate]);

  useEffect(() => {
    if (slaveStatus.state !== "running") return undefined;
    const t = window.setInterval(() => {
      api.slaveStatus().then(setSlaveStatus, () => undefined);
    }, 1000);
    return () => window.clearInterval(t);
  }, [slaveStatus.state]);

  const selected = captures.find((c) => c.id === selectedId) ?? null;
  const report = useAsync(
    () => (selectedId === null ? Promise.resolve(null) : api.report(selectedId)),
    [selectedId, reloads],
  );
  const hasSimulation = selected?.has_simulation ?? false;
  const simulation = useAsync(
    () =>
      selectedId === null || !hasSimulation ? Promise.resolve(null) : api.simulation(selectedId),
    [selectedId, hasSimulation],
  );

  const selectCapture = useCallback(
    (id: number) => {
      setSelectedId(id);
      setStreamId(0);
      setFrameFilter(NO_FILTER);
      setFrameTarget(null);
      navigate("analysis", "overview");
    },
    [navigate],
  );

  const added = useCallback(
    (info: CaptureInfo) => {
      setCaptures((list) => [...list, info]);
      setSelectedId(info.id);
      setStreamId(0);
      setFrameFilter(NO_FILTER);
      setFrameTarget(null);
      navigate("analysis", info.has_simulation ? "simulation" : "overview");
      toast({
        tone: info.defects > 0 ? "warn" : "success",
        title: `Loaded ${info.name}`,
        description: `${count(info.frames)} frames · ${info.streams} cyclic streams · ${info.defects} defects`,
      });
    },
    [navigate],
  );

  const runCaptureLoad = async (title: string, work: () => Promise<CaptureInfo>) => {
    setBusy(true);
    try {
      added(await work());
    } catch (e) {
      toast({ tone: "error", title, description: errorMessage(e) });
    } finally {
      setBusy(false);
    }
  };

  const onFiles = (files: File[]) => {
    void (async () => {
      for (const f of files) await runCaptureLoad(`Could not open ${f.name}`, () => api.upload(f));
    })();
  };

  const removeCapture = async (id: number) => {
    try {
      await api.remove(id);
      setCaptures((list) => list.filter((c) => c.id !== id));
      if (id === selectedId) {
        setSelectedId(null);
        if (needsCapture(analysisTab)) {
          setAnalysisTab("captures");
        }
      }
    } catch (e) {
      toast({ tone: "error", title: "Could not close the capture", description: errorMessage(e) });
    }
  };

  const updated = (info: CaptureInfo) => {
    setCaptures((list) => list.map((c) => (c.id === info.id ? info : c)));
    setReloads((n) => n + 1);
  };

  const attachEni = async (file: File) => {
    if (selectedId === null) return;
    try {
      const info = await api.attachEni(selectedId, file);
      updated(info);
      toast({ tone: "success", title: `Network configuration from ${file.name} applied` });
    } catch (e) {
      toast({ tone: "error", title: `Could not apply ${file.name}`, description: errorMessage(e) });
    }
  };

  const detachEni = async () => {
    if (selectedId === null) return;
    try {
      updated(await api.detachEni(selectedId));
    } catch (e) {
      toast({
        tone: "error",
        title: "Could not remove the configuration",
        description: errorMessage(e),
      });
    }
  };

  const openFrame = (no: number) => {
    setFrameFilter({ ...NO_FILTER, include_other: true });
    setFrameTarget({ no, nonce: Date.now() });
    navigate("analysis", "frames");
  };

  const stopSlave = async () => {
    try {
      const next = await api.slaveStop();
      setSlaveStatus(next);
      toast({ tone: "info", title: "PDI slave stopped", description: "AL Status set to INIT" });
    } catch (e) {
      toast({ tone: "error", title: "Could not stop PDI slave", description: errorMessage(e) });
    }
  };

  const applyPdiFit = (pdi: PdiValuesView) => {
    const base = scenario.spec ?? scenario.presets[0]?.spec;
    if (!base || base.slaves.length === 0) {
      toast({
        tone: "error",
        title: "Simulate scenario not ready yet",
        description: "Open Simulate once and retry.",
      });
      return;
    }
    const idx = base.slaves.findIndex((s) => s.mailbox_out_bytes > 0 || s.mailbox_in_bytes > 0);
    const targetIdx = idx >= 0 ? idx : 0;
    const targetSlave = base.slaves[targetIdx];
    if (!targetSlave) return;
    const nextSlaves = base.slaves.map((s, i) =>
      i === targetIdx
        ? {
            ...s,
            pdi: {
              ...s.pdi,
              read_bps: pdi.read_bps,
              write_bps: pdi.write_bps,
              access_overhead_ns: pdi.access_overhead_ns,
            },
          }
        : s,
    );
    scenario.setSpec({ ...base, slaves: nextSlaves });
    navigate("simulate");
    toast({
      tone: "success",
      title: `Applied SPI PDI fit to ${targetSlave.name}`,
      description: `${(pdi.read_bps / 1e6).toFixed(2)} Mbit/s read · ${(pdi.write_bps / 1e6).toFixed(2)} Mbit/s write · ${pdi.access_overhead_ns} ns overhead`,
    });
  };

  const slaveTone =
    slaveStatus.state === "running"
      ? "normal"
      : slaveStatus.state === "failed"
        ? "defect"
        : slaveStatus.state === "stopped"
          ? "neutral"
          : "unlabeled";

  const platformTag = capabilities?.platform
    ? [capabilities.platform.hostname, capabilities.platform.arch].filter(Boolean).join(" · ")
    : null;

  const header = (
    <div className="flex h-12 items-center gap-3 border-b border-line bg-surface px-4">
      <div className="flex items-baseline gap-2">
        <span className="text-sm font-semibold tracking-tight text-fg">ECAT FlowLab</span>
        {platformTag ? (
          <span className="hidden font-mono text-[11px] text-fg-subtle sm:inline">
            {platformTag}
          </span>
        ) : null}
      </div>
      <button
        type="button"
        onClick={() => navigate("slave")}
        className="ml-2 rounded-md border border-line bg-raised px-2.5 py-1 text-xs transition-colors hover:bg-surface"
        title="Open PDI Slave control"
      >
        <StatusDot tone={slaveTone}>
          {slaveStatus.state === "running"
            ? `PDI ${slaveStatus.al_state ?? "INIT"}`
            : `PDI ${slaveStatus.state}`}
        </StatusDot>
      </button>
      <div className="ml-auto flex items-center gap-2">
        <FileDrop
          overlay
          accept={CAPTURE_ACCEPT}
          onFiles={onFiles}
          disabled={busy}
          buttonLabel="Open capture…"
          overlayMessage="Drop a pcap or pcapng to analyze it"
        />
        <ThemeToggle />
      </div>
    </div>
  );

  const rail = (
    <NavRail value={view} onValueChange={(v) => navigate(v as View)} labels="visible">
      <NavRailItem value="slave" label="Slave" icon={<Cpu />} />
      <NavRailItem value="throughput" label="Throughput" icon={<ArrowLeftRight />} />
      <NavRailItem value="phy" label="PHY" icon={<Cable />} />
      <NavRailItem value="spi-bench" label="SPI Bench" icon={<Timer />} />
      <NavRailItem value="analysis" label="Analysis" icon={<Activity />} />
      <NavRailItem value="simulate" label="Simulate" icon={<FlaskConical />} />
      <NavRailItem value="planner" label="Planner" icon={<Ruler />} />
    </NavRail>
  );

  let main;
  if (view === "slave") {
    main = (
      <SlaveControl
        capabilities={capabilities}
        status={slaveStatus}
        onStatusChange={setSlaveStatus}
      />
    );
  } else if (view === "throughput") {
    main = <ThroughputLab />;
  } else if (view === "phy") {
    main = (
      <PhyInspector
        capabilities={capabilities}
        slaveRunning={slaveStatus.state === "running"}
        onStopSlave={stopSlave}
      />
    );
  } else if (view === "spi-bench") {
    main = (
      <SpiBench
        capabilities={capabilities}
        slaveRunning={slaveStatus.state === "running"}
        onStopSlave={stopSlave}
        onApplyPdiFit={applyPdiFit}
      />
    );
  } else if (view === "simulate") {
    main = <Scenario scenario={scenario} onCreated={added} />;
  } else if (view === "planner") {
    main = <Planner />;
  } else if (view === "analysis") {
    main = (
      <Analysis
        tab={analysisTab}
        onTabChange={setAnalysisTab}
        captures={captures}
        selectedId={selectedId}
        report={report}
        simulation={simulation}
        busy={busy}
        localPathsEnabled={capabilities?.local_paths ?? true}
        streamId={streamId}
        onStreamChange={setStreamId}
        frameFilter={frameFilter}
        onFilterChange={setFrameFilter}
        frameTarget={frameTarget}
        onSelectCapture={selectCapture}
        onRemoveCapture={(id) => void removeCapture(id)}
        onFiles={onFiles}
        onOpenPath={(p) => void runCaptureLoad("Could not open the file", () => api.openPath(p))}
        onSavedLive={(info) => {
          setCaptures((list) => [...list, info]);
          toast({
            tone: "success",
            title: `Saved ${info.name}`,
            description: `${count(info.frames)} frames, loaded for analysis`,
          });
        }}
        onAttachEni={(f) => void attachEni(f)}
        onDetachEni={() => void detachEni()}
        onOpenFrame={openFrame}
        onEditSimulation={(spec) => {
          scenario.setSpec(spec);
          navigate("simulate");
        }}
      />
    );
  } else {
    main = <ErrorBox className="m-6">Unknown view.</ErrorBox>;
  }

  const r = report.data;
  const latestRate = slaveStatus.series.at(-1);
  const bottom = (
    <StatusBar
      start={[
        {
          label: "slave",
          value:
            slaveStatus.state === "running"
              ? `${slaveStatus.al_state ?? "INIT"}${latestRate ? ` · ${count(Math.round(latestRate.polls_s))} polls/s` : ""}`
              : slaveStatus.state,
        },
        { value: selected ? `${selected.provenance.toUpperCase()} · ${selected.name}` : null },
        { label: "frames", value: selected ? count(selected.frames) : null },
        { label: "span", value: selected ? ns(selected.duration_ns) : null },
      ]}
      end={[
        { value: r ? DIRECTIONS[r.summary.direction_mode] : null },
        { value: version ? `flowlab ${version}` : "server offline" },
      ]}
    />
  );

  return (
    <AppShell
      header={header}
      rail={rail}
      main={<div className="h-full min-h-0 overflow-y-auto">{main}</div>}
      bottom={bottom}
    />
  );
}
