//! Timed ESC accesses over the SPI process data interface (PDI) of a Microchip LAN9252: the
//! samples `flowlab pdi-fit` turns into the simulator's PDI model.
//!
//! The simulator charges every ESC access by the slave application a fixed cost plus its bytes
//! at an effective rate. This crate measures exactly that on the target: it performs complete
//! ESC reads and writes the way a LAN9252 slave stack's hardware layer does ([`Lan9252`]: the
//! CSR interface for registers, the process RAM FIFOs for memory), times each one with a
//! monotonic clock ([`run`]) and writes them as a samples file ([`to_csv`], the format of
//! `flowlab_core::pdi_samples`).
//!
//! * [`SpiBus`] is one SPI transaction with chip select held; [`open_spidev`] gives the Linux
//!   spidev backend (other systems get an error).
//! * [`SimLan9252`] implements the same trait with the LAN9252's registers and FIFOs on a
//!   virtual clock, for tests and dry runs. Its samples are marked `simulated`.
//! * Safety: [`run`] checks the link (`BYTE_TEST`, `HW_CFG`, `ID_REV`), refuses unless the ESC
//!   is in INIT, never touches the AL Control or SyncManager registers ([`PROTECTED`]), and
//!   writes only into an explicitly given area of process RAM.
//!
//! ```
//! use flowlab_core::Provenance;
//! use flowlab_pdi::{BenchOptions, Lan9252, SimLan9252, SimTiming, run, to_csv};
//!
//! let (sim, clock) = SimLan9252::new(SimTiming::default());
//! let mut esc = Lan9252::new(sim);
//! let options = BenchOptions { sizes: vec![4, 64, 256], reps: 5, ..BenchOptions::default() };
//! let bench = run(&mut esc, &clock, &options, 0x9252).unwrap();
//! assert_eq!(bench.samples.len(), 15);
//! let csv = to_csv(&bench.samples, &["simulated LAN9252".into()], Provenance::Simulated);
//! assert!(csv.contains("# flowlab:provenance=simulated\ndirection,bytes,duration_ns\n"));
//! ```

mod bench;
mod hardware;
mod lan9252;
mod sim;
mod spi;

#[cfg(test)]
mod tests;

use std::io;

pub use bench::{Area, BenchOptions, BenchRun, DEFAULT_SIZES, Plan, SizeSummary, plan, run, summarise};
pub use flowlab_core::pdi_samples::{Direction, Sample, to_csv};
pub use hardware::open_spidev;
pub use lan9252::{Lan9252, LinkInfo, PROTECTED, ReadInstruction, check_range, csr_chunk, reg};
pub use sim::{EscAccess, SimLan9252, SimTiming};
pub use spi::{Clock, MonotonicClock, Segment, SimClock, SpiBus};

/// Why an ESC access or a benchmark failed or was refused.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PdiError {
    /// The SPI device failed.
    #[error("SPI: {0}")]
    Spi(#[from] io::Error),
    /// The link check failed: wrong wiring, mode, clock or chip.
    #[error("link check failed: {0}")]
    Link(String),
    /// A busy or available bit never came.
    #[error("timed out after {polls} polls waiting for {what}")]
    Timeout {
        /// What was awaited.
        what: &'static str,
        /// Register reads made.
        polls: u32,
    },
    /// A safety rule forbids the access or the run.
    #[error("refused: {0}")]
    Refused(String),
    /// Invalid options.
    #[error("{0}")]
    Options(String),
    /// Not available on this system.
    #[error("{0}")]
    Unsupported(String),
}
