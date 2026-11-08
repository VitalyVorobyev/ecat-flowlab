//! Master-side transfer protocols as message-level state machines.
//!
//! An [`Endpoint`] hands out the next mailbox message (`next`) and consumes the slave's
//! (`on`). The engine adds the mailbox header and the counter, pads to the SyncManager length
//! and moves the messages; the endpoints only see the data after the mailbox header.
//! The message formats are the simulator's (`flowlab-sim`), so both put the same bytes on the
//! wire for the same transfer.

use std::collections::VecDeque;

use ecat_wire::{
    CoeHeader, CoeService, EoeHeader, EoeType, FoeHeader, FoeOpcode, MAILBOX_HEADER_LEN, MailboxType, SdoCommand,
    SdoHeader, foe_error_text, sdo_abort_text, segment_data_len,
};
use serde::{Deserialize, Serialize};

/// One message for the slave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Outgoing {
    pub(crate) kind: MailboxType,
    /// Data after the mailbox header.
    pub(crate) body: Vec<u8>,
    /// Payload offset after this message's data, if it carries payload.
    pub(crate) payload_end: Option<u64>,
}

impl Outgoing {
    fn control(kind: MailboxType, body: Vec<u8>) -> Self {
        Self { kind, body, payload_end: None }
    }
}

/// The master's side of one transfer protocol.
pub(crate) trait Endpoint {
    /// The next message to send now, if any.
    fn next(&mut self) -> Option<Outgoing>;
    /// A message from the slave (data after the mailbox header, `length` long). `Ok(false)`:
    /// not for this transfer (another protocol), ignored. `Err`: the slave ended the transfer.
    fn on(&mut self, kind: MailboxType, body: &[u8]) -> Result<bool, String>;
    /// The slave may have messages for the master (the master reads only then).
    fn wants_input(&self) -> bool;
    /// The master's part is finished.
    fn done(&self) -> bool;
    /// The slave asked for the last message again (FoE busy): resend it.
    fn take_resend_request(&mut self) -> bool {
        false
    }
    /// Payload received from the slave so far (slave → master transfers).
    fn received(&self) -> Option<&[u8]> {
        None
    }
    /// Payload size, once known (from the master's payload or the slave's announcement).
    fn total(&self) -> Option<u64>;
}

// ---------------------------------------------------------------------------------------------
// FoE
// ---------------------------------------------------------------------------------------------

fn foe(op: FoeOpcode, value: u32, tail: &[u8]) -> Vec<u8> {
    let mut v = FoeHeader { opcode: op, value }.encode().to_vec();
    v.extend_from_slice(tail);
    v
}

fn foe_error(body: &[u8]) -> String {
    let h = FoeHeader::parse(body).map(|h| h.value).unwrap_or_default();
    let text = String::from_utf8_lossy(body.get(FoeHeader::LEN..).unwrap_or_default());
    let text = text.trim_end_matches('\0');
    let name = foe_error_text(h).unwrap_or("unknown error");
    if text.is_empty() {
        format!("FoE error {h:#06x} from the slave: {name}")
    } else {
        format!("FoE error {h:#06x} from the slave: {name} ({text})")
    }
}

