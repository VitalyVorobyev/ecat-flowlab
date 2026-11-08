//! A lab EtherCAT master that measures what the simulator predicts.
//!
//! The crate has three parts:
//!
//! * a **transfer engine** without hardware dependencies ([`run_transfer`]): FoE write and
//!   read, CoE SDO download and upload with segments, and EoE (UDP/IPv4 frames in EoE
//!   fragments), moved through a slave's mailbox SyncManagers one FPWR or FPRD at a time over
//!   a small async trait, [`MailboxBus`]. The mailbox service per cycle follows the
//!   simulator's [`MailboxPolicy`](flowlab_mailbox::MailboxPolicy) (fixed quota, round robin,
//!   opportunistic), so a real run is set up like a simulated one. The result,
//!   [`MeasuredTransfer`], has provenance `measured` and the simulator's field names wherever
//!   a field means the same, plus per-cycle timing;
//! * a **virtual slave** ([`VirtualSlave`]): ESC mailbox semantics (WKC 0 on a full "mailbox
//!   out" or an empty "mailbox in") with FoE, CoE and EoE responders and optional faults, in
//!   virtual time. The engine is tested end to end against it;
//! * with the cargo feature **`hardware`**, the `hardware` module: EtherCrab on a network
//!   interface for `scan`, a cyclic `run` and `transfer`. It sends frames, so it is
//!   behind a feature, and opening a session needs the `QuietSegment`
//!   that only the `safety` module's checks hand out: a named interface, an explicit
//!   acknowledgement that the segment is an isolated lab network, and no other master heard.
//!
//! ```
//! use flowlab_master::{Payload, TransferJob, TransferOptions, TransferProtocol, VirtualSlave, VirtualSlaveConfig, run_transfer};
//!
//! let slave = VirtualSlave::new(VirtualSlaveConfig::default());
//! let file: Vec<u8> = (0..10_000u32).map(|i| i as u8).collect();
//! let job = TransferJob {
//!     slave: "virtual".into(),
//!     mailbox: slave.layout(),
//!     protocol: TransferProtocol::foe(),
//!     payload: Payload::Send(file.clone()),
//!     payload_name: "test pattern".into(),
//! };
//! let r = futures_lite::future::block_on(run_transfer(&slave, job, &TransferOptions::default())).unwrap();
//! assert!(r.completed);
//! assert_eq!(slave.log().files[0].1, file); // byte-identical at the slave
//! assert_eq!(r.meta.source, flowlab_core::Provenance::Measured);
//! ```

mod bus;
mod engine;
mod protocol;
mod result;
mod virtual_slave;

#[cfg(feature = "hardware")]
pub mod hardware;
#[cfg(feature = "hardware")]
pub mod safety;

pub use bus::{
    BusError, MailboxBus, MailboxLayout, Reply, SM_MAILBOX_IN, SM_MAILBOX_OUT, SM_REPEAT_ACK, SM_REPEAT_REQUEST,
    read_mailbox, read_sm, read_sm_status, sm_register, write_mailbox,
};
pub use engine::{
    Payload, PollMode, SimulatedSetup, TransferJob, TransferOptions, TransferProtocol, run_transfer, setup_like,
};
pub use protocol::{EOE_HEADERS, EoeAddresses, EoeReassembler, eoe_fragments, udp_frame, udp_payload};
pub use result::{CycleRecord, CycleTiming, MeasuredTransfer, RunResult, ScanResult, ScannedSlave, ScannedSm, Summary};
pub use virtual_slave::{VirtualSlave, VirtualSlaveConfig, VirtualSlaveLog};

/// Why a transfer or a hardware command could not run.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MasterError {
    /// The request is invalid (sizes, options).
    #[error("{0}")]
    Invalid(String),
    /// The bus failed in a way the transfer cannot recover from.
    #[error("bus: {0}")]
    Bus(String),
    /// A safety check refused to start (no acknowledgement, another master heard, no way to
    /// check).
    #[error("{0}")]
    Refused(String),
}

#[cfg(test)]
mod tests;
