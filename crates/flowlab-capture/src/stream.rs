//! Splitting a pcap or pcapng byte stream into whole blocks as it arrives, for live capture.

use std::collections::VecDeque;

use crate::file::CaptureError;

const PCAP_MAGIC_US: u32 = 0xA1B2_C3D4;
const PCAP_MAGIC_NS: u32 = 0xA1B2_3C4D;
const PCAPNG_SHB: u32 = 0x0A0D_0D0A;
const PCAPNG_BOM: u32 = 0x1A2B_3C4D;
/// Largest block accepted (a packet of 16 MiB is not EtherCAT and not a sane capture).
const MAX_BLOCK_BYTES: usize = 16 << 20;

/// What a block of a capture stream holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    /// Starts a new section: the pcap file header or a pcapng Section Header Block. Any
    /// interfaces declared before it no longer apply.
    Section,
    /// Declares an interface (pcapng Interface Description Block). Packet blocks after it
    /// refer to it, so it belongs with the section header in every excerpt of the stream.
    Interface,
    /// One packet (pcap record, or pcapng Enhanced, Simple or obsolete Packet Block).
    Packet,
    /// Anything else (statistics, name resolution, custom blocks).
    Other,
}

/// One whole block of a capture stream, as it appeared in the stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    /// What it holds.
    pub kind: BlockKind,
    /// Its bytes, header included.
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
enum Format {
    Unknown,
    Pcap { big: bool },
    Pcapng { big: bool },
}

/// Cuts a pcap or pcapng byte stream, fed in arbitrary pieces, into whole blocks.
///
/// Concatenating the section block, the interface blocks and any run of packet blocks gives a
/// file that [`CaptureFile::parse`](crate::CaptureFile::parse) reads, so a live capture can
/// be analysed in excerpts with the same code as a file.
///
/// ```
/// use flowlab_capture::{BlockKind, StreamSplitter};
///
/// // A pcap file header and one empty record, fed one byte at a time.
/// let mut file = vec![0xD4, 0xC3, 0xB2, 0xA1, 2, 0, 4, 0];
/// file.extend([0; 8]);
/// file.extend([0xFF, 0xFF, 0, 0, 1, 0, 0, 0]);
/// file.extend([0; 16]);
/// let mut s = StreamSplitter::default();
/// let mut kinds = Vec::new();
/// for b in &file {
///     s.push(std::slice::from_ref(b));
///     while let Some(block) = s.next_block().unwrap() {
///         kinds.push(block.kind);
///     }
/// }
/// assert_eq!(kinds, [BlockKind::Section, BlockKind::Packet]);
/// ```
#[derive(Debug)]
pub struct StreamSplitter {
    buf: VecDeque<u8>,
    format: Format,
    /// Bytes consumed so far, for error offsets.
    offset: usize,
}

impl Default for StreamSplitter {
    fn default() -> Self {
        Self { buf: VecDeque::new(), format: Format::Unknown, offset: 0 }
    }
}

