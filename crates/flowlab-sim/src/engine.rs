//! The discrete-event engine.
//!
//! Virtual time in ns; events are ordered by `(time, insertion index)`, so a run is a pure
//! function of the spec. The master runs a cycle: one cyclic frame (LRW of the process image
//! and a BRD of AL status), then mailbox datagrams chosen by the policy. Frames occupy the
//! link for their wire time; each datagram reaches its slave after the bytes up to its end
//! have been serialized plus the forwarding delays of the slaves on the way, and the frame
//! returns after the whole ring. ESCs act when a datagram passes; slave applications use the
//! PDI one access at a time.

use std::cmp::{Ordering, Reverse};
use std::collections::{BinaryHeap, HashMap, VecDeque};
use std::io::Write;

use ecat_wire::{
    AlState, Command, DatagramSpec, FrameBuilder, MAILBOX_HEADER_LEN, MacAddr, MailboxHeader, MailboxType,
    SyncManagerConfig, wire_bytes,
};
use flowlab_capture::PcapNgWriter;
use flowlab_core::rng::SplitMix64;
use flowlab_core::units::serialization_ns;
use flowlab_core::{InputRef, Provenance, ReportMeta};
use flowlab_mailbox::{msg, payload_sha256, transfer_seed};

use crate::SimError;
use crate::protocol::{
    ChannelReceiver, ChannelSender, Endpoint, EoeReceiver, EoeSender, FoeReceiver, FoeSender, SdoReceiver, SdoSender,
    Sink, Source,
};
use crate::result::{SimResult, SlaveResult, TransferResult};
use crate::spec::{MailboxAccess, MailboxPolicy, SimSpec, TransferDirection, TransferProtocol};

/// Master MAC (IANA documentation range, RFC 7042).
pub const MASTER_MAC: MacAddr = MacAddr([0x00, 0x00, 0x5E, 0x00, 0x53, 0x02]);
/// Station address of the first slave; slave `p` gets `FIRST_STATION + p`.
pub const FIRST_STATION: u16 = 0x1001;
/// Start of the "mailbox out" SyncManager in every simulated ESC; "mailbox in" follows it.
pub const MBX_BASE: u16 = 0x1000;
const LOGICAL_BASE: u32 = 0x0001_0000;
const PREAMBLE_STEP_NS: u64 = 50_000;
/// Largest EtherCAT payload of one frame (Ethernet MTU − 2-byte EtherCAT header).
const MAX_PDU: usize = 1498;

// ---------------------------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------------------------

enum Ev {
    Cycle(u64),
    MailboxFrame { cycle_start: u64 },
    AtSlave { frame: u32, dg: u16, slave: usize },
    Return { frame: u32 },
    MbxReadDone { slave: usize },
    AppMessage { slave: usize, buf: Vec<u8> },
    MbxWriteDone { slave: usize, buf: Vec<u8> },
    PdReadStart { slave: usize },
    PdReadDone { slave: usize, outputs: Vec<u8> },
    PdWriteDone { slave: usize, inputs: Vec<u8> },
    TransferStart { index: usize },
}

struct Entry {
    t: u64,
    seq: u64,
    ev: Ev,
}

impl PartialEq for Entry {
    fn eq(&self, o: &Self) -> bool {
        (self.t, self.seq) == (o.t, o.seq)
    }
}
impl Eq for Entry {}
impl PartialOrd for Entry {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Entry {
    fn cmp(&self, o: &Self) -> Ordering {
        (self.t, self.seq).cmp(&(o.t, o.seq))
    }
}

// ---------------------------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    Config,
    Lrw,
    AlStatus,
    MbxWrite(usize),
    MbxRead(usize),
}

struct Dg {
    command: Command,
    address: u32,
    tx: Vec<u8>,
    rx: Vec<u8>,
    wkc: u16,
    role: Role,
}

impl Dg {
    fn config(command: Command, address: u32, data: Vec<u8>, wkc: u16) -> Self {
        Self { command, address, rx: data.clone(), tx: data, wkc, role: Role::Config }
    }
}

struct Flight {
    dgs: Vec<Dg>,
    first_index: u8,
    lost: bool,
    mailbox_cycle: Option<u64>,
}

#[derive(Default)]
struct MasterMbx {
    pending: Option<Vec<u8>>,
    write_in_flight: bool,
    read_in_flight: bool,
    counter: u8,
}

#[derive(Default)]
struct Esc {
    sm0: Option<Vec<u8>>,
    sm1: Option<Vec<u8>>,
    sm1_writing: bool,
    counter: u8,
    pdi_free_at: u64,
    pdi_busy_ns: u64,
    pd_out: Vec<u8>,
    pd_out_version: u64,
    pd_read_version: u64,
    pd_read_pending: bool,
    pd_in: Vec<u8>,
}

/// Where one slave's process data sits in the LRW datagram.
#[derive(Clone, Copy, Default)]
struct Region {
    out_at: usize,
    out_len: usize,
    in_at: usize,
    in_len: usize,
    /// Channel bytes at the end of the outputs / inputs.
    chan_out: usize,
    chan_in: usize,
}

enum Kind {
    Mailbox { master: Box<dyn Endpoint>, slave: Box<dyn Endpoint> },
    Channel { sender: Box<ChannelSender>, receiver: Box<ChannelReceiver>, master_sends: bool },
}

#[derive(Default)]
struct Counts {
    to_slave: u64,
    from_slave: u64,
    rejected: u64,
    empty: u64,
}

struct Transfer {
    first_msg_ns: Option<u64>,
    last_msg_ns: Option<u64>,
    seed: u64,
    eligible: bool,
    started_at: Option<u64>,
    kind: Option<Kind>,
    finished: bool,
    counts: Counts,
    retransmitted: u64,
    chunk_cycles: u64,
    sink: Option<Sink>,
}

