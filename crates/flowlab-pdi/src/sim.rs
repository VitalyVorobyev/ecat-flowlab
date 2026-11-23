//! A simulated LAN9252 behind the [`SpiBus`] trait: SPI framing, system registers, the CSR
//! interface and the process RAM FIFOs, on a virtual clock. For tests and `--sim` dry runs.
//!
//! The timing is chosen, not measured: every transaction costs a fixed time plus its bytes at
//! the SPI clock, a CSR command stays busy for a while, and a process RAM read has its first data
//! after a delay. Durations timed against it are labelled `simulated`.

use std::collections::VecDeque;
use std::io;

use crate::lan9252::reg;
use crate::spi::{Clock, Segment, SimClock, SpiBus};

/// Timing of the simulated LAN9252 and its SPI bus. All values are chosen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SimTiming {
    /// SPI clock, Hz: each byte costs 8 clock periods.
    pub clock_hz: u64,
    /// Fixed cost of every transaction (driver call, chip select, setup), ns.
    pub transaction_ns: u64,
    /// Time an `ECAT_CSR_CMD` command stays busy, ns.
    pub csr_busy_ns: u64,
    /// Time from starting a process RAM read until the FIFO has data, ns.
    pub pram_ready_ns: u64,
}

impl Default for SimTiming {
    fn default() -> Self {
        Self { clock_hz: 10_000_000, transaction_ns: 5_000, csr_busy_ns: 300, pram_ready_ns: 600 }
    }
}

impl SimTiming {
    /// Virtual time of one transaction moving `bytes` bytes, ns.
    pub fn transaction_cost_ns(&self, bytes: u64) -> u64 {
        self.transaction_ns + (bytes * 8 * 1_000_000_000).div_ceil(self.clock_hz.max(1))
    }
}

/// One access of the simulated ESC core's memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EscAccess {
    /// Written (else read).
    pub write: bool,
    /// ESC address.
    pub address: u16,
    /// Bytes.
    pub len: u16,
    /// Through the process RAM FIFOs (else the CSR interface).
    pub via_fifo: bool,
}

#[derive(Debug)]
struct ReadOp {
    started_ns: u64,
    queue: VecDeque<u32>,
}

#[derive(Debug)]
struct WriteOp {
    start: u32,
    end: u32,
    pos: u32,
}

/// A simulated LAN9252 with 4 KiB of process RAM, in INIT.
#[derive(Debug)]
pub struct SimLan9252 {
    timing: SimTiming,
    clock: SimClock,
    now_ns: u64,
    esc: Vec<u8>,
    byte_test: u32,
    id_rev: u32,
    hw_cfg: u32,
    csr_data: u32,
    csr_cmd: u32,
    csr_done_ns: u64,
    rd_addr_len: u32,
    wr_addr_len: u32,
    rd: Option<ReadOp>,
    wr: Option<WriteOp>,
    accesses: Vec<EscAccess>,
    errors: Vec<String>,
    transactions: u64,
    /// Registers 0–31 of the PHYs at MII addresses 0–3.
    phys: [[u16; 32]; 4],
}

/// MII Management Control as the simulated ESC shows it: the PDI may take the interface.
const SIM_MII_CONTROL: u16 = crate::mii::mii_reg::PDI_POSSIBLE;

/// A LAN9252 PHY after reset with a link partner: auto-negotiation done at 100 Mbit/s full
/// duplex, auto-MDIX.
fn sim_phy(address: u16) -> [u16; 32] {
    let mut r = [0u16; 32];
    r[0] = 0x3100;
    r[1] = 0x782D;
    r[2] = 0x0007;
    r[3] = 0xC0F1;
    r[4] = 0x01E1;
    r[5] = 0x45E1;
    r[18] = 0x00E0 | address;
    r[27] = 0x400A;
    r[31] = 0x1058;
    r
}

