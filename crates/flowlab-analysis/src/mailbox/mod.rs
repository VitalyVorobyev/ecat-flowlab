//! Mailbox analysis: find each slave's mailbox SyncManagers, extract the messages that went
//! through them, and follow CoE SDO transfers, FoE file transfers and EoE fragment streams.
//!
//! A mailbox write is accepted when it returns with WKC > 0; WKC 0 means the slave's mailbox
//! was still full. A mailbox read returns a message when WKC > 0; WKC 0 means it was empty.
//! Messages are therefore taken from *returning* datagrams.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use ecat_wire::{DatagramAddress, MAILBOX_HEADER_LEN, MailboxErrorReply, MailboxType, SoeHeader};
use flowlab_capture::{Direction, FrameKind, Trace};

use crate::report::*;
use crate::stats::Distribution;

mod channels;
mod coe;
mod describe;
mod eoe;
mod export;
mod findings;
mod foe;
mod voe;

use channels::*;
pub use channels::{Access, Channels, MailboxHint};
use coe::*;
pub use describe::describe;
use eoe::*;
pub use export::write_eoe_pcapng;
pub(crate) use findings::{assumptions, findings};
use foe::*;
use voe::VoeTracker;

const MAX_LIST: usize = 2000;

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
    let mut voe = VoeTracker::default();
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
                    let name = h.mailbox_type.name();
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
                        mean_rate_bps: None,
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
                        MailboxType::Voe => voe.on(&ctx, body),
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
        protocols: protocols
            .into_values()
            .map(|p| {
                let span_s = (p.last_ns - p.first_ns) / 1e9;
                let mean_rate_bps = (span_s > 0.0).then(|| (p.bytes_out + p.bytes_in) as f64 * 8.0 / span_s);
                ProtocolStats { mean_rate_bps, ..p }
            })
            .collect(),
        coe: coe.finish(bins),
        foe: foe.finish(),
        eoe: eoe.finish(),
        voe: voe.finish(),
        soe_errors,
        errors,
        malformed,
    };
    out.channels = channels;
    (out, report)
}

#[cfg(test)]
mod tests;