struct Capture<W: Write> {
    writer: Option<PcapNgWriter<W>>,
    pending: BinaryHeap<Reverse<(u64, u64, Vec<u8>)>>,
    seq: u64,
    written: u64,
    max: u64,
    truncated: bool,
    t0_abs: u64,
}

impl<W: Write> Capture<W> {
    fn push(&mut self, t: u64, bytes: Vec<u8>) {
        if self.writer.is_none() {
            return;
        }
        if self.written + self.pending.len() as u64 >= self.max {
            self.truncated = true;
            return;
        }
        self.seq += 1;
        self.pending.push(Reverse((t, self.seq, bytes)));
    }

    fn flush_until(&mut self, t: u64) -> std::io::Result<()> {
        while let Some(Reverse((at, _, _))) = self.pending.peek() {
            if *at > t {
                break;
            }
            let Some(Reverse((at, _, bytes))) = self.pending.pop() else { break };
            if let Some(w) = self.writer.as_mut() {
                w.packet(self.t0_abs + at, &bytes)?;
                self.written += 1;
            }
        }
        Ok(())
    }
}

struct Sim<'a, W: Write> {
    spec: &'a SimSpec,
    queue: BinaryHeap<Reverse<Entry>>,
    seq: u64,
    now: u64,
    rng: SplitMix64,
    link_free: u64,
    flights: HashMap<u32, Flight>,
    next_frame: u32,
    index: u8,
    frames: u64,
    wire_bytes: u64,
    cycles: u64,
    /// Start of the latest master cycle, ns.
    cycle_start: u64,
    t0: u64,
    cycle_ns: u64,
    regions: Vec<Region>,
    image_len: usize,
    mbx: Vec<MasterMbx>,
    esc: Vec<Esc>,
    transfers: Vec<Transfer>,
    mailbox_queue: Vec<VecDeque<usize>>,
    active_mailbox: Vec<Option<usize>>,
    channel_of: Vec<Option<usize>>,
    rr: usize,
    capture: Capture<W>,
    stop: bool,
}

fn station(p: usize) -> u16 {
    FIRST_STATION + p as u16
}

impl<'a, W: Write> Sim<'a, W> {
    fn schedule(&mut self, t: u64, ev: Ev) {
        self.seq += 1;
        self.queue.push(Reverse(Entry { t, seq: self.seq, ev }));
    }

    fn fwd_to(&self, p: usize) -> u64 {
        // Half of each slave's two-way forwarding delay on the way out, up to and incl. `p`.
        self.spec.slaves[..=p].iter().map(|s| s.forwarding_delay_ns as u64 / 2).sum::<u64>()
    }

    fn ring_delay(&self) -> u64 {
        self.spec.slaves.iter().map(|s| s.forwarding_delay_ns as u64).sum()
    }

    /// Encode a frame as sent (`slaves_passed: None`) or as returned after the whole ring.
    fn build(src: MacAddr, dgs: &[Dg], slaves_passed: Option<u16>, first_index: u8) -> Result<Vec<u8>, SimError> {
        let mut b = FrameBuilder::new(MacAddr([0xFF; 6]), src);
        for (i, d) in dgs.iter().enumerate() {
            let (data, wkc, address) = match slaves_passed {
                // Every slave increments the ADP of position and broadcast datagrams.
                Some(n) => (&d.rx, d.wkc, d.command.address_after(d.address, n)),
                None => (&d.tx, 0, d.address),
            };
            let spec = DatagramSpec::new(d.command, first_index.wrapping_add(i as u8), address, data);
            b.push(spec.with_wkc(wkc))?;
        }
        Ok(b.finish())
    }

    /// Put a frame on the link no earlier than `at`.
    fn send(&mut self, at: u64, dgs: Vec<Dg>, mailbox_cycle: Option<u64>) -> Result<(), SimError> {
        if dgs.is_empty() {
            return Ok(());
        }
        let first_index = self.index;
        self.index = self.index.wrapping_add(dgs.len() as u8);
        let tx = Self::build(MASTER_MAC, &dgs, None, first_index)?;
        let wire = wire_bytes(tx.len() as u32) as u64;
        let start = at.max(self.link_free);
        self.link_free = start + serialization_ns(wire, self.spec.link_bps);
        self.frames += 1;
        self.wire_bytes += wire;
        let lost = self.rng.chance(self.spec.faults.frame_loss_probability);
        self.capture.push(start, tx.clone());
        let id = self.next_frame;
        self.next_frame += 1;
        if !lost {
            // Preamble/SFD (8) and the bytes up to the end of each datagram.
            let mut offset = 8 + 14 + 2;
            for (i, d) in dgs.iter().enumerate() {
                offset += 10 + d.tx.len() + 2;
                let passed = start + serialization_ns(offset as u64, self.spec.link_bps);
                match d.role {
                    Role::Lrw => {
                        for p in 0..self.spec.slaves.len() {
                            let r = self.regions[p];
                            if r.out_len + r.in_len > 0 {
                                let t = passed + self.fwd_to(p);
                                self.schedule(t, Ev::AtSlave { frame: id, dg: i as u16, slave: p });
                            }
                        }
                    }
                    Role::MbxWrite(p) | Role::MbxRead(p) => {
                        let t = passed + self.fwd_to(p);
                        self.schedule(t, Ev::AtSlave { frame: id, dg: i as u16, slave: p });
                    }
                    Role::Config | Role::AlStatus => {}
                }
            }
        }
        // Back at the master once the last bit (incl. FCS) has gone round the ring.
        let back = start + serialization_ns(8 + tx.len().max(60) as u64 + 4, self.spec.link_bps) + self.ring_delay();
        self.flights.insert(id, Flight { dgs, first_index, lost, mailbox_cycle });
        self.schedule(back, Ev::Return { frame: id });
        Ok(())
    }

