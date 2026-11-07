import { Callout, type CalloutTone } from "@vitavision/ui";

import type { Finding, Severity } from "../api";

const TONE: Record<Severity, CalloutTone> = { info: "info", warn: "warning", defect: "error" };
const ORDER: Record<Severity, number> = { defect: 0, warn: 1, info: 2 };

/** Findings, most severe first. */
export function Findings({ findings }: { findings: Finding[] }) {
  const sorted = [...findings].sort((a, b) => ORDER[a.severity] - ORDER[b.severity]);
  if (sorted.length === 0) {
    return (
      <Callout tone="success" title="Nothing to report">
        No finding crossed a declared threshold.
      </Callout>
    );
  }
  return (
    <div className="flex flex-col gap-2">
      {sorted.map((f, i) => (
        // Findings have no id; their order is stable for a given report.
        // oxlint-disable-next-line react/no-array-index-key -- positional list
        <Callout key={i} tone={TONE[f.severity]} title={f.title}>
          {f.detail}
        </Callout>
      ))}
    </div>
  );
}
