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
    /// Mailbox channels, protocols and transfers.
    pub mailbox: MailboxReport,
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
    /// Expected value: the ENI's configured `Cnt` when `expected_from` is `"eni"`, else the most
    /// frequent value on returning frames.
    pub expected: u16,
    /// Returning frames with a different value.
    pub mismatches: u64,
    /// Returning frames with WKC 0.
    pub zero: u64,
    /// `"eni"`: the configured value; `"most_frequent"`: inferred from the capture.
    pub expected_from: String,
    /// The datagram accesses a mailbox; its WKC varies by design (0 = mailbox full/empty)
    /// and is not counted as a mismatch.
    pub mailbox: bool,
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
    /// Cycle-to-cycle intervals, excluding gaps (longer than `gap_factor` periods, default 1.5).
    pub interval: Option<Distribution>,
    /// Deviation of each send time from the ideal grid `t0 + k·period`, ns.
    pub phase_jitter: Option<Distribution>,
    /// 99th percentile of |phase jitter|, ns.
    pub phase_jitter_abs_p99_ns: Option<f64>,
    /// Intervals longer than `gap_factor` periods (default 1.5).
    pub gaps: u64,
    /// Cycles missing inside those gaps.
    pub missed_cycles: u64,
    /// TX→RX round trip at the capture point, ns.
    pub round_trip: Option<Distribution>,
    /// TX frames that did not come back (known directions only).
    pub unanswered: u64,
    /// Working counters per datagram position.
    pub wkc: Vec<WkcSlot>,
    /// Returning frames with a WKC mismatch (all of them; known directions only).
    pub wkc_mismatch_frames: u64,
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
    /// Name from the network configuration (ENI), if one was given.
    pub name: Option<String>,
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

// ---------------------------------------------------------------------------------------------
// Mailbox
// ---------------------------------------------------------------------------------------------

/// Direction of a mailbox message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum MailboxDirection {
    /// Master → slave: written into the slave's "mailbox out" SyncManager.
    ToSlave,
    /// Slave → master: read from the slave's "mailbox in" SyncManager.
    FromSlave,
}

/// Where the analysis learned a station's mailbox SyncManager addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ChannelSource {
    /// From SyncManager configuration writes in the capture.
    SmConfig,
    /// From the network configuration (ENI).
    Eni,
    /// From a user-supplied hint.
    Hint,
    /// From datagrams whose data parses as a mailbox header (no configuration seen).
    Heuristic,
}

/// One slave's mailbox and how it was used.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct MailboxChannel {
    /// Configured station address.
    pub station: u16,
    /// How the SyncManager addresses were found.
    pub source: ChannelSource,
    /// Start of the master → slave mailbox ("mailbox out", usually SM0).
    pub out_start: Option<u16>,
    /// Its length, bytes.
    pub out_len: Option<u16>,
    /// Start of the slave → master mailbox ("mailbox in", usually SM1).
    pub in_start: Option<u16>,
    /// Its length, bytes.
    pub in_len: Option<u16>,
    /// Writes the slave accepted (returned with WKC > 0).
    pub writes: u64,
    /// Writes returned with WKC 0: the mailbox was still full.
    pub writes_rejected: u64,
    /// Reads that returned a message (WKC > 0).
    pub reads: u64,
    /// Reads returned with WKC 0: nothing to read.
    pub empty_reads: u64,
    /// Reads of the "mailbox in" SyncManager status register.
    pub status_polls: u64,
    /// Messages to the slave (excluding repeats).
    pub messages_out: u64,
    /// Messages from the slave (excluding repeats).
    pub messages_in: u64,
    /// Mailbox payload bytes to the slave (after the 6-byte header).
    pub bytes_out: u64,
    /// Mailbox payload bytes from the slave.
    pub bytes_in: u64,
    /// Messages whose counter repeated the previous one (dropped as repeats).
    pub repeats: u64,
    /// Interval between successive read attempts of the "mailbox in" buffer, ns.
    pub poll_interval: Option<Distribution>,
    /// Protocols seen on this channel.
    pub protocols: Vec<String>,
    /// How the master served this mailbox, per cycle of stream 0 (known directions only).
    pub service: Option<MailboxService>,
}

