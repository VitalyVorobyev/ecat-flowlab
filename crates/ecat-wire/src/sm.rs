//! SyncManager configuration registers (0x0800 + 8·n): start address, length, mode, direction.

use crate::error::{WireError, le16, need};

/// First SyncManager register (`SM0` start address).
pub const SM_REGISTER_BASE: u16 = 0x0800;
/// Bytes per SyncManager register block.
pub const SM_REGISTER_LEN: u16 = 8;
/// Number of SyncManagers an ESC can have.
pub const SM_COUNT: u16 = 16;
/// Offset of the status register in a SyncManager's block.
pub const SM_STATUS_OFFSET: u16 = 5;
/// Offset of the activate register in a SyncManager's block.
pub const SM_ACTIVATE_OFFSET: u16 = 6;
/// Status bit 3: the (mailbox) buffer is full.
pub const SM_STATUS_MAILBOX_FULL: u8 = 0x08;
/// Activate register bit 1: repeat request (the master toggles it).
pub const SM_REPEAT_REQUEST: u8 = 0x02;
/// PDI control register bit 1: repeat acknowledge (the slave toggles it).
pub const SM_REPEAT_ACK: u8 = 0x02;

/// Register address of SyncManager `sm`'s 8-byte block.
pub const fn sm_register(sm: u8) -> u16 {
    SM_REGISTER_BASE + SM_REGISTER_LEN * sm as u16
}

/// Register address of SyncManager `sm`'s status byte.
pub const fn sm_status_register(sm: u8) -> u16 {
    sm_register(sm) + SM_STATUS_OFFSET
}

/// Register address of SyncManager `sm`'s activate byte.
pub const fn sm_activate_register(sm: u8) -> u16 {
    sm_register(sm) + SM_ACTIVATE_OFFSET
}

/// Operation mode of a SyncManager (control bits 1:0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum SmMode {
    /// `00`: 3-buffer mode, used for process data.
    Buffered,
    /// `10`: 1-buffer mailbox mode.
    Mailbox,
    /// `01` / `11`: reserved.
    Reserved,
}

/// Which side writes the SyncManager's buffer (control bits 3:2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum SmDirection {
    /// `00`: EtherCAT reads, PDI writes — slave → master (inputs, "mailbox in").
    EcatRead,
    /// `01`: EtherCAT writes, PDI reads — master → slave (outputs, "mailbox out").
    EcatWrite,
    /// `10` / `11`: reserved.
    Reserved,
}

/// The 8-byte register block of one SyncManager (`0x0800 + 8·n`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SyncManagerConfig {
    /// Physical start address in ESC memory.
    pub start: u16,
    /// Length in bytes.
    pub length: u16,
    /// Control register.
    pub control: u8,
    /// Status register (read-only on the ESC; whatever the datagram carried).
    pub status: u8,
    /// Activate register (bit 0: enable).
    pub activate: u8,
    /// PDI control register.
    pub pdi_control: u8,
}

impl SyncManagerConfig {
    /// Length of one SyncManager register block.
    pub const LEN: usize = SM_REGISTER_LEN as usize;

    /// Parse from the start of `buf`.
    pub fn parse(buf: &[u8]) -> Result<Self, WireError> {
        need(buf, 0, Self::LEN)?;
        Ok(Self {
            start: le16(buf, 0),
            length: le16(buf, 2),
            control: buf[4],
            status: buf[5],
            activate: buf[6],
            pdi_control: buf[7],
        })
    }

    /// Encode to 8 bytes.
    pub fn encode(&self) -> [u8; 8] {
        let s = self.start.to_le_bytes();
        let l = self.length.to_le_bytes();
        [s[0], s[1], l[0], l[1], self.control, self.status, self.activate, self.pdi_control]
    }

    /// Operation mode.
    pub const fn mode(&self) -> SmMode {
        match self.control & 0x03 {
            0 => SmMode::Buffered,
            2 => SmMode::Mailbox,
            _ => SmMode::Reserved,
        }
    }

    /// Direction.
    pub const fn direction(&self) -> SmDirection {
        match (self.control >> 2) & 0x03 {
            0 => SmDirection::EcatRead,
            1 => SmDirection::EcatWrite,
            _ => SmDirection::Reserved,
        }
    }

    /// Activate bit 0.
    pub const fn enabled(&self) -> bool {
        self.activate & 0x01 != 0
    }

    /// Status bit 3: the (mailbox) buffer is full.
    pub const fn mailbox_full(&self) -> bool {
        self.status & SM_STATUS_MAILBOX_FULL != 0
    }
}

/// The SyncManager register blocks fully contained in a write of `data` at physical
/// offset `ado`, as `(sm_number, config)`.
pub fn sync_managers_in(ado: u16, data: &[u8]) -> impl Iterator<Item = (u8, SyncManagerConfig)> + '_ {
    let end = ado as u32 + data.len() as u32;
    let base = SM_REGISTER_BASE as u32;
    (0..SM_COUNT as u32).filter_map(move |n| {
        let at = base + n * SM_REGISTER_LEN as u32;
        if at < ado as u32 || at + SM_REGISTER_LEN as u32 > end {
            return None;
        }
        let off = (at - ado as u32) as usize;
        SyncManagerConfig::parse(&data[off..]).ok().map(|c| (n as u8, c))
    })
}
