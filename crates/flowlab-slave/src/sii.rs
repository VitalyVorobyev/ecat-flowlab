//! The slave's EEPROM (SII) image and the CoE objects that describe its process data.
//!
//! The image follows ETG.2010: the fixed words 0x0000–0x003F (identity, standard mailbox,
//! supported protocols) and the categories Strings, General, FMMU, SyncManager, RxPDO and
//! TxPDO. The process data is mapped in 32-bit entries, with 8-bit entries for the rest.

use std::collections::BTreeMap;

use crate::Layout;

/// SyncManager control byte of "mailbox out": mailbox, ECAT writes, PDI interrupt.
pub(crate) const SM_CONTROL_MBX_OUT: u8 = 0x26;
/// "mailbox in": mailbox, ECAT reads, PDI interrupt.
pub(crate) const SM_CONTROL_MBX_IN: u8 = 0x22;
/// Outputs: buffered, ECAT writes, PDI interrupt, watchdog.
pub(crate) const SM_CONTROL_OUTPUTS: u8 = 0x64;
/// Inputs: buffered, ECAT reads, PDI interrupt.
pub(crate) const SM_CONTROL_INPUTS: u8 = 0x20;

/// Mailbox protocol word: EoE (0x02), CoE (0x04), FoE (0x08), VoE (0x20).
pub(crate) const PROTOCOLS: u16 = 0x02 | 0x04 | 0x08 | 0x20;

/// Index of the output data objects (0x7000:n) and input data objects (0x6000:n).
const OUTPUT_OBJECTS: u16 = 0x7000;
const INPUT_OBJECTS: u16 = 0x6000;
const RXPDO: u16 = 0x1600;
const TXPDO: u16 = 0x1A00;

/// What the EEPROM says about the slave.
#[derive(Debug, Clone)]
pub(crate) struct Identity<'a> {
    pub(crate) name: &'a str,
    pub(crate) vendor_id: u32,
    pub(crate) product_code: u32,
    pub(crate) revision: u32,
    pub(crate) serial: u32,
}

/// The bit lengths of a PDO's entries for `bytes` of process data: 32-bit entries, then
/// 8-bit entries for what remains.
fn entries(bytes: u16) -> Vec<u8> {
    let mut v = vec![32u8; usize::from(bytes / 4)];
    v.extend(std::iter::repeat_n(8u8, usize::from(bytes % 4)));
    v
}

/// The most process data bytes per direction (254 entries of 32 bits).
pub(crate) const MAX_PROCESS_DATA: u16 = 254 * 4;

/// A mapping entry as CoE stores it: bit length, subindex, index.
fn mapping(index: u16, subindex: u8, bits: u8) -> u32 {
    u32::from(bits) | u32::from(subindex) << 8 | u32::from(index) << 16
}

/// CRC-8 of the first seven words (polynomial x⁸ + x² + x + 1, initial value 0xFF).
fn crc8(data: &[u8]) -> u8 {
    let mut crc = 0xFFu8;
    for &b in data {
        crc ^= b;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 { (crc << 1) ^ 0x07 } else { crc << 1 };
        }
    }
    crc
}

struct Image(Vec<u8>);

impl Image {
    fn word(&mut self, address: usize, v: u16) {
        self.0[address * 2..address * 2 + 2].copy_from_slice(&v.to_le_bytes());
    }

    fn dword(&mut self, address: usize, v: u32) {
        self.0[address * 2..address * 2 + 4].copy_from_slice(&v.to_le_bytes());
    }

    fn category(&mut self, kind: u16, mut data: Vec<u8>) {
        if data.len() % 2 == 1 {
            data.push(0);
        }
        self.0.extend_from_slice(&kind.to_le_bytes());
        self.0.extend_from_slice(&((data.len() / 2) as u16).to_le_bytes());
        self.0.extend_from_slice(&data);
    }
}

/// A PDO description of the RxPDO/TxPDO categories.
fn pdo(index: u16, sync_manager: u8, name_idx: u8, object: u16, bytes: u16) -> Vec<u8> {
    let e = entries(bytes);
    let mut v = Vec::new();
    v.extend_from_slice(&index.to_le_bytes());
    v.push(e.len() as u8);
    v.push(sync_manager);
    v.push(0); // DC sync
    v.push(name_idx);
    v.extend_from_slice(&0u16.to_le_bytes()); // flags
    for (n, bits) in e.iter().enumerate() {
        v.extend_from_slice(&object.to_le_bytes());
        v.push(n as u8 + 1);
        v.push(0); // name
        v.push(if *bits == 32 { 0x07 } else { 0x05 }); // UNSIGNED32, UNSIGNED8
        v.push(*bits);
        v.extend_from_slice(&0u16.to_le_bytes());
    }
    v
}

