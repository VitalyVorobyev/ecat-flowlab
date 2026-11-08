//! A slave without hardware: ESC mailbox SyncManagers and FoE, CoE and EoE responders, in
//! virtual time.
//!
//! The model follows the ESC's one-buffer mailbox rules: a write into "mailbox out" is
//! accepted (WKC 1) only while the buffer is empty and becomes the slave's when the write
//! reaches its last byte; a read of "mailbox in" returns data (WKC 1) only while the buffer is
//! full and releases it when it reaches the last byte. The slave application takes a message
//! `processing_ns` after it arrived and answers at the same time. Faults (lost frames, BUSY
//! answers, a counter left over from earlier traffic) are optional.
//!
//! It exists to test the transfer engine end to end; nothing it reports is a measurement.

use std::cell::RefCell;
use std::collections::VecDeque;

use ecat_wire::{
    CoeHeader, CoeService, FoeHeader, FoeOpcode, MAILBOX_HEADER_LEN, MailboxErrorReply, MailboxHeader, MailboxType,
    SM_REPEAT_ACK, SM_REPEAT_REQUEST, SM_STATUS_MAILBOX_FULL, SdoCommand, SdoHeader, SyncManagerConfig,
    segment_data_len, sm_activate_register, sm_register,
};

use crate::bus::{BusError, MailboxBus, Reply};
use crate::{MailboxLayout, bus::SM_MAILBOX_IN};
use flowlab_mailbox::msg::{self, EoeAddresses, EoeReassembler, eoe_fragments, foe, udp_frame};

/// Settings of a [`VirtualSlave`].
#[derive(Debug, Clone)]
pub struct VirtualSlaveConfig {
    /// The slave's mailbox (station address, SyncManager 0 and 1).
    pub mailbox: MailboxLayout,
    /// Round trip of one datagram, ns of virtual time.
    pub access_ns: u64,
    /// Round trip of the cyclic exchange, ns; `None`: no process data.
    pub cyclic_ns: Option<u64>,
    /// Time from a message's arrival to the slave taking it and answering, ns.
    pub processing_ns: u64,
    /// Every n-th datagram is lost on its way back (after the slave processed it).
    pub lose_every: Option<u64>,
    /// Every n-th FoE DATA packet of a write is answered BUSY the first time.
    pub busy_every: Option<u32>,
    /// Drop a message whose counter equals the previous message's (not 0), as SOES does.
    pub drop_repeated_counter: bool,
    /// Counter of the last message the slave saw before the transfer (e.g. from the master's
    /// initialisation); 0 for none.
    pub previous_counter: u8,
    /// Object index the SDO server answers for.
    pub sdo_index: u16,
    /// Its subindex.
    pub sdo_subindex: u8,
    /// What the slave sends: the file for an FoE read, the object for an SDO upload, the UDP
    /// payload for an EoE send.
    pub upload: Vec<u8>,
    /// Send `upload` over EoE from the start.
    pub eoe_send: bool,
    /// IP MTU of the slave's EoE frames, bytes.
    pub eoe_mtu: usize,
    /// Addresses of the slave's EoE frames.
    pub eoe_addresses: EoeAddresses,
}

impl Default for VirtualSlaveConfig {
    fn default() -> Self {
        Self {
            mailbox: MailboxLayout { station: 0x1001, out_start: 0x1000, out_len: 128, in_start: 0x1080, in_len: 128 },
            access_ns: 20_000,
            cyclic_ns: None,
            processing_ns: 50_000,
            lose_every: None,
            busy_every: None,
            drop_repeated_counter: true,
            previous_counter: 0,
            sdo_index: msg::SDO_INDEX,
            sdo_subindex: 0,
            upload: Vec::new(),
            eoe_send: false,
            eoe_mtu: 1500,
            eoe_addresses: EoeAddresses::default(),
        }
    }
}

/// What the virtual slave received and did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VirtualSlaveLog {
    /// Files written with FoE, in order: (name, contents).
    pub files: Vec<(String, Vec<u8>)>,
    /// Objects downloaded with CoE, in order.
    pub objects: Vec<Vec<u8>>,
    /// Ethernet frames received over EoE.
    pub eoe_frames: Vec<Vec<u8>>,
    /// EoE fragments that did not continue a frame.
    pub eoe_broken: u64,
    /// Messages taken from "mailbox out".
    pub messages_in: u64,
    /// Messages dropped because their counter repeated the previous one.
    pub duplicates_dropped: u64,
    /// Messages put into "mailbox in".
    pub messages_out: u64,
    /// Repeat requests served.
    pub repeats: u64,
    /// Datagrams addressed to the slave.
    pub accesses: u64,
}

