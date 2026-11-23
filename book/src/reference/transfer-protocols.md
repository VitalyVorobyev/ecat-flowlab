# Transfer protocols

Bulk data reaches an EtherCAT slave through its mailbox: two SyncManager buffers in the ESC,
"mailbox out" (master to slave) and "mailbox in" (slave to master). Every protocol below moves
one buffer's worth at a time, and the slave's application copies each buffer through its PDI.
They differ in what they carry, how often a side has to answer, and what a host can do with
the data. FlowLab implements all four in the lab master, the [emulated slave](../guide/slave.md)
and the [PDI slave](../guide/pdi-slave.md), and decodes them in captures
([Mailbox traffic](../guide/mailbox.md)).

| | FoE | CoE SDO | EoE | FlowLab VoE |
|---|---|---|---|---|
| Carries | a named file | the value of one object dictionary entry | Ethernet frames: any IP traffic | a byte stream of a given length |
| Payload per message, 1024-byte mailbox | 1012 B | 1015 B per segment | 992 B of the tunnelled frame (32-byte steps), 1514-byte frames in 2 messages | 1010 B |
| Answers | one ACK per DATA | one response per segment | none at the EoE level; TCP's own acknowledgements are frames too | one ACK per window of DATA |
| End-to-end check | none | none | TCP checksums; the application's own | the slave confirms length and SHA-256 |
| Typical use | firmware and files | parameters, small blocks | IP services on the slave (HTTP, SSH, gRPC) | lab measurement of the mailbox itself |
| FlowLab command | `master transfer --protocol foe` | `master transfer --protocol coe --sdo IDX:SUB` | `master transfer --protocol eoe` (UDP), [`master gateway`](../guide/master.md#gateway) (TCP) | `master transfer --protocol voe` |

The [Lab master](../guide/master.md#protocols) page lists each protocol's options; the [FlowLab
VoE](voe.md) page specifies VoE.

## What limits a transfer

1. **The slave's time per message.** The slave's application reads each "mailbox out" message
   and writes each "mailbox in" message through its PDI. On a LAN9252 on SPI under Linux,
   that costs milliseconds per kilobyte, and it depends more on the number of SPI transfers
   than on the clock ([LAN9252 on a Verdin iMX8MP](../studies/bulk-transfer/lab-results.md)).
2. **Answers.** A protocol that waits for an answer per message (FoE, CoE) pays the slave's
   write of the answer and the master's read of it for every message. VoE answers once per
   window. EoE has no answers of its own, but TCP inside it acknowledges every one or two
   frames.
3. **The mailbox size.** A larger mailbox means fewer messages per byte. It helps only when the
   slave's time per message grows less than the payload does. A slave that copies the whole
   buffer for every message, short acknowledgements included, gets slower with a larger
   mailbox. `--mailbox-bytes` on the lab master sets both SyncManagers to another size for one
   run without touching the slave's EEPROM.
4. **The master's round trip.** The lab master has one mailbox frame in flight at a time; on
   a host with a USB adapter, each frame's round trip is hundreds of microseconds.
5. **The line.** 100 Mbit/s full duplex is far above all of the above for mailbox traffic.

## Measuring them

| Question | Tool |
|---|---|
| What does the SPI cost per access? | [`flowlab pdi-bench`](../studies/bulk-transfer/lab.md#measuring-the-spi) on the slave's host |
| What do ESC, SPI and master carry, whatever the product's stack does? | [`flowlab pdi-slave`](../guide/pdi-slave.md) on the slave's host, `flowlab master transfer` on the master's |
| How fast is TCP to a service on the slave? | [TCP over EoE](../guide/tcp-bench.md): `master gateway` and `tcp-bench` |
| How does the master serve the mailbox in a capture? | `flowlab analyze`, `flowlab policy` ([Mailbox traffic](../guide/mailbox.md)) |
| What would another mailbox size, policy or cycle time give? | `flowlab simulate`, `flowlab what-if` ([Simulation](../guide/simulation.md)) |
