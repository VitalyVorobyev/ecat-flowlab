import { seriesColour } from "@vitavision/charts";

import type { AlEvent } from "../api";
import { hex, ns } from "../lib/format";

const STATES = ["INIT", "PREOP", "BOOT", "SAFEOP", "OP"] as const;

function colour(state: string): string {
  const i = STATES.indexOf(state as (typeof STATES)[number]);
  return i < 0 ? "var(--fg-subtle)" : seriesColour(i);
}

type Segment = { from: number; to: number; state: string; error: boolean };

/** Reported-state segments per station over `[0, end]`, from change events. */
export function segments(events: readonly AlEvent[], end: number): Map<number, Segment[]> {
  const out = new Map<number, Segment[]>();
  for (const e of events) {
    if (e.kind !== "reported" || e.station === null) continue;
    const list = out.get(e.station) ?? [];
    const last = list[list.length - 1];
    if (last) last.to = e.t_ns;
    list.push({ from: e.t_ns, to: end, state: e.state, error: e.error });
    out.set(e.station, list);
  }
  return out;
}

/**
 * One lane per station showing the AL state it reported over time, with ticks where the
 * master requested a state.
 */
export function StateTimeline({
  stations,
  events,
  end,
}: {
  stations: readonly number[];
  events: readonly AlEvent[];
  end: number;
}) {
  const segs = segments(events, Number.MAX_SAFE_INTEGER);
  const span = Math.max(end, 1);
  const pct = (t: number) => `${(100 * Math.min(t, span)) / span}%`;
  return (
    <div className="flex flex-col gap-1.5">
      {stations.map((st) => (
        <div key={st} className="flex items-center gap-3">
          <span className="w-16 shrink-0 font-mono text-xs text-fg-muted">{hex(st)}</span>
          <div className="relative h-5 flex-1 overflow-hidden rounded-sm bg-raised">
            {(segs.get(st) ?? [])
              .filter((s) => s.from < span)
              .map((s) => (
                <div
                  key={s.from}
                  className="absolute inset-y-0"
                  title={`${s.state}${s.error ? " + error" : ""} from ${ns(s.from)}`}
                  style={{
                    left: pct(s.from),
                    width: `calc(${pct(Math.min(s.to, span))} - ${pct(s.from)})`,
                    backgroundColor: `color-mix(in oklab, ${colour(s.state)} ${s.error ? 90 : 55}%, transparent)`,
                  }}
                />
              ))}
            {events
              .filter(
                (e) =>
                  e.kind === "requested" &&
                  (e.station === st || e.station === null) &&
                  e.t_ns <= span,
              )
              .map((e) => (
                <div
                  key={`r${e.frame}`}
                  className="absolute inset-y-0 w-px bg-fg"
                  style={{ left: pct(e.t_ns) }}
                  title={`requested ${e.state} at ${ns(e.t_ns)} (frame ${e.frame})`}
                />
              ))}
          </div>
        </div>
      ))}
      <div className="flex items-center gap-3">
        <span className="w-16 shrink-0" />
        <div className="flex flex-1 justify-between font-mono text-[10px] text-fg-subtle">
          <span>0</span>
          <span>{ns(span / 2)}</span>
          <span>{ns(span)}</span>
        </div>
      </div>
      <div className="mt-1 flex flex-wrap items-center gap-3 text-xs text-fg-muted">
        {STATES.map((s) => (
          <span key={s} className="flex items-center gap-1.5">
            <span
              className="inline-block size-2.5 rounded-sm"
              style={{ backgroundColor: colour(s) }}
            />
            {s}
          </span>
        ))}
        <span className="flex items-center gap-1.5">
          <span className="inline-block h-3 w-px bg-fg" /> requested by master
        </span>
      </div>
    </div>
  );
}
