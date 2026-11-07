import { TooltipProvider } from "@vitavision/ui";
import { render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { api, type CaptureInfo, type CaptureReport } from "../api";
import fixture from "../test/report.fixture.json";
import { Compare } from "./Compare";

const report = fixture as unknown as CaptureReport;
const info = (id: number, provenance: CaptureInfo["provenance"]): CaptureInfo =>
  ({ id, name: `c${id}.pcapng`, provenance }) as CaptureInfo;

afterEach(() => vi.restoreAllMocks());

describe("Compare", () => {
  it("keeps each column's provenance", async () => {
    const simulated: CaptureReport = {
      ...report,
      meta: {
        ...report.meta,
        source: "simulated",
        input: { ...report.meta.input, name: "sim.pcapng" },
      },
    };
    vi.spyOn(api, "report").mockImplementation((id) =>
      Promise.resolve(id === 1 ? report : simulated),
    );
    render(
      <TooltipProvider>
        <Compare captures={[info(1, "synthetic"), info(2, "simulated")]} />
      </TooltipProvider>,
    );
    expect(await screen.findByText("sim.pcapng")).toBeInTheDocument();
    expect(screen.getAllByText("Simulated").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Synthetic").length).toBeGreaterThan(0);
    // Same report on both sides: every ratio is 1.
    expect(screen.getAllByText("1.000×").length).toBeGreaterThan(5);
  });

  it("asks for two captures", () => {
    render(<Compare captures={[info(1, "measured")]} />);
    expect(screen.getByText(/Load two captures/)).toBeInTheDocument();
  });
});
