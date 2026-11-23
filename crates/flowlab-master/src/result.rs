//! Serializable results: a measured transfer, a cyclic run and a network scan.
//!
//! Fields that mean the same as in the simulator's `TransferResult` have the same names, so a
//! measured and a simulated transfer are compared field by field. Every result carries
//! provenance `measured` in its `meta`.

use flowlab_core::ReportMeta;
use flowlab_core::stats::quantile;
use serde::Serialize;

/// Summary of a set of durations, ns.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Summary {
    /// Number of values.
    pub count: u64,
    /// Smallest, ns.
    pub min_ns: f64,
    /// Median, ns.
    pub p50_ns: f64,
    /// 99th percentile (linear interpolation, as every FlowLab percentile), ns.
    pub p99_ns: f64,
    /// Largest, ns.
    pub max_ns: f64,
    /// Mean, ns.
    pub mean_ns: f64,
}

impl Summary {
    /// Summary of `values` (in any order); all zeros when empty.
    pub fn of(values: &[f64]) -> Self {
        if values.is_empty() {
            return Self::default();
        }
        let mut v = values.to_vec();
        v.sort_by(f64::total_cmp);
        Self {
            count: v.len() as u64,
            min_ns: v[0],
            p50_ns: quantile(&v, 0.5),
            p99_ns: quantile(&v, 0.99),
            max_ns: v[v.len() - 1],
            mean_ns: v.iter().sum::<f64>() / v.len() as f64,
        }
    }
}

/// One master cycle as the host saw it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CycleRecord {
    /// Cycle number from the start of the run (skipped cycles leave gaps).
    pub index: u64,
    /// When the cycle started, ns since the start of the run.
    pub start_ns: f64,
    /// How much later than planned it started, ns.
    pub late_ns: f64,
    /// Round trip of the cyclic process data exchange, ns (none without process data).
    pub cyclic_ns: Option<f64>,
    /// Working counter of the cyclic exchange.
    pub cyclic_wkc: Option<u16>,
    /// Mailbox datagrams sent in this cycle (writes, reads, status and repeat accesses).
    pub mailbox_accesses: u32,
    /// From the cycle's start to the end of its last access, ns.
    pub busy_ns: f64,
}

/// Timing of the cycles of a run or a transfer.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct CycleTiming {
    /// Cycles run.
    pub cycles: u64,
    /// Planned cycles that were skipped because the previous one ran past them.
    pub skipped_cycles: u64,
    /// Cycles whose work ended after the next cycle's planned start.
    pub overruns: u64,
    /// Start lateness against the plan.
    pub start_late: Summary,
    /// Time between consecutive cycle starts.
    pub period: Summary,
    /// Round trip of the cyclic exchange.
    pub cyclic: Summary,
    /// Round trip of each mailbox datagram.
    pub mailbox_access: Summary,
    /// From cycle start to the end of the cycle's last access.
    pub busy: Summary,
    /// Most mailbox datagrams in one cycle.
    pub max_accesses_per_cycle: u32,
}

