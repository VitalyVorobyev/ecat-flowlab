import { TooltipProvider } from "@vitavision/ui";
import { render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { api, type PlanResult } from "../api";
import fixture from "../test/plan.fixture.json";
import { Planner } from "./Planner";

const result = fixture as unknown as PlanResult;

afterEach(() => vi.restoreAllMocks());

describe("Planner", () => {
  it("shows the budget the server computed", async () => {
    vi.spyOn(api, "planSchema").mockResolvedValue({
      type: "object",
      properties: { cycle_time_us: { type: "integer", minimum: 0 } },
    });
    vi.spyOn(api, "planExample").mockResolvedValue({ cycle_time_us: 1000, slaves: [] });
    const plan = vi.spyOn(api, "plan").mockResolvedValue(result);
    render(
      <TooltipProvider>
        <Planner />
      </TooltipProvider>,
    );
    expect(
      await screen.findByText("Cyclic frames back", {}, { timeout: 2000 }),
    ).toBeInTheDocument();
    expect(plan).toHaveBeenCalledWith({ cycle_time_us: 1000, slaves: [] });
    for (const c of result.checks) expect(screen.getAllByText(c.message).length).toBeGreaterThan(0);
    expect(screen.getByText(`${result.frames[0]?.frame_bytes} B`)).toBeInTheDocument();
  });
});
