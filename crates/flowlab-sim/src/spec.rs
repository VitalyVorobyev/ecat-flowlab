//! The simulator's input (`SimSpec`) and its validation against what the modelled devices support.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Input of `flowlab simulate`: a line of slaves, a master, and transfers to run.
///
/// Every value is a model parameter, chosen or calibrated, never a measurement; results are
/// labelled `simulated`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SimSpec {
    /// Seed for frame loss and master jitter. Same spec ⇒ identical capture and result.
    pub seed: u64,
    /// Free text stored in the capture comment and the result.
    #[serde(default)]
    pub description: String,
    /// Link rate, bit/s.
    #[serde(default = "default_link")]
    pub link_bps: u64,
    /// Master cycle time, µs.
    pub cycle_time_us: u32,
    /// Standard deviation of the master's cycle start jitter, ns (clamped to ±40 % of a cycle).
    #[serde(default)]
    pub master_jitter_ns: u32,
    /// Slaves in ring order.
    pub slaves: Vec<SimSlave>,
    /// How the master schedules mailbox accesses.
    #[serde(default)]
    pub mailbox_policy: MailboxPolicy,
    /// Transfers to run. Transfers on the same slave run one after another, in this order.
    pub transfers: Vec<TransferSpec>,
    /// Stop after this much virtual time even if transfers are unfinished, ms.
    #[serde(default = "default_max_duration")]
    pub max_duration_ms: u64,
    /// Faults.
    #[serde(default)]
    pub faults: SimFaults,
    /// What to write into the capture.
    #[serde(default)]
    pub capture: CaptureSpec,
    /// Unix time of the first frame, seconds.
    #[serde(default = "default_start")]
    pub start_unix_s: u64,
}

/// One slave: its process data, ESC memory, mailbox and PDI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SimSlave {
    /// Display name.
    pub name: String,
    /// Cyclic output process data (master → slave), bytes, besides any PDO channel.
    #[serde(default)]
    pub outputs_bytes: u16,
    /// Cyclic input process data (slave → master), bytes, besides any PDO channel.
    #[serde(default)]
    pub inputs_bytes: u16,
    /// Forwarding delay of this slave (both directions together), ns.
    #[serde(default = "default_forwarding")]
    pub forwarding_delay_ns: u32,
    /// Size of the "mailbox out" SyncManager (master → slave), bytes; 0 = no mailbox.
    #[serde(default)]
    pub mailbox_out_bytes: u16,
    /// Size of the "mailbox in" SyncManager (slave → master), bytes.
    #[serde(default)]
    pub mailbox_in_bytes: u16,
    /// ESC process RAM available to SyncManagers, bytes (e.g. 4096 for a LAN9252).
    #[serde(default = "default_dpram")]
    pub dpram_bytes: u32,
    /// The slave application's access to the ESC.
    #[serde(default)]
    pub pdi: PdiSpec,
    /// Time the slave application needs to handle a message or a process data update, ns.
    #[serde(default = "default_app_processing")]
    pub app_processing_ns: u32,
}

/// Process data interface between ESC and slave application (e.g. SPI).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PdiSpec {
    /// Effective read rate (ESC → application), bit/s.
    pub read_bps: u64,
    /// Effective write rate (application → ESC), bit/s.
    pub write_bps: u64,
    /// Fixed cost per access (command, address phase, driver call), ns.
    pub access_overhead_ns: u32,
    /// How the application moves a mailbox message over the PDI.
    #[serde(default)]
    pub mailbox_access: MailboxAccess,
}

impl Default for PdiSpec {
    fn default() -> Self {
        Self {
            read_bps: 20_000_000,
            write_bps: 20_000_000,
            access_overhead_ns: 5_000,
            mailbox_access: MailboxAccess::default(),
        }
    }
}

/// How the slave application reads and writes its mailbox SyncManagers. The ESC releases a
/// mailbox when its last byte is read and marks it full when its last byte is written.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MailboxAccess {
    /// One access over the whole SyncManager buffer.
    #[default]
    WholeBuffer,
    /// Only the message: reading takes three accesses (header, data, last byte), writing two
    /// (header and data, last byte).
    MessageOnly,
}

