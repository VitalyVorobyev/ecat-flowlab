use super::*;
use flowlab_analysis::{AnalysisOptions, SdoOutcome, analyze};
use flowlab_capture::{CaptureFile, Trace};
use flowlab_core::Provenance;

fn run(spec: &SimSpec) -> (SimResult, Vec<u8>) {
    let mut bytes = Vec::new();
    let r = simulate(spec, Some(&mut bytes)).unwrap();
    (r, bytes)
}

fn one(protocol: TransferProtocol, direction: TransferDirection, payload_bytes: u64) -> SimSpec {
    let mut s = SimSpec::example();
    s.transfers = vec![TransferSpec { slave: 2, direction, payload_bytes, protocol, start_ms: 0.0 }];
    s
}

fn protocols() -> Vec<TransferProtocol> {
    vec![
        TransferProtocol::Eoe { mtu_bytes: 1500 },
        TransferProtocol::Foe,
        TransferProtocol::CoeSegmented,
        TransferProtocol::PdoChannel { bytes_per_cycle: 256, window: 2, retransmit_after_cycles: 8 },
    ]
}

#[test]
fn example_completes_verified_and_deterministic() {
    let spec = SimSpec::example();
    let (a, bytes_a) = run(&spec);
    let (b, bytes_b) = run(&spec);
    assert_eq!(bytes_a, bytes_b, "same spec ⇒ identical capture");
    assert_eq!(serde_json::to_string(&a).unwrap(), serde_json::to_string(&b).unwrap());
    assert!(a.all_completed, "{:#?}", a.transfers);
    for t in &a.transfers {
        assert!(t.verified, "{} not verified: {t:?}", t.protocol);
        let d = t.duration_ns.unwrap();
        assert!(d >= t.physical_bound_ns, "{} faster than the link allows", t.protocol);
        assert!(t.goodput_bps.unwrap() < a.link_bps);
    }
    assert_eq!(a.meta.source, Provenance::Simulated);
    assert!(!a.capture_truncated);
}

#[test]
fn both_directions_survive_frame_loss() {
    for protocol in protocols() {
        for direction in [TransferDirection::MasterToSlave, TransferDirection::SlaveToMaster] {
            let mut s = one(protocol.clone(), direction, 20_000);
            s.faults.frame_loss_probability = 0.02;
            s.capture.enabled = false;
            let r = simulate(&s, None::<Vec<u8>>).unwrap();
            let t = &r.transfers[0];
            assert!(t.completed && t.verified, "{protocol:?} {direction:?}: {t:?}");
            assert_eq!(t.delivered_bytes, 20_000);
        }
    }
}

#[test]
fn boundary_payloads() {
    for len in [0u64, 1, 7, 8, 503, 504, 505, 1472, 1473] {
        for protocol in protocols() {
            let mut s = one(protocol.clone(), TransferDirection::MasterToSlave, len);
            s.capture.enabled = false;
            let r = simulate(&s, None::<Vec<u8>>).unwrap();
            assert!(r.transfers[0].verified, "{protocol:?} {len} B: {:?}", r.transfers[0]);
        }
    }
}

#[test]
fn slower_pdi_never_speeds_a_transfer_up() {
    for protocol in protocols() {
        let mut last = 0.0;
        for overhead in [0u32, 5_000, 50_000, 200_000] {
            let mut s = one(protocol.clone(), TransferDirection::MasterToSlave, 30_000);
            s.capture.enabled = false;
            s.slaves[2].pdi.access_overhead_ns = overhead;
            let r = simulate(&s, None::<Vec<u8>>).unwrap();
            let d = r.transfers[0].duration_ns.unwrap();
            assert!(d >= last, "{protocol:?}: overhead {overhead} ns gave {d} < {last}");
            last = d;
        }
    }
}

#[test]
fn mailbox_policy_matters_and_is_reported() {
    let foe = |policy: MailboxPolicy, slaves: usize| {
        let mut s = one(TransferProtocol::Foe, TransferDirection::MasterToSlave, 50_000);
        s.capture.enabled = false;
        s.mailbox_policy = policy;
        for extra in 1..slaves {
            s.slaves.push(s.slaves[2].clone());
            s.transfers.push(TransferSpec { slave: 2 + extra, ..s.transfers[0].clone() });
        }
        let r = simulate(&s, None::<Vec<u8>>).unwrap();
        assert!(r.transfers.iter().all(|t| t.verified));
        r.transfers.iter().map(|t| t.duration_ns.unwrap()).fold(0.0, f64::max)
    };
    let q1 = MailboxPolicy::FixedQuota { accesses_per_cycle: 1 };
    let q2 = MailboxPolicy::FixedQuota { accesses_per_cycle: 2 };
    let opp = MailboxPolicy::Opportunistic { bus_budget_percent: 80.0, turnaround_ns: 5_000 };
    // One slave: FoE waits for each ACK, so a second access per cycle does not help…
    assert_eq!(foe(q1.clone(), 1), foe(q2.clone(), 1));
    // …but several mailbox frames per cycle do.
    assert!(foe(opp, 1) < foe(q2.clone(), 1) / 2.0);
    // Two slaves share the quota.
    assert!(foe(q1, 2) > foe(q2, 2));
    let r = simulate(&SimSpec::example(), None::<Vec<u8>>).unwrap();
    assert!(r.policy.starts_with("fixed quota"));
}