/// FoE write: WRQ, then DATA packets, each acknowledged before the next; the transfer ends
/// with a packet shorter than a full one (zero bytes if the file fills the last packet).
pub(crate) struct FoeWrite {
    data: Vec<u8>,
    request: Vec<u8>,
    capacity: usize,
    sent: usize,
    state: FoeTx,
    last_short: bool,
    resend: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FoeTx {
    Request,
    WaitAck(u32),
    Send(u32),
    Done,
}

impl FoeWrite {
    pub(crate) fn new(data: Vec<u8>, file_name: &str, password: u32, out_len: usize) -> Self {
        let capacity = out_len - MAILBOX_HEADER_LEN - FoeHeader::LEN;
        let request = foe(FoeOpcode::Write, password, file_name.as_bytes());
        Self { data, request, capacity, sent: 0, state: FoeTx::Request, last_short: false, resend: false }
    }
}

impl Endpoint for FoeWrite {
    fn next(&mut self) -> Option<Outgoing> {
        match self.state {
            FoeTx::Request => {
                self.state = FoeTx::WaitAck(0);
                Some(Outgoing::control(MailboxType::Foe, self.request.clone()))
            }
            FoeTx::Send(n) => {
                let end = (self.sent + self.capacity).min(self.data.len());
                let chunk = &self.data[self.sent..end];
                self.last_short = chunk.len() < self.capacity;
                let body = foe(FoeOpcode::Data, n, chunk);
                self.sent = end;
                self.state = FoeTx::WaitAck(n);
                Some(Outgoing { kind: MailboxType::Foe, body, payload_end: Some(end as u64) })
            }
            _ => None,
        }
    }

    fn on(&mut self, kind: MailboxType, body: &[u8]) -> Result<bool, String> {
        if kind != MailboxType::Foe {
            return Ok(false);
        }
        let h = FoeHeader::parse(body).map_err(|e| format!("FoE message from the slave: {e}"))?;
        match (self.state, h.opcode) {
            (FoeTx::WaitAck(n), FoeOpcode::Ack) if h.value == n => {
                self.state = if n > 0 && self.last_short { FoeTx::Done } else { FoeTx::Send(n + 1) };
            }
            (FoeTx::WaitAck(_), FoeOpcode::Busy) => self.resend = true,
            (_, FoeOpcode::Error) => return Err(foe_error(body)),
            _ => {}
        }
        Ok(true)
    }

    fn wants_input(&self) -> bool {
        matches!(self.state, FoeTx::WaitAck(_))
    }

    fn done(&self) -> bool {
        self.state == FoeTx::Done
    }

    fn take_resend_request(&mut self) -> bool {
        std::mem::take(&mut self.resend)
    }

    fn total(&self) -> Option<u64> {
        Some(self.data.len() as u64)
    }
}

/// FoE read: RRQ, then the slave's DATA packets, each acknowledged; a packet shorter than
/// `packet_bytes` ends the file.
pub(crate) struct FoeRead {
    data: Vec<u8>,
    packet_bytes: usize,
    pending: VecDeque<Outgoing>,
    expected: u32,
    finished: bool,
}

impl FoeRead {
    pub(crate) fn new(file_name: &str, password: u32, packet_bytes: usize) -> Self {
        let mut pending = VecDeque::new();
        pending.push_back(Outgoing::control(MailboxType::Foe, foe(FoeOpcode::Read, password, file_name.as_bytes())));
        Self { data: Vec::new(), packet_bytes, pending, expected: 1, finished: false }
    }
}

impl Endpoint for FoeRead {
    fn next(&mut self) -> Option<Outgoing> {
        self.pending.pop_front()
    }

    fn on(&mut self, kind: MailboxType, body: &[u8]) -> Result<bool, String> {
        if kind != MailboxType::Foe {
            return Ok(false);
        }
        let h = FoeHeader::parse(body).map_err(|e| format!("FoE message from the slave: {e}"))?;
        match h.opcode {
            FoeOpcode::Data if h.value == self.expected && !self.finished => {
                let chunk = &body[FoeHeader::LEN..];
                self.data.extend_from_slice(chunk);
                self.finished = chunk.len() < self.packet_bytes;
                self.pending.push_back(Outgoing::control(MailboxType::Foe, foe(FoeOpcode::Ack, h.value, &[])));
                self.expected += 1;
            }
            // Our acknowledgement was lost and the slave sent the packet again.
            FoeOpcode::Data if h.value + 1 == self.expected => {
                self.pending.push_back(Outgoing::control(MailboxType::Foe, foe(FoeOpcode::Ack, h.value, &[])));
            }
            FoeOpcode::Error => return Err(foe_error(body)),
            _ => {}
        }
        Ok(true)
    }

