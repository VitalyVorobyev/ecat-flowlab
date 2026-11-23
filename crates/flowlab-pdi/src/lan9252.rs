//! ESC accesses through the SPI protocol of the Microchip LAN9252.
//!
//! The LAN9252's SPI slave reaches its own system registers directly (DWORD-wide, 14-bit byte
//! address). The EtherCAT core behind it is reached indirectly:
//!
//! * ESC registers below 0x1000 through the CSR interface: write `ECAT_CSR_CMD` with the
//!   address, size (1, 2 or 4 bytes, aligned) and the read bit, wait for its busy bit to clear,
//!   then read or write `ECAT_CSR_DATA`;
//! * process RAM (0x1000 and up) through the PRAM FIFOs: abort any previous command, write the
//!   address and length, start the command, then move DWORDs through the 16-DWORD data FIFO as
//!   the command register's available count allows.
//!
//! These are the access paths of the LAN9252 hardware layers of slave stacks (SOES, Microchip's
//! SSC), so timing them times what the stack pays per access. Constants are from the LAN9252
//! data sheet and application notes AN1907 and AN2655; see the book's sources page.

use std::io;

use crate::PdiError;
use crate::spi::{Segment, SpiBus};

/// LAN9252 SPI instructions and system registers (byte addresses).
pub mod reg {
    /// SPI instruction: write DWORDs.
    pub const SPI_WRITE: u8 = 0x02;
    /// SPI instruction: read DWORDs (up to 30 MHz).
    pub const SPI_READ: u8 = 0x03;
    /// SPI instruction: read DWORDs after one dummy byte (up to 80 MHz).
    pub const SPI_FAST_READ: u8 = 0x0B;
    /// Value sent in the dummy byte of a FAST READ (as SOES sends it).
    pub const FAST_READ_DUMMY: u8 = 0x01;
    /// Address field bits 15:14: `01` auto-increments, `10` auto-decrements; `00` (used here)
    /// keeps the address, so a burst at a FIFO port moves the whole burst through the FIFO.
    pub const ADDRESS_MASK: u16 = 0x3FFF;

    /// Process RAM read data FIFO (`ECAT_PRAM_RD_DATA`, 0x000–0x01C).
    pub const PRAM_RD_DATA: u16 = 0x0000;
    /// Process RAM write data FIFO (`ECAT_PRAM_WR_DATA`, 0x020–0x03C).
    pub const PRAM_WR_DATA: u16 = 0x0020;
    /// Chip ID (bits 31:16) and revision (bits 15:0), `ID_REV`.
    pub const ID_REV: u16 = 0x0050;
    /// Byte order test register, `BYTE_TEST`.
    pub const BYTE_TEST: u16 = 0x0064;
    /// Hardware configuration, `HW_CFG`.
    pub const HW_CFG: u16 = 0x0074;
    /// EtherCAT CSR interface data, `ECAT_CSR_DATA`.
    pub const ECAT_CSR_DATA: u16 = 0x0300;
    /// EtherCAT CSR interface command, `ECAT_CSR_CMD`.
    pub const ECAT_CSR_CMD: u16 = 0x0304;
    /// Process RAM read address (15:0) and length in bytes (31:16), `ECAT_PRAM_RD_ADDR_LEN`.
    pub const ECAT_PRAM_RD_ADDR_LEN: u16 = 0x0308;
    /// Process RAM read command, `ECAT_PRAM_RD_CMD`.
    pub const ECAT_PRAM_RD_CMD: u16 = 0x030C;
    /// Process RAM write address and length, `ECAT_PRAM_WR_ADDR_LEN`.
    pub const ECAT_PRAM_WR_ADDR_LEN: u16 = 0x0310;
    /// Process RAM write command, `ECAT_PRAM_WR_CMD`.
    pub const ECAT_PRAM_WR_CMD: u16 = 0x0314;

