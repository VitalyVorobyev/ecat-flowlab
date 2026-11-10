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
- Measure the SPI's effective read and write rates and per-access latency on the Verdin
  (`flowlab pdi-bench`, see [Measuring the SPI](#measuring-the-spi)).
  The [report](report.md) shows why this comes first.
- Keep the lab network isolated. Never connect production equipment. Never run two masters on
  one segment.

## Master

1. **Linux PC** first: FlowLab's own master, `flowlab master` ([Lab master](../../guide/master.md)),
   built on EtherCrab; or SOEM/IgH (check licences). Interface ownership, permissions and
   timestamping are controllable.
2. **Windows PC** second, with a master that supports Npcap (`flowlab master` builds there with
   the Npcap SDK). Do not assume Windows networking is deterministic.
3. **TwinCAT** as an optional interoperability baseline.

FlowLab's master runs a transfer set up like a simulated one: the same payload bytes, cycle
time and mailbox policy (`flowlab master transfer --like SPEC --transfer K`), so its result
compares field by field with the simulated one. It is a measuring instrument, not a product
master: it serves one slave and one transfer at a time, and its timing is the host's. Run
`flowlab capture` on the same port meanwhile; `flowlab policy` on that capture reads the policy
back from the wire.

The development Mac only runs FlowLab; it is not a lab master.

## Sequence

| Step | Goal | FlowLab's part |
|---|---|---|
| L0 | Plain Ethernet: PC ↔ Verdin TCP, 10 MB with checksum, throughput and CPU | the baseline every result is compared with |
| L1 | ESC bring-up: PDI register access and timing, EEPROM, identity, state machine, small PDOs | `flowlab pdi-bench` (link check, SPI timing, [below](#measuring-the-spi)); `flowlab master scan` (identity, SyncManagers, ENI), `flowlab master run`; capture → Stations (AL states), Timing (cycle, WKC) |
| L2 | Basic transfer: 1 KB, 1 MB, 10 MB with integrity, timeouts and diagnostics | `flowlab master transfer`; Mailbox view: messages, rejected writes, empty reads |
| L3 | One protocol at a time: FoE, EoE, CoE, VoE, PDO channel, each verified on both ends | `flowlab master transfer` for FoE, CoE, EoE and [FlowLab VoE](../../reference/voe.md) (it has no PDO channel, and a slave needs FlowLab VoE in its stack); per-protocol transfer tables, Compare |
| L4 | Calibration: measured PDI timing (from L1) and master policy as lab inputs; check against the captures and report residuals | `flowlab pdi-fit`, `flowlab policy`, `flowlab study --capture` |
| L5 | Optimisation: constrained sweeps; check the best candidates over several trials with control traffic present | `scripts/bulk-transfer-report.py --lab lab.json --out DIR` |
| L6 | Other masters, only once L0–L5 work | Compare per master |

## Measuring the SPI

The simulator charges every PDI access a fixed cost plus its bytes at an effective rate.
`flowlab pdi-bench` measures those on the Verdin: it performs complete ESC reads and writes
through the LAN9252's SPI protocol, the same paths a slave stack's hardware layer takes (the
CSR interface for registers, the process RAM FIFOs for memory, and the AL Event read SOES does
after every access), and times each one ([flowlab-pdi](../../workspace/flowlab-pdi.md)).

1. **Put `flowlab` on the Verdin**: the aarch64 Linux release binary runs on any of its images
   ([installing](../../getting-started.md#install-a-release-binary)). To build one yourself, see
   [cross-compiling](../../workspace/contributing.md#releases-and-cross-compiling).
2. **Free the ESC.** Stop the slave application (the benchmark needs the SPI device to itself),
   disconnect or stop the master, and leave the ESC in INIT. The SPI device must be bound to
   spidev (`/dev/spidevB.C`, for example through a device-tree overlay).
3. **Run** with the clock and mode the application uses:

   ```bash
   flowlab pdi-bench --device /dev/spidev1.0 --clock-hz 20000000 --mode 0 -o spi.csv
   ```

   The command first reads `BYTE_TEST` (must be 0x87654321), `HW_CFG` and `ID_REV` (chip
   0x9252), and stops with a hint if they are wrong (wiring, SPI mode, clock, level shifting). It
   refuses to run unless the ESC is in INIT, and discards the run if AL Status changes. It never
   accesses the AL Control or SyncManager registers. By default it only reads, from process RAM
   at 0x1000; writes need `--allow-writes` and an `--area START+LEN` of process RAM that nothing
   uses (in INIT nothing does; the area is overwritten):

   ```bash
   flowlab pdi-bench --device /dev/spidev1.0 --clock-hz 20000000 \
       --allow-writes --area 0x1800+1024 -o spi.csv --fit --lab lab.json
   ```

   Sizes default to 1 to 1024 bytes in powers of two (`--sizes`), 50 timed accesses each
   (`--reps`) after 3 untimed ones (`--warmup`), in a shuffled order. Above 30 MHz use
   `--read-instruction fast-read`. It prints the median and 95th percentile per size and writes
   the [PDI timing samples](../../reference/specs.md#pdi-timing-samples), with the setup in
   comment lines.
4. **Fit** (or pass `--fit` above):

   ```bash
   flowlab pdi-fit spi.csv --result spi-fit.json --lab lab.json
   ```

   The output lists, per access size, the median, the 95th percentile and the fitted line. If
   some sizes are far off the line (long accesses split into chunks, a DMA threshold), the line
   is the wrong model for those sizes. Time the sizes the application actually uses
   (`--sizes`), and note it in `lab.json`'s `description`.
5. Repeat with each driver variant (SPI clock, read instruction, kernel driver) as its own file.

`flowlab pdi-bench --sim -o sim.csv` runs the same benchmark against a simulated LAN9252 on any
system, to try the command. Its samples are marked `simulated` (`# flowlab:provenance=simulated`);
`pdi-fit` labels the fit accordingly and refuses to store it in `lab.json`.

The benchmark times one access pattern of its own. A stack that reads in other pieces, or a
kernel driver, can be timed instead by wrapping its ESC read and write functions with a
monotonic clock and logging one line per call (direction, bytes, duration in ns) in the same
samples format, outside the timed section.

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
| Master lacks EoE/FoE/VoE APIs | no transport adapter | verify versions and APIs before choosing; `flowlab master` has FoE, CoE, EoE and FlowLab VoE |
| Licence conflicts (e.g. GPL master) | cannot ship | review before embedding |
| Mailbox scheduling modelled wrongly | misleading optimisation | capture each master's polling; calibrate |
| Bulk traffic disturbs control | safety, determinism | isolated lab, cycle constraints in every study |
| Payload size confused with cadence | wrong technology choice | answer the product questions first |
