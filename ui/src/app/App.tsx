import { Button, ErrorBox, Select, SkeletonRows, ThemeToggle, toast } from "@vitavision/ui";
import { AppShell, FileDrop, NavRail, NavRailItem, StatusBar } from "@vitavision/workbench";
import {
  Activity,
  ChartScatter,
  Columns2,
  FlaskConical,
  FolderOpen,
  Gauge,
  Mail,
  Network,
  Rows3,
  Ruler,
  Sparkles,
  Waypoints,
} from "lucide-react";
import { useCallback, useEffect, useState } from "react";

import { api, type CaptureInfo, errorMessage, type FrameFilter, NO_FILTER } from "../api";
import { ProvenanceBadge } from "../components/ProvenanceBadge";
import { SimulateDialog, SynthDialog } from "../components/SynthDialog";
import { count, ns } from "../lib/format";
import { useAsync } from "../lib/useAsync";
import { CAPTURE_ACCEPT, Captures } from "../screens/Captures";
import { Compare } from "../screens/Compare";
import { Frames, type FramesTarget } from "../screens/Frames";
import { Mailbox } from "../screens/Mailbox";
import { Overview } from "../screens/Overview";
import { Planner } from "../screens/Planner";
import { Simulation } from "../screens/Simulation";
import { Stations } from "../screens/Stations";
import { Study } from "../screens/Study";
import { Timing } from "../screens/Timing";

export type View =
  | "captures"
  | "overview"
  | "timing"
  | "frames"
  | "stations"
  | "mailbox"
  | "simulation"
  | "planner"
  | "study"
  | "compare";

const DIRECTIONS = {
  mac_local_bit: "directions: MAC bit",
  wkc_heuristic: "directions: WKC heuristic",
  unknown: "directions: unknown",
} as const;

