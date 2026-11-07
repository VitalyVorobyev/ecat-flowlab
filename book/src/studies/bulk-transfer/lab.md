# Lab plan

## Feasibility gate

The Verdin's Ethernet MAC/PHY is ordinary Ethernet. An EtherCAT slave needs an **ESC** for
on-the-fly frame processing, SyncManagers and DPRAM. Use an external ESC evaluation board, such
as a LAN9252 board, connected to the Verdin over SPI (or another supported PDI). Its EtherCAT
ports connect to the master. Before any transfer work:

- Confirm the Verdin module and carrier revision, and access to the SPI pins, IRQ and reset.
- **Voltage:** Verdin I/O is 1.8 V. Many ESC boards use other levels. Use level shifting or a
  board known to be compatible. Check grounds and the pin mux.
- Choose the slave stack. SOES has a Linux LAN9252 hardware layer, but check its EoE and FoE
  support on the chosen revision, and its licence.
- Program the ESC's EEPROM/SII identity, and make the ESI match it. Check that the SyncManager,
  mailbox and PDO sizes fit the 4 KiB of ESC RAM (`flowlab plan` checks this).
- Measure the SPI's effective read and write rates and per-access latency on the Verdin, both
  with a kernel driver and with user-space spidev (see [Measuring the SPI](#measuring-the-spi)).
  The [report](report.md) shows why this comes first.
- Keep the lab network isolated. Never connect production equipment. Never run two masters on
  one segment.

## Master

1. **Linux PC** first: EtherCrab (Rust) or SOEM/IgH (check licences). Interface ownership,
   permissions and timestamping are controllable.
2. **Windows PC** second, with a master that supports Npcap. Do not assume Windows networking is
   deterministic.
3. **TwinCAT** as an optional interoperability baseline.

The development Mac only runs FlowLab; it is not a lab master.

## Sequence

| Step | Goal | FlowLab's part |
|---|---|---|
| L0 | Plain Ethernet: PC ↔ Verdin TCP, 10 MB with checksum, throughput and CPU | the baseline every result is compared with |
| L1 | ESC bring-up: PDI register access, EEPROM, identity, state machine, small PDOs | capture → Stations (AL states), Timing (cycle, WKC) |
| L2 | Basic transfer: 1 KB, 1 MB, 10 MB with integrity, timeouts and diagnostics | Mailbox view: messages, rejected writes, empty reads |
| L3 | One protocol at a time: FoE, EoE, CoE, VoE, PDO channel, each verified on both ends | per-protocol transfer tables, Compare |
| L4 | Calibration: measured PDI timing and master policy as lab inputs; check against the captures and report residuals | `flowlab pdi-fit`, `flowlab policy`, `flowlab study --capture` |
| L5 | Optimisation: constrained sweeps; check the best candidates over several trials with control traffic present | `scripts/bulk-transfer-report.py --lab lab.json --out DIR` |
| L6 | Other masters, only once L0–L5 work | Compare per master |

## Measuring the SPI

The simulator charges every PDI access a fixed cost plus its bytes at an effective rate. Measure
those on the target, with the driver and stack that will ship:

1. Wrap the slave stack's ESC read and write functions (the hardware layer every access goes
   through) with a monotonic clock, and log one line per call: direction, bytes, duration in ns.
   Keep the logging out of the timed section, for example in a preallocated ring buffer written
   out at the end.
2. Run a representative workload: mailbox traffic with several message sizes (an FoE transfer,
   some SDO uploads) and process data. Each direction needs at least two different access sizes.
3. Save it as [PDI timing samples](../../reference/specs.md#pdi-timing-samples) and fit:

   ```bash
   flowlab pdi-fit spi.csv --result spi-fit.json --lab lab.json
   ```

   The output lists, per access size, the median, the 95th percentile and the fitted line. If
   some sizes are far off the line (long accesses split into chunks, a DMA threshold), the line
   is the wrong model for those sizes. Time the sizes the application actually uses, and note it
   in `lab.json`'s `description`.
4. Repeat with each driver variant (kernel driver, spidev, SPI clock) as its own file.

Whether the application reads the whole mailbox buffer or only the message is a property of the
stack. Set `pdi.mailbox_access` in `lab.json` to `"whole_buffer"` or `"message_only"`
accordingly. Also measure the time from the mailbox interrupt to the reply being written, and set
it as `app_processing_ns`.

## Capturing the master's mailbox service

Capture at the master's port, or with a tap, while it moves a file to the slave with FoE. Then:

```bash
flowlab policy master-foe.pcapng --lab lab.json        # --mailbox / --eni if SM setup is not in the capture
python3 scripts/bulk-transfer-report.py --lab lab.json --out results/report-lab
```

`flowlab policy` needs known directions (the capture shows sent and returning frames), a cyclic
stream, and mailbox accesses in at least 10 cycles. It prints accesses and frames per cycle, the
turnaround and the last send in the cycle, and the policy it stores. The re-run report labels
every table with the lab inputs it used.

## Stop/go decisions

- **Deadline below the physical bound** (0.8 s for 10 MB at 100 Mbit/s): EtherCAT cannot meet
  it on one 100 Mbit/s segment. Use a separate, faster link.
- **Frequent or sustained 10 MB transfers:** test a dedicated Gigabit link or a split
  control/data network before tuning EtherCAT.
- **PDI or mailbox capacity limiting after L4:** consider a faster PDI, another ESC, or a
  process-data channel, rather than tuning EoE further.
- **Verdin integration impractical:** validate master and model with a ready-made EtherCAT
  slave evaluation kit first.
- **Advantages vanish after calibration:** report it and stop.

## Risks

| Risk | Consequence | Mitigation |
|---|---|---|
| No suitable ESC or slave stack | no real slave | select and validate ESC and stack first |
| SPI/PDI too slow | transfer far below link rate | measure early (L1); consider HBI or another ESC |
| ESC RAM too small | mailbox/PDO configuration impossible | plan against 4 KiB; verify ESI and SII |
| Master lacks EoE/FoE/VoE APIs | no transport adapter | verify versions and APIs before choosing |
| Licence conflicts (e.g. GPL master) | cannot ship | review before embedding |
| Mailbox scheduling modelled wrongly | misleading optimisation | capture each master's polling; calibrate |
| Bulk traffic disturbs control | safety, determinism | isolated lab, cycle constraints in every study |
| Payload size confused with cadence | wrong technology choice | answer the product questions first |
