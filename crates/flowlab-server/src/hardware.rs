//! Slave hardware lab endpoints: PDI slave (`flowlab pdi-slave`), LAN9252 PHY inspector
//! (`flowlab pdi-phy`), and timed SPI PDI benchmark + model fit (`flowlab pdi-bench` /
//! `pdi-fit`).
//!
//! All three share the ESC's SPI bus through [`HardwareSlot`], so at most one operation accesses
//! `/dev/spidev*` (or the simulated LAN9252) at a time.

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::State;
use ecat_wire::AlState;
use flowlab_core::Provenance;
use flowlab_mailbox::responder::ResponderLog;
use flowlab_pdi::{
    Area, BenchOptions, Clock, DEFAULT_SIZES, Lan9252, LinkInfo, MonotonicClock, PdiError, PhyAccess, PhyStatus,
    ReadInstruction, SimLan9252, SimTiming, SlaveAccess, SpiBus, SpiTuning, open_spidev, run as run_pdi_bench,
    summarise as summarise_pdi_bench, to_csv as pdi_to_csv,
};
use flowlab_slave::pdi::{AlChange, MIN_MAILBOX_BYTES, Mailboxes, PdiSlave, PdiSlaveConfig, PdiSlaveStats};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{ApiError, AppState, blocking};

const DEFAULT_CHIP_ID: u16 = 0x9252;
const DEFAULT_SLAVE_CLOCK_HZ: u32 = 20_000_000;
const DEFAULT_PHY_CLOCK_HZ: u32 = 10_000_000;
const MAX_UPLOAD_PAYLOAD_BYTES: u64 = 64 * 1024 * 1024;
const MAX_BENCH_REPS: u32 = 500;
const MAX_BENCH_WARMUP: u32 = 50;
const MAX_SERIES_POINTS: usize = 120;
const MAX_EVENT_LOG: usize = 200;

/// Hardware backend settings passed from `flowlab serve` flags.
#[derive(Debug, Clone)]
pub struct HardwareConfig {
    /// Default LAN9252 spidev path (e.g. `/dev/spidev1.0`).
    pub device: Option<PathBuf>,
    /// Allow simulated LAN9252 mode in the UI (`--sim-pdi`).
    pub sim_pdi: bool,
    /// Permit taking over a real ESC from the UI (`--replace-slave-application`).
    pub allow_replace_app: bool,
    /// Permit SPI write benchmarks on a real ESC from the UI (`--allow-writes`).
    pub allow_writes: bool,
    /// Permit PHY auto-neg and register writes from the UI (`--allow-phy-writes`).
    pub allow_phy_writes: bool,
    /// Expected LAN9252 chip ID (`ID_REV` bits 31:16).
    pub chip_id: u16,
}

impl Default for HardwareConfig {
    fn default() -> Self {
        Self {
            device: None,
            sim_pdi: false,
            allow_replace_app: false,
            allow_writes: false,
            allow_phy_writes: false,
            chip_id: DEFAULT_CHIP_ID,
        }
    }
}

/// Hardware capabilities and discovered devices reported by `/api/capabilities`.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct HardwareCapabilities {
    /// True when either a default spidev device, discovered `/dev/spidev*`, or `--sim-pdi` is present.
    pub pdi_enabled: bool,
    /// True when compiled on Linux with raw-socket EoE bridge support (`slave-wire` feature).
    pub eoe_bridge_supported: bool,
    /// Default spidev path from `--device`, if provided.
    pub default_device: Option<String>,
    /// Simulated LAN9252 backend enabled (`--sim-pdi`).
    pub sim_pdi: bool,
    /// Real ESC takeover enabled (`--replace-slave-application`).
    pub allow_replace_app: bool,
    /// Real ESC SPI write benchmark enabled (`--allow-writes`).
    pub allow_writes: bool,
    /// Real ESC PHY writes enabled (`--allow-phy-writes`).
    pub allow_phy_writes: bool,
    /// Discovered `/dev/spidev*` device paths on this host.
    pub spidev_devices: Vec<String>,
    /// Discovered non-loopback network interface names on Linux.
    pub net_interfaces: Vec<String>,
}

impl HardwareConfig {
    /// Build the public capability summary by scanning local device nodes.
    pub fn capabilities(&self) -> HardwareCapabilities {
        let spidev_devices = discover_spidev();
        let net_interfaces = discover_net_interfaces();
        let pdi_enabled = self.sim_pdi || self.device.is_some() || !spidev_devices.is_empty();
        HardwareCapabilities {
            pdi_enabled,
            eoe_bridge_supported: cfg!(all(feature = "slave-wire", target_os = "linux")),
            default_device: self.device.as_ref().map(|p| p.display().to_string()),
            sim_pdi: self.sim_pdi,
            allow_replace_app: self.allow_replace_app,
            allow_writes: self.allow_writes,
            allow_phy_writes: self.allow_phy_writes,
            spidev_devices,
            net_interfaces,
        }
    }
}

fn discover_spidev() -> Vec<String> {
    let Ok(entries) = std::fs::read_dir("/dev") else {
        return Vec::new();
    };
    let mut paths: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name();
            let s = name.to_str()?;
            s.starts_with("spidev").then(|| format!("/dev/{s}"))
        })
        .collect();
    paths.sort();
    paths
}

fn discover_net_interfaces() -> Vec<String> {
    if !cfg!(target_os = "linux") {
        return Vec::new();
    }
    let Ok(entries) = std::fs::read_dir("/sys/class/net") else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            let s = e.file_name().to_str()?.to_owned();
            (s != "lo").then_some(s)
        })
        .collect();
    names.sort();
    names
}

/// SPI read opcode choice for the LAN9252.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SlaveReadInstruction {
    /// Standard `READ` (`0x03`, up to 30 MHz).
    #[default]
    Read,
    /// `FAST READ` (`0x0B` + dummy byte, up to 80 MHz).
    FastRead,
}

impl From<SlaveReadInstruction> for ReadInstruction {
    fn from(v: SlaveReadInstruction) -> Self {
        match v {
            SlaveReadInstruction::Read => ReadInstruction::Read,
            SlaveReadInstruction::FastRead => ReadInstruction::FastRead,
        }
    }
}

/// LAN9252 chip identity and AL status returned by link check.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct LinkInfoView {
    /// Chip ID (`ID_REV` bits 31:16).
    pub chip_id: u16,
    /// Chip revision (`ID_REV` bits 15:0).
    pub revision: u16,
    /// AL status name (`INIT`, `PREOP`, etc.).
    pub al_state: String,
    /// Raw `ID_REV` register.
    pub id_rev: u32,
    /// Raw `HW_CFG` register.
    pub hw_cfg: u32,
}

impl From<LinkInfo> for LinkInfoView {
    fn from(l: LinkInfo) -> Self {
        Self {
            chip_id: l.chip_id(),
            revision: l.revision(),
            al_state: l.al_state(),
            id_rev: l.id_rev,
            hw_cfg: l.hw_cfg,
        }
    }
}

/// Active mailbox SyncManager geometry in ESC RAM.
#[derive(Debug, Clone, Copy, Serialize, TS)]
#[ts(export)]
pub struct MailboxesView {
    /// SM0 ("mailbox out") start address.
    pub out_start: u16,
    /// SM0 length, bytes.
    pub out_len: u16,
    /// SM1 ("mailbox in") start address.
    pub in_start: u16,
    /// SM1 length, bytes.
    pub in_len: u16,
}

