//! Bus budget: frame layout, wire time and headroom of a cycle, and what the devices allow.
//!
//! Arithmetic only — no simulation. The layout is the one most masters use: one LRW over the
//! process image (split into frames of at most 1486 data bytes), a BRD of AL status, and a
//! FRMW of the DC system time when distributed clocks are on.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::esi::DeviceDescription;

/// Largest datagram data in one frame: 1500 − 2 (EtherCAT header) − 12 (datagram header + WKC).
pub const MAX_DATAGRAM_DATA: usize = 1486;

/// A network to budget.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanSpec {
    /// Cycle time, µs.
    pub cycle_time_us: u32,
    /// Link rate, bit/s.
    #[serde(default = "default_link")]
    pub link_bps: u64,
    /// The cyclic frame carries a DC system-time FRMW.
    #[serde(default = "yes")]
    pub distributed_clocks: bool,
    /// Warn when less than this share of the cycle is left after the cyclic frames, percent.
    #[serde(default = "default_idle")]
    pub min_idle_percent: f64,
    /// Slaves in ring order.
    pub slaves: Vec<PlanSlave>,
}

/// One slave of a plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct PlanSlave {
    /// Name.
    pub name: String,
    /// Output process data, bytes.
    #[serde(default)]
    pub outputs_bytes: u16,
    /// Input process data, bytes.
    #[serde(default)]
    pub inputs_bytes: u16,
    /// Forwarding delay (both directions together), ns.
    #[serde(default = "default_forwarding")]
    pub forwarding_delay_ns: u32,
    /// "Mailbox out" size, bytes; 0 = no mailbox.
    #[serde(default)]
    pub mailbox_out_bytes: u16,
    /// "Mailbox in" size, bytes.
    #[serde(default)]
    pub mailbox_in_bytes: u16,
    /// ESC RAM available to SyncManagers, bytes.
    #[serde(default = "default_dpram")]
    pub dpram_bytes: u32,
    /// Mailbox protocols the device supports (from its ESI); absent = unknown.
    #[serde(default)]
    pub supports: Option<Vec<String>>,
    /// Mailbox protocols this application needs from the device.
    #[serde(default)]
    pub requires: Vec<String>,
    /// Largest mailbox the device allows (ESI `MaxSize`), bytes.
    #[serde(default)]
    pub max_mailbox_bytes: Option<u16>,
}

fn yes() -> bool {
    true
}
fn default_link() -> u64 {
    100_000_000
}
fn default_idle() -> f64 {
    25.0
}
fn default_forwarding() -> u32 {
    700
}
fn default_dpram() -> u32 {
    4096
}

impl PlanSlave {
    /// A plan entry for a device's default configuration from its ESI.
    pub fn from_device(d: &DeviceDescription) -> Self {
        let mbx_out = d.sm_size("MBoxOut").unwrap_or(0);
        let max = d.sync_managers.iter().filter(|s| s.kind.starts_with("MBox")).filter_map(|s| s.max_size).min();
        Self {
            name: if d.type_name.is_empty() { d.name.clone() } else { d.type_name.clone() },
            outputs_bytes: d.default_outputs_bits.div_ceil(8) as u16,
            inputs_bytes: d.default_inputs_bits.div_ceil(8) as u16,
            forwarding_delay_ns: default_forwarding(),
            mailbox_out_bytes: mbx_out,
            mailbox_in_bytes: d.sm_size("MBoxIn").unwrap_or(0),
            dpram_bytes: default_dpram(),
            supports: d.mailbox.as_ref().map(|m| m.names()),
            requires: Vec::new(),
            max_mailbox_bytes: max,
        }
    }
}

/// One datagram of a planned frame.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct PlannedDatagram {
    /// What it carries.
    pub label: String,
    /// Command mnemonic.
    pub command: String,
    /// Data bytes.
    pub data_bytes: u32,
}

/// One planned cyclic frame.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct PlannedFrame {
    /// Datagrams.
    pub datagrams: Vec<PlannedDatagram>,
    /// Frame length without FCS (padded to 60), bytes.
    pub frame_bytes: u32,
    /// Bytes on the wire incl. preamble, FCS and inter-frame gap.
    pub wire_bytes: u32,
    /// Time on the wire, ns.
    pub wire_ns: f64,
}

/// A judgement of the plan.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct PlanCheck {
    /// `"defect"`: cannot work as specified; `"warn"`: works with little margin; `"info"`.
    pub severity: String,
    /// What it is about (a slave name, `"cycle"`).
    pub subject: String,
    /// The finding with its threshold.
    pub message: String,
}

