//! Where each station's mailbox is: SyncManager configuration, the ENI, hints, a heuristic.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use ecat_wire::{
    DatagramAddress, MAILBOX_HEADER_LEN, MailboxHeader, MailboxType, SM_COUNT, SM_REGISTER_BASE, SM_REGISTER_LEN,
    SmDirection, SmMode, sm_status_register, sync_managers_in,
};
use flowlab_capture::{DatagramRecord, Direction, FrameKind, Trace};

use super::*;

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
pub(super) struct ChannelInfo {
    pub(super) source: ChannelSource,
    pub(super) out: BTreeMap<u16, u16>,
    pub(super) inn: BTreeMap<u16, u16>,
    /// Physical addresses of the "mailbox in" SyncManagers' status registers.
    pub(super) status: BTreeSet<u16>,
}

impl ChannelInfo {
    pub(super) fn new(source: ChannelSource) -> Self {
        Self { source, out: BTreeMap::new(), inn: BTreeMap::new(), status: BTreeSet::new() }
    }
}

/// Mailbox SyncManager addresses per station.
#[derive(Debug, Clone, Default)]
pub struct Channels {
    pub(super) map: BTreeMap<u16, ChannelInfo>,
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

    pub(super) fn detect(
        trace: &Trace,
        order: &[u32],
        hints: &[MailboxHint],
        network: Option<&flowlab_devices::NetworkConfig>,
        directions_known: bool,
    ) -> Self {
        let mut map: BTreeMap<u16, ChannelInfo> = BTreeMap::new();
        let mut positions: HashMap<u16, u16> = HashMap::new();
        let mut by_position: Vec<(u16, u8, ecat_wire::SyncManagerConfig)> = Vec::new();
        let sm_end = SM_REGISTER_BASE as u32 + SM_COUNT as u32 * SM_REGISTER_LEN as u32;
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
                c.status.insert(sm_status_register(1));
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

pub(super) fn add_sm(map: &mut BTreeMap<u16, ChannelInfo>, station: u16, n: u8, sm: ecat_wire::SyncManagerConfig) {
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
            c.status.insert(sm_status_register(n));
        }
        SmDirection::Reserved => {}
    }
}

/// Plausible mailbox header filling at most `data`.
pub(super) fn plausible(data: &[u8]) -> Option<MailboxHeader> {
    let h = MailboxHeader::parse(data).ok()?;
    let ok = h.length > 0
        && h.length as usize + MAILBOX_HEADER_LEN <= data.len()
        && !matches!(h.mailbox_type, MailboxType::Reserved(_));
    ok.then_some(h)
}

/// Stations without configuration or hint: FPWR at ≥ 0x1000 carrying a plausible mailbox
/// header, accepted by the slave, defines "mailbox out"; then FPRD at ≥ 0x1000 returning a
/// plausible header defines "mailbox in".
pub(super) fn heuristic(trace: &Trace, order: &[u32], map: &mut BTreeMap<u16, ChannelInfo>, directions_known: bool) {
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
