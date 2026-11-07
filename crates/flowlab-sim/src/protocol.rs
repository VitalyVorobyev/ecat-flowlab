//! Transfer protocols as message-level state machines, one per side.
//!
//! An [`Endpoint`] produces mailbox messages (`next`) and consumes the peer's (`on`). The
//! engine moves messages through the ESC's SyncManagers; it adds the mailbox header and pads
//! to the SyncManager length. Payload bytes come from [`payload_bytes`], a deterministic
//! function of seed and offset, so the receiver can check every byte it gets.

use std::collections::VecDeque;

use ecat_wire::{
    CoeHeader, CoeService, EoeHeader, EoeType, FoeHeader, FoeOpcode, MAILBOX_HEADER_LEN, MailboxType, SdoCommand,
    SdoHeader, segment_data_len,
};
use sha2::{Digest, Sha256};

use crate::spec::{PDO_ACK_BYTES, PDO_CHUNK_HEADER};

/// Fill `out` with the payload bytes at `offset`.
pub fn payload_bytes(seed: u64, offset: u64, out: &mut [u8]) {
    for (i, b) in out.iter_mut().enumerate() {
        let pos = offset + i as u64;
        let mut z = seed ^ (pos >> 3).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        *b = (z >> ((pos & 7) * 8)) as u8;
    }
}