    fn wants_input(&self) -> bool {
        !self.finished
    }

    fn done(&self) -> bool {
        self.finished && self.pending.is_empty()
    }

    fn received(&self) -> Option<&[u8]> {
        Some(&self.data)
    }

    fn total(&self) -> Option<u64> {
        self.finished.then_some(self.data.len() as u64)
    }
}

// ---------------------------------------------------------------------------------------------
// CoE SDO, normal transfer with segments
// ---------------------------------------------------------------------------------------------

fn sdo(cmd: SdoCommand, index: u16, subindex: u8, data: u32, tail: &[u8]) -> Vec<u8> {
    let mut v = CoeHeader { number: 0, service: CoeService::SdoRequest }.encode().to_vec();
    v.extend_from_slice(&SdoHeader { command: cmd.encode(), index, subindex, data }.encode());
    v.extend_from_slice(tail);
    v
}

fn segment(cmd: SdoCommand, data: &[u8]) -> Vec<u8> {
    let mut v = CoeHeader { number: 0, service: CoeService::SdoRequest }.encode().to_vec();
    v.push(cmd.encode());
    v.extend_from_slice(data);
    if data.len() < 7 {
        v.resize(CoeHeader::LEN + 1 + 7, 0);
    }
    v
}

fn unused(n: usize) -> u8 {
    if n < 7 { (7 - n) as u8 } else { 0 }
}

/// An SDO response, or `None` for other CoE services (emergencies, …).
fn sdo_response(body: &[u8]) -> Result<Option<(SdoCommand, Option<SdoHeader>)>, String> {
    let c = CoeHeader::parse(body).map_err(|e| format!("CoE message from the slave: {e}"))?;
    if c.service != CoeService::SdoResponse && c.service != CoeService::SdoRequest {
        return Ok(None);
    }
    let Some(&byte) = body.get(CoeHeader::LEN) else { return Err("CoE SDO response without a command".into()) };
    let cmd = SdoCommand::decode(byte, false);
    let hdr = SdoHeader::parse(&body[CoeHeader::LEN..]).ok();
    if cmd == SdoCommand::Abort {
        let code = hdr.map_or(0, |h| h.data);
        let text = sdo_abort_text(code).unwrap_or("unknown abort code");
        return Err(format!("SDO abort {code:#010x} from the slave: {text}"));
    }
    Ok(Some((cmd, hdr)))
}

/// SDO download (master → slave): initiate request with the size and the first data, then
/// segments with alternating toggle bits, each answered before the next.
pub(crate) struct SdoDownload {
    data: Vec<u8>,
    index: u16,
    subindex: u8,
    init_cap: usize,
    seg_cap: usize,
    sent: usize,
    state: SdoTx,
    toggle: bool,
    last: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SdoTx {
    Init,
    WaitInit,
    Segment,
    WaitSegment,
    Done,
}

impl SdoDownload {
    pub(crate) fn new(data: Vec<u8>, index: u16, subindex: u8, out_len: usize) -> Self {
        Self {
            data,
            index,
            subindex,
            init_cap: out_len - MAILBOX_HEADER_LEN - CoeHeader::LEN - SdoHeader::LEN,
            seg_cap: out_len - MAILBOX_HEADER_LEN - CoeHeader::LEN - 1,
            sent: 0,
            state: SdoTx::Init,
            toggle: false,
            last: false,
        }
    }
}

impl Endpoint for SdoDownload {
    fn next(&mut self) -> Option<Outgoing> {
        match self.state {
            SdoTx::Init => {
                let end = self.init_cap.min(self.data.len());
                self.state = SdoTx::WaitInit;
                let cmd = SdoCommand::InitiateDownloadRequest {
                    complete_access: false,
                    expedited: false,
                    size_indicated: true,
                    unused: 0,
                };
                let body = sdo(cmd, self.index, self.subindex, self.data.len() as u32, &self.data[..end]);
                self.sent = end;
                Some(Outgoing { kind: MailboxType::Coe, body, payload_end: (end > 0).then_some(end as u64) })
            }
            SdoTx::Segment => {
                let end = (self.sent + self.seg_cap).min(self.data.len());
                let n = end - self.sent;
                self.last = end == self.data.len();
                let cmd =
                    SdoCommand::DownloadSegmentRequest { toggle: self.toggle, unused: unused(n), last: self.last };
                let body = segment(cmd, &self.data[self.sent..end]);
                self.sent = end;
                self.state = SdoTx::WaitSegment;
                Some(Outgoing { kind: MailboxType::Coe, body, payload_end: Some(end as u64) })
            }
            _ => None,
        }
    }

