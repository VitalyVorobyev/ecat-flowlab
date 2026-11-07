//! Analysis of EtherCAT captures.
//!
//! [`analyze`] turns a [`Trace`] into a [`CaptureReport`] plus per-frame side tables
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

mod analyze;
pub mod frames;
pub mod mailbox;
mod report;
mod stats;

pub use analyze::{Analysis, AnalysisOptions, analyze};
pub use report::*;
pub use stats::{Distribution, Histogram};

#[cfg(test)]
mod tests;