    /// Send `dgs` in as few frames as fit.
    /// Send `dgs` in as few frames as fit. Only the last frame carries `mailbox_cycle`, so an
    /// opportunistic master runs one chain of mailbox frames per cycle however many frames a
    /// batch needs.
    fn send_packed(&mut self, at: u64, dgs: Vec<Dg>, mailbox_cycle: Option<u64>) -> Result<(), SimError> {
        let mut frame = Vec::new();
        let mut size = 0;
        for d in dgs {
            let len = 10 + d.tx.len() + 2;
            if size + len > MAX_PDU && !frame.is_empty() {
                self.send(at, std::mem::take(&mut frame), None)?;
                size = 0;
            }
            size += len;
            frame.push(d);
        }
        self.send(at, frame, mailbox_cycle)
    }

    // -- master ------------------------------------------------------------------------------

    fn encode(counter: &mut u8, t: MailboxType, body: &[u8], len: usize) -> Vec<u8> {
        *counter = msg::next_counter(*counter);
        msg::mailbox_buffer(t, *counter, 0, body, len)
    }

    fn master_fill(&mut self, p: usize) {
        if self.mbx[p].pending.is_some() {
            return;
        }
        let Some(k) = self.active_mailbox[p] else { return };
        let Some(Kind::Mailbox { master, .. }) = self.transfers[k].kind.as_mut() else { return };
        if let Some((t, body)) = master.next() {
            let len = self.spec.slaves[p].mailbox_out_bytes as usize;
            self.mbx[p].pending = Some(Self::encode(&mut self.mbx[p].counter, t, &body, len));
        }
    }

    fn wants_read(&self, p: usize) -> bool {
        let Some(k) = self.active_mailbox[p] else { return false };
        match &self.transfers[k].kind {
            Some(Kind::Mailbox { master, .. }) => master.wants_input(),
            _ => false,
        }
    }

    /// Mailbox datagrams for one frame: at most `quota` datagrams and `slaves_limit` slaves.
    fn mailbox_datagrams(&mut self, quota: usize, slaves_limit: usize) -> Vec<Dg> {
        let n = self.spec.slaves.len();
        let mut out = Vec::new();
        let mut served = 0;
        for i in 0..n {
            if out.len() >= quota || served >= slaves_limit {
                break;
            }
            let p = (self.rr + i) % n;
            self.master_fill(p);
            let write = self.mbx[p].pending.is_some() && !self.mbx[p].write_in_flight;
            let read = self.wants_read(p) && !self.mbx[p].read_in_flight;
            if !(write || read) {
                continue;
            }
            served += 1;
            let st = station(p) as u32;
            if write && out.len() < quota {
                let data = self.mbx[p].pending.clone().unwrap_or_default();
                self.mbx[p].write_in_flight = true;
                out.push(Dg {
                    command: Command::Fpwr,
                    address: st | (MBX_BASE as u32) << 16,
                    rx: data.clone(),
                    tx: data,
                    wkc: 0,
                    role: Role::MbxWrite(p),
                });
            }
            if read && out.len() < quota {
                let len = self.spec.slaves[p].mailbox_in_bytes as usize;
                let at = MBX_BASE + self.spec.slaves[p].mailbox_out_bytes;
                self.mbx[p].read_in_flight = true;
                out.push(Dg {
                    command: Command::Fprd,
                    address: st | (at as u32) << 16,
                    tx: vec![0; len],
                    rx: vec![0; len],
                    wkc: 0,
                    role: Role::MbxRead(p),
                });
            }
        }
        if n > 0 {
            self.rr = (self.rr + 1) % n;
        }
        out
    }

    fn cycle(&mut self, k: u64) -> Result<(), SimError> {
        let t = self.now;
        if self.stop || self.all_finished() {
            self.stop = true;
            return Ok(());
        }
        if t >= self.t0 + self.spec.max_duration_ms * 1_000_000 {
            self.stop = true;
            return Ok(());
        }
        self.cycles += 1;
        self.cycle_start = t;
        let mut dgs = Vec::with_capacity(2);
        if self.image_len > 0 {
            let mut data = vec![0u8; self.image_len];
            for p in 0..self.spec.slaves.len() {
                let r = self.regions[p];
                let base = r.out_len - r.chan_out;
                for (j, b) in data[r.out_at..r.out_at + base].iter_mut().enumerate() {
                    *b = (k as u8).wrapping_add(j as u8);
                }
                if let Some(i) = self.channel_of[p]
                    && let Some(Kind::Channel { sender, receiver, master_sends }) = self.transfers[i].kind.as_mut()
                {
                    let region = &mut data[r.out_at + base..r.out_at + r.out_len];
                    let mut new_chunks = 0;
                    if *master_sends {
                        let before = sender.chunk_updates;
                        sender.fill(region);
                        new_chunks = sender.chunk_updates - before;
                    } else {
                        receiver.fill_ack(region);
                    }
                    self.transfers[i].chunk_cycles += new_chunks;
                }
            }
            dgs.push(Dg {
                command: Command::Lrw,
                address: LOGICAL_BASE,
                rx: data.clone(),
                tx: data,
                wkc: 0,
                role: Role::Lrw,
            });
        }
        dgs.push(Dg {
            command: Command::Brd,
            address: 0x0130 << 16,
            tx: vec![0, 0],
            rx: vec![AlState::Op.code(), 0],
            wkc: self.spec.slaves.len() as u16,
            role: Role::AlStatus,
        });
        self.send(t, dgs, None)?;
        let n = self.spec.slaves.len();
        let (quota, limit) = match self.spec.mailbox_policy {
            MailboxPolicy::FixedQuota { accesses_per_cycle } => (accesses_per_cycle as usize, n),
            MailboxPolicy::RoundRobin { slaves_per_cycle } => (usize::MAX, slaves_per_cycle as usize),
            MailboxPolicy::Opportunistic { .. } => (usize::MAX, n),
        };
        let m = self.mailbox_datagrams(quota, limit);
        self.send_packed(t, m, Some(t))?;
        let max_j = (self.cycle_ns as f64 * 0.4) as i64;
        let j = ((self.rng.normal() * self.spec.master_jitter_ns as f64) as i64).clamp(-max_j, max_j);
        let next = (self.t0 + (k + 1) * self.cycle_ns).saturating_add_signed(j).max(t + 1);
        self.schedule(next, Ev::Cycle(k + 1));
        Ok(())
    }

