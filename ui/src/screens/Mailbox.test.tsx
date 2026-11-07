import { TooltipProvider } from "@vitavision/ui";
import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { CaptureReport } from "../api";
import fixture from "../test/report.fixture.json";
import { Mailbox } from "./Mailbox";

const report = fixture as unknown as CaptureReport;

function renderMailbox(r: CaptureReport = report) {
  const onOpenFrame = vi.fn();
  render(
    <TooltipProvider>
      <Mailbox captureId={1} report={r} onOpenFrame={onOpenFrame} />
    </TooltipProvider>,
  );
  return { onOpenFrame };
}

describe("Mailbox", () => {
  it("lists every mailbox channel with its SyncManager addresses", () => {
    renderMailbox();
    const m = report.mailbox;
    expect(m.channels.length).toBeGreaterThan(0);
    for (const c of m.channels) {
      const station = `0x${c.station.toString(16).toUpperCase()}`;
      expect(screen.getAllByText(station).length).toBeGreaterThan(0);
    }
    expect(screen.getAllByText("0x1000 · 128 B").length).toBe(m.channels.length);
  });

  it("judges SDO aborts against the any-abort threshold", () => {
    renderMailbox();
    const tile = screen.getByText("SDO aborts").closest("div.rounded-md") as HTMLElement;
    const value = within(tile).getByText(String(report.mailbox.coe.aborted));
    expect(value.className).toContain(report.mailbox.coe.aborted > 0 ? "text-warn" : "text-normal");
  });

  it("opens the first frame of a transfer", () => {
    const { onOpenFrame } = renderMailbox();
    const first = report.mailbox.coe.transactions[0];
    expect(first).toBeDefined();
    const time = `${((first?.start_ns ?? 0) / 1e9).toFixed(6)} s`;
    fireEvent.click(screen.getAllByText(time)[0] as HTMLElement);
    expect(onOpenFrame).toHaveBeenCalledWith(first?.first_frame);
  });

  it("offers the reassembled EoE frames for download", () => {
    renderMailbox();
    const link = screen.getByRole("link", { name: /EoE frames/ });
    expect(link.getAttribute("href")).toBe("/api/captures/1/eoe.pcapng");
  });

  it("shows how the master serves the mailbox, as a simulator policy", () => {
    renderMailbox();
    const served = report.mailbox.channels.filter((c) => c.service !== null);
    expect(served.length).toBe(1);
    const pattern = served[0]?.service?.pattern;
    expect(pattern?.kind).toBe("fixed_quota");
    screen.getByText("How the master serves the mailbox");
    expect(screen.getByText("fixed quota, 1 per cycle")).toBeInTheDocument();
  });

  it("explains an empty mailbox", () => {
    renderMailbox({ ...report, mailbox: { ...report.mailbox, channels: [] } });
    expect(screen.getByText(/No mailbox traffic/)).toBeInTheDocument();
  });
});
