import { TooltipProvider } from "@vitavision/ui";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { api } from "../api";
import { fillSeededBytes, sha256Hex } from "../lib/sha256";
import { ThroughputLab } from "./ThroughputLab";

afterEach(() => vi.restoreAllMocks());

describe("ThroughputLab", () => {
  it("runs latency probe and renders RTT metrics", async () => {
    const user = userEvent.setup();
    vi.spyOn(api, "benchPing").mockResolvedValue({ server_time_ns: 123456 });

    render(
      <TooltipProvider>
        <ThroughputLab />
      </TooltipProvider>,
    );

    await user.click(screen.getByRole("button", { name: "Probe RTT (20×)" }));
    expect(await screen.findByText("RTT per request (20 pings)")).toBeInTheDocument();
  });

  it("downloads payload and verifies matching SHA-256", async () => {
    const user = userEvent.setup();
    const buf = new Uint8Array(1024 * 1024);
    fillSeededBytes(1, buf);
    const digest = sha256Hex(buf);

    vi.spyOn(api, "benchDownload").mockResolvedValue(
      new Response(buf, {
        status: 200,
        headers: { "x-sha256": digest },
      }),
    );

    render(
      <TooltipProvider>
        <ThroughputLab />
      </TooltipProvider>,
    );

    await user.click(screen.getByRole("radio", { name: "Download ↓" }));
    await user.click(screen.getByRole("button", { name: "Run transfer" }));

    await waitFor(() => expect(screen.getByText("Recorded transfers (1)")).toBeInTheDocument());
    expect(screen.getByText("OK")).toBeInTheDocument();
  });
});
