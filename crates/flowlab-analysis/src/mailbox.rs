//! Mailbox analysis: find each slave's mailbox SyncManagers, extract the messages that went
//! through them, and follow CoE SDO transfers, FoE file transfers and EoE fragment streams.
//!
//! A mailbox write is accepted when it returns with WKC > 0; WKC 0 means the slave's mailbox
//! was still full. A mailbox read returns a message when WKC > 0; WKC 0 means it was empty.
//! Messages are therefore taken from *returning* datagrams.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use ecat_wire::{
    CoeEmergency, CoeHeader, CoeService, DatagramAddress, EoeHeader, EoeType, FoeHeader, FoeOpcode, MAILBOX_HEADER_LEN,
    MailboxErrorReply, MailboxHeader, MailboxType, SM_REGISTER_BASE, SM_REGISTER_LEN, SdoCommand, SdoHeader,
    SdoInfoHeader, SmDirection, SmMode, SoeHeader, VoeHeader, foe_error_text, sdo_abort_text, segment_data_len,
    sync_managers_in,
};
use flowlab_capture::{DatagramRecord, Direction, FrameKind, Trace};

use crate::report::*;
use crate::stats::Distribution;

const MAX_LIST: usize = 2000;

/// Mailbox SyncManager addresses of a station, given by the user when the capture does not
/// show the SyncManager configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MailboxHint {
    /// Configured station address.
    pub station: u16,
    /// Start of the master → slave mailbox (SM0 by convention).
    pub out_start: u16,
    /// Start of the slave → master mailbox (SM1 by convention).
    pub in_start: u16,
}

impl std::str::FromStr for MailboxHint {
    type Err = String;

    /// `STATION=OUT,IN`, numbers decimal or `0x` hex, e.g. `0x1001=0x1000,0x1080`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let num = |t: &str| -> Result<u16, String> {
            let t = t.trim();
            let r = match t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
                Some(h) => u16::from_str_radix(h, 16),
                None => t.parse(),
            };
            r.map_err(|_| format!("not a 16-bit number: {t:?}"))
        };
        let (st, rest) = s.split_once('=').ok_or("expected STATION=OUT,IN")?;
        let (o, i) = rest.split_once(',').ok_or("expected STATION=OUT,IN")?;
        Ok(Self { station: num(st)?, out_start: num(o)?, in_start: num(i)? })
    }
}

/// How a datagram touches a mailbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Write to the master → slave mailbox.
    Out,
    /// Read of the slave → master mailbox.
    In,
    /// Read of the "mailbox in" SyncManager status register.
    Status,
}

#[derive(Debug, Clone)]
struct ChannelInfo {
    source: ChannelSource,
    out: BTreeMap<u16, u16>,
    inn: BTreeMap<u16, u16>,
    /// Physical addresses of the "mailbox in" SyncManagers' status registers.
    status: BTreeSet<u16>,
}

impl ChannelInfo {
    fn new(source: ChannelSource) -> Self {
        Self { source, out: BTreeMap::new(), inn: BTreeMap::new(), status: BTreeSet::new() }
    }
}

/// Mailbox SyncManager addresses per station.
#[derive(Debug, Clone, Default)]
pub struct Channels {
    map: BTreeMap<u16, ChannelInfo>,
}

impl Channels {
    /// How datagram `d` accesses a mailbox, if it does.
    pub fn access(&self, d: &DatagramRecord) -> Option<Access> {
        let DatagramAddress::Fixed { station, ado } = d.address() else { return None };
        let c = self.map.get(&station)?;
        if d.command.writes() && c.out.contains_key(&ado) {
            Some(Access::Out)
        } else if d.command.reads() && !d.command.writes() && c.inn.contains_key(&ado) {
            Some(Access::In)
        } else if d.command.reads()
            && c.status.iter().any(|&s| s >= ado && (s as u32) < ado as u32 + d.len as u32)
            && !c.inn.contains_key(&ado)
        {
            Some(Access::Status)
        } else {
            None
        }
    }

    /// Source of the channel of `station`.
    pub fn source(&self, station: u16) -> Option<ChannelSource> {
        self.map.get(&station).map(|c| c.source)
    }