    fn on(&mut self, kind: MailboxType, body: &[u8]) -> Result<bool, String> {
        if kind != MailboxType::Coe {
            return Ok(false);
        }
        let Some((cmd, _)) = sdo_response(body)? else { return Ok(false) };
        match (self.state, cmd) {
            (SdoTx::WaitInit, SdoCommand::InitiateDownloadResponse) => {
                self.state = if self.sent == self.data.len() { SdoTx::Done } else { SdoTx::Segment };
                self.toggle = false;
            }
            (SdoTx::WaitSegment, SdoCommand::DownloadSegmentResponse { toggle }) if toggle == self.toggle => {
                if self.last {
                    self.state = SdoTx::Done;
                } else {
                    self.toggle = !self.toggle;
                    self.state = SdoTx::Segment;
                }
            }
            _ => {}
        }
        Ok(true)
    }

    fn wants_input(&self) -> bool {
        matches!(self.state, SdoTx::WaitInit | SdoTx::WaitSegment)
    }

    fn done(&self) -> bool {
        self.state == SdoTx::Done
    }

    fn total(&self) -> Option<u64> {
        Some(self.data.len() as u64)
    }
}

/// SDO upload (slave → master): initiate request; the response announces the size (and
/// carries the data if expedited, or the first data if not); then segment requests until the
/// slave marks a segment as the last.
pub(crate) struct SdoUpload {
    data: Vec<u8>,
    size: Option<u64>,
    pending: VecDeque<Outgoing>,
    toggle: bool,
    waiting: bool,
    finished: bool,
}

impl SdoUpload {
    pub(crate) fn new(index: u16, subindex: u8) -> Self {
        let mut pending = VecDeque::new();
        let cmd = SdoCommand::InitiateUploadRequest { complete_access: false };
        pending.push_back(Outgoing::control(MailboxType::Coe, sdo(cmd, index, subindex, 0, &[])));
        Self { data: Vec::new(), size: None, pending, toggle: false, waiting: true, finished: false }
    }

    fn request_segment(&mut self) {
        let cmd = SdoCommand::UploadSegmentRequest { toggle: self.toggle };
        self.pending.push_back(Outgoing::control(MailboxType::Coe, segment(cmd, &[])));
        self.waiting = true;
    }
}

impl Endpoint for SdoUpload {
    fn next(&mut self) -> Option<Outgoing> {
        self.pending.pop_front()
    }

