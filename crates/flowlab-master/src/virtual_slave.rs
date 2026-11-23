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
    MailboxType, SM_REPEAT_ACK, SM_REPEAT_REQUEST, SM_STATUS_MAILBOX_FULL, SyncManagerConfig, sm_activate_register,
    sm_register,
};

use crate::bus::{BusError, MailboxBus, Reply};
use crate::{MailboxLayout, bus::SM_MAILBOX_IN};
use flowlab_mailbox::msg::{self, EoeAddresses};
use flowlab_mailbox::responder::{Responder, ResponderConfig};

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
    /// payload for an EoE send, the payload of a FlowLab VoE upload.
    pub upload: Vec<u8>,
    /// Send `upload` over EoE from the start.
    pub eoe_send: bool,
    /// IP MTU of the slave's EoE frames, bytes.
    pub eoe_mtu: usize,
    /// Addresses of the slave's EoE frames.
    pub eoe_addresses: EoeAddresses,
    /// Answer every n-th EoE frame received with a frame of the slave's own, as a network
    /// stack behind EoE does (ARP requests, ICMP replies).
    pub eoe_reply_every: Option<u64>,
    /// Take a message from "mailbox out" only while "mailbox in" is free and nothing waits to
    /// go into it: a stack that answers one message at a time.
    pub one_message_at_a_time: bool,
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
            eoe_reply_every: None,
            one_message_at_a_time: false,
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
    /// Payloads received over FlowLab VoE, in order.
    pub voe_received: Vec<Vec<u8>>,
    /// Uploads sent over FlowLab VoE to the end.
    pub voe_sent: u64,
    /// Messages put into "mailbox in".
    pub messages_out: u64,
    /// Repeat requests served.
    pub repeats: u64,
    /// Datagrams addressed to the slave.
    pub accesses: u64,
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
    app: Responder,
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
            app: Responder::new(
                ResponderConfig {
                    busy_every: cfg.busy_every,
                    drop_repeated_counter: cfg.drop_repeated_counter,
                    previous_counter: cfg.previous_counter,
                    sdo_index: cfg.sdo_index,
                    sdo_subindex: cfg.sdo_subindex,
                    upload: cfg.upload.clone(),
                    eoe_send: cfg.eoe_send,
                    eoe_mtu: cfg.eoe_mtu,
                    eoe_addresses: cfg.eoe_addresses.clone(),
                    ..ResponderConfig::default()
                },
                cfg.mailbox.out_len as usize,
                cfg.mailbox.in_len as usize,
            ),
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
        let app = st.app.log();
        VirtualSlaveLog {
            files: app.files,
            objects: app.objects,
            eoe_frames: app.eoe_frames,
            eoe_broken: app.eoe_broken,
            messages_in: app.messages_in,
            duplicates_dropped: app.duplicates_dropped,
            voe_received: app.voe_received,
            voe_sent: app.voe_sent,
            ..st.log.clone()
        }
    }

    /// Run the slave application up to `st.now`.
    fn step(&self, st: &mut State) {
        loop {
            let mut changed = false;
            let busy = self.cfg.one_message_at_a_time && (st.sm1.is_some() || !st.outbox.is_empty());
            if !busy && let Some((buf, arrived)) = st.sm0.take() {
                let at = arrived + self.cfg.processing_ns;
                if at <= st.now {
                    let frames = self.cfg.eoe_reply_every.map(|_| st.app.log().eoe_frames.len() as u64);
                    for m in st.app.take_message(&buf) {
                        st.outbox.push_back((at, m.kind, m.body));
                    }
                    if let Some(n) = self.cfg.eoe_reply_every
                        && let Some(frames) = frames
                        && st.app.log().eoe_frames.len() as u64 > frames
                        && (frames + 1).is_multiple_of(n)
                    {
                        let frame = msg::udp_frame(&[0; 32], &self.cfg.eoe_addresses, false);
                        let capacity = msg::eoe_capacity(self.cfg.mailbox.in_len as usize);
                        for body in msg::eoe_fragments(&frame, capacity, 1) {
                            st.outbox.push_back((at, MailboxType::Eoe, body));
                        }
                    }
                    changed = true;
                } else {
                    st.sm0 = Some((buf, arrived));
                }
            }
            if st.outbox.is_empty() && st.app.eoe_pending() {
                let now = st.now;
                for m in st.app.next_eoe_frame() {
                    st.outbox.push_back((now, m.kind, m.body));
                }
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