    fn mailbox_frame(&mut self, cycle_start: u64) -> Result<(), SimError> {
        if self.stop {
            return Ok(());
        }
        let m = self.mailbox_datagrams(usize::MAX, self.spec.slaves.len());
        self.send_packed(self.now, m, Some(cycle_start))
    }

    fn on_return(&mut self, id: u32) -> Result<(), SimError> {
        let Some(f) = self.flights.remove(&id) else { return Ok(()) };
        let now = self.now;
        if !f.lost {
            let rx =
                Self::build(MASTER_MAC.with_local_bit(), &f.dgs, Some(self.spec.slaves.len() as u16), f.first_index)?;
            self.capture.push(now, rx);
        }
        let mut touched = Vec::new();
        for d in &f.dgs {
            match d.role {
                Role::MbxWrite(p) => {
                    self.mbx[p].write_in_flight = false;
                    let k = self.active_mailbox[p];
                    if !f.lost && d.wkc > 0 {
                        self.mbx[p].pending = None;
                        if let Some(k) = k {
                            let t = &mut self.transfers[k];
                            t.counts.to_slave += 1;
                            t.first_msg_ns.get_or_insert(now);
                            t.last_msg_ns = Some(now);
                        }
                    } else if !f.lost
                        && let Some(k) = k
                    {
                        self.transfers[k].counts.rejected += 1;
                    }
                    touched.push(p);
                }
                Role::MbxRead(p) => {
                    self.mbx[p].read_in_flight = false;
                    let k = self.active_mailbox[p];
                    if !f.lost
                        && d.wkc > 0
                        && let Some(k) = k
                    {
                        let t = &mut self.transfers[k];
                        t.counts.from_slave += 1;
                        t.first_msg_ns.get_or_insert(now);
                        t.last_msg_ns = Some(now);
                        if let Ok(h) = MailboxHeader::parse(&d.rx) {
                            let end = (MAILBOX_HEADER_LEN + h.length as usize).min(d.rx.len());
                            if let Some(Kind::Mailbox { master, .. }) = self.transfers[k].kind.as_mut() {
                                master.on(h.mailbox_type, &d.rx[MAILBOX_HEADER_LEN..end], now);
                            }
                        }
                    } else if !f.lost
                        && let Some(k) = k
                    {
                        self.transfers[k].counts.empty += 1;
                    }
                    touched.push(p);
                }
                Role::Lrw if !f.lost => {
                    for p in 0..self.spec.slaves.len() {
                        let Some(i) = self.channel_of[p] else { continue };
                        let r = self.regions[p];
                        let region = &d.rx[r.in_at + r.in_len - r.chan_in..r.in_at + r.in_len];
                        if let Some(Kind::Channel { sender, receiver, master_sends }) = self.transfers[i].kind.as_mut()
                        {
                            if *master_sends {
                                sender.on_ack(region);
                            } else {
                                receiver.on_chunk(region, now);
                            }
                        }
                        self.check_channel(p);
                    }
                }
                _ => {}
            }
        }
        for p in touched {
            self.check_mailbox(p);
            self.master_fill(p);
        }
        if let (Some(cs), MailboxPolicy::Opportunistic { bus_budget_percent, turnaround_ns }) =
            (f.mailbox_cycle, &self.spec.mailbox_policy)
        {
            // A frame still in flight when the next cycle starts continues in that cycle's
            // budget: the master keeps using spare time whichever cycle it falls in.
            let cs = cs.max(self.cycle_start);
            let at = now + *turnaround_ns as u64;
            let limit = cs + (self.cycle_ns as f64 * bus_budget_percent / 100.0) as u64;
            if at < limit && !self.stop {
                self.schedule(at, Ev::MailboxFrame { cycle_start: cs });
            }
        }
        Ok(())
    }

    // -- ESC and slave application --------------------------------------------------------------

    fn pdi(&mut self, p: usize, from: u64, bytes: usize, write: bool) -> u64 {
        self.pdi_accesses(p, from, bytes, 1, write)
    }

    /// PDI time for a mailbox buffer holding a message of `message_len` bytes (header included).
    fn pdi_mailbox(&mut self, p: usize, from: u64, buffer_len: usize, message_len: usize, write: bool) -> u64 {
        match self.spec.slaves[p].pdi.mailbox_access {
            MailboxAccess::WholeBuffer => self.pdi_accesses(p, from, buffer_len, 1, write),
            MailboxAccess::MessageOnly => {
                let n = message_len.min(buffer_len.saturating_sub(1)) + 1;
                self.pdi_accesses(p, from, n, if write { 2 } else { 3 }, write)
            }
        }
    }

    fn pdi_accesses(&mut self, p: usize, from: u64, bytes: usize, accesses: u64, write: bool) -> u64 {
        let s = &self.spec.slaves[p];
        let rate = if write { s.pdi.write_bps } else { s.pdi.read_bps };
        let dur = accesses * s.pdi.access_overhead_ns as u64 + serialization_ns(bytes as u64, rate);
        let e = &mut self.esc[p];
        let start = from.max(e.pdi_free_at);
        e.pdi_free_at = start + dur;
        e.pdi_busy_ns += dur;
        start + dur
    }