/// A measured transfer.
#[derive(Debug, Clone, Serialize)]
pub struct MeasuredTransfer {
    /// Provenance (`measured`), the payload's identity and the assumptions behind the numbers.
    pub meta: ReportMeta,
    /// Station address of the slave.
    pub station: u16,
    /// Slave name.
    pub slave: String,
    /// `"EoE"`, `"FoE"` or `"CoE"`.
    pub protocol: String,
    /// `"master_to_slave"` or `"slave_to_master"`.
    pub direction: String,
    /// Mailbox policy, as text (the simulator's wording).
    pub policy: String,
    /// How the master learns that "mailbox in" is full: `"read"` (read the buffer, WKC 0 when
    /// empty) or `"status"` (read the SyncManager status first).
    pub poll: String,
    /// Cycle time, ns.
    pub cycle_time_ns: f64,
    /// Mailbox sizes used, bytes: "mailbox out" (SM0) and "mailbox in" (SM1).
    pub mailbox_out_bytes: u16,
    /// "Mailbox in" size, bytes.
    pub mailbox_in_bytes: u16,
    /// Payload, bytes (for slave → master: the size announced or expected).
    pub payload_bytes: u64,
    /// Master → slave: payload bytes in writes the slave accepted. Slave → master: payload
    /// bytes received in order.
    pub delivered_bytes: u64,
    /// Slave → master: the received bytes' SHA-256 equals the expected one. Master → slave
    /// over FlowLab VoE: the length and SHA-256 the slave reported in DONE equal the payload's.
    /// `None` when there was nothing to compare with, or for master → slave over FoE, CoE and
    /// EoE (the master cannot see the slave's copy; compare `sha256` with a hash taken on the
    /// slave).
    pub verified: Option<bool>,
    /// SHA-256 of the payload (sent, or received), hex.
    pub sha256: String,
    /// The protocol finished (last acknowledgement exchanged).
    pub completed: bool,
    /// Why the transfer stopped early, if it did.
    pub error: Option<String>,
    /// When the transfer started (its first cycle), ns. Always 0: times are relative to it.
    pub start_ns: f64,
    /// Return of the access that carried the first payload byte, ns.
    pub first_byte_ns: Option<f64>,
    /// Return of the access that completed the transfer, ns: the read of the last
    /// acknowledgement (master → slave FoE and CoE), the write of the last fragment (EoE), or
    /// the read carrying the last byte (slave → master).
    pub end_ns: Option<f64>,
    /// `end − start`, ns.
    pub duration_ns: Option<f64>,
    /// From the return of the first access that carried a message to the return of the last,
    /// ns: the span a capture at the master shows, defined as the simulator's `exchange_ns`.
    pub exchange_ns: Option<f64>,
    /// Payload bits / duration, bit/s.
    pub goodput_bps: Option<f64>,
    /// Mailbox messages master → slave the slave accepted (protocol messages, control
    /// included; resent messages count again).
    pub messages_to_slave: u64,
    /// Mailbox messages slave → master read (duplicates excluded).
    pub messages_from_slave: u64,
    /// Mailbox writes the slave rejected (WKC 0: mailbox still full).
    pub rejected_writes: u64,
    /// Mailbox reads that found nothing (WKC 0, or the status showed an empty mailbox).
    pub empty_reads: u64,
    /// SyncManager status reads (`poll = "status"`).
    pub status_reads: u64,
    /// Accesses whose frame did not come back.
    pub lost_frames: u64,
    /// Mailbox repeat requests after a lost read.
    pub repeat_requests: u64,
    /// Messages sent again with a new counter: no reply in time, or the slave was busy.
    pub resent_messages: u64,
    /// Messages from the slave with the counter of the previous one, dropped.
    pub duplicate_messages: u64,
    /// Messages from the slave for another protocol (e.g. CoE emergencies), ignored.
    pub other_messages: u64,
    /// Stale messages read from "mailbox in" before the transfer started, discarded.
    pub drained_messages: u64,
    /// Time the payload needs at `link_bps` at zero overhead, ns (as in the simulator).
    pub physical_bound_ns: f64,
    /// Link rate the bound uses, bit/s.
    pub link_bps: f64,
    /// Cycle timing over the transfer.
    pub timing: CycleTiming,
    /// Per-cycle records, up to the configured maximum.
    pub cycle_records: Vec<CycleRecord>,
    /// Some cycles are not in `cycle_records`.
    pub cycle_records_truncated: bool,
    /// Slave → master: the bytes received (not written to JSON).
    #[serde(skip)]
    pub received: Option<Vec<u8>>,
}

/// One slave found by a scan.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ScannedSlave {
    /// Ring position, from 0.
    pub position: u16,
    /// Configured station address.
    pub station: u16,
    /// Alias address (0 if none).
    pub alias: u16,
    /// Name from the EEPROM.
    pub name: String,
    /// Vendor id.
    pub vendor_id: u32,
    /// Product code.
    pub product_code: u32,
    /// Revision.
    pub revision: u32,
    /// Serial number.
    pub serial: u32,
    /// AL state, e.g. `"PREOP"` (with `" (error)"` when the error bit is set).
    pub al_state: String,
    /// AL status code (0 = no error).
    pub al_status_code: u16,
    /// Mailbox protocols from the EEPROM (`"CoE"`, `"FoE"`, `"EoE"`, …).
    pub mailbox_protocols: Vec<String>,
    /// SyncManagers as read from registers `0x0800 + 8·n` (enabled ones).
    pub sync_managers: Vec<ScannedSm>,
    /// Distributed clocks support: `"none"`, `"ref_only"`, `"32 bit"` or `"64 bit"`.
    pub dc_support: String,
    /// Propagation delay from the reference clock, ns, as EtherCrab measured it.
    pub propagation_delay_ns: u32,
}

