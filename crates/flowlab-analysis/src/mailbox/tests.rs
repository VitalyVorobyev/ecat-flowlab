//! The mailbox trackers against hand-built messages.

use ecat_wire::{
    CoeHeader, CoeService, EoeHeader, EoeType, FoeHeader, FoeOpcode, MailboxHeader, SdoCommand, SdoHeader,
};

use super::*;

fn ctx(t: u64, direction: MailboxDirection) -> Ctx {
    Ctx { t_ns: t as f64, frame: t as u32, station: 0x1001, direction, buffer_len: 128, start_ts_ns: 0 }
}

fn msg(t: MailboxType, body: Vec<u8>) -> (MailboxHeader, Vec<u8>) {
    let h =
        MailboxHeader { length: body.len() as u16, address: 0, channel: 0, priority: 0, mailbox_type: t, counter: 1 };
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
    assert_eq!((o.outcome, o.kind.as_str(), o.aborted_by.as_deref()), (SdoOutcome::Aborted, "unknown", Some("master")));
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