    fn on(&mut self, kind: MailboxType, body: &[u8]) -> Result<bool, String> {
        if kind != MailboxType::Coe {
            return Ok(false);
        }
        let Some((cmd, hdr)) = sdo_response(body)? else { return Ok(false) };
        match cmd {
            SdoCommand::InitiateUploadResponse { expedited, size_indicated, unused, .. } if self.size.is_none() => {
                let h = hdr.ok_or("SDO upload response too short")?;
                self.waiting = false;
                if expedited {
                    let n = if size_indicated { 4 - unused.min(3) as usize } else { 4 };
                    self.data.extend_from_slice(&h.data.to_le_bytes()[..n]);
                    self.size = Some(n as u64);
                    self.finished = true;
                } else {
                    let size = h.data as u64;
                    let tail = &body[CoeHeader::LEN + SdoHeader::LEN..];
                    let n = tail.len().min(size as usize);
                    self.data.extend_from_slice(&tail[..n]);
                    self.size = Some(size);
                    self.finished = self.data.len() as u64 >= size;
                    if !self.finished {
                        self.toggle = false;
                        self.request_segment();
                    }
                }
            }
            SdoCommand::UploadSegmentResponse { toggle, unused, last } if self.waiting && toggle == self.toggle => {
                let n = segment_data_len(body.len() as u16, unused);
                let room = self.size.unwrap_or(u64::MAX).saturating_sub(self.data.len() as u64);
                let n = n.min(room as usize);
                self.data.extend_from_slice(&body[CoeHeader::LEN + 1..][..n]);
                self.waiting = false;
                self.finished = last;
                if !last {
                    self.toggle = !self.toggle;
                    self.request_segment();
                }
            }
            _ => {}
        }
        Ok(true)
    }

    fn wants_input(&self) -> bool {
        self.waiting
    }

    fn done(&self) -> bool {
        self.finished && self.pending.is_empty()
    }

    fn received(&self) -> Option<&[u8]> {
        Some(&self.data)
    }

