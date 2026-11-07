//! SDO command specifiers, abort codes, CoE emergency and SDO Information headers
//! (CiA 301 as profiled by ETG.1000.6 for CoE).

use crate::error::{WireError, le16, need};

/// The SDO command byte, interpreted.
///
/// Client (request) and server (response) command specifiers share codes, so decoding needs
/// to know which side sent the byte — the CoE service says so.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case", tag = "kind"))]
pub enum SdoCommand {
    /// ccs 1. `expedited` + `size_indicated`: data is in the 4-byte field, `4 − unused` bytes.
    /// Not expedited: the 4-byte field is the complete size and data follows it.
    InitiateDownloadRequest {
        /// ETG complete-access bit (whole object, subindex 0 or 1 onwards).
        complete_access: bool,
        /// `e`.
        expedited: bool,
        /// `s`.
        size_indicated: bool,
        /// `n`: unused bytes of the expedited data field.
        unused: u8,
    },
    /// ccs 0.
    DownloadSegmentRequest {
        /// `t`.
        toggle: bool,
        /// `n`: unused bytes of a 7-byte segment.
        unused: u8,
        /// `c`: no more segments follow.
        last: bool,
    },
    /// ccs 2.
    InitiateUploadRequest {
        /// ETG complete-access bit.
        complete_access: bool,
    },
    /// ccs 3.
    UploadSegmentRequest {
        /// `t`.
        toggle: bool,
    },
    /// scs 3.
    InitiateDownloadResponse,
    /// scs 1.
    DownloadSegmentResponse {
        /// `t`.
        toggle: bool,
    },
    /// scs 2. Same bit layout as [`SdoCommand::InitiateDownloadRequest`].
    InitiateUploadResponse {
        /// ETG complete-access bit.
        complete_access: bool,
        /// `e`.
        expedited: bool,
        /// `s`.
        size_indicated: bool,
        /// `n`.
        unused: u8,
    },
    /// scs 0.
    UploadSegmentResponse {
        /// `t`.
        toggle: bool,
        /// `n`.
        unused: u8,
        /// `c`.
        last: bool,
    },
    /// cs 4: abort transfer (either side).
    Abort,
    /// A specifier this decoder does not know (5–7, e.g. block transfer, unused in CoE).
    Unknown {
        /// Sent by the client.
        request: bool,
        /// The raw byte.
        byte: u8,
    },
}

impl SdoCommand {
    /// Decode a command byte sent by the client (`request`) or the server.
    pub const fn decode(byte: u8, request: bool) -> Self {
        const fn bit_of(byte: u8, n: u8) -> bool {
            byte & (1 << n) != 0
        }
        let cs = byte >> 5;
        match (request, cs) {
            (_, 4) => Self::Abort,
            (true, 1) => Self::InitiateDownloadRequest {
                complete_access: bit_of(byte, 4),
                expedited: bit_of(byte, 1),
                size_indicated: bit_of(byte, 0),
                unused: (byte >> 2) & 0x03,
            },
            (true, 0) => Self::DownloadSegmentRequest {
                toggle: bit_of(byte, 4),
                unused: (byte >> 1) & 0x07,
                last: bit_of(byte, 0),
            },
            (true, 2) => Self::InitiateUploadRequest { complete_access: bit_of(byte, 4) },
            (true, 3) => Self::UploadSegmentRequest { toggle: bit_of(byte, 4) },
            (false, 3) => Self::InitiateDownloadResponse,
            (false, 1) => Self::DownloadSegmentResponse { toggle: bit_of(byte, 4) },
            (false, 2) => Self::InitiateUploadResponse {
                complete_access: bit_of(byte, 4),
                expedited: bit_of(byte, 1),
                size_indicated: bit_of(byte, 0),
                unused: (byte >> 2) & 0x03,
            },
            (false, 0) => Self::UploadSegmentResponse {
                toggle: bit_of(byte, 4),
                unused: (byte >> 1) & 0x07,
                last: bit_of(byte, 0),
            },
            _ => Self::Unknown { request, byte },
        }
    }

