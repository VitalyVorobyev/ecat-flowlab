//! The transfer engine: cycles, the mailbox service policy, counters, retries and timing.

use ecat_wire::{MAILBOX_HEADER_LEN, MailboxErrorReply, MailboxHeader, MailboxType};
use flowlab_core::{InputRef, Provenance, ReportMeta, units};
use flowlab_mailbox::{MailboxPolicy, MailboxProtocol, TransferDirection, TransferPlan};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::MasterError;
use crate::bus::{
    BusError, MailboxBus, MailboxLayout, SM_MAILBOX_IN, SM_REPEAT_ACK, SM_REPEAT_REQUEST, read_mailbox, read_sm,
    read_sm_status, sm_register, write_mailbox,
};
use crate::protocol::{
    Endpoint, EoeAddresses, EoeReceive, EoeSend, FoeRead, FoeWrite, Outgoing, SdoDownload, SdoUpload,
};
use crate::result::{CycleRecord, CycleTiming, MeasuredTransfer, Summary};

/// The protocol a transfer uses, with its parameters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TransferProtocol {
    /// File access over EtherCAT: write (master → slave) or read (slave → master).
    Foe {
        /// File name in the request.
        file_name: String,
        /// Password in the request.
        password: u32,
    },
    /// CoE SDO download (master → slave) or upload (slave → master), normal transfer with
    /// segments.
    Coe {
        /// Object index.
        index: u16,
        /// Subindex.
        subindex: u8,
    },
    /// UDP/IPv4 datagrams tunnelled through EoE.
    Eoe {
        /// IP MTU of the tunnelled frames, bytes (≤ 1500).
        mtu_bytes: u16,
        /// Addresses and ports of the tunnelled frames.
        addresses: EoeAddresses,
    },
}

impl TransferProtocol {
    /// FoE with the simulator's file name (`flowlab.bin`) and password 0.
    pub fn foe() -> Self {
        Self::Foe { file_name: "flowlab.bin".into(), password: 0 }
    }

    /// CoE at the simulator's object, `0x2000:00`.
    pub fn coe() -> Self {
        Self::Coe { index: 0x2000, subindex: 0 }
    }

    /// EoE with a 1500-byte MTU and the simulator's addresses.
    pub fn eoe() -> Self {
        Self::Eoe { mtu_bytes: 1500, addresses: EoeAddresses::default() }
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Foe { .. } => "FoE",
            Self::Coe { .. } => "CoE",
            Self::Eoe { .. } => "EoE",
        }
    }
}

/// What moves: the master's payload, or what the master expects to receive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Payload {
    /// Master → slave: these bytes.
    Send(Vec<u8>),
    /// Slave → master.
    Receive {
        /// Bytes to wait for. Required for EoE (the stream has no end); optional otherwise.
        expected_bytes: Option<u64>,
        /// SHA-256 (hex) the received bytes should have.
        expected_sha256: Option<String>,
    },
}

/// One transfer to run.
#[derive(Debug, Clone)]
pub struct TransferJob {
    /// Slave name, for the result.
    pub slave: String,
    /// The slave's mailbox.
    pub mailbox: MailboxLayout,
    /// Protocol.
    pub protocol: TransferProtocol,
    /// Payload, which also sets the direction.
    pub payload: Payload,
    /// What to call the payload in the result (file name, or a description).
    pub payload_name: String,
}

/// How the master learns that "mailbox in" holds a message.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PollMode {
    /// Read the buffer; WKC 0 means it was empty (as the simulator does).
    #[default]
    Read,
    /// Read SyncManager 1's status register first, the buffer only when it shows full.
    Status,
}

impl PollMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Status => "status",
        }
    }
}

/// How a transfer is run.
#[derive(Debug, Clone)]
pub struct TransferOptions {
    /// Master cycle time, ns.
    pub cycle_time_ns: u64,
    /// Mailbox service policy, the simulator's type: a real run is set up like a simulated one.
    pub policy: MailboxPolicy,
    /// How "mailbox in" is polled.
    pub poll: PollMode,
    /// Without a reply for this long, send the last message again with a new counter, ns.
    pub reply_timeout_ns: u64,
    /// Times a message is sent again without progress before the transfer fails.
    pub max_resends: u32,
    /// Stop after this long even if the transfer is unfinished, ns.
    pub max_duration_ns: u64,
    /// Counter of the first message (1–7; later ones cycle 1–7).
    pub first_counter: u8,
    /// Read and discard messages waiting in "mailbox in" before starting (not for an EoE
    /// receive, whose stream may already have started).
    pub drain_first: bool,
    /// Keep at most this many per-cycle records.
    pub max_cycle_records: usize,
    /// Link rate for the physical bound, bit/s.
    pub link_bps: u64,
    /// Assumptions of the bus implementation (e.g. where timestamps come from), added to the
    /// result's.
    pub assumptions: Vec<String>,
}