/// How the master decides which mailbox datagrams go on the wire. None of these is what every
/// master does; compare them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MailboxPolicy {
    /// Up to `accesses_per_cycle` mailbox datagrams (writes and reads together) per cycle,
    /// shared round-robin across slaves, in one frame after the cyclic frame.
    FixedQuota {
        /// Mailbox datagrams per cycle.
        accesses_per_cycle: u32,
    },
    /// Each cycle, `slaves_per_cycle` slaves with mailbox work get one write and one read.
    RoundRobin {
        /// Slaves served per cycle.
        slaves_per_cycle: u32,
    },
    /// Mailbox frames back to back while the cycle has spare time: after each mailbox frame
    /// returns, another is sent if it would start within `bus_budget_percent` of the cycle.
    Opportunistic {
        /// Share of the cycle the master may fill with mailbox frames, percent.
        bus_budget_percent: f64,
        /// Master turnaround between a frame's return and the next frame, ns.
        turnaround_ns: u32,
    },
}

impl Default for MailboxPolicy {
    fn default() -> Self {
        Self::FixedQuota { accesses_per_cycle: 2 }
    }
}

/// Direction of a transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TransferDirection {
    /// Master → slave.
    MasterToSlave,
    /// Slave → master.
    SlaveToMaster,
}

/// How the payload crosses the bus.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TransferProtocol {
    /// Ethernet over EtherCAT carrying UDP/IPv4 datagrams (a simplified tunnel: no TCP).
    Eoe {
        /// IP MTU of the tunnelled frames, bytes (≤ 1500).
        #[serde(default = "default_mtu")]
        mtu_bytes: u16,
    },
    /// File access over EtherCAT: write (master → slave) or read (slave → master).
    Foe,
    /// CoE SDO download or upload, normal transfer with segments.
    CoeSegmented,
    /// A custom channel in cyclic process data: sequence-numbered chunks, cumulative
    /// acknowledgements in the opposite direction, go-back-N on a stalled window.
    PdoChannel {
        /// Bytes reserved per cycle for the channel, including its 8-byte chunk header.
        bytes_per_cycle: u16,
        /// Chunks that may be unacknowledged.
        #[serde(default = "default_window")]
        window: u32,
        /// Chunk updates without acknowledgement progress before resending from the first
        /// unacknowledged chunk: master cycles for master → slave, the slave application's
        /// process-data updates for slave → master.
        #[serde(default = "default_retransmit")]
        retransmit_after_cycles: u32,
    },
}

/// One transfer of an opaque payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TransferSpec {
    /// Index of the slave in `slaves`.
    pub slave: usize,
    /// Direction.
    pub direction: TransferDirection,
    /// Payload, bytes (decimal: 10 MB = 10,000,000; 10 MiB = 10,485,760).
    pub payload_bytes: u64,
    /// Protocol.
    pub protocol: TransferProtocol,
    /// Earliest start after the first cycle, ms.
    #[serde(default)]
    pub start_ms: f64,
}

/// Fault injection.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SimFaults {
    /// A frame is lost before the first slave (it occupies the link but never returns).
    #[serde(default)]
    pub frame_loss_probability: f64,
}

/// Capture output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptureSpec {
    /// Write frames at all.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Stop writing after this many frames (the simulation continues).
    #[serde(default = "default_max_frames")]
    pub max_frames: u64,
}

impl Default for CaptureSpec {
    fn default() -> Self {
        Self { enabled: true, max_frames: default_max_frames() }
    }
}

fn yes() -> bool {
    true
}
fn default_link() -> u64 {
    100_000_000
}
fn default_max_duration() -> u64 {
    60_000
}
fn default_start() -> u64 {
    1_767_225_600
}
fn default_forwarding() -> u32 {
    700
}
fn default_dpram() -> u32 {
    4096
}
fn default_app_processing() -> u32 {
    20_000
}
fn default_mtu() -> u16 {
    1500
}
fn default_window() -> u32 {
    2
}
fn default_retransmit() -> u32 {
    8
}
fn default_max_frames() -> u64 {
    200_000
}

/// Bytes of the PDO channel chunk header: sequence (u32), length (u16), reserved (u16).
pub const PDO_CHUNK_HEADER: usize = 8;
/// Bytes of the PDO channel acknowledgement: next expected sequence (u32).
pub const PDO_ACK_BYTES: usize = 4;

