import { Button, Disclosure, PageHeader, Panel, Table, type Column } from "@vitavision/ui";
import { Download, Pencil } from "lucide-react";
import { useState } from "react";

import type { SimResult, SimSpec, TransferResult } from "../api";
import { ProvenanceBadge } from "../components/ProvenanceBadge";
import { downloadJson } from "../lib/download";
import { count, ns } from "../lib/format";
import { ScenarioResult } from "../scenario/ScenarioResult";
import { SystemSummary } from "../scenario/SystemSummary";

const DETAIL_COLUMNS: Column<TransferResult>[] = [
  { key: "i", header: "#", numeric: true, width: "2.5rem", cell: (t) => t.index },
  { key: "p", header: "Protocol", width: "7rem", cell: (t) => t.protocol },
  { key: "mo", header: "Msgs ↓", numeric: true, cell: (t) => count(t.messages_to_slave) },
  { key: "mi", header: "Msgs ↑", numeric: true, cell: (t) => count(t.messages_from_slave) },
  { key: "rej", header: "Rejected writes", numeric: true, cell: (t) => count(t.rejected_writes) },
  { key: "emp", header: "Empty reads", numeric: true, cell: (t) => count(t.empty_reads) },
  { key: "ch", header: "Chunk cycles", numeric: true, cell: (t) => count(t.chunk_cycles) },
  {
    key: "re",
    header: "Resent chunks",
    numeric: true,
    cell: (t) => count(t.retransmitted_chunks),
  },
  {
    key: "fb",
    header: "First byte",
    numeric: true,
    cell: (t) => (t.first_byte_ns === null ? "–" : ns(t.first_byte_ns - t.start_ns)),
  },
  {
    key: "tcp",
    header: "TCP ref.",
    numeric: true,
    cell: (t) => ns(t.ethernet_tcp_reference_ns),
  },
];

/** A simulation run loaded as a capture: what was simulated and what came out. */
export function Simulation({
  result,
  onEdit,
}: {
  result: SimResult;
  /** Open this run's spec in the Simulate view. */
  onEdit?: (spec: SimSpec) => void;
}) {
  const r = result;
  const [selected, setSelected] = useState<number | null>(null);
  return (
    <div className="mx-auto flex max-w-[110rem] flex-col gap-4 p-5">
      <PageHeader
        title="Simulation run"
        meta={
          <span className="flex flex-wrap items-center gap-x-3 gap-y-1">
            <ProvenanceBadge provenance={r.meta.source} />
            <span className="text-xs">a model prediction, not a measurement</span>
            <span className="font-mono text-xs">seed {r.seed}</span>
            <span className="font-mono text-xs" title={r.meta.input.sha256}>
              spec {r.meta.input.sha256.slice(0, 12)}…
            </span>
          </span>
        }
        actions={
          <>
            {onEdit ? (
              <Button
                variant="primary"
                icon={<Pencil className="size-4" />}
                onClick={() => onEdit(r.spec)}
              >
                Edit in Simulate
              </Button>
            ) : null}
            <Button
              variant="secondary"
              icon={<Download className="size-4" />}
              onClick={() => downloadJson(`sim-seed${r.seed}.result.json`, r)}
            >
              Result JSON
            </Button>
          </>
        }
      />
      <Panel title="What was simulated">
        <SystemSummary spec={r.spec} policy={r.policy} />
        {r.capture_truncated ? (
          <p className="mt-2 text-xs text-fg-muted">
            The capture holds the first {count(r.capture_frames)} frames; the analysis views show
            only those. The results here cover the whole run.
          </p>
        ) : null}
      </Panel>
      <ScenarioResult result={r} baseline={null} selected={selected} onSelect={setSelected} />
      <Disclosure summary="Protocol work per transfer">
        <Table columns={DETAIL_COLUMNS} rows={r.transfers} rowKey={(t) => t.index} />
        <p className="mt-2 text-xs text-fg-muted">
          <span className="font-medium">TCP ref.</span>: TCP over a dedicated link at the same rate,
          steady state, no loss — a lower bound for plain Ethernet, not a prediction.
        </p>
      </Disclosure>
      <Disclosure summary="Model assumptions" count={r.meta.assumptions.length}>
        <ul className="ml-4 list-disc space-y-1 text-sm text-fg-muted">
          {r.meta.assumptions.map((a) => (
            <li key={a}>{a}</li>
          ))}
        </ul>
      </Disclosure>
    </div>
  );
}