impl Default for TransferOptions {
    fn default() -> Self {
        Self {
            cycle_time_ns: 1_000_000,
            policy: MailboxPolicy::default(),
            poll: PollMode::Read,
            reply_timeout_ns: 1_000_000_000,
            max_resends: 3,
            max_duration_ns: 600_000_000_000,
            first_counter: 1,
            drain_first: true,
            max_cycle_records: 100_000,
            link_bps: units::FAST_ETHERNET_BPS,
            assumptions: Vec::new(),
        }
    }
}

const fn next_counter(c: u8) -> u8 {
    if c >= 7 { 1 } else { c + 1 }
}

/// A message encoded for "mailbox out", with its counter.
struct Pending {
    msg: Outgoing,
    buffer: Vec<u8>,
}

#[derive(Default)]
struct Counts {
    to_slave: u64,
    from_slave: u64,
    rejected: u64,
    empty: u64,
    status_reads: u64,
    lost: u64,
    repeats: u64,
    resent: u64,
    duplicates: u64,
    other: u64,
    drained: u64,
}

enum ReadResult {
    Message(Vec<u8>),
    Empty,
    /// The buffer read did not come back: the message may be gone; ask for a repeat.
    LostBuffer,
    /// Another access did not come back; nothing was consumed.
    Lost,
    /// The repeat request was written (expected acknowledge bit).
    RepeatRequested(bool),
    /// Waiting for the slave to acknowledge the repeat request.
    RepeatPending,
    Fatal(String),
}

struct ReadOutcome {
    result: ReadResult,
    accesses: u32,
    status_reads: u32,
    rtts: Vec<f64>,
    ret_ns: u64,
}

struct WriteOutcome {
    result: Result<u16, BusError>,
    rtt: f64,
    ret_ns: u64,
}

struct Engine<'a, B: MailboxBus> {
    bus: &'a B,
    m: MailboxLayout,
    ep: Box<dyn Endpoint>,
    to_slave: bool,
    counter: u8,
    last_in_counter: Option<u8>,
    pending: Option<Pending>,
    last_accepted: Option<Outgoing>,
    need_repeat: bool,
    repeat_ack: Option<bool>,
    poll: PollMode,
    c: Counts,
    t0: u64,
    first_msg: Option<u64>,
    last_msg: Option<u64>,
    first_byte: Option<u64>,
    end: Option<u64>,
    delivered: u64,
    last_progress: u64,
    consecutive_resends: u32,
    error: Option<String>,
    access_rtts: Vec<f64>,
}

impl<'a, B: MailboxBus> Engine<'a, B> {
    fn encode(&mut self, msg: Outgoing) -> Result<Pending, MasterError> {
        let len = self.m.out_len as usize;
        if MAILBOX_HEADER_LEN + msg.body.len() > len {
            return Err(MasterError::Invalid(format!(
                "a {} message of {} bytes does not fit the {len}-byte mailbox",
                msg.kind.code(),
                msg.body.len()
            )));
        }
        let h = MailboxHeader {
            length: msg.body.len() as u16,
            address: 0,
            channel: 0,
            priority: 0,
            mailbox_type: msg.kind,
            counter: self.counter,
        };
        self.counter = next_counter(self.counter);
        let mut buffer = h.encode().to_vec();
        buffer.extend_from_slice(&msg.body);
        buffer.resize(len, 0);
        Ok(Pending { msg, buffer })
    }

    fn fill_pending(&mut self) -> Result<(), MasterError> {
        if self.pending.is_none()
            && self.error.is_none()
            && let Some(msg) = self.ep.next()
        {
            self.pending = Some(self.encode(msg)?);
        }
        Ok(())
    }

    fn want_read(&self) -> bool {
        self.error.is_none() && (self.ep.wants_input() || self.need_repeat || self.repeat_ack.is_some())
    }

