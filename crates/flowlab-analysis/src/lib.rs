//! Analysis of EtherCAT captures.
//!
//! [`analyze`] turns a [`Trace`](flowlab_capture::Trace) into a [`CaptureReport`] plus per-frame side tables
//! (TX/RX pairing, stream membership, WKC verdicts) that [`frames`] uses to list, filter
//! and explain individual frames.
//!
//! What it infers, and from what:
//!
//! * **Cyclic streams**: frames with an identical datagram layout (command, address, length)
//!   repeated at a regular interval. Period is the least-squares slope of send time over
//!   cycle number, so gaps do not bias it; *phase jitter* is each send time's deviation from
//!   that grid, *interval* is the cycle-to-cycle difference.
//! * **Round trip**: a TX frame is paired with the next RX frame of the same layout and the
//!   same first datagram index.
//! * **Working counters**: without the network configuration, the expected WKC of each
//!   datagram position is the most frequent value seen on returning frames.
//! * **Stations and AL states**: from configured-address datagrams, APWR to 0x0010, writes to
//!   AL control (0x0120) and reads of AL status (0x0130).
//! * **Mailbox**: SyncManager configuration writes give each slave's mailbox addresses;
//!   returning datagrams there carry the messages, which [`mailbox`] follows as CoE SDO
//!   transfers, FoE file transfers and EoE fragment streams.
//! * **Mailbox service**: per slave, how many mailbox accesses and frames the master sends per
//!   cycle and how far into the cycle, read as the simulator policy that produces the pattern.
//!
//! Every heuristic is written into the report's `meta.assumptions`, and every threshold behind
//! a finding is a field of [`AnalysisOptions`].
//!
//! ```
//! use flowlab_analysis::{AnalysisOptions, analyze};
//! use flowlab_capture::{CaptureFile, Trace};
//!
//! // A synthetic capture with known ground truth stands in for a measurement.
//! let mut bytes = Vec::new();
//! let truth = flowlab_synth::generate(&flowlab_synth::SynthSpec::example(), &mut bytes).unwrap();
//! let trace = Trace::build(CaptureFile::parse(bytes).unwrap(), "example.pcapng");
//! let report = analyze(&trace, &AnalysisOptions::default()).report;
//!
//! let stream = &report.streams[0];
//! assert!((stream.period_ns - truth.cycle_time_ns as f64).abs() < 100.0);
//! assert_eq!(report.meta.source, flowlab_core::Provenance::Synthetic);
//! ```

mod analyze;
pub mod frames;
pub mod mailbox;
mod report;
mod service;
mod stats;

pub use analyze::{Analysis, AnalysisOptions, analyze};
pub use report::*;
pub use stats::{Distribution, Histogram};

#[cfg(test)]
mod tests;