impl SimLan9252 {
    /// A simulated LAN9252 (`ID_REV` 0x92520001, AL Status INIT) whose process RAM holds a
    /// fixed pattern, and the clock it advances.
    pub fn new(timing: SimTiming) -> (Self, SimClock) {
        let mut esc = vec![0u8; reg::PROCESS_RAM_END as usize];
        for (i, b) in esc.iter_mut().enumerate().skip(usize::from(reg::PROCESS_RAM_START)) {
            *b = (i as u8).wrapping_mul(7).wrapping_add(3);
        }
        esc[usize::from(reg::ESC_AL_STATUS)] = 0x01;
        let clock = SimClock::default();
        let mut sim = Self {
            timing,
            clock: clock.clone(),
            now_ns: 0,
            esc,
            byte_test: reg::BYTE_TEST_VALUE,
            id_rev: 0x9252_0001,
            hw_cfg: reg::HW_CFG_READY,
            csr_data: 0,
            csr_cmd: 0,
            csr_done_ns: 0,
            rd_addr_len: 0,
            wr_addr_len: 0,
            rd: None,
            wr: None,
            accesses: Vec::new(),
            errors: Vec::new(),
            transactions: 0,
            phys: [sim_phy(0), sim_phy(1), [0xFFFF; 32], [0xFFFF; 32]],
        };
        let c = usize::from(crate::mii::mii_reg::CONTROL);
        sim.esc[c..c + 2].copy_from_slice(&SIM_MII_CONTROL.to_le_bytes());
        (sim, clock)
    }

    /// Make `BYTE_TEST` read `v` (a broken link).
    pub fn set_byte_test(&mut self, v: u32) {
        self.byte_test = v;
    }

    /// Make `ID_REV` read `v`.
    pub fn set_id_rev(&mut self, v: u32) {
        self.id_rev = v;
    }

    /// Set the ESC's AL Status register (low four bits: the state).
    pub fn set_al_status(&mut self, v: u16) {
        let a = usize::from(reg::ESC_AL_STATUS);
        self.esc[a..a + 2].copy_from_slice(&v.to_le_bytes());
    }

    /// The ESC core's memory, 0x0000–0x1FFF.
    pub fn esc(&self) -> &[u8] {
        &self.esc
    }

    /// The ESC core's memory, to set up what the master side would have written.
    pub fn esc_mut(&mut self) -> &mut [u8] {
        &mut self.esc
    }

    /// Every access of the ESC core's memory so far.
    pub fn accesses(&self) -> &[EscAccess] {
        &self.accesses
    }

    /// Protocol misuse seen so far (FIFO underruns, invalid CSR sizes, partial DWORDs, …).
    pub fn errors(&self) -> &[String] {
        &self.errors
    }

    /// SPI transactions so far.
    pub fn transactions(&self) -> u64 {
        self.transactions
    }

    /// Register `register` of the PHY at MII address `phy` (0–3).
    pub fn phy_register(&self, phy: u8, register: u8) -> u16 {
        self.phys[usize::from(phy & 3)][usize::from(register & 31)]
    }

    /// Run the MII command just written to MII Management Control.
    fn mii_command(&mut self) {
        use crate::mii::mii_reg;
        let at = |a: u16| usize::from(a);
        let cmd = u16::from_le_bytes([self.esc[at(mii_reg::CONTROL)], self.esc[at(mii_reg::CONTROL) + 1]]);
        let mut control = SIM_MII_CONTROL;
        let (phy, register) = (self.esc[at(mii_reg::PHY_ADDRESS)], self.esc[at(mii_reg::PHY_REGISTER)]);
        let owned = self.esc[at(mii_reg::PDI_ACCESS)] & 1 != 0;
        match cmd & 0x0300 {
            0 => {}
            _ if !owned || register > 31 => {
                self.errors.push(format!("MII command 0x{cmd:04X} without PDI access or to register {register}"));
                control |= mii_reg::COMMAND_ERROR;
            }
            mii_reg::CMD_READ => {
                let v = if phy < 4 { self.phys[usize::from(phy)][usize::from(register)] } else { 0xFFFF };
                self.esc[at(mii_reg::PHY_DATA)..at(mii_reg::PHY_DATA) + 2].copy_from_slice(&v.to_le_bytes());
            }
            mii_reg::CMD_WRITE => {
                let d = at(mii_reg::PHY_DATA);
                if phy < 4 {
                    self.phys[usize::from(phy)][usize::from(register)] =
                        u16::from_le_bytes([self.esc[d], self.esc[d + 1]]);
                }
            }
            _ => control |= mii_reg::COMMAND_ERROR,
        }
        self.esc[at(mii_reg::CONTROL)..at(mii_reg::CONTROL) + 2].copy_from_slice(&control.to_le_bytes());
    }

