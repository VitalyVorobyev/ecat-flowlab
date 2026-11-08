//! FoE: file transfers, DATA and ACK, errors and busy replies.

use std::collections::HashMap;

use ecat_wire::{FoeHeader, FoeOpcode, MAILBOX_HEADER_LEN, MailboxHeader, foe_error_text};

use super::*;

pub(super) struct OpenFoe {
    pub(super) t: FoeTransfer,
    pub(super) last_packet: u32,
    pub(super) data_done: bool,
    pub(super) sequence_error: bool,
}

#[derive(Default)]
pub(super) struct FoeTracker {
    pub(super) open: HashMap<u16, OpenFoe>,
    pub(super) done: Vec<FoeTransfer>,
}

pub(super) fn text_of(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

impl FoeTracker {
    pub(super) fn close(&mut self, st: u16, ctx: Option<&Ctx>, outcome: &str) {
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

    pub(super) fn on(&mut self, ctx: &Ctx, h: &MailboxHeader, body: &[u8]) {
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

    pub(super) fn finish(mut self) -> Vec<FoeTransfer> {
        let stations: Vec<u16> = self.open.keys().copied().collect();
        for s in stations {
            self.close(s, None, "incomplete");
        }
        self.done.sort_by(|a, b| a.start_ns.total_cmp(&b.start_ns));
        self.done.truncate(MAX_LIST);
        self.done
    }
}
