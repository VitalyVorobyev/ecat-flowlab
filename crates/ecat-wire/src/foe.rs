//! File access over EtherCAT (ETG.1000.6 §5.8).

use crate::error::{WireError, le32, need};

/// FoE op code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum FoeOpcode {
    /// `1`: read request (file slave → master). Value = password, followed by the file name.
    Read,
    /// `2`: write request (file master → slave). Value = password, followed by the file name.
    Write,
    /// `3`: data packet. Value = packet number, followed by data.
    Data,
    /// `4`: acknowledge. Value = packet number.
    Ack,
    /// `5`: error. Value = error code, followed by text.
    Error,
    /// `6`: busy. Value = done (low 16 bits) and entire (high 16 bits), followed by text.
    Busy,
    /// Anything else.
    Reserved(u8),
}

impl FoeOpcode {
    /// Decode.
    pub const fn from_code(c: u8) -> Self {
        match c {
            1 => Self::Read,
            2 => Self::Write,
            3 => Self::Data,
            4 => Self::Ack,
            5 => Self::Error,
            6 => Self::Busy,
            o => Self::Reserved(o),
        }
    }

    /// Encode.
    pub const fn code(self) -> u8 {
        match self {
            Self::Read => 1,
            Self::Write => 2,
            Self::Data => 3,
            Self::Ack => 4,
            Self::Error => 5,
            Self::Busy => 6,
            Self::Reserved(c) => c,
        }
    }

    /// Short name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Read => "RRQ",
            Self::Write => "WRQ",
            Self::Data => "DATA",
            Self::Ack => "ACK",
            Self::Error => "ERR",
            Self::Busy => "BUSY",
            Self::Reserved(_) => "reserved",
        }
    }
}

/// The 6-byte FoE header after the mailbox header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct FoeHeader {
    /// Op code.
    pub opcode: FoeOpcode,
    /// Password, packet number, error code, or busy done/entire, depending on the op code.
    pub value: u32,
}

impl FoeHeader {
    /// Length of the FoE header.
    pub const LEN: usize = 6;

    /// Parse from the start of `buf`.
    pub fn parse(buf: &[u8]) -> Result<Self, WireError> {
        need(buf, 0, Self::LEN)?;
        Ok(Self { opcode: FoeOpcode::from_code(buf[0]), value: le32(buf, 2) })
    }

    /// Encode to 6 bytes.
    pub fn encode(&self) -> [u8; 6] {
        let v = self.value.to_le_bytes();
        [self.opcode.code(), 0, v[0], v[1], v[2], v[3]]
    }

    /// For BUSY: `(done, entire)`.
    pub const fn busy_progress(&self) -> (u16, u16) {
        (self.value as u16, (self.value >> 16) as u16)
    }
}

/// Text of an FoE error code.
pub const fn foe_error_text(code: u32) -> Option<&'static str> {
    Some(match code {
        0x8000 => "Not defined",
        0x8001 => "Not found",
        0x8002 => "Access denied",
        0x8003 => "Disk full",
        0x8004 => "Illegal",
        0x8005 => "Packet number wrong",
        0x8006 => "Already exists",
        0x8007 => "No user",
        0x8008 => "Bootstrap only",
        0x8009 => "Not in bootstrap",
        0x800A => "No rights",
        0x800B => "Program error",
        _ => return None,
    })
}