    /// Encode to the command byte.
    pub const fn encode(self) -> u8 {
        const fn b(v: bool, n: u8) -> u8 {
            (v as u8) << n
        }
        match self {
            Self::InitiateDownloadRequest { complete_access, expedited, size_indicated, unused } => {
                1 << 5 | b(complete_access, 4) | (unused & 3) << 2 | b(expedited, 1) | b(size_indicated, 0)
            }
            Self::DownloadSegmentRequest { toggle, unused, last } => b(toggle, 4) | (unused & 7) << 1 | b(last, 0),
            Self::InitiateUploadRequest { complete_access } => 2 << 5 | b(complete_access, 4),
            Self::UploadSegmentRequest { toggle } => 3 << 5 | b(toggle, 4),
            Self::InitiateDownloadResponse => 3 << 5,
            Self::DownloadSegmentResponse { toggle } => 1 << 5 | b(toggle, 4),
            Self::InitiateUploadResponse { complete_access, expedited, size_indicated, unused } => {
                2 << 5 | b(complete_access, 4) | (unused & 3) << 2 | b(expedited, 1) | b(size_indicated, 0)
            }
            Self::UploadSegmentResponse { toggle, unused, last } => b(toggle, 4) | (unused & 7) << 1 | b(last, 0),
            Self::Abort => 0x80,
            Self::Unknown { byte, .. } => byte,
        }
    }

    /// Whether the byte opens a transfer (initiate request).
    pub const fn is_initiate_request(self) -> bool {
        matches!(self, Self::InitiateDownloadRequest { .. } | Self::InitiateUploadRequest { .. })
    }

    /// Compact name for one-line summaries: `"upload req"`, `"download seg resp"`.
    pub const fn short_name(self) -> &'static str {
        match self {
            Self::InitiateDownloadRequest { .. } => "download req",
            Self::DownloadSegmentRequest { .. } => "download seg req",
            Self::InitiateUploadRequest { .. } => "upload req",
            Self::UploadSegmentRequest { .. } => "upload seg req",
            Self::InitiateDownloadResponse => "download resp",
            Self::DownloadSegmentResponse { .. } => "download seg resp",
            Self::InitiateUploadResponse { .. } => "upload resp",
            Self::UploadSegmentResponse { .. } => "upload seg resp",
            Self::Abort => "abort",
            Self::Unknown { .. } => "unknown",
        }
    }

    /// Full name, e.g. "initiate download request" ([`Self::short_name`] is the compact one).
    pub const fn name(self) -> &'static str {
        match self {
            Self::InitiateDownloadRequest { .. } => "initiate download request",
            Self::DownloadSegmentRequest { .. } => "download segment request",
            Self::InitiateUploadRequest { .. } => "initiate upload request",
            Self::UploadSegmentRequest { .. } => "upload segment request",
            Self::InitiateDownloadResponse => "initiate download response",
            Self::DownloadSegmentResponse { .. } => "download segment response",
            Self::InitiateUploadResponse { .. } => "initiate upload response",
            Self::UploadSegmentResponse { .. } => "upload segment response",
            Self::Abort => "abort",
            Self::Unknown { .. } => "unknown specifier",
        }
    }
}

/// Bytes after the CoE header of a segment frame that precede the segment data (the command).
pub const SDO_SEGMENT_HEADER_LEN: usize = 1;

/// Data bytes in a segment frame whose mailbox `length` (CoE header onward) is given.
/// Segments of up to 7 data bytes are padded to 7 and declare the unused count.
pub const fn segment_data_len(mailbox_length: u16, unused: u8) -> usize {
    let room = (mailbox_length as usize).saturating_sub(2 + SDO_SEGMENT_HEADER_LEN);
    let short = 7usize.saturating_sub(unused as usize);
    if room > 7 {
        room
    } else if short < room {
        short
    } else {
        room
    }
}

