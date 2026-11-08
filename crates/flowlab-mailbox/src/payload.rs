//! The generated payload: deterministic bytes from a seed, the same in the simulator and in a
//! lab transfer set up like it.

use sha2::{Digest, Sha256};

/// Fill `out` with the payload bytes at `offset`.
pub fn payload_bytes(seed: u64, offset: u64, out: &mut [u8]) {
    for (i, b) in out.iter_mut().enumerate() {
        let pos = offset + i as u64;
        let mut z = seed ^ (pos >> 3).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        *b = (z >> ((pos & 7) * 8)) as u8;
    }
}

/// The whole payload of `len` bytes.
pub fn payload(seed: u64, len: usize) -> Vec<u8> {
    let mut v = vec![0u8; len];
    payload_bytes(seed, 0, &mut v);
    v
}

/// SHA-256 of the whole payload, hex.
pub fn payload_sha256(seed: u64, len: u64) -> String {
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 65536];
    let mut at = 0;
    while at < len {
        let n = (len - at).min(buf.len() as u64) as usize;
        payload_bytes(seed, at, &mut buf[..n]);
        h.update(&buf[..n]);
        at += n as u64;
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Seed of the payload of transfer `index` in a spec with seed `spec_seed`: the transfer's
/// bytes are [`payload_bytes`]`(transfer_seed(..), offset, ..)`, so another tool can send the
/// very payload a simulation moved and compare SHA-256s.
pub fn transfer_seed(spec_seed: u64, index: usize) -> u64 {
    spec_seed ^ (index as u64 + 1).wrapping_mul(0xA24B_AED4_963E_E407)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pieces_and_the_whole_agree() {
        let seed = transfer_seed(1, 2);
        let whole = payload(seed, 1000);
        let mut piece = [0u8; 100];
        payload_bytes(seed, 450, &mut piece);
        assert_eq!(&whole[450..550], &piece);
        let h: String = Sha256::digest(&whole).iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(payload_sha256(seed, 1000), h);
        assert_ne!(transfer_seed(1, 0), transfer_seed(1, 1));
    }
}
