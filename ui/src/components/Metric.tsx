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
    <div className="flex min-w-0 flex-col gap-1 rounded-md border border-line bg-surface px-3.5 py-3">
      <div className="flex items-center gap-1 text-xs text-fg-muted">
        <span className="truncate">{label}</span>
        {hint ? <InfoHint label={`About ${label}`}>{hint}</InfoHint> : null}
      </div>
      <div
        className={cn(
          "truncate font-mono text-xl leading-tight font-medium",
          verdict === "defect" && "text-defect",
          verdict === "warn" && "text-warn",
          verdict === "normal" && "text-normal",
          !verdict && "text-fg",
        )}
      >
        {value}
      </div>
      {sub ? <div className="truncate font-mono text-xs text-fg-subtle">{sub}</div> : null}
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
          ? "grid grid-cols-2 gap-3 2xl:grid-cols-4"
          : "grid grid-cols-2 gap-3 md:grid-cols-4 2xl:grid-cols-8"
      }
    >
      {children}
    </div>
  );
}
