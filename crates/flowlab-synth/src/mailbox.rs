//! Mailbox conversations: what the master writes into a slave's mailbox, what it reads back,
//! and the polling in between.
//!
//! A conversation is a queue of steps. A `Send` is written into the slave's "mailbox out"
//! SyncManager; the slave may reject it (WKC 0) and the master retries. A `Receive` is read
//! from "mailbox in": until the slave has produced the message the read returns WKC 0 (an
//! empty poll). The slave's response becomes readable `response_delay_polls` ticks after the
//! request was accepted; consecutive messages from the slave are readable one tick apart.

use std::collections::VecDeque;

use ecat_wire::{
    CoeEmergency, CoeHeader, CoeService, EoeHeader, EoeType, FoeHeader, FoeOpcode, MAILBOX_HEADER_LEN, MailboxHeader,
    MailboxType, SdoCommand, SdoHeader,
};
use serde::Serialize;

/// Bytes of each mailbox SyncManager in the generated slaves.
pub const MBX_LEN: usize = 128;
/// Start of SM0 ("mailbox out").
pub const SM0_START: u16 = 0x1000;
/// Start of SM1 ("mailbox in").
pub const SM1_START: u16 = 0x1080;
/// Object uploaded by aborted requests (does not exist in the generated slaves).
pub const MISSING_OBJECT: u16 = 0x5FFF;
/// Abort code the slave answers with.
pub const ABORT_NO_OBJECT: u32 = 0x0602_0000;
/// Destination MAC of tunnelled EoE frames (IANA documentation range, RFC 7042).
pub const EOE_DST: [u8; 6] = [0x00, 0x00, 0x5E, 0x00, 0x53, 0x10];
/// Source MAC of tunnelled EoE frames.
pub const EOE_SRC: [u8; 6] = [0x00, 0x00, 0x5E, 0x00, 0x53, 0x11];
/// EtherType of tunnelled EoE frames: IEEE 802 local experimental 1.
pub const EOE_ETHERTYPE: u16 = 0x88B5;

/// What the generator put into mailboxes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct MailboxTruth {
    /// Messages accepted by slaves.
    pub messages_to_slaves: u64,
    /// Messages read from slaves.
    pub messages_from_slaves: u64,
    /// Reads of an empty mailbox.
    pub empty_polls: u64,
    /// Writes rejected because the mailbox was full.
    pub rejected_writes: u64,
    /// SDO transfers completed (expedited, normal and segmented, uploads and downloads).
    pub sdo_ok: u64,
    /// SDO transfers the slave aborted.
    pub sdo_aborted: u64,
    /// SDO transfers that needed segments.
    pub sdo_segmented: u64,
    /// SDO segments (upload + download).
    pub sdo_segments: u64,
    /// FoE write transfers completed.
    pub foe_transfers: u64,
    /// FoE data bytes.
    pub foe_bytes: u64,
    /// EoE frames to the slave.
    pub eoe_frames_to_slave: u64,
    /// EoE frames from the slave.
    pub eoe_frames_from_slave: u64,
    /// EoE bytes (reassembled Ethernet frames) to the slave.
    pub eoe_bytes_to_slave: u64,
    /// EoE bytes from the slave.
    pub eoe_bytes_from_slave: u64,
    /// CoE emergencies.
    pub emergencies: u64,
}

pub(crate) enum Step {
    Send(Vec<u8>),
    Receive(Vec<u8>),
}

/// Messages for one slave, with its mailbox counters.
pub(crate) struct Conversation {
    pub(crate) station: u16,
    pub(crate) steps: VecDeque<Step>,
    out_counter: u8,
    in_counter: u8,
    eoe_frame_no: u8,
    /// Tick from which the front `Receive` is readable.
    pub(crate) ready_at: u64,
}

fn next(c: &mut u8) -> u8 {
    *c = *c % 7 + 1;
    *c
}

fn mbx(counter: u8, t: MailboxType, payload: &[u8]) -> Vec<u8> {
    debug_assert!(payload.len() + MAILBOX_HEADER_LEN <= MBX_LEN);
    let h =
        MailboxHeader { length: payload.len() as u16, address: 0, channel: 0, priority: 0, mailbox_type: t, counter };
    let mut v = Vec::with_capacity(MBX_LEN);
    v.extend_from_slice(&h.encode());
    v.extend_from_slice(payload);
    v.resize(MBX_LEN, 0);
    v
}

fn coe(service: CoeService, rest: &[u8]) -> Vec<u8> {
    let mut v = CoeHeader { number: 0, service }.encode().to_vec();
    v.extend_from_slice(rest);
    v
}

fn sdo(service: CoeService, cmd: SdoCommand, index: u16, subindex: u8, data: u32, tail: &[u8]) -> Vec<u8> {
    let mut v = SdoHeader { command: cmd.encode(), index, subindex, data }.encode().to_vec();
    v.extend_from_slice(tail);
    coe(service, &v)
}

/// A segment frame: command byte then data, padded to 7 data bytes.
fn segment(service: CoeService, cmd: SdoCommand, data: &[u8]) -> Vec<u8> {
    let mut v = vec![cmd.encode()];
    v.extend_from_slice(data);
    if data.len() < 7 {
        v.resize(1 + 7, 0);
    }
    coe(service, &v)
}

