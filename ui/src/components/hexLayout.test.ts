import { describe, expect, it } from "vitest";

import type { ByteRange } from "../api";
import { hexToBytes, printable, rangeIndexByByte } from "./hexLayout";

const r = (start: number, len: number): ByteRange => ({
  start,
  len,
  label: "",
  kind: "data",
  datagram: null,
});

describe("hex layout", () => {
  it("maps bytes to ranges", () => {
    const idx = rangeIndexByByte([r(0, 2), r(2, 3), r(4, 10)], 6);
    expect(Array.from(idx)).toEqual([0, 0, 1, 1, 2, 2]);
    expect(Array.from(rangeIndexByByte([], 2))).toEqual([-1, -1]);
  });

  it("decodes hex and ASCII", () => {
    expect(Array.from(hexToBytes("88a4ff"))).toEqual([0x88, 0xa4, 0xff]);
    expect(printable(0x41)).toBe("A");
    expect(printable(0x00)).toBe("·");
  });
});