    /// What `BYTE_TEST` reads when the SPI link works.
    pub const BYTE_TEST_VALUE: u32 = 0x8765_4321;
    /// `HW_CFG`: the device is ready.
    pub const HW_CFG_READY: u32 = 1 << 27;
    /// Chip ID of the LAN9252 in `ID_REV` bits 31:16.
    pub const CHIP_ID_LAN9252: u16 = 0x9252;

    /// `ECAT_CSR_CMD`: command busy (set to start).
    pub const CSR_BUSY: u32 = 1 << 31;
    /// `ECAT_CSR_CMD`: read (clear for write).
    pub const CSR_READ: u32 = 1 << 30;
    /// `ECAT_CSR_CMD`: size in bytes, bits 18:16.
    pub const CSR_SIZE_SHIFT: u32 = 16;

    /// `ECAT_PRAM_*_CMD`: command busy (set to start).
    pub const PRAM_BUSY: u32 = 1 << 31;
    /// `ECAT_PRAM_*_CMD`: abort the command.
    pub const PRAM_ABORT: u32 = 1 << 30;
    /// `ECAT_PRAM_*_CMD`: data available (read) or space available (write).
    pub const PRAM_AVAIL: u32 = 1;
    /// `ECAT_PRAM_*_CMD`: DWORDs available in (read) or free in (write) the FIFO, bits 12:8.
    pub const PRAM_COUNT_SHIFT: u32 = 8;
    /// Mask of the available count after the shift.
    pub const PRAM_COUNT_MASK: u32 = 0x1F;
    /// Depth of each data FIFO, DWORDs.
    pub const FIFO_DWORDS: usize = 16;

    /// First byte of process RAM in the ESC address space.
    pub const PROCESS_RAM_START: u16 = 0x1000;
    /// One past the last byte of the LAN9252's 4 KiB process RAM.
    pub const PROCESS_RAM_END: u32 = 0x2000;
    /// ESC register: AL Status (read only here).
    pub const ESC_AL_STATUS: u16 = 0x0130;
    /// ESC register: AL Event Request, which SOES reads after every access.
    pub const ESC_AL_EVENT: u16 = 0x0220;
}

/// ESC address ranges this crate never reads or writes: `(start, end exclusive, name)`.
/// Reading AL Control clears its event; SyncManager registers belong to the running stack.
pub const PROTECTED: [(u16, u16, &str); 2] = [(0x0120, 0x0122, "AL Control"), (0x0800, 0x0880, "SyncManager")];

/// SPI read instruction used for every read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadInstruction {
    /// `READ` (0x03): instruction and address, then data. Up to 30 MHz.
    Read,
    /// `FAST READ` (0x0B): instruction, address and one dummy byte, then data. Up to 80 MHz.
    FastRead,
}

impl ReadInstruction {
    /// Name as written in the samples header.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Read => "READ",
            Self::FastRead => "FAST READ",
        }
    }
}

/// What the link check read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkInfo {
    /// `BYTE_TEST`, always [`reg::BYTE_TEST_VALUE`] after a passed check.
    pub byte_test: u32,
    /// `ID_REV`.
    pub id_rev: u32,
    /// `HW_CFG`.
    pub hw_cfg: u32,
    /// ESC AL Status (0x0130); the low four bits are the state, 1 = INIT.
    pub al_status: u16,
}

impl LinkInfo {
    /// Chip ID, `ID_REV` bits 31:16.
    pub const fn chip_id(&self) -> u16 {
        (self.id_rev >> 16) as u16
    }

    /// Chip revision, `ID_REV` bits 15:0.
    pub const fn revision(&self) -> u16 {
        self.id_rev as u16
    }

    /// The ESC's state from AL Status: `INIT`, `PREOP`, `BOOT`, `SAFEOP`, `OP` or `0xN`.
    pub fn al_state(&self) -> String {
        match self.al_status & 0x0F {
            1 => "INIT".into(),
            2 => "PREOP".into(),
            3 => "BOOT".into(),
            4 => "SAFEOP".into(),
            8 => "OP".into(),
            s => format!("0x{s:X}"),
        }
    }
}

