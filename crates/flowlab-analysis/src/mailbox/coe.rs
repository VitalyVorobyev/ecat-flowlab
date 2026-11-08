//! CoE: SDO transfers followed as a state machine per station, emergencies, SDO Information.

use std::collections::HashMap;

use ecat_wire::{
    CoeEmergency, CoeHeader, CoeService, MailboxHeader, SdoCommand, SdoHeader, sdo_abort_text, segment_data_len,
};

use crate::stats::Distribution;

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Expect {
    InitResponse,
    UploadSegRequest,
    UploadSegResponse,
    DownloadSegRequest,
    DownloadSegResponse,
}

pub(super) struct OpenSdo {
    pub(super) t: SdoTransaction,
    pub(super) expect: Expect,
    pub(super) toggle: bool,
    pub(super) last_pending: bool,
}

#[derive(Default)]
pub(super) struct CoeTracker {
    pub(super) open: HashMap<u16, OpenSdo>,
    pub(super) done: Vec<SdoTransaction>,
    pub(super) emergencies: Vec<EmergencyEvent>,
    pub(super) emergencies_total: u64,
    pub(super) info: u64,
}

pub(super) fn data_after_sdo_header(body: &[u8]) -> u64 {
    body.len().saturating_sub(CoeHeader::LEN + SdoHeader::LEN) as u64
}

impl CoeTracker {
    pub(super) fn close(&mut self, station: u16, ctx: &Ctx, outcome: SdoOutcome, note: Option<String>) {
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

    pub(super) fn on(&mut self, ctx: &Ctx, h: &MailboxHeader, body: &[u8]) {
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
                        data: flowlab_core::hex(&e.data),
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

    pub(super) fn sdo(&mut self, ctx: &Ctx, h: &MailboxHeader, body: &[u8], cmd: SdoCommand) {
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

    pub(super) fn finish(mut self, bins: usize) -> CoeReport {
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