/// Deterministic object content.
fn content(len: usize, seed: u8) -> Vec<u8> {
    (0..len).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed)).collect()
}

const FIRST_CHUNK: usize = MBX_LEN - MAILBOX_HEADER_LEN - CoeHeader::LEN - SdoHeader::LEN;
const SEGMENT_CHUNK: usize = MBX_LEN - MAILBOX_HEADER_LEN - CoeHeader::LEN - 1;
const FOE_CHUNK: usize = MBX_LEN - MAILBOX_HEADER_LEN - FoeHeader::LEN;
/// Non-final EoE fragments must be a multiple of 32 bytes.
const EOE_CHUNK: usize = (MBX_LEN - MAILBOX_HEADER_LEN - EoeHeader::LEN) / EoeHeader::BLOCK * EoeHeader::BLOCK;

impl Conversation {
    pub(crate) fn new(station: u16) -> Self {
        Self { station, steps: VecDeque::new(), out_counter: 0, in_counter: 0, eoe_frame_no: 0, ready_at: 0 }
    }

    fn send(&mut self, t: MailboxType, payload: Vec<u8>) {
        let c = next(&mut self.out_counter);
        self.steps.push_back(Step::Send(mbx(c, t, &payload)));
    }

    fn receive(&mut self, t: MailboxType, payload: Vec<u8>) {
        let c = next(&mut self.in_counter);
        self.steps.push_back(Step::Receive(mbx(c, t, &payload)));
    }

    /// Expedited upload of `index:subindex` returning `value`.
    pub(crate) fn expedited_upload(&mut self, index: u16, subindex: u8, value: u32, truth: &mut MailboxTruth) {
        let req = SdoCommand::InitiateUploadRequest { complete_access: false };
        self.send(MailboxType::Coe, sdo(CoeService::SdoRequest, req, index, subindex, 0, &[]));
        let resp = SdoCommand::InitiateUploadResponse {
            complete_access: false,
            expedited: true,
            size_indicated: true,
            unused: 0,
        };
        self.receive(MailboxType::Coe, sdo(CoeService::SdoResponse, resp, index, subindex, value, &[]));
        truth.sdo_ok += 1;
    }

    /// Normal (and, beyond one mailbox, segmented) upload of `len` bytes.
    pub(crate) fn segmented_upload(&mut self, index: u16, len: usize, truth: &mut MailboxTruth) {
        let data = content(len, index as u8);
        let req = SdoCommand::InitiateUploadRequest { complete_access: false };
        self.send(MailboxType::Coe, sdo(CoeService::SdoRequest, req, index, 0, 0, &[]));
        let first = len.min(FIRST_CHUNK);
        let resp = SdoCommand::InitiateUploadResponse {
            complete_access: false,
            expedited: false,
            size_indicated: true,
            unused: 0,
        };
        self.receive(MailboxType::Coe, sdo(CoeService::SdoResponse, resp, index, 0, len as u32, &data[..first]));
        let mut at = first;
        let mut toggle = false;
        if at < len {
            truth.sdo_segmented += 1;
        }
        while at < len {
            let n = (len - at).min(SEGMENT_CHUNK);
            let last = at + n == len;
            let req = SdoCommand::UploadSegmentRequest { toggle };
            self.send(MailboxType::Coe, segment(CoeService::SdoRequest, req, &[]));
            let unused = if n < 7 { 7 - n as u8 } else { 0 };
            let resp = SdoCommand::UploadSegmentResponse { toggle, unused, last };
            self.receive(MailboxType::Coe, segment(CoeService::SdoResponse, resp, &data[at..at + n]));
            truth.sdo_segments += 1;
            at += n;
            toggle = !toggle;
        }
        truth.sdo_ok += 1;
    }

    /// Download of `len` bytes: expedited up to 4 bytes, else normal plus segments.
    pub(crate) fn download(&mut self, index: u16, len: usize, truth: &mut MailboxTruth) {
        let data = content(len, 0xA5);
        let ok = SdoCommand::InitiateDownloadResponse;
        if len <= 4 {
            let mut v = [0u8; 4];
            v[..len].copy_from_slice(&data);
            let req = SdoCommand::InitiateDownloadRequest {
                complete_access: false,
                expedited: true,
                size_indicated: true,
                unused: 4 - len as u8,
            };
            self.send(MailboxType::Coe, sdo(CoeService::SdoRequest, req, index, 0, u32::from_le_bytes(v), &[]));
            self.receive(MailboxType::Coe, sdo(CoeService::SdoResponse, ok, index, 0, 0, &[]));
            truth.sdo_ok += 1;
            return;
        }
        let first = len.min(FIRST_CHUNK);
        let req = SdoCommand::InitiateDownloadRequest {
            complete_access: false,
            expedited: false,
            size_indicated: true,
            unused: 0,
        };
        self.send(MailboxType::Coe, sdo(CoeService::SdoRequest, req, index, 0, len as u32, &data[..first]));
        self.receive(MailboxType::Coe, sdo(CoeService::SdoResponse, ok, index, 0, 0, &[]));
        let mut at = first;
        let mut toggle = false;
        if at < len {
            truth.sdo_segmented += 1;
        }
        while at < len {
            let n = (len - at).min(SEGMENT_CHUNK);
            let last = at + n == len;
            let unused = if n < 7 { 7 - n as u8 } else { 0 };
            let req = SdoCommand::DownloadSegmentRequest { toggle, unused, last };
            self.send(MailboxType::Coe, segment(CoeService::SdoRequest, req, &data[at..at + n]));
            let resp = SdoCommand::DownloadSegmentResponse { toggle };
            self.receive(MailboxType::Coe, segment(CoeService::SdoResponse, resp, &[]));
            truth.sdo_segments += 1;
            at += n;
            toggle = !toggle;
        }
        truth.sdo_ok += 1;
    }

