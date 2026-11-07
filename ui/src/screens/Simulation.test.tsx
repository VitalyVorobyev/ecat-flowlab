import { TooltipProvider } from "@vitavision/ui";
import { render, screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { SimResult } from "../api";
import fixture from "../test/sim.fixture.json";
import { Simulation } from "./Simulation";

// Output of `flowlab simulate` (see ui/src/test/README.md).
const result = fixture as unknown as SimResult;

function renderSimulation(r: SimResult = result) {
  render(
    <TooltipProvider>
      <Simulation result={r} />
    </TooltipProvider>,
  );
}

describe("Simulation", () => {
  it("is labelled as a model, not a measurement", () => {
    renderSimulation();
    expect(screen.getAllByText("Simulated").length).toBeGreaterThan(0);
    expect(screen.getByText("Model output")).toBeInTheDocument();
  });

  it("lists every transfer with its integrity check", () => {
    renderSimulation();
    expect(result.transfers.length).toBeGreaterThan(1);
    expect(screen.getAllByText("SHA-256 ok")).toHaveLength(result.transfers.length);
    const tile = screen.getByText("Transfers verified").closest("div.rounded-md") as HTMLElement;
    expect(
      within(tile).getByText(`${result.transfers.length} / ${result.transfers.length}`),
    ).toHaveClass("text-normal");
  });

  it("flags a transfer that did not verify", () => {
    const [first, ...rest] = result.transfers;
    expect(first).toBeDefined();
    renderSimulation({
      ...result,
      transfers: [
        { ...(first as SimResult["transfers"][number]), verified: false, completed: false },
        ...rest,
      ],
    });
    expect(screen.getByText("incomplete")).toBeInTheDocument();
  });
});