/// Size of one CSR access at `address` with `left` bytes to go: 4 only on a DWORD boundary,
/// 2 only on a WORD boundary, never 3 (LAN9252 data sheet, EtherCAT CSR address vs size).
/// Three bytes left on a DWORD boundary go as 1 + 2, as in the SOES hardware layer.
pub fn csr_chunk(address: u16, left: usize) -> usize {
    let size = left.min(4);
    if address & 1 != 0 || size == 3 {
        1
    } else if address & 2 != 0 {
        size.min(2)
    } else {
        size
    }
}

/// How the driver uses the SPI bus. [`SpiTuning::STACK`] (the default) accesses the LAN9252
/// the way the slave stacks' hardware layers do, one bus call per LAN9252 transaction;
/// [`SpiTuning::FAST`] uses the shortcuts that paid off on the Verdin iMX8MP rig (split writes,
/// no abort). The bytes on the wire per transaction are the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpiTuning {
    /// Send a write's instruction and address as a transfer of their own ahead of the data, so
    /// that no transfer is longer than 64 bytes: Linux SPI drivers such as the i.MX ECSPI's
    /// move longer transfers by DMA, which costs more than it saves at these sizes.
    pub split_writes: bool,
    /// Several LAN9252 transactions per bus call ([`SpiBus::transactions`]): a CSR command
    /// with the status and data reads after it, the start of a process RAM command with the
    /// status read, a FIFO burst with the status read after it. A status that still shows busy
    /// falls back to polling.
    pub batch: bool,
    /// Fill the write FIFO with as many DWORDs as it has room for, instead of waiting until it
    /// has room for a whole burst.
    pub partial_fifo: bool,
    /// Skip the abort (and its poll) before a process RAM command when the previous command in
    /// that direction finished.
    pub skip_abort: bool,
}

impl SpiTuning {
    /// One bus call per LAN9252 transaction, whole bursts, an abort before every command.
    pub const STACK: Self = Self { split_writes: false, batch: false, partial_fifo: false, skip_abort: false };
    /// Split writes and no abort before a command. Batching and partial FIFO fills gained
    /// nothing on the Verdin iMX8MP rig (each `cs_change` costs the kernel's default 10 µs).
    pub const FAST: Self = Self { split_writes: true, batch: false, partial_fifo: false, skip_abort: true };

    /// Parse `stack`, `fast` or a comma-separated list of `split`, `batch`, `fifo`, `noabort`.
    pub fn parse(s: &str) -> Result<Self, PdiError> {
        match s {
            "stack" => return Ok(Self::STACK),
            "fast" => return Ok(Self::FAST),
            _ => {}
        }
        let mut t = Self::STACK;
        for part in s.split(',').map(str::trim) {
            match part {
                "split" => t.split_writes = true,
                "batch" => t.batch = true,
                "fifo" => t.partial_fifo = true,
                "noabort" => t.skip_abort = true,
                other => {
                    return Err(PdiError::Options(format!(
                        "SPI tuning {other:?}: expected stack, fast or a list of split, batch, fifo, noabort"
                    )));
                }
            }
        }
        Ok(t)
    }

    /// `stack`, `fast` or the list of shortcuts.
    pub fn name(&self) -> String {
        if *self == Self::STACK {
            return "stack".into();
        }
        if *self == Self::FAST {
            return "fast".into();
        }
        let names = [
            (self.split_writes, "split"),
            (self.batch, "batch"),
            (self.partial_fifo, "fifo"),
            (self.skip_abort, "noabort"),
        ];
        names.iter().filter(|(on, _)| *on).map(|(_, n)| *n).collect::<Vec<_>>().join(",")
    }
}

impl Default for SpiTuning {
    fn default() -> Self {
        Self::STACK
    }
}