impl From<Mailboxes> for MailboxesView {
    fn from(m: Mailboxes) -> Self {
        Self { out_start: m.out_start, out_len: m.out_len, in_start: m.in_start, in_len: m.in_len }
    }
}

/// Lifecycle state of the PDI slave loop.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SlaveRunState {
    /// Never started in this server session.
    #[default]
    Idle,
    /// Polling loop is active on the ESC.
    Running,
    /// Stopped cleanly (AL Status returned to `INIT`).
    Stopped,
    /// Exited due to an error.
    Failed,
}

/// Cumulative counters of the PDI slave loop.
#[derive(Debug, Clone, Copy, Default, Serialize, TS)]
#[ts(export)]
pub struct SlaveStatsView {
    /// Total `step()` polls executed.
    pub polls: u64,
    /// AL Control requests handled.
    pub al_requests: u64,
    /// Mailbox messages read from SM0 ("mailbox out").
    pub messages_in: u64,
    /// Mailbox messages written to SM1 ("mailbox in").
    pub messages_out: u64,
    /// Bytes read from SM0 buffers.
    pub bytes_in: u64,
    /// Bytes written to SM1 buffers.
    pub bytes_out: u64,
    /// Fallback SyncManager status reads.
    pub status_reads: u64,
}

impl From<PdiSlaveStats> for SlaveStatsView {
    fn from(s: PdiSlaveStats) -> Self {
        Self {
            polls: s.polls,
            al_requests: s.al_requests,
            messages_in: s.messages_in,
            messages_out: s.messages_out,
            bytes_in: s.bytes_in,
            bytes_out: s.bytes_out,
            status_reads: s.status_reads,
        }
    }
}

/// EoE raw-socket bridge statistics and last applied Set-IP summary.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct BridgeStatsView {
    /// Raw-socket peer interface (`--eoe-bridge`).
    pub bridge_iface: String,
    /// Host network interface (`--eoe-host`).
    pub host_iface: String,
    /// Ethernet frames forwarded from master EoE to host.
    pub to_host: u64,
    /// Bytes forwarded from master EoE to host.
    pub to_host_bytes: u64,
    /// Ethernet frames forwarded from host to master EoE.
    pub to_master: u64,
    /// Bytes forwarded from host to master EoE.
    pub to_master_bytes: u64,
    /// Last EoE Set IP Parameter request applied to `host_iface`, formatted.
    pub last_set_ip: Option<String>,
}

/// One 1 Hz telemetry sample of the running PDI slave.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct SlaveRatePoint {
    /// Seconds since start.
    pub t_s: f64,
    /// Polls per second in this interval.
    pub polls_s: f64,
    /// Mailbox messages/s from master to slave.
    pub msg_in_s: f64,
    /// Mailbox messages/s from slave to master.
    pub msg_out_s: f64,
    /// Mailbox bytes/s read from SM0.
    pub bytes_in_s: f64,
    /// Mailbox bytes/s written to SM1.
    pub bytes_out_s: f64,
    /// Bridged EoE frames/s toward host.
    pub eoe_to_host_s: f64,
    /// Bridged EoE frames/s toward master.
    pub eoe_to_master_s: f64,
}

/// AL state transition logged by the PDI slave.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct SlaveAlEvent {
    /// Seconds since start.
    pub t_s: f64,
    /// Resulting AL state name.
    pub state: String,
    /// Error code if refused.
    pub error: Option<u16>,
    /// Mailbox SyncManagers configured in `PREOP`.
    pub mailboxes: Option<MailboxesView>,
}

/// Payload received by the PDI slave's mailbox responder.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ReceivedPayload {
    /// Seconds since start.
    pub t_s: f64,
    /// Protocol name (`FoE`, `CoE SDO`, `FlowLab VoE`, `EoE`).
    pub protocol: String,
    /// Optional filename (FoE) or note.
    pub name: Option<String>,
    /// Payload size, bytes.
    pub bytes: u64,
    /// Lower-case hex SHA-256 of the payload.
    pub sha256: String,
}

/// Request body for `POST /api/slave/start`.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct SlaveStartRequest {
    /// spidev path (defaults to server `--device` or first discovered `/dev/spidev*`).
    #[serde(default)]
    pub device: Option<String>,
    /// Run against a simulated LAN9252 (requires server `--sim-pdi`).
    #[serde(default)]
    pub sim: bool,
    /// SPI clock frequency, Hz.
    #[serde(default = "default_slave_clock")]
    pub clock_hz: u32,
    /// SPI mode (`0` or `3`).
    #[serde(default)]
    pub mode: u8,
    /// SPI read opcode.
    #[serde(default)]
    pub read_instruction: SlaveReadInstruction,
    /// SPI tuning (`fast`, `stack`, or comma-separated `split,batch,fifo,noabort`).
    #[serde(default = "default_fast_tuning")]
    pub spi_tuning: String,
    /// Largest mailbox SyncManager accepted in PRE-OP, bytes.
    #[serde(default = "default_max_mailbox")]
    pub max_mailbox_bytes: u16,
    /// Read/write only message length + last DWORD (`true`), vs whole mailbox buffer (`false`).
    #[serde(default = "default_true")]
    pub message_only: bool,
    /// Generated upload payload size for FoE read / SDO upload / VoE from-slave, bytes.
    #[serde(default)]
    pub upload_bytes: u64,
    /// Seed for the generated upload payload.
    #[serde(default)]
    pub upload_seed: u64,
    /// Optional peer interface for EoE bridging (e.g. `fleoe0b`).
    #[serde(default)]
    pub eoe_bridge: Option<String>,
    /// Optional host interface behind `eoe_bridge` (e.g. `fleoe0`).
    #[serde(default)]
    pub eoe_host: Option<String>,
    /// Override expected LAN9252 chip ID.
    #[serde(default)]
    pub chip_id: Option<u16>,
}

fn default_slave_clock() -> u32 {
    DEFAULT_SLAVE_CLOCK_HZ
}
fn default_phy_clock() -> u32 {
    DEFAULT_PHY_CLOCK_HZ
}
fn default_fast_tuning() -> String {
    "fast".to_owned()
}
fn default_max_mailbox() -> u16 {
    1024
}
fn default_true() -> bool {
    true
}
fn default_phys() -> Vec<u8> {
    vec![0, 1, 2, 3]
}
fn default_bench_sizes() -> Vec<u32> {
    DEFAULT_SIZES.to_vec()
}
fn default_bench_reps() -> u32 {
    30
}
fn default_bench_warmup() -> u32 {
    3
}
fn default_bench_seed() -> u64 {
    1
}
fn default_sim_tx_ns() -> u64 {
    5_000
}

/// Live or last snapshot of the PDI slave (`GET /api/slave/status`).
#[derive(Debug, Clone, Default, Serialize, TS)]
#[ts(export)]
pub struct SlaveStatus {
    /// Lifecycle state.
    pub state: SlaveRunState,
    /// Provenance (`measured` for spidev, `simulated` for `--sim-pdi`).
    pub provenance: Option<Provenance>,
    /// Target description (device path or simulated target).
    pub target: Option<String>,
    /// Active SPI tuning name.
    pub spi_tuning: Option<String>,
    /// SPI clock frequency, Hz.
    pub clock_hz: Option<u32>,
    /// Configured maximum mailbox size, bytes.
    pub max_mailbox_bytes: Option<u16>,
    /// True if message-only mailbox buffer access is enabled.
    pub message_only: Option<bool>,
    /// Elapsed runtime, seconds.
    pub elapsed_s: f64,
    /// Link check identity.
    pub link: Option<LinkInfoView>,
    /// Current AL status state name.
    pub al_state: Option<String>,
    /// Current AL error code, if any.
    pub al_error: Option<u16>,
    /// Active mailbox SyncManager layout in `PREOP`.
    pub mailboxes: Option<MailboxesView>,
    /// Cumulative counters.
    pub stats: SlaveStatsView,
    /// EoE bridge stats, if configured.
    pub bridge: Option<BridgeStatsView>,
    /// Rolling 1 Hz rate samples (newest last).
    pub series: Vec<SlaveRatePoint>,
    /// AL state changes observed in this run.
    pub al_events: Vec<SlaveAlEvent>,
    /// Payloads received over FoE / CoE / VoE / EoE.
    pub received: Vec<ReceivedPayload>,
    /// Error message when `state == "failed"`.
    pub error: Option<String>,
}

