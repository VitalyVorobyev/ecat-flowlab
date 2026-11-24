import { ErrorBox, Select, SkeletonRows, Tabs, type TabItem } from "@vitavision/ui";

import type { CaptureInfo, CaptureReport, FrameFilter, SimResult, SimSpec } from "../api";
import { ProvenanceBadge } from "../components/ProvenanceBadge";
import type { Loaded } from "../lib/useAsync";
import { Captures } from "./Captures";
import { Compare } from "./Compare";
import { Frames, type FramesTarget } from "./Frames";
import { Live } from "./Live";
import { Mailbox } from "./Mailbox";
import { Overview } from "./Overview";
import { Simulation } from "./Simulation";
import { Stations } from "./Stations";
import { Timing } from "./Timing";

export type AnalysisTab =
  | "captures"
  | "live"
  | "overview"
  | "timing"
  | "frames"
  | "stations"
  | "mailbox"
  | "compare"
  | "simulation";

export const ANALYSIS_TABS: readonly AnalysisTab[] = [
  "captures",
  "live",
  "overview",
  "timing",
  "frames",
  "stations",
  "mailbox",
  "compare",
  "simulation",
];

const CAPTURE_TABS: readonly AnalysisTab[] = [
  "overview",
  "timing",
  "frames",
  "stations",
  "mailbox",
  "simulation",
];

export function needsCapture(tab: AnalysisTab): boolean {
  return CAPTURE_TABS.includes(tab);
}

/**
 * Consolidated capture analysis workspace: capture management, live capture/replay,
 * timing/frame/station/mailbox inspection, side-by-side comparison, and simulation results.
 */
