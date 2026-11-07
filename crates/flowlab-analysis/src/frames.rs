//! Listing, filtering and explaining individual frames.

use ecat_wire::{
    AlStatus, DatagramAddress, MAILBOX_HEADER_LEN, MailboxHeader, MailboxType, parse_frame, register_info, wire_bytes,
};
use flowlab_capture::{Direction, FrameKind, Trace};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::analyze::{Analysis, NONE};
use crate::mailbox::{ANY_MAILBOX, Access, describe, protocol_name};
use crate::report::ChannelSource;

/// Which frames to list. All conditions must hold.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct FrameFilter {
    /// A datagram has this command mnemonic (`"LRW"`).
    pub command: Option<String>,
    /// `"tx"`, `"rx"` or `"unknown"`.
    pub direction: Option<String>,
    /// A datagram addresses this configured station.
    pub station: Option<u16>,
    /// The frame belongs to this cyclic stream; `65535` = to none.
    pub stream: Option<u16>,
    /// Only frames with a WKC mismatch, no answer, or a parse error.
    pub problems_only: bool,
    /// Include non-EtherCAT frames.
    pub include_other: bool,
    /// `"any"`: a datagram accesses a mailbox; or a protocol (`"coe"`, `"foe"`, `"eoe"`,
    /// `"soe"`, `"voe"`, `"aoe"`, `"error"`): the frame carries a message of it.
    pub mailbox: Option<String>,
}

fn mailbox_mask(name: &str) -> Option<u32> {
    let t = match name.to_ascii_lowercase().as_str() {
        "any" => return Some(ANY_MAILBOX),
        "error" => MailboxType::Error,
        "aoe" => MailboxType::Aoe,
        "eoe" => MailboxType::Eoe,
        "coe" => MailboxType::Coe,
        "foe" => MailboxType::Foe,
        "soe" => MailboxType::Soe,
        "voe" => MailboxType::Voe,
        _ => return None,
    };
    Some(1 << t.code())
}

/// One row of the frame list.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct FrameRow {
    /// Frame number (1-based).
    pub no: u32,
    /// Time since capture start, ns.
    pub t_ns: f64,
    /// Direction.
    pub direction: Direction,
    /// Classification.
    pub kind: FrameKind,
    /// Captured length.
    pub len: u32,
    /// Datagram count.
    pub datagrams: u16,
    /// Compact description of the datagrams.
    pub summary: String,
    /// Stream id.
    pub stream: Option<u16>,
    /// WKC differs from the stream's expected value.
    pub wkc_mismatch: bool,
    /// TX frame that never returned.
    pub unanswered: bool,
    /// Paired TX/RX frame number.
    pub paired: Option<u32>,
    /// Round trip for a paired TX frame, ns.
    pub rtt_ns: Option<f64>,
}

/// A page of rows.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct FramePage {
    /// Rows matching the filter.
    pub total: u64,
    /// Offset of the first row.
    pub offset: u64,
    /// The rows.
    pub rows: Vec<FrameRow>,
}

fn direction_name(d: Direction) -> &'static str {
    match d {
        Direction::Tx => "tx",
        Direction::Rx => "rx",
        Direction::Unknown => "unknown",
    }
}

fn matches(trace: &Trace, a: &Analysis, i: usize, f: &FrameFilter) -> bool {
    let fr = &trace.frames[i];
    if fr.kind == FrameKind::Other && !f.include_other {
        return false;
    }
    if let Some(d) = &f.direction
        && direction_name(fr.direction) != d
    {
        return false;
    }
    if let Some(s) = f.stream
        && a.stream_of[i] != s
    {
        return false;
    }
    if f.problems_only && !(a.wkc_mismatch[i] || a.unanswered[i] || fr.kind == FrameKind::Malformed) {
        return false;
    }
    if let Some(m) = &f.mailbox
        && mailbox_mask(m).is_none_or(|mask| a.mailbox.frame_flags[i] & mask == 0)
    {
        return false;
    }
    if f.command.is_some() || f.station.is_some() {
        let dgs = trace.datagrams_of(fr);
        if let Some(c) = &f.command
            && !dgs.iter().any(|d| d.command.mnemonic().eq_ignore_ascii_case(c))
        {
            return false;
        }
        if let Some(st) = f.station
            && !dgs.iter().any(|d| matches!(d.address(), DatagramAddress::Fixed { station, .. } if station == st))
        {
            return false;
        }
    }
    true
}

