//! Deterministic discrete-event simulation of an EtherCAT segment.
//!
//! The simulator models a master, the link and each slave's ESC (SyncManagers, ESC RAM) and
//! PDI (the slave application's access to the ESC), runs bulk transfers over EoE, FoE, CoE or
//! a process-data channel, and writes every frame through `ecat-wire` into a pcapng labelled
//! `simulated` — so a simulated run is analysed by exactly the same code as a capture.
//!
//! It is a model: every result follows from the spec's parameters, which are listed with
//! the model's assumptions in [`SimResult::meta`]. Nothing here is a measurement.
//!
//! ```
//! use flowlab_sim::{SimSpec, simulate};
//!
//! let mut spec = SimSpec::example();
//! spec.transfers.truncate(1); // the EoE transfer only
//! let result = simulate(&spec, None::<Vec<u8>>).unwrap();
//! let t = &result.transfers[0];
//! assert!(t.verified, "every byte arrived, SHA-256 matches");
//! // No transfer is faster than its payload alone on the link.
//! assert!(t.duration_ns.unwrap() >= t.physical_bound_ns);
//! ```

mod engine;
mod protocol;
mod result;
mod spec;

pub use engine::{FIRST_STATION, MASTER_MAC, MBX_BASE, simulate};
pub use protocol::{payload_bytes, payload_sha256};
pub use result::{SimResult, SlaveResult, TransferResult};
pub use spec::{
    CaptureSpec, MailboxAccess, MailboxPolicy, PDO_ACK_BYTES, PDO_CHUNK_HEADER, PdiSpec, SimFaults, SimSlave, SimSpec,
    TransferDirection, TransferProtocol, TransferSpec,
};

/// Why a simulation could not run.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SimError {
    /// The spec is out of range or asks for something the modelled devices cannot do.
    #[error("invalid spec: {0}")]
    Spec(String),
    /// Writing the capture failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// A frame did not fit.
    #[error(transparent)]
    Wire(#[from] ecat_wire::WireError),
}

/// JSON Schema of [`SimSpec`] (draft 2020-12), for editors and the UI.
pub fn spec_schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(SimSpec)).unwrap_or_default()
}

#[cfg(test)]
mod tests;
