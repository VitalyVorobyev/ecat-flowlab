import { TooltipProvider } from "@vitavision/ui";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { api, type Capabilities, type PdiBenchResult } from "../api";
import { SpiBench } from "./SpiBench";

const CAPS: Capabilities = {
  version: "0.1.0",
  platform: { os: "linux", arch: "aarch64", hostname: "verdin" },
  default_view: "slave",
  local_paths: false,
  live_interfaces: false,
  slave: {
    pdi_enabled: true,
    eoe_bridge_supported: false,
    default_device: null,
    sim_pdi: true,
    allow_replace_app: false,
    allow_writes: false,
    allow_phy_writes: false,
    spidev_devices: [],
    net_interfaces: [],
  },
};

const BENCH_RES: PdiBenchResult = {
  provenance: "simulated",
  target: "simulated LAN9252 @ 20.00 MHz",
  link: { chip_id: 0x9252, revision: 1, al_state: "INIT", id_rev: 0x92520001, hw_cfg: 0x08000000 },
  clock_hz: 20_000_000,
  mode: 0,
  read_instruction: "READ",
  spi_tuning: "fast",
  read_area: "0x1000+1024",
  write_area: null,
  summaries: [
    {
      direction: "read",
      bytes: 64,
      samples: 10,
      median_ns: 150000,
      p95_ns: 160000,
      min_ns: 145000,
    },
    {
      direction: "read",
      bytes: 1024,
      samples: 10,
      median_ns: 1950000,
      p95_ns: 2010000,
      min_ns: 1910000,
    },
  ],
  fit: {
    read: {
      samples: 20,
      overhead_ns: 30000,
      rate_bps: 4_250_000,
      max_relative_error: 0.01,
      sizes: [
        {
          bytes: 64,
          samples: 10,
          median_ns: 150000,
          p95_ns: 160000,
          fitted_ns: 150470,
          relative_error: -0.003,
        },
        {
          bytes: 1024,
          samples: 10,
          median_ns: 1950000,
          p95_ns: 2010000,
          fitted_ns: 1957500,
          relative_error: -0.004,
        },
      ],
    },
    write: null,
    pdi: { read_bps: 4_250_000, write_bps: 4_250_000, access_overhead_ns: 30_000 },
    assumptions: ["Simulated LAN9252."],
  },
  fit_error: null,
  csv: "# flowlab:provenance=simulated\ndirection,bytes,duration_ns\n",
};

afterEach(() => vi.restoreAllMocks());

describe("SpiBench", () => {
  it("runs simulated SPI benchmark, shows linear fit, and applies fit to Simulate", async () => {
    const run = vi.spyOn(api, "pdiBenchRun").mockResolvedValue(BENCH_RES);
    const apply = vi.fn();

    render(
      <TooltipProvider>
        <SpiBench
          capabilities={CAPS}
          slaveRunning={false}
          onStopSlave={vi.fn()}
          onApplyPdiFit={apply}
        />
      </TooltipProvider>,
    );

    fireEvent.click(screen.getByRole("button", { name: "Run benchmark" }));
    await waitFor(() => expect(run).toHaveBeenCalled());

    expect(screen.getAllByText("4.25 Mbit/s").length).toBeGreaterThan(0);
    fireEvent.click(screen.getByRole("button", { name: "Apply to Simulate" }));
    expect(apply).toHaveBeenCalledWith(BENCH_RES.fit!.pdi);
  });
});
