import type { ByteRange } from "../api";

/** For each byte, the index of the range covering it, or -1. Later ranges win. */
export function rangeIndexByByte(ranges: readonly ByteRange[], length: number): Int32Array {
  const out = new Int32Array(length).fill(-1);
  ranges.forEach((r, i) => {
    const end = Math.min(length, r.start + r.len);
    for (let b = r.start; b < end; b++) out[b] = i;
  });
  return out;
}

/** Bytes from a lower-case hex string. Odd trailing nibbles are dropped. */
export function hexToBytes(hex: string): Uint8Array {
  const out = new Uint8Array(Math.floor(hex.length / 2));
  for (let i = 0; i < out.length; i++) out[i] = Number.parseInt(hex.slice(2 * i, 2 * i + 2), 16);
  return out;
}

/** Printable ASCII for a byte, `·` otherwise. */
export function printable(b: number): string {
  return b >= 0x20 && b < 0x7f ? String.fromCharCode(b) : "·";
}
