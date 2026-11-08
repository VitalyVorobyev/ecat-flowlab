//! EoE: fragments reassembled into Ethernet frames per station and direction.

use std::collections::HashMap;

use ecat_wire::{EoeHeader, EoeType};

use super::*;

pub(super) struct Assembly {
    pub(super) frame_no: u8,
    pub(super) next_fragment: u8,
    pub(super) data: Vec<u8>,
    pub(super) start_ns: f64,
    pub(super) first_frame: u32,
}

#[derive(Default)]
pub(super) struct EoeTracker {
    pub(super) open: HashMap<(u16, MailboxDirection), Assembly>,
    pub(super) frames: Vec<EoeFrame>,
    pub(super) total: u64,
    pub(super) to_slaves: u64,
    pub(super) from_slaves: u64,
    pub(super) errors: u64,
    pub(super) services: u64,
}

impl EoeTracker {
    pub(super) fn on(&mut self, ctx: &Ctx, body: &[u8], packets: &mut Vec<(u64, Vec<u8>)>) {
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

    pub(super) fn finish(self) -> EoeReport {
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
