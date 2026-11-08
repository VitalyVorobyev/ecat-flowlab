import { Button, Disclosure, Panel, SkeletonRows, cn } from "@vitavision/ui";
import { Check } from "lucide-react";
import type { ReactNode } from "react";

import type { Lever, RankedChange, SpecEdit, WhatIf, WhatIfResult, WhatIfRun } from "../api";
import { ns, percent } from "../lib/format";

const LEVER: Record<Lever, string> = {
  master: "master",
  mailbox_policy: "policy",
  pdi: "PDI",
  mailbox: "mailbox",
  application: "application",
  protocol: "protocol",
  combined: "two changes",
};

export interface Scored {
  change: WhatIf;
  /** Relative change of the measured time, or null when the run is not feasible. */
  delta: number | null;
  time: number | null;
}

export interface Ranking {
  /** Single changes that shorten the time by more than the result's `no_effect`, best first. */
  helps: Scored[];
  flat: Scored[];
  hurts: Scored[];
  impossible: Scored[];
  /** Pairs that beat both of their parts alone by more than `no_effect`, best first. */
  together: Scored[];
}

/**
 * The study's ranking of the changes by their effect on one transfer's duration, or on when all
 * are done (`flowlab_study::WhatIfRanking`, the same ranking `flowlab what-if` prints).
 */
export function rank(r: WhatIfResult, transfer: number | null): Ranking {
  const ranking = r.rankings.find((k) => k.transfer === transfer) ?? r.rankings[0];
  const scored = (list: readonly RankedChange[] | undefined): Scored[] =>
    (list ?? []).flatMap((x) => {
      const change = r.changes[x.change];
      return change ? [{ change, delta: x.delta, time: x.time_ns }] : [];
    });
  return {
    helps: scored(ranking?.helps),
    flat: scored(ranking?.flat),
    hurts: scored(ranking?.hurts),
    impossible: scored(ranking?.impossible),
    together: scored(ranking?.together),
  };
}

function side(change: WhatIf, base: WhatIfRun): string | null {
  const parts: string[] = [];
  const add = (label: string, a: number | null, b: number | null) => {
    if (a !== null && b !== null && Math.abs(b - a) >= 1)
      parts.push(`${label} ${percent(a)} → ${percent(b)}`);
  };
  add("link", base.link_utilization_percent, change.run.link_utilization_percent);
  add("busiest PDI", base.pdi_busy_percent_max, change.run.pdi_busy_percent_max);
  add("ESC RAM", base.dpram_used_percent_max, change.run.dpram_used_percent_max);
  return parts.length ? parts.join(" · ") : null;
}

function signed(delta: number): string {
  const digits = Math.abs(delta) < 0.1 ? 1 : 0;
  return `${delta < 0 ? "−" : "+"}${Math.abs(100 * delta).toFixed(digits)} %`;
}

function ChangeRow({
  row,
  base,
  maxGain,
  onApply,
}: {
  row: Scored;
  base: WhatIfRun;
  maxGain: number;
  onApply: (edits: readonly SpecEdit[]) => void;
}) {
  const { change, delta, time } = row;
  const extra = side(change, base);
  return (
    <li className="flex items-center gap-3 border-b border-line py-1.5 last:border-b-0">
      <div className="w-14 shrink-0 text-right font-mono text-sm text-fg">
        {delta === null ? "–" : signed(delta)}
      </div>
      <div className="hidden w-24 shrink-0 md:block" aria-hidden="true">
        {delta !== null && delta < 0 ? (
          <div className="h-1.5 rounded-full bg-raised">
            <div
              className="h-1.5 rounded-full bg-fg-subtle"
              style={{ width: `${Math.min(100, (100 * -delta) / maxGain)}%` }}
            />
          </div>
        ) : null}
      </div>
      <div className="min-w-0 flex-1">
        <div className="text-sm text-fg">
          {change.parts.length ? change.parts.join("  +  ") : change.label}
        </div>
        <div className="font-mono text-[11px] text-fg-muted">
          <span className="mr-2 rounded-sm bg-raised px-1">{LEVER[change.lever]}</span>
          {change.run.feasible ? (
            <>
              {time === null ? "" : ns(time)}
              {extra ? ` · ${extra}` : ""}
            </>
          ) : (
            change.run.reason
          )}
        </div>
      </div>
      <Button
        size="sm"
        variant="ghost"
        icon={<Check className="size-3.5" />}
        onClick={() => onApply(change.edits)}
        title="Apply to the setup"
      >
        Apply
      </Button>
    </li>
  );
}

/**
 * What would help: the scenario rerun with one change at a time (and pairs of them), ranked
 * by how much each shortens the selected transfer, or the whole run.
 */
export function WhatIfPanel({
  result,
  running,
  error,
  transfer,
  transferLabel,
  onApply,
  className,
}: {
  result: WhatIfResult | null;
  running: boolean;
  error: string | null;
  /** Rank by this transfer's duration; `null` ranks by when the last transfer finishes. */
  transfer: number | null;
  transferLabel: string | null;
  onApply: (edits: readonly SpecEdit[]) => void;
  className?: string;
}) {
  const title = (
    <span className="flex flex-wrap items-baseline gap-x-2">
      What would help
      <span className="text-xs font-normal text-fg-muted">
        {transfer === null
          ? "ranked by when all transfers are done"
          : `ranked for ${transferLabel}`}
        {running ? " · simulating the changes…" : ""}
      </span>
    </span>
  );
  if (!result) {
    return (
      <Panel title={title} className={className}>
        {error ? <p className="text-sm text-fg-muted">{error}</p> : <SkeletonRows rows={6} />}
      </Panel>
    );
  }
  const { helps, flat, hurts, impossible, together } = rank(result, transfer);
  const maxGain = Math.max(0.01, ...[...helps, ...together].map((r) => -(r.delta ?? 0)));
  const list = (items: Scored[]) => (
    <ul>
      {items.map((r) => (
        <ChangeRow
          key={r.change.label}
          row={r}
          base={result.base}
          maxGain={maxGain}
          onApply={onApply}
        />
      ))}
    </ul>
  );
  const group = (summary: string, items: Scored[]): ReactNode =>
    items.length ? (
      <Disclosure summary={summary} count={items.length}>
        {list(items)}
      </Disclosure>
    ) : null;
  return (
    <Panel title={title} className={cn(running && "opacity-80", className)}>
      <p className="mb-2 text-xs text-fg-muted">
        Each line is a separate simulation with one parameter doubled or halved, or one choice
        switched. What tops the list limits this scenario; what has no effect does not.
      </p>
      {helps.length ? (
        list(helps)
      ) : (
        <p className="py-1 text-sm text-fg-muted">
          No single change shortens it by more than 1 %.
          {together.length ? " Two limits coincide: see the pairs below." : ""}
        </p>
      )}
      {together.length ? (
        <div className="mt-3">
          <h4 className="mb-1 text-xs font-medium text-fg-muted">
            Two changes together, better than either alone
          </h4>
          {list(together)}
        </div>
      ) : null}
      <div className="mt-2 flex flex-col gap-1">
        {group("No effect (within 1 %)", flat)}
        {group("Slower", hurts)}
        {group("Not possible or does not finish", impossible)}
      </div>
    </Panel>
  );
}
