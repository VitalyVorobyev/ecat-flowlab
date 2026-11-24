import { TooltipProvider } from "@vitavision/ui";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { api, type Capabilities, type PhyReport } from "../api";
import { PhyInspector } from "./PhyInspector";

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

const REPORT: PhyReport = {
  provenance: "simulated",
  target: "simulated LAN9252 @ 10.00 MHz",
  link: { chip_id: 0x9252, revision: 1, al_state: "INIT", id_rev: 0x92520001, hw_cfg: 0x08000000 },
  ports: [
    {
      phy: 0,
      present: true,
      link_up: true,
      autoneg_enabled: true,
      autoneg_done: true,
      speed_100: true,
      full_duplex: true,
      mdix_override: false,
      auto_mdix: true,
      mdix: false,
      mode: 7,
      summary: "link up, 100 Mbit/s full duplex, auto-negotiation done, auto-MDIX, mode 111",
      registers: [
        { reg: 0, name: "Basic Control", value: 0x3100 },
        { reg: 1, name: "Basic Status", value: 0x782d },
      ],
    },
    {
      phy: 2,
      present: false,
      link_up: false,
      autoneg_enabled: false,
      autoneg_done: false,
      speed_100: false,
      full_duplex: false,
      mdix_override: false,
      auto_mdix: false,
      mdix: false,
      mode: 0,
      summary: "none",
      registers: [],
    },
  ],
};

afterEach(() => vi.restoreAllMocks());

describe("PhyInspector", () => {
  it("reads PHY status and enables auto-negotiation in sim mode", async () => {
    const read = vi.spyOn(api, "phyRead").mockResolvedValue(REPORT);
    const autoneg = vi.spyOn(api, "phyAutoneg").mockResolvedValue(REPORT);

    render(
      <TooltipProvider>
        <PhyInspector capabilities={CAPS} slaveRunning={false} onStopSlave={vi.fn()} />
      </TooltipProvider>,
    );

    fireEvent.click(screen.getByRole("button", { name: "Read PHYs" }));
    await waitFor(() => expect(read).toHaveBeenCalled());

    expect(screen.getByText("Link up")).toBeInTheDocument();
    expect(screen.getByText("100 Mbit/s full duplex")).toBeInTheDocument();
    expect(screen.getByText("0x3100")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Enable Auto-Neg & Auto-MDIX" }));
    await waitFor(() =>
      expect(autoneg).toHaveBeenCalledWith(expect.objectContaining({ target_phys: [0] })),
    );
  });

  it("blocks PHY reads while PDI slave holds the SPI bus", () => {
    render(
      <TooltipProvider>
        <PhyInspector capabilities={CAPS} slaveRunning={true} onStopSlave={vi.fn()} />
      </TooltipProvider>,
    );

    expect(screen.getByText("PDI slave is currently using the SPI bus")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Read PHYs" })).toBeDisabled();
  });
});
