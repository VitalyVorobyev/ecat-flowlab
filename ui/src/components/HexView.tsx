import { seriesColour } from "@vitavision/charts";
import { cn } from "@vitavision/ui";
import { type CSSProperties, useMemo } from "react";

import type { ByteRange } from "../api";
import { hexToBytes, printable, rangeIndexByByte } from "./hexLayout";

const PER_ROW = 16;

/** Tint for a byte range: datagrams take the series palette, framing stays neutral. */
function tint(r: ByteRange | undefined, emphasized: boolean): CSSProperties | undefined {
  if (!r) return undefined;
  if (r.datagram === null) {
    return r.kind === "padding" ? undefined : { backgroundColor: "var(--raised)" };
  }
  const strength = { datagram_header: 26, data: 12, wkc: 34 }[r.kind] ?? 12;
  const pct = emphasized ? Math.min(60, strength * 2) : strength;
  return {
    backgroundColor: `color-mix(in oklab, ${seriesColour(r.datagram)} ${pct}%, transparent)`,
  };
}

/**
 * Hex dump with labelled byte ranges. Hovering a byte names its range; `emphasis` highlights
 * one datagram (e.g. the one hovered in the decoded view).
 */
export function HexView({
  hex,
  ranges,
  emphasis,
  onHoverDatagram,
}: {
  hex: string;
  ranges: readonly ByteRange[];
  emphasis?: number | null;
  onHoverDatagram?: (datagram: number | null) => void;
}) {
  const bytes = useMemo(() => hexToBytes(hex), [hex]);
  const index = useMemo(() => rangeIndexByByte(ranges, bytes.length), [ranges, bytes.length]);
  const rows = Math.ceil(bytes.length / PER_ROW);

  const cell = (b: number, ascii: boolean) => {
    const ri = index[b] ?? -1;
    const r = ri >= 0 ? ranges[ri] : undefined;
    const emph = r?.datagram !== null && r?.datagram !== undefined && r.datagram === emphasis;
    const value = bytes[b] ?? 0;
    return (
      <span
        key={b}
        title={r ? `${r.label} · byte ${b}` : `byte ${b}`}
        style={tint(r, emph)}
        onMouseEnter={() => onHoverDatagram?.(r?.datagram ?? null)}
        className={cn(
          "inline-block text-center",
          ascii ? "w-[1ch]" : "w-[2.6ch]",
          r?.kind === "padding" && "text-fg-subtle",
          !ascii && b % 8 === 7 && b % PER_ROW !== PER_ROW - 1 && "mr-[1ch]",
        )}
      >
        {ascii ? printable(value) : value.toString(16).padStart(2, "0")}
      </span>
    );
  };

  return (
    <div
      className="overflow-x-auto rounded-md border border-line bg-surface p-2 font-mono text-xs leading-5"
      onMouseLeave={() => onHoverDatagram?.(null)}
      role="group"
      aria-label="Frame bytes"
    >
      {Array.from({ length: rows }, (_, row) => {
        const start = row * PER_ROW;
        const end = Math.min(bytes.length, start + PER_ROW);
        const idx = Array.from({ length: end - start }, (_, k) => start + k);
        return (
          <div key={row} className="flex gap-3 whitespace-pre">
            <span className="w-[4ch] shrink-0 text-fg-subtle">
              {start.toString(16).padStart(4, "0")}
            </span>
            <span className="w-[44ch] shrink-0">{idx.map((b) => cell(b, false))}</span>
            <span className="text-fg-muted">{idx.map((b) => cell(b, true))}</span>
          </div>
        );
      })}
    </div>
  );
}
