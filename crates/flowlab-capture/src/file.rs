//! pcap and pcapng reading from memory: formats, interfaces, timestamp resolution and offsets.

use thiserror::Error;

/// Link type of Ethernet (DLT_EN10MB).
pub const LINKTYPE_ETHERNET: u16 = 1;

const PCAP_MAGIC_US: u32 = 0xA1B2_C3D4;
const PCAP_MAGIC_NS: u32 = 0xA1B2_3C4D;
const PCAPNG_SHB: u32 = 0x0A0D_0D0A;
const PCAPNG_BOM: u32 = 0x1A2B_3C4D;

/// Why a capture file could not be read.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CaptureError {
    /// Neither a pcap nor a pcapng magic number.
    #[error("not a pcap or pcapng file")]
    UnknownFormat,
    /// The file is structurally broken.
    #[error("malformed {format} at byte {offset}: {reason}")]
    Malformed {
        /// `"pcap"` or `"pcapng"`.
        format: &'static str,
        /// Byte offset of the problem.
        offset: usize,
        /// What is wrong.
        reason: &'static str,
    },
    /// I/O error while reading or writing.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Container format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CaptureFormat {
    /// Classic libpcap format.
    Pcap,
    /// pcapng.
    Pcapng,
}

/// One capture interface (pcapng IDB, or the single pcap link).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceInfo {
    /// Link type.
    pub link_type: u16,
    /// `if_name`, when present.
    pub name: Option<String>,
    /// `if_description`, when present.
    pub description: Option<String>,
    /// Timestamp units per second.
    ts_per_sec: u64,
    /// `if_tsoffset` in seconds.
    ts_offset_s: i64,
}

/// One captured packet, pointing into [`CaptureFile::bytes`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacketRecord {
    /// Absolute timestamp, ns since the Unix epoch (0 when the block has none).
    pub ts_ns: u64,
    /// Start of the captured bytes in [`CaptureFile::bytes`].
    pub data_start: usize,
    /// Captured length.
    pub captured_len: u32,
    /// Original length on the wire (without FCS, as reported by the capture).
    pub orig_len: u32,
    /// Interface index.
    pub interface: u32,
}

/// A capture file held in memory with an index of its packets.
#[derive(Debug, Clone)]
pub struct CaptureFile {
    /// Container format.
    pub format: CaptureFormat,
    /// The whole file.
    pub bytes: Vec<u8>,
    /// Packets in file order.
    pub packets: Vec<PacketRecord>,
    /// Interfaces.
    pub interfaces: Vec<InterfaceInfo>,
    /// Section and interface comments (`opt_comment`), in file order.
    pub comments: Vec<String>,
    /// `shb_userappl` of the first section, when present.
    pub user_application: Option<String>,
}

impl CaptureFile {
    /// Parse a capture held in memory.
    pub fn parse(bytes: Vec<u8>) -> Result<Self, CaptureError> {
        if bytes.len() < 4 {
            return Err(CaptureError::UnknownFormat);
        }
        let magic_le = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let magic_be = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        if magic_le == PCAPNG_SHB {
            return parse_pcapng(bytes);
        }
        for (magic, big) in [(magic_le, false), (magic_be, true)] {
            if magic == PCAP_MAGIC_US || magic == PCAP_MAGIC_NS {
                return parse_pcap(bytes, big, magic == PCAP_MAGIC_NS);
            }
        }
        Err(CaptureError::UnknownFormat)
    }

    /// Read and parse a file from disk.
    pub fn read(path: &std::path::Path) -> Result<Self, CaptureError> {
        Self::parse(std::fs::read(path)?)
    }

    /// Captured bytes of packet `p`.
    pub fn data(&self, p: &PacketRecord) -> &[u8] {
        &self.bytes[p.data_start..p.data_start + p.captured_len as usize]
    }

    /// Link type of the interface a packet was captured on.
    pub fn link_type(&self, p: &PacketRecord) -> Option<u16> {
        self.interfaces.get(p.interface as usize).map(|i| i.link_type)
    }
}

#[derive(Clone, Copy)]
struct Endian(bool);

impl Endian {
    fn u16(self, b: &[u8], at: usize) -> u16 {
        let a = [b[at], b[at + 1]];
        if self.0 { u16::from_be_bytes(a) } else { u16::from_le_bytes(a) }
    }
    fn u32(self, b: &[u8], at: usize) -> u32 {
        let a = [b[at], b[at + 1], b[at + 2], b[at + 3]];
        if self.0 { u32::from_be_bytes(a) } else { u32::from_le_bytes(a) }
    }
}