/// Text of a CiA 301 / ETG.1000.6 SDO abort code.
pub const fn sdo_abort_text(code: u32) -> Option<&'static str> {
    Some(match code {
        0x0503_0000 => "Toggle bit not changed",
        0x0504_0000 => "SDO protocol timeout",
        0x0504_0001 => "Client/server command specifier not valid or unknown",
        0x0504_0005 => "Out of memory",
        0x0601_0000 => "Unsupported access to an object",
        0x0601_0001 => "Attempt to read a write-only object",
        0x0601_0002 => "Attempt to write a read-only object",
        0x0601_0003 => "Subindex cannot be written, SI0 must be 0 for write access",
        0x0601_0004 => "Complete access not supported for objects of variable length",
        0x0601_0005 => "Object length exceeds mailbox size",
        0x0601_0006 => "Object mapped to RxPDO, SDO download blocked",
        0x0602_0000 => "Object does not exist in the object dictionary",
        0x0604_0041 => "Object cannot be mapped into the PDO",
        0x0604_0042 => "Number and length of mapped objects would exceed the PDO length",
        0x0604_0043 => "General parameter incompatibility",
        0x0604_0047 => "General internal incompatibility in the device",
        0x0606_0000 => "Access failed due to a hardware error",
        0x0607_0010 => "Data type does not match, length of service parameter does not match",
        0x0607_0012 => "Data type does not match, length of service parameter too high",
        0x0607_0013 => "Data type does not match, length of service parameter too low",
        0x0609_0011 => "Subindex does not exist",
        0x0609_0030 => "Value range of parameter exceeded",
        0x0609_0031 => "Value of parameter written too high",
        0x0609_0032 => "Value of parameter written too low",
        0x0609_0036 => "Maximum value is less than minimum value",
        0x0800_0000 => "General error",
        0x0800_0020 => "Data cannot be transferred or stored to the application",
        0x0800_0021 => "Data cannot be transferred or stored because of local control",
        0x0800_0022 => "Data cannot be transferred or stored in the present device state",
        0x0800_0023 => "Object dictionary dynamic generation failed or no object dictionary present",
        _ => return None,
    })
}

/// A CoE emergency message (service 1), after the CoE header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CoeEmergency {
    /// CiA 301 error code.
    pub error_code: u16,
    /// Error register (object 0x1001).
    pub error_register: u8,
    /// Manufacturer-specific data.
    pub data: [u8; 5],
}

impl CoeEmergency {
    /// Length after the CoE header.
    pub const LEN: usize = 8;

    /// Parse from the start of `buf`.
    pub fn parse(buf: &[u8]) -> Result<Self, WireError> {
        need(buf, 0, Self::LEN)?;
        Ok(Self { error_code: le16(buf, 0), error_register: buf[2], data: [buf[3], buf[4], buf[5], buf[6], buf[7]] })
    }

    /// Encode to 8 bytes.
    pub fn encode(&self) -> [u8; 8] {
        let c = self.error_code.to_le_bytes();
        let d = self.data;
        [c[0], c[1], self.error_register, d[0], d[1], d[2], d[3], d[4]]
    }
}

/// The header of an SDO Information message (CoE service 8), after the CoE header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SdoInfoHeader {
    /// Op code (7 bits): 1 get OD list req, 2 resp, 3 get object description req, 4 resp,
    /// 5 get entry description req, 6 resp, 7 SDO Information error.
    pub opcode: u8,
    /// More fragments of this response follow.
    pub incomplete: bool,
    /// Fragments still to come.
    pub fragments_left: u16,
}

impl SdoInfoHeader {
    /// Length after the CoE header.
    pub const LEN: usize = 4;

    /// Parse from the start of `buf`.
    pub fn parse(buf: &[u8]) -> Result<Self, WireError> {
        need(buf, 0, Self::LEN)?;
        Ok(Self { opcode: buf[0] & 0x7F, incomplete: buf[0] & 0x80 != 0, fragments_left: le16(buf, 2) })
    }

    /// Encode to 4 bytes.
    pub fn encode(&self) -> [u8; 4] {
        let f = self.fragments_left.to_le_bytes();
        [(self.opcode & 0x7F) | ((self.incomplete as u8) << 7), 0, f[0], f[1]]
    }

    /// Whether the op code is a request (odd codes 1, 3, 5).
    pub const fn is_request(&self) -> bool {
        matches!(self.opcode, 1 | 3 | 5)
    }

    /// Name of the op code.
    pub const fn opcode_name(&self) -> &'static str {
        match self.opcode {
            1 => "get OD list request",
            2 => "get OD list response",
            3 => "get object description request",
            4 => "get object description response",
            5 => "get entry description request",
            6 => "get entry description response",
            7 => "SDO information error",
            _ => "unknown",
        }
    }
}
