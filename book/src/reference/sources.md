# Sources

Design and research sources, checked 2026-11-07. A link documents a concept or a product; it is
not evidence that a particular version fits a particular system.

## EtherCAT specifications and devices

1. ETG.1000 (IEC 61158): EtherCAT data link and application layer: frame and datagram layout,
   commands, mailbox, CoE, FoE, EoE, SoE. FlowLab's encodings are cross-checked against
   Wireshark's dissector rather than quoted from the specification.
2. ETG.2000 (ESI) and ETG.2100 (ENI): XML formats read by `flowlab-devices` (subset).
3. ETG.2200 Slave Implementation Guide: ESC, DPRAM, state machine, SyncManagers, mailbox —
   <https://www.ethercat.org/download/documents/ETG2200_V3i1i0_G_R_SlaveImplementationGuide.pdf>
4. CiA 301: SDO command specifiers and abort codes.
5. Microchip LAN9252: EtherCAT slave controller with 4 KiB process RAM and SPI or host-bus
   (HBI) PDI — <https://www.microchip.com/en-us/product/lan9252>
6. Toradex Verdin iMX8M Plus — <https://developer.toradex.com/hardware/verdin-som-family/modules/verdin-imx8m-plus/>;
   SPI on Torizon and the module's 1.8 V I/O —
   <https://developer.toradex.com/torizon/application-development/use-cases/peripheral-access/how-to-use-spi-on-torizon/>

## Open-source stacks (not used by FlowLab; candidates for a lab setup)

7. EtherCrab, Rust master (MIT/Apache-2.0) — <https://github.com/ethercrab-rs/ethercrab>
8. SOEM, C master (GPLv3 or commercial) — <https://github.com/OpenEtherCATsociety/SOEM>
9. SOES, C slave stack, with a Linux LAN9252 hardware layer — <https://github.com/OpenEtherCATsociety/SOES>
10. IgH EtherCAT Master for Linux — <https://etherlab.org/>

## Context on mailbox throughput

11. Beckhoff EL6601/EL6614 documentation: EoE throughput depends on cycle time, idle bus time,
    mailbox size and contention (device-specific examples, not general figures) —
    <https://infosys.beckhoff.com/content/1033/el6601_el6614/2349505163.html>

## Formats and tools

12. pcapng (IETF draft) — <https://datatracker.ietf.org/doc/draft-ietf-opsawg-pcapng/>
13. Link types (`LINKTYPE_ETHERNET` = 1) — <https://www.tcpdump.org/linktypes.html>
14. Wireshark EtherCAT dissectors (`ecat`, `ecat_mailbox`) — <https://gitlab.com/wireshark/wireshark/-/tree/master/plugins/epan/ethercat>
15. EtherType 0x88A4 and UDP port 34980 (EtherCAT) — <https://www.iana.org/assignments/service-names-port-numbers/>
16. RFC 7042: documentation MAC addresses 00-00-5E-00-53-xx, used in generated captures — <https://www.rfc-editor.org/rfc/rfc7042>

## Dependencies

Rust: serde, serde_json, thiserror, clap, tracing, schemars, sha2, roxmltree, axum, tokio,
tower-http, ts-rs, proptest; all MIT and/or Apache-2.0. UI: the `@vitavision/*` lab-ui packages
(MIT OR Apache-2.0), React, Tailwind CSS, lucide-react (ISC). CI checks licences with
`cargo deny`.
