//! The EtherCAT header and the chain of datagrams it carries, parsed without allocation.

use crate::command::{AddressMode, Command};
use crate::error::{WireError, le16, le32, need};

/// Length of the EtherCAT header that precedes the datagrams.
pub const ECAT_HEADER_LEN: usize = 2;
/// Length of a datagram header (command .. IRQ).
pub const DATAGRAM_HEADER_LEN: usize = 10;
/// Length of the working counter that trails each datagram.
pub const WKC_LEN: usize = 2;

/// Type field of the EtherCAT header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum EcatPduType {
    /// `1`: EtherCAT device protocol (datagrams). The only type this crate decodes.
    Datagrams,
    /// `4`: network variables.
    NetworkVariables,
    /// `5`: mailbox gateway.
    MailboxGateway,
    /// Reserved type value.
    Reserved(u8),
}

impl EcatPduType {
    /// Decode the 4-bit type field.
    pub const fn from_code(code: u8) -> Self {
        match code {
            1 => Self::Datagrams,
            4 => Self::NetworkVariables,
            5 => Self::MailboxGateway,
            other => Self::Reserved(other),
        }
    }

    /// The 4-bit type field.
    pub const fn code(self) -> u8 {
        match self {
            Self::Datagrams => 1,
            Self::NetworkVariables => 4,
            Self::MailboxGateway => 5,
            Self::Reserved(c) => c,
        }
    }
}

/// The 2-byte EtherCAT header: 11-bit length, 1 reserved bit, 4-bit type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct EcatHeader {
    /// Number of bytes following the header (all datagrams).
    pub length: u16,
    /// Reserved bit 11.
    pub reserved: bool,
    /// Protocol type.
    pub pdu_type: EcatPduType,
}

impl EcatHeader {
    /// Decode from the raw little-endian word.
    pub const fn from_word(word: u16) -> Self {
        Self {
            length: word & 0x07FF,
            reserved: word & 0x0800 != 0,
            pdu_type: EcatPduType::from_code((word >> 12) as u8),
        }
    }

    /// Encode to the raw word.
    pub const fn to_word(self) -> u16 {
        (self.length & 0x07FF) | ((self.reserved as u16) << 11) | ((self.pdu_type.code() as u16) << 12)
    }
}

/// An EtherCAT PDU: header plus the bytes it covers.
#[derive(Debug, Clone, Copy)]
pub struct EcatPdu<'a> {
    /// Decoded header.
    pub header: EcatHeader,
    /// The `header.length` bytes after the header.
    pub body: &'a [u8],
    /// Bytes after `body` in the Ethernet payload (minimum-frame padding, normally zero).
    pub trailing: &'a [u8],
}

impl<'a> EcatPdu<'a> {
    /// Parse an EtherCAT header and slice its body out of `payload`.
    pub fn parse(payload: &'a [u8]) -> Result<Self, WireError> {
        need(payload, 0, ECAT_HEADER_LEN)?;
        let header = EcatHeader::from_word(le16(payload, 0));
        let rest = &payload[ECAT_HEADER_LEN..];
        let declared = header.length as usize;
        if declared > rest.len() {
            return Err(WireError::LengthMismatch { declared, available: rest.len() });
        }
        Ok(Self { header, body: &rest[..declared], trailing: &rest[declared..] })
    }

    /// Iterate the datagrams. Empty when the PDU type is not [`EcatPduType::Datagrams`].
    pub fn datagrams(&self) -> DatagramIter<'a> {
        let body = if self.header.pdu_type == EcatPduType::Datagrams { self.body } else { &[] };
        DatagramIter { body, offset: 0, done: false }
    }
}

/// A datagram's 32-bit address field, interpreted per its command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(tag = "mode", rename_all = "snake_case"))]
pub enum DatagramAddress {
    /// NOP or unknown command: the raw field.
    Raw {
        /// Raw 32-bit value.
        value: u32,
    },
    /// Auto-increment addressing. `adp` as sent by the master is `-position`.
    Position {
        /// Auto-increment address as on the wire.
        adp: u16,
        /// Physical memory offset.
        ado: u16,
    },
    /// Configured station address.
    Fixed {
        /// Station address.
        station: u16,
        /// Physical memory offset.
        ado: u16,
    },
    /// Broadcast.
    Broadcast {
        /// Auto-increment field (incremented by each slave).
        adp: u16,
        /// Physical memory offset.
        ado: u16,
    },
    /// Logical address.
    Logical {
        /// 32-bit logical address.
        address: u32,
    },
}