    fn detect(
        trace: &Trace,
        order: &[u32],
        hints: &[MailboxHint],
        network: Option<&flowlab_devices::NetworkConfig>,
        directions_known: bool,
    ) -> Self {
        let mut map: BTreeMap<u16, ChannelInfo> = BTreeMap::new();
        let mut positions: HashMap<u16, u16> = HashMap::new();
        let mut by_position: Vec<(u16, u8, ecat_wire::SyncManagerConfig)> = Vec::new();
        let sm_end = SM_REGISTER_BASE as u32 + 16 * SM_REGISTER_LEN as u32;
        for &i in order {
            let f = &trace.frames[i as usize];
            if f.kind != FrameKind::Ethercat || (directions_known && f.direction != Direction::Tx) {
                continue;
            }
            for d in trace.datagrams_of(f) {
                if !d.command.writes() {
                    continue;
                }
                let addr = d.address();
                let data = trace.datagram_data(d);
                if let DatagramAddress::Position { ado: 0x0010, .. } = addr
                    && data.len() >= 2
                    && let Some(p) = addr.position()
                {
                    positions.insert(p, u16::from_le_bytes([data[0], data[1]]));
                }
                let Some(ado) = addr.ado() else { continue };
                if (ado as u32) >= sm_end || ado as u32 + d.len as u32 <= SM_REGISTER_BASE as u32 {
                    continue;
                }
                for (n, sm) in sync_managers_in(ado, data) {
                    match addr {
                        DatagramAddress::Fixed { station, .. } => add_sm(&mut map, station, n, sm),
                        DatagramAddress::Position { .. } => {
                            let p = addr.position().unwrap_or(0);
                            match positions.get(&p) {
                                Some(&st) => add_sm(&mut map, st, n, sm),
                                None => by_position.push((p, n, sm)),
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        for (p, n, sm) in by_position {
            if let Some(&st) = positions.get(&p) {
                add_sm(&mut map, st, n, sm);
            }
        }
        for s in network.map(|n| n.slaves.as_slice()).unwrap_or_default() {
            let Some(m) = &s.mailbox else { continue };
            map.entry(s.station).or_insert_with(|| {
                let mut c = ChannelInfo::new(ChannelSource::Eni);
                c.out.insert(m.out_start, m.out_len);
                c.inn.insert(m.in_start, m.in_len);
                // ENI does not say which SyncManager is "mailbox in"; SM1 by convention.
                c.status.insert(SM_REGISTER_BASE + SM_REGISTER_LEN + 5);
                c
            });
        }
        for h in hints {
            let c = map.entry(h.station).or_insert_with(|| ChannelInfo::new(ChannelSource::Hint));
            if c.source == ChannelSource::Hint {
                c.out.entry(h.out_start).or_insert(0);
                c.inn.entry(h.in_start).or_insert(0);
            }
        }
        heuristic(trace, order, &mut map, directions_known);
        Self { map }
    }
}

fn add_sm(map: &mut BTreeMap<u16, ChannelInfo>, station: u16, n: u8, sm: ecat_wire::SyncManagerConfig) {
    if sm.mode() != SmMode::Mailbox || sm.length == 0 {
        return;
    }
    let c = map.entry(station).or_insert_with(|| ChannelInfo::new(ChannelSource::SmConfig));
    c.source = ChannelSource::SmConfig;
    match sm.direction() {
        SmDirection::EcatWrite => {
            c.out.insert(sm.start, sm.length);
        }
        SmDirection::EcatRead => {
            c.inn.insert(sm.start, sm.length);
            c.status.insert(SM_REGISTER_BASE + n as u16 * SM_REGISTER_LEN + 5);
        }
        SmDirection::Reserved => {}
    }
}

/// Plausible mailbox header filling at most `data`.
fn plausible(data: &[u8]) -> Option<MailboxHeader> {
    let h = MailboxHeader::parse(data).ok()?;
    let ok = h.length > 0
        && h.length as usize + MAILBOX_HEADER_LEN <= data.len()
        && !matches!(h.mailbox_type, MailboxType::Reserved(_));
    ok.then_some(h)
}

/// Stations without configuration or hint: FPWR at ≥ 0x1000 carrying a plausible mailbox
/// header, accepted by the slave, defines "mailbox out"; then FPRD at ≥ 0x1000 returning a
/// plausible header defines "mailbox in".
fn heuristic(trace: &Trace, order: &[u32], map: &mut BTreeMap<u16, ChannelInfo>, directions_known: bool) {
    let known: BTreeSet<u16> = map.keys().copied().collect();
    let mut found: BTreeMap<u16, ChannelInfo> = BTreeMap::new();
    for pass_reads in [false, true] {
        for &i in order {
            let f = &trace.frames[i as usize];
            if f.kind != FrameKind::Ethercat || (directions_known && f.direction != Direction::Rx) {
                continue;
            }
            for d in trace.datagrams_of(f) {
                let DatagramAddress::Fixed { station, ado } = d.address() else { continue };
                if ado < 0x1000 || d.wkc == 0 || known.contains(&station) {
                    continue;
                }
                let data = trace.datagram_data(d);
                if plausible(data).is_none() {
                    continue;
                }
                let writes = d.command.writes() && !d.command.reads();
                let reads = d.command.reads() && !d.command.writes();
                if !pass_reads && writes {
                    found
                        .entry(station)
                        .or_insert_with(|| ChannelInfo::new(ChannelSource::Heuristic))
                        .out
                        .insert(ado, d.len);
                } else if pass_reads
                    && reads
                    && let Some(c) = found.get_mut(&station)
                    && !c.out.contains_key(&ado)
                {
                    c.inn.insert(ado, d.len);
                }
            }
        }
    }
    map.extend(found);
}

/// A mailbox message extracted from the capture.
#[derive(Debug, Clone)]
pub struct MessageRecord {
    /// Frame index (0-based) of the returning datagram.
    pub frame: u32,
    /// Datagram position.
    pub datagram: u16,
    /// Station.
    pub station: u16,
    /// Direction.
    pub direction: MailboxDirection,
    /// Protocol.
    pub protocol: MailboxType,
    /// One-line description.
    pub summary: String,
    /// Dropped as a repeat of the previous message.
    pub repeat: bool,
}

/// Mailbox results plus the side tables used to decorate frames.
#[derive(Debug, Default)]
pub struct MailboxAnalysis {
    /// Channels used for classification.
    pub channels: Channels,
    /// Extracted messages in capture order.
    pub messages: Vec<MessageRecord>,
    /// (frame index, datagram position) → message index, for both copies of a write.
    pub by_datagram: HashMap<(u32, u16), u32>,
    /// Per frame: bit `t` set when a message of mailbox type `t` is in it; bit 16 when any
    /// datagram accesses a mailbox.
    pub frame_flags: Vec<u32>,
    /// Ethernet frames reassembled from EoE: (absolute time ns, bytes).
    pub eoe_packets: Vec<(u64, Vec<u8>)>,
}

/// Bit in [`MailboxAnalysis::frame_flags`] for any mailbox access.
pub const ANY_MAILBOX: u32 = 1 << 16;

pub(crate) fn protocol_name(t: MailboxType) -> &'static str {
    match t {
        MailboxType::Error => "Error",
        MailboxType::Aoe => "AoE",
        MailboxType::Eoe => "EoE",
        MailboxType::Coe => "CoE",
        MailboxType::Foe => "FoE",
        MailboxType::Soe => "SoE",
        MailboxType::Voe => "VoE",
        MailboxType::Reserved(_) => "Reserved",
    }
}

#[derive(Default)]
struct ChannelAcc {
    writes: u64,
    writes_rejected: u64,
    reads: u64,
    empty_reads: u64,
    status_polls: u64,
    messages_out: u64,
    messages_in: u64,
    bytes_out: u64,
    bytes_in: u64,
    repeats: u64,
    poll_times: Vec<f64>,
    protocols: BTreeSet<&'static str>,
    last_counter: [Option<u8>; 2],
}

struct Ctx {
    t_ns: f64,
    frame: u32,
    station: u16,
    direction: MailboxDirection,
    /// Bytes of the datagram's data (the SyncManager buffer as transferred).
    buffer_len: usize,
    start_ts_ns: u64,
}

pub(crate) fn analyze(
    trace: &Trace,
    order: &[u32],
    pair_of: &[u32],
    hints: &[MailboxHint],
    network: Option<&flowlab_devices::NetworkConfig>,
    directions_known: bool,
    bins: usize,
) -> (MailboxAnalysis, MailboxReport) {
    let channels = Channels::detect(trace, order, hints, network, directions_known);
    let mut out = MailboxAnalysis { frame_flags: vec![0; trace.frames.len()], ..Default::default() };
    let mut acc: BTreeMap<u16, ChannelAcc> = channels.map.keys().map(|&s| (s, ChannelAcc::default())).collect();
    let mut protocols: BTreeMap<&'static str, ProtocolStats> = BTreeMap::new();
    let mut coe = CoeTracker::default();
    let mut foe = FoeTracker::default();
    let mut eoe = EoeTracker::default();
    let mut soe_errors = 0u64;
    let mut errors = Vec::new();
    let mut malformed = 0u64;

    for &i in order {
        let f = &trace.frames[i as usize];
        if f.kind != FrameKind::Ethercat {
            continue;
        }
        for (p, d) in trace.datagrams_of(f).iter().enumerate() {
            let Some(access) = channels.access(d) else { continue };
            let DatagramAddress::Fixed { station, .. } = d.address() else { continue };
            out.frame_flags[i as usize] |= ANY_MAILBOX;
            let a = acc.entry(station).or_default();
            let t = f.t_ns as f64;
            let returned = match f.direction {
                Direction::Rx => true,
                Direction::Tx => false,
                Direction::Unknown => !directions_known || d.wkc > 0,
            };
            match (access, returned) {
                (Access::Status, false) => a.status_polls += 1,
                (Access::Status, true) if !directions_known => a.status_polls += 1,
                (Access::In, false) => a.poll_times.push(t),
                (Access::Out, true) | (Access::In, true) if d.wkc == 0 => {
                    if directions_known {
                        if access == Access::Out {
                            a.writes_rejected += 1;
                        } else {
                            a.empty_reads += 1;
                        }
                    }
                }
                (Access::Out, true) | (Access::In, true) => {
                    let direction =
                        if access == Access::Out { MailboxDirection::ToSlave } else { MailboxDirection::FromSlave };
                    if access == Access::Out {
                        a.writes += 1;
                    } else {
                        a.reads += 1;
                    }
                    let data = trace.datagram_data(d);
                    let Some(h) = plausible(data) else {
                        malformed += 1;
                        continue;
                    };
                    let body = &data[MAILBOX_HEADER_LEN..MAILBOX_HEADER_LEN + h.length as usize];
                    let slot = direction as usize;
                    let repeat = h.counter != 0 && a.last_counter[slot] == Some(h.counter);
                    a.last_counter[slot] = Some(h.counter);
                    let name = protocol_name(h.mailbox_type);
                    let msg_idx = out.messages.len() as u32;
                    out.messages.push(MessageRecord {
                        frame: i,
                        datagram: p as u16,
                        station,
                        direction,
                        protocol: h.mailbox_type,
                        summary: describe(&h, body).0,
                        repeat,
                    });
                    let flag = 1u32 << (h.mailbox_type.code() & 0x0F);
                    out.frame_flags[i as usize] |= flag;
                    out.by_datagram.insert((i, p as u16), msg_idx);
                    let partner = pair_of[i as usize];
                    if partner != u32::MAX {
                        out.frame_flags[partner as usize] |= flag | ANY_MAILBOX;
                        if direction == MailboxDirection::ToSlave {
                            out.by_datagram.insert((partner, p as u16), msg_idx);
                        }
                    }
                    if repeat {
                        a.repeats += 1;
                        continue;
                    }
                    a.protocols.insert(name);
                    let ps = protocols.entry(name).or_insert_with(|| ProtocolStats {
                        protocol: name.to_owned(),
                        messages_out: 0,
                        messages_in: 0,
                        bytes_out: 0,
                        bytes_in: 0,
                        first_ns: t,
                        last_ns: t,
                    });
                    ps.last_ns = t;
                    if direction == MailboxDirection::ToSlave {
                        a.messages_out += 1;
                        a.bytes_out += h.length as u64;
                        ps.messages_out += 1;
                        ps.bytes_out += h.length as u64;
                    } else {
                        a.messages_in += 1;
                        a.bytes_in += h.length as u64;
                        ps.messages_in += 1;
                        ps.bytes_in += h.length as u64;
                    }
                    let ctx = Ctx {
                        t_ns: t,
                        frame: i + 1,
                        station,
                        direction,
                        buffer_len: data.len(),
                        start_ts_ns: trace.start_ts_ns,
                    };
                    match h.mailbox_type {
                        MailboxType::Coe => coe.on(&ctx, &h, body),
                        MailboxType::Foe => foe.on(&ctx, &h, body),
                        MailboxType::Eoe => eoe.on(&ctx, body, &mut out.eoe_packets),
                        MailboxType::Soe => {
                            if SoeHeader::parse(body).is_ok_and(|s| s.error) {
                                soe_errors += 1;
                            }
                        }
                        MailboxType::Error => {
                            if let Ok(e) = MailboxErrorReply::parse(body)
                                && errors.len() < MAX_LIST
                            {
                                errors.push(MailboxErrorEvent {
                                    t_ns: t,
                                    frame: i + 1,
                                    station,
                                    detail: e.detail,
                                    text: e.detail_text().to_owned(),
                                });
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }

    let channels_report = acc
        .into_iter()
        .filter_map(|(station, a)| {
            let c = channels.map.get(&station)?;
            let first = |m: &BTreeMap<u16, u16>| m.iter().next().map(|(s, l)| (*s, *l));
            let (o, n) = (first(&c.out), first(&c.inn));
            let mut iv: Vec<f64> = a.poll_times.windows(2).map(|w| w[1] - w[0]).collect();
            iv.retain(|v| *v > 0.0);
            Some(MailboxChannel {
                station,
                source: c.source,
                out_start: o.map(|x| x.0),
                out_len: o.map(|x| x.1).filter(|l| *l > 0),
                in_start: n.map(|x| x.0),
                in_len: n.map(|x| x.1).filter(|l| *l > 0),
                writes: a.writes,
                writes_rejected: a.writes_rejected,
                reads: a.reads,
                empty_reads: a.empty_reads,
                status_polls: a.status_polls,
                messages_out: a.messages_out,
                messages_in: a.messages_in,
                bytes_out: a.bytes_out,
                bytes_in: a.bytes_in,
                repeats: a.repeats,
                poll_interval: Distribution::of(&iv, bins),
                protocols: a.protocols.into_iter().map(str::to_owned).collect(),
                service: None,
            })
        })
        .collect();
    let report = MailboxReport {
        channels: channels_report,
        protocols: protocols.into_values().collect(),
        coe: coe.finish(bins),
        foe: foe.finish(),
        eoe: eoe.finish(),
        soe_errors,
        errors,
        malformed,
    };
    out.channels = channels;
    (out, report)
}

// ---------------------------------------------------------------------------------------------
// CoE
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expect {
    InitResponse,
    UploadSegRequest,
    UploadSegResponse,
    DownloadSegRequest,
    DownloadSegResponse,
}

struct OpenSdo {
    t: SdoTransaction,
    expect: Expect,
    toggle: bool,
    last_pending: bool,
}

#[derive(Default)]
struct CoeTracker {
    open: HashMap<u16, OpenSdo>,
    done: Vec<SdoTransaction>,
    emergencies: Vec<EmergencyEvent>,
    emergencies_total: u64,
    info: u64,
}

fn data_after_sdo_header(body: &[u8]) -> u64 {
    body.len().saturating_sub(CoeHeader::LEN + SdoHeader::LEN) as u64
}

impl CoeTracker {
    fn close(&mut self, station: u16, ctx: &Ctx, outcome: SdoOutcome, note: Option<String>) {
        if let Some(mut o) = self.open.remove(&station) {
            if matches!(outcome, SdoOutcome::Ok | SdoOutcome::Aborted | SdoOutcome::ProtocolError) {
                o.t.end_ns = Some(ctx.t_ns);
                o.t.last_frame = ctx.frame;
            }
            o.t.outcome = outcome;
            if o.t.note.is_none() {
                o.t.note = note;
            }
            self.done.push(o.t);
        }
    }

    fn on(&mut self, ctx: &Ctx, h: &MailboxHeader, body: &[u8]) {
        let Ok(c) = CoeHeader::parse(body) else { return };
        let rest = &body[CoeHeader::LEN..];
        match c.service {
            CoeService::Emergency => {
                self.emergencies_total += 1;
                if let Ok(e) = CoeEmergency::parse(rest)
                    && self.emergencies.len() < MAX_LIST
                {
                    self.emergencies.push(EmergencyEvent {
                        t_ns: ctx.t_ns,
                        frame: ctx.frame,
                        station: ctx.station,
                        error_code: e.error_code,
                        error_register: e.error_register,
                        data: e.data.iter().map(|b| format!("{b:02x}")).collect(),
                    });
                }
            }
            CoeService::SdoInformation => self.info += 1,
            CoeService::SdoRequest | CoeService::SdoResponse => {
                let Some(&byte) = rest.first() else { return };
                let cmd = SdoCommand::decode(byte, c.service == CoeService::SdoRequest);
                self.sdo(ctx, h, body, cmd);
            }
            _ => {}
        }
    }

    fn sdo(&mut self, ctx: &Ctx, h: &MailboxHeader, body: &[u8], cmd: SdoCommand) {
        let st = ctx.station;
        let sdo = SdoHeader::parse(&body[CoeHeader::LEN..]).ok();
        let expect = self.open.get(&st).map(|o| o.expect);
        let protocol_error = |me: &mut Self, what: String| me.close(st, ctx, SdoOutcome::ProtocolError, Some(what));
        match cmd {
            SdoCommand::InitiateUploadRequest { complete_access }
            | SdoCommand::InitiateDownloadRequest { complete_access, .. } => {
                let Some(sdo) = sdo else { return };
                self.close(st, ctx, SdoOutcome::Superseded, None);
                let download = matches!(cmd, SdoCommand::InitiateDownloadRequest { .. });
                let (transfer, size, bytes) = match cmd {
                    SdoCommand::InitiateDownloadRequest { expedited: true, size_indicated, unused, .. } => {
                        let n = if size_indicated { 4 - unused as u32 } else { 4 };
                        ("expedited", Some(n), n as u64)
                    }
                    SdoCommand::InitiateDownloadRequest { .. } => {
                        ("normal", Some(sdo.data), data_after_sdo_header(body).min(sdo.data as u64))
                    }
                    _ => ("expedited", None, 0),
                };
                let t = SdoTransaction {
                    station: st,
                    kind: if download { "download" } else { "upload" }.into(),
                    index: sdo.index,
                    subindex: sdo.subindex,
                    complete_access,
                    transfer: transfer.into(),
                    size,
                    bytes,
                    segments: 0,
                    start_ns: ctx.t_ns,
                    end_ns: None,
                    first_frame: ctx.frame,
                    last_frame: ctx.frame,
                    outcome: SdoOutcome::Incomplete,
                    abort_code: None,
                    abort_text: None,
                    aborted_by: None,
                    note: None,
                };
                self.open.insert(st, OpenSdo { t, expect: Expect::InitResponse, toggle: false, last_pending: false });
            }
            SdoCommand::Abort => {
                let code = sdo.map(|s| s.data).unwrap_or(0);
                let by = if ctx.direction == MailboxDirection::ToSlave { "master" } else { "slave" };
                let s = sdo.unwrap_or(SdoHeader { command: 0x80, index: 0, subindex: 0, data: code });
                let o = self.open.entry(st).or_insert_with(|| OpenSdo {
                    t: SdoTransaction {
                        station: st,
                        kind: "unknown".into(),
                        index: s.index,
                        subindex: s.subindex,
                        complete_access: false,
                        transfer: "expedited".into(),
                        size: None,
                        bytes: 0,
                        segments: 0,
                        start_ns: ctx.t_ns,
                        end_ns: None,
                        first_frame: ctx.frame,
                        last_frame: ctx.frame,
                        outcome: SdoOutcome::Aborted,
                        abort_code: None,
                        abort_text: None,
                        aborted_by: None,
                        note: Some("abort without a preceding initiate request in the capture".into()),
                    },
                    expect: Expect::InitResponse,
                    toggle: false,
                    last_pending: false,
                });
                o.t.abort_code = Some(code);
                o.t.abort_text = sdo_abort_text(code).map(str::to_owned);
                o.t.aborted_by = Some(by.into());
                self.close(st, ctx, SdoOutcome::Aborted, None);
            }
            SdoCommand::InitiateUploadResponse { expedited, size_indicated, unused, .. } => {
                if expect != Some(Expect::InitResponse) || self.open[&st].t.kind != "upload" {
                    return protocol_error(self, "unexpected initiate upload response".into());
                }
                let Some(sdo) = sdo else { return };
                let o = self.open.get_mut(&st).expect("checked");
                if (sdo.index, sdo.subindex) != (o.t.index, o.t.subindex) {
                    return protocol_error(self, "response index/subindex differ from the request".into());
                }
                if expedited {
                    let n = if size_indicated { 4 - unused as u32 } else { 4 };
                    o.t.size = Some(n);
                    o.t.bytes = n as u64;
                    self.close(st, ctx, SdoOutcome::Ok, None);
                } else {
                    o.t.size = Some(sdo.data);
                    o.t.bytes = data_after_sdo_header(body).min(sdo.data as u64);
                    if o.t.bytes >= sdo.data as u64 {
                        o.t.transfer = "normal".into();
                        self.close(st, ctx, SdoOutcome::Ok, None);
                    } else {
                        o.t.transfer = "segmented".into();
                        o.expect = Expect::UploadSegRequest;
                        o.toggle = false;
                    }
                }
            }
            SdoCommand::UploadSegmentRequest { toggle } => {
                if expect != Some(Expect::UploadSegRequest) {
                    return protocol_error(self, "unexpected upload segment request".into());
                }
                let o = self.open.get_mut(&st).expect("checked");
                if toggle != o.toggle {
                    return protocol_error(self, "toggle bit not alternated (upload segment request)".into());
                }
                o.expect = Expect::UploadSegResponse;
            }
            SdoCommand::UploadSegmentResponse { toggle, unused, last } => {
                if expect != Some(Expect::UploadSegResponse) {
                    return protocol_error(self, "unexpected upload segment response".into());
                }
                let o = self.open.get_mut(&st).expect("checked");
                if toggle != o.toggle {
                    return protocol_error(self, "toggle bit differs from the request (upload segment)".into());
                }
                o.t.bytes += segment_data_len(h.length, unused) as u64;
                o.t.segments += 1;
                o.t.last_frame = ctx.frame;
                if last {
                    let size_ok = o.t.size.is_none_or(|s| s as u64 == o.t.bytes);
                    let note = (!size_ok).then(|| format!("received {} bytes, announced {:?}", o.t.bytes, o.t.size));
                    self.close(st, ctx, if size_ok { SdoOutcome::Ok } else { SdoOutcome::ProtocolError }, note);
                } else {
                    o.toggle = !o.toggle;
                    o.expect = Expect::UploadSegRequest;
                }
            }
            SdoCommand::InitiateDownloadResponse => {
                if expect != Some(Expect::InitResponse) || self.open[&st].t.kind != "download" {
                    return protocol_error(self, "unexpected initiate download response".into());
                }
                let o = self.open.get_mut(&st).expect("checked");
                if let Some(sdo) = sdo
                    && (sdo.index, sdo.subindex) != (o.t.index, o.t.subindex)
                {
                    return protocol_error(self, "response index/subindex differ from the request".into());
                }
                if o.t.transfer == "expedited" || o.t.size.is_some_and(|s| o.t.bytes >= s as u64) {
                    self.close(st, ctx, SdoOutcome::Ok, None);
                } else {
                    o.t.transfer = "segmented".into();
                    o.expect = Expect::DownloadSegRequest;
                    o.toggle = false;
                }
            }
            SdoCommand::DownloadSegmentRequest { toggle, unused, last } => {
                if expect != Some(Expect::DownloadSegRequest) {
                    return protocol_error(self, "unexpected download segment request".into());
                }
                let o = self.open.get_mut(&st).expect("checked");
                if toggle != o.toggle {
                    return protocol_error(self, "toggle bit not alternated (download segment request)".into());
                }
                o.t.bytes += segment_data_len(h.length, unused) as u64;
                o.t.segments += 1;
                o.last_pending = last;
                o.expect = Expect::DownloadSegResponse;
            }
            SdoCommand::DownloadSegmentResponse { toggle } => {
                if expect != Some(Expect::DownloadSegResponse) {
                    return protocol_error(self, "unexpected download segment response".into());
                }
                let o = self.open.get_mut(&st).expect("checked");
                if toggle != o.toggle {
                    return protocol_error(self, "toggle bit differs from the request (download segment)".into());
                }
                if o.last_pending {
                    let size_ok = o.t.size.is_none_or(|s| s as u64 == o.t.bytes);
                    let note = (!size_ok).then(|| format!("sent {} bytes, announced {:?}", o.t.bytes, o.t.size));
                    self.close(st, ctx, if size_ok { SdoOutcome::Ok } else { SdoOutcome::ProtocolError }, note);
                } else {
                    o.toggle = !o.toggle;
                    o.expect = Expect::DownloadSegRequest;
                }
            }
            SdoCommand::Unknown { .. } => {}
        }
    }

    fn finish(mut self, bins: usize) -> CoeReport {
        let mut rest: Vec<_> = self.open.drain().map(|(_, o)| o.t).collect();
        self.done.append(&mut rest);
        self.done.sort_by(|a, b| a.start_ns.total_cmp(&b.start_ns).then(a.station.cmp(&b.station)));
        let count = |o: SdoOutcome| self.done.iter().filter(|t| t.outcome == o).count() as u64;
        let durations: Vec<f64> = self
            .done
            .iter()
            .filter(|t| t.outcome == SdoOutcome::Ok)
            .filter_map(|t| t.end_ns.map(|e| e - t.start_ns))
            .collect();
        CoeReport {
            transactions_total: self.done.len() as u64,
            ok: count(SdoOutcome::Ok),
            aborted: count(SdoOutcome::Aborted),
            unfinished: count(SdoOutcome::Superseded) + count(SdoOutcome::Incomplete),
            protocol_errors: count(SdoOutcome::ProtocolError),
            duration: Distribution::of(&durations, bins),
            transactions: self.done.into_iter().take(MAX_LIST).collect(),
            emergencies: self.emergencies,
            emergencies_total: self.emergencies_total,
            sdo_info_messages: self.info,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// FoE
// ---------------------------------------------------------------------------------------------

struct OpenFoe {
    t: FoeTransfer,
    last_packet: u32,
    data_done: bool,
    sequence_error: bool,
}

#[derive(Default)]
struct FoeTracker {
    open: HashMap<u16, OpenFoe>,
    done: Vec<FoeTransfer>,
}

fn text_of(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

impl FoeTracker {
    fn close(&mut self, st: u16, ctx: Option<&Ctx>, outcome: &str) {
        if let Some(mut o) = self.open.remove(&st) {
            if let Some(ctx) = ctx {
                o.t.end_ns = Some(ctx.t_ns);
                o.t.last_frame = ctx.frame;
            }
            o.t.outcome = if o.sequence_error && outcome == "ok" { "protocol_error" } else { outcome }.into();
            if let Some(e) = o.t.end_ns
                && e > o.t.start_ns
                && o.t.outcome == "ok"
            {
                o.t.goodput_bps = Some(o.t.bytes as f64 * 8.0 / ((e - o.t.start_ns) / 1e9));
            }
            self.done.push(o.t);
        }
    }

    fn on(&mut self, ctx: &Ctx, h: &MailboxHeader, body: &[u8]) {
        let Ok(f) = FoeHeader::parse(body) else { return };
        let payload = &body[FoeHeader::LEN..];
        let st = ctx.station;
        match f.opcode {
            FoeOpcode::Read | FoeOpcode::Write => {
                self.close(st, None, "incomplete");
                self.open.insert(
                    st,
                    OpenFoe {
                        t: FoeTransfer {
                            station: st,
                            kind: if f.opcode == FoeOpcode::Write { "write" } else { "read" }.into(),
                            file_name: text_of(payload),
                            bytes: 0,
                            packets: 0,
                            busy: 0,
                            start_ns: ctx.t_ns,
                            end_ns: None,
                            goodput_bps: None,
                            first_frame: ctx.frame,
                            last_frame: ctx.frame,
                            outcome: "incomplete".into(),
                            error_code: None,
                            error_text: None,
                        },
                        last_packet: 0,
                        data_done: false,
                        sequence_error: false,
                    },
                );
            }
            FoeOpcode::Data => {
                if let Some(o) = self.open.get_mut(&st) {
                    if f.value != o.last_packet + 1 {
                        o.sequence_error = true;
                    }
                    o.last_packet = f.value;
                    o.t.bytes += payload.len() as u64;
                    o.t.packets += 1;
                    o.t.last_frame = ctx.frame;
                    // A packet that does not fill the mailbox is the last one.
                    if MAILBOX_HEADER_LEN + (h.length as usize) < ctx.buffer_len {
                        o.data_done = true;
                    }
                }
            }
            FoeOpcode::Ack => {
                if self.open.get(&st).is_some_and(|o| o.data_done && f.value == o.last_packet) {
                    self.close(st, Some(ctx), "ok");
                }
            }
            FoeOpcode::Error => {
                if let Some(o) = self.open.get_mut(&st) {
                    o.t.error_code = Some(f.value);
                    o.t.error_text =
                        Some(foe_error_text(f.value).map(str::to_owned).unwrap_or_else(|| text_of(payload)));
                }
                self.close(st, Some(ctx), "error");
            }
            FoeOpcode::Busy => {
                if let Some(o) = self.open.get_mut(&st) {
                    o.t.busy += 1;
                }
            }
            FoeOpcode::Reserved(_) => {}
        }
    }

    fn finish(mut self) -> Vec<FoeTransfer> {
        let stations: Vec<u16> = self.open.keys().copied().collect();
        for s in stations {
            self.close(s, None, "incomplete");
        }
        self.done.sort_by(|a, b| a.start_ns.total_cmp(&b.start_ns));
        self.done.truncate(MAX_LIST);
        self.done
    }
}

// ---------------------------------------------------------------------------------------------
// EoE
// ---------------------------------------------------------------------------------------------

struct Assembly {
    frame_no: u8,
    next_fragment: u8,
    data: Vec<u8>,
    start_ns: f64,
    first_frame: u32,
}

#[derive(Default)]
struct EoeTracker {
    open: HashMap<(u16, MailboxDirection), Assembly>,
    frames: Vec<EoeFrame>,
    total: u64,
    to_slaves: u64,
    from_slaves: u64,
    errors: u64,
    services: u64,
}

impl EoeTracker {
    fn on(&mut self, ctx: &Ctx, body: &[u8], packets: &mut Vec<(u64, Vec<u8>)>) {
        let Ok(h) = EoeHeader::parse(body) else { return };
        if h.frame_type != EoeType::Fragment {
            self.services += 1;
            return;
        }
        let mut payload = &body[EoeHeader::LEN..];
        if h.last_fragment && h.time_appended && payload.len() >= 4 {
            payload = &payload[..payload.len() - 4];
        }
        let key = (ctx.station, ctx.direction);
        if h.fragment == 0 {
            if self.open.remove(&key).is_some() {
                self.errors += 1;
            }
            self.open.insert(
                key,
                Assembly {
                    frame_no: h.frame_no,
                    next_fragment: 0,
                    data: Vec::with_capacity(h.offset_blocks as usize * EoeHeader::BLOCK),
                    start_ns: ctx.t_ns,
                    first_frame: ctx.frame,
                },
            );
        }
        let Some(a) = self.open.get_mut(&key) else {
            self.errors += 1;
            return;
        };
        let offset_ok = h.fragment == 0 || h.offset_blocks as usize * EoeHeader::BLOCK == a.data.len();
        if a.frame_no != h.frame_no || a.next_fragment != h.fragment || !offset_ok {
            self.errors += 1;
            self.open.remove(&key);
            return;
        }
        a.data.extend_from_slice(payload);
        a.next_fragment = a.next_fragment.wrapping_add(1);
        if !h.last_fragment {
            return;
        }
        let a = self.open.remove(&key).expect("present");
        let eth = |r: std::ops::Range<usize>| {
            a.data.get(r).map(|b| b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(":"))
        };
        let frame = EoeFrame {
            station: ctx.station,
            direction: ctx.direction,
            start_ns: a.start_ns,
            end_ns: ctx.t_ns,
            first_frame: a.first_frame,
            last_frame: ctx.frame,
            fragments: a.next_fragment as u32,
            bytes: a.data.len() as u32,
            dst: eth(0..6),
            src: eth(6..12),
            ethertype: a.data.get(12..14).map(|b| u16::from_be_bytes([b[0], b[1]])),
        };
        self.total += 1;
        match ctx.direction {
            MailboxDirection::ToSlave => self.to_slaves += a.data.len() as u64,
            MailboxDirection::FromSlave => self.from_slaves += a.data.len() as u64,
        }
        if self.frames.len() < MAX_LIST {
            self.frames.push(frame);
        }
        packets.push((ctx.start_ts_ns + ctx.t_ns as u64, a.data));
    }

    fn finish(self) -> EoeReport {
        EoeReport {
            frames: self.frames,
            frames_total: self.total,
            bytes_to_slaves: self.to_slaves,
            bytes_from_slaves: self.from_slaves,
            reassembly_errors: self.errors + self.open.len() as u64,
            service_messages: self.services,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Describing one message
// ---------------------------------------------------------------------------------------------

/// One line and labelled fields for a mailbox message (`body` = the bytes after the
/// mailbox header, `header.length` long).
pub fn describe(h: &MailboxHeader, body: &[u8]) -> (String, Vec<(String, String)>) {
    let mut f: Vec<(String, String)> = Vec::new();
    let mut kv = |k: &str, v: String| f.push((k.to_owned(), v));
    let summary = match h.mailbox_type {
        MailboxType::Coe => match CoeHeader::parse(body) {
            Err(_) => "CoE (truncated)".to_owned(),
            Ok(c) => {
                let rest = &body[CoeHeader::LEN..];
                kv("CoE service", format!("{:?}", c.service));
                match c.service {
                    CoeService::SdoRequest | CoeService::SdoResponse if !rest.is_empty() => {
                        let cmd = SdoCommand::decode(rest[0], c.service == CoeService::SdoRequest);
                        kv("SDO command", format!("0x{:02X} — {}", rest[0], cmd.name()));
                        let segment = matches!(
                            cmd,
                            SdoCommand::UploadSegmentRequest { .. }
                                | SdoCommand::UploadSegmentResponse { .. }
                                | SdoCommand::DownloadSegmentRequest { .. }
                                | SdoCommand::DownloadSegmentResponse { .. }
                        );
                        if segment {
                            let (t, data) = match cmd {
                                SdoCommand::UploadSegmentResponse { toggle, unused, last }
                                | SdoCommand::DownloadSegmentRequest { toggle, unused, last } => {
                                    let n = segment_data_len(h.length, unused);
                                    kv("Segment data", format!("{n} B{}", if last { ", last" } else { "" }));
                                    (toggle, format!(" {n} B{}", if last { " last" } else { "" }))
                                }
                                SdoCommand::UploadSegmentRequest { toggle }
                                | SdoCommand::DownloadSegmentResponse { toggle } => (toggle, String::new()),
                                _ => (false, String::new()),
                            };
                            kv("Toggle", (t as u8).to_string());
                            format!("CoE SDO {} t={}{data}", cmd.short_name(), t as u8)
                        } else if let Ok(s) = SdoHeader::parse(rest) {
                            kv("Index", format!("0x{:04X}:{:02X}", s.index, s.subindex));
                            let obj = format!("0x{:04X}:{:02X}", s.index, s.subindex);
                            match cmd {
                                SdoCommand::Abort => {
                                    let text = sdo_abort_text(s.data).unwrap_or("unknown abort code");
                                    kv("Abort code", format!("0x{:08X} — {text}", s.data));
                                    format!("CoE SDO abort {obj}: {text}")
                                }
                                SdoCommand::InitiateUploadResponse {
                                    expedited: true, size_indicated, unused, ..
                                }
                                | SdoCommand::InitiateDownloadRequest {
                                    expedited: true, size_indicated, unused, ..
                                } => {
                                    let n = if size_indicated { 4 - unused as usize } else { 4 };
                                    let v = &s.data.to_le_bytes()[..n];
                                    let hex: String = v.iter().rev().map(|b| format!("{b:02X}")).collect();
                                    kv("Expedited data", format!("{n} B = 0x{hex}"));
                                    format!("CoE SDO {} {obj} = 0x{hex}", cmd.short_name())
                                }
                                SdoCommand::InitiateUploadResponse { .. }
                                | SdoCommand::InitiateDownloadRequest { .. } => {
                                    kv("Complete size", format!("{} B", s.data));
                                    let here = body.len().saturating_sub(CoeHeader::LEN + SdoHeader::LEN);
                                    kv("Data in this message", format!("{here} B"));
                                    format!("CoE SDO {} {obj} size {} B", cmd.short_name(), s.data)
                                }
                                _ => format!("CoE SDO {} {obj}", cmd.short_name()),
                            }
                        } else {
                            format!("CoE SDO {} (truncated)", cmd.short_name())
                        }
                    }
                    CoeService::Emergency => match CoeEmergency::parse(rest) {
                        Ok(e) => {
                            kv("Error code", format!("0x{:04X}", e.error_code));
                            kv("Error register", format!("0x{:02X}", e.error_register));
                            format!("CoE emergency 0x{:04X} reg 0x{:02X}", e.error_code, e.error_register)
                        }
                        Err(_) => "CoE emergency (truncated)".into(),
                    },
                    CoeService::SdoInformation => match SdoInfoHeader::parse(rest) {
                        Ok(i) => {
                            kv("Op code", format!("{} — {}", i.opcode, i.opcode_name()));
                            kv("Fragments left", i.fragments_left.to_string());
                            format!("CoE SDO info {}", i.opcode_name())
                        }
                        Err(_) => "CoE SDO info (truncated)".into(),
                    },
                    other => format!("CoE {other:?}"),
                }
            }
        },
        MailboxType::Foe => match FoeHeader::parse(body) {
            Err(_) => "FoE (truncated)".into(),
            Ok(x) => {
                let payload = &body[FoeHeader::LEN..];
                kv("Op code", x.opcode.name().into());
                match x.opcode {
                    FoeOpcode::Read | FoeOpcode::Write => {
                        kv("Password", format!("0x{:08X}", x.value));
                        kv("File name", text_of(payload));
                        format!("FoE {} '{}'", x.opcode.name(), text_of(payload))
                    }
                    FoeOpcode::Data => {
                        kv("Packet", x.value.to_string());
                        kv("Data", format!("{} B", payload.len()));
                        format!("FoE DATA #{} {} B", x.value, payload.len())
                    }
                    FoeOpcode::Ack => {
                        kv("Packet", x.value.to_string());
                        format!("FoE ACK #{}", x.value)
                    }
                    FoeOpcode::Error => {
                        let t = foe_error_text(x.value).map(str::to_owned).unwrap_or_else(|| text_of(payload));
                        kv("Error", format!("0x{:08X} — {t}", x.value));
                        format!("FoE ERR 0x{:X} {t}", x.value)
                    }
                    FoeOpcode::Busy => {
                        let (d, e) = x.busy_progress();
                        kv("Progress", format!("{d}/{e}"));
                        format!("FoE BUSY {d}/{e}")
                    }
                    FoeOpcode::Reserved(c) => format!("FoE op {c}"),
                }
            }
        },
        MailboxType::Eoe => match EoeHeader::parse(body) {
            Err(_) => "EoE (truncated)".into(),
            Ok(e) => {
                kv("Type", e.frame_type.name().into());
                kv("Port", e.port.to_string());
                if e.frame_type == EoeType::Fragment {
                    let n = body.len() - EoeHeader::LEN;
                    kv("Frame number", e.frame_no.to_string());
                    kv("Fragment", e.fragment.to_string());
                    if e.fragment == 0 {
                        kv("Frame size", format!("≤ {} B", e.offset_blocks as usize * EoeHeader::BLOCK));
                    } else {
                        kv("Offset", format!("{} B", e.offset_blocks as usize * EoeHeader::BLOCK));
                    }
                    kv("Fragment data", format!("{n} B"));
                    kv("Last fragment", e.last_fragment.to_string());
                    format!(
                        "EoE frame {} fragment {} {n} B{}",
                        e.frame_no,
                        e.fragment,
                        if e.last_fragment { " last" } else { "" }
                    )
                } else {
                    format!("EoE {}", e.frame_type.name())
                }
            }
        },
        MailboxType::Soe => match SoeHeader::parse(body) {
            Err(_) => "SoE (truncated)".into(),
            Ok(s) => {
                kv("Op code", format!("{} — {}", s.opcode, s.opcode_name()));
                kv("Drive", s.drive.to_string());
                kv("IDN", format!("0x{:04X}", s.idn));
                kv("Error", s.error.to_string());
                format!("SoE {} IDN 0x{:04X}{}", s.opcode_name(), s.idn, if s.error { " error" } else { "" })
            }
        },
        MailboxType::Voe => match VoeHeader::parse(body) {
            Err(_) => "VoE (truncated)".into(),
            Ok(v) => {
                kv("Vendor id", format!("0x{:08X}", v.vendor_id));
                kv("Vendor type", format!("0x{:04X}", v.vendor_type));
                format!("VoE vendor 0x{:08X} type 0x{:04X}", v.vendor_id, v.vendor_type)
            }
        },
        MailboxType::Error => match MailboxErrorReply::parse(body) {
            Ok(e) => {
                kv("Detail", format!("0x{:04X} — {}", e.detail, e.detail_text()));
                format!("Mailbox error: {}", e.detail_text())
            }
            Err(_) => "Mailbox error (truncated)".into(),
        },
        MailboxType::Aoe => "AoE".into(),
        MailboxType::Reserved(c) => format!("mailbox type {c}"),
    };
    (summary, f)
}

// ---------------------------------------------------------------------------------------------
// Findings and assumptions
// ---------------------------------------------------------------------------------------------

fn plural(n: u64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

pub(crate) fn findings(m: &MailboxReport, out: &mut Vec<Finding>) {
    let mut add = |severity, title: String, detail: String| out.push(Finding { severity, title, detail });
    let coe = &m.coe;
    if coe.protocol_errors > 0 {
        add(
            Severity::Defect,
            format!("{} violated the protocol", plural(coe.protocol_errors, "SDO transfer", "SDO transfers")),
            "Toggle bit, size or response sequence did not match the request. Threshold: any.".into(),
        );
    }
    if coe.aborted > 0 {
        let mut codes: BTreeMap<u32, u64> = BTreeMap::new();
        for t in coe.transactions.iter().filter(|t| t.outcome == SdoOutcome::Aborted) {
            *codes.entry(t.abort_code.unwrap_or(0)).or_default() += 1;
        }
        let (code, n) = codes.iter().max_by_key(|(_, n)| **n).map(|(c, n)| (*c, *n)).unwrap_or((0, 0));
        add(
            Severity::Warn,
            format!("{} aborted", plural(coe.aborted, "SDO transfer", "SDO transfers")),
            format!(
                "Most frequent: 0x{code:08X} {} ({n}×). Masters probing optional objects cause aborts too. Threshold: any.",
                sdo_abort_text(code).unwrap_or("")
            ),
        );
    }
    if coe.emergencies_total > 0 {
        add(
            Severity::Warn,
            plural(coe.emergencies_total, "CoE emergency", "CoE emergencies"),
            "Slaves reported emergency messages; see the Mailbox view for codes. Threshold: any.".into(),
        );
    }
    if coe.unfinished > 0 {
        add(
            Severity::Info,
            format!("{} unfinished", plural(coe.unfinished, "SDO transfer", "SDO transfers")),
            "Superseded by a new request or still open at the end of the capture.".into(),
        );
    }
    let foe_bad = m.foe.iter().filter(|t| t.outcome == "error" || t.outcome == "protocol_error").count();
    if foe_bad > 0 {
        add(
            Severity::Warn,
            format!("{} failed", plural(foe_bad as u64, "FoE transfer", "FoE transfers")),
            "An FoE error reply or a packet-number gap ended the transfer. Threshold: any.".into(),
        );
    }
    if m.eoe.reassembly_errors > 0 {
        add(
            Severity::Warn,
            format!("{} could not be reassembled", plural(m.eoe.reassembly_errors, "EoE frame", "EoE frames")),
            "Fragments were missing, out of order, or the capture ended mid-frame. Threshold: any.".into(),
        );
    }
    if !m.errors.is_empty() {
        add(
            Severity::Warn,
            plural(m.errors.len() as u64, "mailbox error reply", "mailbox error replies"),
            format!("First: station 0x{:04X}, {}. Threshold: any.", m.errors[0].station, m.errors[0].text),
        );
    }
    if m.malformed > 0 {
        add(
            Severity::Warn,
            plural(m.malformed, "malformed mailbox message", "malformed mailbox messages"),
            "Data read from or written to a mailbox address is not a valid mailbox header.".into(),
        );
    }
    let rejected: u64 = m.channels.iter().map(|c| c.writes_rejected).sum();
    if rejected > 0 {
        add(
            Severity::Info,
            format!("{} rejected", plural(rejected, "mailbox write", "mailbox writes")),
            "The slave's mailbox was still full (WKC 0); masters retry. Many rejections mean the slave is slow to read its mailbox.".into(),
        );
    }
}

pub(crate) fn assumptions(m: &MailboxReport) -> Vec<String> {
    let mut a = vec![
        "Each station's mailbox SyncManagers come, in this order of preference, from SyncManager configuration writes in the capture (mailbox mode), the ENI, a user hint or a heuristic; the source is reported per channel. Messages are taken from returning datagrams at the mailbox start address with WKC > 0.".to_owned(),
        "Logically mapped mailbox access and the SyncManager repeat mechanism are not interpreted; a message whose counter equals the previous one is treated as a repeat.".into(),
    ];
    if m.channels.iter().any(|c| c.source == ChannelSource::Eni) {
        a.push("Mailboxes of stations without SyncManager configuration in the capture come from the ENI; its \"Recv\" mailbox is taken to be SM1 for status polls.".into());
    }
    if m.channels.iter().any(|c| c.source == ChannelSource::Heuristic) {
        a.push("Some stations had no SyncManager configuration in the capture; their mailboxes were inferred from FPWR/FPRD at ≥ 0x1000 whose data parses as a mailbox header.".into());
    }
    a
}

/// Write the Ethernet frames reassembled from EoE as a pcapng, carrying the source
/// capture's provenance. Returns the number of frames written.
pub fn write_eoe_pcapng<W: std::io::Write>(
    a: &MailboxAnalysis,
    provenance: flowlab_core::Provenance,
    source_name: &str,
    out: W,
) -> std::io::Result<usize> {
    let comment = format!(
        "{}{}; derived=EoE reassembly of {source_name}; generator=flowlab {}",
        flowlab_core::Provenance::COMMENT_PREFIX,
        provenance.as_str(),
        env!("CARGO_PKG_VERSION")
    );
    let mut w =
        flowlab_capture::PcapNgWriter::new(out, concat!("flowlab ", env!("CARGO_PKG_VERSION")), &comment, "eoe0")?;
    for (t, bytes) in &a.eoe_packets {
        w.packet(*t, bytes)?;
    }
    w.finish()?;
    Ok(a.eoe_packets.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(t: u64, direction: MailboxDirection) -> Ctx {
        Ctx { t_ns: t as f64, frame: t as u32, station: 0x1001, direction, buffer_len: 128, start_ts_ns: 0 }
    }

    fn msg(t: MailboxType, body: Vec<u8>) -> (MailboxHeader, Vec<u8>) {
        let h = MailboxHeader {
            length: body.len() as u16,
            address: 0,
            channel: 0,
            priority: 0,
            mailbox_type: t,
            counter: 1,
        };
        (h, body)
    }

    fn sdo(service: CoeService, cmd: SdoCommand, index: u16, data: u32) -> (MailboxHeader, Vec<u8>) {
        let mut b = CoeHeader { number: 0, service }.encode().to_vec();
        b.extend_from_slice(&SdoHeader { command: cmd.encode(), index, subindex: 0, data }.encode());
        msg(MailboxType::Coe, b)
    }

    fn seg(service: CoeService, cmd: SdoCommand, data: usize) -> (MailboxHeader, Vec<u8>) {
        let mut b = CoeHeader { number: 0, service }.encode().to_vec();
        b.push(cmd.encode());
        b.resize(3 + data.max(7), 0);
        msg(MailboxType::Coe, b)
    }

    const UP: SdoCommand = SdoCommand::InitiateUploadRequest { complete_access: false };
    const TO: MailboxDirection = MailboxDirection::ToSlave;
    const FROM: MailboxDirection = MailboxDirection::FromSlave;

    fn feed(c: &mut CoeTracker, t: u64, dir: MailboxDirection, m: (MailboxHeader, Vec<u8>)) {
        c.on(&ctx(t, dir), &m.0, &m.1);
    }

    #[test]
    fn sdo_toggle_violation_is_a_protocol_error() {
        let mut c = CoeTracker::default();
        feed(&mut c, 1, TO, sdo(CoeService::SdoRequest, UP, 0x2000, 0));
        let normal = SdoCommand::InitiateUploadResponse {
            complete_access: false,
            expedited: false,
            size_indicated: true,
            unused: 0,
        };
        feed(&mut c, 2, FROM, sdo(CoeService::SdoResponse, normal, 0x2000, 500));
        feed(&mut c, 3, TO, seg(CoeService::SdoRequest, SdoCommand::UploadSegmentRequest { toggle: false }, 0));
        let resp = SdoCommand::UploadSegmentResponse { toggle: false, unused: 0, last: false };
        feed(&mut c, 4, FROM, seg(CoeService::SdoResponse, resp, 119));
        // Toggle must now be 1.
        feed(&mut c, 5, TO, seg(CoeService::SdoRequest, SdoCommand::UploadSegmentRequest { toggle: false }, 0));
        let r = c.finish(10);
        assert_eq!(r.protocol_errors, 1);
        let t = &r.transactions[0];
        assert_eq!((t.segments, t.bytes, t.end_ns), (1, 119, Some(5.0)));
        assert!(t.note.as_deref().unwrap().contains("toggle"));
    }

    #[test]
    fn superseded_incomplete_and_orphan_abort() {
        let mut c = CoeTracker::default();
        feed(&mut c, 1, TO, sdo(CoeService::SdoRequest, UP, 0x1000, 0));
        feed(&mut c, 2, TO, sdo(CoeService::SdoRequest, UP, 0x1001, 0)); // supersedes 0x1000
        feed(&mut c, 3, FROM, sdo(CoeService::SdoRequest, SdoCommand::Abort, 0x1001, 0x0609_0011));
        feed(&mut c, 4, TO, sdo(CoeService::SdoRequest, SdoCommand::Abort, 0x1234, 0x0800_0000)); // no request
        feed(&mut c, 5, TO, sdo(CoeService::SdoRequest, UP, 0x1002, 0)); // never answered
        let r = c.finish(10);
        let by_index = |i: u16| r.transactions.iter().find(|t| t.index == i).unwrap();
        assert_eq!(by_index(0x1000).outcome, SdoOutcome::Superseded);
        let a = by_index(0x1001);
        assert_eq!(
            (a.outcome, a.aborted_by.as_deref(), a.abort_code),
            (SdoOutcome::Aborted, Some("slave"), Some(0x0609_0011))
        );
        assert_eq!(a.abort_text.as_deref(), Some("Subindex does not exist"));
        let o = by_index(0x1234);
        assert_eq!(
            (o.outcome, o.kind.as_str(), o.aborted_by.as_deref()),
            (SdoOutcome::Aborted, "unknown", Some("master"))
        );
        assert_eq!(by_index(0x1002).outcome, SdoOutcome::Incomplete);
        assert_eq!((r.ok, r.aborted, r.unfinished), (0, 2, 2));
    }

    #[test]
    fn foe_error_and_eoe_gap() {
        let foe = |op: FoeOpcode, v: u32, tail: &[u8]| {
            let mut b = FoeHeader { opcode: op, value: v }.encode().to_vec();
            b.extend_from_slice(tail);
            msg(MailboxType::Foe, b)
        };
        let mut f = FoeTracker::default();
        let m = foe(FoeOpcode::Read, 0, b"boot.bin");
        f.on(&ctx(1, TO), &m.0, &m.1);
        let m = foe(FoeOpcode::Error, 0x8001, b"");
        f.on(&ctx(2, FROM), &m.0, &m.1);
        let t = &f.finish()[0];
        assert_eq!((t.kind.as_str(), t.file_name.as_str(), t.outcome.as_str()), ("read", "boot.bin", "error"));
        assert_eq!(t.error_text.as_deref(), Some("Not found"));

        let frag = |fragment: u8, offset_blocks: u8, last: bool| {
            let h = EoeHeader {
                frame_type: EoeType::Fragment,
                port: 0,
                last_fragment: last,
                time_appended: false,
                time_request: false,
                fragment,
                offset_blocks,
                frame_no: 1,
            };
            let mut b = h.encode().to_vec();
            b.extend_from_slice(&[0u8; 32]);
            b
        };
        let mut e = EoeTracker::default();
        let mut packets = Vec::new();
        e.on(&ctx(1, TO), &frag(0, 3, false), &mut packets);
        e.on(&ctx(2, TO), &frag(2, 2, true), &mut packets); // fragment 1 missing
        e.on(&ctx(3, TO), &frag(0, 2, false), &mut packets);
        e.on(&ctx(4, TO), &frag(1, 1, true), &mut packets);
        let r = e.finish();
        assert_eq!((r.frames_total, r.reassembly_errors), (1, 1));
        assert_eq!((r.frames[0].fragments, r.frames[0].bytes), (2, 64));
        assert_eq!(packets.len(), 1);
    }
}