/// A LAN9252 on an SPI bus.
#[derive(Debug)]
pub struct Lan9252<S> {
    bus: S,
    read_instruction: ReadInstruction,
    al_event_read: bool,
    tuning: SpiTuning,
    /// The last process RAM read / write command finished (no abort needed before the next).
    read_idle: bool,
    write_idle: bool,
    max_polls: u32,
    tx: Vec<u8>,
}

impl<S: SpiBus> Lan9252<S> {
    /// A LAN9252 on `bus`, read with `READ`, with the AL Event read after every access (as SOES
    /// does).
    pub fn new(bus: S) -> Self {
        Self {
            bus,
            read_instruction: ReadInstruction::Read,
            al_event_read: true,
            tuning: SpiTuning::STACK,
            read_idle: false,
            write_idle: false,
            max_polls: 100_000,
            tx: Vec::with_capacity(3 + 4 * reg::FIFO_DWORDS),
        }
    }

    /// Use this SPI read instruction.
    pub fn with_read_instruction(mut self, r: ReadInstruction) -> Self {
        self.read_instruction = r;
        self
    }

    /// Use the bus this way (see [`SpiTuning`]).
    pub fn with_tuning(mut self, tuning: SpiTuning) -> Self {
        self.tuning = tuning;
        self
    }

    /// How the bus is used.
    pub fn tuning(&self) -> SpiTuning {
        self.tuning
    }

    /// Read AL Event Request (0x0220) after every [`read`](Self::read) and
    /// [`write`](Self::write), as the SOES hardware layer does.
    pub fn with_al_event_read(mut self, on: bool) -> Self {
        self.al_event_read = on;
        self
    }

    /// The bus.
    pub fn bus(&self) -> &S {
        &self.bus
    }

    /// The bus, mutably.
    pub fn bus_mut(&mut self) -> &mut S {
        &mut self.bus
    }

    /// Read a LAN9252 system register (a DWORD at a byte address below 0x4000).
    pub fn read_register(&mut self, address: u16) -> Result<u32, PdiError> {
        let mut v = [0u8; 4];
        self.spi_read(address, &mut v)?;
        Ok(u32::from_le_bytes(v))
    }

    /// Check the SPI link before anything else: `BYTE_TEST` must read 0x87654321, `HW_CFG`
    /// must say ready and `ID_REV` must carry `chip_id` (0x9252 for a LAN9252). Then read AL
    /// Status. Only reads.
    pub fn check_link(&mut self, chip_id: u16) -> Result<LinkInfo, PdiError> {
        let byte_test = self.read_register(reg::BYTE_TEST)?;
        if byte_test != reg::BYTE_TEST_VALUE {
            let hint = match byte_test {
                0 | 0xFFFF_FFFF => "nothing answers: check chip select, wiring, power and the device path",
                v if v == reg::BYTE_TEST_VALUE.swap_bytes() => "bytes swapped: check the board's PDI mode",
                _ => "check the SPI mode (0 or 3), a lower clock, level shifting and that the PDI is set to SPI",
            };
            return Err(PdiError::Link(format!(
                "BYTE_TEST (0x0064) reads 0x{byte_test:08X}, expected 0x{:08X}; {hint}",
                reg::BYTE_TEST_VALUE
            )));
        }
        let hw_cfg = self.read_register(reg::HW_CFG)?;
        if hw_cfg & reg::HW_CFG_READY == 0 {
            return Err(PdiError::Link(format!("HW_CFG (0x0074) reads 0x{hw_cfg:08X}: the device is not ready")));
        }
        let id_rev = self.read_register(reg::ID_REV)?;
        if (id_rev >> 16) as u16 != chip_id {
            return Err(PdiError::Link(format!(
                "ID_REV (0x0050) reads 0x{id_rev:08X}: chip 0x{:04X}, expected 0x{chip_id:04X}",
                id_rev >> 16
            )));
        }
        let mut al = [0u8; 2];
        self.csr_read(reg::ESC_AL_STATUS, &mut al)?;
        Ok(LinkInfo { byte_test, id_rev, hw_cfg, al_status: u16::from_le_bytes(al) })
    }

