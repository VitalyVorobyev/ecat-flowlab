import { describe, expect, it } from "vitest";

import type { SimResult } from "../api";
import fixture from "../test/sim.fixture.json";
import {
  applyEdits,
  changeText,
  diffSpecs,
  payload,
  payloadUnit,
  removeSlave,
  transferLine,
} from "./spec";

const spec = (fixture as unknown as SimResult).spec;

describe("scenario spec helpers", () => {
  it("shows payloads in the unit they were most likely entered in", () => {
    expect(payload(10_000_000)).toBe("10 MB");
    expect(payload(65_536)).toBe("64 KiB");
    expect(payload(1500)).toBe("1500 B");
    expect(payload(1_500_000)).toBe("1500 kB");
    expect(payload(1501)).toBe("1501 B");
    expect(payloadUnit(10 * 1024 * 1024).label).toBe("MiB");
  });

  it("applies what-if edits by JSON pointer without touching the original", () => {
    const next = applyEdits(spec, [
      { path: "/slaves/2/pdi/read_bps", value: 40_000_000 },
      { path: "/mailbox_policy", value: { kind: "round_robin", slaves_per_cycle: 1 } },
      { path: "/no/such/path", value: 1 },
    ]);
    expect(next.slaves[2]?.pdi.read_bps).toBe(40_000_000);
    expect(next.mailbox_policy).toEqual({ kind: "round_robin", slaves_per_cycle: 1 });
    expect(spec.slaves[2]?.pdi.read_bps).toBe(20_000_000);
  });

  it("describes the changes since a baseline in units", () => {
    const next = applyEdits(spec, [
      { path: "/slaves/2/pdi/read_bps", value: 40_000_000 },
      { path: "/cycle_time_us", value: 500 },
    ]);
    const text = diffSpecs(spec, next).map((c) => changeText(c, next));
    expect(text).toContain("cycle time 1.000 ms → 500.00 µs");
    expect(text).toContain("bulk-endpoint: PDI read rate 20.00 Mbit/s → 40.00 Mbit/s");
  });

  it("removes a slave with its transfers and renumbers the rest", () => {
    const next = removeSlave(spec, 0);
    expect(next.slaves).toHaveLength(spec.slaves.length - 1);
    expect(next.transfers.every((t) => t.slave === 1)).toBe(true);
    expect(transferLine(next.transfers[0]!, next)).toBe("EoE ↓ 8 kB to bulk-endpoint");
  });
});
