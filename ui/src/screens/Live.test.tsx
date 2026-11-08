import { TooltipProvider } from "@vitavision/ui";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { api, type CaptureInfo, type CaptureReport, type LiveSnapshot } from "../api";
import reportFixture from "../test/report.fixture.json";
import { Live } from "./Live";

const report = reportFixture as unknown as CaptureReport;

const capture: CaptureInfo = {
  id: 7,
  name: "live-1.pcapng",
  provenance: "measured",
  format: "pcapng",
  bytes: 1000,
  frames: 4000,
  ethercat_frames: 4000,
  duration_ns: 2e9,
  streams: 1,
  defects: 0,
  warnings: 0,
  sha256: "ab",
  has_simulation: false,
  eni: null,
};

function snapshot(over: Partial<LiveSnapshot> = {}): LiveSnapshot {
  const point = {
    frames: 2000,
    tx_percent: 1.2,
    period_ns: 1e6,
    jitter_p99_ns: 3200,
    round_trip_p99_ns: 41000,
    wkc_mismatches: 0,
    unanswered: 1,
    missed_cycles: 0,
    mailbox_messages: 4,
    malformed: 0,
  };
  return {
    state: "running",
    source: "dumpcap on eth1",
    message: null,
    provenance: "measured",
    elapsed_s: 2.1,
    packets: 4000,
    bytes: 400000,
    saved_to: "results/live/live-1.pcapng",
    window_ns: 10e9,
    totals: {
      ethercat_frames: 4000,
      wkc_mismatches: 0,
      unanswered: 1,
      missed_cycles: 0,
      mailbox_messages: 8,
      malformed: 0,
    },
    series: [
      { t_s: 1, ...point },
      { t_s: 2, ...point },
    ],
    window: report,
    assumptions: ["Rolling analysis."],
    ...over,
  };
}

function renderLive(onSaved = vi.fn(), onOpen = vi.fn()) {
  render(
    <TooltipProvider>
      <Live captures={[capture]} onSaved={onSaved} onOpen={onOpen} />
    </TooltipProvider>,
  );
  return { onSaved, onOpen };
}

afterEach(() => vi.restoreAllMocks());

describe("Live", () => {
  it("says how to allow interface capture and offers a replay", async () => {
    vi.spyOn(api, "live").mockResolvedValue(null);
    vi.spyOn(api, "liveInterfaces").mockResolvedValue({
      enabled: false,
      tool: "dumpcap",
      interfaces: [{ name: "eth1", description: null }],
      error: null,
    });
    renderLive();
    expect(await screen.findByText("flowlab serve --live")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Start" })).toBeDisabled();
    expect(screen.getByRole("radio", { name: "Replay a capture" })).toBeChecked();
  });

  it("shows a running capture and loads it when stopped", async () => {
    vi.spyOn(api, "live").mockResolvedValue(snapshot());
    vi.spyOn(api, "liveInterfaces").mockResolvedValue({
      enabled: true,
      tool: "dumpcap",
      interfaces: [],
      error: null,
    });
    const stop = vi
      .spyOn(api, "liveStop")
      .mockResolvedValue({ snapshot: snapshot({ state: "finished" }), capture });
    const { onSaved, onOpen } = renderLive();
    expect(await screen.findByText("dumpcap on eth1")).toBeInTheDocument();
    expect(screen.getByText("1.000 ms")).toBeInTheDocument();
    expect(screen.getByText("3.20 µs")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Start" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Stop" }));
    await waitFor(() => expect(stop).toHaveBeenCalled());
    expect(onSaved).toHaveBeenCalledWith(capture);
    fireEvent.click(await screen.findByRole("button", { name: "Open the saved capture" }));
    expect(onOpen).toHaveBeenCalledWith(7);
  });
});
