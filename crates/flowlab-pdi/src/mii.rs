//! The LAN9252's Ethernet PHYs, through the ESC's MII management interface (0x0510–0x0517).
//!
//! The PHYs set the EtherCAT ports' link: speed, duplex, auto-negotiation and MDI/MDI-X.
//! Firmware and slave stacks sometimes force them (for example 100 Mbit/s full duplex without
//! auto-negotiation and a fixed MDI-X), and a master's port that expects auto-negotiation then
//! links badly or not at all. [`PhyAccess`] reads every PHY register and decodes the link;
//! [`PhyAccess::write`] and [`PhyAccess::enable_autoneg`] change them.
//!
//! An access takes the MII interface for the PDI (0x0517 bit 0), writes the PHY address
//! (0x0512) and register (0x0513), and for a write the data (0x0514), then the command (0x0510:
//! 0x0100 read, 0x0200 write); it polls the busy bit, checks the error bits, reads the data and
//! gives the interface back. It refuses while the master claims the interface (0x0516 bit 0).
//! These are the only ESC registers written; AL Control and the SyncManagers are never touched.
//!
//! ```
//! use flowlab_pdi::{Lan9252, PhyAccess, SimLan9252, SimTiming};
//!
//! let (sim, _) = SimLan9252::new(SimTiming::default());
//! let mut phys = PhyAccess::open(Lan9252::new(sim), 0x9252).unwrap();
//! let p = phys.status(0).unwrap().unwrap();
//! assert!(p.link_up && p.autoneg_enabled);
//! assert!(phys.status(3).unwrap().is_none()); // no PHY at address 3
//! ```

use crate::PdiError;
use crate::lan9252::{Lan9252, LinkInfo};
use crate::spi::SpiBus;

/// ESC MII management registers.
pub mod mii_reg {
    /// MII Management Control/Status (16 bit).
    pub const CONTROL: u16 = 0x0510;
    /// PHY address.
    pub const PHY_ADDRESS: u16 = 0x0512;
    /// PHY register address.
    pub const PHY_REGISTER: u16 = 0x0513;
    /// PHY data (16 bit).
    pub const PHY_DATA: u16 = 0x0514;
    /// MII Management ECAT Access State: bit 0, the master claims the interface.
    pub const ECAT_ACCESS: u16 = 0x0516;
    /// MII Management PDI Access State: bit 0, the PDI has the interface.
    pub const PDI_ACCESS: u16 = 0x0517;
    /// Command: read.
    pub const CMD_READ: u16 = 0x0100;
    /// Command: write.
    pub const CMD_WRITE: u16 = 0x0200;
    /// Control: the PDI may take the interface.
    pub const PDI_POSSIBLE: u16 = 1 << 1;
    /// Control: read error.
    pub const READ_ERROR: u16 = 1 << 13;
    /// Control: command error.
    pub const COMMAND_ERROR: u16 = 1 << 14;
    /// Control: busy.
    pub const BUSY: u16 = 1 << 15;
}

/// The PHY registers [`PhyAccess::status`] reads: basic control and status, the two ID words,
/// auto-negotiation advertisement and link partner, special modes (18), special
/// control/status indication (27) and PHY special control/status (31).
pub const STATUS_REGISTERS: [u8; 9] = [0, 1, 2, 3, 4, 5, 18, 27, 31];

/// Polls of the busy bit before giving up.
const MII_POLLS: u32 = 1000;

/// One PHY's registers and what they say about its link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhyStatus {
    /// PHY address.
    pub phy: u8,
    /// `(register, value)` for every register of [`STATUS_REGISTERS`].
    pub registers: Vec<(u8, u16)>,
    /// Basic status bit 2.
    pub link_up: bool,
    /// Basic control bit 12.
    pub autoneg_enabled: bool,
    /// Basic status bit 5.
    pub autoneg_done: bool,
    /// The speed the PHY runs at: from register 31 when auto-negotiation is done, else
    /// basic control bit 13.
    pub speed_100: bool,
    /// The duplex, from the same sources (register 31, else basic control bit 8).
    pub full_duplex: bool,
    /// Register 27 bit 15: MDI/MDI-X set by register 27, not by the strap.
    pub mdix_override: bool,
    /// Register 27 bit 14: auto-MDIX.
    pub auto_mdix: bool,
    /// Register 27 bit 13: the crossover state (true: MDI-X).
    pub mdix: bool,
    /// Register 18 bits 7:5, the PHY mode (0b111: all capable, auto-negotiation).
    pub mode: u8,
}