    fn at_slave(&mut self, id: u32, i: u16, p: usize) {
        let t = self.now;
        let Some(f) = self.flights.get_mut(&id) else { return };
        let d = &mut f.dgs[i as usize];
        match d.role {
            Role::Lrw => {
                let r = self.regions[p];
                let e = &mut self.esc[p];
                if r.out_len > 0 {
                    e.pd_out = d.tx[r.out_at..r.out_at + r.out_len].to_vec();
                    e.pd_out_version += 1;
                    d.wkc += 2;
                }
                if r.in_len > 0 {
                    if e.pd_in.len() == r.in_len {
                        d.rx[r.in_at..r.in_at + r.in_len].copy_from_slice(&e.pd_in);
                    }
                    d.wkc += 1;
                }
                if !e.pd_read_pending {
                    e.pd_read_pending = true;
                    let at = t.max(e.pdi_free_at);
                    self.schedule(at, Ev::PdReadStart { slave: p });
                }
            }
            Role::MbxWrite(_) => {
                if self.esc[p].sm0.is_none() {
                    self.esc[p].sm0 = Some(d.tx.clone());
                    d.wkc = 1;
                    let len = self.spec.slaves[p].mailbox_out_bytes as usize;
                    let msg = MailboxHeader::parse(&d.tx).map_or(len, |h| MAILBOX_HEADER_LEN + h.length as usize);
                    let done = self.pdi_mailbox(p, t, len, msg, false);
                    self.schedule(done, Ev::MbxReadDone { slave: p });
                }
            }
            Role::MbxRead(_) => {
                if let Some(buf) = self.esc[p].sm1.take() {
                    d.rx = buf;
                    d.wkc = 1;
                    self.slave_write(p);
                }
            }
            Role::Config | Role::AlStatus => {}
        }
    }

    /// The slave application writes its next outgoing message into "mailbox in" if it is free.
    fn slave_write(&mut self, p: usize) {
        let e = &self.esc[p];
        if e.sm1.is_some() || e.sm1_writing {
            return;
        }
        let Some(k) = self.active_mailbox[p] else { return };
        let Some(Kind::Mailbox { slave, .. }) = self.transfers[k].kind.as_mut() else { return };
        let Some((t, body)) = slave.next() else { return };
        let len = self.spec.slaves[p].mailbox_in_bytes as usize;
        let buf = Self::encode(&mut self.esc[p].counter, t, &body, len);
        self.esc[p].sm1_writing = true;
        let done = self.pdi_mailbox(p, self.now, len, MAILBOX_HEADER_LEN + body.len(), true);
        self.schedule(done, Ev::MbxWriteDone { slave: p, buf });
    }

    fn app_message(&mut self, p: usize, buf: &[u8]) {
        if let Some(k) = self.active_mailbox[p]
            && let Ok(h) = MailboxHeader::parse(buf)
            && let Some(Kind::Mailbox { slave, .. }) = self.transfers[k].kind.as_mut()
        {
            let end = (MAILBOX_HEADER_LEN + h.length as usize).min(buf.len());
            slave.on(h.mailbox_type, &buf[MAILBOX_HEADER_LEN..end], self.now);
        }
        self.slave_write(p);
        self.check_mailbox(p);
    }

    fn pd_read_start(&mut self, p: usize) {
        let e = &mut self.esc[p];
        e.pd_read_version = e.pd_out_version;
        let outputs = e.pd_out.clone();
        let r = self.regions[p];
        let done = self.pdi(p, self.now, r.out_len, false);
        self.schedule(done, Ev::PdReadDone { slave: p, outputs });
    }

    fn pd_read_done(&mut self, p: usize, outputs: Vec<u8>) {
        let r = self.regions[p];
        let now = self.now;
        let mut inputs = vec![0u8; r.in_len];
        for (j, b) in inputs[..r.in_len - r.chan_in].iter_mut().enumerate() {
            *b = (now / 1000) as u8 ^ j as u8;
        }
        if let Some(i) = self.channel_of[p]
            && let Some(Kind::Channel { sender, receiver, master_sends }) = self.transfers[i].kind.as_mut()
        {
            let chan_out = &outputs[outputs.len().saturating_sub(r.chan_out)..];
            let chan_in = &mut inputs[r.in_len - r.chan_in..];
            let mut new_chunks = 0;
            if *master_sends {
                receiver.on_chunk(chan_out, now);
                receiver.fill_ack(chan_in);
            } else {
                sender.on_ack(chan_out);
                let before = sender.chunk_updates;
                sender.fill(chan_in);
                new_chunks = sender.chunk_updates - before;
            }
            self.transfers[i].chunk_cycles += new_chunks;
            self.check_channel(p);
        }
        let ready = now + self.spec.slaves[p].app_processing_ns as u64;
        let done = if r.in_len > 0 { self.pdi(p, ready, r.in_len, true) } else { ready };
        self.schedule(done, Ev::PdWriteDone { slave: p, inputs });
        let e = &mut self.esc[p];
        e.pd_read_pending = false;
        if e.pd_out_version > e.pd_read_version {
            e.pd_read_pending = true;
            let at = now.max(e.pdi_free_at);
            self.schedule(at, Ev::PdReadStart { slave: p });
        }
    }

    // -- transfers -------------------------------------------------------------------------------

    fn all_finished(&self) -> bool {
        self.transfers.iter().all(|t| t.finished)
    }

