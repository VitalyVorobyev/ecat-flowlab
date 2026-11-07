import { TooltipProvider } from "@vitavision/ui";
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { SimResult } from "../api";
import fixture from "../test/sim.fixture.json";
import { Simulation } from "./Simulation";

// Output of `flowlab simulate` (see ui/src/test/README.md).
const result = fixture as unknown as SimResult;

function renderSimulation(r: SimResult = result, onEdit?: (spec: SimResult["spec"]) => void) {
  render(
    <TooltipProvider>
      <Simulation result={r} {...(onEdit ? { onEdit } : {})} />
    </TooltipProvider>,
  );
}

describe("Simulation", () => {
  it("is labelled as a model and says what was simulated", () => {
    renderSimulation();
    expect(screen.getAllByText("Simulated").length).toBeGreaterThan(0);
    expect(screen.getAllByText(/a model prediction, not a measurement/).length).toBeGreaterThan(0);
    expect(screen.getByText("What was simulated")).toBeInTheDocument();
    expect(screen.getByText(/fixed quota, 2 mailbox datagrams per cycle/)).toBeInTheDocument();
    expect(screen.getAllByText(/EoE ↓ 8 kB to bulk-endpoint/).length).toBeGreaterThan(0);
  });

  it("lists every transfer with its integrity check", () => {
    renderSimulation();
    expect(result.transfers.length).toBeGreaterThan(1);
    expect(screen.getAllByText("verified")).toHaveLength(result.transfers.length);
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

  it("opens the run's spec for editing", () => {
    const edit = vi.fn();
    renderSimulation(result, edit);
    fireEvent.click(screen.getByRole("button", { name: "Edit in Simulate" }));
    expect(edit).toHaveBeenCalledWith(result.spec);
  });
});
