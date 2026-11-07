//! The mailbox header, the protocol type it announces, VoE headers and mailbox error replies.

use crate::error::{WireError, le16, le32, need};

/// Length of the mailbox header.
pub const MAILBOX_HEADER_LEN: usize = 6;

/// Mailbox protocol type (4-bit field).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum MailboxType {
    /// `0`: mailbox error reply.
    Error,
    /// `1`: ADS over EtherCAT.
    Aoe,
    /// `2`: Ethernet over EtherCAT.
    Eoe,
    /// `3`: CANopen over EtherCAT.
    Coe,
    /// `4`: File access over EtherCAT.
    Foe,
    /// `5`: Servo drive profile over EtherCAT.
    Soe,
    /// `15`: Vendor-specific over EtherCAT.
    Voe,
    /// Any other value.
    Reserved(u8),
}

impl MailboxType {
    /// Decode the 4-bit type.
    pub const fn from_code(code: u8) -> Self {
        match code {
            0 => Self::Error,
            1 => Self::Aoe,
            2 => Self::Eoe,
            3 => Self::Coe,
            4 => Self::Foe,
            5 => Self::Soe,
            15 => Self::Voe,
            other => Self::Reserved(other),
        }
    }

    /// The 4-bit type.
    pub const fn code(self) -> u8 {
        match self {
            Self::Error => 0,
            Self::Aoe => 1,
            Self::Eoe => 2,
            Self::Coe => 3,
            Self::Foe => 4,
            Self::Soe => 5,
            Self::Voe => 15,
            Self::Reserved(c) => c,
        }
    }
}

/// The 6-byte mailbox header that starts every mailbox SyncManager buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MailboxHeader {
    /// Length of the mailbox service data following the header.
    pub length: u16,
    /// Station address of the originator (or 0).
    pub address: u16,
    /// Channel (6 bits).
    pub channel: u8,
    /// Priority (2 bits).
    pub priority: u8,
    /// Protocol type.
    pub mailbox_type: MailboxType,
    /// Counter (3 bits) used to detect repeated mailbox messages; 0 is reserved.
    pub counter: u8,
}

impl MailboxHeader {
    /// Parse the header at the start of `buf`.
    pub fn parse(buf: &[u8]) -> Result<Self, WireError> {
        need(buf, 0, MAILBOX_HEADER_LEN)?;
        Ok(Self {
            length: le16(buf, 0),
            address: le16(buf, 2),
            channel: buf[4] & 0x3F,
            priority: buf[4] >> 6,
            mailbox_type: MailboxType::from_code(buf[5] & 0x0F),
            counter: (buf[5] >> 4) & 0x07,
        })
    }

    /// Encode to 6 bytes.
    pub fn encode(&self) -> [u8; MAILBOX_HEADER_LEN] {
        let l = self.length.to_le_bytes();
        let a = self.address.to_le_bytes();
        [
            l[0],
            l[1],
            a[0],
            a[1],
            (self.channel & 0x3F) | (self.priority << 6),
            (self.mailbox_type.code() & 0x0F) | ((self.counter & 0x07) << 4),
        ]
    }
}

/// The 6-byte VoE header after the mailbox header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct VoeHeader {
    /// Vendor id (ETG-assigned).
    pub vendor_id: u32,
    /// Vendor-specific type.
    pub vendor_type: u16,
}

impl VoeHeader {
    /// Length of the VoE header.
    pub const LEN: usize = 6;

    /// Parse from the start of `buf`.
    pub fn parse(buf: &[u8]) -> Result<Self, WireError> {
        need(buf, 0, Self::LEN)?;
        Ok(Self { vendor_id: le32(buf, 0), vendor_type: le16(buf, 4) })
    }

    /// Encode to 6 bytes.
    pub fn encode(&self) -> [u8; 6] {
        let v = self.vendor_id.to_le_bytes();
        let t = self.vendor_type.to_le_bytes();
        [v[0], v[1], v[2], v[3], t[0], t[1]]
    }
}

/// A mailbox error reply (mailbox type 0): the slave rejected a mailbox message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MailboxErrorReply {
    /// Service type, `0x0001` for a mailbox command error.
    pub service: u16,
    /// Error detail code.
    pub detail: u16,
}

impl MailboxErrorReply {
    /// Length after the mailbox header.
    pub const LEN: usize = 4;

    /// Parse from the start of `buf`.
    pub fn parse(buf: &[u8]) -> Result<Self, WireError> {
        need(buf, 0, Self::LEN)?;
        Ok(Self { service: le16(buf, 0), detail: le16(buf, 2) })
    }

    /// Encode to 4 bytes.
    pub fn encode(&self) -> [u8; 4] {
        let s = self.service.to_le_bytes();
        let d = self.detail.to_le_bytes();
        [s[0], s[1], d[0], d[1]]
    }

    /// Text of the detail code.
    pub const fn detail_text(&self) -> &'static str {
        match self.detail {
            0x0001 => "Syntax of the 6-byte mailbox header is wrong",
            0x0002 => "Mailbox protocol not supported",
            0x0003 => "Channel field contains a wrong value",
            0x0004 => "Service in the mailbox protocol not supported",
            0x0005 => "Mailbox protocol header is wrong",
            0x0006 => "Length of received mailbox data is too short",
            0x0007 => "Mailbox protocol cannot be processed: not enough memory",
            0x0008 => "Length of data is inconsistent",
            0x0009 => "Mailbox service already in use",
            _ => "Unknown mailbox error",
        }
    }
}