impl SimSlave {
    /// Output and input bytes this slave's PDO channel adds, if a transfer uses one.
    pub(crate) fn channel_bytes(&self, index: usize, transfers: &[TransferSpec]) -> (usize, usize) {
        let mut out = 0usize;
        let mut inp = 0usize;
        for t in transfers.iter().filter(|t| t.slave == index) {
            if let TransferProtocol::PdoChannel { bytes_per_cycle, .. } = t.protocol {
                let (o, i) = match t.direction {
                    TransferDirection::MasterToSlave => (bytes_per_cycle as usize, PDO_ACK_BYTES),
                    TransferDirection::SlaveToMaster => (PDO_ACK_BYTES, bytes_per_cycle as usize),
                };
                out = out.max(o);
                inp = inp.max(i);
            }
        }
        (out, inp)
    }

    /// ESC RAM the SyncManagers need: both mailboxes, plus three buffers each for outputs and
    /// inputs (3-buffer mode).
    pub(crate) fn dpram_needed(&self, index: usize, transfers: &[TransferSpec]) -> u32 {
        let (co, ci) = self.channel_bytes(index, transfers);
        self.mailbox_out_bytes as u32
            + self.mailbox_in_bytes as u32
            + 3 * (self.outputs_bytes as u32 + co as u32)
            + 3 * (self.inputs_bytes as u32 + ci as u32)
    }
}

impl SimSpec {
    /// Check ranges and what the modelled devices can support.
    pub fn validate(&self) -> Result<(), String> {
        if self.slaves.is_empty() {
            return Err("at least one slave is required".into());
        }
        if self.slaves.len() > 0xEFFF {
            return Err("too many slaves".into());
        }
        if self.cycle_time_us == 0 {
            return Err("cycle_time_us must be > 0".into());
        }
        if self.link_bps < 1_000_000 {
            return Err("link_bps must be ≥ 1 Mbit/s".into());
        }
        if !(0.0..1.0).contains(&self.faults.frame_loss_probability) {
            return Err("frame_loss_probability must be within [0, 1)".into());
        }
        match &self.mailbox_policy {
            MailboxPolicy::FixedQuota { accesses_per_cycle: 0 } | MailboxPolicy::RoundRobin { slaves_per_cycle: 0 } => {
                return Err("the mailbox policy must allow at least one access per cycle".into());
            }
            MailboxPolicy::Opportunistic { bus_budget_percent, .. } if !(0.0..=100.0).contains(bus_budget_percent) => {
                return Err("bus_budget_percent must be within 0..=100".into());
            }
            _ => {}
        }
        let mut image = 0usize;
        for (i, s) in self.slaves.iter().enumerate() {
            let has_mbx = s.mailbox_out_bytes > 0 || s.mailbox_in_bytes > 0;
            if has_mbx && (s.mailbox_out_bytes < 32 || s.mailbox_in_bytes < 32) {
                return Err(format!("slave {i} ({}): mailboxes need ≥ 32 bytes in both directions", s.name));
            }
            if s.mailbox_out_bytes > 1484 || s.mailbox_in_bytes > 1484 {
                return Err(format!("slave {i} ({}): a mailbox must fit one frame (≤ 1484 bytes)", s.name));
            }
            if s.pdi.read_bps == 0 || s.pdi.write_bps == 0 {
                return Err(format!("slave {i} ({}): PDI rates must be > 0", s.name));
            }
            let need = s.dpram_needed(i, &self.transfers);
            if need > s.dpram_bytes {
                return Err(format!(
                    "slave {i} ({}): SyncManagers need {need} bytes of ESC RAM (mailboxes + 3 × process data), only {} available",
                    s.name, s.dpram_bytes
                ));
            }
            let (co, ci) = s.channel_bytes(i, &self.transfers);
            image += s.outputs_bytes as usize + s.inputs_bytes as usize + co + ci;
        }
        if image > 1400 {
            return Err(format!("process image of {image} bytes does not fit one frame (max 1400 in this model)"));
        }
        for (k, t) in self.transfers.iter().enumerate() {
            let Some(s) = self.slaves.get(t.slave) else {
                return Err(format!("transfer {k}: no slave {}", t.slave));
            };
            if t.start_ms < 0.0 || !t.start_ms.is_finite() {
                return Err(format!("transfer {k}: start_ms must be ≥ 0"));
            }
            if t.payload_bytes > 1_000_000_000 {
                return Err(format!("transfer {k}: payload above 1 GB is not supported"));
            }
            match &t.protocol {
                TransferProtocol::PdoChannel { bytes_per_cycle, window, .. } => {
                    if (*bytes_per_cycle as usize) <= PDO_CHUNK_HEADER {
                        return Err(format!("transfer {k}: bytes_per_cycle must exceed the 8-byte chunk header"));
                    }
                    if *window == 0 {
                        return Err(format!("transfer {k}: window must be ≥ 1"));
                    }
                    let other = self.transfers.iter().enumerate().any(|(j, o)| {
                        j != k && o.slave == t.slave && matches!(o.protocol, TransferProtocol::PdoChannel { .. })
                    });
                    if other {
                        return Err(format!("transfer {k}: one PDO channel per slave"));
                    }
                }
                other => {
                    if s.mailbox_out_bytes == 0 {
                        return Err(format!("transfer {k}: slave {} ({}) has no mailbox", t.slave, s.name));
                    }
                    if let TransferProtocol::Eoe { mtu_bytes } = other {
                        if !(68..=1500).contains(mtu_bytes) {
                            return Err(format!("transfer {k}: mtu_bytes must be within 68..=1500"));
                        }
                        // Mailbox header (6) + EoE header (4) + at least one 32-byte block.
                        if s.mailbox_out_bytes.min(s.mailbox_in_bytes) < 42 {
                            return Err(format!(
                                "transfer {k}: EoE needs mailboxes of ≥ 42 bytes (one 32-byte fragment)"
                            ));
                        }
                    }
                }
            }
        }
        if self.transfers.is_empty() {
            return Err("at least one transfer is required".into());
        }
        Ok(())
    }

