use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Input of `flowlab synth`: a master, a line of slaves, and what goes wrong.
///
/// All values are *chosen*, not measured; the generated capture is labelled `synthetic`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SynthSpec {
    /// Seed of the pseudo-random generator. Same spec + seed ⇒ byte-identical capture.
    pub seed: u64,
    /// Free text stored in the capture comment.
    #[serde(default)]
    pub description: String,
    /// Slaves in ring order.
    pub slaves: Vec<SlaveSpec>,
    /// Nominal cycle time of the cyclic frame, µs.
    pub cycle_time_us: u32,
    /// Number of cycles in the operational phase.
    pub cycles: u32,
    /// Standard deviation of the master's send-time jitter, ns (clamped to ±40 % of a cycle).
    #[serde(default)]
    pub tx_jitter_ns: u32,
    /// Forwarding delay per slave (both directions together), ns.
    #[serde(default = "default_slave_delay")]
    pub slave_delay_ns: u32,
    /// Emit the start-up sequence (counting, addressing, SM/FMMU set-up, state changes,
    /// CoE SDO uploads) before the cyclic phase.
    #[serde(default = "yes")]
    pub include_startup: bool,
    /// SDO uploads per mailbox slave during PREOP.
    #[serde(default = "default_sdo")]
    pub sdo_uploads_per_slave: u32,
    /// Add a distributed-clock FRMW of the system time to every cycle.
    #[serde(default = "yes")]
    pub distributed_clocks: bool,
    /// Unix time of the first frame, seconds.
    #[serde(default = "default_start")]
    pub start_unix_s: u64,
    /// Mailbox traffic beyond the expedited SDO uploads.
    #[serde(default)]
    pub mailbox: MailboxSpec,
    /// Faults to inject.
    #[serde(default)]
    pub faults: FaultSpec,
}

/// Mailbox traffic. PREOP items run per mailbox slave during start-up; OP items run on the
/// first (EoE, FoE) or last (emergencies) mailbox slave in an acyclic frame half a cycle
/// after each cyclic frame, one mailbox access per slave per cycle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MailboxSpec {
    /// Empty polls before a slave's response becomes readable.
    #[serde(default = "default_response_delay")]
    pub response_delay_polls: u32,
    /// PREOP: upload of an object of this many bytes (normal, segmented beyond 112 B); 0 = none.
    #[serde(default)]
    pub segmented_upload_bytes: u32,
    /// PREOP: download of this many bytes (expedited ≤ 4 B, else normal / segmented); 0 = none.
    #[serde(default)]
    pub download_bytes: u32,
    /// PREOP: uploads of a missing object, which the slave aborts.
    #[serde(default)]
    pub aborted_uploads: u32,
    /// OP: FoE write of this many bytes; 0 = none.
    #[serde(default)]
    pub foe_write_bytes: u32,
    /// OP: Ethernet frames tunnelled by EoE to the slave.
    #[serde(default)]
    pub eoe_frames_to_slave: u32,
    /// OP: Ethernet frames tunnelled by EoE from the slave.
    #[serde(default)]
    pub eoe_frames_from_slave: u32,
    /// Size of each tunnelled Ethernet frame, bytes (60–1514).
    #[serde(default = "default_eoe_bytes")]
    pub eoe_frame_bytes: u32,
    /// OP: CoE emergencies.
    #[serde(default)]
    pub emergencies: u32,
}

impl Default for MailboxSpec {
    fn default() -> Self {
        Self {
            response_delay_polls: default_response_delay(),
            segmented_upload_bytes: 0,
            download_bytes: 0,
            aborted_uploads: 0,
            foe_write_bytes: 0,
            eoe_frames_to_slave: 0,
            eoe_frames_from_slave: 0,
            eoe_frame_bytes: default_eoe_bytes(),
            emergencies: 0,
        }
    }
}

/// One slave.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SlaveSpec {
    /// Display name.
    pub name: String,
    /// Output process data (master → slave), bytes.
    #[serde(default)]
    pub outputs_bytes: u16,
    /// Input process data (slave → master), bytes.
    #[serde(default)]
    pub inputs_bytes: u16,
    /// Whether the slave has a CoE mailbox.
    #[serde(default = "yes")]
    pub mailbox: bool,
}

