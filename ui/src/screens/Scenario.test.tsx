import { TooltipProvider } from "@vitavision/ui";
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { SimResult, WhatIfResult } from "../api";
import type { Scenario as ScenarioState } from "../scenario/useScenario";
import simFixture from "../test/sim.fixture.json";
import whatIfFixture from "../test/whatif.fixture.json";
import { Scenario } from "./Scenario";

const result = simFixture as unknown as SimResult;
const whatIf = whatIfFixture as unknown as WhatIfResult;

function state(over: Partial<ScenarioState> = {}): ScenarioState {
  return {
    presets: [{ id: "example", title: "Example", spec: result.spec }],
    slaveTemplate: result.spec.slaves[0] ?? null,
    spec: result.spec,
    result,
    resultSpec: result.spec,
    running: false,
    error: null,
    whatIf,
    whatIfRunning: false,
    whatIfError: null,
    baseline: null,
    canUndo: false,
    setSpec: vi.fn(),
    apply: vi.fn(),
    load: vi.fn(),
    undo: vi.fn(),
    pinBaseline: vi.fn(),
    clearBaseline: vi.fn(),
    ...over,
  };
}

function renderScenario(s: ScenarioState) {
  render(
    <TooltipProvider>
      <Scenario scenario={s} onCreated={vi.fn()} />
    </TooltipProvider>,
  );
}

describe("Scenario", () => {
  it("shows the setup in the units people use and edits the spec", () => {
    const s = state();
    renderScenario(s);
    const cycle = screen.getByRole("spinbutton", { name: /^Cycle time/ });
    expect(cycle).toHaveValue(1000);
    fireEvent.change(cycle, { target: { value: "500" } });
    expect(s.setSpec).toHaveBeenLastCalledWith(expect.objectContaining({ cycle_time_us: 500 }));
    expect(screen.getByRole("spinbutton", { name: /^Link rate/ })).toHaveValue(100);
    // The first transfer is open: its protocol and payload.
    expect(screen.getByRole("combobox", { name: "Protocol" })).toHaveTextContent("EoE");
    expect(screen.getByRole("spinbutton", { name: /^Payload/ })).toHaveValue(8);
  });

  it("names what was simulated next to the result and ranks the changes", () => {
    const s = state();
    renderScenario(s);
    expect(screen.getAllByText(/EoE ↓ 8 kB/).length).toBeGreaterThan(0);
    expect(screen.getByText("What would help")).toBeInTheDocument();
    fireEvent.click(screen.getAllByRole("button", { name: "Apply" })[0]!);
    expect(s.apply).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole("button", { name: /Pin as baseline/ }));
    expect(s.pinBaseline).toHaveBeenCalled();
  });

  it("lists the changes since the baseline", () => {
    const changed = { ...result.spec, cycle_time_us: 500 };
    renderScenario(state({ spec: changed, baseline: { spec: result.spec, result } }));
    expect(screen.getByText(/changed: cycle time 1.000 ms → 500.00 µs/)).toBeInTheDocument();
    expect(screen.getByRole("columnheader", { name: "vs. baseline" })).toBeInTheDocument();
  });

  it("keeps the last result visible when an edit does not simulate", () => {
    renderScenario(state({ error: "cycle_time_us must be > 0" }));
    expect(screen.getByText(/cycle_time_us must be > 0/)).toBeInTheDocument();
    expect(screen.getByText(/the result below is of the last setup that ran/)).toBeInTheDocument();
  });
});
