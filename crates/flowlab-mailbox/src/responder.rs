//! The slave application's side of the mailbox: FoE, CoE SDO and EoE responders.
//!
//! A [`Responder`] takes each message the master put into "mailbox out" and returns the
//! messages the slave answers with, in order. It knows nothing of time or of SyncManagers:
//! the lab master's virtual slave runs it in virtual time behind a model of the mailbox
//! buffers, and the emulated ESC (`flowlab-slave`) runs it as soon as a message arrives.
//!
//! * FoE: a write request opens a file that DATA packets fill (ACK per packet, optionally BUSY
//!   every n-th packet); a read request is answered with [`ResponderConfig::upload`].
//! * CoE: SDO download (normal, segmented) into the transfer object and upload (expedited,
//!   normal, segmented) from it; further objects ([`ResponderConfig::objects`]) are read-only.
//! * EoE: fragments are reassembled into frames; with [`ResponderConfig::eoe_send`] the slave
//!   sends [`ResponderConfig::upload`] as UDP frames.
//! * FlowLab VoE ([`crate::voe`]): OPEN to the slave is answered READY; DATA messages are
//!   acknowledged once per window and the payload ends with DONE (length and SHA-256); OPEN
//!   from the slave is answered READY and a window of DATA messages from
//!   [`ResponderConfig::upload`], continued after each ACK. VoE of another vendor is refused
//!   with a mailbox error reply.
//! * Settings ([`ResponderConfig::settings`]): objects the master may write as well as read.
//!
//! ```
//! use ecat_wire::{FoeOpcode, MailboxType};
//! use flowlab_mailbox::msg;
//! use flowlab_mailbox::responder::{Responder, ResponderConfig};
//!
//! let mut r = Responder::new(ResponderConfig::default(), 128, 128);
//! let wrq = msg::foe(FoeOpcode::Write, 0, b"a.bin");
//! let replies = r.take_message(&msg::mailbox_buffer(MailboxType::Foe, 1, 0, &wrq, 128));
//! assert_eq!(replies[0].kind, MailboxType::Foe); // ACK 0
//! let data = msg::foe(FoeOpcode::Data, 1, b"hello");
//! r.take_message(&msg::mailbox_buffer(MailboxType::Foe, 2, 0, &data, 128));
//! assert_eq!(r.log().files, vec![("a.bin".to_string(), b"hello".to_vec())]);
//! ```

use std::collections::BTreeMap;

use ecat_wire::{
    CoeHeader, CoeService, FoeHeader, FoeOpcode, MAILBOX_HEADER_LEN, MailboxErrorReply, MailboxHeader, MailboxType,
    SdoCommand, SdoHeader, segment_data_len,
};

use crate::msg::{self, EoeAddresses, EoeReassembler, eoe_fragments, foe, udp_frame};
use crate::voe;

/// SDO abort: object does not exist in the object dictionary.
const ABORT_NO_OBJECT: u32 = 0x0602_0000;
/// SDO abort: subindex does not exist.
const ABORT_NO_SUBINDEX: u32 = 0x0609_0011;
/// SDO abort: attempt to write a read-only object.
const ABORT_READ_ONLY: u32 = 0x0601_0002;
/// SDO abort: toggle bit not alternated.
const ABORT_TOGGLE: u32 = 0x0503_0000;
/// SDO abort: client/server command specifier not valid or unknown.
const ABORT_COMMAND: u32 = 0x0504_0001;

/// Settings of a [`Responder`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponderConfig {
    /// Every n-th FoE DATA packet of a write is answered BUSY the first time.
    pub busy_every: Option<u32>,
    /// Drop a message whose counter equals the previous message's (not 0), as SOES does.
    pub drop_repeated_counter: bool,
    /// Counter of the last message the slave saw before (e.g. from the master's
    /// initialisation); 0 for none.
    pub previous_counter: u8,
    /// Index of the transfer object (SDO download target and upload source).
    pub sdo_index: u16,
    /// Its subindex.
    pub sdo_subindex: u8,
    /// Further objects, read-only: `(index, subindex) → value`.
    pub objects: BTreeMap<(u16, u8), Vec<u8>>,
    /// What the slave sends: the file for an FoE read, the object for an SDO upload, the UDP
    /// payload for an EoE send.
    pub upload: Vec<u8>,
    /// Send `upload` over EoE from the start.
    pub eoe_send: bool,
    /// IP MTU of the slave's EoE frames, bytes.
    pub eoe_mtu: usize,
    /// Addresses of the slave's EoE frames.
    pub eoe_addresses: EoeAddresses,
    /// Largest FlowLab VoE window the slave grants.
    pub voe_max_window: u16,
    /// Objects the master may write (any size up to the mailbox) and read: `(index, subindex)
    /// → value`. A write replaces the value and is logged in [`ResponderLog::settings`].
    pub settings: BTreeMap<(u16, u8), Vec<u8>>,
}