    /// Read `buf.len()` bytes of the ESC address space at `address`, the way a slave stack's
    /// hardware layer does: through the CSR interface below 0x1000, through the PRAM read FIFO
    /// from 0x1000. Refuses [`PROTECTED`] ranges and addresses beyond the process RAM.
    pub fn read(&mut self, address: u16, buf: &mut [u8]) -> Result<(), PdiError> {
        check_range(address, buf.len())?;
        if address >= reg::PROCESS_RAM_START {
            self.pram_read(address, buf)?;
        } else {
            let mut a = address;
            let mut done = 0;
            while done < buf.len() {
                let n = csr_chunk(a, buf.len() - done);
                self.csr_read(a, &mut buf[done..done + n])?;
                done += n;
                a += n as u16;
            }
        }
        self.al_event()
    }

    /// Write `data` to process RAM at `address` through the PRAM write FIFO, and wait until
    /// the ESC has taken it. Only process RAM (0x1000 and up) is writable here.
    pub fn write(&mut self, address: u16, data: &[u8]) -> Result<(), PdiError> {
        check_range(address, data.len())?;
        if address < reg::PROCESS_RAM_START {
            return Err(PdiError::Refused(format!(
                "write to 0x{address:04X}: only process RAM (0x1000 and up) is written, never ESC registers"
            )));
        }
        self.pram_write(address, data)?;
        self.al_event()
    }

    fn al_event(&mut self) -> Result<(), PdiError> {
        if self.al_event_read {
            let mut e = [0u8; 2];
            self.csr_read(reg::ESC_AL_EVENT, &mut e)?;
        }
        Ok(())
    }

    /// One CSR read of 1, 2 or 4 aligned bytes.
    pub(crate) fn csr_read(&mut self, address: u16, out: &mut [u8]) -> Result<(), PdiError> {
        let size = out.len() as u32;
        let cmd = reg::CSR_BUSY | reg::CSR_READ | (size << reg::CSR_SIZE_SHIFT) | u32::from(address);
        let v = if self.tuning.batch {
            let w = write_bytes(reg::ECAT_CSR_CMD, cmd);
            let (h1, n1) = self.read_header(reg::ECAT_CSR_CMD);
            let (h2, n2) = self.read_header(reg::ECAT_CSR_DATA);
            let (mut status, mut data) = ([0u8; 4], [0u8; 4]);
            self.bus.transactions(&mut [
                &mut [Segment::Write(&w)][..],
                &mut [Segment::Write(&h1[..n1]), Segment::Read(&mut status)][..],
                &mut [Segment::Write(&h2[..n2]), Segment::Read(&mut data)][..],
            ])?;
            if u32::from_le_bytes(status) & reg::CSR_BUSY == 0 {
                data
            } else {
                self.poll(reg::ECAT_CSR_CMD, "ECAT_CSR_CMD busy to clear", |v| v & reg::CSR_BUSY == 0)?;
                self.read_register(reg::ECAT_CSR_DATA)?.to_le_bytes()
            }
        } else {
            self.write_register(reg::ECAT_CSR_CMD, cmd)?;
            self.poll(reg::ECAT_CSR_CMD, "ECAT_CSR_CMD busy to clear", |v| v & reg::CSR_BUSY == 0)?;
            self.read_register(reg::ECAT_CSR_DATA)?.to_le_bytes()
        };
        out.copy_from_slice(&v[..out.len()]);
        Ok(())
    }