    fn read_dword(&mut self, a: u16) -> u32 {
        match a {
            0x000..=0x01C => {
                let now = self.now_ns;
                let ready = self.timing.pram_ready_ns;
                match self.rd.as_mut() {
                    Some(op) if now >= op.started_ns + ready && !op.queue.is_empty() => {
                        let v = op.queue.pop_front().unwrap_or(0);
                        if op.queue.is_empty() {
                            self.rd = None;
                        }
                        v
                    }
                    _ => {
                        self.errors.push("read FIFO underrun".into());
                        0
                    }
                }
            }
            reg::ID_REV => self.id_rev,
            reg::BYTE_TEST => self.byte_test,
            reg::HW_CFG => self.hw_cfg,
            reg::ECAT_CSR_DATA => self.csr_data,
            reg::ECAT_CSR_CMD => {
                if self.now_ns < self.csr_done_ns {
                    self.csr_cmd | reg::CSR_BUSY
                } else {
                    self.csr_cmd & !reg::CSR_BUSY
                }
            }
            reg::ECAT_PRAM_RD_ADDR_LEN => self.rd_addr_len,
            reg::ECAT_PRAM_WR_ADDR_LEN => self.wr_addr_len,
            reg::ECAT_PRAM_RD_CMD => match &self.rd {
                None => 0,
                Some(op) => {
                    let ready = self.now_ns >= op.started_ns + self.timing.pram_ready_ns;
                    let n = if ready { op.queue.len().min(reg::FIFO_DWORDS) as u32 } else { 0 };
                    reg::PRAM_BUSY | (n << reg::PRAM_COUNT_SHIFT) | u32::from(n > 0)
                }
            },
            reg::ECAT_PRAM_WR_CMD => match &self.wr {
                None => 0,
                Some(_) => reg::PRAM_BUSY | ((reg::FIFO_DWORDS as u32) << reg::PRAM_COUNT_SHIFT) | reg::PRAM_AVAIL,
            },
            _ => 0,
        }
    }

    fn write_dword(&mut self, a: u16, v: u32) {
        match a {
            0x020..=0x03C => self.push_write_fifo(v),
            reg::ECAT_CSR_DATA => self.csr_data = v,
            reg::ECAT_CSR_CMD => self.csr_command(v),
            reg::ECAT_PRAM_RD_ADDR_LEN => self.rd_addr_len = v,
            reg::ECAT_PRAM_WR_ADDR_LEN => self.wr_addr_len = v,
            reg::ECAT_PRAM_RD_CMD => {
                if v & reg::PRAM_ABORT != 0 {
                    self.rd = None;
                } else if v & reg::PRAM_BUSY != 0 {
                    self.start_read();
                }
            }
            reg::ECAT_PRAM_WR_CMD => {
                if v & reg::PRAM_ABORT != 0 {
                    self.wr = None;
                } else if v & reg::PRAM_BUSY != 0 {
                    self.start_write();
                }
            }
            _ => self.errors.push(format!("write of 0x{v:08X} to system register 0x{a:03X}")),
        }
    }

    fn pram_range(&mut self, addr_len: u32) -> Option<(u32, u32)> {
        let (start, len) = (addr_len & 0xFFFF, addr_len >> 16);
        if start < u32::from(reg::PROCESS_RAM_START) || start + len > reg::PROCESS_RAM_END || len == 0 {
            self.errors.push(format!("process RAM command for {len} bytes at 0x{start:04X}"));
            return None;
        }
        Some((start, start + len))
    }

    fn start_read(&mut self) {
        let Some((start, end)) = self.pram_range(self.rd_addr_len) else { return };
        let queue = (start & !3..end)
            .step_by(4)
            .map(|a| {
                let a = a as usize;
                u32::from_le_bytes([self.esc[a], self.esc[a + 1], self.esc[a + 2], self.esc[a + 3]])
            })
            .collect();
        self.accesses.push(EscAccess {
            write: false,
            address: start as u16,
            len: (end - start) as u16,
            via_fifo: true,
        });
        self.rd = Some(ReadOp { started_ns: self.now_ns, queue });
    }

    fn start_write(&mut self) {
        let Some((start, end)) = self.pram_range(self.wr_addr_len) else { return };
        self.accesses.push(EscAccess { write: true, address: start as u16, len: (end - start) as u16, via_fifo: true });
        self.wr = Some(WriteOp { start, end, pos: start & !3 });
    }

    fn push_write_fifo(&mut self, v: u32) {
        let Some(op) = self.wr.as_mut() else {
            self.errors.push("write FIFO written with no write command".into());
            return;
        };
        for (k, b) in v.to_le_bytes().into_iter().enumerate() {
            let a = op.pos + k as u32;
            if a >= op.start && a < op.end {
                self.esc[a as usize] = b;
            }
        }
        op.pos += 4;
        if op.pos >= op.end {
            self.wr = None;
        }
    }