impl Default for ResponderConfig {
    fn default() -> Self {
        Self {
            busy_every: None,
            drop_repeated_counter: true,
            previous_counter: 0,
            sdo_index: msg::SDO_INDEX,
            sdo_subindex: msg::SDO_SUBINDEX,
            objects: BTreeMap::new(),
            upload: Vec::new(),
            eoe_send: false,
            eoe_mtu: 1500,
            eoe_addresses: EoeAddresses::default(),
            voe_max_window: voe::MAX_WINDOW,
            settings: BTreeMap::new(),
        }
    }
}

/// What the responder received.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResponderLog {
    /// Files written with FoE, in order: (name, contents).
    pub files: Vec<(String, Vec<u8>)>,
    /// Objects downloaded with CoE into the transfer object, in order.
    pub objects: Vec<Vec<u8>>,
    /// Ethernet frames received over EoE.
    pub eoe_frames: Vec<Vec<u8>>,
    /// EoE fragments that did not continue a frame.
    pub eoe_broken: u64,
    /// Messages taken.
    pub messages_in: u64,
    /// Messages dropped because their counter repeated the previous one.
    pub duplicates_dropped: u64,
    /// Payloads received over FlowLab VoE, in order.
    pub voe_received: Vec<Vec<u8>>,
    /// Uploads sent over FlowLab VoE to the end.
    pub voe_sent: u64,
    /// Settings the master wrote, in order: (index, subindex, value).
    pub settings: Vec<(u16, u8, Vec<u8>)>,
}

/// A message the slave sends: its mailbox type and the data after the mailbox header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// Mailbox type.
    pub kind: MailboxType,
    /// Data after the mailbox header.
    pub body: Vec<u8>,
}

impl Message {
    fn new(kind: MailboxType, body: Vec<u8>) -> Self {
        Self { kind, body }
    }
}

enum Foe {
    Idle,
    Writing { name: String, expected: u32, data: Vec<u8>, busy_sent: bool },
    Reading { packet: u32, offset: usize, last: bool },
}

enum Voe {
    Idle,
    Receiving { length: u64, window: u64, data: Vec<u8>, count: u64 },
    Sending { window: u64, offset: usize, queued: u64, acked: u64, messages: u64 },
}

enum Sdo {
    Idle,
    Download { toggle: bool, data: Vec<u8> },
    Upload { data: Vec<u8>, offset: usize, toggle: bool },
}

/// The slave application behind a mailbox (see the [module docs](self)).
pub struct Responder {
    cfg: ResponderConfig,
    out_len: usize,
    in_len: usize,
    last_in_counter: u8,
    foe: Foe,
    data_packets: u32,
    sdo: Sdo,
    eoe_rx: EoeReassembler,
    eoe_tx_offset: usize,
    eoe_frame_no: u8,
    voe: Voe,
    /// The last DONE, repeated when the master writes DATA again (its DONE was lost).
    voe_last_done: Option<Vec<u8>>,
    log: ResponderLog,
}

impl std::fmt::Debug for Responder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Responder").field("cfg", &self.cfg).finish_non_exhaustive()
    }
}

impl Responder {
    /// A responder behind a "mailbox out" of `out_len` bytes and a "mailbox in" of `in_len`
    /// bytes (they decide how much data each message carries).
    pub fn new(cfg: ResponderConfig, out_len: usize, in_len: usize) -> Self {
        Self {
            last_in_counter: cfg.previous_counter,
            cfg,
            out_len,
            in_len,
            foe: Foe::Idle,
            data_packets: 0,
            sdo: Sdo::Idle,
            eoe_rx: EoeReassembler::default(),
            eoe_tx_offset: 0,
            eoe_frame_no: 0,
            voe: Voe::Idle,
            voe_last_done: None,
            log: ResponderLog::default(),
        }
    }