    /// Upload of an object the slave does not have; the slave aborts.
    pub(crate) fn aborted_upload(&mut self, truth: &mut MailboxTruth) {
        let req = SdoCommand::InitiateUploadRequest { complete_access: false };
        self.send(MailboxType::Coe, sdo(CoeService::SdoRequest, req, MISSING_OBJECT, 0, 0, &[]));
        // The abort transfer request uses the SDO request service in either direction.
        self.receive(
            MailboxType::Coe,
            sdo(CoeService::SdoRequest, SdoCommand::Abort, MISSING_OBJECT, 0, ABORT_NO_OBJECT, &[]),
        );
        truth.sdo_aborted += 1;
    }

    /// FoE write of `len` bytes as `name`.
    pub(crate) fn foe_write(&mut self, name: &str, len: usize, truth: &mut MailboxTruth) {
        let foe = |op: FoeOpcode, value: u32, tail: &[u8]| {
            let mut v = FoeHeader { opcode: op, value }.encode().to_vec();
            v.extend_from_slice(tail);
            v
        };
        let data = content(len, 0x3C);
        self.send(MailboxType::Foe, foe(FoeOpcode::Write, 0, name.as_bytes()));
        self.receive(MailboxType::Foe, foe(FoeOpcode::Ack, 0, &[]));
        let mut packet = 0u32;
        let mut at = 0;
        loop {
            let n = (len - at).min(FOE_CHUNK);
            packet += 1;
            self.send(MailboxType::Foe, foe(FoeOpcode::Data, packet, &data[at..at + n]));
            self.receive(MailboxType::Foe, foe(FoeOpcode::Ack, packet, &[]));
            at += n;
            // A packet shorter than the mailbox allows ends the file; a file that is a whole
            // number of packets ends with an empty one.
            if n < FOE_CHUNK {
                break;
            }
        }
        truth.foe_transfers += 1;
        truth.foe_bytes += len as u64;
    }

    /// One Ethernet frame of `len` bytes tunnelled by EoE, to (`to_slave`) or from the slave.
    pub(crate) fn eoe_frame(&mut self, len: usize, to_slave: bool, truth: &mut MailboxTruth) {
        let mut frame = Vec::with_capacity(len);
        let (dst, src) = if to_slave { (EOE_DST, EOE_SRC) } else { (EOE_SRC, EOE_DST) };
        frame.extend_from_slice(&dst);
        frame.extend_from_slice(&src);
        frame.extend_from_slice(&EOE_ETHERTYPE.to_be_bytes());
        frame.extend(content(len.saturating_sub(14), self.eoe_frame_no));
        self.eoe_frame_no = self.eoe_frame_no % 15 + 1;
        let frame_no = self.eoe_frame_no;
        let mut at = 0;
        let mut fragment = 0u8;
        while at < frame.len() {
            let n = (frame.len() - at).min(EOE_CHUNK);
            let last = at + n == frame.len();
            let offset_blocks =
                if fragment == 0 { frame.len().div_ceil(EoeHeader::BLOCK) } else { at / EoeHeader::BLOCK };
            let h = EoeHeader {
                frame_type: EoeType::Fragment,
                port: 0,
                last_fragment: last,
                time_appended: false,
                time_request: false,
                fragment,
                offset_blocks: offset_blocks as u8,
                frame_no,
            };
            let mut v = h.encode().to_vec();
            v.extend_from_slice(&frame[at..at + n]);
            if to_slave {
                self.send(MailboxType::Eoe, v);
            } else {
                self.receive(MailboxType::Eoe, v);
            }
            at += n;
            fragment += 1;
        }
        if to_slave {
            truth.eoe_frames_to_slave += 1;
            truth.eoe_bytes_to_slave += frame.len() as u64;
        } else {
            truth.eoe_frames_from_slave += 1;
            truth.eoe_bytes_from_slave += frame.len() as u64;
        }
    }

    /// An unsolicited CoE emergency from the slave.
    pub(crate) fn emergency(&mut self, error_code: u16, truth: &mut MailboxTruth) {
        let e = CoeEmergency { error_code, error_register: 0x80, data: [0, 0, 0, 0, 0] };
        self.receive(MailboxType::Coe, coe(CoeService::Emergency, &e.encode()));
        truth.emergencies += 1;
    }
}