    fn activate(&mut self, k: usize) {
        let all: &'a SimSpec = self.spec;
        let spec = &all.transfers[k];
        let p = spec.slave;
        let s = &all.slaves[p];
        let tr = &mut self.transfers[k];
        let (seed, len, now) = (tr.seed, spec.payload_bytes, self.now);
        let src = || Source::new(seed, len);
        let sink = || Sink::new(seed, len, now);
        let (out_len, in_len) = (s.mailbox_out_bytes as usize, s.mailbox_in_bytes as usize);
        let m2s = spec.direction == TransferDirection::MasterToSlave;
        let kind = match (&spec.protocol, m2s) {
            (TransferProtocol::Eoe { mtu_bytes }, true) => Kind::Mailbox {
                master: Box::new(EoeSender::new(src(), *mtu_bytes as usize, out_len, true)),
                slave: Box::new(EoeReceiver::new(sink())),
            },
            (TransferProtocol::Eoe { mtu_bytes }, false) => Kind::Mailbox {
                master: Box::new(EoeReceiver::new(sink())),
                slave: Box::new(EoeSender::new(src(), *mtu_bytes as usize, in_len, false)),
            },
            (TransferProtocol::Foe, true) => Kind::Mailbox {
                master: Box::new(FoeSender::new(src(), out_len, true)),
                slave: Box::new(FoeReceiver::new(sink(), out_len, false)),
            },
            (TransferProtocol::Foe, false) => Kind::Mailbox {
                master: Box::new(FoeReceiver::new(sink(), in_len, true)),
                slave: Box::new(FoeSender::new(src(), in_len, false)),
            },
            (TransferProtocol::CoeSegmented, true) => Kind::Mailbox {
                master: Box::new(SdoSender::new(src(), len, out_len, true)),
                slave: Box::new(SdoReceiver::new(sink(), false)),
            },
            (TransferProtocol::CoeSegmented, false) => Kind::Mailbox {
                master: Box::new(SdoReceiver::new(sink(), true)),
                slave: Box::new(SdoSender::new(src(), len, in_len, false)),
            },
            (TransferProtocol::PdoChannel { bytes_per_cycle, window, retransmit_after_cycles }, _) => Kind::Channel {
                sender: Box::new(ChannelSender::new(
                    seed,
                    len,
                    *bytes_per_cycle as usize,
                    *window,
                    *retransmit_after_cycles,
                )),
                receiver: Box::new(ChannelReceiver::new(sink())),
                master_sends: m2s,
            },
        };
        tr.started_at = Some(now);
        let channel = matches!(kind, Kind::Channel { .. });
        tr.kind = Some(kind);
        if channel {
            self.channel_of[p] = Some(k);
            self.check_channel(p);
        } else {
            self.active_mailbox[p] = Some(k);
            self.mbx[p].pending = None;
            self.slave_write(p);
            self.check_mailbox(p);
        }
    }

    fn try_activate_mailbox(&mut self, p: usize) {
        if self.active_mailbox[p].is_some() {
            return;
        }
        if let Some(&k) = self.mailbox_queue[p].front()
            && self.transfers[k].eligible
        {
            self.mailbox_queue[p].pop_front();
            self.activate(k);
        }
    }

    fn finish(&mut self, k: usize) {
        let tr = &mut self.transfers[k];
        tr.finished = true;
        if let Some(kind) = tr.kind.take() {
            match kind {
                Kind::Mailbox { master, slave } => {
                    tr.sink = master.sink().or(slave.sink()).cloned();
                }
                Kind::Channel { sender, receiver, .. } => {
                    tr.retransmitted = sender.retransmitted;
                    tr.sink = Some(receiver.sink);
                }
            }
        }
    }

    fn check_mailbox(&mut self, p: usize) {
        let Some(k) = self.active_mailbox[p] else { return };
        let done = match &self.transfers[k].kind {
            Some(Kind::Mailbox { master, slave }) => master.done() && slave.done() && self.mbx[p].pending.is_none(),
            _ => false,
        };
        if done {
            self.finish(k);
            self.active_mailbox[p] = None;
            self.try_activate_mailbox(p);
        }
    }

    fn check_channel(&mut self, p: usize) {
        let Some(k) = self.channel_of[p] else { return };
        let done = match &self.transfers[k].kind {
            Some(Kind::Channel { sender, receiver, .. }) => sender.done() && receiver.sink.complete(),
            _ => false,
        };
        if done {
            self.finish(k);
            self.channel_of[p] = None;
        }
    }

    // -- run --------------------------------------------------------------------------------------

    fn handle(&mut self, ev: Ev) -> Result<(), SimError> {
        match ev {
            Ev::Cycle(k) => self.cycle(k)?,
            Ev::MailboxFrame { cycle_start } => self.mailbox_frame(cycle_start)?,
            Ev::AtSlave { frame, dg, slave } => self.at_slave(frame, dg, slave),
            Ev::Return { frame } => self.on_return(frame)?,
            Ev::MbxReadDone { slave } => {
                let buf = self.esc[slave].sm0.take().unwrap_or_default();
                let at = self.now + self.spec.slaves[slave].app_processing_ns as u64;
                self.schedule(at, Ev::AppMessage { slave, buf });
            }
            Ev::AppMessage { slave, buf } => self.app_message(slave, &buf),
            Ev::MbxWriteDone { slave, buf } => {
                self.esc[slave].sm1 = Some(buf);
                self.esc[slave].sm1_writing = false;
            }
            Ev::PdReadStart { slave } => self.pd_read_start(slave),
            Ev::PdReadDone { slave, outputs } => self.pd_read_done(slave, outputs),
            Ev::PdWriteDone { slave, inputs } => self.esc[slave].pd_in = inputs,
            Ev::TransferStart { index } => {
                self.transfers[index].eligible = true;
                if matches!(self.spec.transfers[index].protocol, TransferProtocol::PdoChannel { .. }) {
                    self.activate(index);
                } else {
                    self.try_activate_mailbox(self.spec.transfers[index].slave);
                }
            }
        }
        Ok(())
    }

