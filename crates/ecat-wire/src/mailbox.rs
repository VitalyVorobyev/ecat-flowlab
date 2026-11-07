use crate::error::{WireError, le16, need};

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