fn malformed(format: &'static str, offset: usize, reason: &'static str) -> CaptureError {
    CaptureError::Malformed { format, offset, reason }
}

fn parse_pcap(bytes: Vec<u8>, big: bool, nanos: bool) -> Result<CaptureFile, CaptureError> {
    let e = Endian(big);
    if bytes.len() < 24 {
        return Err(malformed("pcap", 0, "file header truncated"));
    }
    let link_type = (e.u32(&bytes, 20) & 0xFFFF) as u16;
    let mut packets = Vec::new();
    let mut at = 24;
    while at < bytes.len() {
        if bytes.len() - at < 16 {
            return Err(malformed("pcap", at, "record header truncated"));
        }
        let sec = e.u32(&bytes, at) as u64;
        let frac = e.u32(&bytes, at + 4) as u64;
        let incl = e.u32(&bytes, at + 8);
        let orig = e.u32(&bytes, at + 12);
        let data_start = at + 16;
        if bytes.len() - data_start < incl as usize {
            return Err(malformed("pcap", at, "record data truncated"));
        }
        let ts_ns = sec * 1_000_000_000 + if nanos { frac } else { frac * 1_000 };
        packets.push(PacketRecord { ts_ns, data_start, captured_len: incl, orig_len: orig, interface: 0 });
        at = data_start + incl as usize;
    }
    let iface = InterfaceInfo {
        link_type,
        name: None,
        description: None,
        ts_per_sec: if nanos { 1_000_000_000 } else { 1_000_000 },
        ts_offset_s: 0,
    };
    Ok(CaptureFile {
        format: CaptureFormat::Pcap,
        bytes,
        packets,
        interfaces: vec![iface],
        comments: Vec::new(),
        user_application: None,
    })
}

/// Timestamp in ns since the epoch. Saturates instead of overflowing: a file may carry any
/// 64-bit timestamp with any resolution (`if_tsresol` down to one tick per second).
fn ts_to_ns(raw: u64, iface: &InterfaceInfo) -> u64 {
    let per = iface.ts_per_sec.max(1) as u128;
    let ns = (raw as u128 * 1_000_000_000 / per).min(u64::MAX as u128) as u64;
    ns.saturating_add_signed(iface.ts_offset_s.saturating_mul(1_000_000_000))
}

/// Iterate pcapng options in `body`. Calls `f(code, value)`.
fn options(e: Endian, body: &[u8], mut f: impl FnMut(u16, &[u8])) {
    let mut at = 0;
    while body.len() >= at + 4 {
        let code = e.u16(body, at);
        let len = e.u16(body, at + 2) as usize;
        if code == 0 || body.len() < at + 4 + len {
            break;
        }
        f(code, &body[at + 4..at + 4 + len]);
        at += 4 + len.next_multiple_of(4);
    }
}

fn text(v: &[u8]) -> String {
    String::from_utf8_lossy(v).trim_end_matches('\0').to_owned()
}

