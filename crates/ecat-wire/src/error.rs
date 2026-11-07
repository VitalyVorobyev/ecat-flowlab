use thiserror::Error;

/// Why a byte sequence is not a well-formed EtherCAT structure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum WireError {
    /// The buffer ended before a structure of `needed` bytes at `offset` was complete.
    #[error("truncated: need {needed} bytes at offset {offset}, have {available}")]
    Truncated {
        /// Byte offset of the structure within the buffer being parsed.
        offset: usize,
        /// Bytes the structure requires.
        needed: usize,
        /// Bytes that were available from `offset`.
        available: usize,
    },
    /// The EtherCAT header announces more bytes than the frame carries.
    #[error("EtherCAT header length {declared} exceeds available payload {available}")]
    LengthMismatch {
        /// Length declared in the header.
        declared: usize,
        /// Payload bytes actually present.
        available: usize,
    },
    /// A value cannot be encoded in its field.
    #[error("{field} value {value} does not fit its field")]
    FieldOverflow {
        /// Field name.
        field: &'static str,
        /// Offending value.
        value: usize,
    },
}

pub(crate) fn need(buf: &[u8], offset: usize, needed: usize) -> Result<(), WireError> {
    let available = buf.len().saturating_sub(offset);
    if available < needed { Err(WireError::Truncated { offset, needed, available }) } else { Ok(()) }
}

pub(crate) fn le16(buf: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([buf[at], buf[at + 1]])
}

pub(crate) fn le32(buf: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([buf[at], buf[at + 1], buf[at + 2], buf[at + 3]])
}
