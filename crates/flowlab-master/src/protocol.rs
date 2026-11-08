//! Master-side transfer protocols as message-level state machines.
//!
//! An [`Endpoint`] hands out the next mailbox message (`next`) and consumes the slave's
//! (`on`). The engine adds the mailbox header and the counter, pads to the SyncManager length
//! and moves the messages; the endpoints only see the data after the mailbox header.
//! The messages come from `flowlab_mailbox::msg`, as the simulator's do, so both put the same
//! bytes on the wire for the same transfer.

use std::collections::VecDeque;

use ecat_wire::{
    CoeHeader, CoeService, FoeHeader, FoeOpcode, MailboxType, SdoCommand, SdoHeader, foe_error_text, sdo_abort_text,
    segment_data_len,
};
use flowlab_mailbox::msg::{self, EoeAddresses, EoeReassembler, eoe_fragments, foe, udp_frame, udp_payload};

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
        let capacity = msg::foe_capacity(out_len);
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
            init_cap: msg::sdo_initiate_capacity(out_len),
            seg_cap: msg::sdo_segment_capacity(out_len),
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
                let body = msg::sdo(
                    CoeService::SdoRequest,
                    cmd,
                    self.index,
                    self.subindex,
                    self.data.len() as u32,
                    &self.data[..end],
                );
                self.sent = end;
                Some(Outgoing { kind: MailboxType::Coe, body, payload_end: (end > 0).then_some(end as u64) })
            }
            SdoTx::Segment => {
                let end = (self.sent + self.seg_cap).min(self.data.len());
                let n = end - self.sent;
                self.last = end == self.data.len();
                let cmd = SdoCommand::DownloadSegmentRequest {
                    toggle: self.toggle,
                    unused: msg::sdo_unused(n),
                    last: self.last,
                };
                let body = msg::sdo_segment(CoeService::SdoRequest, cmd, &self.data[self.sent..end]);
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
        pending.push_back(Outgoing::control(
            MailboxType::Coe,
            msg::sdo(CoeService::SdoRequest, cmd, index, subindex, 0, &[]),
        ));
        Self { data: Vec::new(), size: None, pending, toggle: false, waiting: true, finished: false }
    }

    fn request_segment(&mut self) {
        let cmd = SdoCommand::UploadSegmentRequest { toggle: self.toggle };
        self.pending.push_back(Outgoing::control(MailboxType::Coe, msg::sdo_segment(CoeService::SdoRequest, cmd, &[])));
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
        let capacity = msg::eoe_capacity(out_len);
        Self { data, addresses, mtu, capacity, taken: 0, queue: VecDeque::new(), frame_no: 0 }
    }

    fn fragment_next_frame(&mut self) {
        let end = (self.taken + self.mtu - 28).min(self.data.len());
        let frame = udp_frame(&self.data[self.taken..end], &self.addresses, true);
        self.taken = end;
        self.frame_no = msg::next_eoe_frame_no(self.frame_no);
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