fn parse_pcapng(bytes: Vec<u8>) -> Result<CaptureFile, CaptureError> {
    const F: &str = "pcapng";
    let mut packets = Vec::new();
    let mut interfaces: Vec<InterfaceInfo> = Vec::new();
    let mut section_base = 0usize; // interface index offset of the current section
    let mut comments = Vec::new();
    let mut user_application = None;
    let mut e = Endian(false);
    let mut at = 0usize;
    while at < bytes.len() {
        if bytes.len() - at < 12 {
            return Err(malformed(F, at, "block header truncated"));
        }
        let raw_type = u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
        if raw_type == PCAPNG_SHB {
            let bom = u32::from_le_bytes([bytes[at + 8], bytes[at + 9], bytes[at + 10], bytes[at + 11]]);
            e = match bom {
                PCAPNG_BOM => Endian(false),
                b if b.swap_bytes() == PCAPNG_BOM => Endian(true),
                _ => return Err(malformed(F, at, "bad byte-order magic")),
            };
            section_base = interfaces.len();
        }
        let block_type = e.u32(&bytes, at);
        let total = e.u32(&bytes, at + 4) as usize;
        if total < 12 || !total.is_multiple_of(4) || bytes.len() - at < total {
            return Err(malformed(F, at, "bad block length"));
        }
        let body = &bytes[at + 8..at + total - 4];
        match block_type {
            PCAPNG_SHB => {
                if body.len() < 16 {
                    return Err(malformed(F, at, "section header truncated"));
                }
                options(e, &body[16..], |code, v| match code {
                    1 => comments.push(text(v)),
                    4 if user_application.is_none() => user_application = Some(text(v)),
                    _ => {}
                });
            }
            1 => {
                if body.len() < 8 {
                    return Err(malformed(F, at, "interface block truncated"));
                }
                let mut iface = InterfaceInfo {
                    link_type: e.u16(body, 0),
                    name: None,
                    description: None,
                    ts_per_sec: 1_000_000,
                    ts_offset_s: 0,
                };
                options(e, &body[8..], |code, v| match code {
                    1 => comments.push(text(v)),
                    2 => iface.name = Some(text(v)),
                    3 => iface.description = Some(text(v)),
                    9 if !v.is_empty() => {
                        let r = v[0];
                        iface.ts_per_sec = if r & 0x80 == 0 {
                            10u64.saturating_pow((r & 0x7F) as u32)
                        } else {
                            1u64.checked_shl((r & 0x7F) as u32).unwrap_or(u64::MAX)
                        }
                        .max(1);
                    }
                    14 if v.len() >= 8 => {
                        let mut a = [0u8; 8];
                        a.copy_from_slice(&v[..8]);
                        iface.ts_offset_s = if e.0 { i64::from_be_bytes(a) } else { i64::from_le_bytes(a) };
                    }
                    _ => {}
                });
                interfaces.push(iface);
            }
            6 | 2 => {
                // Enhanced Packet Block, or the obsolete Packet Block (u16 iface + u16 drops).
                if body.len() < 20 {
                    return Err(malformed(F, at, "packet block truncated"));
                }
                let iface_id = if block_type == 6 { e.u32(body, 0) as usize } else { e.u16(body, 0) as usize };
                let ts = ((e.u32(body, 4) as u64) << 32) | e.u32(body, 8) as u64;
                let captured_len = e.u32(body, 12);
                let orig_len = e.u32(body, 16);
                if body.len() < 20 + captured_len as usize {
                    return Err(malformed(F, at, "packet data truncated"));
                }
                let interface = section_base + iface_id;
                let iface = interfaces.get(interface).ok_or(malformed(F, at, "unknown interface id"))?;
                packets.push(PacketRecord {
                    ts_ns: ts_to_ns(ts, iface),
                    data_start: at + 8 + 20,
                    captured_len,
                    orig_len,
                    interface: interface as u32,
                });
            }
            3 => {
                // Simple Packet Block: no timestamp, interface 0 of the section.
                if body.len() < 4 {
                    return Err(malformed(F, at, "simple packet block truncated"));
                }
                let orig_len = e.u32(body, 0);
                let captured_len = (orig_len as usize).min(body.len() - 4) as u32;
                packets.push(PacketRecord {
                    ts_ns: 0,
                    data_start: at + 12,
                    captured_len,
                    orig_len,
                    interface: section_base as u32,
                });
            }
            _ => {}
        }
        at += total;
    }
    Ok(CaptureFile { format: CaptureFormat::Pcapng, bytes, packets, interfaces, comments, user_application })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iface(ts_per_sec: u64, ts_offset_s: i64) -> InterfaceInfo {
        InterfaceInfo { link_type: 1, name: None, description: None, ts_per_sec, ts_offset_s }
    }

    #[test]
    fn timestamps_convert_at_any_resolution() {
        assert_eq!(ts_to_ns(1_500_000, &iface(1_000_000, 0)), 1_500_000_000);
        assert_eq!(ts_to_ns(7, &iface(1_000_000_000, 2)), 2_000_000_007);
        assert_eq!(ts_to_ns(3, &iface(1, 0)), 3_000_000_000);
        assert_eq!(ts_to_ns(1 << 10, &iface(1 << 10, -1)), 0);
    }

    #[test]
    fn huge_timestamps_saturate_instead_of_overflowing() {
        assert_eq!(ts_to_ns(u64::MAX, &iface(1, 0)), u64::MAX);
        assert_eq!(ts_to_ns(u64::MAX, &iface(1_000_000, 0)), u64::MAX);
        assert_eq!(ts_to_ns(u64::MAX, &iface(1_000_000, i64::MAX)), u64::MAX);
        assert_eq!(ts_to_ns(u64::MAX, &iface(u64::MAX, 0)), 1_000_000_000);
    }
}
