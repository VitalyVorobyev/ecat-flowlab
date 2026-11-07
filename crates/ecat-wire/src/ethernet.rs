//! Ethernet II framing, the optional 802.1Q tag, and EtherCAT over UDP/IPv4.

use crate::datagram::EcatPdu;
use crate::error::{WireError, need};

/// EtherType of EtherCAT frames.
pub const ETHERTYPE_ETHERCAT: u16 = 0x88A4;
/// EtherType of IPv4.
pub const ETHERTYPE_IPV4: u16 = 0x0800;
/// EtherType of an 802.1Q VLAN tag.
pub const ETHERTYPE_VLAN: u16 = 0x8100;
/// UDP port registered for EtherCAT (34980).
pub const UDP_PORT_ETHERCAT: u16 = 0x88A4;

const ETH_HEADER_LEN: usize = 14;
/// Minimum Ethernet payload (frames are padded to 60 bytes without FCS).
pub(crate) const MIN_ETH_PAYLOAD: usize = 46;

/// A 48-bit MAC address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct MacAddr(pub [u8; 6]);

impl MacAddr {
    /// Whether the locally-administered bit (bit 1 of the first octet) is set.
    ///
    /// ESCs set this bit in the source address of frames they forward back to the master,
    /// which lets a capture at the master separate outgoing from returning frames.
    pub const fn locally_administered(self) -> bool {
        self.0[0] & 0x02 != 0
    }

    /// The address with the locally-administered bit cleared.
    pub const fn without_local_bit(self) -> Self {
        let mut a = self.0;
        a[0] &= !0x02;
        Self(a)
    }

    /// The address with the locally-administered bit set.
    pub const fn with_local_bit(self) -> Self {
        let mut a = self.0;
        a[0] |= 0x02;
        Self(a)
    }
}

impl std::fmt::Display for MacAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let a = self.0;
        write!(f, "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}", a[0], a[1], a[2], a[3], a[4], a[5])
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for MacAddr {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

/// Ethernet II header, after resolving at most one VLAN tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct EthernetHeader {
    /// Destination address.
    pub dst: MacAddr,
    /// Source address.
    pub src: MacAddr,
    /// VLAN tag control information, when an 802.1Q tag is present.
    pub vlan_tci: Option<u16>,
    /// EtherType after any VLAN tag.
    pub ethertype: u16,
}

/// How the EtherCAT PDU was carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Encapsulation {
    /// Directly in an Ethernet II frame with EtherType `0x88A4`.
    Ethernet,
    /// In a UDP/IPv4 datagram on port `0x88A4`.
    Udp,
}

/// An Ethernet frame that carries an EtherCAT PDU.
#[derive(Debug, Clone, Copy)]
pub struct ParsedFrame<'a> {
    /// Ethernet header.
    pub ethernet: EthernetHeader,
    /// Encapsulation of the PDU.
    pub encapsulation: Encapsulation,
    /// Offset of the EtherCAT header within the frame bytes.
    pub pdu_offset: usize,
    /// The EtherCAT PDU.
    pub pdu: EcatPdu<'a>,
}

/// Parse an Ethernet II frame (no FCS).
///
/// Returns `Ok(None)` when the frame is well-formed Ethernet but does not carry EtherCAT,
/// and an error when it claims to carry EtherCAT but is malformed.
pub fn parse_frame(frame: &[u8]) -> Result<Option<ParsedFrame<'_>>, WireError> {
    need(frame, 0, ETH_HEADER_LEN)?;
    let mut dst = [0u8; 6];
    let mut src = [0u8; 6];
    dst.copy_from_slice(&frame[0..6]);
    src.copy_from_slice(&frame[6..12]);
    let mut ethertype = u16::from_be_bytes([frame[12], frame[13]]);
    let mut at = ETH_HEADER_LEN;
    let mut vlan_tci = None;
    if ethertype == ETHERTYPE_VLAN {
        need(frame, at, 4)?;
        vlan_tci = Some(u16::from_be_bytes([frame[at], frame[at + 1]]));
        ethertype = u16::from_be_bytes([frame[at + 2], frame[at + 3]]);
        at += 4;
    }
    let ethernet = EthernetHeader { dst: MacAddr(dst), src: MacAddr(src), vlan_tci, ethertype };
    let (encapsulation, pdu_offset) = match ethertype {
        ETHERTYPE_ETHERCAT => (Encapsulation::Ethernet, at),
        ETHERTYPE_IPV4 => match udp_ethercat_offset(frame, at)? {
            Some(off) => (Encapsulation::Udp, off),
            None => return Ok(None),
        },
        _ => return Ok(None),
    };
    let pdu = EcatPdu::parse(&frame[pdu_offset..]).map_err(|e| shift(e, pdu_offset))?;
    Ok(Some(ParsedFrame { ethernet, encapsulation, pdu_offset, pdu }))
}

/// Offset of the UDP payload when the IPv4 packet at `at` is UDP to/from port 0x88A4.
fn udp_ethercat_offset(frame: &[u8], at: usize) -> Result<Option<usize>, WireError> {
    if frame.len() < at + 20 {
        return Ok(None);
    }
    let version = frame[at] >> 4;
    let ihl = (frame[at] & 0x0F) as usize * 4;
    let protocol = frame[at + 9];
    if version != 4 || ihl < 20 || protocol != 17 {
        return Ok(None);
    }
    let udp = at + ihl;
    if frame.len() < udp + 8 {
        return Ok(None);
    }
    let sport = u16::from_be_bytes([frame[udp], frame[udp + 1]]);
    let dport = u16::from_be_bytes([frame[udp + 2], frame[udp + 3]]);
    if sport != UDP_PORT_ETHERCAT && dport != UDP_PORT_ETHERCAT {
        return Ok(None);
    }
    Ok(Some(udp + 8))
}

fn shift(e: WireError, by: usize) -> WireError {
    match e {
        WireError::Truncated { offset, needed, available } => {
            WireError::Truncated { offset: offset + by, needed, available }
        }
        other => other,
    }
}