    fn csr_command(&mut self, v: u32) {
        if self.now_ns < self.csr_done_ns {
            self.errors.push("ECAT_CSR_CMD written while busy".into());
        }
        self.csr_cmd = v;
        if v & reg::CSR_BUSY == 0 {
            return;
        }
        let size = (v >> reg::CSR_SIZE_SHIFT) & 0x7;
        let address = v & 0xFFFF;
        let aligned = match size {
            1 => true,
            2 => address.is_multiple_of(2),
            4 => address.is_multiple_of(4),
            _ => false,
        };
        if !aligned || address + size > reg::PROCESS_RAM_END {
            self.errors.push(format!("CSR access of size {size} at 0x{address:04X}"));
            return;
        }
        let (a, n) = (address as usize, size as usize);
        let write = v & reg::CSR_READ == 0;
        if write {
            self.esc[a..a + n].copy_from_slice(&self.csr_data.to_le_bytes()[..n]);
            if address == u32::from(crate::mii::mii_reg::CONTROL) {
                self.mii_command();
            }
        } else {
            let mut d = [0u8; 4];
            d[..n].copy_from_slice(&self.esc[a..a + n]);
            self.csr_data = u32::from_le_bytes(d);
        }
        self.accesses.push(EscAccess { write, address: address as u16, len: size as u16, via_fifo: false });
        self.csr_done_ns = self.now_ns + self.timing.csr_busy_ns;
    }
}

/// The address step after each DWORD for the mode in address bits 15:14.
fn step(mode: u16) -> i32 {
    match mode {
        0b01 => 4,
        0b10 => -4,
        _ => 0,
    }
}

impl SpiBus for SimLan9252 {
    fn transaction(&mut self, segments: &mut [Segment<'_>]) -> io::Result<()> {
        let bytes: usize = segments
            .iter()
            .map(|s| match s {
                Segment::Write(b) => b.len(),
                Segment::Read(b) => b.len(),
            })
            .sum();
        self.transactions += 1;
        // The transaction's effects happen at its end.
        self.now_ns = self.clock.now_ns() + self.timing.transaction_cost_ns(bytes as u64);
        let mut header: Vec<u8> = Vec::with_capacity(4);
        let mut header_len = 3;
        let (mut address, mut inc) = (0u16, 0i32);
        let mut acc: Vec<u8> = Vec::with_capacity(4);
        let mut fetched: VecDeque<u8> = VecDeque::new();
        for seg in segments.iter_mut() {
            match seg {
                Segment::Write(data) => {
                    for &b in data.iter() {
                        if header.len() < header_len {
                            header.push(b);
                            if header.len() == 1 {
                                header_len = match b {
                                    reg::SPI_READ | reg::SPI_WRITE => 3,
                                    reg::SPI_FAST_READ => 4,
                                    other => {
                                        self.errors.push(format!("unknown SPI instruction 0x{other:02X}"));
                                        return Ok(());
                                    }
                                };
                            }
                            if header.len() == 3 {
                                let field = u16::from_be_bytes([header[1], header[2]]);
                                address = field & reg::ADDRESS_MASK;
                                inc = step(field >> 14);
                            }
                        } else if header[0] == reg::SPI_WRITE {
                            acc.push(b);
                            if acc.len() == 4 {
                                let v = u32::from_le_bytes([acc[0], acc[1], acc[2], acc[3]]);
                                self.write_dword(address & !3, v);
                                address = (i32::from(address) + inc) as u16 & reg::ADDRESS_MASK;
                                acc.clear();
                            }
                        }
                    }
                }
                Segment::Read(out) => {
                    let reading = header.len() == header_len && header[0] != reg::SPI_WRITE;
                    if !reading {
                        self.errors.push("read segment outside a read's data phase".into());
                        out.fill(0xFF);
                        continue;
                    }
                    for o in out.iter_mut() {
                        if fetched.is_empty() {
                            let v = self.read_dword(address & !3);
                            fetched.extend(v.to_le_bytes());
                            address = (i32::from(address) + inc) as u16 & reg::ADDRESS_MASK;
                        }
                        *o = fetched.pop_front().unwrap_or(0);
                    }
                }
            }
        }
        if !acc.is_empty() {
            self.errors.push(format!("write ended {} bytes into a DWORD", acc.len()));
        }
        self.clock.advance(self.now_ns - self.clock.now_ns());
        Ok(())
    }
}
