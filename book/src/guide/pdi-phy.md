# PHY registers

`flowlab pdi-phy` shows the link of a LAN9252's two EtherCAT ports as their PHYs see it: link,
speed, duplex, auto-negotiation and MDI/MDI-X. It reads the PHY registers over the slave's SPI,
through the ESC's MII management interface, so it runs on the slave's host and needs no
network access.

Firmware and slave stacks sometimes force the PHYs, for example to 100 Mbit/s full duplex
without auto-negotiation and with a fixed MDI or MDI-X. A master's port that expects
auto-negotiation then links slowly, at half duplex, or not at all, depending on the adapter and
the cable. `pdi-phy` shows whether that is the case.

```bash
# On the slave's host, with the slave's own application stopped (it shares the SPI).
flowlab pdi-phy --device /dev/spidev1.0
```

```text
LAN9252 rev 1 on /dev/spidev1.0 at 10000000 Hz: ESC in INIT
PHY 0: link up, 100 Mbit/s full duplex, auto-negotiation off, MDI-X (register 27), mode 011
       r0=0x2100 r1=0x780D r2=0x0007 r3=0xC0F1 r4=0x01E1 r5=0x0000 r18=0x0060 r27=0xA00B r31=0x0058
PHY 1: link down, 100 Mbit/s full duplex, auto-negotiation off, MDI (register 27), mode 011
       …
PHY 2: none
PHY 3: none
```

The listing above is an example of the format, not a measurement. `--sim` runs against a
simulated LAN9252 whose two PHYs have a link with auto-negotiation done.

## Changing a PHY

```bash
flowlab pdi-phy --device /dev/spidev1.0 --allow-phy-writes --autoneg 0 --autoneg 1
flowlab pdi-phy --device /dev/spidev1.0 --allow-phy-writes --set 0:0=0x2100
```

| Option | Effect |
|---|---|
| `--autoneg PHY` | register 27 bits 15 and 14 (auto-MDIX, set by the register), register 18 mode 0b111 (all capable) with its PHY address kept, basic control 0x3300 (100 Mbit/s, full duplex, auto-negotiation on and restarted); the registers are shown again after 3 s |
| `--set PHY:REG=VALUE` | one register write, numbers decimal or `0x` hex |

Both need `--allow-phy-writes`. The change lasts until the next reset, or until the slave's
application sets the PHYs again when it starts.

## How it reaches the PHYs

The command checks the SPI link first (`BYTE_TEST`, `HW_CFG`, `ID_REV`), as
[`pdi-bench`](../studies/bulk-transfer/lab.md#measuring-the-spi) does, and that MII Management
Control (0x0510) lets the PDI take the interface. For each PHY access it:

1. refuses if the master claims the MII interface (0x0516 bit 0);
2. takes the interface for the PDI (0x0517 = 1);
3. writes the PHY address (0x0512) and register (0x0513), for a write also the data (0x0514);
4. writes the command (0x0510: 0x0100 read, 0x0200 write) and polls until busy (bit 15)
   clears;
5. fails on a read or command error (bits 13 and 14);
6. reads the data (0x0514) and gives the interface back (0x0517 = 0).

The MII management registers are the only ESC registers it writes. The registers shown are
basic control and status (0, 1), the PHY ID (2, 3), the auto-negotiation advertisement and
link partner (4, 5), special modes (18), special control/status indication (27) and PHY special
control/status (31), as the LAN9252 data sheet defines them.