impl StreamSplitter {
    /// Append bytes read from the stream.
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend(bytes);
    }

    /// Bytes received but not yet returned as a block.
    pub fn pending(&self) -> usize {
        self.buf.len()
    }

    fn peek_u32(&self, at: usize, big: bool) -> u32 {
        let a = [self.buf[at], self.buf[at + 1], self.buf[at + 2], self.buf[at + 3]];
        if big { u32::from_be_bytes(a) } else { u32::from_le_bytes(a) }
    }

    fn take(&mut self, kind: BlockKind, len: usize) -> Block {
        self.offset += len;
        Block { kind, bytes: self.buf.drain(..len).collect() }
    }

    fn malformed(&self, format: &'static str, reason: &'static str) -> CaptureError {
        CaptureError::Malformed { format, offset: self.offset, reason }
    }

    /// The next whole block, `None` until enough bytes have arrived, or an error when the
    /// stream is not a capture. After an error the stream cannot be resumed.
    pub fn next_block(&mut self) -> Result<Option<Block>, CaptureError> {
        if self.buf.len() < 4 {
            return Ok(None);
        }
        let magic_le = self.peek_u32(0, false);
        if let Format::Unknown = self.format {
            let magic_be = self.peek_u32(0, true);
            self.format = if magic_le == PCAPNG_SHB {
                if self.buf.len() < 12 {
                    return Ok(None);
                }
                Format::Pcapng { big: self.bom_big()? }
            } else if magic_le == PCAP_MAGIC_US || magic_le == PCAP_MAGIC_NS {
                Format::Pcap { big: false }
            } else if magic_be == PCAP_MAGIC_US || magic_be == PCAP_MAGIC_NS {
                Format::Pcap { big: true }
            } else {
                return Err(CaptureError::UnknownFormat);
            };
        }
        match self.format {
            Format::Unknown => Ok(None),
            Format::Pcap { big } => {
                // The file header comes first, exactly once.
                if self.offset == 0 {
                    return Ok((self.buf.len() >= 24).then(|| self.take(BlockKind::Section, 24)));
                }
                if self.buf.len() < 16 {
                    return Ok(None);
                }
                let incl = self.peek_u32(8, big) as usize;
                if incl > MAX_BLOCK_BYTES {
                    return Err(self.malformed("pcap", "record longer than 16 MiB"));
                }
                Ok((self.buf.len() >= 16 + incl).then(|| self.take(BlockKind::Packet, 16 + incl)))
            }
            Format::Pcapng { big } => {
                if self.buf.len() < 12 {
                    return Ok(None);
                }
                // A new section may switch the byte order.
                let big = if magic_le == PCAPNG_SHB {
                    let b = self.bom_big()?;
                    self.format = Format::Pcapng { big: b };
                    b
                } else {
                    big
                };
                let block_type = self.peek_u32(0, big);
                let total = self.peek_u32(4, big) as usize;
                if total < 12 || !total.is_multiple_of(4) || total > MAX_BLOCK_BYTES {
                    return Err(self.malformed("pcapng", "bad block length"));
                }
                let kind = match block_type {
                    PCAPNG_SHB => BlockKind::Section,
                    1 => BlockKind::Interface,
                    2 | 3 | 6 => BlockKind::Packet,
                    _ => BlockKind::Other,
                };
                Ok((self.buf.len() >= total).then(|| self.take(kind, total)))
            }
        }
    }

    fn bom_big(&self) -> Result<bool, CaptureError> {
        match self.peek_u32(8, false) {
            PCAPNG_BOM => Ok(false),
            b if b.swap_bytes() == PCAPNG_BOM => Ok(true),
            _ => Err(self.malformed("pcapng", "bad byte-order magic")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CaptureFile, PcapNgWriter};

    fn pcapng(n: usize) -> Vec<u8> {
        let mut w = PcapNgWriter::new(Vec::new(), "test", "comment", "eth0").unwrap();
        for i in 0..n {
            w.packet(i as u64 * 1000, &vec![i as u8; 60 + i % 7]).unwrap();
        }
        w.finish().unwrap()
    }

    fn split(bytes: &[u8], piece: usize) -> Vec<Block> {
        let mut s = StreamSplitter::default();
        let mut blocks = Vec::new();
        for chunk in bytes.chunks(piece) {
            s.push(chunk);
            while let Some(b) = s.next_block().unwrap() {
                blocks.push(b);
            }
        }
        assert_eq!(s.pending(), 0);
        blocks
    }

    #[test]
    fn blocks_reassemble_the_stream_whatever_the_piece_size() {
        let file = pcapng(25);
        for piece in [1, 3, 7, 64, 4096] {
            let blocks = split(&file, piece);
            assert_eq!(blocks.iter().filter(|b| b.kind == BlockKind::Packet).count(), 25);
            assert_eq!(blocks[0].kind, BlockKind::Section);
            let joined: Vec<u8> = blocks.iter().flat_map(|b| b.bytes.clone()).collect();
            assert_eq!(joined, file);
        }
    }

    #[test]
    fn header_blocks_plus_a_run_of_packets_parse_as_a_file() {
        let blocks = split(&pcapng(10), 5);
        let mut excerpt: Vec<u8> =
            blocks.iter().filter(|b| b.kind != BlockKind::Packet).flat_map(|b| b.bytes.clone()).collect();
        let packets: Vec<_> = blocks.iter().filter(|b| b.kind == BlockKind::Packet).collect();
        for p in &packets[4..7] {
            excerpt.extend(&p.bytes);
        }
        let f = CaptureFile::parse(excerpt).unwrap();
        assert_eq!(f.packets.len(), 3);
        assert_eq!(f.packets[0].ts_ns, 4000);
    }

    #[test]
    fn classic_pcap_is_split_into_header_and_records() {
        let mut file = vec![0x4D, 0x3C, 0xB2, 0xA1, 2, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        file.extend([0xFF, 0xFF, 0, 0, 1, 0, 0, 0]);
        for i in 0..3u32 {
            file.extend(1u32.to_le_bytes());
            file.extend(i.to_le_bytes());
            file.extend(4u32.to_le_bytes());
            file.extend(4u32.to_le_bytes());
            file.extend([i as u8; 4]);
        }
        let blocks = split(&file, 5);
        assert_eq!(blocks.len(), 4);
        assert_eq!(blocks[0].kind, BlockKind::Section);
        let f = CaptureFile::parse(file).unwrap();
        assert_eq!(f.packets.len(), 3);
    }

    #[test]
    fn garbage_is_rejected() {
        let mut s = StreamSplitter::default();
        s.push(b"hello world");
        assert!(matches!(s.next_block(), Err(CaptureError::UnknownFormat)));
    }
}
