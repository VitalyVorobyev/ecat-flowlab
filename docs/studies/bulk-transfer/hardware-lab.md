# Hardware lab plan: Windows/Linux master + Toradex Verdin i.MX8M Plus slave

## Critical feasibility gate (must pass BEFORE real slave work)

**The on-module Ethernet MAC/PHY is ordinary Ethernet, not a verified EtherCAT Slave Controller (ESC).** A compliant EtherCAT slave normally has an ESC providing on-the-fly frame handling, SyncManagers and DPRAM. The Toradex alone should not be represented as a real EtherCAT slave via its RJ45. Procure/borrow a validated **external ESC evaluation module**, e.g. Microchip LAN9252, for the slave role. Its own EtherCAT IN/OUT ports connect to the PC master. The Toradex communicates with the ESC via PDI (SPI or supported alternative).

Likely topology:

```text
Linux/Windows PC Master ----[100BASE-TX EtherCAT]---- LAN9252 ESC eval board
                                                           |
                                                     SPI / IRQ / reset
                                                           |
                                                  Verdin i.MX8M Plus
                                                Linux slave application
```

Do not confuse the Toradex RJ45 ordinary Ethernet port with the external ESC's EtherCAT port. Toradex's normal Ethernet can still be used for SSH and **plain Ethernet baseline** simultaneously, on an isolated management network.

## Hardware validation checklist

- Confirm exact Toradex module SKU/revision and carrier/connector access to SPI_1 (or other PDI).
- Confirm I/O logic voltage: Toradex Verdin I/O is **1.8 V**; many ESC evaluation boards may use different logic levels. Use proper level shifting or a proven voltage-compatible kit; verify grounds, reset, IRQ, pin mux.
- Confirm controller supports SPI or suitable PDI and exposed timing/data-rate needs for 10 MB transfers; LAN9252 includes only **4 KiB ESC DPRAM** and this can be a limiting factor.
- Verify whether the selected SOES slave stack has a functional Linux LAN9252 hardware layer for the board/driver/kernel, the desired EoE/FoE features, and allowed licensing.
- Confirm ESC EEPROM/SII identity is programmed, ESI XML matches, and required SM/mailbox/PDO sizes actually fit memory.
- Capture SPI actual throughput and per-transaction latency on Toradex. Consider native kernel driver vs Linux userspace spidev; a slow PDI may dominate.
- Ensure ESC 100BASE-TX ports, PHY links, and direct cabling are correct; use the ESC port for EtherCAT traffic.
- Keep production control equipment disconnected and use lab-only NIC.

## Master choice

- **Linux PC** is the preferred first hardware master: explore EtherCrab Rust and SOEM alternatives; permissions, interface ownership and timestamping can be controlled. Consider real-time Linux only if timing requirements justify it.
- **Windows PC** second: use a master/library with confirmed Windows/Npcap support, and benchmark realistic user/kernel scheduling. Avoid assuming Windows standard networking is deterministic.
- **TwinCAT** optional interoperability baseline: do not require it, and do not run competing masters on same EtherCAT segment.
- MacBook is the *software-only development host*, not a required lab EtherCAT master.

## Target software

Torizon OS/Linux on Verdin iMX8M Plus: build an ARM64 application using an ESC driver abstraction and a compatible slave stack (SOES is a candidate, not a proven drop-in for current board/Torizon). Containerization may be convenient, but raw SPI access, timing and permissions need testing; start with simplest measurable application before introducing container overhead.

The Toradex device does not automatically provide a slave mode through a Linux network interface. A simulation can emulate this, but only a real ESC provides real slave interoperability for the planned setup.

## Progressive lab sequence

1. **L0 plain Ethernet:** PC ↔ Toradex direct TCP on ordinary Ethernet; transfer 10 MB and measure checksum/throughput/CPU. This establishes whether EtherCAT is appropriate at all.
2. **L1 ESC bring-up:** verify PDI access (register read/write), EEPROM, identity, state transitions and simple small PDOs; no bulk transfer.
3. **L2 basic transfer:** master ↔ slave 1 KB then 1 MB then 10 MB, with integrity, timeout and diagnostics.
4. **L3 protocols:** test feasible FoE, EoE, VoE, CoE and custom PDO adapters one at a time, verifying both master and slave support.
5. **L4 model calibration:** capture frames, service frequency, PDI throughput; fit simulator and report deviations.
6. **L5 optimization:** run constrained parameter sweeps, validate best candidates with multiple trials and control PDO background load.
7. **L6 Windows/TwinCAT:** repeat on other master platforms only when hardware baseline works.

## Stop/go decisions

- If 10 MB target deadline is below the physical limit (~0.8 s for one 100 Mbit/s link, ignoring all overhead), EtherCAT physically cannot satisfy it on standard 100 Mbit/s transport; use faster alternate network/architecture.
- If 10 MB is frequent and sustained, test whether direct Gigabit Ethernet or a split control/data network is a better system design.
- If PDI or mailbox capacity proves limiting, consider a different ESC/parallel interface or PDO arrangement rather than endless EoE tuning.
- If Toradex hardware integration is impractical, use an independent prebuilt EtherCAT slave evaluation kit first to validate the PC master and simulation models.

## Suggested external references

See `references.md`: Toradex SPI and voltage documentation, ETG slave architecture guide, Microchip LAN9252 hardware overview, SOES Linux LAN9252 HAL source.
