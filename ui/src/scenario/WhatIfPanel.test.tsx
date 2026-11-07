import { TooltipProvider } from "@vitavision/ui";
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { WhatIfResult } from "../api";
import fixture from "../test/whatif.fixture.json";
import { NO_EFFECT, rank, WhatIfPanel } from "./WhatIfPanel";

// Output of `flowlab what-if` (see ui/src/test/README.md).
const result = fixture as unknown as WhatIfResult;

describe("what-if ranking", () => {
  it("sorts single changes into helps, no effect, slower and not possible", () => {
    const r = rank(result, null);
    expect(r.helps.length).toBeGreaterThan(0);
    const deltas = r.helps.map((s) => s.delta ?? 0);
    expect(deltas).toEqual([...deltas].sort((a, b) => a - b));
    expect(deltas.every((d) => d < -NO_EFFECT)).toBe(true);
    expect(r.flat.every((s) => Math.abs(s.delta ?? 1) <= NO_EFFECT)).toBe(true);
    expect(r.impossible.every((s) => !s.change.run.feasible)).toBe(true);
    const all = [...r.helps, ...r.flat, ...r.hurts, ...r.impossible];
    expect(all.every((s) => s.change.parts.length === 0)).toBe(true);
  });

  it("keeps only pairs that beat both of their parts", () => {
    const r = rank(result, null);
    const single = new Map(
      [...r.helps, ...r.flat, ...r.hurts].map((s) => [s.change.label, s.delta ?? 0]),
    );
    for (const p of r.together) {
      for (const part of p.change.parts) {
        expect(p.delta ?? 0).toBeLessThan(Math.min(single.get(part) ?? 0, 0));
      }
    }
  });

  it("ranks for one transfer without the other transfers' protocol switches", () => {
    const r = rank(result, 3);
    const all = [...r.helps, ...r.flat, ...r.hurts, ...r.impossible];
    expect(all.every((s) => s.change.transfer === null || s.change.transfer === 3)).toBe(true);
  });
});

describe("WhatIfPanel", () => {
  it("applies the change it shows", () => {
    const apply = vi.fn();
    render(
      <TooltipProvider>
        <WhatIfPanel
          result={result}
          running={false}
          error={null}
          transfer={null}
          transferLabel={null}
          onApply={apply}
        />
      </TooltipProvider>,
    );
    const best = rank(result, null).helps[0];
    expect(best).toBeDefined();
    expect(screen.getByText(best!.change.label)).toBeInTheDocument();
    fireEvent.click(screen.getAllByRole("button", { name: "Apply" })[0]!);
    expect(apply).toHaveBeenCalledWith(best!.change.edits);
  });
});