enum Foe {
    Idle,
    Writing { name: String, expected: u32, data: Vec<u8>, busy_sent: bool },
    Reading { packet: u32, offset: usize, last: bool },
}

enum Sdo {
    Idle,
    Download { toggle: bool, data: Vec<u8> },
    Upload { offset: usize, toggle: bool },
}

struct State {
    now: u64,
    datagrams: u64,
    sm0: Option<(Vec<u8>, u64)>,
    sm1: Option<Vec<u8>>,
    sm1_activate: u8,
    sm1_pdi_control: u8,
    outbox: VecDeque<(u64, MailboxType, Vec<u8>)>,
    last_sent: Option<Vec<u8>>,
    out_counter: u8,
    last_in_counter: u8,
    foe: Foe,
    data_packets: u32,
    sdo: Sdo,
    eoe_rx: EoeReassembler,
    eoe_tx_offset: usize,
    eoe_frame_no: u8,
    log: VirtualSlaveLog,
}

/// A simulated slave on a simulated bus; implements [`MailboxBus`] in virtual time.
pub struct VirtualSlave {
    cfg: VirtualSlaveConfig,
    st: RefCell<State>,
}

impl std::fmt::Debug for VirtualSlave {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VirtualSlave").field("cfg", &self.cfg).finish_non_exhaustive()
    }
}

impl VirtualSlave {
    /// A slave with these settings, at virtual time 0.
    pub fn new(cfg: VirtualSlaveConfig) -> Self {
        let st = State {
            now: 0,
            datagrams: 0,
            sm0: None,
            sm1: None,
            sm1_activate: 0x01,
            sm1_pdi_control: 0,
            outbox: VecDeque::new(),
            last_sent: None,
            out_counter: 0,
            last_in_counter: cfg.previous_counter,
            foe: Foe::Idle,
            data_packets: 0,
            sdo: Sdo::Idle,
            eoe_rx: EoeReassembler::default(),
            eoe_tx_offset: 0,
            eoe_frame_no: 0,
            log: VirtualSlaveLog::default(),
        };
        Self { cfg, st: RefCell::new(st) }
    }

    /// The slave's mailbox layout.
    pub fn layout(&self) -> MailboxLayout {
        self.cfg.mailbox
    }

    /// What the slave received and did so far.
    pub fn log(&self) -> VirtualSlaveLog {
        let st = self.st.borrow();
        VirtualSlaveLog { eoe_broken: st.eoe_rx.broken, ..st.log.clone() }
    }

    fn reply(&self, st: &mut State, at: u64, kind: MailboxType, body: Vec<u8>) {
        st.outbox.push_back((at, kind, body));
    }

    /// Run the slave application up to `st.now`.
    fn step(&self, st: &mut State) {
        loop {
            let mut changed = false;
            if let Some((buf, arrived)) = st.sm0.take() {
                let at = arrived + self.cfg.processing_ns;
                if at <= st.now {
                    self.take_message(st, &buf, at);
                    changed = true;
                } else {
                    st.sm0 = Some((buf, arrived));
                }
            }
            if self.cfg.eoe_send && st.outbox.is_empty() && st.eoe_tx_offset < self.cfg.upload.len() {
                self.queue_eoe_frame(st);
                changed = true;
            }
            if st.sm1.is_none()
                && let Some((ready, _, _)) = st.outbox.front()
                && *ready <= st.now
                && let Some((_, kind, body)) = st.outbox.pop_front()
            {
                st.out_counter = msg::next_counter(st.out_counter);
                let buf = msg::mailbox_buffer(
                    kind,
                    st.out_counter,
                    self.cfg.mailbox.station,
                    &body,
                    self.cfg.mailbox.in_len as usize,
                );
                st.last_sent = Some(buf.clone());
                st.sm1 = Some(buf);
                st.log.messages_out += 1;
                changed = true;
            }
            if !changed {
                break;
            }
        }
    }

    fn queue_eoe_frame(&self, st: &mut State) {
        let end = (st.eoe_tx_offset + self.cfg.eoe_mtu - 28).min(self.cfg.upload.len());
        let frame = udp_frame(&self.cfg.upload[st.eoe_tx_offset..end], &self.cfg.eoe_addresses, false);
        st.eoe_tx_offset = end;
        st.eoe_frame_no = msg::next_eoe_frame_no(st.eoe_frame_no);
        let cap = msg::eoe_capacity(self.cfg.mailbox.in_len as usize);
        let now = st.now;
        for f in eoe_fragments(&frame, cap, st.eoe_frame_no) {
            st.outbox.push_back((now, MailboxType::Eoe, f));
        }
    }

