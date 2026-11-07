import { Histogram, LineChart, type Series } from "@vitavision/charts";

import type { Distribution, Point } from "../api";
import { timeAxis } from "../lib/format";

/** A pre-binned time distribution (ns) as a histogram in a readable unit. */
export function TimeHistogram({
  d,
  label,
  xLabel,
  markers,
  logY,
  variant = "fluid",
}: {
  d: Distribution;
  label: string;
  xLabel: string;
  markers?: { position: number; label: string }[];
  logY?: boolean;
  variant?: "panel" | "wide" | "fluid";
}) {
  const h = d.histogram;
  const hi = h.lo + h.bin_width * h.counts.length;
  const { unit, div } = timeAxis(Math.max(Math.abs(h.lo), Math.abs(hi)));
  return (
    <Histogram
      label={label}
      xLabel={xLabel}
      unit={unit}
      yLabel="count"
      counts={h.counts}
      domain={[h.lo / div, hi / div]}
      logY={logY ?? false}
      variant={variant}
      height={220}
      markers={(markers ?? []).map((m) => ({ position: m.position / div, label: m.label }))}
    />
  );
}

/** Time series of ns values against capture time. */
export function TimeSeries({
  series,
  label,
  yLabel,
  yIsTime = true,
  yFromZero = false,
  height = 220,
}: {
  series: { name: string; points: Point[] }[];
  label: string;
  yLabel: string;
  yIsTime?: boolean;
  /** Start the y axis at 0 (for loads and counts, where the baseline matters). */
  yFromZero?: boolean;
  height?: number;
}) {
  const all = series.flatMap((s) => s.points);
  const tMax = all.reduce((m, p) => Math.max(m, p.t_ns), 0);
  const vMax = all.reduce((m, p) => Math.max(m, Math.abs(p.v)), 0);
  const x = timeAxis(tMax);
  const y = yIsTime ? timeAxis(vMax) : { unit: "", div: 1 };
  const data: Series[] = series.map((s) => ({
    name: s.name,
    points: s.points.map((p) => ({ x: p.t_ns / x.div, y: p.v / y.div })),
  }));
  return (
    <LineChart
      label={label}
      series={data}
      xLabel={`capture time (${x.unit})`}
      yLabel={y.unit ? `${yLabel} (${y.unit})` : yLabel}
      variant="fluid"
      height={height}
      showLegend={series.length > 1}
      {...(yFromZero
        ? { yDomain: [0, Math.max(vMax / y.div, 1e-9) * 1.1] as [number, number] }
        : {})}
    />
  );
}
