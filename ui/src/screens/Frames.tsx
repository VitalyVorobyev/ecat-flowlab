import {
  Badge,
  Button,
  DensityProvider,
  Empty,
  ErrorBox,
  Input,
  SegmentedControl,
  Select,
  SkeletonRows,
  Table,
  ToggleChip,
  type Column,
} from "@vitavision/ui";
import { SplitPane } from "@vitavision/workbench";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { useState } from "react";

import { api, type CaptureReport, type FrameFilter, type FrameRow, MAILBOX_FILTERS } from "../api";
import { captureTime, count, ns } from "../lib/format";
import { useAsync } from "../lib/useAsync";
import { FrameDetailView } from "./FrameDetailView";

export const PAGE = 1000;
const COMMANDS = [
  "APRD",
  "APWR",
  "APRW",
  "FPRD",
  "FPWR",
  "FPRW",
  "BRD",
  "BWR",
  "BRW",
  "LRD",
  "LWR",
  "LRW",
  "ARMW",
  "FRMW",
  "NOP",
];

const columns: Column<FrameRow>[] = [
  { key: "no", header: "No.", numeric: true, width: "4.5rem", cell: (r) => r.no },
  { key: "t", header: "Time", numeric: true, width: "7.5rem", cell: (r) => captureTime(r.t_ns) },
  {
    key: "dir",
    header: "Dir",
    width: "3.5rem",
    cell: (r) => (
      <span className="font-mono text-xs text-fg-muted">
        {r.direction === "unknown" ? "?" : r.direction.toUpperCase()}
      </span>
    ),
  },
  { key: "len", header: "Len", numeric: true, width: "3.5rem", cell: (r) => r.len },
  {
    key: "sum",
    header: "Datagrams",
    cell: (r) => <span className="font-mono text-xs">{r.summary}</span>,
  },
  {
    key: "flag",
    header: "Status",
    width: "8rem",
    cell: (r) =>
      r.kind === "malformed" ? (
        <Badge tone="defect">malformed</Badge>
      ) : r.unanswered ? (
        <Badge tone="defect">no return</Badge>
      ) : r.wkc_mismatch ? (
        <Badge tone="defect">WKC</Badge>
      ) : r.kind === "other" ? (
        <span className="text-xs text-fg-subtle">other</span>
      ) : null,
  },
  {
    key: "rtt",
    header: "RTT",
    numeric: true,
    width: "6rem",
    cell: (r) => (r.rtt_ns === null ? "" : ns(r.rtt_ns)),
  },
];

export type FramesTarget = { no: number; nonce: number };