    /// Carry on with the mailbox sizes and EoE send of a reconfigured slave: the log and the
    /// settings stay, transfers start over (as after INIT).
    pub fn reconfigure(&mut self, out_len: usize, in_len: usize, eoe_send: bool) {
        self.out_len = out_len;
        self.in_len = in_len;
        self.cfg.eoe_send = eoe_send;
        self.restart();
    }

    /// The current value of a setting.
    pub fn setting(&self, index: u16, subindex: u8) -> Option<&[u8]> {
        self.cfg.settings.get(&(index, subindex)).map(Vec::as_slice)
    }

    /// Set a setting's value (as the slave's own correction, not logged as a write).
    pub fn put_setting(&mut self, key: (u16, u8), value: Vec<u8>) {
        self.cfg.settings.insert(key, value);
    }

    /// The settings.
    pub fn config(&self) -> &ResponderConfig {
        &self.cfg
    }

    /// What was received so far.
    pub fn log(&self) -> ResponderLog {
        ResponderLog { eoe_broken: self.eoe_rx.broken, ..self.log.clone() }
    }

    /// What was received since the last call, handed over and forgotten here: a long-running
    /// slave keeps neither the payloads nor the time to copy them.
    pub fn drain_log(&mut self) -> ResponderLog {
        let broken = std::mem::take(&mut self.eoe_rx.broken);
        ResponderLog { eoe_broken: broken, ..std::mem::take(&mut self.log) }
    }

    /// Start over, as after the slave went back to INIT: abandon open FoE and SDO transfers
    /// and EoE frames, forget the last message counter, and restart an EoE send from its
    /// first byte. The log is kept.
    pub fn restart(&mut self) {
        self.foe = Foe::Idle;
        self.sdo = Sdo::Idle;
        self.voe = Voe::Idle;
        self.voe_last_done = None;
        self.last_in_counter = 0;
        let broken = self.eoe_rx.broken;
        self.eoe_rx = EoeReassembler::default();
        self.eoe_rx.broken = broken;
        self.eoe_tx_offset = 0;
        self.eoe_frame_no = 0;
    }

    /// Whether an EoE send has frames left.
    pub fn eoe_pending(&self) -> bool {
        self.cfg.eoe_send && self.eoe_tx_offset < self.cfg.upload.len()
    }

    /// The fragments of the next EoE frame of the send, or nothing when it is complete.
    pub fn next_eoe_frame(&mut self) -> Vec<Message> {
        if !self.eoe_pending() {
            return Vec::new();
        }
        let end = (self.eoe_tx_offset + self.cfg.eoe_mtu.saturating_sub(28).max(1)).min(self.cfg.upload.len());
        let frame = udp_frame(&self.cfg.upload[self.eoe_tx_offset..end], &self.cfg.eoe_addresses, false);
        self.eoe_tx_offset = end;
        self.eoe_frame_no = msg::next_eoe_frame_no(self.eoe_frame_no);
        let cap = msg::eoe_capacity(self.in_len);
        eoe_fragments(&frame, cap, self.eoe_frame_no).into_iter().map(|f| Message::new(MailboxType::Eoe, f)).collect()
    }

    /// Take one "mailbox out" buffer (header and data) and return the answers.
    pub fn take_message(&mut self, buf: &[u8]) -> Vec<Message> {
        let Ok(h) = MailboxHeader::parse(buf) else { return Vec::new() };
        if self.cfg.drop_repeated_counter && h.counter != 0 && h.counter == self.last_in_counter {
            self.log.duplicates_dropped += 1;
            return Vec::new();
        }
        self.last_in_counter = h.counter;
        self.log.messages_in += 1;
        let end = (MAILBOX_HEADER_LEN + h.length as usize).min(buf.len());
        let body = &buf[MAILBOX_HEADER_LEN..end];
        match h.mailbox_type {
            MailboxType::Foe => self.on_foe(body).into_iter().collect(),
            MailboxType::Coe => self.on_coe(body).into_iter().collect(),
            MailboxType::Eoe => {
                if let Some(frame) = self.eoe_rx.push(body) {
                    self.log.eoe_frames.push(frame);
                }
                Vec::new()
            }
            MailboxType::Voe if voe::parse(body).is_some() => self.on_voe(body),
            _ => {
                // Mailbox error reply: protocol not supported.
                let e = MailboxErrorReply { service: 1, detail: 2 };
                vec![Message::new(MailboxType::Error, e.encode().to_vec())]
            }
        }
    }