/// The EEPROM contents.
pub(crate) fn image(id: &Identity<'_>, l: &Layout) -> Vec<u8> {
    let mut img = Image(vec![0u8; 0x40 * 2]);
    img.dword(0x0008, id.vendor_id);
    img.dword(0x000A, id.product_code);
    img.dword(0x000C, id.revision);
    img.dword(0x000E, id.serial);
    img.word(0x0018, l.mbx_out);
    img.word(0x0019, l.mbx_len);
    img.word(0x001A, l.mbx_in);
    img.word(0x001B, l.mbx_len);
    img.word(0x001C, PROTOCOLS);
    img.word(0x003F, 1); // version
    let crc = crc8(&img.0[..14]);
    img.word(0x0007, u16::from(crc));

    // Strings: 1 name, 2 group, 3 outputs, 4 inputs.
    let strings = [id.name, "FlowLab", "Outputs", "Inputs"];
    let mut s = vec![strings.len() as u8];
    for t in strings {
        let b = &t.as_bytes()[..t.len().min(255)];
        s.push(b.len() as u8);
        s.extend_from_slice(b);
    }
    img.category(10, s);

    // General: group 2, image 0, order 1, name 1; CoE (SDO), FoE and EoE enabled.
    let mut g = vec![0u8; 32];
    g[0] = 2;
    g[2] = 1;
    g[3] = 1;
    g[5] = 0x01;
    g[6] = 0x01;
    g[7] = 0x01;
    g[14] = 2;
    img.category(30, g);

    // FMMU usage: outputs, inputs, SyncManager status (unused 0xFF pads to a word).
    img.category(40, vec![0x01, 0x02, 0x03, 0xFF]);

    // SyncManagers: start, length, control, status, enable, type.
    let mut sm = Vec::new();
    for (start, len, control, kind) in [
        (l.mbx_out, l.mbx_len, SM_CONTROL_MBX_OUT, 1u8),
        (l.mbx_in, l.mbx_len, SM_CONTROL_MBX_IN, 2),
        (l.outputs, l.output_len, SM_CONTROL_OUTPUTS, 3),
        (l.inputs, l.input_len, SM_CONTROL_INPUTS, 4),
    ] {
        sm.extend_from_slice(&start.to_le_bytes());
        sm.extend_from_slice(&len.to_le_bytes());
        sm.extend_from_slice(&[control, 0, 0x01, kind]);
    }
    img.category(41, sm);

    if l.input_len > 0 {
        img.category(50, pdo(TXPDO, 3, 4, INPUT_OBJECTS, l.input_len));
    }
    if l.output_len > 0 {
        img.category(51, pdo(RXPDO, 2, 3, OUTPUT_OBJECTS, l.output_len));
    }
    img.0.extend_from_slice(&0xFFFFu16.to_le_bytes());
    // EEPROM size word: kbit − 1.
    let kbit = img.0.len().div_ceil(128).max(1);
    img.word(0x003E, (kbit - 1) as u16);
    img.0
}

/// The CoE objects a master reads to map the process data, and the identity objects.
pub(crate) fn objects(id: &Identity<'_>, l: &Layout) -> BTreeMap<(u16, u8), Vec<u8>> {
    let mut o = BTreeMap::new();
    o.insert((0x1000, 0), 0u32.to_le_bytes().to_vec());
    o.insert((0x1008, 0), id.name.as_bytes().to_vec());
    o.insert((0x1018, 0), vec![4]);
    for (sub, v) in [id.vendor_id, id.product_code, id.revision, id.serial].into_iter().enumerate() {
        o.insert((0x1018, sub as u8 + 1), v.to_le_bytes().to_vec());
    }
    for (assign, pdo_index, object, bytes) in
        [(0x1C12u16, RXPDO, OUTPUT_OBJECTS, l.output_len), (0x1C13, TXPDO, INPUT_OBJECTS, l.input_len)]
    {
        let used = bytes > 0;
        o.insert((assign, 0), vec![u8::from(used)]);
        if used {
            o.insert((assign, 1), pdo_index.to_le_bytes().to_vec());
            let e = entries(bytes);
            o.insert((pdo_index, 0), vec![e.len() as u8]);
            for (n, bits) in e.iter().enumerate() {
                let m = mapping(object, n as u8 + 1, *bits);
                o.insert((pdo_index, n as u8 + 1), m.to_le_bytes().to_vec());
            }
        }
    }
    // The mailbox SyncManagers' communication types (0x1C00).
    o.insert((0x1C00, 0), vec![4]);
    for (sub, t) in [1u8, 2, 3, 4].into_iter().enumerate() {
        o.insert((0x1C00, sub as u8 + 1), vec![t]);
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_has_the_check_value_of_its_algorithm() {
        // Polynomial 0x07, initial value 0xFF, not reflected, as EtherCrab checks an EEPROM:
        // the `crc` crate with those parameters gives 0xFB for "123456789".
        assert_eq!(crc8(b"123456789"), 0xFB);
    }

    #[test]
    fn entries_cover_the_bytes() {
        assert_eq!(entries(0), Vec::<u8>::new());
        assert_eq!(entries(6), vec![32, 8, 8]);
        let bits: u32 = entries(1000).iter().map(|b| u32::from(*b)).sum();
        assert_eq!(bits, 8000);
    }
}
