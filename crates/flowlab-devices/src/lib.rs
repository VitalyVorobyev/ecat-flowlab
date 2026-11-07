//! What devices can do and how a network is configured — and whether a plan fits.
//!
//! * [`parse_esi`] reads an EtherCAT Slave Information file (ETG.2000): device identity,
//!   SyncManagers, PDOs and their default assignment, supported mailbox protocols.
//! * [`parse_eni`] reads an EtherCAT Network Information file (ETG.2100): configured slaves,
//!   mailbox SyncManagers and the cyclic commands with their expected working counters;
//!   [`write_eni`] writes the same subset.
//! * [`plan`] lays out the cyclic frames of a network and checks cycle time, ESC RAM and
//!   mailbox capabilities.
//!
//! Only the parts FlowLab uses are read; unknown elements are ignored. DTDs are rejected.

mod eni;
mod esi;
mod plan;
mod xml;

pub use eni::{CyclicCmd, CyclicFrame, MailboxConfig, NetworkConfig, SlaveConfig, parse_eni, write_eni};
pub use esi::{DeviceDescription, EsiFile, MailboxSupport, PdoDescription, PdoEntry, SmDescription, parse_esi};
pub use plan::{MAX_DATAGRAM_DATA, PlanCheck, PlanResult, PlanSlave, PlanSpec, PlannedDatagram, PlannedFrame, plan};

/// Why a file could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum DeviceError {
    /// Not well-formed XML.
    #[error("XML: {0}")]
    Xml(String),
    /// Well-formed, but not the expected structure.
    #[error("{0}")]
    Format(String),
}

/// JSON Schema of [`PlanSpec`] (draft 2020-12).
pub fn plan_schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(PlanSpec)).unwrap_or_default()
}

impl PlanSpec {
    /// A coupler, I/O terminals and two drives at 1 ms.
    pub fn example() -> Self {
        let s = |name: &str, o, i, mbx: u16, requires: &[&str]| PlanSlave {
            name: name.into(),
            outputs_bytes: o,
            inputs_bytes: i,
            forwarding_delay_ns: 700,
            mailbox_out_bytes: mbx,
            mailbox_in_bytes: mbx,
            dpram_bytes: 4096,
            supports: None,
            requires: requires.iter().map(|r| (*r).to_owned()).collect(),
            max_mailbox_bytes: None,
        };
        Self {
            cycle_time_us: 1000,
            link_bps: 100_000_000,
            distributed_clocks: true,
            min_idle_percent: 25.0,
            slaves: vec![
                s("coupler", 0, 0, 0, &[]),
                s("DI-16", 0, 2, 0, &[]),
                s("DO-16", 2, 0, 0, &[]),
                s("Servo-1", 12, 16, 128, &["CoE"]),
                s("Servo-2", 12, 16, 128, &["CoE"]),
                s("bulk-endpoint", 8, 8, 512, &["EoE", "FoE"]),
            ],
        }
    }
}

#[cfg(test)]
mod tests;