    fn take_message(&self, st: &mut State, buf: &[u8], at: u64) {
        let Ok(h) = MailboxHeader::parse(buf) else { return };
        if self.cfg.drop_repeated_counter && h.counter != 0 && h.counter == st.last_in_counter {
            st.log.duplicates_dropped += 1;
            return;
        }
        st.last_in_counter = h.counter;
        st.log.messages_in += 1;
        let end = (MAILBOX_HEADER_LEN + h.length as usize).min(buf.len());
        let body = &buf[MAILBOX_HEADER_LEN..end];
        match h.mailbox_type {
            MailboxType::Foe => self.on_foe(st, body, at),
            MailboxType::Coe => self.on_coe(st, body, at),
            MailboxType::Eoe => {
                if let Some(frame) = st.eoe_rx.push(body) {
                    st.log.eoe_frames.push(frame);
                }
            }
            _ => {
                // Mailbox error reply: protocol not supported.
                let e = MailboxErrorReply { service: 1, detail: 2 };
                self.reply(st, at, MailboxType::Error, e.encode().to_vec());
            }
        }
    }

    fn on_foe(&self, st: &mut State, body: &[u8], at: u64) {
        let Ok(h) = FoeHeader::parse(body) else { return };
        let data = &body[FoeHeader::LEN..];
        let cap_out = msg::foe_capacity(self.cfg.mailbox.out_len as usize);
        let cap_in = msg::foe_capacity(self.cfg.mailbox.in_len as usize);
        match (h.opcode, &mut st.foe) {
            (FoeOpcode::Write, _) => {
                let name = String::from_utf8_lossy(data).into_owned();
                st.foe = Foe::Writing { name, expected: 1, data: Vec::new(), busy_sent: false };
                self.reply(st, at, MailboxType::Foe, foe(FoeOpcode::Ack, 0, &[]));
            }
            (FoeOpcode::Data, Foe::Writing { name, expected, data: file, busy_sent }) if h.value == *expected => {
                st.data_packets += 1;
                if let Some(n) = self.cfg.busy_every
                    && n > 0
                    && st.data_packets.is_multiple_of(n)
                    && !*busy_sent
                {
                    *busy_sent = true;
                    let busy = foe(FoeOpcode::Busy, 0, &[]);
                    self.reply(st, at, MailboxType::Foe, busy);
                    return;
                }
                *busy_sent = false;
                file.extend_from_slice(data);
                *expected += 1;
                let ack = foe(FoeOpcode::Ack, h.value, &[]);
                if data.len() < cap_out {
                    let done = (std::mem::take(name), std::mem::take(file));
                    st.log.files.push(done);
                    st.foe = Foe::Idle;
                }
                self.reply(st, at, MailboxType::Foe, ack);
            }
            (FoeOpcode::Data, Foe::Writing { expected, .. }) if h.value + 1 == *expected => {
                self.reply(st, at, MailboxType::Foe, foe(FoeOpcode::Ack, h.value, &[]));
            }
            (FoeOpcode::Read, _) => {
                let n = cap_in.min(self.cfg.upload.len());
                st.foe = Foe::Reading { packet: 1, offset: n, last: n < cap_in };
                let pkt = foe(FoeOpcode::Data, 1, &self.cfg.upload[..n]);
                self.reply(st, at, MailboxType::Foe, pkt);
            }
            (FoeOpcode::Ack, Foe::Reading { packet, offset, last }) if h.value == *packet => {
                if *last {
                    st.foe = Foe::Idle;
                    return;
                }
                let end = (*offset + cap_in).min(self.cfg.upload.len());
                *packet += 1;
                *last = end - *offset < cap_in;
                let pkt = foe(FoeOpcode::Data, *packet, &self.cfg.upload[*offset..end]);
                *offset = end;
                self.reply(st, at, MailboxType::Foe, pkt);
            }
            _ => {
                let err = foe(FoeOpcode::Error, 0x8004, b"illegal");
                self.reply(st, at, MailboxType::Foe, err);
            }
        }
    }

