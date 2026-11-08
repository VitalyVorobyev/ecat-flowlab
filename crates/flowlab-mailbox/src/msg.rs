//! Mailbox messages as both FlowLab's master and its slaves write them: the layer above
//! `ecat-wire`'s headers.
//!
//! The simulator, the lab master, its virtual slave and the capture generator build their
//! messages here, so the same transfer puts the same bytes on the wire in each:
//!
//! * the mailbox buffer: header, data, zero padding to the SyncManager length
//!   ([`mailbox_buffer`]), and the counter that cycles 1–7 ([`next_counter`]);
//! * FoE packets ([`foe`]) and CoE SDO initiate and segment messages ([`sdo`], [`sdo_segment`],
//!   [`sdo_unused`]), with how much payload each carries in a mailbox of a given size;
//! * EoE: an Ethernet II / IPv4 / UDP frame ([`udp_frame`], [`udp_payload`]), its fragments
//!   ([`eoe_fragments`]) and their reassembly ([`EoeReassembler`]).
//!
//! The analysis does not use this module: it reads messages with its own trackers, so that a
//! misreading here cannot hide behind a matching misreading there.

use ecat_wire::{
    CoeHeader, CoeService, EoeHeader, EoeType, FoeHeader, FoeOpcode, MAILBOX_HEADER_LEN, MailboxHeader, MailboxType,
    SdoCommand, SdoHeader,
};
use serde::{Deserialize, Serialize};

/// File name of an FoE request unless another is given.
pub const FOE_FILE_NAME: &str = "flowlab.bin";
/// CoE object of an SDO transfer unless another is given: index.
pub const SDO_INDEX: u16 = 0x2000;
/// CoE object of an SDO transfer unless another is given: subindex.
pub const SDO_SUBINDEX: u8 = 0;
/// Ethernet + IPv4 + UDP header bytes of a tunnelled frame.
pub const EOE_HEADERS: usize = 14 + 20 + 8;
/// Bytes of an SDO segment's data field below which it is padded (and the command's `unused`
/// counts the padding).
const SEGMENT_DATA: usize = 7;

/// The mailbox counter after `c`: 1, 2, …, 7, 1, … (0 is "no counter", and is followed by 1).
pub const fn next_counter(c: u8) -> u8 {
    if c >= 7 { 1 } else { c + 1 }
}

/// The EoE frame number after `n`: 1, 2, …, 15, 1, …
pub const fn next_eoe_frame_no(n: u8) -> u8 {
    n % 15 + 1
}

/// A mailbox buffer of `buffer_len` bytes: the header (station `address`, `kind`, `counter`),
/// `body`, and zeros to the end, so a write reaches the SyncManager's last byte.
pub fn mailbox_buffer(kind: MailboxType, counter: u8, address: u16, body: &[u8], buffer_len: usize) -> Vec<u8> {
    let h = MailboxHeader { length: body.len() as u16, address, channel: 0, priority: 0, mailbox_type: kind, counter };
    let mut v = Vec::with_capacity(buffer_len.max(MAILBOX_HEADER_LEN + body.len()));
    v.extend_from_slice(&h.encode());
    v.extend_from_slice(body);
    if v.len() < buffer_len {
        v.resize(buffer_len, 0);
    }
    v
}

/// An FoE packet: the header (`value` is the password, packet number or error code) and `tail`.
pub fn foe(op: FoeOpcode, value: u32, tail: &[u8]) -> Vec<u8> {
    let mut v = FoeHeader { opcode: op, value }.encode().to_vec();
    v.extend_from_slice(tail);
    v
}

/// FoE data bytes per packet in a mailbox of `mailbox_len` bytes.
pub const fn foe_capacity(mailbox_len: usize) -> usize {
    mailbox_len - MAILBOX_HEADER_LEN - FoeHeader::LEN
}

/// A CoE SDO initiate message (request or response, per `service`) for `index:subindex`, with
/// `data` (the size, or expedited data) and `tail`.
pub fn sdo(service: CoeService, cmd: SdoCommand, index: u16, subindex: u8, data: u32, tail: &[u8]) -> Vec<u8> {
    let mut v = CoeHeader { number: 0, service }.encode().to_vec();
    v.extend_from_slice(&SdoHeader { command: cmd.encode(), index, subindex, data }.encode());
    v.extend_from_slice(tail);
    v
}

/// A CoE SDO segment: the command byte and `data`, padded to seven data bytes.
pub fn sdo_segment(service: CoeService, cmd: SdoCommand, data: &[u8]) -> Vec<u8> {
    let mut v = CoeHeader { number: 0, service }.encode().to_vec();
    v.push(cmd.encode());
    v.extend_from_slice(data);
    if data.len() < SEGMENT_DATA {
        v.resize(CoeHeader::LEN + 1 + SEGMENT_DATA, 0);
    }
    v
}

/// The `unused` field of a segment carrying `n` data bytes.
pub const fn sdo_unused(n: usize) -> u8 {
    if n < SEGMENT_DATA { (SEGMENT_DATA - n) as u8 } else { 0 }
}

