//! The mailbox vocabulary the simulator (`flowlab-sim`) and the lab master (`flowlab-master`)
//! share, so that a measured transfer is set up, and described, exactly like a simulated one:
//!
//! * [`MailboxPolicy`]: how a master schedules mailbox datagrams per cycle, with its text form
//!   (`Display`, as results print it) and its short command-line form (`FromStr`);
//! * [`TransferDirection`] and [`MailboxProtocol`];
//! * the generated payload ([`payload_bytes`], [`payload_sha256`], [`transfer_seed`]): the same
//!   bytes on both sides, so the SHA-256s compare;
//! * [`TransferPlan`]: one transfer of a simulator spec as the lab master needs it.
//!
//! ```
//! use flowlab_mailbox::MailboxPolicy;
//!
//! let p: MailboxPolicy = "opportunistic:60,10000".parse().unwrap();
//! assert_eq!(p.to_string(), "opportunistic: mailbox frames within 60 % of the cycle, 10000 ns turnaround");
//! assert_eq!("fixed-quota:2".parse::<MailboxPolicy>().unwrap(), MailboxPolicy::default());
//! ```

mod payload;
mod policy;

pub use payload::{payload, payload_bytes, payload_sha256, transfer_seed};
pub use policy::MailboxPolicy;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Direction of a transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum TransferDirection {
    /// Master → slave.
    MasterToSlave,
    /// Slave → master.
    SlaveToMaster,
}

/// A mailbox protocol that carries a bulk transfer, with the parameters both sides share.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailboxProtocol {
    /// File access over EtherCAT.
    Foe,
    /// CoE SDO normal transfer with segments.
    CoeSegmented,
    /// UDP/IPv4 datagrams tunnelled through EoE.
    Eoe {
        /// IP MTU of the tunnelled frames, bytes.
        mtu_bytes: u16,
    },
}

impl MailboxProtocol {
    /// Short name: `FoE`, `CoE`, `EoE`.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Foe => "FoE",
            Self::CoeSegmented => "CoE",
            Self::Eoe { .. } => "EoE",
        }
    }
}

/// One mailbox transfer of a simulator spec, with what a lab run needs to repeat it: the
/// protocol, the payload (by seed and size), the master's cycle and policy, and the slave's
/// mailbox sizes.
#[derive(Debug, Clone, PartialEq)]
pub struct TransferPlan {
    /// Index of the transfer in the spec.
    pub index: usize,
    /// Seed of the spec (names the payload in results).
    pub spec_seed: u64,
    /// Protocol.
    pub protocol: MailboxProtocol,
    /// Direction.
    pub direction: TransferDirection,
    /// Payload size, bytes.
    pub payload_bytes: u64,
    /// Seed of the payload bytes ([`transfer_seed`] of the spec's seed and the index).
    pub payload_seed: u64,
    /// The spec's cycle time, ns.
    pub cycle_time_ns: u64,
    /// The spec's mailbox policy.
    pub policy: MailboxPolicy,
    /// The slave's "mailbox out" size, bytes.
    pub mailbox_out_bytes: u16,
    /// The slave's "mailbox in" size, bytes.
    pub mailbox_in_bytes: u16,
}
