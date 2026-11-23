# Lab toolbox

The toolbox is the part of FlowLab that talks to hardware. It puts both ends of an EtherCAT
link under your control: a master that measures, slave applications that answer, and the
instruments in between. Each tool is opt-in and states what it touches
([Safety](../getting-started.md#safety)).

| Tool | Runs on | What it does |
|---|---|---|
| [`flowlab master`](master.md) | the master's host (Linux, macOS, Windows) | scan a lab segment, run cycles, move FoE, CoE, EoE and FlowLab VoE transfers, and reach a slave's TCP services through an [EoE gateway](master.md#gateway); results labelled measured |
| [`flowlab slave`](slave.md) | any host with an Ethernet port | an emulated ESC with FoE, CoE, EoE and VoE behind its mailbox, for a rig without EtherCAT hardware |
| [`flowlab pdi-slave`](pdi-slave.md) | the slave's host (Linux, LAN9252 on spidev) | FlowLab's slave application on a real ESC, in place of the product's, with its EoE bridged to a network interface |
| [`flowlab pdi-bench`](../studies/bulk-transfer/lab.md#measuring-the-spi) | the slave's host | the SPI's cost per ESC access, as samples for the simulator's PDI model |
| [`flowlab pdi-phy`](pdi-phy.md) | the slave's host | the LAN9252's PHY registers: link, speed, duplex, auto-negotiation, MDI-X |
| [`flowlab tcp-bench`](tcp-bench.md) | both ends of a TCP path | TCP throughput with SHA-256 checks, for TCP over EoE |
| `flowlab capture` | the master's host or a tap | records and analyses what crosses the wire ([Live capture](live.md)) |

Typical rigs:

- **No EtherCAT hardware.** Two computers on a cable (or one with two ports): `flowlab master`
  on one, `flowlab slave` on the other ([Emulated slave](slave.md)).
- **A real slave as it ships.** `flowlab master` against the product's own slave
  application: what the product delivers.
- **A real ESC, FlowLab's application.** `flowlab pdi-slave` on the slave's host in place of
  the product's application: what the ESC, its SPI and the master can carry
  ([On a Verdin board](verdin.md)).

The [transfer protocols](../reference/transfer-protocols.md) page compares what the protocols
carry and what limits them; [LAN9252 on a Verdin iMX8MP](../studies/bulk-transfer/lab-results.md)
has measured figures.