    fn finished(&self) -> bool {
        self.error.is_some() || (self.ep.done() && self.pending.is_none())
    }

    async fn write(&self, buffer: Vec<u8>) -> WriteOutcome {
        let t = self.bus.now_ns();
        let result = write_mailbox(self.bus, &self.m, &buffer).await;
        let ret_ns = self.bus.now_ns();
        WriteOutcome { result, rtt: (ret_ns - t) as f64, ret_ns }
    }

    async fn read(&self, need_repeat: bool, repeat_ack: Option<bool>) -> ReadOutcome {
        let bus = self.bus;
        let st = self.m.station;
        let mut o =
            ReadOutcome { result: ReadResult::Empty, accesses: 0, status_reads: 0, rtts: Vec::new(), ret_ns: 0 };
        // Each step: time it, count it.
        macro_rules! step {
            ($e:expr) => {{
                let t = bus.now_ns();
                let r = $e.await;
                let now = bus.now_ns();
                o.accesses += 1;
                o.rtts.push((now - t) as f64);
                o.ret_ns = now;
                r
            }};
        }
        if need_repeat {
            // Toggle the repeat request bit; the slave writes the last message again and
            // toggles its acknowledge bit.
            let sm = match step!(read_sm(bus, st, SM_MAILBOX_IN)) {
                Ok(Some(sm)) => sm,
                Ok(None) => return ReadOutcome { result: ReadResult::RepeatPending, ..o },
                Err(BusError::Lost) => return ReadOutcome { result: ReadResult::Lost, ..o },
                Err(BusError::Fatal(e)) => return ReadOutcome { result: ReadResult::Fatal(e), ..o },
            };
            let activate = sm.activate ^ SM_REPEAT_REQUEST;
            return match step!(bus.fpwr(st, sm_register(SM_MAILBOX_IN) + 6, &[activate])) {
                Ok(w) if w > 0 => {
                    ReadOutcome { result: ReadResult::RepeatRequested(activate & SM_REPEAT_REQUEST != 0), ..o }
                }
                Ok(_) => ReadOutcome { result: ReadResult::RepeatPending, ..o },
                Err(BusError::Lost) => ReadOutcome { result: ReadResult::Lost, ..o },
                Err(BusError::Fatal(e)) => ReadOutcome { result: ReadResult::Fatal(e), ..o },
            };
        }
        if let Some(ack) = repeat_ack {
            match step!(read_sm(bus, st, SM_MAILBOX_IN)) {
                Ok(Some(sm)) if (sm.pdi_control & SM_REPEAT_ACK != 0) == ack => {}
                Ok(_) => return ReadOutcome { result: ReadResult::RepeatPending, ..o },
                Err(BusError::Lost) => return ReadOutcome { result: ReadResult::Lost, ..o },
                Err(BusError::Fatal(e)) => return ReadOutcome { result: ReadResult::Fatal(e), ..o },
            }
        } else if self.poll == PollMode::Status {
            o.status_reads += 1;
            match step!(read_sm_status(bus, st, SM_MAILBOX_IN)) {
                Ok(r) if r.wkc > 0 && r.data.first().is_some_and(|s| s & 0x08 != 0) => {}
                Ok(_) => return ReadOutcome { result: ReadResult::Empty, ..o },
                Err(BusError::Lost) => return ReadOutcome { result: ReadResult::Lost, ..o },
                Err(BusError::Fatal(e)) => return ReadOutcome { result: ReadResult::Fatal(e), ..o },
            }
        }
        let result = match step!(read_mailbox(bus, &self.m)) {
            Ok(r) if r.wkc > 0 => ReadResult::Message(r.data),
            Ok(_) => ReadResult::Empty,
            Err(BusError::Lost) => ReadResult::LostBuffer,
            Err(BusError::Fatal(e)) => ReadResult::Fatal(e),
        };
        ReadOutcome { result, ..o }
    }

