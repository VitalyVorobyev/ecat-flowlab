//! Serializable simulation results (the UI's API types).

use flowlab_core::ReportMeta;
use serde::Serialize;
use ts_rs::TS;

use crate::SimSpec;

/// Outcome of one transfer.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct TransferResult {
    /// Index in the spec.
    pub index: u32,
    /// Station address of the slave.
    pub station: u16,
    /// Slave name.
    pub slave: String,
    /// `"EoE"`, `"FoE"`, `"CoE"` or `"PDO channel"`.
    pub protocol: String,
    /// `"master_to_slave"` or `"slave_to_master"`.
    pub direction: String,
    /// Payload, bytes.
    pub payload_bytes: u64,
    /// Payload bytes the receiver got in order.
    pub delivered_bytes: u64,
    /// The receiver got every byte, in order, and its SHA-256 equals the sender's.
    pub verified: bool,
    /// SHA-256 of the payload, hex.
    pub sha256: String,
    /// The protocol finished (last acknowledgement exchanged).
    pub completed: bool,
    /// When the transfer became active, ns since the first frame.
    pub start_ns: f64,
    /// First payload byte at the receiver, ns.
    pub first_byte_ns: Option<f64>,
    /// Last payload byte at the receiver, ns.
    pub end_ns: Option<f64>,
    /// `end − start`, ns.
    pub duration_ns: Option<f64>,
    /// From the return of the frame carrying the transfer's first mailbox message to the
    /// return of the frame carrying its last, ns — the span a capture at the master shows
    /// (and `flowlab analyze` reports). Mailbox protocols only.
    pub exchange_ns: Option<f64>,
    /// Payload bits / duration, bit/s.
    pub goodput_bps: Option<f64>,
    /// Mailbox messages master → slave (protocol messages, control included).
    pub messages_to_slave: u64,
    /// Mailbox messages slave → master.
    pub messages_from_slave: u64,
    /// Mailbox writes the slave rejected (mailbox still full).
    pub rejected_writes: u64,
    /// Mailbox reads that found nothing.
    pub empty_reads: u64,
    /// PDO channel chunks sent again after a stalled window.
    pub retransmitted_chunks: u64,
    /// Cycles in which the PDO channel carried a new chunk.
    pub chunk_cycles: u64,
    /// Time the payload needs on the link at zero overhead, ns. No transfer can be faster.
    pub physical_bound_ns: f64,
    /// Plain-Ethernet reference: TCP over a dedicated link at the same rate, MSS 1460,
    /// steady state, no loss, no slow start, ns. A lower bound, not a prediction.
    pub ethernet_tcp_reference_ns: f64,
}

/// Per-slave resource use.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct SlaveResult {
    /// Station address.
    pub station: u16,
    /// Name.
    pub name: String,
    /// PDI busy time / simulated time, percent.
    pub pdi_busy_percent: f64,
    /// ESC RAM used by SyncManagers, bytes.
    pub dpram_used_bytes: u32,
    /// ESC RAM available, bytes.
    pub dpram_bytes: u32,
}

/// Result of one simulation run.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct SimResult {
    /// Provenance (`simulated`), the spec's identity, the model's assumptions.
    pub meta: ReportMeta,
    /// Spec description.
    pub description: String,
    /// Seed.
    pub seed: u64,
    /// Mailbox policy, as text.
    pub policy: String,
    /// Cycle time, ns.
    pub cycle_time_ns: f64,
    /// Link rate, bit/s.
    pub link_bps: f64,
    /// Virtual time simulated, ns.
    pub virtual_time_ns: f64,
    /// Master cycles run.
    pub cycles: u64,
    /// Frames sent by the master.
    pub frames: u64,
    /// Bytes on the wire master → slaves, incl. preamble, FCS, padding and gap.
    pub wire_bytes: u64,
    /// Wire bits / (link rate × simulated time), percent.
    pub link_utilization_percent: f64,
    /// Every transfer finished within `max_duration_ms`.
    pub all_completed: bool,
    /// Transfers.
    pub transfers: Vec<TransferResult>,
    /// Slaves.
    pub slaves: Vec<SlaveResult>,
    /// Frames written to the capture (TX + RX).
    pub capture_frames: u64,
    /// The capture stopped at `capture.max_frames`.
    pub capture_truncated: bool,
    /// The spec that was simulated, so the result says what it is a result of.
    pub spec: SimSpec,
}
