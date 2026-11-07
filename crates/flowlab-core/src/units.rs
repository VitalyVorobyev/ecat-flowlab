//! Unit constants and conversions. Decimal and binary byte units are never mixed.

/// One decimal megabyte, 10^6 bytes.
pub const MB: u64 = 1_000_000;
/// One binary mebibyte, 2^20 bytes.
pub const MIB: u64 = 1 << 20;
/// Nanoseconds per second.
pub const NS_PER_S: u64 = 1_000_000_000;
/// Nanoseconds per microsecond.
pub const NS_PER_US: u64 = 1_000;
/// Default EtherCAT physical layer rate, 100BASE-TX.
pub const FAST_ETHERNET_BPS: u64 = 100_000_000;

/// Time to serialize `bytes` at `bps`, in nanoseconds (rounded up).
pub const fn serialization_ns(bytes: u64, bps: u64) -> u64 {
    // u128: 10^9 ns × 8 bits overflows u64 above ~2.3 GB.
    ((bytes as u128 * 8 * NS_PER_S as u128).div_ceil(bps as u128)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialization_time() {
        // 1538 wire bytes (max frame + overhead) at 100 Mbit/s = 123.04 us.
        assert_eq!(serialization_ns(1538, FAST_ETHERNET_BPS), 123_040);
        // 10 MB cannot cross 100 Mbit/s in under 0.8 s.
        assert_eq!(serialization_ns(10 * MB, FAST_ETHERNET_BPS), 800_000_000);
    }
}