    /// One mailbox frame: a write and/or a read, in flight together.
    async fn frame(&mut self, write: bool, read: bool) -> Result<u32, MasterError> {
        let buffer = if write { self.pending.as_ref().map(|p| p.buffer.clone()) } else { None };
        let (need_repeat, repeat_ack) = (self.need_repeat, self.repeat_ack);
        let wf = async {
            match buffer {
                Some(b) => Some(self.write(b).await),
                None => None,
            }
        };
        let rf = async { if read { Some(self.read(need_repeat, repeat_ack).await) } else { None } };
        let (w, r) = futures_lite::future::zip(wf, rf).await;
        let mut accesses = 0;
        if let Some(w) = w {
            accesses += 1;
            self.access_rtts.push(w.rtt);
            self.on_write(w)?;
        }
        if let Some(r) = r {
            accesses += r.accesses;
            self.access_rtts.extend_from_slice(&r.rtts);
            self.c.status_reads += r.status_reads as u64;
            self.on_read(r)?;
        }
        Ok(accesses)
    }

    fn on_write(&mut self, w: WriteOutcome) -> Result<(), MasterError> {
        match w.result {
            Ok(wkc) if wkc > 0 => {
                let Some(p) = self.pending.take() else { return Ok(()) };
                self.c.to_slave += 1;
                self.first_msg.get_or_insert(w.ret_ns);
                self.last_msg = Some(w.ret_ns);
                self.last_progress = w.ret_ns;
                if self.to_slave
                    && let Some(end) = p.msg.payload_end
                {
                    self.delivered = self.delivered.max(end);
                    if end > 0 {
                        self.first_byte.get_or_insert(w.ret_ns);
                    }
                }
                self.last_accepted = Some(p.msg);
                if self.ep.done() && self.end.is_none() && self.to_slave {
                    self.end = Some(w.ret_ns);
                }
            }
            Ok(_) => self.c.rejected += 1,
            Err(BusError::Lost) => self.c.lost += 1,
            Err(BusError::Fatal(e)) => return Err(MasterError::Bus(e)),
        }
        Ok(())
    }

    fn on_read(&mut self, r: ReadOutcome) -> Result<(), MasterError> {
        match r.result {
            ReadResult::Message(data) => {
                self.repeat_ack = None;
                self.on_message(&data, r.ret_ns);
            }
            ReadResult::Empty => self.c.empty += 1,
            ReadResult::LostBuffer => {
                self.c.lost += 1;
                self.need_repeat = true;
            }
            ReadResult::Lost => self.c.lost += 1,
            ReadResult::RepeatRequested(ack) => {
                self.c.repeats += 1;
                self.need_repeat = false;
                self.repeat_ack = Some(ack);
            }
            ReadResult::RepeatPending => {}
            ReadResult::Fatal(e) => return Err(MasterError::Bus(e)),
        }
        Ok(())
    }

    fn on_message(&mut self, data: &[u8], now: u64) {
        let Ok(h) = MailboxHeader::parse(data) else {
            self.error = Some("a mailbox message from the slave is shorter than its header".into());
            return;
        };
        if h.counter != 0 && self.last_in_counter == Some(h.counter) {
            self.c.duplicates += 1;
            return;
        }
        self.last_in_counter = Some(h.counter);
        self.c.from_slave += 1;
        self.first_msg.get_or_insert(now);
        self.last_msg = Some(now);
        let end = (MAILBOX_HEADER_LEN + h.length as usize).min(data.len());
        let body = &data[MAILBOX_HEADER_LEN..end];
        if h.mailbox_type == MailboxType::Error {
            let e = MailboxErrorReply::parse(body).map_or("malformed", |e| e.detail_text());
            self.error = Some(format!("mailbox error reply from the slave: {e}"));
            return;
        }
        match self.ep.on(h.mailbox_type, body) {
            Ok(true) => {
                self.last_progress = now;
                self.consecutive_resends = 0;
            }
            Ok(false) => self.c.other += 1,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        }
        if self.ep.take_resend_request()
            && let Some(msg) = self.last_accepted.clone()
        {
            self.resend(msg, now);
        }
        if !self.to_slave
            && let Some(got) = self.ep.received().map(|d| d.len() as u64)
        {
            self.delivered = got;
            if got > 0 {
                self.first_byte.get_or_insert(now);
            }
            if self.end.is_none() && self.ep.total() == Some(got) {
                self.end = Some(now);
            }
        }
        if self.to_slave && self.ep.done() && self.end.is_none() {
            self.end = Some(now);
        }
    }