impl DatagramAddress {
    /// Interpret a raw address for `command`.
    pub const fn new(command: Command, raw: u32) -> Self {
        let adp = (raw & 0xFFFF) as u16;
        let ado = (raw >> 16) as u16;
        match command.address_mode() {
            AddressMode::None => Self::Raw { value: raw },
            AddressMode::Position => Self::Position { adp, ado },
            AddressMode::Fixed => Self::Fixed { station: adp, ado },
            AddressMode::Broadcast => Self::Broadcast { adp, ado },
            AddressMode::Logical => Self::Logical { address: raw },
        }
    }

    /// The raw 32-bit field.
    pub const fn raw(self) -> u32 {
        match self {
            Self::Raw { value } => value,
            Self::Logical { address } => address,
            Self::Position { adp, ado } | Self::Broadcast { adp, ado } => adp as u32 | ((ado as u32) << 16),
            Self::Fixed { station, ado } => station as u32 | ((ado as u32) << 16),
        }
    }

    /// Physical memory offset (register address) for physical addressing modes.
    pub const fn ado(self) -> Option<u16> {
        match self {
            Self::Position { ado, .. } | Self::Fixed { ado, .. } | Self::Broadcast { ado, .. } => Some(ado),
            Self::Raw { .. } | Self::Logical { .. } => None,
        }
    }

    /// Slave ring position for auto-increment addressing as the master sent it
    /// (`adp = 0` is position 0, `0xFFFF` is position 1, ...).
    pub const fn position(self) -> Option<u16> {
        match self {
            Self::Position { adp, .. } => Some(adp.wrapping_neg()),
            _ => None,
        }
    }
}

/// One EtherCAT datagram, borrowed from the frame.
#[derive(Debug, Clone, Copy)]
pub struct Datagram<'a> {
    /// Command.
    pub command: Command,
    /// Index set by the master to match replies.
    pub index: u8,
    /// Interpreted address.
    pub address: DatagramAddress,
    /// Reserved bits 11..=13 of the length word.
    pub reserved: u8,
    /// Circulating-frame flag (bit 14).
    pub circulating: bool,
    /// More-datagrams-follow flag (bit 15).
    pub more: bool,
    /// Interrupt field.
    pub irq: u16,
    /// Data bytes.
    pub data: &'a [u8],
    /// Working counter.
    pub wkc: u16,
    /// Byte offset of the datagram header within the EtherCAT PDU body.
    pub offset: usize,
}

impl Datagram<'_> {
    /// Bytes this datagram occupies (header + data + WKC).
    pub fn encoded_len(&self) -> usize {
        DATAGRAM_HEADER_LEN + self.data.len() + WKC_LEN
    }
}

/// Iterator over the datagrams of a PDU. Follows the `more` flag; stops after the first error.
#[derive(Debug, Clone)]
pub struct DatagramIter<'a> {
    body: &'a [u8],
    offset: usize,
    done: bool,
}

impl<'a> Iterator for DatagramIter<'a> {
    type Item = Result<Datagram<'a>, WireError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done || self.offset >= self.body.len() {
            return None;
        }
        let b = self.body;
        let at = self.offset;
        if let Err(e) = need(b, at, DATAGRAM_HEADER_LEN) {
            self.done = true;
            return Some(Err(e));
        }
        let command = Command::from_code(b[at]);
        let index = b[at + 1];
        let raw_addr = le32(b, at + 2);
        let len_word = le16(b, at + 6);
        let len = (len_word & 0x07FF) as usize;
        let irq = le16(b, at + 8);
        let data_at = at + DATAGRAM_HEADER_LEN;
        if let Err(e) = need(b, data_at, len + WKC_LEN) {
            self.done = true;
            return Some(Err(e));
        }
        let more = len_word & 0x8000 != 0;
        let dg = Datagram {
            command,
            index,
            address: DatagramAddress::new(command, raw_addr),
            reserved: ((len_word >> 11) & 0x7) as u8,
            circulating: len_word & 0x4000 != 0,
            more,
            irq,
            data: &b[data_at..data_at + len],
            wkc: le16(b, data_at + len),
            offset: at,
        };
        self.offset = data_at + len + WKC_LEN;
        if !more {
            self.done = true;
        }
        Some(Ok(dg))
    }
}