/// Request body for `POST /api/phy/read`.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct PhyReadRequest {
    /// spidev path (defaults to server `--device` or first discovered `/dev/spidev*`).
    #[serde(default)]
    pub device: Option<String>,
    /// Use simulated LAN9252 (requires `--sim-pdi`).
    #[serde(default)]
    pub sim: bool,
    /// SPI clock frequency, Hz.
    #[serde(default = "default_phy_clock")]
    pub clock_hz: u32,
    /// SPI mode (`0` or `3`).
    #[serde(default)]
    pub mode: u8,
    /// Expected chip ID.
    #[serde(default)]
    pub chip_id: Option<u16>,
    /// MII PHY addresses to inspect (`0..=31`).
    #[serde(default = "default_phys")]
    pub phys: Vec<u8>,
}

/// Request body for `POST /api/phy/autoneg`.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct PhyAutonegRequest {
    /// spidev path.
    #[serde(default)]
    pub device: Option<String>,
    /// Use simulated LAN9252 (requires `--sim-pdi`).
    #[serde(default)]
    pub sim: bool,
    /// SPI clock frequency, Hz.
    #[serde(default = "default_phy_clock")]
    pub clock_hz: u32,
    /// SPI mode (`0` or `3`).
    #[serde(default)]
    pub mode: u8,
    /// Expected chip ID.
    #[serde(default)]
    pub chip_id: Option<u16>,
    /// MII PHY addresses to read back after applying auto-negotiation.
    #[serde(default = "default_phys")]
    pub phys: Vec<u8>,
    /// PHY addresses on which to enable auto-negotiation and auto-MDIX.
    pub target_phys: Vec<u8>,
}

/// Request body for `POST /api/phy/write`.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct PhyWriteRequest {
    /// spidev path.
    #[serde(default)]
    pub device: Option<String>,
    /// Use simulated LAN9252 (requires `--sim-pdi`).
    #[serde(default)]
    pub sim: bool,
    /// SPI clock frequency, Hz.
    #[serde(default = "default_phy_clock")]
    pub clock_hz: u32,
    /// SPI mode (`0` or `3`).
    #[serde(default)]
    pub mode: u8,
    /// Expected chip ID.
    #[serde(default)]
    pub chip_id: Option<u16>,
    /// MII PHY addresses to read back after writing.
    #[serde(default = "default_phys")]
    pub phys: Vec<u8>,
    /// Target PHY address (`0..=31`).
    pub phy: u8,
    /// Target MII register (`0..=31`).
    pub reg: u8,
    /// 16-bit value to write.
    pub value: u16,
}

/// One MII PHY register readout.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct PhyRegisterView {
    /// Register index (`0..=31`).
    pub reg: u8,
    /// LAN9252 datasheet name of the register.
    pub name: String,
    /// Raw 16-bit register value.
    pub value: u16,
}

/// Decoded status of one Ethernet PHY on the LAN9252.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct PhyPortView {
    /// MII PHY address.
    pub phy: u8,
    /// Whether a PHY answered at this address.
    pub present: bool,
    /// Basic status bit 2.
    pub link_up: bool,
    /// Basic control bit 12.
    pub autoneg_enabled: bool,
    /// Basic status bit 5.
    pub autoneg_done: bool,
    /// True for 100 Mbit/s, false for 10 Mbit/s.
    pub speed_100: bool,
    /// True for full duplex, false for half duplex.
    pub full_duplex: bool,
    /// Register 27 bit 15: MDI/MDI-X forced via register instead of strap.
    pub mdix_override: bool,
    /// Register 27 bit 14: auto-MDIX enabled.
    pub auto_mdix: bool,
    /// Register 27 bit 13: MDI-X crossover state.
    pub mdix: bool,
    /// Register 18 bits 7:5.
    pub mode: u8,
    /// Human-readable one-line status summary.
    pub summary: String,
    /// Raw registers read from the PHY.
    pub registers: Vec<PhyRegisterView>,
}

/// Response of `/api/phy/*` endpoints.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct PhyReport {
    /// Provenance (`measured` or `simulated`).
    pub provenance: Provenance,
    /// Target description.
    pub target: String,
    /// ESC link check details.
    pub link: LinkInfoView,
    /// Decoded PHY port entries.
    pub ports: Vec<PhyPortView>,
}

/// Request body for `POST /api/pdi-bench/run`.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct PdiBenchRequest {
    /// spidev path.
    #[serde(default)]
    pub device: Option<String>,
    /// Use simulated LAN9252 (requires `--sim-pdi`).
    #[serde(default)]
    pub sim: bool,
    /// SPI clock frequency, Hz.
    #[serde(default = "default_phy_clock")]
    pub clock_hz: u32,
    /// SPI mode (`0` or `3`).
    #[serde(default)]
    pub mode: u8,
    /// SPI read opcode.
    #[serde(default)]
    pub read_instruction: SlaveReadInstruction,
    /// SPI tuning (`fast`, `stack`, or comma-separated `split,batch,fifo,noabort`).
    #[serde(default = "default_fast_tuning")]
    pub spi_tuning: String,
    /// Access sizes to time, bytes.
    #[serde(default = "default_bench_sizes")]
    pub sizes: Vec<u32>,
    /// Timed repetitions per size and direction (`1..=500`).
    #[serde(default = "default_bench_reps")]
    pub reps: u32,
    /// Untimed warm-up accesses per size and direction (`0..=50`).
    #[serde(default = "default_bench_warmup")]
    pub warmup: u32,
    /// Process RAM area (`START+LEN`, e.g. `0x1800+1024`). Required for writes.
    #[serde(default)]
    pub area: Option<String>,
    /// Include writes into `area` (requires `--allow-writes` on real hardware).
    #[serde(default)]
    pub allow_writes: bool,
    /// Accept non-INIT AL state when slave app is stopped (reads only, explicit `area`).
    #[serde(default)]
    pub app_stopped: bool,
    /// Skip reading AL Event Request after every access.
    #[serde(default)]
    pub no_al_event: bool,
    /// Expected chip ID.
    #[serde(default)]
    pub chip_id: Option<u16>,
    /// Shuffle RNG seed.
    #[serde(default = "default_bench_seed")]
    pub seed: u64,
    /// Fixed transaction cost when `sim == true`, nanoseconds.
    #[serde(default = "default_sim_tx_ns")]
    pub sim_transaction_ns: u64,
}

/// Summary timing statistics for one direction and size.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct BenchSizeSummaryView {
    /// `"read"` or `"write"`.
    pub direction: String,
    /// Access size, bytes.
    pub bytes: u32,
    /// Number of timed samples.
    pub samples: u64,
    /// Median duration, nanoseconds.
    pub median_ns: f64,
    /// 95th percentile duration, nanoseconds.
    pub p95_ns: f64,
    /// Minimum duration, nanoseconds.
    pub min_ns: f64,
}