    fn resend(&mut self, msg: Outgoing, now: u64) {
        if self.pending.is_some() {
            return;
        }
        match self.encode(msg) {
            Ok(p) => {
                self.pending = Some(p);
                self.c.resent += 1;
                self.last_progress = now;
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    fn check_reply_timeout(&mut self, now: u64, o: &TransferOptions) {
        if self.error.is_some() || self.pending.is_some() || !self.ep.wants_input() {
            return;
        }
        let idle = now.saturating_sub(self.last_progress);
        match self.last_accepted.clone() {
            Some(msg) if idle > o.reply_timeout_ns => {
                if self.consecutive_resends >= o.max_resends {
                    self.error = Some(format!(
                        "no reply from the slave within {} ms after {} resends",
                        o.reply_timeout_ns / 1_000_000,
                        o.max_resends
                    ));
                } else {
                    self.consecutive_resends += 1;
                    self.resend(msg, now);
                }
            }
            None if idle > o.reply_timeout_ns * (o.max_resends as u64 + 1) => {
                self.error = Some(format!(
                    "nothing from the slave within {} ms",
                    o.reply_timeout_ns * (o.max_resends as u64 + 1) / 1_000_000
                ));
            }
            _ => {}
        }
    }

    async fn drain(&mut self) -> Result<(), MasterError> {
        for _ in 0..16 {
            match read_mailbox(self.bus, &self.m).await {
                Ok(r) if r.wkc > 0 => {
                    self.c.drained += 1;
                    if let Ok(h) = MailboxHeader::parse(&r.data) {
                        self.last_in_counter = Some(h.counter);
                    }
                }
                Ok(_) => break,
                Err(BusError::Lost) => continue,
                Err(BusError::Fatal(e)) => return Err(MasterError::Bus(e)),
            }
        }
        Ok(())
    }
}

fn endpoint(job: &TransferJob) -> Result<Box<dyn Endpoint>, MasterError> {
    let out_len = job.mailbox.out_len as usize;
    let in_len = job.mailbox.in_len as usize;
    let min = MAILBOX_HEADER_LEN + 16;
    if out_len < min || in_len < min {
        return Err(MasterError::Invalid(format!(
            "mailboxes of {out_len} and {in_len} bytes are too small (at least {min} each)"
        )));
    }
    Ok(match (&job.protocol, &job.payload) {
        (TransferProtocol::Foe { file_name, password }, Payload::Send(d)) => {
            if MAILBOX_HEADER_LEN + 6 + file_name.len() > out_len {
                return Err(MasterError::Invalid("the FoE file name does not fit the mailbox".into()));
            }
            Box::new(FoeWrite::new(d.clone(), file_name, *password, out_len))
        }
        (TransferProtocol::Foe { file_name, password }, Payload::Receive { .. }) => {
            if MAILBOX_HEADER_LEN + 6 + file_name.len() > out_len {
                return Err(MasterError::Invalid("the FoE file name does not fit the mailbox".into()));
            }
            Box::new(FoeRead::new(file_name, *password, in_len - MAILBOX_HEADER_LEN - 6))
        }
        (TransferProtocol::Coe { index, subindex }, Payload::Send(d)) => {
            if d.len() > u32::MAX as usize {
                return Err(MasterError::Invalid("an SDO holds at most 4 GiB".into()));
            }
            Box::new(SdoDownload::new(d.clone(), *index, *subindex, out_len))
        }
        (TransferProtocol::Coe { index, subindex }, Payload::Receive { .. }) => {
            Box::new(SdoUpload::new(*index, *subindex))
        }
        (TransferProtocol::Eoe { mtu_bytes, addresses }, Payload::Send(d)) => {
            if !(64..=1500).contains(mtu_bytes) {
                return Err(MasterError::Invalid(format!("EoE MTU {mtu_bytes} is outside 64–1500 bytes")));
            }
            Box::new(EoeSend::new(d.clone(), addresses.clone(), *mtu_bytes as usize, out_len))
        }
        (TransferProtocol::Eoe { addresses, .. }, Payload::Receive { expected_bytes, .. }) => {
            let Some(n) = expected_bytes else {
                return Err(MasterError::Invalid("an EoE receive needs the number of bytes to wait for".into()));
            };
            Box::new(EoeReceive::new(addresses.destination_port, *n))
        }
    })
}

fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

/// Run one transfer over `bus` and measure it.
///
/// Every cycle starts with the bus's cyclic exchange; then the policy decides the mailbox
/// frames: a write when a message is waiting, a read when the protocol expects the slave to
/// answer. A rejected write (WKC 0) is repeated in a later frame; a read that found nothing
/// counts as empty. A lost frame is repeated (a write keeps its counter, so the slave can drop
/// a duplicate); a lost buffer read is recovered with the mailbox repeat request.
pub async fn run_transfer<B: MailboxBus>(
    bus: &B,
    job: TransferJob,
    o: &TransferOptions,
) -> Result<MeasuredTransfer, MasterError> {
    if o.cycle_time_ns == 0 {
        return Err(MasterError::Invalid("cycle time must be positive".into()));
    }
    if !(1..=7).contains(&o.first_counter) {
        return Err(MasterError::Invalid("the first mailbox counter must be 1–7".into()));
    }
    if let MailboxPolicy::Opportunistic { bus_budget_percent, .. } = o.policy
        && !(bus_budget_percent > 0.0 && bus_budget_percent <= 100.0)
    {
        return Err(MasterError::Invalid("bus budget must be in (0, 100] percent".into()));
    }
    let ep = endpoint(&job)?;
    let to_slave = matches!(job.payload, Payload::Send(_));
    let mut e = Engine {
        bus,
        m: job.mailbox,
        ep,
        to_slave,
        counter: o.first_counter,
        last_in_counter: None,
        pending: None,
        last_accepted: None,
        need_repeat: false,
        repeat_ack: None,
        poll: o.poll,
        c: Counts::default(),
        t0: 0,
        first_msg: None,
        last_msg: None,
        first_byte: None,
        end: None,
        delivered: 0,
        last_progress: 0,
        consecutive_resends: 0,
        error: None,
        access_rtts: Vec::new(),
    };
    // An EoE stream from the slave may already be running: its first fragments are the payload.
    let slave_streams = !to_slave && matches!(job.protocol, TransferProtocol::Eoe { .. });
    if o.drain_first && !slave_streams {
        e.drain().await?;
    }
    let cycle = o.cycle_time_ns;
    let t0 = bus.now_ns();
    e.t0 = t0;
    e.last_progress = t0;
    let mut records = Vec::new();
    let mut truncated = false;
    let (mut late, mut periods, mut cyclic, mut busy) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let (mut cycles, mut skipped, mut overruns, mut max_acc) = (0u64, 0u64, 0u64, 0u32);
    let mut prev_start: Option<u64> = None;
    let mut k = 0u64;
    loop {
        let planned = t0 + k * cycle;
        bus.sleep_until_ns(planned).await;
        let start = bus.now_ns();
        cycles += 1;
        late.push((start - planned) as f64);
        if let Some(p) = prev_start {
            periods.push((start - p) as f64);
        }
        prev_start = Some(start);
        let (cyclic_ns, cyclic_wkc) = match bus.cyclic().await {
            Ok(Some(w)) => {
                let d = (bus.now_ns() - start) as f64;
                cyclic.push(d);
                (Some(d), Some(w))
            }
            Ok(None) => (None, None),
            Err(BusError::Lost) => (Some((bus.now_ns() - start) as f64), None),
            Err(BusError::Fatal(m)) => return Err(MasterError::Bus(m)),
        };
        let mut accesses = 0u32;
        let mut frames = 0u32;
        loop {
            e.fill_pending()?;
            let write_wanted = e.pending.is_some();
            let read_wanted = e.want_read();
            let (write, read) = match o.policy {
                MailboxPolicy::FixedQuota { accesses_per_cycle: q } => {
                    if frames > 0 {
                        break;
                    }
                    let w = write_wanted && q >= 1;
                    (w, read_wanted && q > w as u32)
                }
                MailboxPolicy::RoundRobin { slaves_per_cycle } => {
                    if frames > 0 || slaves_per_cycle == 0 {
                        break;
                    }
                    (write_wanted, read_wanted)
                }
                MailboxPolicy::Opportunistic { .. } => (write_wanted, read_wanted),
            };
            if !(write || read) {
                break;
            }
            accesses += e.frame(write, read).await?;
            frames += 1;
            if e.finished() {
                break;
            }
            if let MailboxPolicy::Opportunistic { bus_budget_percent, turnaround_ns } = o.policy {
                let at = bus.now_ns() + turnaround_ns as u64;
                if at >= start + (cycle as f64 * bus_budget_percent / 100.0) as u64 {
                    break;
                }
                bus.sleep_until_ns(at).await;
            }
        }
        let end = bus.now_ns();
        busy.push((end - start) as f64);
        max_acc = max_acc.max(accesses);
        if end > planned + cycle {
            overruns += 1;
        }
        if records.len() < o.max_cycle_records {
            records.push(CycleRecord {
                index: k,
                start_ns: (start - t0) as f64,
                late_ns: (start - planned) as f64,
                cyclic_ns,
                cyclic_wkc,
                mailbox_accesses: accesses,
                busy_ns: (end - start) as f64,
            });
        } else {
            truncated = true;
        }
        e.check_reply_timeout(end, o);
        if e.finished() {
            break;
        }
        if end - t0 >= o.max_duration_ns {
            e.error = Some(format!("time limit of {} s reached", o.max_duration_ns as f64 / 1e9));
            break;
        }
        let next = k + 1;
        let now_k = (end - t0) / cycle;
        k = if now_k > next {
            skipped += now_k - next;
            now_k
        } else {
            next
        };
    }

    let completed = e.error.is_none() && e.ep.done() && e.pending.is_none();
    let rel = |t: Option<u64>| t.map(|t| (t - t0) as f64);
    let payload_bytes = match &job.payload {
        Payload::Send(d) => d.len() as u64,
        Payload::Receive { expected_bytes, .. } => e.ep.total().or(*expected_bytes).unwrap_or(e.delivered),
    };
    let (sha256, verified) = match &job.payload {
        Payload::Send(d) => (sha256_hex(d), None),
        Payload::Receive { expected_sha256, expected_bytes, .. } => {
            let got = e.ep.received().unwrap_or_default();
            let sha = sha256_hex(got);
            let ok_len = expected_bytes.is_none_or(|n| n == got.len() as u64);
            (sha.clone(), expected_sha256.as_ref().map(|x| completed && ok_len && x.eq_ignore_ascii_case(&sha)))
        }
    };
    let end_ns = if completed { e.end.or(e.last_msg) } else { None };
    let duration_ns = end_ns.map(|t| (t - t0) as f64);
    let goodput_bps = duration_ns.filter(|d| *d > 0.0).map(|d| payload_bytes as f64 * 8.0 / (d / 1e9));
    let mut assumptions = vec![
        format!(
            "Mailbox policy: {}; one transfer to one slave, so the policy's limits apply to this slave alone.",
            o.policy.to_string()
        ),
        match o.poll {
            PollMode::Read => "\"Mailbox in\" is read directly; WKC 0 means it was empty.".to_owned(),
            PollMode::Status => {
                "\"Mailbox in\" is read only when SyncManager 1's status shows it full; an empty status counts as an empty read.".to_owned()
            }
        },
        format!(
            "Mailbox counters start at {} and cycle 1–7; a message from the slave with the previous message's counter is dropped as a duplicate.",
            o.first_counter
        ),
        format!(
            "Without a reply for {} ms, the last message is sent again with a new counter, up to {} times in a row; an FoE BUSY also resends it.",
            o.reply_timeout_ns / 1_000_000,
            o.max_resends
        ),
        "A lost write is repeated with its counter; a lost read of \"mailbox in\" is recovered with the repeat request (SyncManager 1, activate bit 1).".into(),
        "Times are relative to the transfer's first cycle; each is the return of an access (the end of its round trip).".into(),
    ];
    if to_slave {
        assumptions.push(
            "Master → slave: delivered bytes are bytes in writes the slave accepted, and the end is the return of the last acknowledgement (EoE: of the last fragment's write). The simulator's end is when the slave got the last byte, so compare exchange_ns, defined alike."
                .into(),
        );
    }
    assumptions.extend(o.assumptions.iter().cloned());
    let input = InputRef {
        name: job.payload_name.clone(),
        bytes: match &job.payload {
            Payload::Send(d) => d.len() as u64,
            Payload::Receive { .. } => e.delivered,
        },
        sha256: sha256.clone(),
    };
    Ok(MeasuredTransfer {
        meta: ReportMeta::new(Provenance::Measured, input, assumptions),
        station: job.mailbox.station,
        slave: job.slave,
        protocol: job.protocol.name().into(),
        direction: if to_slave { "master_to_slave" } else { "slave_to_master" }.into(),
        policy: o.policy.to_string(),
        poll: o.poll.as_str().into(),
        cycle_time_ns: cycle as f64,
        mailbox_out_bytes: job.mailbox.out_len,
        mailbox_in_bytes: job.mailbox.in_len,
        payload_bytes,
        delivered_bytes: e.delivered,
        verified,
        sha256,
        completed,
        error: e.error.clone(),
        start_ns: 0.0,
        first_byte_ns: rel(e.first_byte),
        end_ns: end_ns.map(|t| (t - t0) as f64),
        duration_ns,
        exchange_ns: match (e.first_msg, e.last_msg) {
            (Some(a), Some(b)) => Some((b - a) as f64),
            _ => None,
        },
        goodput_bps,
        messages_to_slave: e.c.to_slave,
        messages_from_slave: e.c.from_slave,
        rejected_writes: e.c.rejected,
        empty_reads: e.c.empty,
        status_reads: e.c.status_reads,
        lost_frames: e.c.lost,
        repeat_requests: e.c.repeats,
        resent_messages: e.c.resent,
        duplicate_messages: e.c.duplicates,
        other_messages: e.c.other,
        drained_messages: e.c.drained,
        physical_bound_ns: units::serialization_ns(payload_bytes, o.link_bps) as f64,
        link_bps: o.link_bps as f64,
        timing: CycleTiming {
            cycles,
            skipped_cycles: skipped,
            overruns,
            start_late: Summary::of(&late),
            period: Summary::of(&periods),
            cyclic: Summary::of(&cyclic),
            mailbox_access: Summary::of(&e.access_rtts),
            busy: Summary::of(&busy),
            max_accesses_per_cycle: max_acc,
        },
        cycle_records: records,
        cycle_records_truncated: truncated,
        received: e.ep.received().map(<[u8]>::to_vec),
    })
}

/// A transfer set up like transfer `index` of a simulator spec: same protocol, direction,
/// payload bytes (the simulator's generated payload, so the SHA-256s compare), cycle time and
/// mailbox policy.
#[derive(Debug, Clone)]
pub struct SimulatedSetup {
    /// Protocol (EoE with the spec's MTU and the simulator's addresses).
    pub protocol: TransferProtocol,
    /// The payload: the simulator's bytes to send, or the size and SHA-256 to expect.
    pub payload: Payload,
    /// Names the spec and the transfer, for the result.
    pub payload_name: String,
    /// The spec's cycle time, ns.
    pub cycle_time_ns: u64,
    /// The spec's mailbox policy.
    pub policy: MailboxPolicy,
    /// SHA-256 of the payload, hex (as in the simulator's result).
    pub sha256: String,
    /// The slave's mailbox sizes in the spec, bytes: "mailbox out", "mailbox in".
    pub mailbox_bytes: (u16, u16),
}

/// Set up a transfer like a simulated one, from its [`TransferPlan`] (`SimSpec::transfer_plan`
/// in `flowlab-sim`): the simulator's protocol parameters, payload bytes, cycle and policy.
pub fn setup_like(plan: &TransferPlan) -> Result<SimulatedSetup, MasterError> {
    let protocol = match plan.protocol {
        MailboxProtocol::Eoe { mtu_bytes } => TransferProtocol::Eoe { mtu_bytes, addresses: EoeAddresses::default() },
        MailboxProtocol::Foe => TransferProtocol::foe(),
        MailboxProtocol::CoeSegmented => TransferProtocol::coe(),
    };
    let sha256 = flowlab_mailbox::payload_sha256(plan.payload_seed, plan.payload_bytes);
    let payload = match plan.direction {
        TransferDirection::MasterToSlave => {
            let n =
                usize::try_from(plan.payload_bytes).map_err(|_| MasterError::Invalid("payload too large".into()))?;
            Payload::Send(flowlab_mailbox::payload(plan.payload_seed, n))
        }
        TransferDirection::SlaveToMaster => {
            Payload::Receive { expected_bytes: Some(plan.payload_bytes), expected_sha256: Some(sha256.clone()) }
        }
    };
    Ok(SimulatedSetup {
        protocol,
        payload,
        payload_name: format!(
            "transfer {} of sim spec seed {} ({} B, flowlab-sim payload)",
            plan.index, plan.spec_seed, plan.payload_bytes
        ),
        cycle_time_ns: plan.cycle_time_ns,
        policy: plan.policy.clone(),
        sha256,
        mailbox_bytes: (plan.mailbox_out_bytes, plan.mailbox_in_bytes),
    })
}
