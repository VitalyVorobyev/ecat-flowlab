//! The trace index: every frame decoded once into compact per-frame and per-datagram records,
//! with TX/RX directions inferred.

use std::collections::HashMap;

use ecat_wire::{Command, DatagramAddress, Encapsulation, MacAddr, parse_frame};
use flowlab_core::{InputRef, Provenance};
use serde::Serialize;

use crate::file::{CaptureFile, LINKTYPE_ETHERNET};

/// What a captured frame turned out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum FrameKind {
    /// A well-formed EtherCAT frame.
    Ethercat,
    /// Not EtherCAT (other EtherType, or a link type FlowLab does not decode).
    Other,
    /// Claims to be EtherCAT but does not parse.
    Malformed,
}

/// Direction of an EtherCAT frame relative to the master.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Direction {
    /// Sent by the master.
    Tx,
    /// Returned to the master after passing the slaves.
    Rx,
    /// Cannot be told from the capture.
    Unknown,
}

/// How frame directions were inferred.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DirectionMode {
    /// Source MACs appear both with and without the locally-administered bit, which ESCs set
    /// on returning frames: without the bit is TX, with it is RX.
    MacLocalBit,
    /// No MAC split, but some frames carry only zero working counters and others do not.
    /// Masters send WKC 0, so all-zero frames are taken as TX and the rest as RX. A returning
    /// frame that no slave answered is then misread as TX.
    WkcHeuristic,
    /// Neither rule applies (for example a capture of one direction only). Directions are
    /// unknown.
    Unknown,
}

/// Index entry for one captured frame. Frame number `n` (1-based, as in Wireshark) is
/// `trace.frames[n - 1]`.
#[derive(Debug, Clone, Copy)]
pub struct FrameRecord {
    /// Index into `file.packets`.
    pub packet: u32,
    /// Time since the earliest packet in the capture, ns.
    pub t_ns: u64,
    /// Captured length in bytes (no FCS).
    pub captured_len: u32,
    /// Original length in bytes.
    pub orig_len: u32,
    /// Classification.
    pub kind: FrameKind,
    /// Direction.
    pub direction: Direction,
    /// Encapsulation, for EtherCAT frames.
    pub encapsulation: Option<Encapsulation>,
    /// Source MAC, for Ethernet frames.
    pub src: MacAddr,
    /// Index of the first datagram in `trace.datagrams`.
    pub first_datagram: u32,
    /// Number of datagrams.
    pub datagram_count: u16,
}

/// Index entry for one datagram.
#[derive(Debug, Clone, Copy)]
pub struct DatagramRecord {
    /// Index of the owning frame in `trace.frames`.
    pub frame: u32,
    /// Command.
    pub command: Command,
    /// Datagram index.
    pub index: u8,
    /// Raw address field.
    pub address_raw: u32,
    /// Data length.
    pub len: u16,
    /// Working counter.
    pub wkc: u16,
    /// IRQ field.
    pub irq: u16,
    /// Circulating flag.
    pub circulating: bool,
    /// Offset of the datagram *data* within the frame bytes.
    pub data_offset: u32,
}

impl DatagramRecord {
    /// Interpreted address.
    pub fn address(&self) -> DatagramAddress {
        DatagramAddress::new(self.command, self.address_raw)
    }
}

/// A capture decoded into a frame and datagram index.
#[derive(Debug)]
pub struct Trace {
    /// The capture file.
    pub file: CaptureFile,
    /// Identification of the input.
    pub input: InputRef,
    /// Where the capture came from (declared by a FlowLab marker, else `Measured`).
    pub provenance: Provenance,
    /// Absolute time of the earliest packet, ns since the Unix epoch.
    pub start_ts_ns: u64,
    /// All frames, in file order.
    pub frames: Vec<FrameRecord>,
    /// All datagrams of EtherCAT frames, grouped by frame.
    pub datagrams: Vec<DatagramRecord>,
    /// How directions were inferred.
    pub direction_mode: DirectionMode,
    /// Parse errors of malformed frames, by frame index.
    pub errors: Vec<(u32, String)>,
}

impl Trace {
    /// Read the capture at `path` and decode it, named after the file.
    pub fn read(path: &std::path::Path) -> Result<Self, crate::CaptureError> {
        let file = CaptureFile::read(path)?;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        Ok(Self::build(file, &name))
    }

