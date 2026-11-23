//! FlowLab VoE: a bulk transfer protocol of FlowLab's own, carried in VoE (vendor specific over
//! EtherCAT, mailbox type 15).
//!
//! It moves a payload in either direction with as little mailbox traffic as possible:
//!
//! * a DATA message carries as many payload bytes as the mailbox holds after a 2-byte
//!   operation/sequence word ([`capacity`]); there is no fragment rounding and no inner header;
//! * the receiver acknowledges cumulatively, once per `window` DATA messages ([`Op::Ack`]),
//!   so the sender writes DATA whenever the mailbox is free and reads an answer only once per
//!   window;
//! * the receiver of a payload sent to the slave ends the transfer with [`Op::Done`], which
//!   carries the length and SHA-256 of what it received: the sender verifies the transfer
//!   without another channel.
//!
//! Every message starts with the VoE header (vendor ID [`VENDOR_ID`], vendor type
//! [`VENDOR_TYPE`]) and a little-endian word: the operation in bits 15–13 and a sequence
//! number in bits 12–0. DATA messages number themselves 0, 1, 2, … modulo [`SEQ_MOD`]; an ACK
//! carries the number of the next DATA message it expects.
//!
//! | Operation | From | Sequence | After the word |
//! |---|---|---|---|
//! | OPEN | master | 0 | direction (u8: 0 to the slave, 1 from it), 0 (u8), window (u16), length (u64; 0 from the slave) |
//! | READY | slave | 0 | window granted (u16), length (u64) |
//! | DATA | sender | message number | payload bytes |
//! | ACK | receiver | next expected | — |
//! | DONE | slave | 0 | length (u64), SHA-256 (32 bytes) of the payload it received |
//! | ABORT | either | 0 | reason (u16), text |
//!
//! A transfer from the slave ends when the master has received the length READY announced and
//! has acknowledged the last DATA message.
//!
//! Losses: writes to "mailbox out" are confirmed by their working counter and repeated, so
//! DATA to the slave arrives in order. DATA from the slave can be lost: when the master's read
//! of "mailbox in" does not come back, the slave may already have written the next message,
//! and the mailbox repeat request returns that one. The master then acknowledges the number it
//! still expects, once per gap ([`classify`]), and the slave goes back to it and sends again
//! from there (go-back-N). An ACK or DONE that does not reach the master is recovered when the
//! master, after its reply timeout, writes its last message again: the slave answers a repeated
//! DATA message with an ACK of what it expects, and with its DONE once the payload is
//! complete.

use ecat_wire::{MAILBOX_HEADER_LEN, VoeHeader};

/// Vendor ID of FlowLab VoE messages (0: FlowLab has no ETG vendor ID).
pub const VENDOR_ID: u32 = 0;
/// Vendor type of FlowLab VoE messages ("FL").
pub const VENDOR_TYPE: u16 = 0x464C;
/// Bytes before the payload of a DATA message, after the mailbox header.
pub const HEADER: usize = VoeHeader::LEN + 2;
/// DATA messages are numbered modulo this.
pub const SEQ_MOD: u32 = 1 << 13;
/// Largest window: half the sequence space, so a cumulative ACK is never ambiguous.
pub const MAX_WINDOW: u16 = (SEQ_MOD / 2) as u16;
/// Window a master asks for unless told otherwise.
pub const DEFAULT_WINDOW: u16 = 16;
/// Smallest mailbox a FlowLab VoE transfer runs in: room for DONE.
pub const MIN_MAILBOX: usize = MAILBOX_HEADER_LEN + HEADER + 8 + 32;
/// ABORT reason: the slave cannot send (nothing to upload) or receive.
pub const ABORT_REFUSED: u16 = 1;
/// ABORT reason: a message out of order.
pub const ABORT_PROTOCOL: u16 = 2;

/// What a FlowLab VoE message does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    /// The master starts a transfer.
    Open,
    /// The slave accepts it.
    Ready,
    /// Payload bytes.
    Data,
    /// The receiver got every DATA message before the given number.
    Ack,
    /// The slave received the whole payload: its length and SHA-256.
    Done,
    /// The transfer is abandoned.
    Abort,
}

