# The workbench

`flowlab serve` starts the web UI and REST/streaming API:

```bash
flowlab serve [CAPTURES…] [--eni network.xml] [--mailbox STATION=OUT,IN] \
    [--host 127.0.0.1] [--port 7878] [--sim-pdi] \
    [--replace-slave-application] [--allow-writes] [--allow-phy-writes] [--open]
```

By default it binds to <http://127.0.0.1:7878> for local capture analysis and simulation. On a
slave device (see [On a Verdin board](verdin.md#web-workbench-on-the-verdin)), pass
`--host 0.0.0.0` (and opt-in hardware flags as needed) to use the embedded UI over management LAN
or an EtherCAT EoE tunnel (`flowlab master gateway`).

## Layout

The header holds the capture selector, **Open capture…**, a live slave status pill, and a theme
toggle (light/dark, persisted in `localStorage`). The left navigation rail opens the seven
workspaces:

| View | Shows |
|---|---|
| **Slave** | Run and monitor FlowLab's PDI slave (`pdi-slave`) on Linux spidev or simulated LAN9252: AL state, SyncManagers, 1 Hz polls/messages/throughput charts, EoE bridge stats, and received payloads |
| **Throughput** | Browser-to-server streaming HTTP/TCP throughput & RTT latency lab (`tcp-bench`-compatible `/api/bench/*` and `/upload`), with pure-TypeScript SHA-256 verification over plain HTTP and CSV/JSON export |
| **PHY** | Read LAN9252 PHY 0–3 status cards, decoded registers, Auto-Neg & Auto-MDIX controls, and raw MII register write (`pdi-phy`) |
| **SPI Bench** | Run timed SPI read/write benchmarks and linear regression (`pdi-bench` + `pdi-fit`), inspect residuals, export CSV/JSON, and click **Apply to Simulate** |
| **Analysis** | Tabbed capture workspace combining **Captures**, **Live** (with `--live` or replay), **Overview**, **Timing**, **Frames**, **Stations**, **Mailbox**, **Compare**, and **Sim run** |
| **Simulate** | Set up a segment and its transfers, inspect timing, PDI, and working-counter breakdown after every edit, and see what single change (or pair) would shorten it most ([Simulation](simulation.md#the-simulate-view)) |
| **Planner** | Frame layout and bus budget of a network; ESI import |

When launched without initial captures, hardware builds (Linux or `--sim-pdi`) open on
**Slave**; otherwise the UI opens on **Simulate** (or **Analysis** when started with captures).
The active route is kept in the URL hash (`#slave`, `#throughput`, `#phy`, `#spi-bench`,
`#analysis`, `#simulate`, `#planner`, with legacy hashes such as `#frames` or `#overview` mapped
into **Analysis**).

## Reading the views

- **Colours** mark judgements only. *Defect* and *warning* colours appear where a value was
  compared with a declared threshold, such as a working counter that differs from the expected
  value or jitter above 10 % of the period. Magnitudes such as load or duration are neutral.
- **Assumptions** are listed in the Overview, Mailbox, and Sim run tabs under **Analysis**.
  They are the same list that the JSON report carries.
- **Provenance badges** (*Measured*, *Simulated*, *Synthetic*) sit next to every readout and
  capture name.

## Hardware safety gates

Real ESC mutations from the browser are disabled by default and require explicit CLI flags when
starting `flowlab serve`:

- `--replace-slave-application` to start the PDI slave against a real `/dev/spidev*` device.
- `--allow-writes` to permit SPI write benchmarks into LAN9252 process RAM (`--area START+LEN`).
- `--allow-phy-writes` to permit PHY Auto-Neg/Auto-MDIX or raw MII register writes.
- `--sim-pdi` to allow running the **Slave**, **PHY**, and **SPI Bench** screens against the
  built-in simulated LAN9252 (`SimLan9252`) without hardware.

Starting the PDI slave holds an exclusive SPI lock on the server: **PHY** and **SPI Bench**
requests return HTTP `409 Conflict` until the PDI slave is stopped (and pressing `Ctrl-C` or
sending `SIGTERM` stops the PDI slave cleanly, resetting AL Status to `INIT`).

## Security

The server has no authentication and binds to loopback (`127.0.0.1`) by default:

- **Local paths**: `POST /api/captures/open` (`Open path` in **Captures**) is only enabled when
  bound to loopback, unless `--allow-local-paths` is passed.
- **Host allow-list**: requests are accepted when the `Host` header is an IP address (for EoE or
  LAN IPs), `localhost`, an mDNS `.local` name when bound off loopback (such as `verdin-imx8mp.local`),
  or a host explicitly listed via `--allow-host <NAME>`. Other `Host` values return *403*.
