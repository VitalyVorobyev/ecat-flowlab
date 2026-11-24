import { TooltipProvider } from "@vitavision/ui";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { api, type Capabilities, type SlaveStatus } from "../api";
import { SlaveControl } from "./SlaveControl";

const CAPS: Capabilities = {
  version: "0.1.0",
  platform: { os: "linux", arch: "aarch64", hostname: "verdin" },
  default_view: "slave",
  local_paths: false,
  live_interfaces: false,
  slave: {
    pdi_enabled: true,
    eoe_bridge_supported: true,
    default_device: "/dev/spidev1.0",
    sim_pdi: true,
    allow_replace_app: false,
    allow_writes: false,
    allow_phy_writes: false,
    spidev_devices: ["/dev/spidev1.0"],
    net_interfaces: ["eth0", "fleoe0", "fleoe0b"],
  },
};

const IDLE: SlaveStatus = {
  state: "idle",
  provenance: null,
  target: null,
  spi_tuning: null,
  clock_hz: null,
  max_mailbox_bytes: null,
  message_only: null,
  elapsed_s: 0,
  link: null,
  al_state: null,
  al_error: null,
  mailboxes: null,
  stats: {
    polls: 0,
    al_requests: 0,
    messages_in: 0,
    messages_out: 0,
    bytes_in: 0,
    bytes_out: 0,
    status_reads: 0,
  },
  bridge: null,
  series: [],
  al_events: [],
  received: [],
  error: null,
};

afterEach(() => vi.restoreAllMocks());

describe("SlaveControl", () => {
  it("explains hardware opt-in when allow_replace_app is false and starts in sim mode", async () => {
    const user = userEvent.setup();
    const start = vi.spyOn(api, "slaveStart").mockResolvedValue({
      ...IDLE,
      state: "running",
      provenance: "simulated",
      target: "simulated LAN9252 @ 20.00 MHz",
      al_state: "INIT",
    });
    const onStatus = vi.fn();
    render(
      <TooltipProvider>
        <SlaveControl capabilities={CAPS} status={IDLE} onStatusChange={onStatus} />
      </TooltipProvider>,
    );

    expect(screen.getByText("--replace-slave-application")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Start PDI slave" })).toBeDisabled();

    await user.click(screen.getByRole("radio", { name: "Simulated LAN9252" }));
    const btn = screen.getByRole("button", { name: "Start PDI slave" });
    expect(btn).toBeEnabled();
    await user.click(btn);

    await waitFor(() => expect(start).toHaveBeenCalled());
    expect(start.mock.calls[0]?.[0].sim).toBe(true);
    expect(onStatus).toHaveBeenCalled();
  });

  it("renders running telemetry and stops cleanly", async () => {
    const user = userEvent.setup();
    const stop = vi.spyOn(api, "slaveStop").mockResolvedValue({ ...IDLE, state: "stopped" });
    const onStatus = vi.fn();
    render(
      <TooltipProvider>
        <SlaveControl
          capabilities={CAPS}
          status={{
            ...IDLE,
            state: "running",
            provenance: "measured",
            al_state: "PREOP",
            stats: { ...IDLE.stats, polls: 12000, messages_in: 42, bytes_in: 40000 },
            series: [
              {
                t_s: 1,
                polls_s: 12000,
                msg_in_s: 42,
                msg_out_s: 10,
                bytes_in_s: 40000,
                bytes_out_s: 2000,
                eoe_to_host_s: 0,
                eoe_to_master_s: 0,
              },
            ],
          }}
          onStatusChange={onStatus}
        />
      </TooltipProvider>,
    );

    expect(screen.getByText("Measured")).toBeInTheDocument();
    expect(screen.getAllByText("12,000").length).toBeGreaterThan(0);
    await user.click(screen.getByRole("button", { name: "Stop PDI slave" }));
    await waitFor(() => expect(stop).toHaveBeenCalled());
    expect(onStatus).toHaveBeenCalled();
  });
});