    fn on_foe(&mut self, body: &[u8]) -> Option<Message> {
        let h = FoeHeader::parse(body).ok()?;
        let data = &body[FoeHeader::LEN..];
        let cap_out = msg::foe_capacity(self.out_len);
        let cap_in = msg::foe_capacity(self.in_len);
        let reply = |op, value, tail: &[u8]| Some(Message::new(MailboxType::Foe, foe(op, value, tail)));
        match (h.opcode, &mut self.foe) {
            (FoeOpcode::Write, _) => {
                let name = String::from_utf8_lossy(data).into_owned();
                self.foe = Foe::Writing { name, expected: 1, data: Vec::new(), busy_sent: false };
                reply(FoeOpcode::Ack, 0, &[])
            }
            (FoeOpcode::Data, Foe::Writing { name, expected, data: file, busy_sent }) if h.value == *expected => {
                self.data_packets += 1;
                if let Some(n) = self.cfg.busy_every
                    && n > 0
                    && self.data_packets.is_multiple_of(n)
                    && !*busy_sent
                {
                    *busy_sent = true;
                    return reply(FoeOpcode::Busy, 0, &[]);
                }
                *busy_sent = false;
                file.extend_from_slice(data);
                *expected += 1;
                if data.len() < cap_out {
                    let done = (std::mem::take(name), std::mem::take(file));
                    self.log.files.push(done);
                    self.foe = Foe::Idle;
                }
                reply(FoeOpcode::Ack, h.value, &[])
            }
            (FoeOpcode::Data, Foe::Writing { expected, .. }) if h.value + 1 == *expected => {
                reply(FoeOpcode::Ack, h.value, &[])
            }
            (FoeOpcode::Read, _) => {
                let n = cap_in.min(self.cfg.upload.len());
                self.foe = Foe::Reading { packet: 1, offset: n, last: n < cap_in };
                reply(FoeOpcode::Data, 1, &self.cfg.upload[..n])
            }
            (FoeOpcode::Ack, Foe::Reading { packet, offset, last }) if h.value == *packet => {
                if *last {
                    self.foe = Foe::Idle;
                    return None;
                }
                let end = (*offset + cap_in).min(self.cfg.upload.len());
                *packet += 1;
                *last = end - *offset < cap_in;
                let (p, o) = (*packet, *offset);
                *offset = end;
                reply(FoeOpcode::Data, p, &self.cfg.upload[o..end])
            }
            _ => reply(FoeOpcode::Error, 0x8004, b"illegal"),
        }
    }

    fn on_voe(&mut self, body: &[u8]) -> Vec<Message> {
        let Some(m) = voe::parse(body) else { return Vec::new() };
        let reply = |b: Vec<u8>| Message::new(MailboxType::Voe, b);
        let cap_in = voe::capacity(self.in_len) as u64;
        match (m.op, &mut self.voe) {
            (voe::Op::Open, _) => {
                self.voe_last_done = None;
                let Some((dir, window, length)) = voe::parse_open(m.tail) else {
                    return vec![reply(voe::abort(voe::ABORT_PROTOCOL, "malformed OPEN"))];
                };
                let window = window.clamp(1, self.cfg.voe_max_window.clamp(1, voe::MAX_WINDOW));
                match dir {
                    voe::Direction::ToSlave => {
                        self.voe = Voe::Receiving { length, window: window.into(), data: Vec::new(), count: 0 };
                        let mut out = vec![reply(voe::ready(window, length))];
                        if length == 0 {
                            out.extend(self.voe_finish_receive());
                        }
                        out
                    }
                    voe::Direction::FromSlave => {
                        let len = self.cfg.upload.len() as u64;
                        let messages = len.div_ceil(cap_in.max(1));
                        self.voe = Voe::Sending { window: window.into(), offset: 0, queued: 0, acked: 0, messages };
                        let mut out = vec![reply(voe::ready(window, len))];
                        out.extend(self.voe_fill_window());
                        if messages == 0 {
                            self.voe = Voe::Idle;
                            self.log.voe_sent += 1;
                        }
                        out
                    }
                }
            }
            (voe::Op::Data, Voe::Receiving { length, window, data, count }) => {
                if voe::classify(*count, m.seq) != voe::Arrival::Next {
                    // A repeat (the master's reply timeout) or a gap: say again what is next.
                    return vec![reply(voe::encode(voe::Op::Ack, *count as u32, &[]))];
                }
                let room = (*length - data.len() as u64) as usize;
                data.extend_from_slice(&m.tail[..m.tail.len().min(room)]);
                *count += 1;
                if data.len() as u64 >= *length {
                    self.voe_finish_receive()
                } else if (*count).is_multiple_of(*window) {
                    vec![reply(voe::encode(voe::Op::Ack, *count as u32, &[]))]
                } else {
                    Vec::new()
                }
            }
            (voe::Op::Ack, Voe::Sending { acked, queued, offset, messages, .. }) => {
                let n = voe::unwrap_seq(*acked, m.seq);
                if n > *queued {
                    return Vec::new();
                }
                *acked = n;
                if n >= *messages {
                    self.voe = Voe::Idle;
                    self.log.voe_sent += 1;
                    return Vec::new();
                }
                if n < *queued {
                    // The master misses message n: go back and send again from there.
                    *queued = n;
                    *offset = n as usize * voe::capacity(self.in_len).max(1);
                }
                self.voe_fill_window()
            }
            (voe::Op::Abort, _) => {
                self.voe = Voe::Idle;
                Vec::new()
            }
            (voe::Op::Data, Voe::Idle) => self.voe_last_done.iter().map(|d| reply(d.clone())).collect(),
            (voe::Op::Ack, Voe::Idle) => Vec::new(),
            _ => vec![reply(voe::abort(voe::ABORT_PROTOCOL, "unexpected message"))],
        }
    }