/// How the master serves one slave's mailbox, measured per cycle of cyclic stream 0 over the
/// cycles in which it accessed the slave's mailbox buffers.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct MailboxService {
    /// Cycles with at least one read or write of this slave's mailbox buffers.
    pub cycles: u64,
    /// Reads and writes of the mailbox buffers per such cycle.
    pub accesses_per_cycle: Distribution,
    /// Frames carrying them per such cycle.
    pub frames_per_cycle: Distribution,
    /// Accesses that rode in a frame of a cyclic stream.
    pub in_cyclic_frames: u64,
    /// Time from a mailbox frame's return to the next mailbox frame's send in the same cycle, ns.
    pub turnaround: Option<Distribution>,
    /// Send time of the cycle's last mailbox frame, percent of the period after the cycle start.
    pub last_send_percent: Distribution,
    /// The simulator mailbox policy that produces this pattern.
    pub pattern: ServicePattern,
}

/// A master mailbox policy in the simulator's terms (`flowlab_sim::MailboxPolicy`), as read
/// from a capture.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum ServicePattern {
    /// A fixed number of mailbox datagrams per cycle, in one frame.
    FixedQuota {
        /// Mailbox datagrams per cycle.
        accesses_per_cycle: u32,
    },
    /// Mailbox frames back to back while the cycle has spare time.
    Opportunistic {
        /// Share of the cycle within which mailbox frames are still sent, percent.
        bus_budget_percent: f64,
        /// Time from a mailbox frame's return to the next send, ns.
        turnaround_ns: f64,
    },
}

/// Traffic per mailbox protocol.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ProtocolStats {
    /// `"CoE"`, `"FoE"`, `"EoE"`, `"SoE"`, `"VoE"`, `"AoE"`, `"Error"` or `"Reserved"`.
    pub protocol: String,
    /// Messages to slaves.
    pub messages_out: u64,
    /// Messages from slaves.
    pub messages_in: u64,
    /// Payload bytes to slaves (after the mailbox header).
    pub bytes_out: u64,
    /// Payload bytes from slaves.
    pub bytes_in: u64,
    /// First message, ns.
    pub first_ns: f64,
    /// Last message, ns.
    pub last_ns: f64,
}

/// How an SDO transfer ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SdoOutcome {
    /// Completed.
    Ok,
    /// Aborted by master or slave.
    Aborted,
    /// A new transfer to the same station started before this one completed.
    Superseded,
    /// Still open when the capture ended.
    Incomplete,
    /// Sequence violated: toggle bit, size, or an unexpected response.
    ProtocolError,
}

/// One CoE SDO upload or download, from the initiate request to the last response.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct SdoTransaction {
    /// Station.
    pub station: u16,
    /// `"upload"` (read from slave) or `"download"` (write to slave).
    pub kind: String,
    /// Object index.
    pub index: u16,
    /// Subindex.
    pub subindex: u8,
    /// Complete access.
    pub complete_access: bool,
    /// `"expedited"`, `"normal"` (one message) or `"segmented"`.
    pub transfer: String,
    /// Size announced by the initiating side.
    pub size: Option<u32>,
    /// Data bytes transferred.
    pub bytes: u64,
    /// Segments after the initiate exchange.
    pub segments: u32,
    /// Time of the initiate request, ns.
    pub start_ns: f64,
    /// Time of the final message, ns.
    pub end_ns: Option<f64>,
    /// Frame carrying the initiate request.
    pub first_frame: u32,
    /// Frame carrying the final message.
    pub last_frame: u32,
    /// Outcome.
    pub outcome: SdoOutcome,
    /// Abort code, if aborted.
    pub abort_code: Option<u32>,
    /// Abort code text.
    pub abort_text: Option<String>,
    /// `"master"` or `"slave"`, if aborted.
    pub aborted_by: Option<String>,
    /// What went wrong for a protocol error.
    pub note: Option<String>,
}

/// A CoE emergency message.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct EmergencyEvent {
    /// Time, ns.
    pub t_ns: f64,
    /// Frame (1-based).
    pub frame: u32,
    /// Station.
    pub station: u16,
    /// CiA 301 error code.
    pub error_code: u16,
    /// Error register.
    pub error_register: u8,
    /// Manufacturer-specific bytes, hex.
    pub data: String,
}