    /// One CSR write of 1, 2 or 4 aligned bytes: the data first, then the command, then wait
    /// until it is done.
    pub(crate) fn csr_write(&mut self, address: u16, data: &[u8]) -> Result<(), PdiError> {
        let mut v = [0u8; 4];
        v[..data.len()].copy_from_slice(data);
        let cmd = reg::CSR_BUSY | ((data.len() as u32) << reg::CSR_SIZE_SHIFT) | u32::from(address);
        if self.tuning.batch {
            let d = write_bytes(reg::ECAT_CSR_DATA, u32::from_le_bytes(v));
            let c = write_bytes(reg::ECAT_CSR_CMD, cmd);
            let (h, n) = self.read_header(reg::ECAT_CSR_CMD);
            let mut status = [0u8; 4];
            self.bus.transactions(&mut [
                &mut [Segment::Write(&d)][..],
                &mut [Segment::Write(&c)][..],
                &mut [Segment::Write(&h[..n]), Segment::Read(&mut status)][..],
            ])?;
            if u32::from_le_bytes(status) & reg::CSR_BUSY == 0 {
                return Ok(());
            }
        } else {
            self.write_register(reg::ECAT_CSR_DATA, u32::from_le_bytes(v))?;
            self.write_register(reg::ECAT_CSR_CMD, cmd)?;
        }
        self.poll(reg::ECAT_CSR_CMD, "ECAT_CSR_CMD busy to clear", |v| v & reg::CSR_BUSY == 0)?;
        Ok(())
    }

    pub(crate) fn pram_read(&mut self, address: u16, buf: &mut [u8]) -> Result<(), PdiError> {
        let mut status = self.pram_start(Way::Read, address, buf.len())?;
        let mut skip = usize::from(address & 3);
        let mut left = (skip + buf.len()).div_ceil(4);
        let mut done = 0;
        let mut chunk = [0u8; 4 * reg::FIFO_DWORDS];
        while left > 0 {
            let want = left.min(reg::FIFO_DWORDS);
            let ready = |v: u32| v & reg::PRAM_AVAIL != 0 && fifo_count(v) >= want;
            let v = match status {
                Some(v) if ready(v) => v,
                _ => self.poll(reg::ECAT_PRAM_RD_CMD, "ECAT_PRAM_RD_CMD to have data", ready)?,
            };
            let n = fifo_count(v).min(left);
            let bytes = &mut chunk[..4 * n];
            if self.tuning.batch && n < left {
                let (h, hn) = self.read_header(reg::PRAM_RD_DATA);
                let (hs, hsn) = self.read_header(reg::ECAT_PRAM_RD_CMD);
                let mut st = [0u8; 4];
                self.bus.transactions(&mut [
                    &mut [Segment::Write(&h[..hn]), Segment::Read(bytes)][..],
                    &mut [Segment::Write(&hs[..hsn]), Segment::Read(&mut st)][..],
                ])?;
                status = Some(u32::from_le_bytes(st));
            } else {
                self.spi_read(reg::PRAM_RD_DATA, bytes)?;
                status = None;
            }
            for &b in bytes.iter() {
                if skip > 0 {
                    skip -= 1;
                } else if done < buf.len() {
                    buf[done] = b;
                    done += 1;
                }
            }
            left -= n;
        }
        self.read_idle = true;
        Ok(())
    }

