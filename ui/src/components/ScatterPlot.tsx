import {
  areaFor,
  extent,
  Frame,
  Legend,
  linearScale,
  padDomain,
  seriesColour,
  useFluidSize,
} from "@vitavision/charts";

export type ScatterPoint = { id: number; x: number; y: number; highlight: boolean };

/**
 * Points in two dimensions, highlighted ones in the first series colour and the rest in a
 * neutral tone. Built here from the charts primitives; a candidate for `@vitavision/charts`.
 */
export function ScatterPlot({
  points,
  label,
  xLabel,
  yLabel,
  highlightLabel,
  otherLabel,
  selected,
  onSelect,
  height = 260,
}: {
  points: ScatterPoint[];
  label: string;
  xLabel: string;
  yLabel: string;
  highlightLabel: string;
  otherLabel: string;
  selected?: number | null;
  onSelect?: (id: number) => void;
  height?: number;
}) {
  const [ref, size] = useFluidSize(height);
  const area = areaFor("fluid", size);
  const finite = points.filter((p) => Number.isFinite(p.x) && Number.isFinite(p.y));
  const [x0, x1] = extent(finite.map((p) => p.x));
  const [y0, y1] = extent(finite.map((p) => p.y));
  const xScale = linearScale(padDomain(x0, x1), area.x0, area.x1);
  const yScale = linearScale(padDomain(y0, y1), area.y0, area.y1);
  // Neutral points first so highlighted ones are drawn on top.
  const ordered = [...finite].sort((a, b) => Number(a.highlight) - Number(b.highlight));
  return (
    <Frame
      label={label}
      xLabel={xLabel}
      yLabel={yLabel}
      xScale={xScale}
      yScale={yScale}
      variant="fluid"
      size={size}
      figureRef={ref}
      footer={
        <Legend
          items={[
            { label: highlightLabel, colour: seriesColour(0) },
            { label: otherLabel, colour: "var(--fg-subtle)" },
          ]}
        />
      }
    >
      {ordered.map((p) => (
        <circle
          key={p.id}
          cx={xScale.project(p.x)}
          cy={yScale.project(p.y)}
          r={selected === p.id ? 6 : p.highlight ? 4.5 : 3.5}
          fill={p.highlight ? seriesColour(0) : "var(--fg-subtle)"}
          fillOpacity={p.highlight ? 0.95 : 0.55}
          stroke={selected === p.id ? "var(--fg)" : "none"}
          strokeWidth={1.5}
          className={onSelect ? "cursor-pointer" : undefined}
          onClick={onSelect ? () => onSelect(p.id) : undefined}
        >
          <title>{`run ${p.id}: ${p.x.toPrecision(4)}, ${p.y.toPrecision(4)}`}</title>
        </circle>
      ))}
    </Frame>
  );
}