/// CoE results.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct CoeReport {
    /// SDO transfers (first 2000).
    pub transactions: Vec<SdoTransaction>,
    /// All SDO transfers.
    pub transactions_total: u64,
    /// Completed.
    pub ok: u64,
    /// Aborted.
    pub aborted: u64,
    /// Superseded or incomplete.
    pub unfinished: u64,
    /// Protocol errors.
    pub protocol_errors: u64,
    /// Duration of completed transfers, ns.
    pub duration: Option<Distribution>,
    /// Emergencies (first 2000).
    pub emergencies: Vec<EmergencyEvent>,
    /// All emergencies.
    pub emergencies_total: u64,
    /// SDO Information messages.
    pub sdo_info_messages: u64,
}

/// An FoE file read or write.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct FoeTransfer {
    /// Station.
    pub station: u16,
    /// `"write"` (to slave) or `"read"` (from slave).
    pub kind: String,
    /// File name from the request.
    pub file_name: String,
    /// Data bytes.
    pub bytes: u64,
    /// Data packets.
    pub packets: u32,
    /// BUSY replies.
    pub busy: u32,
    /// Request time, ns.
    pub start_ns: f64,
    /// Final message time, ns.
    pub end_ns: Option<f64>,
    /// Data bytes · 8 / (end − start), bit/s.
    pub goodput_bps: Option<f64>,
    /// Request frame.
    pub first_frame: u32,
    /// Final frame.
    pub last_frame: u32,
    /// `"ok"`, `"error"`, `"incomplete"` or `"protocol_error"`.
    pub outcome: String,
    /// FoE error code.
    pub error_code: Option<u32>,
    /// Error text (from the table, else the message).
    pub error_text: Option<String>,
}

/// An Ethernet frame reassembled from EoE fragments.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct EoeFrame {
    /// Station.
    pub station: u16,
    /// Direction.
    pub direction: MailboxDirection,
    /// First fragment, ns.
    pub start_ns: f64,
    /// Last fragment, ns.
    pub end_ns: f64,
    /// Frame of the first fragment.
    pub first_frame: u32,
    /// Frame of the last fragment.
    pub last_frame: u32,
    /// Fragments.
    pub fragments: u32,
    /// Reassembled length, bytes.
    pub bytes: u32,
    /// Destination MAC of the tunnelled frame.
    pub dst: Option<String>,
    /// Source MAC.
    pub src: Option<String>,
    /// EtherType.
    pub ethertype: Option<u16>,
}

/// EoE results.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct EoeReport {
    /// Reassembled frames (first 2000).
    pub frames: Vec<EoeFrame>,
    /// All reassembled frames.
    pub frames_total: u64,
    /// Reassembled bytes to slaves.
    pub bytes_to_slaves: u64,
    /// Reassembled bytes from slaves.
    pub bytes_from_slaves: u64,
    /// Fragment sequences that broke off or arrived out of order.
    pub reassembly_errors: u64,
    /// Non-fragment EoE services (IP parameter, address filter, time stamp).
    pub service_messages: u64,
}

/// A mailbox error reply from a slave.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct MailboxErrorEvent {
    /// Time, ns.
    pub t_ns: f64,
    /// Frame.
    pub frame: u32,
    /// Station.
    pub station: u16,
    /// Detail code.
    pub detail: u16,
    /// Text.
    pub text: String,
}

/// Mailbox analysis.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct MailboxReport {
    /// Stations with a mailbox, by station address.
    pub channels: Vec<MailboxChannel>,
    /// Traffic per protocol.
    pub protocols: Vec<ProtocolStats>,
    /// CoE.
    pub coe: CoeReport,
    /// FoE transfers (first 2000).
    pub foe: Vec<FoeTransfer>,
    /// EoE.
    pub eoe: EoeReport,
    /// SoE messages with the error flag.
    pub soe_errors: u64,
    /// Mailbox error replies (first 2000).
    pub errors: Vec<MailboxErrorEvent>,
    /// Data at a mailbox address that is not a valid mailbox header.
    pub malformed: u64,
}