    /// Configuration frames so the capture describes itself: count, addresses, mailbox
    /// SyncManagers, AL state OP. Not part of the timing model.
    fn preamble(&mut self) -> Result<u64, SimError> {
        let spec: &'a SimSpec = self.spec;
        let n = spec.slaves.len() as u16;
        let mut t = 0;
        let mut send = |me: &mut Self, dgs: Vec<Dg>| -> Result<(), SimError> {
            t += PREAMBLE_STEP_NS;
            me.send(t, dgs, None)
        };
        send(self, vec![Dg::config(Command::Brd, 0, vec![0], n)])?;
        for p in 0..n {
            let a = (p.wrapping_neg() as u32) | 0x0010 << 16;
            send(self, vec![Dg::config(Command::Apwr, a, station(p as usize).to_le_bytes().to_vec(), 1)])?;
        }
        for (p, s) in spec.slaves.iter().enumerate() {
            if s.mailbox_out_bytes == 0 {
                continue;
            }
            let sm = |start: u16, length: u16, control: u8| {
                SyncManagerConfig { start, length, control, status: 0, activate: 1, pdi_control: 0 }.encode()
            };
            let mut data = sm(MBX_BASE, s.mailbox_out_bytes, 0x26).to_vec();
            data.extend_from_slice(&sm(MBX_BASE + s.mailbox_out_bytes, s.mailbox_in_bytes, 0x22));
            let a = station(p) as u32 | 0x0800 << 16;
            send(self, vec![Dg::config(Command::Fpwr, a, data, 1)])?;
        }
        for p in 0..n as usize {
            let a = station(p) as u32 | 0x0120 << 16;
            send(self, vec![Dg::config(Command::Fpwr, a, vec![AlState::Op.code(), 0], 1)])?;
            let a = station(p) as u32 | 0x0130 << 16;
            send(self, vec![Dg::config(Command::Fprd, a, vec![AlState::Op.code(), 0], 1)])?;
        }
        Ok(t)
    }

    fn run(&mut self) -> Result<(), SimError> {
        let end_of_preamble = self.preamble()?;
        // Let the preamble frames return before the first cycle.
        while let Some(Reverse(e)) = self.queue.pop() {
            self.now = e.t;
            self.capture.flush_until(self.now)?;
            self.handle(e.ev)?;
        }
        self.t0 = (end_of_preamble + self.cycle_ns).div_ceil(self.cycle_ns) * self.cycle_ns;
        for k in 0..self.transfers.len() {
            let at = self.t0 + (self.spec.transfers[k].start_ms * 1e6) as u64;
            self.schedule(at, Ev::TransferStart { index: k });
        }
        self.schedule(self.t0, Ev::Cycle(0));
        while let Some(Reverse(e)) = self.queue.pop() {
            self.now = e.t;
            self.capture.flush_until(self.now)?;
            self.handle(e.ev)?;
        }
        self.capture.flush_until(u64::MAX)?;
        Ok(())
    }
}

fn assumptions() -> Vec<String> {
    vec![
        "Model, not measurement: every number follows from the spec's parameters.".into(),
        "Link: frames are serialized at link_bps with preamble, FCS, padding and inter-frame gap; each slave forwards with a fixed delay; frame loss (if configured) happens before the first slave.".into(),
        "ESC: mailboxes are 1-buffer SyncManagers; a write is accepted only while \"mailbox out\" is empty, a read returns data only while \"mailbox in\" is full. Process data uses 3-buffer SyncManagers: the application reads the newest outputs when its PDI is free.".into(),
        "PDI: the slave application moves mailbox data (the whole SyncManager buffer, or only the message plus the triggering last byte, per pdi.mailbox_access) and process data at the configured rate plus a fixed overhead per access, one access at a time; mailbox and process data compete for it.".into(),
        "Master: cycle-driven; it learns whether a mailbox write was accepted from the returning working counter and retries otherwise; master CPU and operating-system delays are not modelled beyond the configured cycle jitter.".into(),
        "EoE carries UDP/IPv4 datagrams: a simplified tunnel without TCP, ARP or IP fragmentation.".into(),
        "Payload bytes are generated deterministically and checked byte for byte and by SHA-256 at the receiver.".into(),
        "Ethernet reference: TCP over a dedicated link at the same rate, MSS 1460 in 1538 wire bytes per segment, steady state, no loss, no slow start; a lower bound on plain-Ethernet time, not a prediction.".into(),
    ]
}