/// SHA-256 of the whole payload, hex.
pub fn payload_sha256(seed: u64, len: u64) -> String {
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 65536];
    let mut at = 0;
    while at < len {
        let n = (len - at).min(buf.len() as u64) as usize;
        payload_bytes(seed, at, &mut buf[..n]);
        h.update(&buf[..n]);
        at += n as u64;
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// The sending application: hands out the payload in order.
#[derive(Debug, Clone)]
pub(crate) struct Source {
    seed: u64,
    total: u64,
    sent: u64,
}

impl Source {
    pub(crate) fn new(seed: u64, total: u64) -> Self {
        Self { seed, total, sent: 0 }
    }

    pub(crate) fn remaining(&self) -> u64 {
        self.total - self.sent
    }

    pub(crate) fn take(&mut self, max: usize) -> Vec<u8> {
        let n = self.remaining().min(max as u64) as usize;
        let mut v = vec![0u8; n];
        payload_bytes(self.seed, self.sent, &mut v);
        self.sent += n as u64;
        v
    }
}

/// The receiving application: checks every byte against the expected payload and hashes it.
#[derive(Debug, Clone)]
pub(crate) struct Sink {
    seed: u64,
    pub(crate) total: u64,
    pub(crate) got: u64,
    pub(crate) corrupt: bool,
    hasher: Sha256,
    pub(crate) first_ns: Option<u64>,
    pub(crate) end_ns: Option<u64>,
}

impl Sink {
    pub(crate) fn new(seed: u64, total: u64, now: u64) -> Self {
        let mut s = Self { seed, total, got: 0, corrupt: false, hasher: Sha256::new(), first_ns: None, end_ns: None };
        if total == 0 {
            s.end_ns = Some(now);
        }
        s
    }

    pub(crate) fn deliver(&mut self, data: &[u8], now: u64) {
        if data.is_empty() {
            return;
        }
        if self.got + data.len() as u64 > self.total {
            self.corrupt = true;
            return;
        }
        let mut expect = vec![0u8; data.len()];
        payload_bytes(self.seed, self.got, &mut expect);
        if expect != data {
            self.corrupt = true;
        }
        self.hasher.update(data);
        self.got += data.len() as u64;
        self.first_ns.get_or_insert(now);
        if self.got == self.total {
            self.end_ns = Some(now);
        }
    }

    pub(crate) fn complete(&self) -> bool {
        self.got == self.total
    }

    pub(crate) fn sha256(&self) -> String {
        self.hasher.clone().finalize().iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// One side of a mailbox protocol.
pub(crate) trait Endpoint {
    /// The next message to send now, if any: mailbox type and data after the mailbox header.
    fn next(&mut self) -> Option<(MailboxType, Vec<u8>)>;
    /// A message from the peer arrived (`body` = data after the mailbox header, `length` long).
    fn on(&mut self, t: MailboxType, body: &[u8], now: u64);
    /// The peer may have messages for this side (the master polls only then).
    fn wants_input(&self) -> bool;
    /// This side has finished its part.
    fn done(&self) -> bool;
    /// The receiving application, if this side receives the payload.
    fn sink(&self) -> Option<&Sink> {
        None
    }
}

// ---------------------------------------------------------------------------------------------
// EoE: UDP/IPv4 datagrams tunnelled in EoE fragments
// ---------------------------------------------------------------------------------------------

/// Ethernet + IPv4 + UDP header bytes of a tunnelled frame.
pub(crate) const EOE_HEADERS: usize = 14 + 20 + 8;
const EOE_DST: [u8; 6] = [0x00, 0x00, 0x5E, 0x00, 0x53, 0x20];
const EOE_SRC: [u8; 6] = [0x00, 0x00, 0x5E, 0x00, 0x53, 0x21];

fn udp_frame(payload: &[u8], to_slave: bool) -> Vec<u8> {
    let (dst, src) = if to_slave { (EOE_DST, EOE_SRC) } else { (EOE_SRC, EOE_DST) };
    // 192.0.2.0/24 (RFC 5737 TEST-NET-1).
    let (sip, dip) = if to_slave { ([192, 0, 2, 1], [192, 0, 2, 2]) } else { ([192, 0, 2, 2], [192, 0, 2, 1]) };
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
    f.extend_from_slice(&[0xC3, 0x50, 0xC3, 0x51, (ulen >> 8) as u8, ulen as u8, 0, 0]); // ports 50000 → 50001
    f.extend_from_slice(payload);
    f
}

pub(crate) struct EoeSender {
    src: Source,
    mtu: usize,
    capacity: usize,
    to_slave: bool,
    queue: VecDeque<Vec<u8>>,
    frame_no: u8,
}

impl EoeSender {
    pub(crate) fn new(src: Source, mtu: usize, mailbox_len: usize, to_slave: bool) -> Self {
        let capacity = mailbox_len - MAILBOX_HEADER_LEN - EoeHeader::LEN;
        Self { src, mtu, capacity, to_slave, queue: VecDeque::new(), frame_no: 0 }
    }

    fn fragment_next_frame(&mut self) {
        let frame = udp_frame(&self.src.take(self.mtu - 28), self.to_slave);
        self.frame_no = self.frame_no % 15 + 1;
        // Fragments other than the last carry a multiple of 32 bytes.
        let chunk = self.capacity / EoeHeader::BLOCK * EoeHeader::BLOCK;
        let mut at = 0;
        let mut fragment = 0u8;
        while at < frame.len() {
            let left = frame.len() - at;
            let n = if left <= self.capacity { left } else { chunk };
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
                frame_no: self.frame_no,
            };
            let mut v = h.encode().to_vec();
            v.extend_from_slice(&frame[at..at + n]);
            self.queue.push_back(v);
            at += n;
            fragment += 1;
        }
    }
}

impl Endpoint for EoeSender {
    fn next(&mut self) -> Option<(MailboxType, Vec<u8>)> {
        if self.queue.is_empty() && self.src.remaining() > 0 {
            self.fragment_next_frame();
        }
        self.queue.pop_front().map(|v| (MailboxType::Eoe, v))
    }
    fn on(&mut self, _: MailboxType, _: &[u8], _: u64) {}
    fn wants_input(&self) -> bool {
        false
    }
    fn done(&self) -> bool {
        self.src.remaining() == 0 && self.queue.is_empty()
    }
}

pub(crate) struct EoeReceiver {
    sink: Sink,
    frame: Vec<u8>,
    frame_no: Option<u8>,
    next_fragment: u8,
}

impl EoeReceiver {
    pub(crate) fn new(sink: Sink) -> Self {
        Self { sink, frame: Vec::new(), frame_no: None, next_fragment: 0 }
    }
}

impl Endpoint for EoeReceiver {
    fn next(&mut self) -> Option<(MailboxType, Vec<u8>)> {
        None
    }
    fn on(&mut self, t: MailboxType, body: &[u8], now: u64) {
        let Ok(h) = EoeHeader::parse(body) else { return };
        if t != MailboxType::Eoe || h.frame_type != EoeType::Fragment {
            return;
        }
        if h.fragment == 0 {
            self.frame.clear();
            self.frame_no = Some(h.frame_no);
            self.next_fragment = 0;
        }
        if self.frame_no != Some(h.frame_no) || self.next_fragment != h.fragment {
            self.sink.corrupt = true;
            return;
        }
        self.frame.extend_from_slice(&body[EoeHeader::LEN..]);
        self.next_fragment += 1;
        if h.last_fragment && self.frame.len() >= EOE_HEADERS {
            let data = std::mem::take(&mut self.frame);
            self.sink.deliver(&data[EOE_HEADERS..], now);
            self.frame_no = None;
        }
    }
    fn wants_input(&self) -> bool {
        !self.sink.complete()
    }
    fn done(&self) -> bool {
        self.sink.complete()
    }
    fn sink(&self) -> Option<&Sink> {
        Some(&self.sink)
    }
}

// ---------------------------------------------------------------------------------------------
// FoE
// ---------------------------------------------------------------------------------------------

fn foe(op: FoeOpcode, value: u32, tail: &[u8]) -> (MailboxType, Vec<u8>) {
    let mut v = FoeHeader { opcode: op, value }.encode().to_vec();
    v.extend_from_slice(tail);
    (MailboxType::Foe, v)
}

const FOE_FILE: &str = "flowlab.bin";

/// Sends a file in DATA packets, each acknowledged before the next. Used by the master for a
/// write (after WRQ) and by the slave for a read (after RRQ).
pub(crate) struct FoeSender {
    src: Source,
    capacity: usize,
    initiator: bool,
    state: FoeTx,
    last_short: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FoeTx {
    Request,
    Idle,
    Send(u32),
    WaitAck(u32),
    Done,
}

impl FoeSender {
    /// `initiator`: the sender opens with WRQ (master write); otherwise it waits for RRQ.
    pub(crate) fn new(src: Source, mailbox_len: usize, initiator: bool) -> Self {
        let capacity = mailbox_len - MAILBOX_HEADER_LEN - FoeHeader::LEN;
        let state = if initiator { FoeTx::Request } else { FoeTx::Idle };
        Self { src, capacity, initiator, state, last_short: false }
    }
}

impl Endpoint for FoeSender {
    fn next(&mut self) -> Option<(MailboxType, Vec<u8>)> {
        match self.state {
            FoeTx::Request => {
                self.state = FoeTx::WaitAck(0);
                Some(foe(FoeOpcode::Write, 0, FOE_FILE.as_bytes()))
            }
            FoeTx::Send(n) => {
                let data = self.src.take(self.capacity);
                self.last_short = data.len() < self.capacity;
                self.state = FoeTx::WaitAck(n);
                Some(foe(FoeOpcode::Data, n, &data))
            }
            _ => None,
        }
    }
    fn on(&mut self, t: MailboxType, body: &[u8], _: u64) {
        let Ok(h) = FoeHeader::parse(body) else { return };
        if t != MailboxType::Foe {
            return;
        }
        match (self.state, h.opcode) {
            (FoeTx::Idle, FoeOpcode::Read) if !self.initiator => self.state = FoeTx::Send(1),
            (FoeTx::WaitAck(n), FoeOpcode::Ack) if h.value == n => {
                self.state = if n > 0 && self.last_short { FoeTx::Done } else { FoeTx::Send(n + 1) };
            }
            _ => {}
        }
    }
    fn wants_input(&self) -> bool {
        matches!(self.state, FoeTx::WaitAck(_) | FoeTx::Idle)
    }
    fn done(&self) -> bool {
        self.state == FoeTx::Done
    }
}

/// Receives a file: acknowledges each DATA packet. The master side of a read opens with RRQ.
pub(crate) struct FoeReceiver {
    sink: Sink,
    capacity: usize,
    pending: VecDeque<(MailboxType, Vec<u8>)>,
    expected: u32,
    finished: bool,
}

impl FoeReceiver {
    /// `initiator`: this side sends RRQ first (master read).
    pub(crate) fn new(sink: Sink, mailbox_len: usize, initiator: bool) -> Self {
        let capacity = mailbox_len - MAILBOX_HEADER_LEN - FoeHeader::LEN;
        let mut pending = VecDeque::new();
        if initiator {
            pending.push_back(foe(FoeOpcode::Read, 0, FOE_FILE.as_bytes()));
        }
        Self { sink, capacity, pending, expected: 1, finished: false }
    }
}

impl Endpoint for FoeReceiver {
    fn next(&mut self) -> Option<(MailboxType, Vec<u8>)> {
        self.pending.pop_front()
    }
    fn on(&mut self, t: MailboxType, body: &[u8], now: u64) {
        let Ok(h) = FoeHeader::parse(body) else { return };
        if t != MailboxType::Foe {
            return;
        }
        match h.opcode {
            FoeOpcode::Write => self.pending.push_back(foe(FoeOpcode::Ack, 0, &[])),
            FoeOpcode::Data if h.value == self.expected => {
                let data = &body[FoeHeader::LEN..];
                self.sink.deliver(data, now);
                if data.len() < self.capacity {
                    self.finished = true;
                }
                self.pending.push_back(foe(FoeOpcode::Ack, h.value, &[]));
                self.expected += 1;
            }
            _ => {}
        }
    }
    fn wants_input(&self) -> bool {
        !self.finished
    }
    fn done(&self) -> bool {
        self.finished && self.pending.is_empty()
    }
    fn sink(&self) -> Option<&Sink> {
        Some(&self.sink)
    }
}

// ---------------------------------------------------------------------------------------------
// CoE SDO, normal transfer with segments
// ---------------------------------------------------------------------------------------------

const SDO_INDEX: u16 = 0x2000;

fn sdo(service: CoeService, cmd: SdoCommand, data: u32, tail: &[u8]) -> (MailboxType, Vec<u8>) {
    let mut v = CoeHeader { number: 0, service }.encode().to_vec();
    v.extend_from_slice(&SdoHeader { command: cmd.encode(), index: SDO_INDEX, subindex: 0, data }.encode());
    v.extend_from_slice(tail);
    (MailboxType::Coe, v)
}

fn segment(service: CoeService, cmd: SdoCommand, data: &[u8]) -> (MailboxType, Vec<u8>) {
    let mut v = CoeHeader { number: 0, service }.encode().to_vec();
    v.push(cmd.encode());
    v.extend_from_slice(data);
    if data.len() < 7 {
        v.resize(CoeHeader::LEN + 1 + 7, 0);
    }
    (MailboxType::Coe, v)
}

fn unused(n: usize) -> u8 {
    if n < 7 { (7 - n) as u8 } else { 0 }
}

fn init_capacity(mailbox_len: usize) -> usize {
    mailbox_len - MAILBOX_HEADER_LEN - CoeHeader::LEN - SdoHeader::LEN
}

fn segment_capacity(mailbox_len: usize) -> usize {
    mailbox_len - MAILBOX_HEADER_LEN - CoeHeader::LEN - 1
}

fn parse_sdo(body: &[u8], request: bool) -> Option<(SdoCommand, Option<SdoHeader>)> {
    let c = CoeHeader::parse(body).ok()?;
    let want = if request { CoeService::SdoRequest } else { CoeService::SdoResponse };
    if c.service != want {
        return None;
    }
    let cmd = SdoCommand::decode(*body.get(CoeHeader::LEN)?, request);
    Some((cmd, SdoHeader::parse(&body[CoeHeader::LEN..]).ok()))
}

/// The side that has the data: a download client (master) or an upload server (slave).
pub(crate) struct SdoSender {
    src: Source,
    total: u64,
    init_cap: usize,
    seg_cap: usize,
    client: bool,
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

impl SdoSender {
    /// `client`: master download (sends the initiate request); otherwise slave upload (waits
    /// for it).
    pub(crate) fn new(src: Source, total: u64, mailbox_len: usize, client: bool) -> Self {
        let state = if client { SdoTx::Init } else { SdoTx::WaitInit };
        Self {
            src,
            total,
            init_cap: init_capacity(mailbox_len),
            seg_cap: segment_capacity(mailbox_len),
            client,
            state,
            toggle: false,
            last: false,
        }
    }

    fn after_init(&mut self) {
        self.state = if self.src.remaining() == 0 { SdoTx::Done } else { SdoTx::Segment };
        self.toggle = false;
    }
}

impl Endpoint for SdoSender {
    fn next(&mut self) -> Option<(MailboxType, Vec<u8>)> {
        match self.state {
            SdoTx::Init if self.client => {
                let first = self.src.take(self.init_cap);
                self.state = SdoTx::WaitInit;
                let cmd = SdoCommand::InitiateDownloadRequest {
                    complete_access: false,
                    expedited: false,
                    size_indicated: true,
                    unused: 0,
                };
                Some(sdo(CoeService::SdoRequest, cmd, self.total as u32, &first))
            }
            SdoTx::Init => {
                // Server: answer the initiate upload request with the size and the first chunk.
                let first = self.src.take(self.init_cap);
                self.after_init();
                if self.state == SdoTx::Segment {
                    self.state = SdoTx::WaitSegment; // wait for the client's segment request
                }
                let cmd = SdoCommand::InitiateUploadResponse {
                    complete_access: false,
                    expedited: false,
                    size_indicated: true,
                    unused: 0,
                };
                Some(sdo(CoeService::SdoResponse, cmd, self.total as u32, &first))
            }
            SdoTx::Segment => {
                let data = self.src.take(self.seg_cap);
                self.last = self.src.remaining() == 0;
                let n = data.len();
                if self.client {
                    self.state = SdoTx::WaitSegment;
                    let cmd =
                        SdoCommand::DownloadSegmentRequest { toggle: self.toggle, unused: unused(n), last: self.last };
                    Some(segment(CoeService::SdoRequest, cmd, &data))
                } else {
                    self.state = if self.last { SdoTx::Done } else { SdoTx::WaitSegment };
                    let cmd =
                        SdoCommand::UploadSegmentResponse { toggle: self.toggle, unused: unused(n), last: self.last };
                    self.toggle = !self.toggle;
                    Some(segment(CoeService::SdoResponse, cmd, &data))
                }
            }
            _ => None,
        }
    }
    fn on(&mut self, t: MailboxType, body: &[u8], _: u64) {
        if t != MailboxType::Coe {
            return;
        }
        if self.client {
            let Some((cmd, _)) = parse_sdo(body, false) else { return };
            match (self.state, cmd) {
                (SdoTx::WaitInit, SdoCommand::InitiateDownloadResponse) => self.after_init(),
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
        } else {
            let Some((cmd, _)) = parse_sdo(body, true) else { return };
            match (self.state, cmd) {
                (SdoTx::WaitInit, SdoCommand::InitiateUploadRequest { .. }) => self.state = SdoTx::Init,
                (SdoTx::WaitSegment, SdoCommand::UploadSegmentRequest { toggle }) if toggle == self.toggle => {
                    self.state = SdoTx::Segment;
                }
                _ => {}
            }
        }
    }
    fn wants_input(&self) -> bool {
        matches!(self.state, SdoTx::WaitInit | SdoTx::WaitSegment)
    }
    fn done(&self) -> bool {
        self.state == SdoTx::Done
    }
}

/// The side that receives the data: a download server (slave) or an upload client (master).
pub(crate) struct SdoReceiver {
    sink: Sink,
    client: bool,
    pending: VecDeque<(MailboxType, Vec<u8>)>,
    toggle: bool,
    waiting: bool,
    finished: bool,
}

impl SdoReceiver {
    /// `client`: master upload (sends the initiate request); otherwise slave download.
    pub(crate) fn new(sink: Sink, client: bool) -> Self {
        let mut pending = VecDeque::new();
        if client {
            pending.push_back(sdo(
                CoeService::SdoRequest,
                SdoCommand::InitiateUploadRequest { complete_access: false },
                0,
                &[],
            ));
        }
        Self { sink, client, pending, toggle: false, waiting: client, finished: false }
    }
}

impl Endpoint for SdoReceiver {
    fn next(&mut self) -> Option<(MailboxType, Vec<u8>)> {
        self.pending.pop_front()
    }
    fn on(&mut self, t: MailboxType, body: &[u8], now: u64) {
        if t != MailboxType::Coe {
            return;
        }
        let Some((cmd, hdr)) = parse_sdo(body, !self.client) else { return };
        let initial = |me: &mut Self, size: u32| {
            let n = body.len().saturating_sub(CoeHeader::LEN + SdoHeader::LEN).min(size as usize);
            me.sink.deliver(&body[CoeHeader::LEN + SdoHeader::LEN..][..n], now);
            me.toggle = false;
            me.finished = me.sink.complete();
        };
        let seg = |me: &mut Self, unused: u8| {
            let n = segment_data_len(body.len() as u16, unused);
            me.sink.deliver(&body[CoeHeader::LEN + 1..][..n], now);
        };
        match cmd {
            SdoCommand::InitiateDownloadRequest { .. } if !self.client => {
                initial(self, hdr.map_or(0, |h| h.data));
                self.pending.push_back(sdo(CoeService::SdoResponse, SdoCommand::InitiateDownloadResponse, 0, &[]));
            }
            SdoCommand::DownloadSegmentRequest { toggle, unused, last } if !self.client && toggle == self.toggle => {
                seg(self, unused);
                self.finished = last;
                let resp = SdoCommand::DownloadSegmentResponse { toggle };
                self.pending.push_back(segment(CoeService::SdoResponse, resp, &[]));
                self.toggle = !self.toggle;
            }
            SdoCommand::InitiateUploadResponse { .. } if self.client => {
                initial(self, hdr.map_or(0, |h| h.data));
                self.waiting = false;
                if !self.finished {
                    let req = SdoCommand::UploadSegmentRequest { toggle: self.toggle };
                    self.pending.push_back(segment(CoeService::SdoRequest, req, &[]));
                    self.waiting = true;
                }
            }
            SdoCommand::UploadSegmentResponse { toggle, unused, last } if self.client && toggle == self.toggle => {
                seg(self, unused);
                self.finished = last;
                self.toggle = !self.toggle;
                self.waiting = !last;
                if !last {
                    let req = SdoCommand::UploadSegmentRequest { toggle: self.toggle };
                    self.pending.push_back(segment(CoeService::SdoRequest, req, &[]));
                }
            }
            _ => {}
        }
    }
    fn wants_input(&self) -> bool {
        self.waiting || (!self.client && !self.finished)
    }
    fn done(&self) -> bool {
        self.finished && self.pending.is_empty()
    }
    fn sink(&self) -> Option<&Sink> {
        Some(&self.sink)
    }
}

// ---------------------------------------------------------------------------------------------
// PDO channel
// ---------------------------------------------------------------------------------------------

/// Sends sequence-numbered chunks in a process data area, one per update.
#[derive(Debug, Clone)]
pub(crate) struct ChannelSender {
    seed: u64,
    total: u64,
    capacity: usize,
    window: u32,
    retransmit_after: u32,
    total_chunks: u32,
    next_seq: u32,
    acked: u32,
    stall: u32,
    pub(crate) retransmitted: u64,
    pub(crate) chunk_updates: u64,
    last_sent: u32,
}

impl ChannelSender {
    pub(crate) fn new(seed: u64, total: u64, bytes_per_update: usize, window: u32, retransmit_after: u32) -> Self {
        let capacity = bytes_per_update - PDO_CHUNK_HEADER;
        let total_chunks = total.div_ceil(capacity as u64) as u32;
        Self {
            seed,
            total,
            capacity,
            window,
            retransmit_after,
            total_chunks,
            next_seq: 1,
            acked: 1,
            stall: 0,
            retransmitted: 0,
            chunk_updates: 0,
            last_sent: 0,
        }
    }

    /// Write this update's chunk into `out` (header + data).
    pub(crate) fn fill(&mut self, out: &mut [u8]) {
        out.fill(0);
        if self.done() {
            return;
        }
        if self.next_seq < self.acked {
            self.next_seq = self.acked;
        }
        let seq = if self.next_seq <= self.total_chunks && self.next_seq < self.acked + self.window {
            self.next_seq += 1;
            self.chunk_updates += 1;
            self.next_seq - 1
        } else {
            self.stall += 1;
            if self.stall >= self.retransmit_after {
                self.retransmitted += (self.next_seq - self.acked) as u64;
                self.next_seq = self.acked;
                self.stall = 0;
            }
            self.last_sent
        };
        if seq == 0 {
            return;
        }
        self.last_sent = seq;
        let offset = (seq - 1) as u64 * self.capacity as u64;
        let n = (self.total - offset).min(self.capacity as u64) as usize;
        out[..4].copy_from_slice(&seq.to_le_bytes());
        out[4..6].copy_from_slice(&(n as u16).to_le_bytes());
        payload_bytes(self.seed, offset, &mut out[PDO_CHUNK_HEADER..PDO_CHUNK_HEADER + n]);
    }

    /// The receiver's acknowledgement field (next expected sequence).
    pub(crate) fn on_ack(&mut self, ack: &[u8]) {
        if ack.len() < PDO_ACK_BYTES {
            return;
        }
        let a = u32::from_le_bytes([ack[0], ack[1], ack[2], ack[3]]);
        if a > self.acked && a <= self.total_chunks + 1 {
            self.acked = a;
            self.stall = 0;
        }
    }

    pub(crate) fn done(&self) -> bool {
        self.acked > self.total_chunks
    }
}

/// Receives chunks in order and acknowledges them.
#[derive(Debug, Clone)]
pub(crate) struct ChannelReceiver {
    pub(crate) sink: Sink,
    expected: u32,
}

impl ChannelReceiver {
    pub(crate) fn new(sink: Sink) -> Self {
        Self { sink, expected: 1 }
    }

    pub(crate) fn on_chunk(&mut self, buf: &[u8], now: u64) {
        if buf.len() < PDO_CHUNK_HEADER {
            return;
        }
        let seq = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        let n = u16::from_le_bytes([buf[4], buf[5]]) as usize;
        if seq != self.expected || PDO_CHUNK_HEADER + n > buf.len() {
            return;
        }
        self.sink.deliver(&buf[PDO_CHUNK_HEADER..PDO_CHUNK_HEADER + n], now);
        self.expected += 1;
    }

    pub(crate) fn fill_ack(&self, out: &mut [u8]) {
        out.fill(0);
        out[..PDO_ACK_BYTES].copy_from_slice(&self.expected.to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Case = (&'static str, Box<dyn Endpoint>, Box<dyn Endpoint>);

    /// Run two endpoints against each other with perfect delivery.
    fn pump(a: &mut dyn Endpoint, b: &mut dyn Endpoint) -> u32 {
        let mut msgs = 0;
        for t in 0..1_000_000u64 {
            let mut moved = false;
            if let Some((ty, m)) = a.next() {
                b.on(ty, &m, t);
                moved = true;
                msgs += 1;
            }
            if let Some((ty, m)) = b.next() {
                a.on(ty, &m, t);
                moved = true;
                msgs += 1;
            }
            if !moved {
                break;
            }
        }
        msgs
    }

    #[test]
    fn every_protocol_delivers_boundary_sizes() {
        let mbx = 128;
        for len in [0u64, 1, 6, 7, 8, 100, 112, 113, 116, 117, 232, 1000, 1472, 1473, 10_000] {
            let seed = 9;
            let s = || Source::new(seed, len);
            let k = || Sink::new(seed, len, 0);
            let cases: Vec<Case> = vec![
                ("eoe", Box::new(EoeSender::new(s(), 1500, mbx, true)), Box::new(EoeReceiver::new(k()))),
                ("foe write", Box::new(FoeSender::new(s(), mbx, true)), Box::new(FoeReceiver::new(k(), mbx, false))),
                ("foe read", Box::new(FoeReceiver::new(k(), mbx, true)), Box::new(FoeSender::new(s(), mbx, false))),
                ("sdo download", Box::new(SdoSender::new(s(), len, mbx, true)), Box::new(SdoReceiver::new(k(), false))),
                ("sdo upload", Box::new(SdoReceiver::new(k(), true)), Box::new(SdoSender::new(s(), len, mbx, false))),
            ];
            for (name, mut a, mut b) in cases {
                pump(a.as_mut(), b.as_mut());
                let sink = a.sink().or(b.sink()).unwrap();
                assert!(sink.complete() && !sink.corrupt, "{name} {len}: got {}", sink.got);
                assert_eq!(sink.sha256(), payload_sha256(seed, len), "{name} {len}");
                assert!(a.done() && b.done(), "{name} {len}: done {} {}", a.done(), b.done());
            }
        }
    }

    #[test]
    fn channel_survives_lost_updates() {
        let mut tx = ChannelSender::new(3, 5000, 64, 2, 4);
        let mut rx = ChannelReceiver::new(Sink::new(3, 5000, 0));
        let (mut out, mut ack) = (vec![0u8; 64], vec![0u8; 4]);
        for k in 0..10_000u64 {
            tx.fill(&mut out);
            if k % 5 != 3 {
                rx.on_chunk(&out, k); // every fifth update is overwritten before it is read
            }
            rx.fill_ack(&mut ack);
            tx.on_ack(&ack);
            if tx.done() {
                break;
            }
        }
        assert!(tx.done() && rx.sink.complete() && !rx.sink.corrupt);
        assert!(tx.retransmitted > 0);
    }
}