    fn on_coe(&self, st: &mut State, body: &[u8], at: u64) {
        let Ok(c) = CoeHeader::parse(body) else { return };
        if c.service != CoeService::SdoRequest || body.len() < CoeHeader::LEN + 1 {
            return;
        }
        let cmd = SdoCommand::decode(body[CoeHeader::LEN], true);
        let hdr = SdoHeader::parse(&body[CoeHeader::LEN..]).ok();
        let (ix, sub) = (self.cfg.sdo_index, self.cfg.sdo_subindex);
        let abort = |code: u32| msg::sdo(CoeService::SdoResponse, SdoCommand::Abort, ix, sub, code, &[]);
        let addressed = |h: Option<SdoHeader>| h.is_some_and(|h| h.index == ix && h.subindex == sub);
        let cap_init = msg::sdo_initiate_capacity(self.cfg.mailbox.in_len as usize);
        let cap_seg = msg::sdo_segment_capacity(self.cfg.mailbox.in_len as usize);
        let msg = match (cmd, &mut st.sdo) {
            (SdoCommand::InitiateDownloadRequest { .. }, _) if !addressed(hdr) => abort(0x0602_0000),
            (SdoCommand::InitiateDownloadRequest { expedited: false, .. }, _) => {
                let size = hdr.map_or(0, |h| h.data) as usize;
                let tail = &body[CoeHeader::LEN + SdoHeader::LEN..];
                let data = tail[..tail.len().min(size)].to_vec();
                if data.len() >= size {
                    st.log.objects.push(data);
                    st.sdo = Sdo::Idle;
                } else {
                    st.sdo = Sdo::Download { toggle: false, data };
                }
                msg::sdo(CoeService::SdoResponse, SdoCommand::InitiateDownloadResponse, ix, sub, 0, &[])
            }
            (SdoCommand::DownloadSegmentRequest { toggle, unused, last }, Sdo::Download { toggle: t, data })
                if toggle == *t =>
            {
                let n = segment_data_len(body.len() as u16, unused);
                data.extend_from_slice(&body[CoeHeader::LEN + 1..][..n]);
                *t = !*t;
                if last {
                    let d = std::mem::take(data);
                    st.log.objects.push(d);
                    st.sdo = Sdo::Idle;
                }
                msg::sdo_segment(CoeService::SdoResponse, SdoCommand::DownloadSegmentResponse { toggle }, &[])
            }
            (SdoCommand::DownloadSegmentRequest { .. }, _) => abort(0x0503_0000),
            (SdoCommand::InitiateUploadRequest { .. }, _) if !addressed(hdr) => abort(0x0602_0000),
            (SdoCommand::InitiateUploadRequest { .. }, _) => {
                let obj = &self.cfg.upload;
                if !obj.is_empty() && obj.len() <= 4 {
                    let mut d = [0u8; 4];
                    d[..obj.len()].copy_from_slice(obj);
                    let cmd = SdoCommand::InitiateUploadResponse {
                        complete_access: false,
                        expedited: true,
                        size_indicated: true,
                        unused: (4 - obj.len()) as u8,
                    };
                    st.sdo = Sdo::Idle;
                    msg::sdo(CoeService::SdoResponse, cmd, ix, sub, u32::from_le_bytes(d), &[])
                } else {
                    let n = cap_init.min(obj.len());
                    let cmd = SdoCommand::InitiateUploadResponse {
                        complete_access: false,
                        expedited: false,
                        size_indicated: true,
                        unused: 0,
                    };
                    st.sdo = Sdo::Upload { offset: n, toggle: false };
                    msg::sdo(CoeService::SdoResponse, cmd, ix, sub, obj.len() as u32, &obj[..n])
                }
            }
            (SdoCommand::UploadSegmentRequest { toggle }, Sdo::Upload { offset, toggle: t }) if toggle == *t => {
                let obj = &self.cfg.upload;
                let end = (*offset + cap_seg).min(obj.len());
                let n = end - *offset;
                let last = end == obj.len();
                let cmd =
                    SdoCommand::UploadSegmentResponse { toggle, unused: if n < 7 { (7 - n) as u8 } else { 0 }, last };
                let msg = msg::sdo_segment(CoeService::SdoResponse, cmd, &obj[*offset..end]);
                *offset = end;
                *t = !*t;
                if last {
                    st.sdo = Sdo::Idle;
                }
                msg
            }
            _ => abort(0x0504_0001),
        };
        self.reply(st, at, MailboxType::Coe, msg);
    }