/// Fault injection, as probabilities per cycle.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FaultSpec {
    /// The cyclic frame does not come back (TX without RX).
    #[serde(default)]
    pub lost_frame_probability: f64,
    /// The cyclic frame comes back with a too-low LRW working counter.
    #[serde(default)]
    pub wkc_error_probability: f64,
    /// The master skips a cycle entirely (no TX).
    #[serde(default)]
    pub skipped_cycle_probability: f64,
    /// A mailbox write finds the mailbox still full (WKC 0) and is retried.
    #[serde(default)]
    pub mailbox_write_reject_probability: f64,
}

fn yes() -> bool {
    true
}
fn default_slave_delay() -> u32 {
    700
}
fn default_sdo() -> u32 {
    2
}
fn default_response_delay() -> u32 {
    2
}
fn default_eoe_bytes() -> u32 {
    590
}
fn default_start() -> u64 {
    1_767_225_600 // 2026-01-01T00:00:00Z
}

impl SynthSpec {
    /// The demo line: coupler, digital in/out terminals and a servo drive at 1 kHz, with a
    /// little jitter and rare faults.
    pub fn example() -> Self {
        let slave = |name: &str, outputs_bytes, inputs_bytes, mailbox| SlaveSpec {
            name: name.to_owned(),
            outputs_bytes,
            inputs_bytes,
            mailbox,
        };
        Self {
            seed: 42,
            description: "Demo line: coupler, DI/DO terminals, servo drive; 1 ms cycle".to_owned(),
            slaves: vec![
                slave("EK-coupler", 0, 0, false),
                slave("DI-16", 0, 2, false),
                slave("DO-16", 2, 0, false),
                slave("AI-4", 0, 8, true),
                slave("Servo-1", 12, 16, true),
                slave("Servo-2", 12, 16, true),
            ],
            cycle_time_us: 1000,
            cycles: 5000,
            tx_jitter_ns: 4_000,
            slave_delay_ns: default_slave_delay(),
            include_startup: true,
            sdo_uploads_per_slave: default_sdo(),
            distributed_clocks: true,
            start_unix_s: default_start(),
            mailbox: MailboxSpec {
                segmented_upload_bytes: 300,
                download_bytes: 200,
                aborted_uploads: 1,
                foe_write_bytes: 4096,
                eoe_frames_to_slave: 8,
                eoe_frames_from_slave: 8,
                emergencies: 1,
                ..MailboxSpec::default()
            },
            faults: FaultSpec {
                lost_frame_probability: 0.002,
                wkc_error_probability: 0.001,
                skipped_cycle_probability: 0.001,
                mailbox_write_reject_probability: 0.05,
            },
        }
    }

    /// Check value ranges.
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
        let image: usize = self.slaves.iter().map(|s| s.outputs_bytes as usize + s.inputs_bytes as usize).sum();
        if image > 1400 {
            return Err(format!("process image of {image} bytes does not fit one frame (max 1400 in this generator)"));
        }
        for (name, p) in [
            ("lost_frame_probability", self.faults.lost_frame_probability),
            ("wkc_error_probability", self.faults.wkc_error_probability),
            ("skipped_cycle_probability", self.faults.skipped_cycle_probability),
            ("mailbox_write_reject_probability", self.faults.mailbox_write_reject_probability),
        ] {
            if !(0.0..=1.0).contains(&p) {
                return Err(format!("{name} must be within [0, 1]"));
            }
        }
        if self.faults.mailbox_write_reject_probability >= 1.0 {
            return Err("mailbox_write_reject_probability must be < 1 (writes would never succeed)".into());
        }
        if !(60..=1514).contains(&self.mailbox.eoe_frame_bytes) {
            return Err("eoe_frame_bytes must be within 60..=1514".into());
        }
        if self.mailbox.response_delay_polls > 1000 {
            return Err("response_delay_polls must be ≤ 1000".into());
        }
        if self.mailbox.segmented_upload_bytes > 1_000_000 || self.mailbox.download_bytes > 1_000_000 {
            return Err("SDO transfer sizes must be ≤ 1 MB".into());
        }
        if self.mailbox.foe_write_bytes > 16_000_000 {
            return Err("foe_write_bytes must be ≤ 16 MB".into());
        }
        Ok(())
    }
}