/** Every frame, filterable, with the selected one decoded beside the list. */
export function Frames({
  captureId,
  report,
  target,
  filter,
  onFilterChange,
}: {
  captureId: number;
  report: CaptureReport;
  target: FramesTarget | null;
  filter: FrameFilter;
  onFilterChange: (f: FrameFilter) => void;
}) {
  // A jump from another view remounts this screen (keyed by the target) with the frame at the
  // top of the list, selected.
  const [offset, setOffset] = useState(target ? Math.max(0, target.no - 1) : 0);
  const [selected, setSelected] = useState<number | null>(target?.no ?? null);
  const [station, setStation] = useState(
    filter.station === null ? "" : filter.station.toString(16),
  );

  const page = useAsync(
    () => api.frames(captureId, filter, offset, PAGE),
    [captureId, filter, offset],
  );
  const detail = useAsync(
    () => (selected === null ? Promise.resolve(null) : api.frame(captureId, selected)),
    [captureId, selected],
  );

  const set = (patch: Partial<FrameFilter>) => {
    setOffset(0);
    onFilterChange({ ...filter, ...patch });
  };
  const total = page.data?.total ?? 0;

  const toolbar = (
    <div className="flex flex-wrap items-center gap-2 border-b border-line px-4 py-2">
      <SegmentedControl
        aria-label="Direction"
        size="sm"
        value={filter.direction ?? "all"}
        options={[
          { value: "all", label: "All" },
          { value: "tx", label: "TX" },
          { value: "rx", label: "RX" },
        ]}
        onValueChange={(v) => set({ direction: v === "all" ? null : v })}
      />
      <Select
        aria-label="Command"
        className="w-40"
        value={filter.command ?? ""}
        placeholder="Any command"
        unsetLabel="Any command"
        options={COMMANDS.map((c) => ({ value: c, label: c }))}
        onValueChange={(v) => set({ command: v || null })}
      />
      <Select
        aria-label="Stream"
        className="w-44"
        value={filter.stream === null ? "" : String(filter.stream)}
        placeholder="Any stream"
        unsetLabel="Any stream"
        options={[
          ...report.streams.map((s) => ({
            value: String(s.id),
            label: `Stream ${s.id} · ${ns(s.period_ns)}`,
          })),
          { value: "65535", label: "Not cyclic" },
        ]}
        onValueChange={(v) => set({ stream: v === "" ? null : Number(v) })}
      />
      <Select
        aria-label="Mailbox"
        className="w-48"
        value={filter.mailbox ?? ""}
        placeholder="Any traffic"
        unsetLabel="Any traffic"
        options={MAILBOX_FILTERS.map((m) => ({ value: m.value, label: m.label }))}
        onValueChange={(v) => set({ mailbox: v || null })}
      />
      <form
        className="flex items-center gap-1"
        onSubmit={(e) => {
          e.preventDefault();
          const n = Number.parseInt(station, 16);
          set({ station: station.trim() === "" || Number.isNaN(n) ? null : n });
        }}
      >
        <span className="font-mono text-xs text-fg-muted">0x</span>
        <Input
          aria-label="Station address (hex)"
          placeholder="station"
          className="w-24 font-mono"
          value={station}
          onChange={(e) => setStation(e.target.value)}
          onBlur={(e) => e.currentTarget.form?.requestSubmit()}
        />
      </form>
      <ToggleChip
        size="sm"
        checked={filter.problems_only}
        onCheckedChange={(v) => set({ problems_only: v })}
      >
        Problems only
      </ToggleChip>
      <ToggleChip
        size="sm"
        checked={filter.include_other}
        onCheckedChange={(v) => set({ include_other: v })}
      >
        Non-EtherCAT
      </ToggleChip>
      <div className="ml-auto flex items-center gap-1 font-mono text-xs text-fg-muted">
        {total > 0
          ? `${count(offset + 1)}–${count(Math.min(offset + PAGE, total))} of ${count(total)}`
          : "0 frames"}
        <Button
          variant="ghost"
          size="sm"
          aria-label="Previous page"
          icon={<ChevronLeft className="size-4" />}
          disabled={offset === 0}
          onClick={() => setOffset(Math.max(0, offset - PAGE))}
        />
        <Button
          variant="ghost"
          size="sm"
          aria-label="Next page"
          icon={<ChevronRight className="size-4" />}
          disabled={offset + PAGE >= total}
          onClick={() => setOffset(offset + PAGE)}
        />
      </div>
    </div>
  );

  const list =
    page.status === "error" ? (
      <ErrorBox className="m-4">{page.error}</ErrorBox>
    ) : page.data === null ? (
      <SkeletonRows rows={12} className="m-4" />
    ) : (
      <div className="flex h-full min-h-0 flex-col">
        <DensityProvider value="compact">
          <Table
            className="min-h-0 flex-1 pr-3"
            columns={columns}
            rows={page.data.rows}
            rowKey={(r) => r.no}
            onRowClick={(r) => setSelected(r.no)}
            isRowActive={(r) => r.no === selected}
            selectionFollowsFocus
            windowed={{ rowHeight: 24 }}
            empty="No frame matches the filter."
          />
        </DensityProvider>
      </div>
    );

  const side =
    selected === null ? (
      <Empty className="m-6">Select a frame to decode it.</Empty>
    ) : detail.status === "error" ? (
      <ErrorBox className="m-4">{detail.error}</ErrorBox>
    ) : detail.data ? (
      <FrameDetailView detail={detail.data} onOpenFrame={setSelected} />
    ) : (
      <SkeletonRows rows={8} className="m-4" />
    );

  return (
    <div className="flex h-full min-h-0 flex-col">
      {toolbar}
      <SplitPane
        defaultSize="58%"
        minSize={360}
        maxSize="80%"
        storageKey="flowlab:frames-split"
        className="min-h-0 flex-1"
      >
        <div className="h-full min-h-0">{list}</div>
        <div className="h-full overflow-y-auto">{side}</div>
      </SplitPane>
    </div>
  );
}