    fn sm_block(&self, st: &State, sm: u8) -> [u8; 8] {
        let m = &self.cfg.mailbox;
        let (start, len, control, full, activate, pdi) = match sm {
            0 => (m.out_start, m.out_len, 0x26, st.sm0.is_some(), 0x01, 0),
            1 => (m.in_start, m.in_len, 0x22, st.sm1.is_some(), st.sm1_activate, st.sm1_pdi_control),
            _ => return [0; 8],
        };
        SyncManagerConfig {
            start,
            length: len,
            control,
            status: if full { SM_STATUS_MAILBOX_FULL } else { 0 },
            activate,
            pdi_control: pdi,
        }
        .encode()
    }

    /// Advance virtual time by one datagram, run the slave, and decide whether the frame is
    /// lost on its way back.
    fn begin(&self, st: &mut State) {
        st.now += self.cfg.access_ns;
        self.step(st);
        st.log.accesses += 1;
    }

    fn lost(&self, st: &mut State) -> bool {
        st.datagrams += 1;
        self.cfg.lose_every.is_some_and(|n| n > 0 && st.datagrams.is_multiple_of(n))
    }
}

impl MailboxBus for VirtualSlave {
    fn now_ns(&self) -> u64 {
        self.st.borrow().now
    }

    async fn sleep_until_ns(&self, t_ns: u64) {
        let mut st = self.st.borrow_mut();
        st.now = st.now.max(t_ns);
        self.step(&mut st);
    }

    async fn cyclic(&self) -> Result<Option<u16>, BusError> {
        let Some(d) = self.cfg.cyclic_ns else { return Ok(None) };
        let mut st = self.st.borrow_mut();
        st.now += d;
        self.step(&mut st);
        Ok(Some(3))
    }

    async fn fprd(&self, station: u16, register: u16, len: u16) -> Result<Reply, BusError> {
        let mut st = self.st.borrow_mut();
        if station != self.cfg.mailbox.station {
            st.now += self.cfg.access_ns;
            return Ok(Reply { data: vec![0; len as usize], wkc: 0 });
        }
        self.begin(&mut st);
        let m = self.cfg.mailbox;
        let reply = if register == m.in_start {
            match st.sm1.take() {
                Some(buf) if len >= m.in_len => Reply { data: buf[..len.min(m.in_len) as usize].to_vec(), wkc: 1 },
                Some(buf) => {
                    // Not up to the last byte: the buffer stays full.
                    let r = Reply { data: buf[..len as usize].to_vec(), wkc: 1 };
                    st.sm1 = Some(buf);
                    r
                }
                None => Reply { data: vec![0; len as usize], wkc: 0 },
            }
        } else if (sm_register(0)..sm_register(2)).contains(&register) {
            let mut regs = self.sm_block(&st, 0).to_vec();
            regs.extend_from_slice(&self.sm_block(&st, 1));
            let at = (register - sm_register(0)) as usize;
            let mut data = regs.get(at..).unwrap_or_default().to_vec();
            data.resize(len as usize, 0);
            Reply { data, wkc: 1 }
        } else {
            Reply { data: vec![0; len as usize], wkc: 1 }
        };
        if self.lost(&mut st) { Err(BusError::Lost) } else { Ok(reply) }
    }

    async fn fpwr(&self, station: u16, register: u16, data: &[u8]) -> Result<u16, BusError> {
        let mut st = self.st.borrow_mut();
        if station != self.cfg.mailbox.station {
            st.now += self.cfg.access_ns;
            return Ok(0);
        }
        self.begin(&mut st);
        let m = self.cfg.mailbox;
        let wkc = if register == m.out_start {
            if st.sm0.is_some() {
                0
            } else {
                if data.len() >= m.out_len as usize {
                    let now = st.now;
                    st.sm0 = Some((data[..m.out_len as usize].to_vec(), now));
                }
                1
            }
        } else if register == sm_activate_register(SM_MAILBOX_IN) && data.len() == 1 {
            if (data[0] ^ st.sm1_activate) & SM_REPEAT_REQUEST != 0
                && let Some(buf) = st.last_sent.clone()
            {
                st.sm1 = Some(buf);
                let ack = if data[0] & SM_REPEAT_REQUEST != 0 { SM_REPEAT_ACK } else { 0 };
                st.sm1_pdi_control = (st.sm1_pdi_control & !SM_REPEAT_ACK) | ack;
                st.log.repeats += 1;
            }
            st.sm1_activate = data[0];
            1
        } else {
            1
        };
        if self.lost(&mut st) { Err(BusError::Lost) } else { Ok(wkc) }
    }
}