/// Payload bytes of an SDO initiate message in a mailbox of `mailbox_len` bytes.
pub const fn sdo_initiate_capacity(mailbox_len: usize) -> usize {
    mailbox_len - MAILBOX_HEADER_LEN - CoeHeader::LEN - SdoHeader::LEN
}

/// Payload bytes of an SDO segment in a mailbox of `mailbox_len` bytes.
pub const fn sdo_segment_capacity(mailbox_len: usize) -> usize {
    mailbox_len - MAILBOX_HEADER_LEN - CoeHeader::LEN - 1
}

/// Frame bytes of an EoE fragment in a mailbox of `mailbox_len` bytes.
pub const fn eoe_capacity(mailbox_len: usize) -> usize {
    mailbox_len - MAILBOX_HEADER_LEN - EoeHeader::LEN
}

/// Addresses and ports of the UDP/IPv4 frames EoE carries. The defaults are documentation
/// addresses (RFC 7042 MACs, RFC 5737 TEST-NET-1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EoeAddresses {
    /// The master's MAC address.
    pub master_mac: [u8; 6],
    /// The slave's MAC address.
    pub slave_mac: [u8; 6],
    /// The master's IPv4 address.
    pub master_ip: [u8; 4],
    /// The slave's IPv4 address.
    pub slave_ip: [u8; 4],
    /// UDP source port of the sender.
    pub source_port: u16,
    /// UDP destination port at the receiver.
    pub destination_port: u16,
}

impl Default for EoeAddresses {
    fn default() -> Self {
        Self {
            master_mac: [0x00, 0x00, 0x5E, 0x00, 0x53, 0x21],
            slave_mac: [0x00, 0x00, 0x5E, 0x00, 0x53, 0x20],
            master_ip: [192, 0, 2, 1],
            slave_ip: [192, 0, 2, 2],
            source_port: 50000,
            destination_port: 50001,
        }
    }
}