    pub(crate) fn pram_write(&mut self, address: u16, data: &[u8]) -> Result<(), PdiError> {
        let mut status = self.pram_start(Way::Write, address, data.len())?;
        let offset = usize::from(address & 3);
        let total = (offset + data.len()).div_ceil(4);
        let mut sent = 0;
        let mut chunk = [0u8; 4 * reg::FIFO_DWORDS];
        while sent < total {
            let want = (total - sent).min(reg::FIFO_DWORDS);
            let need = if self.tuning.partial_fifo { 1 } else { want };
            let ready = |v: u32| v & reg::PRAM_AVAIL != 0 && fifo_count(v) >= need;
            let v = match status {
                Some(v) if ready(v) => v,
                _ => self.poll(reg::ECAT_PRAM_WR_CMD, "ECAT_PRAM_WR_CMD to have space", ready)?,
            };
            let n = fifo_count(v).min(total - sent);
            // Byte i of the DWORD stream is data[i - offset]; bytes outside the data are padding.
            for (k, b) in chunk[..4 * n].iter_mut().enumerate() {
                let i = 4 * sent + k;
                *b = i.checked_sub(offset).and_then(|j| data.get(j)).copied().unwrap_or(0);
            }
            if self.tuning.batch {
                let h = write_header(reg::PRAM_WR_DATA);
                let (hs, hsn) = self.read_header(reg::ECAT_PRAM_WR_CMD);
                let mut st = [0u8; 4];
                let mut tx = std::mem::take(&mut self.tx);
                let mut data_segments = if self.tuning.split_writes {
                    vec![Segment::Write(&h), Segment::Write(&chunk[..4 * n])]
                } else {
                    tx.clear();
                    tx.extend_from_slice(&h);
                    tx.extend_from_slice(&chunk[..4 * n]);
                    vec![Segment::Write(&tx)]
                };
                let r = self.bus.transactions(&mut [
                    &mut data_segments[..],
                    &mut [Segment::Write(&hs[..hsn]), Segment::Read(&mut st)][..],
                ]);
                drop(data_segments);
                self.tx = tx;
                r?;
                status = Some(u32::from_le_bytes(st));
            } else {
                self.spi_write(reg::PRAM_WR_DATA, &chunk[..4 * n])?;
                status = None;
            }
            sent += n;
        }
        match status {
            Some(v) if v & reg::PRAM_BUSY == 0 => {}
            _ => {
                self.poll(reg::ECAT_PRAM_WR_CMD, "ECAT_PRAM_WR_CMD to complete", |v| v & reg::PRAM_BUSY == 0)?;
            }
        }
        self.write_idle = true;
        Ok(())
    }

    /// Start a process RAM command for `len` bytes at `address`: abort a previous one and wait
    /// for it (unless [`SpiTuning::skip_abort`] and the previous one finished), write the
    /// address and length, set the busy bit. With [`SpiTuning::batch`], the command register
    /// is read in the same bus call and returned.
    fn pram_start(&mut self, way: Way, address: u16, len: usize) -> Result<Option<u32>, PdiError> {
        let (cmd, addr_len) = match way {
            Way::Read => (reg::ECAT_PRAM_RD_CMD, reg::ECAT_PRAM_RD_ADDR_LEN),
            Way::Write => (reg::ECAT_PRAM_WR_CMD, reg::ECAT_PRAM_WR_ADDR_LEN),
        };
        let idle = match way {
            Way::Read => std::mem::replace(&mut self.read_idle, false),
            Way::Write => std::mem::replace(&mut self.write_idle, false),
        };
        if !(self.tuning.skip_abort && idle) {
            self.write_register(cmd, reg::PRAM_ABORT)?;
            self.poll(cmd, "the PRAM command to abort", |v| v & reg::PRAM_BUSY == 0)?;
        }
        let al = ((len as u32) << 16) | u32::from(address);
        if self.tuning.batch {
            let a = write_bytes(addr_len, al);
            let c = write_bytes(cmd, reg::PRAM_BUSY);
            let (h, n) = self.read_header(cmd);
            let mut st = [0u8; 4];
            self.bus.transactions(&mut [
                &mut [Segment::Write(&a)][..],
                &mut [Segment::Write(&c)][..],
                &mut [Segment::Write(&h[..n]), Segment::Read(&mut st)][..],
            ])?;
            return Ok(Some(u32::from_le_bytes(st)));
        }
        self.write_register(addr_len, al)?;
        self.write_register(cmd, reg::PRAM_BUSY)?;
        Ok(None)
    }

    fn poll(&mut self, register: u16, what: &'static str, done: impl Fn(u32) -> bool) -> Result<u32, PdiError> {
        for _ in 0..self.max_polls {
            let v = self.read_register(register)?;
            if done(v) {
                return Ok(v);
            }
        }
        Err(PdiError::Timeout { what, polls: self.max_polls })
    }

