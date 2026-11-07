//! Serializable result types. These are the JSON report schema and the UI's API types.

use flowlab_capture::{CaptureFormat, DirectionMode};
use flowlab_core::ReportMeta;
use serde::Serialize;
use ts_rs::TS;

use crate::stats::Distribution;

/// Full analysis of one capture (`flowlab analyze` output).
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct CaptureReport {
    /// Provenance, input identity, assumptions.
    pub meta: ReportMeta,
    /// Frame and datagram counts.
    pub summary: CaptureSummary,
    /// Detected cyclic streams, most frequent first.
    pub streams: Vec<CyclicStream>,
    /// Link utilization.
    pub utilization: Utilization,
    /// Stations seen with configured-address commands.
    pub stations: Vec<StationSummary>,
    /// AL state requests and reports, changes only.
    pub al_events: Vec<AlEvent>,
    /// Judgements against declared thresholds, and notes.
    pub findings: Vec<Finding>,
}

/// One command and how often it occurs.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct CommandCount {
    /// Mnemonic.
    pub command: String,
    /// Datagrams with this command.
    pub count: u64,
}

/// Counts over the whole capture.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct CaptureSummary {
    /// Container format.
    pub format: CaptureFormat,
    /// All frames.
    pub frames: u64,
    /// Well-formed EtherCAT frames.
    pub ethercat_frames: u64,
    /// Non-EtherCAT frames.
    pub other_frames: u64,
    /// Frames that claim EtherCAT but do not parse.
    pub malformed_frames: u64,
    /// Datagrams in EtherCAT frames.
    pub datagrams: u64,
    /// Time from first to last frame, ns.
    pub duration_ns: u64,
    /// Absolute time of the first frame, ms since the Unix epoch.
    pub start_unix_ms: f64,
    /// How directions were inferred.
    pub direction_mode: DirectionMode,
    /// Frames classified as TX.
    pub tx_frames: u64,
    /// Frames classified as RX.
    pub rx_frames: u64,
    /// TX frames whose RX was found.
    pub paired_frames: u64,
    /// TX frames whose RX never arrived (only with known directions).
    pub unanswered_frames: u64,
    /// Datagram count per command, most frequent first.
    pub commands: Vec<CommandCount>,
    /// Highest broadcast-read working counter on a returning frame: a lower bound on the
    /// number of slaves that answered.
    pub slaves_responding: Option<u16>,
}

/// Expected-vs-actual working counter at one datagram position of a stream.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct WkcSlot {
    /// Datagram position in the frame.
    pub position: u16,
    /// Datagram label, e.g. `"LRW 0x00010000"`.
    pub label: String,
    /// Expected value: the most frequent observed value (no configuration is known).
    pub expected: u16,
    /// Returning frames with a different value.
    pub mismatches: u64,
    /// Returning frames with WKC 0.
    pub zero: u64,
}

/// A point in time with a value.
#[derive(Debug, Clone, Copy, Serialize, TS)]
#[ts(export)]
pub struct Point {
    /// Time since capture start, ns.
    pub t_ns: f64,
    /// Value.
    pub v: f64,
}

/// An event at a frame.
#[derive(Debug, Clone, Copy, Serialize, TS)]
#[ts(export)]
pub struct FrameEvent {
    /// Frame number (1-based).
    pub frame: u32,
    /// Time since capture start, ns.
    pub t_ns: f64,
}

