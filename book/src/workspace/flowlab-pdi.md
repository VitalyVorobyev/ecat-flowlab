# flowlab-pdi

Timed ESC accesses over the SPI PDI of a Microchip LAN9252: the samples
[`flowlab pdi-fit`](../reference/specs.md#pdi-timing-samples) turns into the simulator's PDI
model. Used by `flowlab pdi-bench` ([Measuring the SPI](../studies/bulk-transfer/lab.md#measuring-the-spi)); the samples file it writes is `flowlab_core::pdi_samples`, which `flowlab-study` reads for the fit.

## Design

The simulator charges every ESC access a fixed cost plus its bytes at an effective rate. The
crate measures that on the target by doing what a slave stack's LAN9252 hardware layer does,
and timing each complete access with a monotonic clock ([ADR-037](decisions.md#live-capture-and-hardware)).

```text
 Bench (sizes × reps, shuffled) ─▶ Lan9252 ─▶ SpiBus ─┬─▶ spidev (/dev/spidevB.C, Linux)
   time each read/write              CSR interface     └─▶ SimLan9252 (virtual clock, --sim)
   ─▶ samples CSV ─▶ pdi-fit         PRAM FIFOs
```

**SPI framing** (LAN9252 data sheet; AN1907): an instruction byte, a 16-bit big-endian address
(14 address bits; bits 15:14 = 00, a fixed address), then data in DWORDs, little-endian.
`WRITE` 0x02 sends instruction, address and data in one buffer; `READ` 0x03 clocks data in after
the address; `FAST READ` 0x0B after one dummy byte (0x01, as SOES sends it). Chip select is held
for all segments of a transaction.

**ESC registers below 0x1000** go through the CSR interface: write `ECAT_CSR_CMD` (0x304) with
busy (bit 31), read (bit 30), the size (bits 18:16) and the address; poll until busy clears;
read `ECAT_CSR_DATA` (0x300). A size is 1, 2 or 4 bytes, aligned; longer accesses are split
(`csr_chunk`, the data sheet's address-vs-size table, as in SOES).

**Process RAM from 0x1000** goes through the FIFOs: abort the previous command
(`ECAT_PRAM_RD_CMD` 0x30C / `ECAT_PRAM_WR_CMD` 0x314, bit 30) and wait for busy to clear;
write address and length (0x308 / 0x310); set busy; then, until all DWORDs have moved, poll the
command register for the available bit and count (bits 12:8) and move up to 16 DWORDs in one
burst at the data FIFO (0x000 read, 0x020 write). The first DWORD carries the bytes from
`address & 3` on. A write waits for busy to clear at the end, so every timed write is complete.

After every access the AL Event Request (0x0220) is read, as the SOES hardware layer does;
`with_al_event_read(false)` (`--no-al-event`) leaves it out.

## Safety

- `check_link` reads `BYTE_TEST` (0x064, must be 0x87654321), `HW_CFG` (0x074, ready bit 27)
  and `ID_REV` (0x050, chip 0x9252 in bits 31:16), then AL Status (0x0130). Wrong wiring, SPI
  mode or clock fails here, before any access of the ESC core.
- `run` refuses unless the ESC is in INIT, and discards the run if AL Status changed by the end.
- `Lan9252::read` and `write` refuse the AL Control (0x0120–0x0121) and SyncManager
  (0x0800–0x087F) registers (`PROTECTED`), anything past the 4 KiB process RAM, and ranges
  crossing 0x1000. `write` accepts process RAM only.
- `plan` checks the options first: writes only with `allow_writes` and an explicit area that
  starts at 0x1000 or above; every size fits its area.

## API

| Item | Purpose |
|---|---|
| `SpiBus`, `Segment` | one transaction with chip select held: write and read segments |
| `open_spidev(path, clock_hz, mode)` | the Linux backend; elsewhere an error saying it needs Linux spidev |
| `Lan9252::new(bus)` | `with_read_instruction`, `with_al_event_read`, `check_link`, `read`, `write`, `read_register` |
| `SimLan9252::new(SimTiming)` | a simulated LAN9252 and its `SimClock`; `accesses()`, `errors()`, `set_byte_test`, `set_id_rev`, `set_al_status` |
| `BenchOptions`, `plan`, `run` | sizes, repetitions, warm-up, area, writes, seed → `BenchRun` (link, plan, samples) |
| `to_csv`, `Sample`, `Direction` | the samples file with its provenance marker (re-exported from `flowlab_core::pdi_samples`) |
| `summarise` | median and p95 per direction and size |
| `reg`, `PROTECTED`, `csr_chunk`, `check_range` | constants and rules |

`SimTiming` (per-transaction cost, SPI clock, CSR busy time, delay until process RAM data) is
chosen, not measured. Samples timed against it are written with
`# flowlab:provenance=simulated`.

## Tests

`crates/flowlab-pdi/src/tests.rs`, against the simulated LAN9252 (which records every ESC
access and any protocol misuse: FIFO underruns, invalid CSR sizes, partial DWORDs):

- SPI framing of `READ` and `FAST READ` against the data sheet's byte layout.
- CSR reads at every alignment return the ESC's registers, in sizes 1, 2 and 4 only.
- Writes to registers and accesses of AL Control, SyncManager registers, past 4 KiB or across
  0x1000 are refused before the bus is touched.
- Process RAM write/read round trips for sizes that are not DWORD multiples, larger than the
  FIFO, at unaligned addresses; bytes around the range stay unchanged; the whole 4 KiB reads
  back.
- A broken link (no answer, swapped bytes) and a wrong chip are refused with a hint; so is an
  ESC outside INIT.
- Writes need `allow_writes` and an area in process RAM; without them a run only reads and
  leaves the memory unchanged; with them every write stays inside the area.
- A run's CSV is parsed and fitted by `flowlab-study`: provenance `simulated`, and the rate equals
  what the simulated bus's per-burst cost implies within 1 %.

The hardware backend has not been run on a LAN9252 yet ([Limitations](../reference/limitations.md)).
