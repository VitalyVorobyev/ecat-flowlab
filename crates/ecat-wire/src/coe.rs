use crate::error::{WireError, le16, le32, need};

/// CoE service (upper 4 bits of the CoE header).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum CoeService {
    /// `1`: emergency.
    Emergency,
    /// `2`: SDO request.
    SdoRequest,
    /// `3`: SDO response.
    SdoResponse,
    /// `4`: TxPDO.
    TxPdo,
    /// `5`: RxPDO.
    RxPdo,
    /// `6`: TxPDO remote request.
    TxPdoRemoteRequest,
    /// `7`: RxPDO remote request.
    RxPdoRemoteRequest,
    /// `8`: SDO information.
    SdoInformation,
    /// Any other value.
    Reserved(u8),
}

impl CoeService {
    /// Decode the 4-bit service.
    pub const fn from_code(code: u8) -> Self {
        match code {
            1 => Self::Emergency,
            2 => Self::SdoRequest,
            3 => Self::SdoResponse,
            4 => Self::TxPdo,
            5 => Self::RxPdo,
            6 => Self::TxPdoRemoteRequest,
            7 => Self::RxPdoRemoteRequest,
            8 => Self::SdoInformation,
            other => Self::Reserved(other),
        }
    }

    /// The 4-bit service.
    pub const fn code(self) -> u8 {
        match self {
            Self::Emergency => 1,
            Self::SdoRequest => 2,
            Self::SdoResponse => 3,
            Self::TxPdo => 4,
            Self::RxPdo => 5,
            Self::TxPdoRemoteRequest => 6,
            Self::RxPdoRemoteRequest => 7,
            Self::SdoInformation => 8,
            Self::Reserved(c) => c,
        }
    }
}

/// The 2-byte CoE header after the mailbox header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CoeHeader {
    /// PDO number / reserved (9 bits).
    pub number: u16,
    /// Service.
    pub service: CoeService,
}

impl CoeHeader {
    /// Length of the CoE header.
    pub const LEN: usize = 2;

    /// Parse from the start of `buf`.
    pub fn parse(buf: &[u8]) -> Result<Self, WireError> {
        need(buf, 0, Self::LEN)?;
        let w = le16(buf, 0);
        Ok(Self { number: w & 0x01FF, service: CoeService::from_code((w >> 12) as u8) })
    }

    /// Encode to 2 bytes.
    pub fn encode(&self) -> [u8; 2] {
        ((self.number & 0x01FF) | ((self.service.code() as u16) << 12)).to_le_bytes()
    }
}

/// The SDO command byte plus index/subindex and the 4-byte data field of an
/// initiate / expedited / abort SDO frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SdoHeader {
    /// Raw command specifier byte.
    pub command: u8,
    /// Object index.
    pub index: u16,
    /// Object subindex.
    pub subindex: u8,
    /// The 4 data bytes (expedited data, size, or abort code).
    pub data: u32,
}

impl SdoHeader {
    /// Length of the SDO header.
    pub const LEN: usize = 8;
    /// Command specifier of an SDO abort.
    pub const ABORT: u8 = 0x80;

    /// Parse from the start of `buf`.
    pub fn parse(buf: &[u8]) -> Result<Self, WireError> {
        need(buf, 0, Self::LEN)?;
        Ok(Self { command: buf[0], index: le16(buf, 1), subindex: buf[3], data: le32(buf, 4) })
    }

    /// Encode to 8 bytes.
    pub fn encode(&self) -> [u8; 8] {
        let i = self.index.to_le_bytes();
        let d = self.data.to_le_bytes();
        [self.command, i[0], i[1], self.subindex, d[0], d[1], d[2], d[3]]
    }

    /// Client/server command specifier (top 3 bits).
    pub const fn specifier(&self) -> u8 {
        self.command >> 5
    }

    /// Whether this is an SDO abort transfer.
    pub const fn is_abort(&self) -> bool {
        self.command == Self::ABORT
    }
}