fn row(trace: &Trace, a: &Analysis, i: usize) -> FrameRow {
    let f = &trace.frames[i];
    let dgs = trace.datagrams_of(f);
    let summary = match f.kind {
        FrameKind::Other => "non-EtherCAT".to_owned(),
        FrameKind::Malformed => "malformed EtherCAT".to_owned(),
        FrameKind::Ethercat => {
            let mut parts: Vec<String> = dgs.iter().take(4).map(short_label).collect();
            if dgs.len() > 4 {
                parts.push(format!("+{}", dgs.len() - 4));
            }
            let mut s = parts.join(" · ");
            let msg = (0..dgs.len() as u16).find_map(|p| a.mailbox.by_datagram.get(&(i as u32, p)));
            if let Some(&m) = msg {
                s.push_str(" — ");
                s.push_str(&a.mailbox.messages[m as usize].summary);
            } else if f.direction == Direction::Rx {
                let note = dgs.iter().find_map(|d| match (a.mailbox.channels.access(d), d.wkc) {
                    (Some(Access::In), 0) => Some(" — mailbox empty"),
                    (Some(Access::Out), 0) => Some(" — mailbox full, write rejected"),
                    _ => None,
                });
                s.push_str(note.unwrap_or(""));
            }
            s
        }
    };
    let paired = (a.pair_of[i] != NONE).then(|| a.pair_of[i]);
    FrameRow {
        no: i as u32 + 1,
        t_ns: f.t_ns as f64,
        direction: f.direction,
        kind: f.kind,
        len: f.captured_len,
        datagrams: f.datagram_count,
        summary,
        stream: (a.stream_of[i] != u16::MAX).then_some(a.stream_of[i]),
        wkc_mismatch: a.wkc_mismatch[i],
        unanswered: a.unanswered[i],
        paired: paired.map(|p| p + 1),
        rtt_ns: paired
            .filter(|_| f.direction == Direction::Tx)
            .map(|p| trace.frames[p as usize].t_ns as f64 - f.t_ns as f64),
    }
}

fn short_label(d: &flowlab_capture::DatagramRecord) -> String {
    match d.address() {
        DatagramAddress::Logical { address } => format!("{} 0x{address:08X} [{}]", d.command, d.len),
        DatagramAddress::Fixed { station, ado } => format!("{} 0x{station:04X}:{:04X}", d.command, ado),
        DatagramAddress::Position { ado, .. } => {
            format!("{} #{}:{:04X}", d.command, d.address().position().unwrap_or(0), ado)
        }
        DatagramAddress::Broadcast { ado, .. } => format!("{} {:04X}", d.command, ado),
        DatagramAddress::Raw { .. } => d.command.to_string(),
    }
}

/// Rows `offset..offset+limit` of the frames matching `filter`, in capture order.
pub fn query(trace: &Trace, a: &Analysis, filter: &FrameFilter, offset: u64, limit: u64) -> FramePage {
    let mut total = 0u64;
    let mut rows = Vec::with_capacity(limit.min(1000) as usize);
    for i in 0..trace.frames.len() {
        if !matches(trace, a, i, filter) {
            continue;
        }
        if total >= offset && (rows.len() as u64) < limit {
            rows.push(row(trace, a, i));
        }
        total += 1;
    }
    FramePage { total, offset, rows }
}

/// A labelled byte range of the frame, for hex highlighting.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ByteRange {
    /// First byte.
    pub start: u32,
    /// Length.
    pub len: u32,
    /// Label.
    pub label: String,
    /// `"ethernet"`, `"ecat_header"`, `"datagram_header"`, `"data"`, `"wkc"`, `"padding"`.
    pub kind: String,
    /// Datagram position for datagram parts.
    pub datagram: Option<u16>,
}

/// Interpreted datagram address.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct AddressInfo {
    /// `"position"`, `"fixed"`, `"broadcast"`, `"logical"` or `"none"`.
    pub mode: String,
    /// Display text.
    pub text: String,
    /// Configured station.
    pub station: Option<u16>,
    /// Ring position.
    pub position: Option<u16>,
    /// Physical offset.
    pub ado: Option<u16>,
    /// Logical address.
    pub logical: Option<u32>,
    /// Register name at `ado`.
    pub register: Option<String>,
    /// Register area at `ado`.
    pub register_area: Option<String>,
}

/// A labelled value of a decoded structure.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct Field {
    /// Label.
    pub name: String,
    /// Value as text.
    pub value: String,
}

/// The mailbox header of a message.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct MailboxHeaderInfo {
    /// Protocol (`"CoE"`, …).
    pub protocol: String,
    /// Declared length of the data after the header.
    pub length: u16,
    /// Address field.
    pub address: u16,
    /// Channel.
    pub channel: u8,
    /// Priority.
    pub priority: u8,
    /// Counter.
    pub counter: u8,
}