/// An Ethernet II frame with an IPv4/UDP datagram carrying `payload` (UDP checksum 0), to
/// (`to_slave`) or from the slave.
pub fn udp_frame(payload: &[u8], a: &EoeAddresses, to_slave: bool) -> Vec<u8> {
    let (dst, src) = if to_slave { (a.slave_mac, a.master_mac) } else { (a.master_mac, a.slave_mac) };
    let (sip, dip) = if to_slave { (a.master_ip, a.slave_ip) } else { (a.slave_ip, a.master_ip) };
    let total = (20 + 8 + payload.len()) as u16;
    let mut ip = vec![0x45, 0, (total >> 8) as u8, total as u8, 0, 0, 0x40, 0, 64, 17, 0, 0];
    ip.extend_from_slice(&sip);
    ip.extend_from_slice(&dip);
    let mut sum = 0u32;
    for w in ip.chunks(2) {
        sum += u16::from_be_bytes([w[0], w[1]]) as u32;
    }
    while sum > 0xFFFF {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    let c = !(sum as u16);
    ip[10] = (c >> 8) as u8;
    ip[11] = c as u8;
    let ulen = (8 + payload.len()) as u16;
    let mut f = Vec::with_capacity(EOE_HEADERS + payload.len());
    f.extend_from_slice(&dst);
    f.extend_from_slice(&src);
    f.extend_from_slice(&0x0800u16.to_be_bytes());
    f.extend_from_slice(&ip);
    f.extend_from_slice(&a.source_port.to_be_bytes());
    f.extend_from_slice(&a.destination_port.to_be_bytes());
    f.extend_from_slice(&ulen.to_be_bytes());
    f.extend_from_slice(&[0, 0]);
    f.extend_from_slice(payload);
    f
}

/// The UDP payload of an Ethernet II / IPv4 / UDP frame to `port`, if it is one.
pub fn udp_payload(frame: &[u8], port: u16) -> Option<&[u8]> {
    if frame.len() < EOE_HEADERS || frame[12..14] != [0x08, 0x00] || frame[14] >> 4 != 4 || frame[23] != 17 {
        return None;
    }
    let ihl = (frame[14] & 0x0F) as usize * 4;
    let udp = 14 + ihl;
    let h = frame.get(udp..udp + 8)?;
    if u16::from_be_bytes([h[2], h[3]]) != port {
        return None;
    }
    let len = (u16::from_be_bytes([h[4], h[5]]) as usize).checked_sub(8)?;
    frame.get(udp + 8..udp + 8 + len)
}

/// Split one Ethernet frame into EoE fragments (header and data) of at most `capacity` data
/// bytes; all but the last carry a multiple of 32 bytes. An empty frame is one empty fragment.
pub fn eoe_fragments(frame: &[u8], capacity: usize, frame_no: u8) -> Vec<Vec<u8>> {
    let chunk = capacity / EoeHeader::BLOCK * EoeHeader::BLOCK;
    let mut out = Vec::new();
    let mut at = 0;
    let mut fragment = 0u8;
    loop {
        let left = frame.len() - at;
        let n = if left <= capacity { left } else { chunk };
        let h = EoeHeader {
            frame_type: EoeType::Fragment,
            port: 0,
            last_fragment: at + n == frame.len(),
            time_appended: false,
            time_request: false,
            fragment,
            offset_blocks: if fragment == 0 {
                frame.len().div_ceil(EoeHeader::BLOCK) as u8
            } else {
                (at / EoeHeader::BLOCK) as u8
            },
            frame_no,
        };
        let mut v = h.encode().to_vec();
        v.extend_from_slice(&frame[at..at + n]);
        out.push(v);
        at += n;
        fragment = fragment.wrapping_add(1);
        if at >= frame.len() {
            return out;
        }
    }
}

/// Reassembles EoE fragments into Ethernet frames.
#[derive(Debug, Default)]
pub struct EoeReassembler {
    frame: Vec<u8>,
    frame_no: Option<u8>,
    next_fragment: u8,
    /// Fragments that did not continue the frame being assembled, and frames abandoned for a
    /// new one (each dropped).
    pub broken: u64,
}

impl EoeReassembler {
    /// Take one fragment (data after the mailbox header). Returns a complete frame.
    pub fn push(&mut self, body: &[u8]) -> Option<Vec<u8>> {
        let h = EoeHeader::parse(body).ok()?;
        if h.frame_type != EoeType::Fragment {
            return None;
        }
        if h.fragment == 0 {
            if self.frame_no.is_some() {
                self.broken += 1;
            }
            self.frame.clear();
            self.frame_no = Some(h.frame_no);
            self.next_fragment = 0;
        }
        if self.frame_no != Some(h.frame_no) || self.next_fragment != h.fragment {
            self.broken += 1;
            self.frame_no = None;
            self.frame.clear();
            return None;
        }
        self.frame.extend_from_slice(&body[EoeHeader::LEN..]);
        self.next_fragment = self.next_fragment.wrapping_add(1);
        if h.last_fragment {
            self.frame_no = None;
            return Some(std::mem::take(&mut self.frame));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_cycle() {
        assert_eq!([0, 1, 6, 7].map(next_counter), [1, 2, 7, 1]);
        assert_eq!([0, 1, 14, 15].map(next_eoe_frame_no), [1, 2, 15, 1]);
    }

    #[test]
    fn buffers_and_segments_are_padded() {
        let b = mailbox_buffer(MailboxType::Foe, 3, 0x1001, &[1, 2, 3], 16);
        assert_eq!(b.len(), 16);
        let h = MailboxHeader::parse(&b).unwrap();
        assert_eq!((h.length, h.counter, h.address, h.mailbox_type), (3, 3, 0x1001, MailboxType::Foe));
        assert_eq!(&b[MAILBOX_HEADER_LEN..MAILBOX_HEADER_LEN + 3], &[1, 2, 3]);
        let cmd = SdoCommand::DownloadSegmentRequest { toggle: false, unused: sdo_unused(2), last: true };
        assert_eq!(sdo_segment(CoeService::SdoRequest, cmd, &[9, 9]).len(), CoeHeader::LEN + 1 + 7);
        assert_eq!(sdo_unused(2), 5);
        assert_eq!(sdo_unused(100), 0);
        assert_eq!(foe_capacity(128), 128 - 6 - 6);
        assert_eq!(sdo_initiate_capacity(128) + 8, sdo_segment_capacity(128) + 1);
    }

    #[test]
    fn eoe_fragments_reassemble() {
        let a = EoeAddresses::default();
        for len in [0usize, 1, 50, 86, 87, 500, 1458] {
            let payload: Vec<u8> = (0..len).map(|i| i as u8).collect();
            let frame = udp_frame(&payload, &a, true);
            let mut r = EoeReassembler::default();
            let mut out = None;
            let parts = eoe_fragments(&frame, eoe_capacity(128), 3);
            assert!(
                parts[..parts.len() - 1].iter().all(|p| (p.len() - EoeHeader::LEN).is_multiple_of(EoeHeader::BLOCK))
            );
            for f in parts {
                assert!(f.len() <= 128 - MAILBOX_HEADER_LEN);
                assert!(out.is_none());
                out = r.push(&f);
            }
            let out = out.expect("a frame");
            assert_eq!(out, frame);
            assert_eq!(udp_payload(&out, a.destination_port), Some(&payload[..]));
            assert_eq!(udp_payload(&out, 7), None);
        }
        assert_eq!(eoe_fragments(&[], 64, 1).len(), 1);
    }

    #[test]
    fn reassembler_drops_a_frame_with_a_missing_fragment() {
        let a = EoeAddresses::default();
        let frame = udp_frame(&[7u8; 300], &a, true);
        let frags = eoe_fragments(&frame, 118, 1);
        assert!(frags.len() > 2);
        let mut r = EoeReassembler::default();
        assert_eq!(r.push(&frags[0]), None);
        assert_eq!(r.push(&frags[2]), None);
        assert_eq!(r.broken, 1);
        for f in &frags {
            if let Some(out) = r.push(f) {
                assert_eq!(out, frame);
            }
        }
    }
}