    /// Decode `file`. `name` is the file name shown to users.
    pub fn build(file: CaptureFile, name: &str) -> Self {
        let input = InputRef::of(name, &file.bytes);
        let provenance = file.comments.iter().find_map(|c| Provenance::from_comment(c)).unwrap_or(Provenance::Measured);
        let start_ts_ns = file.packets.iter().map(|p| p.ts_ns).min().unwrap_or(0);
        let mut frames = Vec::with_capacity(file.packets.len());
        let mut datagrams = Vec::with_capacity(file.packets.len() * 2);
        let mut errors = Vec::new();
        for (i, p) in file.packets.iter().enumerate() {
            let mut rec = FrameRecord {
                packet: i as u32,
                t_ns: p.ts_ns.saturating_sub(start_ts_ns),
                captured_len: p.captured_len,
                orig_len: p.orig_len,
                kind: FrameKind::Other,
                direction: Direction::Unknown,
                encapsulation: None,
                src: MacAddr::default(),
                first_datagram: datagrams.len() as u32,
                datagram_count: 0,
            };
            if file.link_type(p) == Some(LINKTYPE_ETHERNET) {
                let data = file.data(p);
                match parse_frame(data) {
                    Ok(Some(f)) => {
                        rec.src = f.ethernet.src;
                        rec.encapsulation = Some(f.encapsulation);
                        rec.kind = FrameKind::Ethercat;
                        for dg in f.pdu.datagrams() {
                            match dg {
                                Ok(d) => datagrams.push(DatagramRecord {
                                    frame: i as u32,
                                    command: d.command,
                                    index: d.index,
                                    address_raw: d.address.raw(),
                                    len: d.data.len() as u16,
                                    wkc: d.wkc,
                                    irq: d.irq,
                                    circulating: d.circulating,
                                    data_offset: (f.pdu_offset + 2 + d.offset + 10) as u32,
                                }),
                                Err(e) => {
                                    rec.kind = FrameKind::Malformed;
                                    errors.push((i as u32, e.to_string()));
                                    break;
                                }
                            }
                        }
                        rec.datagram_count = (datagrams.len() as u32 - rec.first_datagram) as u16;
                    }
                    Ok(None) => {}
                    Err(e) => {
                        // Too short for Ethernet, or EtherType 0x88A4 with a bad header.
                        if data.len() >= 14 {
                            rec.kind = FrameKind::Malformed;
                            errors.push((i as u32, e.to_string()));
                        }
                    }
                }
            }
            frames.push(rec);
        }
        let direction_mode = infer_directions(&mut frames, &datagrams);
        Self { file, input, provenance, start_ts_ns, frames, datagrams, direction_mode, errors }
    }

    /// Datagrams of a frame.
    pub fn datagrams_of(&self, f: &FrameRecord) -> &[DatagramRecord] {
        let s = f.first_datagram as usize;
        &self.datagrams[s..s + f.datagram_count as usize]
    }

    /// Captured bytes of a frame.
    pub fn frame_bytes(&self, f: &FrameRecord) -> &[u8] {
        self.file.data(&self.file.packets[f.packet as usize])
    }

    /// Data bytes of a datagram.
    pub fn datagram_data(&self, d: &DatagramRecord) -> &[u8] {
        let bytes = self.frame_bytes(&self.frames[d.frame as usize]);
        let s = d.data_offset as usize;
        &bytes[s..s + d.len as usize]
    }

    /// Time span from the first to the last packet, ns.
    pub fn duration_ns(&self) -> u64 {
        self.frames.iter().map(|f| f.t_ns).max().unwrap_or(0)
    }
}

fn infer_directions(frames: &mut [FrameRecord], datagrams: &[DatagramRecord]) -> DirectionMode {
    let mut seen: HashMap<MacAddr, (u64, u64)> = HashMap::new();
    for f in frames.iter().filter(|f| f.kind == FrameKind::Ethercat) {
        let e = seen.entry(f.src.without_local_bit()).or_default();
        if f.src.locally_administered() { e.1 += 1 } else { e.0 += 1 }
    }
    let split: Vec<MacAddr> = seen.iter().filter(|(_, (a, b))| *a > 0 && *b > 0).map(|(m, _)| *m).collect();
    if split.is_empty() {
        return infer_by_wkc(frames, datagrams);
    }
    for f in frames.iter_mut().filter(|f| f.kind == FrameKind::Ethercat) {
        if split.contains(&f.src.without_local_bit()) {
            f.direction = if f.src.locally_administered() { Direction::Rx } else { Direction::Tx };
        }
    }
    DirectionMode::MacLocalBit
}

