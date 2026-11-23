//! The mailbox vocabulary the simulator (`flowlab-sim`), the lab master (`flowlab-master`) and
//! the capture generator (`flowlab-synth`) share, so that a measured transfer is set up, and described, exactly like a simulated one:
//!
//! * [`MailboxPolicy`]: how a master schedules mailbox datagrams per cycle, with its text form
//!   (`Display`, as results print it) and its short command-line form (`FromStr`);
//! * [`TransferDirection`] and [`MailboxProtocol`];
//! * the generated payload ([`payload_bytes`], [`payload_sha256`], [`transfer_seed`]): the same
//!   bytes on both sides, so the SHA-256s compare;
//! * [`TransferPlan`]: one transfer of a simulator spec as the lab master needs it;
//! * [`msg`]: the FoE, CoE SDO and EoE messages, mailbox buffers and counters, as the
//!   simulator, the lab master, its virtual slave and the capture generator write them;
//! * [`voe`]: FlowLab VoE, FlowLab's own bulk transfer protocol in VoE messages (windowed
//!   acknowledgements, the receiver's SHA-256 at the end);
//! * [`responder`]: the slave application's FoE, CoE, EoE and FlowLab VoE answers, shared by
//!   the lab master's virtual slave and the emulated ESC (`flowlab-slave`).
//!
//! ```
//! use flowlab_mailbox::MailboxPolicy;
//!
//! let p: MailboxPolicy = "opportunistic:60,10000".parse().unwrap();
//! assert_eq!(p.to_string(), "opportunistic: mailbox frames within 60 % of the cycle, 10000 ns turnaround");
//! assert_eq!("fixed-quota:2".parse::<MailboxPolicy>().unwrap(), MailboxPolicy::default());
//! ```

pub mod msg;
mod payload;
mod policy;
pub mod responder;
pub mod voe;

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

impl TransferDirection {
    /// `master_to_slave` or `slave_to_master`, as results write it.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MasterToSlave => "master_to_slave",
            Self::SlaveToMaster => "slave_to_master",
        }
    }
}

/// The time `payload_bytes` alone need on a link of `link_bps`, ns: the bound no transfer of
/// that payload can beat, as simulated and measured results report it.
pub fn physical_bound_ns(payload_bytes: u64, link_bps: u64) -> f64 {
    payload_bytes as f64 * 8.0 / link_bps as f64 * 1e9
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
    /// FlowLab VoE ([`voe`]).
    Voe {
        /// DATA messages per acknowledgement.
        window: u16,
    },
}

impl MailboxProtocol {
    /// The mailbox type its messages carry.
    pub const fn mailbox_type(self) -> ecat_wire::MailboxType {
        match self {
            Self::Foe => ecat_wire::MailboxType::Foe,
            Self::CoeSegmented => ecat_wire::MailboxType::Coe,
            Self::Eoe { .. } => ecat_wire::MailboxType::Eoe,
            Self::Voe { .. } => ecat_wire::MailboxType::Voe,
        }
    }

    /// Short name: `FoE`, `CoE`, `EoE`, `VoE`.
    pub const fn name(self) -> &'static str {
        self.mailbox_type().name()
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