    /// DONE for the payload received, and back to idle.
    fn voe_finish_receive(&mut self) -> Vec<Message> {
        let Voe::Receiving { data, .. } = std::mem::replace(&mut self.voe, Voe::Idle) else { return Vec::new() };
        let done = voe::done(data.len() as u64, &voe::sha256(&data));
        self.voe_last_done = Some(done.clone());
        self.log.voe_received.push(data);
        vec![Message::new(MailboxType::Voe, done)]
    }

    /// The DATA messages the window allows now.
    fn voe_fill_window(&mut self) -> Vec<Message> {
        let cap = voe::capacity(self.in_len).max(1);
        let Voe::Sending { window, offset, queued, acked, messages } = &mut self.voe else { return Vec::new() };
        let mut out = Vec::new();
        while *queued < *messages && *queued < *acked + *window {
            let end = (*offset + cap).min(self.cfg.upload.len());
            out.push(Message::new(
                MailboxType::Voe,
                voe::encode(voe::Op::Data, *queued as u32, &self.cfg.upload[*offset..end]),
            ));
            *offset = end;
            *queued += 1;
        }
        out
    }

    /// The value of a read-only object or a setting, or the abort code.
    fn object(&self, index: u16, subindex: u8) -> Result<&[u8], u32> {
        if let Some(v) = self.cfg.objects.get(&(index, subindex)).or_else(|| self.cfg.settings.get(&(index, subindex)))
        {
            return Ok(v);
        }
        let known = |(i, _): &(u16, u8)| *i == index;
        if self.cfg.objects.keys().any(known) || self.cfg.settings.keys().any(known) {
            Err(ABORT_NO_SUBINDEX)
        } else {
            Err(ABORT_NO_OBJECT)
        }
    }

    /// Store a setting the master wrote.
    fn write_setting(&mut self, index: u16, subindex: u8, value: Vec<u8>) {
        self.log.settings.push((index, subindex, value.clone()));
        self.cfg.settings.insert((index, subindex), value);
    }