#[test]
fn esc_memory_gates_the_configuration() {
    let mut s = SimSpec::example();
    s.slaves[2].dpram_bytes = 1024;
    let err = simulate(&s, None::<Vec<u8>>).unwrap_err().to_string();
    assert!(err.contains("ESC RAM"), "{err}");
}

#[test]
fn capture_is_analysed_like_a_measurement() {
    let spec = SimSpec::example();
    let (r, bytes) = run(&spec);
    let trace = Trace::build(CaptureFile::parse(bytes).unwrap(), "sim.pcapng");
    assert_eq!(trace.provenance, Provenance::Simulated);
    assert!(trace.errors.is_empty(), "{:?}", trace.errors);
    let a = analyze(&trace, &AnalysisOptions::default());
    let rep = &a.report;
    assert_eq!(rep.meta.source, Provenance::Simulated);
    let s = &rep.streams[0];
    assert!((s.period_ns - spec.cycle_time_us as f64 * 1000.0).abs() < 100.0, "period {}", s.period_ns);
    assert_eq!(s.unanswered, 0);
    assert!(s.wkc_events.is_empty(), "PDO channel and process data keep a constant WKC");

    let m = &rep.mailbox;
    let ch = m.channels.iter().find(|c| c.station == FIRST_STATION + 2).unwrap();
    assert_eq!(ch.source, flowlab_analysis::ChannelSource::SmConfig);
    let mbx: Vec<_> = r.transfers.iter().filter(|t| t.protocol != "PDO channel").collect();
    assert_eq!(ch.messages_out, mbx.iter().map(|t| t.messages_to_slave).sum::<u64>());
    assert_eq!(ch.messages_in, mbx.iter().map(|t| t.messages_from_slave).sum::<u64>());
    assert_eq!(ch.writes_rejected, mbx.iter().map(|t| t.rejected_writes).sum::<u64>());
    assert_eq!(ch.empty_reads, mbx.iter().map(|t| t.empty_reads).sum::<u64>());

    let payload = spec.transfers[0].payload_bytes;
    assert_eq!(m.eoe.frames_total, payload.div_ceil(1472));
    assert_eq!(m.eoe.reassembly_errors, 0);
    assert_eq!(m.eoe.bytes_to_slaves, payload + m.eoe.frames_total * 42);
    assert_eq!(m.foe.len(), 1);
    assert_eq!((m.foe[0].outcome.as_str(), m.foe[0].bytes), ("ok", payload));
    let sdo = m.coe.transactions.iter().find(|t| t.index == 0x2000).unwrap();
    assert_eq!((sdo.outcome, sdo.transfer.as_str(), sdo.bytes), (SdoOutcome::Ok, "segmented", payload));
    assert_eq!(m.coe.protocol_errors, 0);
}

#[test]
fn schema_and_example_agree() {
    let schema = spec_schema();
    assert!(schema["$schema"].as_str().unwrap().contains("2020-12"));
    let example = serde_json::to_value(SimSpec::example()).unwrap();
    let back: SimSpec = serde_json::from_value(example).unwrap();
    assert_eq!(back, SimSpec::example());
}

#[test]
fn message_only_pdi_access_is_not_slower_for_short_messages() {
    let mut s = one(TransferProtocol::Foe, TransferDirection::MasterToSlave, 50_000);
    s.capture.enabled = false;
    s.slaves[2].pdi.read_bps = 4_000_000; // slow PDI: the ACKs' buffer writes dominate
    s.slaves[2].pdi.write_bps = 4_000_000;
    let whole = simulate(&s, None::<Vec<u8>>).unwrap().transfers[0].duration_ns.unwrap();
    s.slaves[2].pdi.mailbox_access = MailboxAccess::MessageOnly;
    let r = simulate(&s, None::<Vec<u8>>).unwrap();
    assert!(r.transfers[0].verified);
    assert!(r.transfers[0].duration_ns.unwrap() < whole);
}

#[test]
fn shipped_specs_parse_and_validate() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../configs");
    let example: SimSpec =
        serde_json::from_str(&std::fs::read_to_string(format!("{root}/sim/example.json")).unwrap()).unwrap();
    assert_eq!(example, SimSpec::example(), "configs/sim/example.json is SimSpec::example()");
    for name in ["smoke", "large-payload", "mixed-load"] {
        let text = std::fs::read_to_string(format!("{root}/studies/bulk-transfer/{name}.json")).unwrap();
        let spec: SimSpec = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
        spec.validate().unwrap_or_else(|e| panic!("{name}: {e}"));
    }
}