    fn total(&self) -> Option<u64> {
        self.size
    }
}

// ---------------------------------------------------------------------------------------------
// EoE: UDP/IPv4 datagrams tunnelled in EoE fragments
// ---------------------------------------------------------------------------------------------

/// Ethernet + IPv4 + UDP header bytes of a tunnelled frame.
pub const EOE_HEADERS: usize = 14 + 20 + 8;

/// Addresses of the UDP/IPv4 frames an EoE transfer tunnels. The defaults are the
/// simulator's: documentation MACs (RFC 7042) and TEST-NET-1 addresses (RFC 5737), UDP
/// ports 50000 → 50001.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EoeAddresses {
    /// The master's (host side) MAC.
    pub master_mac: [u8; 6],
    /// The slave's MAC (its EoE interface).
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

/// An Ethernet II frame with an IPv4/UDP datagram carrying `payload` (UDP checksum 0).
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

/// Split one Ethernet frame into EoE fragments of at most `capacity` data bytes; all but the
/// last carry a multiple of 32 bytes.
pub fn eoe_fragments(frame: &[u8], capacity: usize, frame_no: u8) -> Vec<Vec<u8>> {
    let chunk = capacity / EoeHeader::BLOCK * EoeHeader::BLOCK;
    let mut out = Vec::new();
    let mut at = 0;
    let mut fragment = 0u8;
    while at < frame.len() || (frame.is_empty() && out.is_empty()) {
        let left = frame.len() - at;
        let n = if left <= capacity { left } else { chunk };
        let last = at + n == frame.len();
        let h = EoeHeader {
            frame_type: EoeType::Fragment,
            port: 0,
            last_fragment: last,
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
        fragment += 1;
        if frame.is_empty() {
            break;
        }
    }
    out
}

/// Reassembles EoE fragments into Ethernet frames.
#[derive(Debug, Default)]
pub struct EoeReassembler {
    frame: Vec<u8>,
    frame_no: Option<u8>,
    next_fragment: u8,
    /// Fragments that did not continue the frame being assembled (dropped with it).
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

/// EoE send: the payload in UDP datagrams of `mtu` bytes (IP MTU), each frame fragmented to
/// fit "mailbox out". No acknowledgements: the transfer ends when the last fragment is written.
pub(crate) struct EoeSend {
    data: Vec<u8>,
    addresses: EoeAddresses,
    mtu: usize,
    capacity: usize,
    taken: usize,
    queue: VecDeque<Outgoing>,
    frame_no: u8,
}

impl EoeSend {
    pub(crate) fn new(data: Vec<u8>, addresses: EoeAddresses, mtu: usize, out_len: usize) -> Self {
        let capacity = out_len - MAILBOX_HEADER_LEN - EoeHeader::LEN;
        Self { data, addresses, mtu, capacity, taken: 0, queue: VecDeque::new(), frame_no: 0 }
    }

    fn fragment_next_frame(&mut self) {
        let end = (self.taken + self.mtu - 28).min(self.data.len());
        let frame = udp_frame(&self.data[self.taken..end], &self.addresses, true);
        self.taken = end;
        self.frame_no = self.frame_no % 15 + 1;
        let fragments = eoe_fragments(&frame, self.capacity, self.frame_no);
        let n = fragments.len();
        for (i, body) in fragments.into_iter().enumerate() {
            let payload_end = (i + 1 == n).then_some(end as u64);
            self.queue.push_back(Outgoing { kind: MailboxType::Eoe, body, payload_end });
        }
    }
}

impl Endpoint for EoeSend {
    fn next(&mut self) -> Option<Outgoing> {
        if self.queue.is_empty() && self.taken < self.data.len() {
            self.fragment_next_frame();
        }
        self.queue.pop_front()
    }

    fn on(&mut self, kind: MailboxType, _: &[u8]) -> Result<bool, String> {
        Ok(kind == MailboxType::Eoe)
    }

    fn wants_input(&self) -> bool {
        false
    }

    fn done(&self) -> bool {
        self.taken == self.data.len() && self.queue.is_empty()
    }

    fn total(&self) -> Option<u64> {
        Some(self.data.len() as u64)
    }
}

/// EoE receive: reassembles the slave's frames and keeps the payload of UDP datagrams to
/// `destination_port` until `expected` bytes arrived. Other frames are counted, not kept.
pub(crate) struct EoeReceive {
    reassembler: EoeReassembler,
    port: u16,
    expected: u64,
    data: Vec<u8>,
    pub(crate) other_frames: u64,
}

impl EoeReceive {
    pub(crate) fn new(port: u16, expected: u64) -> Self {
        Self { reassembler: EoeReassembler::default(), port, expected, data: Vec::new(), other_frames: 0 }
    }
}

impl Endpoint for EoeReceive {
    fn next(&mut self) -> Option<Outgoing> {
        None
    }

    fn on(&mut self, kind: MailboxType, body: &[u8]) -> Result<bool, String> {
        if kind != MailboxType::Eoe {
            return Ok(false);
        }
        if let Some(frame) = self.reassembler.push(body) {
            match udp_payload(&frame, self.port) {
                Some(p) => self.data.extend_from_slice(p),
                None => self.other_frames += 1,
            }
        }
        Ok(true)
    }

    fn wants_input(&self) -> bool {
        (self.data.len() as u64) < self.expected
    }

    fn done(&self) -> bool {
        !self.wants_input()
    }

    fn received(&self) -> Option<&[u8]> {
        Some(&self.data)
    }

    fn total(&self) -> Option<u64> {
        Some(self.expected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eoe_fragments_reassemble() {
        let a = EoeAddresses::default();
        for len in [0usize, 1, 50, 86, 87, 500, 1458] {
            let payload: Vec<u8> = (0..len).map(|i| i as u8).collect();
            let frame = udp_frame(&payload, &a, true);
            let mut r = EoeReassembler::default();
            let mut out = None;
            for f in eoe_fragments(&frame, 128 - 10, 3) {
                assert!(f.len() <= 128 - MAILBOX_HEADER_LEN);
                assert!(out.is_none());
                out = r.push(&f);
            }
            let out = out.expect("a frame");
            assert_eq!(out, frame);
            assert_eq!(udp_payload(&out, a.destination_port), Some(&payload[..]));
            assert_eq!(udp_payload(&out, 7), None);
        }
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
