import { FileDrop } from "@vitavision/workbench";
import { Button, CountRun, Input, PageHeader, Panel, Table, type Column } from "@vitavision/ui";
import { FolderOpen, Sparkles, Trash } from "lucide-react";
import { useState } from "react";

import type { CaptureInfo } from "../api";
import { ProvenanceBadge } from "../components/ProvenanceBadge";
import { bytes, count, ns } from "../lib/format";

export const CAPTURE_ACCEPT = ".pcap,.pcapng,.cap";

function captureColumns(onRemove: (id: number) => void): Column<CaptureInfo>[] {
  return [
    {
      key: "name",
      header: "Capture",
      cell: (c) => <span className="font-medium text-fg">{c.name}</span>,
      sortValue: (c) => c.name,
    },
    { key: "src", header: "Source", cell: (c) => <ProvenanceBadge provenance={c.provenance} /> },
    { key: "fmt", header: "Format", cell: (c) => c.format },
    {
      key: "frames",
      header: "Frames",
      numeric: true,
      cell: (c) => count(c.frames),
      sortValue: (c) => c.frames,
    },
    {
      key: "ecat",
      header: "EtherCAT",
      numeric: true,
      cell: (c) => count(c.ethercat_frames),
      sortValue: (c) => c.ethercat_frames,
    },
    {
      key: "dur",
      header: "Duration",
      numeric: true,
      cell: (c) => ns(c.duration_ns),
      sortValue: (c) => c.duration_ns,
    },
    { key: "streams", header: "Streams", numeric: true, cell: (c) => c.streams },
    {
      key: "findings",
      header: "Findings",
      cell: (c) =>
        c.defects + c.warnings === 0 ? (
          <span className="text-fg-subtle">none</span>
        ) : (
          <CountRun
            counts={[
              ["defects", c.defects, "defect"],
              ["warnings", c.warnings, "warning"],
            ]}
          />
        ),
    },
    { key: "size", header: "Size", numeric: true, cell: (c) => bytes(c.bytes) },
    {
      key: "x",
      header: <span className="sr-only">Remove</span>,
      width: "3rem",
      cell: (c) => (
        <Button
          variant="ghost"
          size="sm"
          aria-label={`Close ${c.name}`}
          icon={<Trash className="size-3.5" />}
          onClick={(e) => {
            e.stopPropagation();
            onRemove(c.id);
          }}
        />
      ),
    },
  ];
}

/** Open captures, and the list of loaded ones. */
export function Captures({
  captures,
  selectedId,
  busy,
  onSelect,
  onRemove,
  onFiles,
  onOpenPath,
  onGenerate,
}: {
  captures: CaptureInfo[];
  selectedId: number | null;
  busy: boolean;
  onSelect: (id: number) => void;
  onRemove: (id: number) => void;
  onFiles: (files: File[]) => void;
  onOpenPath: (path: string) => void;
  onGenerate: () => void;
}) {
  const [path, setPath] = useState("");

  const open = (
    <div className="grid gap-4 lg:grid-cols-3">
      <Panel title="Drop a capture">
        <FileDrop
          accept={CAPTURE_ACCEPT}
          onFiles={onFiles}
          disabled={busy}
          buttonLabel="Choose files…"
        >
          pcap or pcapng from Wireshark, tcpdump or a master's capture tool — drop here, or
        </FileDrop>
      </Panel>
      <Panel title="Open a file on this machine">
        <form
          className="flex flex-col gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            if (path.trim()) onOpenPath(path.trim());
          }}
        >
          <Input
            value={path}
            onChange={(e) => setPath(e.target.value)}
            placeholder="/path/to/capture.pcapng"
            aria-label="Capture path"
            className="font-mono"
          />
          <p className="text-xs text-fg-muted">
            Read by the local FlowLab server — no upload, so large captures open faster.
          </p>
          <div>
            <Button
              type="submit"
              variant="secondary"
              icon={<FolderOpen className="size-4" />}
              disabled={busy || !path.trim()}
            >
              Open path
            </Button>
          </div>
        </form>
      </Panel>
      <Panel title="No capture at hand?">
        <div className="flex flex-col gap-3">
          <p className="text-sm text-fg-muted">
            Generate a synthetic capture of a master and six slaves, with start-up, CoE SDO uploads,
            a 1 ms cycle and rare injected faults. Its ground truth is known, so you can see what
            each view finds.
          </p>
          <div>
            <Button
              variant="primary"
              icon={<Sparkles className="size-4" />}
              onClick={onGenerate}
              disabled={busy}
            >
              Generate demo capture…
            </Button>
          </div>
        </div>
      </Panel>
    </div>
  );

  return (
    <div className="mx-auto flex max-w-7xl flex-col gap-6 p-6">
      <PageHeader
        title="Captures"
        meta="Load EtherCAT traffic captured at the master, or generate a synthetic one."
      />
      {open}
      {captures.length > 0 ? (
        <Panel title={`Loaded (${captures.length})`}>
          <Table
            columns={captureColumns(onRemove)}
            rows={captures}
            rowKey={(c) => c.id}
            onRowClick={(c) => onSelect(c.id)}
            isRowActive={(c) => c.id === selectedId}
          />
        </Panel>
      ) : null}
    </div>
  );
}