    fn write_register(&mut self, address: u16, value: u32) -> Result<(), PdiError> {
        self.spi_write(address, &value.to_le_bytes())
    }

    /// One read transaction of whole DWORDs at a fixed address.
    fn spi_read(&mut self, address: u16, out: &mut [u8]) -> io::Result<()> {
        let (h, n) = self.read_header(address);
        self.bus.transaction(&mut [Segment::Write(&h[..n]), Segment::Read(out)])
    }

    /// The read instruction and address (and the dummy byte of `FAST READ`): bytes and length.
    fn read_header(&self, address: u16) -> ([u8; 4], usize) {
        let [hi, lo] = (address & reg::ADDRESS_MASK).to_be_bytes();
        match self.read_instruction {
            ReadInstruction::Read => ([reg::SPI_READ, hi, lo, 0], 3),
            ReadInstruction::FastRead => ([reg::SPI_FAST_READ, hi, lo, reg::FAST_READ_DUMMY], 4),
        }
    }

    /// One write transaction of whole DWORDs at a fixed address: instruction, address and data
    /// in one buffer, as the stacks send it.
    fn spi_write(&mut self, address: u16, data: &[u8]) -> Result<(), PdiError> {
        if self.tuning.split_writes {
            let h = write_header(address);
            return Ok(self.bus.transaction(&mut [Segment::Write(&h), Segment::Write(data)])?);
        }
        let a = address & reg::ADDRESS_MASK;
        let mut tx = std::mem::take(&mut self.tx);
        tx.clear();
        tx.push(reg::SPI_WRITE);
        tx.extend_from_slice(&a.to_be_bytes());
        tx.extend_from_slice(data);
        let r = self.bus.transaction(&mut [Segment::Write(&tx)]);
        self.tx = tx;
        Ok(r?)
    }
}

/// Direction of a process RAM command.
#[derive(Debug, Clone, Copy)]
enum Way {
    Read,
    Write,
}

/// The write instruction and address.
fn write_header(address: u16) -> [u8; 3] {
    let [hi, lo] = (address & reg::ADDRESS_MASK).to_be_bytes();
    [reg::SPI_WRITE, hi, lo]
}

/// A whole register write: instruction, address, the DWORD.
fn write_bytes(address: u16, value: u32) -> [u8; 7] {
    let [i, hi, lo] = write_header(address);
    let [a, b, c, d] = value.to_le_bytes();
    [i, hi, lo, a, b, c, d]
}

fn fifo_count(cmd: u32) -> usize {
    ((cmd >> reg::PRAM_COUNT_SHIFT) & reg::PRAM_COUNT_MASK) as usize
}

/// Refuse accesses outside `0..0x2000`, empty or overlapping a [`PROTECTED`] range, or
/// crossing from registers into process RAM.
pub fn check_range(address: u16, len: usize) -> Result<(), PdiError> {
    let start = u32::from(address);
    let end = start + len as u32;
    if len == 0 || end > reg::PROCESS_RAM_END {
        return Err(PdiError::Refused(format!(
            "{len} bytes at 0x{address:04X}: accesses must be non-empty and end by 0x{:04X} (4 KiB process RAM)",
            reg::PROCESS_RAM_END
        )));
    }
    if start < u32::from(reg::PROCESS_RAM_START) && end > u32::from(reg::PROCESS_RAM_START) {
        return Err(PdiError::Refused(format!(
            "{len} bytes at 0x{address:04X} cross from ESC registers into process RAM (0x1000)"
        )));
    }
    for (s, e, name) in PROTECTED {
        if start < u32::from(e) && end > u32::from(s) {
            return Err(PdiError::Refused(format!(
                "{len} bytes at 0x{address:04X} touch the {name} registers (0x{s:04X}..0x{e:04X}), which this tool never accesses"
            )));
        }
    }
    Ok(())
}