/// How a datagram uses a slave's mailbox, and what it carries.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct MailboxInfo {
    /// `"write"`, `"write rejected"` (mailbox full), `"write attempt"` (TX), `"read"`,
    /// `"empty read"`, `"read attempt"` (TX) or `"status poll"`.
    pub role: String,
    /// How the mailbox address was learned.
    pub source: ChannelSource,
    /// Header, when the datagram carries a message.
    pub header: Option<MailboxHeaderInfo>,
    /// One-line description of the message.
    pub summary: Option<String>,
    /// Decoded protocol fields.
    pub fields: Vec<Field>,
    /// The message repeats the previous one's counter and was not counted.
    pub repeat: bool,
}

/// Decoded datagram.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct DatagramDetail {
    /// Position in the frame.
    pub position: u16,
    /// Command mnemonic.
    pub command: String,
    /// Command code.
    pub command_code: u8,
    /// Datagram index.
    pub index: u8,
    /// Address.
    pub address: AddressInfo,
    /// Data length.
    pub len: u16,
    /// Circulating flag.
    pub circulating: bool,
    /// More-follows flag.
    pub more: bool,
    /// IRQ field.
    pub irq: u16,
    /// Working counter.
    pub wkc: u16,
    /// Expected WKC when the frame is in a stream and returned.
    pub expected_wkc: Option<u16>,
    /// Decoded AL control/status for accesses at 0x0120/0x0130.
    pub al_state: Option<String>,
    /// Mailbox content, if the data looks like a mailbox.
    pub mailbox: Option<MailboxInfo>,
}

/// Everything about one frame.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct FrameDetail {
    /// The list row.
    pub row: FrameRow,
    /// Original length.
    pub orig_len: u32,
    /// Bytes on the wire incl. preamble, FCS, padding, IFG.
    pub wire_bytes: u32,
    /// Destination MAC.
    pub dst: Option<String>,
    /// Source MAC.
    pub src: Option<String>,
    /// VLAN TCI.
    pub vlan: Option<u16>,
    /// EtherType.
    pub ethertype: Option<u16>,
    /// `"ethernet"` or `"udp"`.
    pub encapsulation: Option<String>,
    /// EtherCAT header length field.
    pub ecat_length: Option<u16>,
    /// EtherCAT header type field.
    pub ecat_type: Option<u8>,
    /// Datagrams.
    pub datagrams: Vec<DatagramDetail>,
    /// Labelled byte ranges.
    pub ranges: Vec<ByteRange>,
    /// Frame bytes, lower-case hex.
    pub hex: String,
    /// Parse error, for malformed frames.
    pub error: Option<String>,
}

fn address_info(d: &ecat_wire::Datagram<'_>) -> AddressInfo {
    let a = d.address;
    let reg = a.ado().and_then(register_info);
    let (mode, text) = match a {
        DatagramAddress::Raw { value } => ("none", format!("0x{value:08X}")),
        DatagramAddress::Position { adp, ado } => {
            ("position", format!("position {} (ADP 0x{adp:04X}), offset 0x{ado:04X}", a.position().unwrap_or(0)))
        }
        DatagramAddress::Fixed { station, ado } => ("fixed", format!("station 0x{station:04X}, offset 0x{ado:04X}")),
        DatagramAddress::Broadcast { adp, ado } => {
            ("broadcast", format!("broadcast (ADP 0x{adp:04X}), offset 0x{ado:04X}"))
        }
        DatagramAddress::Logical { address } => ("logical", format!("logical 0x{address:08X}")),
    };
    AddressInfo {
        mode: mode.to_owned(),
        text,
        station: match a {
            DatagramAddress::Fixed { station, .. } => Some(station),
            _ => None,
        },
        position: a.position(),
        ado: a.ado(),
        logical: match a {
            DatagramAddress::Logical { address } => Some(address),
            _ => None,
        },
        register: reg.map(|r| r.name.to_owned()),
        register_area: reg.map(|r| r.area.to_owned()),
    }
}

fn mailbox_info(trace: &Trace, a: &Analysis, i: usize, p: u16) -> Option<MailboxInfo> {
    let f = &trace.frames[i];
    let rec = trace.datagrams_of(f).get(p as usize)?;
    let access = a.mailbox.channels.access(rec)?;
    let DatagramAddress::Fixed { station, .. } = rec.address() else { return None };
    let tx = f.direction == Direction::Tx;
    let role = match access {
        Access::Status => "status poll",
        Access::Out if tx => "write attempt",
        Access::Out if rec.wkc == 0 => "write rejected",
        Access::Out => "write",
        Access::In if tx => "read attempt",
        Access::In if rec.wkc == 0 => "empty read",
        Access::In => "read",
    };
    let carries = matches!(role, "write attempt" | "write rejected" | "write" | "read");
    let data = trace.datagram_data(rec);
    let parsed = carries
        .then(|| MailboxHeader::parse(data).ok())
        .flatten()
        .filter(|h| h.length as usize + MAILBOX_HEADER_LEN <= data.len());
    let (header, summary, fields) = match parsed {
        Some(h) => {
            let (summary, fields) = describe(&h, &data[MAILBOX_HEADER_LEN..MAILBOX_HEADER_LEN + h.length as usize]);
            (
                Some(MailboxHeaderInfo {
                    protocol: protocol_name(h.mailbox_type).to_owned(),
                    length: h.length,
                    address: h.address,
                    channel: h.channel,
                    priority: h.priority,
                    counter: h.counter,
                }),
                Some(summary),
                fields.into_iter().map(|(name, value)| Field { name, value }).collect(),
            )
        }
        None => (None, None, Vec::new()),
    };
    let repeat = a.mailbox.by_datagram.get(&(i as u32, p)).is_some_and(|&m| a.mailbox.messages[m as usize].repeat);
    Some(MailboxInfo {
        role: role.to_owned(),
        source: a.mailbox.channels.source(station).unwrap_or(ChannelSource::Heuristic),
        header,
        summary,
        fields,
        repeat,
    })
}

