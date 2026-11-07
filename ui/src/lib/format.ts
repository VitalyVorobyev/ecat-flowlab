/** Number and time formatting. Units are always shown; decimal and binary units never mix. */

const group = new Intl.NumberFormat("en-US");

/** Integer with thousands separators: `10,157`. */
export function count(n: number | bigint): string {
  return group.format(n);
}

/** A duration given in ns, in the largest unit that keeps it ≥ 1: `1.000 ms`, `12.24 µs`. */
export function ns(value: number, digits?: number): string {
  if (!Number.isFinite(value)) return "–";
  for (const { div, unit, d } of [
    { div: 1e9, unit: "s", d: 3 },
    { div: 1e6, unit: "ms", d: 3 },
    { div: 1e3, unit: "µs", d: 2 },
  ]) {
    // Decide on the value as displayed, so 999,999.9 ns reads "1.000 ms", not "1000.00 µs".
    const shown = (value / div).toFixed(digits ?? d);
    if (Math.abs(Number(shown)) >= 1) return `${shown} ${unit}`;
  }
  return `${value.toFixed(digits ?? 0)} ns`;
}

/** Same as `ns`, with an explicit sign for deviations: `+4.10 µs`. */
export function signedNs(value: number): string {
  return `${value > 0 ? "+" : value < 0 ? "−" : "±"}${ns(Math.abs(value))}`;
}

/** Byte size in decimal units: `1.24 MB`. */
export function bytes(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(2)} GB`;
  if (n >= 1e6) return `${(n / 1e6).toFixed(2)} MB`;
  if (n >= 1e3) return `${(n / 1e3).toFixed(1)} kB`;
  return `${n} B`;
}

/** Bit rate in decimal units: `219.9 kbit/s`, `4.20 Mbit/s`. */
export function bitRate(bps: number): string {
  if (!Number.isFinite(bps)) return "–";
  if (bps >= 1e9) return `${(bps / 1e9).toFixed(2)} Gbit/s`;
  if (bps >= 1e6) return `${(bps / 1e6).toFixed(2)} Mbit/s`;
  if (bps >= 1e3) return `${(bps / 1e3).toFixed(1)} kbit/s`;
  return `${bps.toFixed(0)} bit/s`;
}

/** Percentage with adaptive precision: `0.042 %`, `1.24 %`, `37.5 %`. */
export function percent(p: number): string {
  if (!Number.isFinite(p)) return "–";
  const d = p < 0.1 ? 3 : p < 10 ? 2 : 1;
  return `${p.toFixed(d)} %`;
}

/** Hex with a fixed width: `0x1001`. */
export function hex(n: number, width = 4): string {
  return `0x${n.toString(16).toUpperCase().padStart(width, "0")}`;
}

/** Time since capture start in seconds with µs resolution: `1.234567 s`. */
export function captureTime(tNs: number): string {
  return `${(tNs / 1e9).toFixed(6)} s`;
}

/** ISO date-time (UTC) from ms since the epoch. */
export function utc(ms: number): string {
  return new Date(ms)
    .toISOString()
    .replace("T", " ")
    .replace(/\.\d+Z$/, " UTC");
}

/** Pick a time unit for an axis spanning `maxNs`, and the divisor to reach it. */
export function timeAxis(maxNs: number): { unit: string; div: number } {
  const a = Math.abs(maxNs);
  if (a >= 1e9) return { unit: "s", div: 1e9 };
  if (a >= 1e6) return { unit: "ms", div: 1e6 };
  if (a >= 1e3) return { unit: "µs", div: 1e3 };
  return { unit: "ns", div: 1 };
}
