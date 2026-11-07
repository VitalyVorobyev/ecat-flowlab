import { TooltipProvider } from "@vitavision/ui";
import { render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { CaptureReport } from "../api";
import fixture from "../test/report.fixture.json";
import { Overview } from "./Overview";

// A report produced by `flowlab analyze` on a synthetic capture (see ui/src/test/README.md).
const report = fixture as unknown as CaptureReport;

function renderOverview() {
  const onOpenStream = vi.fn();
  render(
    <TooltipProvider>
      <Overview report={report} onOpenStream={onOpenStream} />
    </TooltipProvider>,
  );
  return { onOpenStream };
}

describe("Overview", () => {
  it("labels the data source", () => {
    renderOverview();
    expect(screen.getAllByText("Synthetic").length).toBeGreaterThan(0);
  });

  it("shows headline numbers from the report", () => {
    renderOverview();
    const stream = report.streams[0];
    expect(stream).toBeDefined();
    const lost = screen.getByText("Lost frames").closest("div.rounded-md");
    expect(lost).not.toBeNull();
    expect(within(lost as HTMLElement).getByText(String(stream?.unanswered))).toBeInTheDocument();
    const period = screen.getByText("Cycle period").closest("div.rounded-md");
    expect(within(period as HTMLElement).getByText("1.000 ms")).toBeInTheDocument();
  });

  it("lists defects as findings", () => {
    renderOverview();
    const defects = report.findings.filter((f) => f.severity === "defect");
    expect(defects.length).toBeGreaterThan(0);
    for (const f of defects) expect(screen.getByText(f.title)).toBeInTheDocument();
  });

  it("states its assumptions", () => {
    renderOverview();
    expect(screen.getByText("How these numbers were computed")).toBeInTheDocument();
  });
});
