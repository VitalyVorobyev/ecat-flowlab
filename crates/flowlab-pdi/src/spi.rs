//! The SPI bus as the LAN9252 logic sees it, and the clock the benchmark times with.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// One part of an SPI transaction: bytes sent (MOSI) or bytes received (MISO).
///
/// The LAN9252 is half duplex on SPI: it listens to the instruction and address, then drives
/// data. A transaction is therefore a list of write and read segments.
#[derive(Debug)]
pub enum Segment<'a> {
    /// Send these bytes.
    Write(&'a [u8]),
    /// Clock in this many bytes (the master sends zeros meanwhile).
    Read(&'a mut [u8]),
}

/// An SPI master that runs a transaction with chip select held for all its segments.
pub trait SpiBus {
    /// Assert chip select, run the segments in order, release chip select.
    fn transaction(&mut self, segments: &mut [Segment<'_>]) -> io::Result<()>;
}

impl<T: SpiBus + ?Sized> SpiBus for Box<T> {
    fn transaction(&mut self, segments: &mut [Segment<'_>]) -> io::Result<()> {
        (**self).transaction(segments)
    }
}

/// Monotonic time in ns from an arbitrary origin.
pub trait Clock {
    /// Nanoseconds since the clock's origin; never decreases.
    fn now_ns(&self) -> u64;
}

/// The host's monotonic clock (`std::time::Instant`), for hardware runs.
#[derive(Debug, Clone, Copy)]
pub struct MonotonicClock {
    origin: Instant,
}

impl MonotonicClock {
    /// A clock whose origin is now.
    pub fn new() -> Self {
        Self { origin: Instant::now() }
    }
}

impl Default for MonotonicClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for MonotonicClock {
    fn now_ns(&self) -> u64 {
        u64::try_from(self.origin.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }
}

/// Virtual time advanced by a simulated bus (see [`crate::SimLan9252`]). Clones share the time.
#[derive(Debug, Clone, Default)]
pub struct SimClock {
    ns: Arc<AtomicU64>,
}

impl SimClock {
    /// Advance the time by `ns`.
    pub fn advance(&self, ns: u64) {
        self.ns.fetch_add(ns, Ordering::Relaxed);
    }
}

impl Clock for SimClock {
    fn now_ns(&self) -> u64 {
        self.ns.load(Ordering::Relaxed)
    }
}