/// Full decode of frame number `no` (1-based). `None` if out of range.
pub fn detail(trace: &Trace, a: &Analysis, no: u32) -> Option<FrameDetail> {
    let i = no.checked_sub(1)? as usize;
    let f = trace.frames.get(i)?;
    let bytes = trace.frame_bytes(f);
    let mut out = FrameDetail {
        row: row(trace, a, i),
        orig_len: f.orig_len,
        wire_bytes: wire_bytes(f.orig_len.max(f.captured_len)),
        dst: None,
        src: None,
        vlan: None,
        ethertype: None,
        encapsulation: None,
        ecat_length: None,
        ecat_type: None,
        datagrams: Vec::new(),
        ranges: Vec::new(),
        hex: bytes.iter().map(|b| format!("{b:02x}")).collect(),
        error: trace.errors.iter().find(|(fi, _)| *fi as usize == i).map(|(_, e)| e.clone()),
    };
    let range = |start: usize, len: usize, label: String, kind: &str, dg: Option<u16>| ByteRange {
        start: start as u32,
        len: len as u32,
        label,
        kind: kind.to_owned(),
        datagram: dg,
    };
    let parsed = match parse_frame(bytes) {
        Ok(Some(p)) => p,
        _ => return Some(out),
    };
    out.dst = Some(parsed.ethernet.dst.to_string());
    out.src = Some(parsed.ethernet.src.to_string());
    out.vlan = parsed.ethernet.vlan_tci;
    out.ethertype = Some(parsed.ethernet.ethertype);
    out.encapsulation = Some(format!("{:?}", parsed.encapsulation).to_lowercase());
    out.ecat_length = Some(parsed.pdu.header.length);
    out.ecat_type = Some(parsed.pdu.header.pdu_type.code());
    out.ranges.push(range(0, parsed.pdu_offset, "Ethernet".into(), "ethernet", None));
    out.ranges.push(range(parsed.pdu_offset, 2, "EtherCAT header".into(), "ecat_header", None));
    let base = parsed.pdu_offset + 2;
    let mut end = base;
    for (p, dg) in parsed.pdu.datagrams().enumerate() {
        let Ok(d) = dg else { break };
        let p16 = p as u16;
        let at = base + d.offset;
        let name = d.command.to_string();
        out.ranges.push(range(at, 10, format!("#{p} {name} header"), "datagram_header", Some(p16)));
        if !d.data.is_empty() {
            out.ranges.push(range(at + 10, d.data.len(), format!("#{p} data"), "data", Some(p16)));
        }
        out.ranges.push(range(at + 10 + d.data.len(), 2, format!("#{p} WKC"), "wkc", Some(p16)));
        end = at + d.encoded_len();
        let al_state = match d.address.ado() {
            Some(0x0120) | Some(0x0130)
                if !d.data.is_empty() && !matches!(d.address, DatagramAddress::Logical { .. }) =>
            {
                let s = AlStatus::from_byte(d.data[0]);
                Some(format!("{}{}", s.state.name(), if s.error { " + error" } else { "" }))
            }
            _ => None,
        };
        let stream = a.stream_of[i];
        let returned = f.direction == Direction::Rx;
        out.datagrams.push(DatagramDetail {
            position: p16,
            command: name,
            command_code: d.command.code(),
            index: d.index,
            address: address_info(&d),
            len: d.data.len() as u16,
            circulating: d.circulating,
            more: d.more,
            irq: d.irq,
            wkc: d.wkc,
            expected_wkc: if returned && stream != u16::MAX {
                a.expected_wkc.get(&(stream, p16)).copied()
            } else {
                None
            },
            al_state,
            mailbox: mailbox_info(trace, a, i, p16),
        });
    }
    if bytes.len() > end {
        out.ranges.push(range(end, bytes.len() - end, "Padding".into(), "padding", None));
    }
    Some(out)
}