impl Op {
    const fn code(self) -> u16 {
        match self {
            Self::Open => 0,
            Self::Ready => 1,
            Self::Data => 2,
            Self::Ack => 3,
            Self::Done => 4,
            Self::Abort => 5,
        }
    }

    const fn from_code(c: u16) -> Option<Self> {
        Some(match c {
            0 => Self::Open,
            1 => Self::Ready,
            2 => Self::Data,
            3 => Self::Ack,
            4 => Self::Done,
            5 => Self::Abort,
            _ => return None,
        })
    }

    /// Upper-case name, as on the wire documentation.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Open => "OPEN",
            Self::Ready => "READY",
            Self::Data => "DATA",
            Self::Ack => "ACK",
            Self::Done => "DONE",
            Self::Abort => "ABORT",
        }
    }
}

/// Direction of a transfer, as OPEN states it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Master → slave.
    ToSlave,
    /// Slave → master.
    FromSlave,
}

/// A parsed FlowLab VoE message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Message<'a> {
    /// Operation.
    pub op: Op,
    /// Sequence number (DATA: its number; ACK: the next expected).
    pub seq: u16,
    /// Bytes after the operation/sequence word.
    pub tail: &'a [u8],
}

/// Payload bytes of a DATA message in a mailbox of `mailbox_len` bytes.
pub const fn capacity(mailbox_len: usize) -> usize {
    mailbox_len.saturating_sub(MAILBOX_HEADER_LEN + HEADER)
}

/// A message: VoE header, operation and `seq` (modulo [`SEQ_MOD`]), `tail`.
pub fn encode(op: Op, seq: u32, tail: &[u8]) -> Vec<u8> {
    let mut v = VoeHeader { vendor_id: VENDOR_ID, vendor_type: VENDOR_TYPE }.encode().to_vec();
    let word = (op.code() << 13) | (seq % SEQ_MOD) as u16;
    v.extend_from_slice(&word.to_le_bytes());
    v.extend_from_slice(tail);
    v
}

/// The message in a VoE body (the data after the mailbox header), if it is a FlowLab VoE one.
pub fn parse(body: &[u8]) -> Option<Message<'_>> {
    let h = VoeHeader::parse(body).ok()?;
    if h.vendor_id != VENDOR_ID || h.vendor_type != VENDOR_TYPE || body.len() < HEADER {
        return None;
    }
    let word = u16::from_le_bytes([body[VoeHeader::LEN], body[VoeHeader::LEN + 1]]);
    Some(Message { op: Op::from_code(word >> 13)?, seq: word & (SEQ_MOD as u16 - 1), tail: &body[HEADER..] })
}

/// OPEN.
pub fn open(direction: Direction, window: u16, length: u64) -> Vec<u8> {
    let mut t = vec![u8::from(direction == Direction::FromSlave), 0];
    t.extend_from_slice(&window.to_le_bytes());
    t.extend_from_slice(&length.to_le_bytes());
    encode(Op::Open, 0, &t)
}

/// The direction, window and length of an OPEN's tail.
pub fn parse_open(tail: &[u8]) -> Option<(Direction, u16, u64)> {
    let t: &[u8; 12] = tail.get(..12)?.try_into().ok()?;
    let dir = if t[0] == 0 { Direction::ToSlave } else { Direction::FromSlave };
    Some((dir, u16::from_le_bytes([t[2], t[3]]), u64::from_le_bytes(t[4..12].try_into().ok()?)))
}

/// READY.
pub fn ready(window: u16, length: u64) -> Vec<u8> {
    let mut t = window.to_le_bytes().to_vec();
    t.extend_from_slice(&length.to_le_bytes());
    encode(Op::Ready, 0, &t)
}

/// The window and length of a READY's tail.
pub fn parse_ready(tail: &[u8]) -> Option<(u16, u64)> {
    let t: &[u8; 10] = tail.get(..10)?.try_into().ok()?;
    Some((u16::from_le_bytes([t[0], t[1]]), u64::from_le_bytes(t[2..10].try_into().ok()?)))
}

/// DONE.
pub fn done(length: u64, sha256: &[u8; 32]) -> Vec<u8> {
    let mut t = length.to_le_bytes().to_vec();
    t.extend_from_slice(sha256);
    encode(Op::Done, 0, &t)
}

