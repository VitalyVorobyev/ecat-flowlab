//! FlowLab VoE: transfers rebuilt from OPEN, READY, DATA, ACK, DONE and ABORT, with the payload
//! hashed and compared with the slave's DONE.
//!
//! The message format is read here on its own (not through `flowlab_mailbox::voe`), so that a
//! misreading there cannot hide behind the same misreading here.

use std::collections::HashMap;

use ecat_wire::VoeHeader;

use super::*;

const VENDOR_ID: u32 = 0;
const VENDOR_TYPE: u16 = 0x464C;
const SEQ_MOD: u64 = 1 << 13;

/// A FlowLab VoE message: operation code, 13-bit sequence number, the rest.
pub(super) fn parse(body: &[u8]) -> Option<(u16, u16, &[u8])> {
    let h = VoeHeader::parse(body).ok()?;
    if h.vendor_id != VENDOR_ID || h.vendor_type != VENDOR_TYPE || body.len() < VoeHeader::LEN + 2 {
        return None;
    }
    let word = u16::from_le_bytes([body[VoeHeader::LEN], body[VoeHeader::LEN + 1]]);
    Some((word >> 13, word & (SEQ_MOD as u16 - 1), &body[VoeHeader::LEN + 2..]))
}

/// Name of an operation code.
pub(super) fn op_name(op: u16) -> &'static str {
    match op {
        0 => "OPEN",
        1 => "READY",
        2 => "DATA",
        3 => "ACK",
        4 => "DONE",
        5 => "ABORT",
        _ => "reserved",
    }
}

fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

struct OpenVoe {
    t: VoeTransfer,
    to_slave: bool,
    count: u64,
    data: Vec<u8>,
}

#[derive(Default)]
pub(super) struct VoeTracker {
    open: HashMap<u16, OpenVoe>,
    done: Vec<VoeTransfer>,
}

impl VoeTracker {
    fn close(&mut self, st: u16, ctx: &Ctx, outcome: &str) {
        let Some(mut o) = self.open.remove(&st) else { return };
        o.t.end_ns = Some(ctx.t_ns);
        o.t.last_frame = ctx.frame;
        o.t.outcome = outcome.into();
        if outcome == "ok" && ctx.t_ns > o.t.start_ns {
            o.t.goodput_bps = Some(o.t.bytes as f64 * 8.0 / ((ctx.t_ns - o.t.start_ns) / 1e9));
        }
        self.done.push(o.t);
    }

    pub(super) fn on(&mut self, ctx: &Ctx, body: &[u8]) {
        let Some((op, seq, tail)) = parse(body) else { return };
        let st = ctx.station;
        let from_master = ctx.direction == MailboxDirection::ToSlave;
        match op {
            0 if from_master => {
                if self.open.contains_key(&st) {
                    self.close(st, ctx, "incomplete");
                }
                let to_slave = tail.first().is_none_or(|d| *d == 0);
                let t = VoeTransfer {
                    station: st,
                    direction: if to_slave { MailboxDirection::ToSlave } else { MailboxDirection::FromSlave },
                    window: u16_at(tail, 2).unwrap_or(0),
                    length: u64_at(tail, 4).filter(|_| to_slave),
                    bytes: 0,
                    data_messages: 0,
                    acks: 0,
                    repeats: 0,
                    gaps: 0,
                    start_ns: ctx.t_ns,
                    end_ns: None,
                    goodput_bps: None,
                    first_frame: ctx.frame,
                    last_frame: ctx.frame,
                    outcome: "incomplete".into(),
                    digest_matches: None,
                    abort_text: None,
                };
                self.open.insert(st, OpenVoe { t, to_slave, count: 0, data: Vec::new() });
            }
            1 => {
                if let Some(o) = self.open.get_mut(&st) {
                    o.t.window = u16_at(tail, 0).unwrap_or(o.t.window);
                    o.t.length = u64_at(tail, 2).or(o.t.length);
                    o.t.last_frame = ctx.frame;
                }
            }
            2 => {
                let Some(o) = self.open.get_mut(&st) else { return };
                if o.to_slave != from_master {
                    return;
                }
                o.t.last_frame = ctx.frame;
                let ahead = (u64::from(seq) + SEQ_MOD - o.count % SEQ_MOD) % SEQ_MOD;
                if ahead == 0 {
                    let room = o.t.length.map_or(tail.len() as u64, |n| n.saturating_sub(o.t.bytes)) as usize;
                    let take = &tail[..tail.len().min(room)];
                    o.data.extend_from_slice(take);
                    o.t.bytes += take.len() as u64;
                    o.t.data_messages += 1;
                    o.count += 1;
                } else if ahead < SEQ_MOD / 2 {
                    o.t.gaps += 1;
                } else {
                    o.t.repeats += 1;
                }
            }
            3 => {
                let Some(o) = self.open.get_mut(&st) else { return };
                o.t.acks += 1;
                o.t.last_frame = ctx.frame;
                // From the slave: the master's ACK after the last DATA ends the transfer.
                let complete = o.t.length.is_some_and(|n| o.t.bytes >= n);
                if !o.to_slave && from_master && complete && u64::from(seq) == o.count % SEQ_MOD {
                    self.close(st, ctx, "ok");
                }
            }
            4 if !from_master => {
                let Some(o) = self.open.get_mut(&st) else { return };
                if let (Some(len), Some(digest)) = (u64_at(tail, 0), tail.get(8..40)) {
                    let ours = flowlab_core::sha256_hex(&o.data);
                    let theirs: String = digest.iter().map(|b| format!("{b:02x}")).collect();
                    o.t.digest_matches = Some(len == o.t.bytes && ours == theirs);
                }
                let complete = o.t.length.is_some_and(|n| o.t.bytes >= n);
                self.close(st, ctx, if complete { "ok" } else { "protocol_error" });
            }
            5 => {
                if let Some(o) = self.open.get_mut(&st) {
                    let text = String::from_utf8_lossy(tail.get(2..).unwrap_or_default());
                    o.t.abort_text = Some(text.trim_end_matches('\0').to_owned());
                }
                self.close(st, ctx, "aborted");
            }
            _ => {}
        }
    }

    pub(super) fn finish(mut self) -> Vec<VoeTransfer> {
        self.done.extend(self.open.into_values().map(|o| o.t));
        self.done.sort_by(|a, b| a.start_ns.total_cmp(&b.start_ns));
        self.done.truncate(MAX_LIST);
        self.done
    }
}