impl PhyStatus {
    /// Decode `registers` of PHY `phy` (missing registers read as 0).
    pub fn decode(phy: u8, registers: Vec<(u8, u16)>) -> Self {
        let r = |n: u8| registers.iter().find(|(k, _)| *k == n).map_or(0, |(_, v)| *v);
        let (control, status, special) = (r(0), r(1), r(31));
        let autoneg_done = status & (1 << 5) != 0;
        let autoneg_enabled = control & (1 << 12) != 0;
        let (speed_100, full_duplex) = if autoneg_enabled && autoneg_done {
            let s = (special >> 2) & 0x7;
            (s & 0b010 != 0, s & 0b100 != 0)
        } else {
            (control & (1 << 13) != 0, control & (1 << 8) != 0)
        };
        let ctl27 = r(27);
        Self {
            phy,
            link_up: status & (1 << 2) != 0,
            autoneg_enabled,
            autoneg_done,
            speed_100,
            full_duplex,
            mdix_override: ctl27 & (1 << 15) != 0,
            auto_mdix: ctl27 & (1 << 14) != 0,
            mdix: ctl27 & (1 << 13) != 0,
            mode: ((r(18) >> 5) & 0x7) as u8,
            registers,
        }
    }

    /// One line: link, speed, duplex, auto-negotiation and crossover.
    pub fn summary(&self) -> String {
        format!(
            "link {}, {} Mbit/s {} duplex, auto-negotiation {}, {}{}, mode {:03b}",
            if self.link_up { "up" } else { "down" },
            if self.speed_100 { 100 } else { 10 },
            if self.full_duplex { "full" } else { "half" },
            match (self.autoneg_enabled, self.autoneg_done) {
                (false, _) => "off",
                (true, true) => "done",
                (true, false) => "on, not done",
            },
            if self.auto_mdix {
                "auto-MDIX"
            } else if self.mdix {
                "MDI-X"
            } else {
                "MDI"
            },
            if self.mdix_override { " (register 27)" } else { "" },
            self.mode
        )
    }
}

/// MII access to the PHYs of a LAN9252: each access takes the MII interface for the PDI, runs
/// one command and gives the interface back; it refuses while the master claims it.
#[derive(Debug)]
pub struct PhyAccess<S> {
    esc: Lan9252<S>,
    link: LinkInfo,
}

impl<S: SpiBus> PhyAccess<S> {
    /// Check the SPI link (`BYTE_TEST`, `HW_CFG`, `ID_REV` with `chip_id`) and that the PDI may
    /// take the MII interface.
    pub fn open(esc: Lan9252<S>, chip_id: u16) -> Result<Self, PdiError> {
        let mut esc = esc.with_al_event_read(false);
        let link = esc.check_link(chip_id)?;
        let mut me = Self { esc, link };
        let control = me.read16(mii_reg::CONTROL)?;
        if control & mii_reg::PDI_POSSIBLE == 0 {
            return Err(PdiError::Refused(format!(
                "MII Management Control (0x0510) reads 0x{control:04X}: the ESC does not let the PDI take the MII interface"
            )));
        }
        Ok(me)
    }

    /// What the link check read.
    pub fn link(&self) -> LinkInfo {
        self.link
    }

    /// Read register `register` (0–31) of PHY `phy` (0–31).
    pub fn read(&mut self, phy: u8, register: u8) -> Result<u16, PdiError> {
        self.command(phy, register, None)
    }

    /// Write `value` to register `register` of PHY `phy`.
    pub fn write(&mut self, phy: u8, register: u8, value: u16) -> Result<(), PdiError> {
        self.command(phy, register, Some(value)).map(|_| ())
    }