/// Run the simulation; frames go to `capture` as pcapng if given and enabled in the spec.
pub fn simulate<W: Write>(spec: &SimSpec, capture: Option<W>) -> Result<SimResult, SimError> {
    spec.validate().map_err(SimError::Spec)?;
    let spec_json = serde_json::to_vec(spec).map_err(|e| SimError::Spec(e.to_string()))?;
    let spec_sha = flowlab_core::sha256_hex(&spec_json);
    let writer = match capture.filter(|_| spec.capture.enabled) {
        Some(w) => {
            let comment = format!(
                "{}{}; generator=flowlab-sim {}; seed={}; spec_sha256={}; {}",
                Provenance::COMMENT_PREFIX,
                Provenance::Simulated.as_str(),
                env!("CARGO_PKG_VERSION"),
                spec.seed,
                spec_sha,
                spec.description
            );
            Some(PcapNgWriter::new(w, concat!("flowlab-sim ", env!("CARGO_PKG_VERSION")), &comment, "simulated0")?)
        }
        None => None,
    };

    let n = spec.slaves.len();
    let mut regions = Vec::with_capacity(n);
    let mut at = 0;
    for (p, s) in spec.slaves.iter().enumerate() {
        let (co, ci) = s.channel_bytes(p, &spec.transfers);
        let out_len = s.outputs_bytes as usize + co;
        let in_len = s.inputs_bytes as usize + ci;
        regions.push(Region { out_at: at, out_len, in_at: at + out_len, in_len, chan_out: co, chan_in: ci });
        at += out_len + in_len;
    }
    let mut mailbox_queue = vec![VecDeque::new(); n];
    for (k, t) in spec.transfers.iter().enumerate() {
        if !matches!(t.protocol, TransferProtocol::PdoChannel { .. }) {
            mailbox_queue[t.slave].push_back(k);
        }
    }
    let transfers = (0..spec.transfers.len())
        .map(|k| Transfer {
            first_msg_ns: None,
            last_msg_ns: None,
            seed: transfer_seed(spec.seed, k),
            eligible: false,
            started_at: None,
            kind: None,
            finished: false,
            counts: Counts::default(),
            retransmitted: 0,
            chunk_cycles: 0,
            sink: None,
        })
        .collect();
    let mut sim = Sim {
        spec,
        queue: BinaryHeap::new(),
        seq: 0,
        now: 0,
        rng: SplitMix64::new(spec.seed),
        link_free: 0,
        flights: HashMap::new(),
        next_frame: 0,
        index: 0,
        frames: 0,
        wire_bytes: 0,
        cycles: 0,
        cycle_start: 0,
        t0: 0,
        cycle_ns: spec.cycle_time_us as u64 * 1000,
        regions,
        image_len: at,
        mbx: (0..n).map(|_| MasterMbx::default()).collect(),
        esc: (0..n).map(|_| Esc::default()).collect(),
        transfers,
        mailbox_queue,
        active_mailbox: vec![None; n],
        channel_of: vec![None; n],
        rr: 0,
        capture: Capture {
            writer,
            pending: BinaryHeap::new(),
            seq: 0,
            written: 0,
            max: spec.capture.max_frames,
            truncated: false,
            t0_abs: spec.start_unix_s * 1_000_000_000,
        },
        stop: false,
    };
    sim.run()?;
    if let Some(w) = sim.capture.writer.take() {
        w.finish()?;
    }

    let virtual_time = sim.now.max(1);
    let link_bps = spec.link_bps as f64;
    let transfers = spec
        .transfers
        .iter()
        .enumerate()
        .map(|(k, t)| {
            let tr = &sim.transfers[k];
            let sink = tr.sink.as_ref().or(match &tr.kind {
                Some(Kind::Mailbox { master, slave }) => master.sink().or(slave.sink()),
                Some(Kind::Channel { receiver, .. }) => Some(&receiver.sink),
                None => None,
            });
            let sha = payload_sha256(tr.seed, t.payload_bytes);
            let start = tr.started_at.map(|s| s as f64);
            let end = sink.and_then(|s| s.end_ns).map(|e| e as f64);
            let duration = start.zip(end).map(|(s, e)| e - s);
            let delivered = sink.map_or(0, |s| s.got);
            let verified = sink.is_some_and(|s| s.complete() && !s.corrupt && s.sha256() == sha);
            let bits = t.payload_bytes as f64 * 8.0;
            TransferResult {
                index: k as u32,
                station: station(t.slave),
                slave: spec.slaves[t.slave].name.clone(),
                protocol: t.protocol.name().into(),
                direction: t.direction.as_str().into(),
                payload_bytes: t.payload_bytes,
                delivered_bytes: delivered,
                verified,
                sha256: sha,
                completed: tr.finished,
                start_ns: start.unwrap_or(0.0),
                first_byte_ns: sink.and_then(|s| s.first_ns).map(|v| v as f64),
                end_ns: end,
                duration_ns: duration,
                exchange_ns: tr.first_msg_ns.zip(tr.last_msg_ns).map(|(a, b)| (b - a) as f64),
                goodput_bps: duration.filter(|d| *d > 0.0).map(|d| bits / (d / 1e9)),
                messages_to_slave: tr.counts.to_slave,
                messages_from_slave: tr.counts.from_slave,
                rejected_writes: tr.counts.rejected,
                empty_reads: tr.counts.empty,
                retransmitted_chunks: tr.retransmitted,
                chunk_cycles: tr.chunk_cycles,
                physical_bound_ns: flowlab_mailbox::physical_bound_ns(t.payload_bytes, spec.link_bps),
                ethernet_tcp_reference_ns: t.payload_bytes.div_ceil(1460) as f64 * 1538.0 * 8.0 / link_bps * 1e9,
            }
        })
        .collect::<Vec<_>>();
    let slaves = spec
        .slaves
        .iter()
        .enumerate()
        .map(|(p, s)| SlaveResult {
            station: station(p),
            name: s.name.clone(),
            pdi_busy_percent: 100.0 * sim.esc[p].pdi_busy_ns as f64 / virtual_time as f64,
            dpram_used_bytes: s.dpram_needed(p, &spec.transfers),
            dpram_bytes: s.dpram_bytes,
        })
        .collect();
    let input =
        InputRef { name: format!("sim-seed{}.json", spec.seed), bytes: spec_json.len() as u64, sha256: spec_sha };
    Ok(SimResult {
        meta: ReportMeta::new(Provenance::Simulated, input, assumptions()),
        description: spec.description.clone(),
        seed: spec.seed,
        policy: spec.mailbox_policy.to_string(),
        cycle_time_ns: sim.cycle_ns as f64,
        link_bps,
        virtual_time_ns: virtual_time as f64,
        cycles: sim.cycles,
        frames: sim.frames,
        wire_bytes: sim.wire_bytes,
        link_utilization_percent: 100.0 * sim.wire_bytes as f64 * 8.0 / (link_bps * virtual_time as f64 / 1e9),
        all_completed: sim.transfers.iter().all(|t| t.finished),
        transfers,
        slaves,
        capture_frames: sim.capture.written,
        capture_truncated: sim.capture.truncated,
        spec: spec.clone(),
    })
}
