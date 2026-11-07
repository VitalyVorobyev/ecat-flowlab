# Reference material and verification status

Checked as of **2026-11-07**. Links are design/research sources, not proof that a particular project version will fit the final hardware. Recheck exact versions/features/licenses before implementation.

## EtherCAT / ESC / hardware

1. EtherCAT Technology Group, Slave Implementation Guide (ETG.2200) — ESC, DPRAM, state machine, SyncManagers, process data, and mailbox concepts: https://www.ethercat.org/download/documents/ETG2200_V3i1i0_G_R_SlaveImplementationGuide.pdf
2. EtherCAT Technology Group LAN9252 product — 100BASE-TX slave controller, 4 KiB DPRAM, SPI/HBI interfaces: https://www.ethercat.org/en/products/1956774FF0C74D71961404C59398D1B5.htm
3. Microchip LAN9252 hardware — https://www.microchip.com/en-us/product/lan9252
4. Toradex Verdin iMX8M Plus hardware revisions/pinouts — https://developer.toradex.com/hardware/verdin-som-family/modules/verdin-imx8m-plus/
5. Toradex Torizon SPI configuration and Verdin voltage warning — https://developer.toradex.com/torizon/application-development/use-cases/peripheral-access/how-to-use-spi-on-torizon/

## Open-source stacks

6. EtherCrab Rust master; **verify actual mailbox protocol feature coverage**: https://github.com/ethercrab-rs/ethercrab
7. SOEM C master; versions 2.x have GPLv3/commercial dual license — https://github.com/OpenEtherCATsociety/SOEM
8. SOES C slave; verify desired EoE/FoE pathways and examples (README lists EoE with sample app TODO) — https://github.com/OpenEtherCATsociety/SOES
9. SOES Linux LAN9252 hardware access implementation — https://github.com/OpenEtherCATsociety/SOES/blob/master/soes/hal/linux-lan9252/esc_hw.c
10. IgH EtherCAT master as alternate Linux baseline — https://etherlab.org/

## Performance context

11. Beckhoff EL6601/EL6614 documentation: EoE goodput depends on cycle time, idle bus time, mailbox size, and mailbox contention; historical examples only, **not generic guaranteed EoE throughput** — https://infosys.beckhoff.com/content/1033/el6601_el6614/2349505163.html
12. Beckhoff EoE vs cyclic process data modes — https://infosys.beckhoff.com/content/1033/el6601_el6614/2349504139.html

## Formats and tooling used by FlowLab

13. pcapng specification (IETF draft, block and option layout incl. `if_tsresol`, `if_tsoffset`): https://datatracker.ietf.org/doc/draft-ietf-opsawg-pcapng/
14. libpcap file format and link types (`LINKTYPE_ETHERNET` = 1): https://www.tcpdump.org/linktypes.html
15. Wireshark EtherCAT dissectors (`ecat`, `ecat_mailbox`, `esl`) — cross-check for decoding: https://gitlab.com/wireshark/wireshark/-/tree/master/plugins/epan/ethercat
16. IEEE EtherType registry: 0x88A4 EtherCAT; IANA UDP port 34980 (0x88A4) "EtherCAT": https://www.iana.org/assignments/service-names-port-numbers/
17. RFC 7042 — documentation MAC range 00-00-5E-00-53-xx used by `flowlab-synth`: https://www.rfc-editor.org/rfc/rfc7042
18. Rust crates adopted (all MIT/Apache-2.0 dual or MIT): serde, serde_json, thiserror, clap, tracing, schemars 1.x, sha2, axum 0.8, tokio, tower-http, ts-rs 12, proptest. UI: `@vitavision/*` (MIT OR Apache-2.0), React 19, Tailwind 4, lucide-react (ISC).

## Claims requiring future validation

- That a given EtherCrab release supports arbitrary EoE/FoE/VoE transfers.
- That SOES EoE Linux target and chosen ESC kit are usable without custom PDI work.
- That Toradex SPI can sustain any specific EoE goodput.
- That one mailbox transaction necessarily occurs per EtherCAT cycle.
- That CoE/FoE/VoE work with TwinCAT or arbitrary third-party slaves out of the box.

These are engineering hypotheses, not documented guaranteed capabilities.