/// The budget of a plan.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct PlanResult {
    /// Cycle time, ns.
    pub cycle_ns: f64,
    /// Link rate, bit/s.
    pub link_bps: f64,
    /// Output process image, bytes.
    pub image_out_bytes: u32,
    /// Input process image, bytes.
    pub image_in_bytes: u32,
    /// Cyclic frames.
    pub frames: Vec<PlannedFrame>,
    /// Wire time of all cyclic frames, ns.
    pub cyclic_wire_ns: f64,
    /// That time as a share of the cycle, percent.
    pub cyclic_percent: f64,
    /// Sum of the slaves' forwarding delays, ns.
    pub ring_delay_ns: f64,
    /// When the last cyclic frame is back at the master, from cycle start, ns.
    pub cyclic_done_ns: f64,
    /// Cycle time left after that, ns.
    pub idle_ns: f64,
    /// Idle share of the cycle, percent.
    pub idle_percent: f64,
    /// Largest mailbox datagram pair (one write of "mailbox out" and one read of
    /// "mailbox in" of the slave with the largest mailboxes) as one frame, wire ns.
    pub mailbox_frame_ns: Option<f64>,
    /// Such frames that fit in the idle time when the master waits for each to return.
    pub mailbox_frames_per_cycle: Option<u32>,
    /// Upper bound of mailbox payload per second in each direction for one slave at that
    /// rate (one message per frame), bytes/s.
    pub mailbox_bytes_per_s: Option<f64>,
    /// Judgements.
    pub checks: Vec<PlanCheck>,
    /// What the arithmetic assumes.
    pub assumptions: Vec<String>,
}

fn wire(frame_bytes: u32, link_bps: u64) -> (u32, f64) {
    let w = ecat_wire::wire_bytes(frame_bytes);
    (w, w as f64 * 8.0 / link_bps as f64 * 1e9)
}

fn frame(datagrams: Vec<PlannedDatagram>, link_bps: u64) -> PlannedFrame {
    let len = 14 + 2 + datagrams.iter().map(|d| 12 + d.data_bytes).sum::<u32>();
    let frame_bytes = len.max(60);
    let (wire_bytes, wire_ns) = wire(frame_bytes, link_bps);
    PlannedFrame { datagrams, frame_bytes, wire_bytes, wire_ns }
}

