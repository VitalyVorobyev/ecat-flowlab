//! Ethernet over EtherCAT (ETG.1000.6 §5.7): the 4-byte fragment header.

use crate::error::{WireError, le16, need};

/// EoE frame type (header bits 0–3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum EoeType {
    /// `0`: fragment of a tunnelled Ethernet frame.
    Fragment,
    /// `1`: time stamp response.
    TimestampResponse,
    /// `2`: set IP parameter request.
    SetIpRequest,
    /// `3`: set IP parameter response.
    SetIpResponse,
    /// `4`: set address filter request.
    SetFilterRequest,
    /// `5`: set address filter response.
    SetFilterResponse,
    /// Anything else.
    Reserved(u8),
}

impl EoeType {
    /// Decode.
    pub const fn from_code(c: u8) -> Self {
        match c {
            0 => Self::Fragment,
            1 => Self::TimestampResponse,
            2 => Self::SetIpRequest,
            3 => Self::SetIpResponse,
            4 => Self::SetFilterRequest,
            5 => Self::SetFilterResponse,
            o => Self::Reserved(o),
        }
    }

    /// Encode.
    pub const fn code(self) -> u8 {
        match self {
            Self::Fragment => 0,
            Self::TimestampResponse => 1,
            Self::SetIpRequest => 2,
            Self::SetIpResponse => 3,
            Self::SetFilterRequest => 4,
            Self::SetFilterResponse => 5,
            Self::Reserved(c) => c,
        }
    }

    /// Short name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Fragment => "fragment",
            Self::TimestampResponse => "timestamp response",
            Self::SetIpRequest => "set IP parameter request",
            Self::SetIpResponse => "set IP parameter response",
            Self::SetFilterRequest => "set address filter request",
            Self::SetFilterResponse => "set address filter response",
            Self::Reserved(_) => "reserved",
        }
    }
}

/// The 4-byte EoE header after the mailbox header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct EoeHeader {
    /// Frame type.
    pub frame_type: EoeType,
    /// Port (4 bits).
    pub port: u8,
    /// This is the last fragment of the Ethernet frame.
    pub last_fragment: bool,
    /// A 4-byte time stamp follows the last fragment's data.
    pub time_appended: bool,
    /// The sender requests a time stamp.
    pub time_request: bool,
    /// Fragment number (6 bits), 0 for the first.
    pub fragment: u8,
    /// Fragment 0: complete frame size in 32-byte blocks (rounded up); later fragments:
    /// offset of their data in 32-byte blocks (6 bits).
    pub offset_blocks: u8,
    /// Frame number (4 bits), the same for all fragments of one Ethernet frame.
    pub frame_no: u8,
}

impl EoeHeader {
    /// Length of the EoE header.
    pub const LEN: usize = 4;
    /// Granularity of the offset / size field.
    pub const BLOCK: usize = 32;

    /// Parse from the start of `buf`.
    pub fn parse(buf: &[u8]) -> Result<Self, WireError> {
        need(buf, 0, Self::LEN)?;
        let a = le16(buf, 0);
        let b = le16(buf, 2);
        Ok(Self {
            frame_type: EoeType::from_code((a & 0x0F) as u8),
            port: ((a >> 4) & 0x0F) as u8,
            last_fragment: a & 0x0100 != 0,
            time_appended: a & 0x0200 != 0,
            time_request: a & 0x0400 != 0,
            fragment: (b & 0x3F) as u8,
            offset_blocks: ((b >> 6) & 0x3F) as u8,
            frame_no: ((b >> 12) & 0x0F) as u8,
        })
    }

    /// Encode to 4 bytes.
    pub fn encode(&self) -> [u8; 4] {
        let a = (self.frame_type.code() as u16 & 0x0F)
            | (self.port as u16 & 0x0F) << 4
            | (self.last_fragment as u16) << 8
            | (self.time_appended as u16) << 9
            | (self.time_request as u16) << 10;
        let b = (self.fragment as u16 & 0x3F)
            | (self.offset_blocks as u16 & 0x3F) << 6
            | (self.frame_no as u16 & 0x0F) << 12;
        let (a, b) = (a.to_le_bytes(), b.to_le_bytes());
        [a[0], a[1], b[0], b[1]]
    }
}