/// Fit residual at one access size.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct PdiSizeResidualView {
    /// Access size, bytes.
    pub bytes: u32,
    /// Sample count.
    pub samples: u64,
    /// Measured median, ns.
    pub median_ns: f64,
    /// Measured p95, ns.
    pub p95_ns: f64,
    /// Fitted line value, ns.
    pub fitted_ns: f64,
    /// `(median - fitted) / fitted`.
    pub relative_error: f64,
}

/// Linear regression fit for one PDI direction.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct PdiLineFitView {
    /// Total samples in this direction.
    pub samples: u64,
    /// Fixed per-access overhead (intercept), ns.
    pub overhead_ns: f64,
    /// Effective byte throughput converted to bit/s.
    pub rate_bps: f64,
    /// Largest absolute relative error over all sizes.
    pub max_relative_error: f64,
    /// Per-size residuals.
    pub sizes: Vec<PdiSizeResidualView>,
}

/// Fitted PDI parameters ready to apply to the simulator (`SimSlave.pdi`).
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct PdiValuesView {
    /// Effective read rate, bit/s.
    pub read_bps: u64,
    /// Effective write rate, bit/s.
    pub write_bps: u64,
    /// Fixed per-access overhead, ns.
    pub access_overhead_ns: u32,
}

/// Full `flowlab pdi-fit` result returned by `POST /api/pdi-bench/run`.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct PdiFitView {
    /// Linear fit for reads, if timed.
    pub read: Option<PdiLineFitView>,
    /// Linear fit for writes, if timed.
    pub write: Option<PdiLineFitView>,
    /// Simulator PDI fields derived from the fit.
    pub pdi: PdiValuesView,
    /// Fit assumptions.
    pub assumptions: Vec<String>,
}

/// Response of `POST /api/pdi-bench/run`.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct PdiBenchResult {
    /// Provenance (`measured` or `simulated`).
    pub provenance: Provenance,
    /// Target description.
    pub target: String,
    /// Link check details.
    pub link: LinkInfoView,
    /// SPI clock frequency used, Hz.
    pub clock_hz: u32,
    /// SPI mode used.
    pub mode: u8,
    /// SPI read instruction name.
    pub read_instruction: String,
    /// SPI tuning name.
    pub spi_tuning: String,
    /// ESC read area formatted as `0xSTART+LEN`.
    pub read_area: String,
    /// ESC write area formatted as `0xSTART+LEN`, if writes ran.
    pub write_area: Option<String>,
    /// Per-direction and per-size summary statistics.
    pub summaries: Vec<BenchSizeSummaryView>,
    /// Linear PDI model fit (present when >= 2 sizes were timed).
    pub fit: Option<PdiFitView>,
    /// Explanation if model fit could not run.
    pub fit_error: Option<String>,
    /// Exportable CSV samples file compatible with `flowlab pdi-fit`.
    pub csv: String,
}

struct RunningSlave {
    stop: Arc<AtomicBool>,
    telemetry: Arc<Mutex<SlaveStatus>>,
    handle: Option<JoinHandle<()>>,
}

#[derive(Default)]
enum BusPhase {
    #[default]
    Idle,
    SlaveRunning(RunningSlave),
    Busy(&'static str),
}

#[derive(Default)]
struct HardwareState {
    phase: BusPhase,
    last_slave: Option<SlaveStatus>,
}

/// Exclusive coordinator for SPI hardware operations on `/dev/spidev*` or `SimLan9252`.
#[derive(Clone, Default)]
pub(crate) struct HardwareSlot {
    inner: Arc<Mutex<HardwareState>>,
}

struct BusGuard {
    slot: HardwareSlot,
    armed: bool,
}

impl BusGuard {
    fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for BusGuard {
    fn drop(&mut self) {
        if self.armed
            && let Ok(mut st) = self.slot.inner.lock()
            && matches!(st.phase, BusPhase::Busy(_))
        {
            st.phase = BusPhase::Idle;
        }
    }
}

impl HardwareSlot {
    fn acquire_busy(&self, op: &'static str) -> Result<BusGuard, ApiError> {
        let mut st = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        match &st.phase {
            BusPhase::Idle => {
                st.phase = BusPhase::Busy(op);
                Ok(BusGuard { slot: self.clone(), armed: true })
            }
            BusPhase::SlaveRunning(_) => {
                Err(ApiError::conflict(format!("PDI slave is running on the SPI bus — stop it before running {op}")))
            }
            BusPhase::Busy(other) => Err(ApiError::conflict(format!("SPI bus is busy ({other}); try again shortly"))),
        }
    }

    fn status(&self) -> SlaveStatus {
        let st = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        match &st.phase {
            BusPhase::SlaveRunning(running) => running.telemetry.lock().unwrap_or_else(|e| e.into_inner()).clone(),
            _ => st.last_slave.clone().unwrap_or_default(),
        }
    }

    fn stop_running_blocking(&self) -> Result<Option<SlaveStatus>, ApiError> {
        let taken = {
            let mut st = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            match std::mem::take(&mut st.phase) {
                BusPhase::SlaveRunning(running) => {
                    st.phase = BusPhase::Busy("stopping PDI slave");
                    running
                }
                other => {
                    st.phase = other;
                    return Ok(None);
                }
            }
        };
        let guard = BusGuard { slot: self.clone(), armed: true };
        taken.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = taken.handle {
            let _ = handle.join();
        }
        let mut final_status = taken.telemetry.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if final_status.state == SlaveRunState::Running {
            final_status.state = SlaveRunState::Stopped;
        }
        let mut st = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        st.last_slave = Some(final_status.clone());
        st.phase = BusPhase::Idle;
        guard.disarm();
        Ok(Some(final_status))
    }