    /// A small line with one bulk-capable slave: EoE, FoE, CoE and a PDO channel, 256 kB each,
    /// one after another (the PDO channel in parallel), at 1 ms with the default policy.
    pub fn example() -> Self {
        let pdi = PdiSpec::default();
        let io = |name: &str, o, i| SimSlave {
            name: name.into(),
            outputs_bytes: o,
            inputs_bytes: i,
            forwarding_delay_ns: default_forwarding(),
            mailbox_out_bytes: 0,
            mailbox_in_bytes: 0,
            dpram_bytes: default_dpram(),
            pdi: pdi.clone(),
            app_processing_ns: default_app_processing(),
        };
        let t =
            |protocol, direction| TransferSpec { slave: 2, direction, payload_bytes: 256_000, protocol, start_ms: 0.0 };
        Self {
            seed: 1,
            description: "One bulk slave: EoE, FoE, CoE and PDO channel, 256 kB each, 1 ms cycle".into(),
            link_bps: default_link(),
            cycle_time_us: 1000,
            master_jitter_ns: 2_000,
            slaves: vec![
                io("coupler", 0, 0),
                io("io-16", 2, 2),
                SimSlave {
                    name: "bulk-endpoint".into(),
                    outputs_bytes: 8,
                    inputs_bytes: 8,
                    mailbox_out_bytes: 512,
                    mailbox_in_bytes: 512,
                    ..io("", 0, 0)
                },
            ],
            mailbox_policy: MailboxPolicy::default(),
            transfers: vec![
                t(TransferProtocol::Eoe { mtu_bytes: default_mtu() }, TransferDirection::MasterToSlave),
                t(TransferProtocol::Foe, TransferDirection::MasterToSlave),
                t(TransferProtocol::CoeSegmented, TransferDirection::MasterToSlave),
                t(
                    TransferProtocol::PdoChannel {
                        bytes_per_cycle: 256,
                        window: default_window(),
                        retransmit_after_cycles: default_retransmit(),
                    },
                    TransferDirection::MasterToSlave,
                ),
            ],
            max_duration_ms: default_max_duration(),
            faults: SimFaults::default(),
            capture: CaptureSpec::default(),
            start_unix_s: default_start(),
        }
    }
}
