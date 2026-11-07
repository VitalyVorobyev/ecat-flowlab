use crate::command::Command;
use crate::datagram::{DATAGRAM_HEADER_LEN, ECAT_HEADER_LEN, EcatHeader, EcatPduType, WKC_LEN};
use crate::error::WireError;
use crate::ethernet::{ETHERTYPE_ETHERCAT, MIN_ETH_PAYLOAD, MacAddr};

/// One datagram to encode.
#[derive(Debug, Clone, Copy)]
pub struct DatagramSpec<'a> {
    /// Command.
    pub command: Command,
    /// Index.
    pub index: u8,
    /// Raw 32-bit address field (`adp | ado << 16`, or a logical address).
    pub address: u32,
    /// Data bytes (at most 2047, less in practice because of the frame size).
    pub data: &'a [u8],
    /// Interrupt field.
    pub irq: u16,
    /// Working counter.
    pub wkc: u16,
    /// Circulating flag.
    pub circulating: bool,
}

impl<'a> DatagramSpec<'a> {
    /// A datagram with zero IRQ, zero WKC and no circulating flag.
    pub const fn new(command: Command, index: u8, address: u32, data: &'a [u8]) -> Self {
        Self { command, index, address, data, irq: 0, wkc: 0, circulating: false }
    }

    /// Same datagram with `wkc` set.
    pub const fn with_wkc(mut self, wkc: u16) -> Self {
        self.wkc = wkc;
        self
    }
}

/// Builds a raw-Ethernet EtherCAT frame (without FCS).
///
/// ```
/// use ecat_wire::{Command, DatagramSpec, FrameBuilder, MacAddr, parse_frame};
/// let mut b = FrameBuilder::new(MacAddr([0xff; 6]), MacAddr([0, 1, 5, 0, 0, 1]));
/// b.push(DatagramSpec::new(Command::Brd, 1, 0x0130_0000, &[0, 0])).unwrap();
/// let bytes = b.finish();
/// let frame = parse_frame(&bytes).unwrap().unwrap();
/// assert_eq!(frame.pdu.datagrams().count(), 1);
/// ```
#[derive(Debug, Clone)]
pub struct FrameBuilder {
    buf: Vec<u8>,
    last_len_word_at: Option<usize>,
}

/// Largest EtherCAT body that fits a standard 1500-byte Ethernet payload.
const MAX_BODY: usize = 1500 - ECAT_HEADER_LEN;

impl FrameBuilder {
    /// Start a frame with the given Ethernet addresses.
    pub fn new(dst: MacAddr, src: MacAddr) -> Self {
        let mut buf = Vec::with_capacity(128);
        buf.extend_from_slice(&dst.0);
        buf.extend_from_slice(&src.0);
        buf.extend_from_slice(&ETHERTYPE_ETHERCAT.to_be_bytes());
        buf.extend_from_slice(&[0, 0]); // EtherCAT header, patched in `finish`
        Self { buf, last_len_word_at: None }
    }

    fn body_len(&self) -> usize {
        self.buf.len() - 14 - ECAT_HEADER_LEN
    }

    /// Bytes still available for datagrams (headers + data + WKC) in a standard frame.
    pub fn remaining(&self) -> usize {
        MAX_BODY - self.body_len()
    }

    /// Append a datagram. Fails if it would overflow a standard Ethernet frame.
    pub fn push(&mut self, d: DatagramSpec<'_>) -> Result<(), WireError> {
        let need = DATAGRAM_HEADER_LEN + d.data.len() + WKC_LEN;
        if need > self.remaining() {
            return Err(WireError::FieldOverflow { field: "datagram length", value: d.data.len() });
        }
        if let Some(at) = self.last_len_word_at {
            self.buf[at + 1] |= 0x80; // set `more` on the previous datagram
        }
        let len_word = (d.data.len() as u16 & 0x07FF) | ((d.circulating as u16) << 14);
        self.buf.push(d.command.code());
        self.buf.push(d.index);
        self.buf.extend_from_slice(&d.address.to_le_bytes());
        self.last_len_word_at = Some(self.buf.len());
        self.buf.extend_from_slice(&len_word.to_le_bytes());
        self.buf.extend_from_slice(&d.irq.to_le_bytes());
        self.buf.extend_from_slice(d.data);
        self.buf.extend_from_slice(&d.wkc.to_le_bytes());
        Ok(())
    }

    /// Patch the EtherCAT header, pad to the Ethernet minimum and return the frame bytes.
    pub fn finish(mut self) -> Vec<u8> {
        let header = EcatHeader { length: self.body_len() as u16, reserved: false, pdu_type: EcatPduType::Datagrams };
        self.buf[14..16].copy_from_slice(&header.to_word().to_le_bytes());
        let min = 14 + MIN_ETH_PAYLOAD;
        if self.buf.len() < min {
            self.buf.resize(min, 0);
        }
        self.buf
    }
}