/// The length and SHA-256 of a DONE's tail.
pub fn parse_done(tail: &[u8]) -> Option<(u64, [u8; 32])> {
    let t: &[u8; 40] = tail.get(..40)?.try_into().ok()?;
    Some((u64::from_le_bytes(t[..8].try_into().ok()?), t[8..40].try_into().ok()?))
}

/// ABORT with `reason` and a text.
pub fn abort(reason: u16, text: &str) -> Vec<u8> {
    let mut t = reason.to_le_bytes().to_vec();
    t.extend_from_slice(text.as_bytes());
    encode(Op::Abort, 0, &t)
}

/// The reason and text of an ABORT's tail, as one line.
pub fn abort_text(tail: &[u8]) -> String {
    let reason = tail.get(..2).map_or(0, |r| u16::from_le_bytes([r[0], r[1]]));
    let text = String::from_utf8_lossy(tail.get(2..).unwrap_or_default());
    format!("FlowLab VoE abort {reason}: {}", text.trim_end_matches('\0'))
}

/// The DATA message count a 13-bit `seq` stands for, given that `base` messages are already
/// accounted for and the answer lies within `base..=base + window`.
pub const fn unwrap_seq(base: u64, seq: u16) -> u64 {
    let ahead = (seq as u64 + SEQ_MOD as u64 - base % SEQ_MOD as u64) % SEQ_MOD as u64;
    base + ahead
}

/// Where a received DATA message's `seq` lies relative to the `expected` message count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrival {
    /// The message expected next.
    Next,
    /// A later one: messages in between were lost.
    Ahead,
    /// An earlier one: a repeat.
    Behind,
}

/// Classify a DATA message's `seq` against the number of messages received so far.
pub const fn classify(expected: u64, seq: u16) -> Arrival {
    let ahead = (seq as u64 + SEQ_MOD as u64 - expected % SEQ_MOD as u64) % SEQ_MOD as u64;
    if ahead == 0 {
        Arrival::Next
    } else if ahead < MAX_WINDOW as u64 {
        Arrival::Ahead
    } else {
        Arrival::Behind
    }
}

/// SHA-256 of `bytes`.
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip() {
        let m = open(Direction::FromSlave, 16, 0);
        let p = parse(&m).unwrap();
        assert_eq!((p.op, p.seq), (Op::Open, 0));
        assert_eq!(parse_open(p.tail), Some((Direction::FromSlave, 16, 0)));
        let r = ready(8, 1_000_000);
        assert_eq!(parse_ready(parse(&r).unwrap().tail), Some((8, 1_000_000)));
        let d = encode(Op::Data, SEQ_MOD + 5, b"abc");
        let p = parse(&d).unwrap();
        assert_eq!((p.op, p.seq, p.tail), (Op::Data, 5, &b"abc"[..]));
        let digest = sha256(b"abc");
        assert_eq!(parse_done(parse(&done(3, &digest)).unwrap().tail), Some((3, digest)));
        assert_eq!(abort_text(parse(&abort(2, "late")).unwrap().tail), "FlowLab VoE abort 2: late");
        assert_eq!(capacity(128), 128 - 6 - 6 - 2);
        const { assert!(MIN_MAILBOX <= 64) };
    }

    #[test]
    fn other_vendors_are_not_flowlab_voe() {
        let mut m = encode(Op::Ack, 1, &[]);
        m[0] = 1;
        assert_eq!(parse(&m), None);
        assert_eq!(parse(&[0; 4]), None);
    }

    #[test]
    fn sequence_numbers_unwrap_across_the_modulus() {
        assert_eq!(unwrap_seq(0, 0), 0);
        assert_eq!(unwrap_seq(10, 26), 26);
        assert_eq!(unwrap_seq(SEQ_MOD as u64 - 2, 3), SEQ_MOD as u64 + 3);
        assert_eq!(unwrap_seq(3 * SEQ_MOD as u64, 0), 3 * SEQ_MOD as u64);
        assert_eq!(classify(10, 10), Arrival::Next);
        assert_eq!(classify(10, 12), Arrival::Ahead);
        assert_eq!(classify(10, 9), Arrival::Behind);
        assert_eq!(classify(SEQ_MOD as u64, 0), Arrival::Next);
        assert_eq!(classify(SEQ_MOD as u64 + 1, (SEQ_MOD - 1) as u16), Arrival::Behind);
    }
}
