import { describe, expect, it } from "vitest";

import { bytes, count, hex, ns, percent, signedNs, timeAxis } from "./format";

describe("format", () => {
  it("scales durations", () => {
    expect(ns(1_000_000)).toBe("1.000 ms");
    expect(ns(12_240)).toBe("12.24 µs");
    expect(ns(840)).toBe("840 ns");
    expect(ns(5_017_000_000)).toBe("5.017 s");
    expect(ns(Number.NaN)).toBe("–");
    expect(signedNs(-4100)).toBe("−4.10 µs");
  });

  it("formats counts, bytes, percent and hex", () => {
    expect(count(10157)).toBe("10,157");
    expect(bytes(1_240_000)).toBe("1.24 MB");
    expect(bytes(512)).toBe("512 B");
    expect(percent(0.0421)).toBe("0.042 %");
    expect(percent(37.51)).toBe("37.5 %");
    expect(hex(0x1001)).toBe("0x1001");
    expect(hex(0x10000, 8)).toBe("0x00010000");
  });

  it("picks axis units", () => {
    expect(timeAxis(5e9)).toEqual({ unit: "s", div: 1e9 });
    expect(timeAxis(2e4)).toEqual({ unit: "µs", div: 1e3 });
  });
});