    fn on_coe(&mut self, body: &[u8]) -> Option<Message> {
        let c = CoeHeader::parse(body).ok()?;
        if c.service != CoeService::SdoRequest || body.len() < CoeHeader::LEN + 1 {
            return None;
        }
        let cmd = SdoCommand::decode(body[CoeHeader::LEN], true);
        let hdr = SdoHeader::parse(&body[CoeHeader::LEN..]).ok();
        let (ix, sub) = hdr.map_or((self.cfg.sdo_index, self.cfg.sdo_subindex), |h| (h.index, h.subindex));
        let transfer_object = ix == self.cfg.sdo_index && sub == self.cfg.sdo_subindex;
        let abort = |code: u32| msg::sdo(CoeService::SdoResponse, SdoCommand::Abort, ix, sub, code, &[]);
        let cap_init = msg::sdo_initiate_capacity(self.in_len);
        let cap_seg = msg::sdo_segment_capacity(self.in_len);
        let setting = self.cfg.settings.contains_key(&(ix, sub));
        let m = match (cmd, &mut self.sdo) {
            (SdoCommand::InitiateDownloadRequest { expedited: true, size_indicated, unused, .. }, _) if setting => {
                let n = if size_indicated { 4 - usize::from(unused.min(3)) } else { 4 };
                let value = hdr.map_or(0, |h| h.data).to_le_bytes()[..n].to_vec();
                self.write_setting(ix, sub, value);
                msg::sdo(CoeService::SdoResponse, SdoCommand::InitiateDownloadResponse, ix, sub, 0, &[])
            }
            (SdoCommand::InitiateDownloadRequest { expedited: false, .. }, _) if setting => {
                let size = hdr.map_or(0, |h| h.data) as usize;
                let tail = &body[CoeHeader::LEN + SdoHeader::LEN..];
                if tail.len() < size {
                    abort(ABORT_COMMAND)
                } else {
                    self.write_setting(ix, sub, tail[..size].to_vec());
                    msg::sdo(CoeService::SdoResponse, SdoCommand::InitiateDownloadResponse, ix, sub, 0, &[])
                }
            }
            (SdoCommand::InitiateDownloadRequest { .. }, _) if !transfer_object => {
                abort(self.object(ix, sub).map_or_else(|code| code, |_| ABORT_READ_ONLY))
            }
            (SdoCommand::InitiateDownloadRequest { expedited: false, .. }, _) => {
                let size = hdr.map_or(0, |h| h.data) as usize;
                let tail = &body[CoeHeader::LEN + SdoHeader::LEN..];
                let data = tail[..tail.len().min(size)].to_vec();
                if data.len() >= size {
                    self.log.objects.push(data);
                    self.sdo = Sdo::Idle;
                } else {
                    self.sdo = Sdo::Download { toggle: false, data };
                }
                msg::sdo(CoeService::SdoResponse, SdoCommand::InitiateDownloadResponse, ix, sub, 0, &[])
            }
            (SdoCommand::DownloadSegmentRequest { toggle, unused, last }, Sdo::Download { toggle: t, data })
                if toggle == *t =>
            {
                let n = segment_data_len(body.len() as u16, unused).min(body.len() - CoeHeader::LEN - 1);
                data.extend_from_slice(&body[CoeHeader::LEN + 1..][..n]);
                *t = !*t;
                if last {
                    let d = std::mem::take(data);
                    self.log.objects.push(d);
                    self.sdo = Sdo::Idle;
                }
                msg::sdo_segment(CoeService::SdoResponse, SdoCommand::DownloadSegmentResponse { toggle }, &[])
            }
            (SdoCommand::DownloadSegmentRequest { .. }, _) => abort(ABORT_TOGGLE),
            (SdoCommand::InitiateUploadRequest { .. }, _) => {
                let obj = if transfer_object {
                    self.cfg.upload.clone()
                } else {
                    match self.object(ix, sub) {
                        Ok(v) => v.to_vec(),
                        Err(code) => return Some(Message::new(MailboxType::Coe, abort(code))),
                    }
                };
                if !obj.is_empty() && obj.len() <= 4 {
                    let mut d = [0u8; 4];
                    d[..obj.len()].copy_from_slice(&obj);
                    let cmd = SdoCommand::InitiateUploadResponse {
                        complete_access: false,
                        expedited: true,
                        size_indicated: true,
                        unused: (4 - obj.len()) as u8,
                    };
                    self.sdo = Sdo::Idle;
                    msg::sdo(CoeService::SdoResponse, cmd, ix, sub, u32::from_le_bytes(d), &[])
                } else {
                    let n = cap_init.min(obj.len());
                    let cmd = SdoCommand::InitiateUploadResponse {
                        complete_access: false,
                        expedited: false,
                        size_indicated: true,
                        unused: 0,
                    };
                    let m = msg::sdo(CoeService::SdoResponse, cmd, ix, sub, obj.len() as u32, &obj[..n]);
                    self.sdo = Sdo::Upload { data: obj, offset: n, toggle: false };
                    m
                }
            }
            (SdoCommand::UploadSegmentRequest { toggle }, Sdo::Upload { data, offset, toggle: t }) if toggle == *t => {
                let end = (*offset + cap_seg).min(data.len());
                let n = end - *offset;
                let last = end == data.len();
                let cmd = SdoCommand::UploadSegmentResponse { toggle, unused: msg::sdo_unused(n), last };
                let m = msg::sdo_segment(CoeService::SdoResponse, cmd, &data[*offset..end]);
                *offset = end;
                *t = !*t;
                if last {
                    self.sdo = Sdo::Idle;
                }
                m
            }
            _ => abort(ABORT_COMMAND),
        };
        Some(Message::new(MailboxType::Coe, m))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coe_request(cmd: SdoCommand, index: u16, subindex: u8) -> Vec<u8> {
        let body = msg::sdo(CoeService::SdoRequest, cmd, index, subindex, 0, &[]);
        msg::mailbox_buffer(MailboxType::Coe, 1, 0, &body, 128)
    }

    fn upload_request() -> SdoCommand {
        SdoCommand::InitiateUploadRequest { complete_access: false }
    }

    #[test]
    fn read_only_objects_upload_expedited_and_refuse_downloads() {
        let mut cfg = ResponderConfig::default();
        cfg.objects.insert((0x1C12, 0), vec![1]);
        let mut r = Responder::new(cfg, 128, 128);
        let m = r.take_message(&coe_request(upload_request(), 0x1C12, 0));
        let h = SdoHeader::parse(&m[0].body[CoeHeader::LEN..]).unwrap();
        assert_eq!((h.index, h.subindex, h.data), (0x1C12, 0, 1));
        assert!(matches!(
            SdoCommand::decode(h.command, false),
            SdoCommand::InitiateUploadResponse { expedited: true, unused: 3, .. }
        ));

        let abort_code = |m: &[Message]| SdoHeader::parse(&m[0].body[CoeHeader::LEN..]).unwrap().data;
        let m = r.take_message(&msg::mailbox_buffer(
            MailboxType::Coe,
            2,
            0,
            &msg::sdo(CoeService::SdoRequest, upload_request(), 0x1C12, 1, 0, &[]),
            128,
        ));
        assert_eq!(abort_code(&m), ABORT_NO_SUBINDEX);
        let m = r.take_message(&msg::mailbox_buffer(
            MailboxType::Coe,
            3,
            0,
            &msg::sdo(CoeService::SdoRequest, upload_request(), 0x1234, 0, 0, &[]),
            128,
        ));
        assert_eq!(abort_code(&m), ABORT_NO_OBJECT);
        let download = SdoCommand::InitiateDownloadRequest {
            complete_access: false,
            expedited: false,
            size_indicated: true,
            unused: 0,
        };
        let m = r.take_message(&msg::mailbox_buffer(
            MailboxType::Coe,
            4,
            0,
            &msg::sdo(CoeService::SdoRequest, download, 0x1C12, 0, 1, &[0]),
            128,
        ));
        assert_eq!(abort_code(&m), ABORT_READ_ONLY);
    }

    fn voe_buffer(body: &[u8], counter: u8) -> Vec<u8> {
        msg::mailbox_buffer(MailboxType::Voe, counter, 0, body, 128)
    }

    #[test]
    fn voe_to_the_slave_acknowledges_per_window_and_ends_with_its_digest() {
        let mut r = Responder::new(ResponderConfig::default(), 128, 128);
        let payload: Vec<u8> = (0..1000u32).map(|i| i as u8).collect();
        let cap = voe::capacity(128);
        let m = r.take_message(&voe_buffer(&voe::open(voe::Direction::ToSlave, 4, payload.len() as u64), 1));
        let ready = voe::parse(&m[0].body).unwrap();
        assert_eq!((ready.op, voe::parse_ready(ready.tail)), (voe::Op::Ready, Some((4, 1000))));
        let mut counter = 1;
        let mut acks = Vec::new();
        let mut done = None;
        for (n, chunk) in payload.chunks(cap).enumerate() {
            counter = msg::next_counter(counter);
            for reply in r.take_message(&voe_buffer(&voe::encode(voe::Op::Data, n as u32, chunk), counter)) {
                let p = voe::parse(&reply.body).unwrap();
                match p.op {
                    voe::Op::Ack => acks.push(p.seq),
                    voe::Op::Done => done = voe::parse_done(p.tail),
                    op => panic!("unexpected {}", op.name()),
                }
            }
        }
        assert_eq!(acks, vec![4, 8], "one ACK per window of 4, none for the last message");
        assert_eq!(done, Some((1000, voe::sha256(&payload))));
        assert_eq!(r.log().voe_received, vec![payload]);
        // A repeated DATA message is answered with the next expected number, not stored twice.
        // (Counter 0: the slave does not compare it with the previous message's.)
        r.take_message(&voe_buffer(&voe::open(voe::Direction::ToSlave, 4, 500), 0));
        r.take_message(&voe_buffer(&voe::encode(voe::Op::Data, 0, &[1; 114]), 0));
        let again = r.take_message(&voe_buffer(&voe::encode(voe::Op::Data, 0, &[1; 114]), 0));
        assert_eq!(voe::parse(&again[0].body).map(|p| (p.op, p.seq)), Some((voe::Op::Ack, 1)));
    }

    #[test]
    fn voe_from_the_slave_sends_a_window_and_continues_after_each_ack() {
        let upload: Vec<u8> = (0..700u32).map(|i| (i * 7) as u8).collect();
        let cfg = ResponderConfig { upload: upload.clone(), ..ResponderConfig::default() };
        let mut r = Responder::new(cfg, 128, 128);
        let m = r.take_message(&voe_buffer(&voe::open(voe::Direction::FromSlave, 2, 0), 1));
        assert_eq!(voe::parse_ready(voe::parse(&m[0].body).unwrap().tail), Some((2, 700)));
        let data = |m: &[Message]| -> Vec<u8> {
            m.iter()
                .filter_map(|x| voe::parse(&x.body))
                .filter(|p| p.op == voe::Op::Data)
                .flat_map(|p| p.tail.to_vec())
                .collect()
        };
        let mut got = data(&m[1..]);
        assert_eq!(m.len(), 3, "READY and a window of two");
        let mut counter = 1;
        let mut acked = 2;
        while got.len() < upload.len() {
            counter = msg::next_counter(counter);
            let more = r.take_message(&voe_buffer(&voe::encode(voe::Op::Ack, acked, &[]), counter));
            acked += more.len() as u32;
            got.extend(data(&more));
        }
        assert_eq!(got, upload);
        counter = msg::next_counter(counter);
        assert!(r.take_message(&voe_buffer(&voe::encode(voe::Op::Ack, acked, &[]), counter)).is_empty());
        assert_eq!(r.log().voe_sent, 1);
    }

    #[test]
    fn foreign_voe_is_refused_and_settings_are_writable() {
        let mut cfg = ResponderConfig::default();
        cfg.settings.insert((0x2001, 0), 128u16.to_le_bytes().to_vec());
        let mut r = Responder::new(cfg, 128, 128);
        let foreign = ecat_wire::VoeHeader { vendor_id: 7, vendor_type: 1 }.encode();
        assert_eq!(r.take_message(&voe_buffer(&foreign, 1))[0].kind, MailboxType::Error);
        let expedited = SdoCommand::InitiateDownloadRequest {
            complete_access: false,
            expedited: true,
            size_indicated: true,
            unused: 2,
        };
        let body = msg::sdo(CoeService::SdoRequest, expedited, 0x2001, 0, 512, &[]);
        let m = r.take_message(&msg::mailbox_buffer(MailboxType::Coe, 2, 0, &body, 128));
        let h = SdoHeader::parse(&m[0].body[CoeHeader::LEN..]).unwrap();
        assert!(matches!(SdoCommand::decode(h.command, false), SdoCommand::InitiateDownloadResponse));
        assert_eq!(r.setting(0x2001, 0), Some(&512u16.to_le_bytes()[..]));
        assert_eq!(r.log().settings, vec![(0x2001, 0, 512u16.to_le_bytes().to_vec())]);
        let m = r.take_message(&coe_request(upload_request(), 0x2001, 0));
        assert_eq!(SdoHeader::parse(&m[0].body[CoeHeader::LEN..]).unwrap().data, 512);
    }

    #[test]
    fn repeated_counters_are_dropped() {
        let mut r = Responder::new(ResponderConfig::default(), 128, 128);
        let wrq = msg::mailbox_buffer(MailboxType::Foe, 3, 0, &foe(FoeOpcode::Write, 0, b"f"), 128);
        assert_eq!(r.take_message(&wrq).len(), 1);
        assert!(r.take_message(&wrq).is_empty());
        assert_eq!(r.log().duplicates_dropped, 1);
    }
}
