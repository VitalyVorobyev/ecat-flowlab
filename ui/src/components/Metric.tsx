import { cn, InfoHint } from "@vitavision/ui";
import type { ReactNode } from "react";

/**
 * One headline number. `verdict` colours the value only when the app has compared it with a
 * declared threshold; magnitudes stay neutral.
 */
export function Metric({
  label,
  value,
  sub,
  verdict,
  hint,
}: {
  label: string;
  value: ReactNode;
  sub?: ReactNode;
  verdict?: "normal" | "warn" | "defect" | undefined;
  hint?: ReactNode;
}) {
  return (
    <div className="flex min-w-0 flex-col gap-0.5 rounded-md border border-line bg-surface px-3 py-2">
      <div className="flex items-center gap-1 text-[11px] text-fg-muted">
        <span className="truncate">{label}</span>
        {hint ? <InfoHint label={`About ${label}`}>{hint}</InfoHint> : null}
      </div>
      <div
        className={cn(
          "truncate font-mono text-base leading-tight font-medium",
          verdict === "defect" && "text-defect",
          verdict === "warn" && "text-warn",
          verdict === "normal" && "text-normal",
          !verdict && "text-fg",
        )}
      >
        {value}
      </div>
      {sub ? <div className="truncate font-mono text-[11px] text-fg-subtle">{sub}</div> : null}
    </div>
  );
}

/** Headline numbers in a grid; `compact` for a half-width column (at most four across). */
export function MetricGrid({
  children,
  compact = false,
}: {
  children: ReactNode;
  compact?: boolean;
}) {
  return (
    <div
      className={
        compact
          ? "grid grid-cols-[repeat(auto-fill,minmax(8.5rem,1fr))] gap-2"
          : "grid grid-cols-[repeat(auto-fill,minmax(9.5rem,1fr))] gap-2"
      }
    >
      {children}
    </div>
  );
}