    /// The registers of [`STATUS_REGISTERS`] of PHY `phy`, decoded; `None` if no PHY answers
    /// there (ID word 0x0000 or 0xFFFF).
    pub fn status(&mut self, phy: u8) -> Result<Option<PhyStatus>, PdiError> {
        let id = self.read(phy, 2)?;
        if id == 0 || id == 0xFFFF {
            return Ok(None);
        }
        let mut regs = Vec::with_capacity(STATUS_REGISTERS.len());
        for r in STATUS_REGISTERS {
            regs.push((r, self.read(phy, r)?));
        }
        Ok(Some(PhyStatus::decode(phy, regs)))
    }

    /// Auto-negotiation and auto-MDIX on PHY `phy`: register 27 bits 15 and 14 (auto-MDIX set
    /// by the register), register 18 mode 0b111 (all capable) with its PHY address kept, then
    /// basic control 0x3300 (100 Mbit/s, full duplex, auto-negotiation on and restarted).
    pub fn enable_autoneg(&mut self, phy: u8) -> Result<(), PdiError> {
        let ctl27 = self.read(phy, 27)?;
        self.write(phy, 27, ctl27 | 0xC000)?;
        let modes = self.read(phy, 18)?;
        self.write(phy, 18, (modes & !0x00E0) | 0x00E0)?;
        self.write(phy, 0, 0x3300)
    }

    /// The LAN9252 back.
    pub fn into_inner(self) -> Lan9252<S> {
        self.esc
    }

    fn command(&mut self, phy: u8, register: u8, value: Option<u16>) -> Result<u16, PdiError> {
        if phy > 31 || register > 31 {
            return Err(PdiError::Refused(format!("PHY {phy} register {register}: both are 0–31")));
        }
        if self.read8(mii_reg::ECAT_ACCESS)? & 1 != 0 {
            return Err(PdiError::Refused("the master claims the MII interface (0x0516 bit 0)".into()));
        }
        self.esc.csr_write(mii_reg::PDI_ACCESS, &[1])?;
        let result = self.command_with_access(phy, register, value);
        let released = self.esc.csr_write(mii_reg::PDI_ACCESS, &[0]);
        let v = result?;
        released?;
        Ok(v)
    }

    fn command_with_access(&mut self, phy: u8, register: u8, value: Option<u16>) -> Result<u16, PdiError> {
        self.wait_idle()?;
        self.esc.csr_write(mii_reg::PHY_ADDRESS, &[phy])?;
        self.esc.csr_write(mii_reg::PHY_REGISTER, &[register])?;
        let cmd = match value {
            Some(v) => {
                self.esc.csr_write(mii_reg::PHY_DATA, &v.to_le_bytes())?;
                mii_reg::CMD_WRITE
            }
            None => mii_reg::CMD_READ,
        };
        self.esc.csr_write(mii_reg::CONTROL, &cmd.to_le_bytes())?;
        let control = self.wait_idle()?;
        if control & (mii_reg::READ_ERROR | mii_reg::COMMAND_ERROR) != 0 {
            return Err(PdiError::Link(format!(
                "MII command 0x{cmd:04X} on PHY {phy} register {register}: 0x0510 reads 0x{control:04X} (error)"
            )));
        }
        match value {
            Some(v) => Ok(v),
            None => self.read16(mii_reg::PHY_DATA),
        }
    }

    fn wait_idle(&mut self) -> Result<u16, PdiError> {
        for _ in 0..MII_POLLS {
            let c = self.read16(mii_reg::CONTROL)?;
            if c & mii_reg::BUSY == 0 {
                return Ok(c);
            }
        }
        Err(PdiError::Timeout { what: "MII Management busy to clear", polls: MII_POLLS })
    }

    fn read8(&mut self, address: u16) -> Result<u8, PdiError> {
        let mut b = [0u8; 1];
        self.esc.csr_read(address, &mut b)?;
        Ok(b[0])
    }

    fn read16(&mut self, address: u16) -> Result<u16, PdiError> {
        let mut b = [0u8; 2];
        self.esc.csr_read(address, &mut b)?;
        Ok(u16::from_le_bytes(b))
    }
}
