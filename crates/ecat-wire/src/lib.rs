//! Zero-copy EtherCAT codec.
//!
//! Parses and encodes the EtherCAT *wire* layer as defined by IEC 61158 / ETG.1000:
//!
//! * Ethernet II framing with EtherType `0x88A4`, optionally behind one 802.1Q tag;
//! * EtherCAT over UDP/IPv4, destination or source port `0x88A4` (34980);
//! * the 2-byte EtherCAT header and the chain of datagrams (DLPDUs);
//! * the 6-byte mailbox header and the CoE / SDO headers carried inside it;
//! * names for the common ESC registers and AL states.
//!
//! Parsing never allocates and never panics on malformed input; every accessor returns
//! borrowed slices of the input buffer. Encoding ([`FrameBuilder`]) produces exactly the
//! bytes the parser accepts, which the property tests check.
//!
//! This crate does no I/O and knows nothing about captures, time or analysis.

mod al;
mod coe;
mod command;
mod datagram;
mod encode;
mod error;
mod ethernet;
mod mailbox;
mod registers;

pub use al::{AlState, AlStatus};
pub use coe::{CoeHeader, CoeService, SdoHeader};
pub use command::{AddressMode, Command};
pub use datagram::{
    DATAGRAM_HEADER_LEN, Datagram, DatagramAddress, DatagramIter, ECAT_HEADER_LEN, EcatHeader, EcatPdu, EcatPduType,
    WKC_LEN,
};
pub use encode::{DatagramSpec, FrameBuilder};
pub use error::WireError;
pub use ethernet::{
    ETHERTYPE_ETHERCAT, ETHERTYPE_IPV4, ETHERTYPE_VLAN, Encapsulation, EthernetHeader, MacAddr, ParsedFrame,
    UDP_PORT_ETHERCAT, parse_frame,
};
pub use mailbox::{MAILBOX_HEADER_LEN, MailboxHeader, MailboxType};
pub use registers::{RegisterInfo, register_info};

/// Bytes that occupy the wire for one Ethernet frame *beyond* the captured bytes:
/// preamble + SFD (8), FCS (4) and the minimum inter-frame gap (12).
pub const WIRE_OVERHEAD_BYTES: u32 = 8 + 4 + 12;
/// Minimum Ethernet frame length without FCS; shorter frames are padded on the wire.
pub const MIN_FRAME_LEN_NO_FCS: u32 = 60;

/// Number of bytes a frame of `captured_len` bytes (without FCS) occupies on the wire,
/// including padding to the 64-byte minimum, preamble, FCS and inter-frame gap.
pub fn wire_bytes(captured_len: u32) -> u32 {
    captured_len.max(MIN_FRAME_LEN_NO_FCS) + WIRE_OVERHEAD_BYTES
}

#[cfg(test)]
mod tests;