/// One SyncManager register block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScannedSm {
    /// Index.
    pub index: u8,
    /// Start address.
    pub start: u16,
    /// Length, bytes.
    pub length: u16,
    /// Control register.
    pub control: u8,
    /// `"mailbox"` or `"buffered"`.
    pub mode: String,
    /// `"master_to_slave"` or `"slave_to_master"`.
    pub direction: String,
}

/// What a scan found.
#[derive(Debug, Clone, Serialize)]
pub struct ScanResult {
    /// Provenance (`measured`) and the network interface.
    pub meta: ReportMeta,
    /// Interface scanned.
    pub interface: String,
    /// Slaves in ring order.
    pub slaves: Vec<ScannedSlave>,
}

impl ScanResult {
    /// The network as an ENI subset FlowLab's analysis reads (`flowlab analyze --eni`):
    /// slaves, identities and mailbox SyncManagers. Cyclic commands are not included: the
    /// scan does not configure process data.
    pub fn network_config(&self) -> flowlab_devices::NetworkConfig {
        let slaves = self
            .slaves
            .iter()
            .map(|s| {
                let sm = |d: &str| s.sync_managers.iter().find(|m| m.mode == "mailbox" && m.direction == d);
                let mailbox = match (sm("master_to_slave"), sm("slave_to_master")) {
                    (Some(o), Some(i)) => Some(flowlab_devices::MailboxConfig {
                        out_start: o.start,
                        out_len: o.length,
                        in_start: i.start,
                        in_len: i.length,
                        protocols: s.mailbox_protocols.clone(),
                    }),
                    _ => None,
                };
                flowlab_devices::SlaveConfig {
                    name: s.name.clone(),
                    station: s.station,
                    position: Some(s.position),
                    vendor_id: Some(s.vendor_id),
                    product_code: Some(s.product_code),
                    revision: Some(s.revision),
                    outputs_bits: 0,
                    inputs_bits: 0,
                    mailbox,
                }
            })
            .collect();
        flowlab_devices::NetworkConfig {
            master: Some("FlowLab lab master (EtherCrab)".into()),
            cycle_time_ns: None,
            slaves,
            cyclic: Vec::new(),
        }
    }
}

/// A cyclic run: process data at a fixed cycle for a duration.
#[derive(Debug, Clone, Serialize)]
pub struct RunResult {
    /// Provenance (`measured`), the interface and the assumptions.
    pub meta: ReportMeta,
    /// Interface.
    pub interface: String,
    /// Slaves in the group.
    pub slaves: u16,
    /// Requested cycle time, ns.
    pub cycle_time_ns: f64,
    /// Process image, bytes (outputs + inputs).
    pub process_image_bytes: u32,
    /// Expected working counter of the process data exchange.
    pub expected_wkc: u16,
    /// Cycles whose working counter differed from `expected_wkc`.
    pub wkc_mismatches: u64,
    /// Exchanges whose frame did not come back.
    pub lost_frames: u64,
    /// AL state of every slave after the run.
    pub final_states: Vec<String>,
    /// Cycle timing.
    pub timing: CycleTiming,
    /// Per-cycle records, up to the configured maximum.
    pub cycle_records: Vec<CycleRecord>,
    /// Some cycles are not in `cycle_records`.
    pub cycle_records_truncated: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_interpolates_like_the_analysis() {
        let s = Summary::of(&[5.0, 1.0, 3.0, 2.0, 4.0]);
        assert_eq!((s.count, s.min_ns, s.p50_ns, s.max_ns, s.mean_ns), (5, 1.0, 3.0, 5.0, 3.0));
        // Position 0.99 · 4 = 3.96 between 4 and 5, as `Distribution::p99` of a capture.
        assert!((s.p99_ns - 4.96).abs() < 1e-9, "{}", s.p99_ns);
        assert_eq!(Summary::of(&[]), Summary::default());
    }
}