fn infer_by_wkc(frames: &mut [FrameRecord], datagrams: &[DatagramRecord]) -> DirectionMode {
    let all_zero = |f: &FrameRecord| {
        let s = f.first_datagram as usize;
        datagrams[s..s + f.datagram_count as usize].iter().all(|d| d.wkc == 0)
    };
    let ecat = || frames.iter().filter(|f| f.kind == FrameKind::Ethercat);
    let zero = ecat().filter(|f| all_zero(f)).count();
    if zero == 0 || zero == ecat().count() {
        return DirectionMode::Unknown;
    }
    for f in frames.iter_mut().filter(|f| f.kind == FrameKind::Ethercat) {
        f.direction = if all_zero(f) { Direction::Tx } else { Direction::Rx };
    }
    DirectionMode::WkcHeuristic
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CaptureFormat, PcapNgWriter};
    use ecat_wire::{DatagramSpec, FrameBuilder};

    const MASTER: MacAddr = MacAddr([0x00, 0x01, 0x05, 0x10, 0x20, 0x30]);

    fn frame(src: MacAddr, wkc: u16) -> Vec<u8> {
        let mut b = FrameBuilder::new(MacAddr([0xff; 6]), src);
        b.push(DatagramSpec::new(Command::Lrw, 0x80, 0, &[1, 2, 3, 4]).with_wkc(wkc)).unwrap();
        b.push(DatagramSpec::new(Command::Brd, 0x81, 0x0130_0000, &[8, 0]).with_wkc(wkc / 3)).unwrap();
        b.finish()
    }

    fn sample() -> Vec<u8> {
        let mut w = PcapNgWriter::new(Vec::new(), "test", "flowlab:provenance=synthetic", "sim0").unwrap();
        let base = 1_700_000_000_000_000_000u64;
        for k in 0..3u64 {
            w.packet(base + k * 1_000_000, &frame(MASTER, 0)).unwrap();
            w.packet(base + k * 1_000_000 + 20_000, &frame(MASTER.with_local_bit(), 6)).unwrap();
        }
        w.packet(base + 5_000_000, &[0u8; 60]).unwrap(); // not EtherCAT (EtherType 0)
        w.finish().unwrap()
    }

    #[test]
    fn pcapng_round_trip_and_index() {
        let file = CaptureFile::parse(sample()).unwrap();
        assert_eq!(file.format, CaptureFormat::Pcapng);
        assert_eq!(file.packets.len(), 7);
        assert_eq!(file.packets[1].ts_ns - file.packets[0].ts_ns, 20_000);
        let t = Trace::build(file, "sample.pcapng");
        assert_eq!(t.provenance, Provenance::Synthetic);
        assert_eq!(t.direction_mode, DirectionMode::MacLocalBit);
        assert_eq!(t.frames[0].direction, Direction::Tx);
        assert_eq!(t.frames[1].direction, Direction::Rx);
        assert_eq!(t.frames[6].kind, FrameKind::Other);
        assert_eq!(t.datagrams.len(), 12);
        let dgs = t.datagrams_of(&t.frames[1]);
        assert_eq!(dgs[0].wkc, 6);
        assert_eq!(t.datagram_data(&dgs[0]), &[1, 2, 3, 4]);
        assert_eq!(t.datagram_data(&dgs[1]), &[8, 0]);
        assert_eq!(t.duration_ns(), 5_000_000);
        assert_eq!(t.input.sha256.len(), 64);
    }

    #[test]
    fn classic_pcap_microseconds() {
        let f = frame(MASTER, 0);
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0xA1B2_C3D4u32.to_le_bytes());
        bytes.extend_from_slice(&[2, 0, 4, 0]);
        bytes.extend_from_slice(&[0; 8]);
        bytes.extend_from_slice(&65535u32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        for us in [10u32, 1010] {
            bytes.extend_from_slice(&100u32.to_le_bytes());
            bytes.extend_from_slice(&us.to_le_bytes());
            bytes.extend_from_slice(&(f.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&(f.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&f);
        }
        let file = CaptureFile::parse(bytes).unwrap();
        assert_eq!(file.format, CaptureFormat::Pcap);
        assert_eq!(file.packets[1].ts_ns - file.packets[0].ts_ns, 1_000_000);
        let t = Trace::build(file, "x.pcap");
        assert_eq!(t.provenance, Provenance::Measured);
        assert_eq!(t.direction_mode, DirectionMode::Unknown);
        assert_eq!(t.datagrams.len(), 4);
    }

    #[test]
    fn wkc_heuristic_without_mac_split() {
        let mut w = PcapNgWriter::new(Vec::new(), "test", "", "x").unwrap();
        w.packet(0, &frame(MASTER, 0)).unwrap();
        w.packet(10_000, &frame(MASTER, 6)).unwrap();
        let t = Trace::build(CaptureFile::parse(w.finish().unwrap()).unwrap(), "x");
        assert_eq!(t.direction_mode, DirectionMode::WkcHeuristic);
        assert_eq!(t.frames[0].direction, Direction::Tx);
        assert_eq!(t.frames[1].direction, Direction::Rx);
    }

    #[test]
    fn rejects_garbage() {
        assert!(CaptureFile::parse(vec![1, 2, 3, 4, 5]).is_err());
        let mut s = sample();
        s.truncate(s.len() - 10);
        assert!(CaptureFile::parse(s).is_err());
    }
}