    /// Stop any running PDI slave cleanly during server shutdown so AL Status returns to `INIT`.
    pub(crate) fn shutdown(&self) {
        let _ = self.stop_running_blocking();
    }
}

fn map_pdi_error(err: PdiError) -> ApiError {
    match err {
        PdiError::Refused(m) | PdiError::Options(m) | PdiError::Unsupported(m) | PdiError::Link(m) => {
            ApiError::bad_request(m)
        }
        PdiError::Spi(e) => ApiError::bad_request(format!("SPI: {e}")),
        PdiError::Timeout { what, polls } => ApiError::bad_request(format!("timed out after {polls} polls: {what}")),
        other => ApiError::bad_request(other.to_string()),
    }
}

fn resolve_device_path(cfg: &HardwareConfig, req_device: Option<&str>) -> Result<PathBuf, ApiError> {
    if let Some(d) = req_device.map(str::trim).filter(|s| !s.is_empty()) {
        return Ok(PathBuf::from(d));
    }
    if let Some(d) = &cfg.device {
        return Ok(d.clone());
    }
    if let Some(first) = discover_spidev().into_iter().next() {
        return Ok(PathBuf::from(first));
    }
    Err(ApiError::bad_request(
        "no spidev device specified and none discovered under /dev/spidev*; pass `device` or start with `--sim-pdi`",
    ))
}

struct OpenedSpi {
    bus: Box<dyn SpiBus>,
    clock: Box<dyn Clock>,
    target: String,
    provenance: Provenance,
}

fn open_spi_bus(
    cfg: &HardwareConfig,
    device: Option<&str>,
    sim: bool,
    clock_hz: u32,
    mode: u8,
    sim_transaction_ns: u64,
) -> Result<OpenedSpi, ApiError> {
    if mode > 3 {
        return Err(ApiError::bad_request(format!("SPI mode {mode} must be 0 to 3")));
    }
    if clock_hz == 0 {
        return Err(ApiError::bad_request("SPI clock_hz must be positive"));
    }
    if sim {
        if !cfg.sim_pdi {
            return Err(ApiError::forbidden("simulated LAN9252 mode is disabled; start the server with `--sim-pdi`"));
        }
        let timing =
            SimTiming { clock_hz: u64::from(clock_hz), transaction_ns: sim_transaction_ns, ..SimTiming::default() };
        let (bus, clock) = SimLan9252::new(timing);
        return Ok(OpenedSpi {
            bus: Box::new(bus),
            clock: Box::new(clock),
            target: format!("simulated LAN9252 @ {:.2} MHz", f64::from(clock_hz) / 1e6),
            provenance: Provenance::Simulated,
        });
    }
    let path = resolve_device_path(cfg, device)?;
    let bus = open_spidev(&path, clock_hz, mode).map_err(map_pdi_error)?;
    let target = format!("{} @ {:.2} MHz (mode {mode})", path.display(), f64::from(clock_hz) / 1e6);
    Ok(OpenedSpi { bus, clock: Box::new(MonotonicClock::new()), target, provenance: Provenance::Measured })
}

fn cap_push<T>(vec: &mut Vec<T>, item: T, max: usize) {
    if vec.len() >= max {
        vec.remove(0);
    }
    vec.push(item);
}

fn record_log(log: ResponderLog, t_s: f64, out: &mut Vec<ReceivedPayload>) {
    for (name, data) in log.files {
        cap_push(
            out,
            ReceivedPayload {
                t_s,
                protocol: "FoE".to_owned(),
                name: Some(name),
                bytes: data.len() as u64,
                sha256: flowlab_core::sha256_hex(&data),
            },
            MAX_EVENT_LOG,
        );
    }
    for data in log.objects {
        cap_push(
            out,
            ReceivedPayload {
                t_s,
                protocol: "CoE SDO".to_owned(),
                name: None,
                bytes: data.len() as u64,
                sha256: flowlab_core::sha256_hex(&data),
            },
            MAX_EVENT_LOG,
        );
    }
    for data in log.voe_received {
        cap_push(
            out,
            ReceivedPayload {
                t_s,
                protocol: "FlowLab VoE".to_owned(),
                name: None,
                bytes: data.len() as u64,
                sha256: flowlab_core::sha256_hex(&data),
            },
            MAX_EVENT_LOG,
        );
    }
    if !log.eoe_frames.is_empty() {
        let total: u64 = log.eoe_frames.iter().map(|f| f.len() as u64).sum();
        cap_push(
            out,
            ReceivedPayload {
                t_s,
                protocol: "EoE".to_owned(),
                name: Some(format!("{} frame(s)", log.eoe_frames.len())),
                bytes: total,
                sha256: flowlab_core::sha256_hex(log.eoe_frames.last().map_or(&[], Vec::as_slice)),
            },
            MAX_EVENT_LOG,
        );
    }
}

fn record_al_change(c: AlChange, t_s: f64, st: &mut SlaveStatus) {
    st.al_state = Some(c.state.name().to_owned());
    st.al_error = c.error;
    st.mailboxes = c.mailboxes.map(MailboxesView::from);
    cap_push(
        &mut st.al_events,
        SlaveAlEvent {
            t_s,
            state: c.state.name().to_owned(),
            error: c.error,
            mailboxes: c.mailboxes.map(MailboxesView::from),
        },
        MAX_EVENT_LOG,
    );
}

#[cfg(all(feature = "slave-wire", target_os = "linux"))]
mod eoe_linux {
    use flowlab_mailbox::msg::EoeIpParameters;
    use flowlab_slave::bridge::{BridgeStats, EoeBridge, interface_mac};
    use flowlab_slave::pdi::{EscPdi, PdiSlave};
    use flowlab_slave::wire::EthernetPort;

    use crate::ApiError;

    pub(super) struct ActiveBridge {
        bridge: EoeBridge<EthernetPort>,
        pub(super) bridge_iface: String,
        pub(super) host_iface: String,
        pub(super) last_set_ip: Option<String>,
    }

    pub(super) fn open_bridge(bridge_iface: &str, host_iface: &str) -> Result<ActiveBridge, ApiError> {
        let port = EthernetPort::open(bridge_iface).map_err(|e| ApiError::bad_request(e.to_string()))?;
        let mac = interface_mac(host_iface).map_err(|e| ApiError::bad_request(e.to_string()))?;
        Ok(ActiveBridge {
            bridge: EoeBridge::new(port, mac),
            bridge_iface: bridge_iface.to_owned(),
            host_iface: host_iface.to_owned(),
            last_set_ip: None,
        })
    }

    fn apply_set_ip(name: &str, p: &EoeIpParameters) -> Result<String, String> {
        let ip = |args: &[&str]| -> Result<(), String> {
            let out = std::process::Command::new("ip").args(args).output().map_err(|e| format!("ip: {e}"))?;
            if !out.status.success() {
                return Err(format!("ip {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()));
            }
            Ok(())
        };
        if let Some(m) = p.mac {
            let mac = m.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":");
            ip(&["link", "set", "dev", name, "down"])?;
            ip(&["link", "set", "dev", name, "address", &mac])?;
        }
        let mut summary = String::new();
        if let Some(a) = p.ip {
            let prefix = p.subnet_mask.map_or(24, |m| u32::from_be_bytes(m).count_ones());
            let addr = format!("{}.{}.{}.{}/{prefix}", a[0], a[1], a[2], a[3]);
            ip(&["addr", "flush", "dev", name])?;
            ip(&["addr", "add", &addr, "dev", name])?;
            summary = addr;
        }
        ip(&["link", "set", "dev", name, "up"])?;
        Ok(if summary.is_empty() { "link updated".to_owned() } else { summary })
    }

    impl ActiveBridge {
        pub(super) fn pump<E: EscPdi>(&mut self, slave: &mut PdiSlave<E>) -> Result<(), String> {
            let ips = self.bridge.pump(slave).map_err(|e| e.to_string())?;
            for p in ips {
                let text = apply_set_ip(&self.host_iface, &p)?;
                let mac = interface_mac(&self.host_iface).map_err(|e| e.to_string())?;
                self.bridge.set_host_mac(mac);
                self.last_set_ip = Some(text);
            }
            Ok(())
        }

        pub(super) fn stats(&self) -> BridgeStats {
            self.bridge.stats()
        }
    }
}

#[cfg(not(all(feature = "slave-wire", target_os = "linux")))]
mod eoe_linux {
    use flowlab_slave::pdi::{EscPdi, PdiSlave};

    use crate::ApiError;

    pub(super) struct ActiveBridge {
        pub(super) bridge_iface: String,
        pub(super) host_iface: String,
        pub(super) last_set_ip: Option<String>,
    }

    #[derive(Clone, Copy, Default)]
    pub(super) struct BridgeStats {
        pub(super) to_host: u64,
        pub(super) to_host_bytes: u64,
        pub(super) to_master: u64,
        pub(super) to_master_bytes: u64,
    }

    pub(super) fn open_bridge(_bridge_iface: &str, _host_iface: &str) -> Result<ActiveBridge, ApiError> {
        Err(ApiError::bad_request("EoE bridging requires a Linux build with `--features slave`"))
    }

