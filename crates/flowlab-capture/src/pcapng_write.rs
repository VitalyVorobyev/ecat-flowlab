//! pcapng writing, with the provenance marker in the section comment.

use std::io::Write;

use crate::file::LINKTYPE_ETHERNET;

/// Writes a little-endian pcapng stream with one Ethernet interface and ns timestamps.
#[derive(Debug)]
pub struct PcapNgWriter<W: Write> {
    out: W,
}

fn push_option(buf: &mut Vec<u8>, code: u16, value: &[u8]) {
    buf.extend_from_slice(&code.to_le_bytes());
    buf.extend_from_slice(&(value.len() as u16).to_le_bytes());
    buf.extend_from_slice(value);
    buf.resize(buf.len().next_multiple_of(4), 0);
}

fn end_options(buf: &mut Vec<u8>) {
    buf.extend_from_slice(&[0, 0, 0, 0]);
}

impl<W: Write> PcapNgWriter<W> {
    /// Write the section header (with `comment` and `user_application`) and one Ethernet
    /// interface named `interface`, with `if_tsresol = 9` (nanoseconds).
    pub fn new(mut out: W, user_application: &str, comment: &str, interface: &str) -> std::io::Result<Self> {
        let mut shb = Vec::new();
        shb.extend_from_slice(&0x1A2B_3C4Du32.to_le_bytes());
        shb.extend_from_slice(&1u16.to_le_bytes());
        shb.extend_from_slice(&0u16.to_le_bytes());
        shb.extend_from_slice(&(-1i64).to_le_bytes());
        push_option(&mut shb, 1, comment.as_bytes());
        push_option(&mut shb, 4, user_application.as_bytes());
        end_options(&mut shb);
        write_block(&mut out, 0x0A0D_0D0A, &shb)?;

        let mut idb = Vec::new();
        idb.extend_from_slice(&LINKTYPE_ETHERNET.to_le_bytes());
        idb.extend_from_slice(&0u16.to_le_bytes());
        idb.extend_from_slice(&0u32.to_le_bytes()); // snaplen: unlimited
        push_option(&mut idb, 2, interface.as_bytes());
        push_option(&mut idb, 9, &[9]);
        end_options(&mut idb);
        write_block(&mut out, 1, &idb)?;
        Ok(Self { out })
    }

    /// Write one Enhanced Packet Block.
    pub fn packet(&mut self, ts_ns: u64, data: &[u8]) -> std::io::Result<()> {
        let mut epb = Vec::with_capacity(20 + data.len() + 3);
        epb.extend_from_slice(&0u32.to_le_bytes());
        epb.extend_from_slice(&((ts_ns >> 32) as u32).to_le_bytes());
        epb.extend_from_slice(&(ts_ns as u32).to_le_bytes());
        epb.extend_from_slice(&(data.len() as u32).to_le_bytes());
        epb.extend_from_slice(&(data.len() as u32).to_le_bytes());
        epb.extend_from_slice(data);
        epb.resize(epb.len().next_multiple_of(4), 0);
        write_block(&mut self.out, 6, &epb)
    }

    /// Flush and return the writer.
    pub fn finish(mut self) -> std::io::Result<W> {
        self.out.flush()?;
        Ok(self.out)
    }
}

fn write_block(out: &mut impl Write, block_type: u32, body: &[u8]) -> std::io::Result<()> {
    let total = (12 + body.len()) as u32;
    out.write_all(&block_type.to_le_bytes())?;
    out.write_all(&total.to_le_bytes())?;
    out.write_all(body)?;
    out.write_all(&total.to_le_bytes())
}