/// A group of frames with identical datagram layout sent repeatedly.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct CyclicStream {
    /// Stream id, 0 = most frequent.
    pub id: u16,
    /// Human-readable layout, e.g. `"FRMW 0x1001:0x0910 · LRW 0x00010000 · BRD 0x0130"`.
    pub signature: String,
    /// Bytes in one frame (captured, no FCS).
    pub frame_bytes: u32,
    /// TX frames in the stream.
    pub frames: u64,
    /// First frame time, ns.
    pub first_ns: f64,
    /// Last frame time, ns.
    pub last_ns: f64,
    /// Nominal period: least-squares slope of send time over cycle number, ns.
    pub period_ns: f64,
    /// Cycle-to-cycle intervals, excluding gaps (> 1.5 periods).
    pub interval: Option<Distribution>,
    /// Deviation of each send time from the ideal grid `t0 + k·period`, ns.
    pub phase_jitter: Option<Distribution>,
    /// 99th percentile of |phase jitter|, ns.
    pub phase_jitter_abs_p99_ns: Option<f64>,
    /// Intervals longer than 1.5 periods.
    pub gaps: u64,
    /// Cycles missing inside those gaps.
    pub missed_cycles: u64,
    /// TX→RX round trip at the capture point, ns.
    pub round_trip: Option<Distribution>,
    /// TX frames that did not come back (known directions only).
    pub unanswered: u64,
    /// Working counters per datagram position.
    pub wkc: Vec<WkcSlot>,
    /// Frames with a WKC mismatch (first 2000).
    pub wkc_events: Vec<FrameEvent>,
    /// Unanswered TX frames (first 2000).
    pub unanswered_events: Vec<FrameEvent>,
    /// Interval over time (decimated).
    pub interval_series: Vec<Point>,
    /// Round trip over time (decimated).
    pub round_trip_series: Vec<Point>,
}

/// Link utilization over time.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct Utilization {
    /// Assumed link rate, bit/s.
    pub link_bps: f64,
    /// Bin width of the series, ns.
    pub bin_ns: f64,
    /// Mean TX utilization over the capture, percent.
    pub mean_tx_percent: f64,
    /// Highest TX bin, percent.
    pub peak_tx_percent: f64,
    /// Mean RX utilization, percent.
    pub mean_rx_percent: f64,
    /// TX percent per bin.
    pub tx_series: Vec<Point>,
    /// RX percent per bin.
    pub rx_series: Vec<Point>,
}

/// One area of ESC registers and how often it was accessed.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct AreaCount {
    /// Register area name.
    pub area: String,
    /// Datagrams addressing it.
    pub count: u64,
}

/// What the capture shows about one configured station address.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct StationSummary {
    /// Configured station address.
    pub station: u16,
    /// Ring position, when the master assigned the address with APWR 0x0010.
    pub position: Option<u16>,
    /// TX datagrams addressed to it.
    pub datagrams: u64,
    /// Of which read or read/write.
    pub reads: u64,
    /// Of which write or read/write.
    pub writes: u64,
    /// Returning datagrams with WKC 0 (no response).
    pub no_response: u64,
    /// Register areas accessed, most frequent first.
    pub areas: Vec<AreaCount>,
    /// Last AL state requested by the master.
    pub requested_state: Option<String>,
    /// Last AL state reported by the slave.
    pub reported_state: Option<String>,
    /// Last reported AL status had the error bit set.
    pub al_error: bool,
}

/// Whether an AL event is a request or a report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AlEventKind {
    /// Master wrote AL control.
    Requested,
    /// Slave reported AL status.
    Reported,
}

/// An AL state request or report that changed the last known value.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct AlEvent {
    /// Time since capture start, ns.
    pub t_ns: f64,
    /// Frame number (1-based).
    pub frame: u32,
    /// Station, or `None` for broadcast.
    pub station: Option<u16>,
    /// Request or report.
    pub kind: AlEventKind,
    /// State name.
    pub state: String,
    /// Error bit (acknowledge in a request, indication in a report).
    pub error: bool,
}

/// Severity of a finding. `Defect` and `Warn` are verdicts against a declared threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Severity {
    /// Context the reader needs.
    Info,
    /// A threshold was crossed that deserves attention.
    Warn,
    /// A threshold was crossed that indicates a fault.
    Defect,
}

/// A judgement or note produced by the analysis.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct Finding {
    /// Severity.
    pub severity: Severity,
    /// One-line statement.
    pub title: String,
    /// The evidence and the threshold used.
    pub detail: String,
}