    impl ActiveBridge {
        pub(super) fn pump<E: EscPdi>(&mut self, _slave: &mut PdiSlave<E>) -> Result<(), String> {
            Ok(())
        }

        pub(super) fn stats(&self) -> BridgeStats {
            BridgeStats::default()
        }
    }
}

fn bridge_view(b: &eoe_linux::ActiveBridge) -> BridgeStatsView {
    let st = b.stats();
    BridgeStatsView {
        bridge_iface: b.bridge_iface.clone(),
        host_iface: b.host_iface.clone(),
        to_host: st.to_host,
        to_host_bytes: st.to_host_bytes,
        to_master: st.to_master,
        to_master_bytes: st.to_master_bytes,
        last_set_ip: b.last_set_ip.clone(),
    }
}

fn run_slave_loop(
    mut slave: PdiSlave<SlaveAccess<Box<dyn SpiBus>>>,
    mut bridge: Option<eoe_linux::ActiveBridge>,
    sim: bool,
    stop: &Arc<AtomicBool>,
    telemetry: &Arc<Mutex<SlaveStatus>>,
    slot: &HardwareSlot,
) {
    let origin = Instant::now();
    let mut last_publish = Instant::now();
    let mut last_rate = Instant::now();
    let mut prev_stats = slave.stats();
    let mut prev_bridge_host = bridge.as_ref().map_or(0, |b| b.stats().to_host);
    let mut prev_bridge_master = bridge.as_ref().map_or(0, |b| b.stats().to_master);

    let err_msg: Option<String> = loop {
        if stop.load(Ordering::Relaxed) {
            break None;
        }
        let (change, moved) = match slave.step() {
            Ok(step) => step,
            Err(e) => break Some(e.to_string()),
        };
        if let Some(b) = bridge.as_mut()
            && let Err(e) = b.pump(&mut slave)
        {
            break Some(e);
        }
        let now = Instant::now();
        let do_rate = last_rate.elapsed() >= Duration::from_secs(1);
        let do_publish = change.is_some() || do_rate || last_publish.elapsed() >= Duration::from_millis(250);
        if do_publish {
            let t_s = origin.elapsed().as_secs_f64();
            let cur_stats = slave.stats();
            let log = slave.drain_log();
            let mut snap = telemetry.lock().unwrap_or_else(|e| e.into_inner());
            snap.elapsed_s = t_s;
            snap.stats = cur_stats.into();
            if let Some(c) = change {
                record_al_change(c, t_s, &mut snap);
            }
            record_log(log, t_s, &mut snap.received);
            if let Some(b) = bridge.as_ref() {
                snap.bridge = Some(bridge_view(b));
            }
            if do_rate {
                let dt = last_rate.elapsed().as_secs_f64().max(1e-3);
                let b_host = bridge.as_ref().map_or(0, |b| b.stats().to_host);
                let b_master = bridge.as_ref().map_or(0, |b| b.stats().to_master);
                cap_push(
                    &mut snap.series,
                    SlaveRatePoint {
                        t_s,
                        polls_s: cur_stats.polls.saturating_sub(prev_stats.polls) as f64 / dt,
                        msg_in_s: cur_stats.messages_in.saturating_sub(prev_stats.messages_in) as f64 / dt,
                        msg_out_s: cur_stats.messages_out.saturating_sub(prev_stats.messages_out) as f64 / dt,
                        bytes_in_s: cur_stats.bytes_in.saturating_sub(prev_stats.bytes_in) as f64 / dt,
                        bytes_out_s: cur_stats.bytes_out.saturating_sub(prev_stats.bytes_out) as f64 / dt,
                        eoe_to_host_s: b_host.saturating_sub(prev_bridge_host) as f64 / dt,
                        eoe_to_master_s: b_master.saturating_sub(prev_bridge_master) as f64 / dt,
                    },
                    MAX_SERIES_POINTS,
                );
                prev_stats = cur_stats;
                prev_bridge_host = b_host;
                prev_bridge_master = b_master;
                last_rate = now;
            }
            last_publish = now;
        }
        if sim && !moved {
            std::thread::sleep(Duration::from_millis(1));
        }
    };

    let stop_res = slave.stop();
    let final_log = slave.drain_log();
    let final_stats = slave.stats();
    let (al_state, al_error) = slave.al_state();
    let final_status = {
        let mut snap = telemetry.lock().unwrap_or_else(|e| e.into_inner());
        let t_s = origin.elapsed().as_secs_f64();
        snap.elapsed_s = t_s;
        snap.stats = final_stats.into();
        snap.al_state = Some(al_state.name().to_owned());
        snap.al_error = al_error;
        snap.mailboxes = None;
        record_log(final_log, t_s, &mut snap.received);
        if let Some(b) = bridge.as_ref() {
            snap.bridge = Some(bridge_view(b));
        }
        let combined_err = err_msg.or_else(|| stop_res.err().map(|e| e.to_string()));
        if let Some(err) = combined_err {
            snap.state = SlaveRunState::Failed;
            snap.error = Some(err);
        } else {
            snap.state = SlaveRunState::Stopped;
            snap.error = None;
        }
        snap.clone()
    };

    // If the thread exited on its own (SPI error), free the hardware slot here.
    // If `stop_running_blocking` already claimed the handle, it sets `Idle` after joining.
    let mut hw = slot.inner.lock().unwrap_or_else(|e| e.into_inner());
    if matches!(&hw.phase, BusPhase::SlaveRunning(r) if Arc::ptr_eq(&r.stop, stop)) {
        hw.last_slave = Some(final_status);
        hw.phase = BusPhase::Idle;
    }
}

pub(crate) async fn slave_status(State(s): State<AppState>) -> Json<SlaveStatus> {
    Json(s.hardware.status())
}

struct InitializedSlave {
    slave: PdiSlave<SlaveAccess<Box<dyn SpiBus>>>,
    bridge: Option<eoe_linux::ActiveBridge>,
    initial: SlaveStatus,
}

fn init_slave_on_thread(
    cfg: &HardwareConfig,
    req: &SlaveStartRequest,
    tuning: SpiTuning,
    bridge_pair: Option<(String, String)>,
    chip_id: u16,
) -> Result<InitializedSlave, ApiError> {
    let spi = open_spi_bus(cfg, req.device.as_deref(), req.sim, req.clock_hz, req.mode, 5_000)?;
    let bridge = match bridge_pair {
        Some((b, h)) => Some(eoe_linux::open_bridge(&b, &h)?),
        None => None,
    };
    let esc = Lan9252::new(spi.bus).with_read_instruction(req.read_instruction.into()).with_tuning(tuning);
    let esc = SlaveAccess::take_over(esc, chip_id).map_err(map_pdi_error)?;
    let link = esc.link();
    let mut slave_cfg = PdiSlaveConfig {
        max_mailbox_bytes: req.max_mailbox_bytes,
        answer_set_ip: bridge.is_some(),
        message_only: req.message_only,
        ..PdiSlaveConfig::default()
    };
    if req.upload_bytes > 0 {
        slave_cfg.responder.upload = flowlab_mailbox::payload(req.upload_seed, req.upload_bytes as usize);
    }
    let slave = PdiSlave::new(esc, slave_cfg).map_err(map_pdi_error)?;
    let initial = SlaveStatus {
        state: SlaveRunState::Running,
        provenance: Some(spi.provenance),
        target: Some(spi.target),
        spi_tuning: Some(tuning.name()),
        clock_hz: Some(req.clock_hz),
        max_mailbox_bytes: Some(req.max_mailbox_bytes),
        message_only: Some(req.message_only),
        elapsed_s: 0.0,
        link: Some(link.into()),
        al_state: Some(AlState::Init.name().to_owned()),
        al_error: None,
        mailboxes: None,
        stats: SlaveStatsView::default(),
        bridge: bridge.as_ref().map(bridge_view),
        series: Vec::new(),
        al_events: vec![SlaveAlEvent {
            t_s: 0.0,
            state: AlState::Init.name().to_owned(),
            error: None,
            mailboxes: None,
        }],
        received: Vec::new(),
        error: None,
    };
    Ok(InitializedSlave { slave, bridge, initial })
}

pub(crate) async fn slave_start(
    State(s): State<AppState>,
    Json(req): Json<SlaveStartRequest>,
) -> Result<Json<SlaveStatus>, ApiError> {
    if !req.sim && !s.hardware_config.allow_replace_app {
        return Err(ApiError::forbidden(
            "taking over a real ESC requires starting the server with `--replace-slave-application`",
        ));
    }
    if req.max_mailbox_bytes < MIN_MAILBOX_BYTES {
        return Err(ApiError::bad_request(format!(
            "max_mailbox_bytes must be at least {MIN_MAILBOX_BYTES} (got {})",
            req.max_mailbox_bytes
        )));
    }
    if req.upload_bytes > MAX_UPLOAD_PAYLOAD_BYTES {
        return Err(ApiError::bad_request(format!(
            "upload_bytes ({}) exceeds maximum {MAX_UPLOAD_PAYLOAD_BYTES}",
            req.upload_bytes
        )));
    }
    let tuning = SpiTuning::parse(&req.spi_tuning).map_err(map_pdi_error)?;
    let bridge_pair = match (
        req.eoe_bridge.as_deref().map(str::trim).filter(|v| !v.is_empty()),
        req.eoe_host.as_deref().map(str::trim).filter(|v| !v.is_empty()),
    ) {
        (Some(b), Some(h)) => Some((b.to_owned(), h.to_owned())),
        (None, None) => None,
        _ => return Err(ApiError::bad_request("both `eoe_bridge` and `eoe_host` are required together")),
    };

    let guard = s.hardware.acquire_busy("starting PDI slave")?;
    let cfg = s.hardware_config.clone();
    let slot = s.hardware.clone();
    let chip_id = req.chip_id.unwrap_or(cfg.chip_id);

    let status = blocking(move || {
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = std::sync::mpsc::sync_channel::<Result<(SlaveStatus, Arc<Mutex<SlaveStatus>>), ApiError>>(1);
        let stop_t = Arc::clone(&stop);
        let slot_t = slot.clone();
        let sim = req.sim;

        let handle = std::thread::Builder::new()
            .name("flowlab-pdi-slave".to_owned())
            .spawn(move || match init_slave_on_thread(&cfg, &req, tuning, bridge_pair, chip_id) {
                Ok(init) => {
                    let telemetry = Arc::new(Mutex::new(init.initial.clone()));
                    let _ = tx.send(Ok((init.initial, Arc::clone(&telemetry))));
                    run_slave_loop(init.slave, init.bridge, sim, &stop_t, &telemetry, &slot_t);
                }
                Err(e) => {
                    let _ = tx.send(Err(e));
                }
            })
            .map_err(|e| ApiError::internal(format!("spawn PDI slave thread: {e}")))?;

        let (initial, telemetry) = match rx.recv() {
            Ok(Ok(pair)) => pair,
            Ok(Err(e)) => {
                let _ = handle.join();
                return Err(e);
            }
            Err(_) => {
                let _ = handle.join();
                return Err(ApiError::internal("PDI slave thread exited during startup"));
            }
        };

        let mut st = slot.inner.lock().unwrap_or_else(|e| e.into_inner());
        st.phase = BusPhase::SlaveRunning(RunningSlave { stop, telemetry, handle: Some(handle) });
        guard.disarm();
        Ok(initial)
    })
    .await?;

    Ok(Json(status))
}

pub(crate) async fn slave_stop(State(s): State<AppState>) -> Result<Json<SlaveStatus>, ApiError> {
    let slot = s.hardware.clone();
    let status =
        blocking(move || slot.stop_running_blocking()?.ok_or_else(|| ApiError::conflict("PDI slave is not running")))
            .await?;
    Ok(Json(status))
}

fn phy_reg_name(reg: u8) -> &'static str {
    match reg {
        0 => "Basic Control",
        1 => "Basic Status",
        2 => "PHY ID 1",
        3 => "PHY ID 2",
        4 => "Auto-Neg Advertisement",
        5 => "Auto-Neg Link Partner",
        18 => "Special Modes",
        27 => "Special Control/Status Ind.",
        31 => "PHY Special Control/Status",
        _ => "Register",
    }
}

fn decode_phy_ports<S: SpiBus>(phys: &mut PhyAccess<S>, indices: &[u8]) -> Result<Vec<PhyPortView>, PdiError> {
    let mut ports = Vec::with_capacity(indices.len());
    for &phy in indices {
        if phy > 31 {
            return Err(PdiError::Options(format!("PHY address {phy} must be 0 to 31")));
        }
        match phys.status(phy)? {
            None => ports.push(PhyPortView {
                phy,
                present: false,
                link_up: false,
                autoneg_enabled: false,
                autoneg_done: false,
                speed_100: false,
                full_duplex: false,
                mdix_override: false,
                auto_mdix: false,
                mdix: false,
                mode: 0,
                summary: "none".to_owned(),
                registers: Vec::new(),
            }),
            Some(p) => ports.push(phy_port_view(p)),
        }
    }
    Ok(ports)
}

fn phy_port_view(p: PhyStatus) -> PhyPortView {
    let summary = p.summary();
    let registers = p
        .registers
        .into_iter()
        .map(|(reg, value)| PhyRegisterView { reg, name: phy_reg_name(reg).to_owned(), value })
        .collect();
    PhyPortView {
        phy: p.phy,
        present: true,
        link_up: p.link_up,
        autoneg_enabled: p.autoneg_enabled,
        autoneg_done: p.autoneg_done,
        speed_100: p.speed_100,
        full_duplex: p.full_duplex,
        mdix_override: p.mdix_override,
        auto_mdix: p.auto_mdix,
        mdix: p.mdix,
        mode: p.mode,
        summary,
        registers,
    }
}

pub(crate) async fn phy_read(
    State(s): State<AppState>,
    Json(req): Json<PhyReadRequest>,
) -> Result<Json<PhyReport>, ApiError> {
    let _guard = s.hardware.acquire_busy("PHY read")?;
    let cfg = s.hardware_config.clone();
    let chip_id = req.chip_id.unwrap_or(cfg.chip_id);
    let report = blocking(move || {
        let spi = open_spi_bus(&cfg, req.device.as_deref(), req.sim, req.clock_hz, req.mode, 5_000)?;
        let mut access = PhyAccess::open(Lan9252::new(spi.bus), chip_id).map_err(map_pdi_error)?;
        let link = access.link().into();
        let ports = decode_phy_ports(&mut access, &req.phys).map_err(map_pdi_error)?;
        Ok(PhyReport { provenance: spi.provenance, target: spi.target, link, ports })
    })
    .await?;
    Ok(Json(report))
}

pub(crate) async fn phy_autoneg(
    State(s): State<AppState>,
    Json(req): Json<PhyAutonegRequest>,
) -> Result<Json<PhyReport>, ApiError> {
    if !req.sim && !s.hardware_config.allow_phy_writes {
        return Err(ApiError::forbidden("PHY writes require starting the server with `--allow-phy-writes`"));
    }
    if req.target_phys.is_empty() {
        return Err(ApiError::bad_request("target_phys must name at least one PHY"));
    }
    let _guard = s.hardware.acquire_busy("PHY autoneg")?;
    let cfg = s.hardware_config.clone();
    let chip_id = req.chip_id.unwrap_or(cfg.chip_id);
    let report = blocking(move || {
        let spi = open_spi_bus(&cfg, req.device.as_deref(), req.sim, req.clock_hz, req.mode, 5_000)?;
        let mut access = PhyAccess::open(Lan9252::new(spi.bus), chip_id).map_err(map_pdi_error)?;
        for &phy in &req.target_phys {
            access.enable_autoneg(phy).map_err(map_pdi_error)?;
        }
        if !req.sim {
            std::thread::sleep(Duration::from_secs(3));
        }
        let link = access.link().into();
        let ports = decode_phy_ports(&mut access, &req.phys).map_err(map_pdi_error)?;
        Ok(PhyReport { provenance: spi.provenance, target: spi.target, link, ports })
    })
    .await?;
    Ok(Json(report))
}

pub(crate) async fn phy_write(
    State(s): State<AppState>,
    Json(req): Json<PhyWriteRequest>,
) -> Result<Json<PhyReport>, ApiError> {
    if !req.sim && !s.hardware_config.allow_phy_writes {
        return Err(ApiError::forbidden("PHY writes require starting the server with `--allow-phy-writes`"));
    }
    let _guard = s.hardware.acquire_busy("PHY write")?;
    let cfg = s.hardware_config.clone();
    let chip_id = req.chip_id.unwrap_or(cfg.chip_id);
    let report = blocking(move || {
        let spi = open_spi_bus(&cfg, req.device.as_deref(), req.sim, req.clock_hz, req.mode, 5_000)?;
        let mut access = PhyAccess::open(Lan9252::new(spi.bus), chip_id).map_err(map_pdi_error)?;
        access.write(req.phy, req.reg, req.value).map_err(map_pdi_error)?;
        let link = access.link().into();
        let ports = decode_phy_ports(&mut access, &req.phys).map_err(map_pdi_error)?;
        Ok(PhyReport { provenance: spi.provenance, target: spi.target, link, ports })
    })
    .await?;
    Ok(Json(report))
}

pub(crate) async fn pdi_bench_run(
    State(s): State<AppState>,
    Json(req): Json<PdiBenchRequest>,
) -> Result<Json<PdiBenchResult>, ApiError> {
    if req.allow_writes && !req.sim && !s.hardware_config.allow_writes {
        return Err(ApiError::forbidden(
            "SPI write benchmarks on a real ESC require starting the server with `--allow-writes`",
        ));
    }
    if req.reps == 0 || req.reps > MAX_BENCH_REPS {
        return Err(ApiError::bad_request(format!("reps must be 1 to {MAX_BENCH_REPS}")));
    }
    if req.warmup > MAX_BENCH_WARMUP {
        return Err(ApiError::bad_request(format!("warmup must be 0 to {MAX_BENCH_WARMUP}")));
    }
    let tuning = SpiTuning::parse(&req.spi_tuning).map_err(map_pdi_error)?;
    let area = req
        .area
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(Area::from_str)
        .transpose()
        .map_err(ApiError::bad_request)?;
    let options = BenchOptions {
        sizes: req.sizes,
        reps: req.reps,
        warmup: req.warmup,
        area,
        allow_writes: req.allow_writes,
        app_stopped: req.app_stopped,
        seed: req.seed,
    };
    let _guard = s.hardware.acquire_busy("SPI benchmark")?;
    let cfg = s.hardware_config.clone();
    let chip_id = req.chip_id.unwrap_or(cfg.chip_id);

    let result = blocking(move || {
        let instruction: ReadInstruction = req.read_instruction.into();
        let spi = open_spi_bus(&cfg, req.device.as_deref(), req.sim, req.clock_hz, req.mode, req.sim_transaction_ns)?;
        let mut esc = Lan9252::new(spi.bus)
            .with_read_instruction(instruction)
            .with_al_event_read(!req.no_al_event)
            .with_tuning(tuning);
        let bench = run_pdi_bench(&mut esc, &*spi.clock, &options, chip_id).map_err(map_pdi_error)?;

        let comments = [
            format!("flowlab pdi-bench {}: {}", env!("CARGO_PKG_VERSION"), spi.target),
            format!(
                "LAN9252 ID_REV 0x{:08X}, AL Status {}; SPI {} Hz, mode {}, {}; AL Event read after every access: {}; SPI tuning {}",
                bench.link.id_rev,
                bench.link.al_state(),
                req.clock_hz,
                req.mode,
                instruction.name(),
                if req.no_al_event { "no" } else { "yes" },
                tuning.name()
            ),
            format!(
                "reads at {}, writes {}; {} timed after {} warm-up per direction and size, shuffled (seed {})",
                bench.plan.read_area,
                bench.plan.write_area.map_or("not allowed".to_owned(), |w| format!("at {w}")),
                req.reps,
                req.warmup,
                req.seed
            ),
        ];
        let csv = pdi_to_csv(&bench.samples, &comments, spi.provenance);
        let summaries = summarise_pdi_bench(&bench.samples)
            .into_iter()
            .map(|x| BenchSizeSummaryView {
                direction: x.direction.as_str().to_owned(),
                bytes: x.bytes,
                samples: x.samples,
                median_ns: x.median_ns,
                p95_ns: x.p95_ns,
                min_ns: x.min_ns,
            })
            .collect();

        let (fit, fit_error) = match flowlab_study::fit_pdi(&bench.samples, "pdi-bench.csv", csv.as_bytes()) {
            Ok(f) => (Some(to_fit_view(f)), None),
            Err(e) => (None, Some(e.to_string())),
        };

        Ok(PdiBenchResult {
            provenance: spi.provenance,
            target: spi.target,
            link: bench.link.into(),
            clock_hz: req.clock_hz,
            mode: req.mode,
            read_instruction: instruction.name().to_owned(),
            spi_tuning: tuning.name(),
            read_area: bench.plan.read_area.to_string(),
            write_area: bench.plan.write_area.map(|a| a.to_string()),
            summaries,
            fit,
            fit_error,
            csv,
        })
    })
    .await?;

    Ok(Json(result))
}

fn to_line_fit_view(l: flowlab_study::lab::LineFit) -> PdiLineFitView {
    PdiLineFitView {
        samples: l.samples,
        overhead_ns: l.overhead_ns,
        rate_bps: l.rate_bps,
        max_relative_error: l.max_relative_error,
        sizes: l
            .sizes
            .into_iter()
            .map(|r| PdiSizeResidualView {
                bytes: r.bytes,
                samples: r.samples,
                median_ns: r.median_ns,
                p95_ns: r.p95_ns,
                fitted_ns: r.fitted_ns,
                relative_error: r.relative_error,
            })
            .collect(),
    }
}

fn to_fit_view(f: flowlab_study::PdiFit) -> PdiFitView {
    PdiFitView {
        read: f.read.map(to_line_fit_view),
        write: f.write.map(to_line_fit_view),
        pdi: PdiValuesView {
            read_bps: f.pdi.read_bps,
            write_bps: f.pdi.write_bps,
            access_overhead_ns: f.pdi.access_overhead_ns,
        },
        assumptions: f.meta.assumptions,
    }
}