export function App() {
  const [captures, setCaptures] = useState<CaptureInfo[]>([]);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [view, setView] = useState<View>("captures");
  const [busy, setBusy] = useState(false);
  const [synthOpen, setSynthOpen] = useState(false);
  const [simOpen, setSimOpen] = useState(false);
  const [streamId, setStreamId] = useState(0);
  const [frameFilter, setFrameFilter] = useState<FrameFilter>(NO_FILTER);
  const [frameTarget, setFrameTarget] = useState<FramesTarget | null>(null);
  const [version, setVersion] = useState<string | null>(null);
  const [reloads, setReloads] = useState(0);

  useEffect(() => {
    api.health().then(
      (h) => setVersion(h.version),
      (e: unknown) =>
        toast({ tone: "error", title: "FlowLab server unreachable", description: errorMessage(e) }),
    );
    api.listCaptures().then(
      (list) => {
        setCaptures(list);
        const last = list[list.length - 1];
        if (last) {
          setSelectedId(last.id);
          setView("overview");
        }
      },
      () => undefined,
    );
  }, []);

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

  const select = useCallback((id: number) => {
    setSelectedId(id);
    setStreamId(0);
    setFrameFilter(NO_FILTER);
    setFrameTarget(null);
    setView("overview");
  }, []);

  const added = useCallback(
    (info: CaptureInfo) => {
      setCaptures((list) => [...list, info]);
      select(info.id);
      if (info.has_simulation) setView("simulation");
      toast({
        tone: info.defects > 0 ? "warn" : "success",
        title: `Loaded ${info.name}`,
        description: `${count(info.frames)} frames · ${info.streams} cyclic streams · ${info.defects} defects`,
      });
    },
    [select],
  );

  const run = async (title: string, work: () => Promise<CaptureInfo>) => {
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
      for (const f of files) await run(`Could not open ${f.name}`, () => api.upload(f));
    })();
  };

  const remove = async (id: number) => {
    try {
      await api.remove(id);
      setCaptures((list) => list.filter((c) => c.id !== id));
      if (id === selectedId) {
        setSelectedId(null);
        setView("captures");
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
    setView("frames");
  };

  const header = (
    <div className="flex h-12 items-center gap-3 border-b border-line bg-surface px-4">
      <div className="flex items-baseline gap-2">
        <span className="text-sm font-semibold tracking-tight text-fg">ECAT FlowLab</span>
        <span className="hidden font-mono text-[11px] text-fg-subtle sm:inline">
          EtherCAT analysis
        </span>
      </div>
      {captures.length > 0 ? (
        <div className="ml-4 flex items-center gap-2">
          <Select
            aria-label="Capture"
            className="w-64"
            value={selectedId === null ? "" : String(selectedId)}
            placeholder="Select a capture"
            options={captures.map((c) => ({
              value: String(c.id),
              label: c.name,
              note: c.provenance,
            }))}
            onValueChange={(v) => v && select(Number(v))}
          />
          {selected ? <ProvenanceBadge provenance={selected.provenance} /> : null}
        </div>
      ) : null}
      <div className="ml-auto flex items-center gap-2">
        <Button
          variant="ghost"
          size="sm"
          icon={<FlaskConical className="size-4" />}
          onClick={() => setSimOpen(true)}
          disabled={busy}
        >
          Simulate
        </Button>
        <Button
          variant="ghost"
          size="sm"
          icon={<Sparkles className="size-4" />}
          onClick={() => setSynthOpen(true)}
          disabled={busy}
        >
          Generate
        </Button>
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

  const hasReport = selectedId !== null;
  const rail = (
    <NavRail value={view} onValueChange={(v) => setView(v as View)} labels="visible">
      <NavRailItem value="captures" label="Captures" icon={<FolderOpen />} />
      <NavRailItem value="overview" label="Overview" icon={<Gauge />} disabled={!hasReport} />
      <NavRailItem value="timing" label="Timing" icon={<Activity />} disabled={!hasReport} />
      <NavRailItem value="frames" label="Frames" icon={<Rows3 />} disabled={!hasReport} />
      <NavRailItem value="stations" label="Stations" icon={<Network />} disabled={!hasReport} />
      <NavRailItem value="mailbox" label="Mailbox" icon={<Mail />} disabled={!hasReport} />
      <NavRailItem value="planner" label="Planner" icon={<Ruler />} />
      <NavRailItem value="study" label="Study" icon={<ChartScatter />} />
      <NavRailItem
        value="compare"
        label="Compare"
        icon={<Columns2 />}
        disabled={captures.length < 2}
      />
      <NavRailItem
        value="simulation"
        label="Sim run"
        icon={<Waypoints />}
        disabled={!hasReport || !hasSimulation}
      />
    </NavRail>
  );

  let main;
  if (view === "planner") {
    main = <Planner />;
  } else if (view === "study") {
    main = <Study captures={captures} onCreated={added} />;
  } else if (view === "compare") {
    main = <Compare key={captures.map((c) => c.id).join(",")} captures={captures} />;
  } else if (view === "captures" || selectedId === null) {
    main = (
      <Captures
        captures={captures}
        selectedId={selectedId}
        busy={busy}
        onSelect={select}
        onRemove={(id) => void remove(id)}
        onFiles={onFiles}
        onOpenPath={(p) => void run("Could not open the file", () => api.openPath(p))}
        onGenerate={() => setSynthOpen(true)}
      />
    );
  } else if (report.status === "error") {
    main = <ErrorBox className="m-6">{report.error}</ErrorBox>;
  } else if (!report.data || report.status === "loading") {
    main = <SkeletonRows rows={10} className="m-6" />;
  } else if (view === "overview") {
    main = (
      <Overview
        report={report.data}
        eni={selected?.eni ?? null}
        onAttachEni={(f) => void attachEni(f)}
        onDetachEni={() => void detachEni()}
        onOpenStream={(id) => {
          setStreamId(id);
          setView("timing");
        }}
      />
    );
  } else if (view === "timing") {
    main = (
      <Timing
        report={report.data}
        streamId={streamId}
        onStreamChange={setStreamId}
        onOpenFrame={openFrame}
      />
    );
  } else if (view === "frames") {
    main = (
      <Frames
        key={`${selectedId}:${frameTarget?.nonce ?? 0}`}
        captureId={selectedId}
        report={report.data}
        target={frameTarget}
        filter={frameFilter}
        onFilterChange={setFrameFilter}
      />
    );
  } else if (view === "simulation") {
    main =
      simulation.status === "error" ? (
        <ErrorBox className="m-6">{simulation.error}</ErrorBox>
      ) : simulation.data ? (
        <Simulation result={simulation.data} />
      ) : hasSimulation ? (
        <SkeletonRows rows={10} className="m-6" />
      ) : (
        <ErrorBox className="m-6">This capture is not a simulation run.</ErrorBox>
      );
  } else if (view === "mailbox") {
    main = <Mailbox captureId={selectedId} report={report.data} onOpenFrame={openFrame} />;
  } else {
    main = <Stations report={report.data} onOpenFrame={openFrame} />;
  }

  const r = report.data;
  const bottom = (
    <StatusBar
      start={[
        { value: selected ? selected.provenance.toUpperCase() : "no capture" },
        { value: selected?.name },
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
    <>
      <AppShell
        header={header}
        rail={rail}
        main={<div className="h-full min-h-0 overflow-y-auto">{main}</div>}
        bottom={bottom}
      />
      <SynthDialog open={synthOpen} onOpenChange={setSynthOpen} onCreated={added} />
      <SimulateDialog open={simOpen} onOpenChange={setSimOpen} onCreated={added} />
    </>
  );
}