/// Lay out the cyclic frames of `spec` and check it.
pub fn plan(spec: &PlanSpec) -> PlanResult {
    let link = spec.link_bps.max(1);
    let cycle_ns = spec.cycle_time_us as f64 * 1000.0;
    let image_out: u32 = spec.slaves.iter().map(|s| s.outputs_bytes as u32).sum();
    let image_in: u32 = spec.slaves.iter().map(|s| s.inputs_bytes as u32).sum();
    let image = (image_out + image_in) as usize;

    // Cyclic frames: DC and AL status in the first, the process image split over as many
    // LRW datagrams (one per frame) as needed.
    let mut frames = Vec::new();
    let mut first = Vec::new();
    if spec.distributed_clocks && !spec.slaves.is_empty() {
        first.push(PlannedDatagram { label: "DC system time".into(), command: "FRMW".into(), data_bytes: 8 });
    }
    let room_first = MAX_DATAGRAM_DATA - first.iter().map(|d| 12 + d.data_bytes as usize).sum::<usize>() - 14;
    let mut left = image;
    let mut part = 0;
    while left > 0 {
        let room = if frames.is_empty() { room_first } else { MAX_DATAGRAM_DATA };
        let n = left.min(room);
        let dg = PlannedDatagram {
            label: format!("process image part {part}"),
            command: "LRW".into(),
            data_bytes: n as u32,
        };
        if frames.is_empty() {
            first.push(dg);
            frames.push(Vec::new());
        } else {
            frames.push(vec![dg]);
        }
        left -= n;
        part += 1;
    }
    if !spec.slaves.is_empty() {
        first.push(PlannedDatagram { label: "AL status".into(), command: "BRD".into(), data_bytes: 2 });
    }
    if frames.is_empty() {
        frames.push(Vec::new());
    }
    frames[0] = first;
    let frames: Vec<PlannedFrame> = frames.into_iter().filter(|f| !f.is_empty()).map(|f| frame(f, link)).collect();

    let cyclic_wire_ns: f64 = frames.iter().map(|f| f.wire_ns).sum();
    let ring_delay_ns: f64 = spec.slaves.iter().map(|s| s.forwarding_delay_ns as f64).sum();
    // Frames go back to back; the last returns after its own wire time and the ring.
    let cyclic_done_ns = if frames.is_empty() { 0.0 } else { cyclic_wire_ns + ring_delay_ns };
    let idle_ns = (cycle_ns - cyclic_done_ns).max(0.0);
    let idle_percent = if cycle_ns > 0.0 { 100.0 * idle_ns / cycle_ns } else { 0.0 };

    let mailbox_slave = spec
        .slaves
        .iter()
        .filter(|s| s.mailbox_out_bytes > 0)
        .max_by_key(|s| s.mailbox_out_bytes as u32 + s.mailbox_in_bytes as u32);
    let (mailbox_frame_ns, mailbox_frames_per_cycle, mailbox_bytes_per_s) = match mailbox_slave {
        Some(s) => {
            let f = frame(
                vec![
                    PlannedDatagram {
                        label: "mailbox write".into(),
                        command: "FPWR".into(),
                        data_bytes: s.mailbox_out_bytes as u32,
                    },
                    PlannedDatagram {
                        label: "mailbox read".into(),
                        command: "FPRD".into(),
                        data_bytes: s.mailbox_in_bytes as u32,
                    },
                ],
                link,
            );
            let per = f.wire_ns + ring_delay_ns;
            let n = (idle_ns / per).floor() as u32;
            let payload = s.mailbox_out_bytes.min(s.mailbox_in_bytes).saturating_sub(6) as f64;
            (Some(f.wire_ns), Some(n), Some(n as f64 * payload / (cycle_ns / 1e9)))
        }
        None => (None, None, None),
    };

    let mut checks = Vec::new();
    let mut add = |severity: &str, subject: &str, message: String| {
        checks.push(PlanCheck { severity: severity.into(), subject: subject.into(), message })
    };
    if spec.cycle_time_us == 0 {
        add("defect", "cycle", "The cycle time is 0.".into());
    } else if cyclic_done_ns > cycle_ns {
        add(
            "defect",
            "cycle",
            format!(
                "The cyclic frames need {:.1} µs on the wire and the ring, more than the {:.1} µs cycle.",
                cyclic_done_ns / 1e3,
                cycle_ns / 1e3
            ),
        );
    } else if idle_percent < spec.min_idle_percent {
        add(
            "warn",
            "cycle",
            format!(
                "Only {idle_percent:.1} % of the cycle is left after the cyclic frames (threshold {:.0} %): little room for mailbox traffic and master jitter.",
                spec.min_idle_percent
            ),
        );
    }
    if frames.len() > 1 {
        add(
            "info",
            "process image",
            format!("The {image}-byte process image needs {} frames per cycle.", frames.len()),
        );
    }
    for s in &spec.slaves {
        let need = s.mailbox_out_bytes as u32
            + s.mailbox_in_bytes as u32
            + 3 * (s.outputs_bytes as u32 + s.inputs_bytes as u32);
        if need > s.dpram_bytes {
            add(
                "defect",
                &s.name,
                format!(
                    "SyncManagers need {need} bytes of ESC RAM (mailboxes + 3 × process data), {} available.",
                    s.dpram_bytes
                ),
            );
        }
        if let Some(max) = s.max_mailbox_bytes
            && (s.mailbox_out_bytes > max || s.mailbox_in_bytes > max)
        {
            add("defect", &s.name, format!("Mailbox larger than the device allows ({max} bytes)."));
        }
        for p in &s.requires {
            if s.mailbox_out_bytes == 0 {
                add("defect", &s.name, format!("{p} needs a mailbox; none is configured."));
                continue;
            }
            match &s.supports {
                Some(list) if !list.iter().any(|x| x.eq_ignore_ascii_case(p)) => {
                    add("defect", &s.name, format!("The device does not declare {p} support in its ESI."))
                }
                None => add("info", &s.name, format!("{p} support is unknown (no ESI).")),
                _ => {}
            }
        }
    }

    PlanResult {
        cycle_ns,
        link_bps: link as f64,
        image_out_bytes: image_out,
        image_in_bytes: image_in,
        cyclic_percent: if cycle_ns > 0.0 { 100.0 * cyclic_wire_ns / cycle_ns } else { 0.0 },
        frames,
        cyclic_wire_ns,
        ring_delay_ns,
        cyclic_done_ns,
        idle_ns,
        idle_percent,
        mailbox_frame_ns,
        mailbox_frames_per_cycle,
        mailbox_bytes_per_s,
        checks,
        assumptions: vec![
            "Frames are sent back to back at cycle start and occupy the link for their wire time (incl. preamble, FCS, padding and inter-frame gap).".into(),
            "Each slave forwards with a fixed delay; the ring delay is their sum.".into(),
            "Mailbox headroom assumes the master waits for each mailbox frame to return before sending the next, and one message per direction per frame. Slave-side PDI time is not included — use the simulator for that.".into(),
            "ESC RAM: both mailboxes plus three buffers each for outputs and inputs.".into(),
        ],
    }
}
