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

/// A LAN9252 on an SPI bus.
#[derive(Debug)]
pub struct Lan9252<S> {
    bus: S,
    read_instruction: ReadInstruction,
    al_event_read: bool,
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
            max_polls: 100_000,
            tx: Vec::with_capacity(3 + 4 * reg::FIFO_DWORDS),
        }
    }

    /// Use this SPI read instruction.
    pub fn with_read_instruction(mut self, r: ReadInstruction) -> Self {
        self.read_instruction = r;
        self
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
    fn csr_read(&mut self, address: u16, out: &mut [u8]) -> Result<(), PdiError> {
        let size = out.len() as u32;
        let cmd = reg::CSR_BUSY | reg::CSR_READ | (size << reg::CSR_SIZE_SHIFT) | u32::from(address);
        self.write_register(reg::ECAT_CSR_CMD, cmd)?;
        self.poll(reg::ECAT_CSR_CMD, "ECAT_CSR_CMD busy to clear", |v| v & reg::CSR_BUSY == 0)?;
        let v = self.read_register(reg::ECAT_CSR_DATA)?.to_le_bytes();
        out.copy_from_slice(&v[..out.len()]);
        Ok(())
    }

    fn pram_read(&mut self, address: u16, buf: &mut [u8]) -> Result<(), PdiError> {
        self.pram_start(reg::ECAT_PRAM_RD_CMD, reg::ECAT_PRAM_RD_ADDR_LEN, address, buf.len())?;
        let mut skip = usize::from(address & 3);
        let mut left = (skip + buf.len()).div_ceil(4);
        let mut done = 0;
        let mut chunk = [0u8; 4 * reg::FIFO_DWORDS];
        while left > 0 {
            let want = left.min(reg::FIFO_DWORDS);
            let v = self.poll(reg::ECAT_PRAM_RD_CMD, "ECAT_PRAM_RD_CMD to have data", |v| {
                v & reg::PRAM_AVAIL != 0 && fifo_count(v) >= want
            })?;
            let n = fifo_count(v).min(left);
            let bytes = &mut chunk[..4 * n];
            self.spi_read(reg::PRAM_RD_DATA, bytes)?;
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
        Ok(())
    }

    fn pram_write(&mut self, address: u16, data: &[u8]) -> Result<(), PdiError> {
        self.pram_start(reg::ECAT_PRAM_WR_CMD, reg::ECAT_PRAM_WR_ADDR_LEN, address, data.len())?;
        let offset = usize::from(address & 3);
        let total = (offset + data.len()).div_ceil(4);
        let mut sent = 0;
        let mut chunk = [0u8; 4 * reg::FIFO_DWORDS];
        while sent < total {
            let want = (total - sent).min(reg::FIFO_DWORDS);
            let v = self.poll(reg::ECAT_PRAM_WR_CMD, "ECAT_PRAM_WR_CMD to have space", |v| {
                v & reg::PRAM_AVAIL != 0 && fifo_count(v) >= want
            })?;
            let n = fifo_count(v).min(total - sent);
            // Byte i of the DWORD stream is data[i - offset]; bytes outside the data are padding.
            for (k, b) in chunk[..4 * n].iter_mut().enumerate() {
                let i = 4 * sent + k;
                *b = i.checked_sub(offset).and_then(|j| data.get(j)).copied().unwrap_or(0);
            }
            self.spi_write(reg::PRAM_WR_DATA, &chunk[..4 * n])?;
            sent += n;
        }
        self.poll(reg::ECAT_PRAM_WR_CMD, "ECAT_PRAM_WR_CMD to complete", |v| v & reg::PRAM_BUSY == 0)?;
        Ok(())
    }

    /// Abort a previous command, wait for it, then start a new one for `len` bytes at `address`.
    fn pram_start(&mut self, cmd: u16, addr_len: u16, address: u16, len: usize) -> Result<(), PdiError> {
        self.write_register(cmd, reg::PRAM_ABORT)?;
        self.poll(cmd, "the PRAM command to abort", |v| v & reg::PRAM_BUSY == 0)?;
        self.write_register(addr_len, ((len as u32) << 16) | u32::from(address))?;
        self.write_register(cmd, reg::PRAM_BUSY)
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
        let a = address & reg::ADDRESS_MASK;
        let [hi, lo] = a.to_be_bytes();
        let header_read = [reg::SPI_READ, hi, lo];
        let header_fast = [reg::SPI_FAST_READ, hi, lo, reg::FAST_READ_DUMMY];
        let header: &[u8] = match self.read_instruction {
            ReadInstruction::Read => &header_read,
            ReadInstruction::FastRead => &header_fast,
        };
        self.bus.transaction(&mut [Segment::Write(header), Segment::Read(out)])
    }

    /// One write transaction of whole DWORDs at a fixed address: instruction, address and data
    /// in one buffer, as the stacks send it.
    fn spi_write(&mut self, address: u16, data: &[u8]) -> Result<(), PdiError> {
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
