//! Servo drive profile over EtherCAT (ETG.1000.6 §5.9): the 4-byte SoE header.

use crate::error::{WireError, le16, need};

/// The 4-byte SoE header after the mailbox header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SoeHeader {
    /// Op code (3 bits): 1 read request, 2 read response, 3 write request, 4 write response,
    /// 5 notification, 6 slave info, 7 reserved.
    pub opcode: u8,
    /// More fragments follow.
    pub incomplete: bool,
    /// The response reports an error.
    pub error: bool,
    /// Drive number (3 bits).
    pub drive: u8,
    /// Element flags: data state, name, attribute, unit, min, max, value, default.
    pub elements: u8,
    /// IDN, or fragments left when `incomplete`.
    pub idn: u16,
}

impl SoeHeader {
    /// Length of the SoE header.
    pub const LEN: usize = 4;

    /// Parse from the start of `buf`.
    pub fn parse(buf: &[u8]) -> Result<Self, WireError> {
        need(buf, 0, Self::LEN)?;
        Ok(Self {
            opcode: buf[0] & 0x07,
            incomplete: buf[0] & 0x08 != 0,
            error: buf[0] & 0x10 != 0,
            drive: buf[0] >> 5,
            elements: buf[1],
            idn: le16(buf, 2),
        })
    }

    /// Encode to 4 bytes.
    pub fn encode(&self) -> [u8; 4] {
        let i = self.idn.to_le_bytes();
        let b0 =
            (self.opcode & 0x07) | (self.incomplete as u8) << 3 | (self.error as u8) << 4 | (self.drive & 0x07) << 5;
        [b0, self.elements, i[0], i[1]]
    }

    /// Name of the op code.
    pub const fn opcode_name(&self) -> &'static str {
        match self.opcode {
            1 => "read request",
            2 => "read response",
            3 => "write request",
            4 => "write response",
            5 => "notification",
            6 => "slave info",
            _ => "reserved",
        }
    }

    /// Whether the op code is a request.
    pub const fn is_request(&self) -> bool {
        matches!(self.opcode, 1 | 3)
    }
}