export function Analysis({
  tab,
  onTabChange,
  captures,
  selectedId,
  report,
  simulation,
  busy,
  localPathsEnabled,
  streamId,
  onStreamChange,
  frameFilter,
  onFilterChange,
  frameTarget,
  onSelectCapture,
  onRemoveCapture,
  onFiles,
  onOpenPath,
  onSavedLive,
  onAttachEni,
  onDetachEni,
  onOpenFrame,
  onEditSimulation,
}: {
  tab: AnalysisTab;
  onTabChange: (tab: AnalysisTab) => void;
  captures: CaptureInfo[];
  selectedId: number | null;
  report: Loaded<CaptureReport | null>;
  simulation: Loaded<SimResult | null>;
  busy: boolean;
  localPathsEnabled: boolean;
  streamId: number;
  onStreamChange: (id: number) => void;
  frameFilter: FrameFilter;
  onFilterChange: (filter: FrameFilter) => void;
  frameTarget: FramesTarget | null;
  onSelectCapture: (id: number) => void;
  onRemoveCapture: (id: number) => void;
  onFiles: (files: File[]) => void;
  onOpenPath: (path: string) => void;
  onSavedLive: (info: CaptureInfo) => void;
  onAttachEni: (file: File) => void;
  onDetachEni: () => void;
  onOpenFrame: (no: number) => void;
  onEditSimulation: (spec: SimSpec) => void;
}) {
  const selected = captures.find((c) => c.id === selectedId) ?? null;
  const hasCapture = selectedId !== null;
  const hasSimulation = selected?.has_simulation ?? false;

  const captureTab = (id: AnalysisTab, label: string): TabItem<AnalysisTab> => ({
    id,
    label,
    disabled: !hasCapture,
    ...(hasCapture ? {} : { title: "Load and select a capture first" }),
  });

  const items: TabItem<AnalysisTab>[] = [
    {
      id: "captures",
      label: "Captures",
      ...(captures.length > 0 ? { count: captures.length } : {}),
    },
    { id: "live", label: "Live" },
    captureTab("overview", "Overview"),
    captureTab("timing", "Timing"),
    captureTab("frames", "Frames"),
    captureTab("stations", "Stations"),
    captureTab("mailbox", "Mailbox"),
    {
      id: "compare",
      label: "Compare",
      disabled: captures.length < 2,
      ...(captures.length < 2 ? { title: "Load at least two captures to compare" } : {}),
    },
  ];
  if (hasSimulation) {
    items.push({ id: "simulation", label: "Sim run" });
  }

  const effectiveTab: AnalysisTab =
    tab === "simulation" && !hasSimulation
      ? hasCapture
        ? "overview"
        : "captures"
      : needsCapture(tab) && !hasCapture
        ? "captures"
        : tab === "compare" && captures.length < 2
          ? hasCapture
            ? "overview"
            : "captures"
          : tab;

  let body;
  if (effectiveTab === "captures") {
    body = (
      <Captures
        captures={captures}
        selectedId={selectedId}
        busy={busy}
        localPathsEnabled={localPathsEnabled}
        onSelect={onSelectCapture}
        onRemove={onRemoveCapture}
        onFiles={onFiles}
        onOpenPath={onOpenPath}
      />
    );
  } else if (effectiveTab === "live") {
    body = <Live captures={captures} onSaved={onSavedLive} onOpen={onSelectCapture} />;
  } else if (effectiveTab === "compare") {
    body = <Compare key={captures.map((c) => c.id).join(",")} captures={captures} />;
  } else if (selectedId === null) {
    body = (
      <Captures
        captures={captures}
        selectedId={selectedId}
        busy={busy}
        localPathsEnabled={localPathsEnabled}
        onSelect={onSelectCapture}
        onRemove={onRemoveCapture}
        onFiles={onFiles}
        onOpenPath={onOpenPath}
      />
    );
  } else if (report.status === "error") {
    body = <ErrorBox className="m-6">{report.error}</ErrorBox>;
  } else if (!report.data || report.status === "loading") {
    body = <SkeletonRows rows={10} className="m-6" />;
  } else if (effectiveTab === "overview") {
    body = (
      <Overview
        report={report.data}
        eni={selected?.eni ?? null}
        onAttachEni={onAttachEni}
        onDetachEni={onDetachEni}
        onOpenStream={(id) => {
          onStreamChange(id);
          onTabChange("timing");
        }}
      />
    );
  } else if (effectiveTab === "timing") {
    body = (
      <Timing
        report={report.data}
        streamId={streamId}
        onStreamChange={onStreamChange}
        onOpenFrame={onOpenFrame}
      />
    );
  } else if (effectiveTab === "frames") {
    body = (
      <Frames
        key={`${selectedId}:${frameTarget?.nonce ?? 0}`}
        captureId={selectedId}
        report={report.data}
        target={frameTarget}
        filter={frameFilter}
        onFilterChange={onFilterChange}
      />
    );
  } else if (effectiveTab === "simulation") {
    body =
      simulation.status === "error" ? (
        <ErrorBox className="m-6">{simulation.error}</ErrorBox>
      ) : simulation.data ? (
        <Simulation result={simulation.data} onEdit={onEditSimulation} />
      ) : hasSimulation ? (
        <SkeletonRows rows={10} className="m-6" />
      ) : (
        <ErrorBox className="m-6">This capture is not a simulation run.</ErrorBox>
      );
  } else if (effectiveTab === "mailbox") {
    body = <Mailbox captureId={selectedId} report={report.data} onOpenFrame={onOpenFrame} />;
  } else {
    body = <Stations report={report.data} onOpenFrame={onOpenFrame} />;
  }

  return (
    <div className="flex min-h-full flex-col">
      <div className="sticky top-0 z-10 flex flex-wrap items-center gap-3 border-b border-line bg-surface px-5 py-2">
        <Tabs
          items={items}
          active={effectiveTab}
          onSelect={onTabChange}
          label="Analysis views"
          idPrefix="analysis"
        />
        {captures.length > 0 ? (
          <div className="ml-auto flex items-center gap-2">
            <Select
              aria-label="Active capture"
              className="w-60"
              value={selectedId === null ? "" : String(selectedId)}
              placeholder="Select a capture"
              options={captures.map((c) => ({
                value: String(c.id),
                label: c.name,
                note: c.provenance,
              }))}
              onValueChange={(v) => v && onSelectCapture(Number(v))}
            />
            {selected ? <ProvenanceBadge provenance={selected.provenance} /> : null}
          </div>
        ) : null}
      </div>
      <div className="min-h-0 flex-1">{body}</div>
    </div>
  );
}
