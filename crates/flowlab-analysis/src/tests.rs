//! Analysis checked against flowlab-synth ground truth.

use flowlab_capture::{CaptureFile, Trace};
use flowlab_synth::{FaultSpec, GroundTruth, SlaveSpec, SynthSpec, generate};

use super::frames::{FrameFilter, detail, query};
use super::*;

fn spec(faults: FaultSpec) -> SynthSpec {
    SynthSpec {
        seed: 42,
        description: String::new(),
        slaves: vec![
            SlaveSpec { name: "coupler".into(), outputs_bytes: 0, inputs_bytes: 0, mailbox: false },
            SlaveSpec { name: "di".into(), outputs_bytes: 0, inputs_bytes: 2, mailbox: true },
            SlaveSpec { name: "do".into(), outputs_bytes: 2, inputs_bytes: 0, mailbox: true },
            SlaveSpec { name: "drive".into(), outputs_bytes: 12, inputs_bytes: 16, mailbox: true },
        ],
        cycle_time_us: 1000,
        cycles: 2000,
        tx_jitter_ns: 5_000,
        slave_delay_ns: 700,
        include_startup: true,
        sdo_uploads_per_slave: 2,
        distributed_clocks: true,
        start_unix_s: 1_767_225_600,
        faults,
    }
}

fn run(s: &SynthSpec) -> (Trace, Analysis, GroundTruth) {
    let mut bytes = Vec::new();
    let truth = generate(s, &mut bytes).unwrap();
    let trace = Trace::build(CaptureFile::parse(bytes).unwrap(), "t.pcapng");
    let opts = AnalysisOptions { in_flight_window_ns: 0, ..AnalysisOptions::default() };
    let a = analyze(&trace, &opts);
    (trace, a, truth)
}

fn faulty() -> SynthSpec {
    spec(FaultSpec { lost_frame_probability: 0.01, wkc_error_probability: 0.01, skipped_cycle_probability: 0.005 })
}

#[test]
fn recovers_cycle_timing() {
    let (_, a, truth) = run(&faulty());
    let r = &a.report;
    assert_eq!(r.streams.len(), 1, "{:#?}", r.streams.iter().map(|s| &s.signature).collect::<Vec<_>>());
    let s = &r.streams[0];
    assert!(s.signature.contains("LRW 0x00010000"), "{}", s.signature);
    assert_eq!(s.frames, truth.cyclic_tx_frames);
    assert!((s.period_ns - truth.cycle_time_ns as f64).abs() < 500.0, "period {}", s.period_ns);
    let pj = s.phase_jitter.as_ref().unwrap();
    let rel = pj.std / truth.tx_jitter_ns as f64;
    assert!((0.85..1.15).contains(&rel), "phase jitter std {} vs {}", pj.std, truth.tx_jitter_ns);
    // Interval jitter is the difference of two independent phase errors: std ≈ √2 σ.
    let iv = s.interval.as_ref().unwrap();
    assert!((iv.std / pj.std - 2f64.sqrt()).abs() < 0.2, "interval std {}", iv.std);
    assert!(s.missed_cycles <= truth.skipped_cycles && s.missed_cycles + 2 >= truth.skipped_cycles);
    let rtt = s.round_trip.as_ref().unwrap();
    // Frame: 14 Ethernet + 2 header + FRMW(10+8+2) + LRW(10+32+2) + BRD(10+2+2) = 94 bytes,
    // 118 on the wire = 9.44 µs at 100 Mbit/s, plus 4 slaves × 700 ns.
    assert_eq!(s.frame_bytes, 94);
    assert!((rtt.p50 - (9_440.0 + 2_800.0)).abs() < 1.0, "rtt {}", rtt.p50);
}

#[test]
fn recovers_faults() {
    let (trace, a, truth) = run(&faulty());
    let r = &a.report;
    let s = &r.streams[0];
    assert!(truth.lost_frames > 0 && truth.wkc_errors > 0);
    assert_eq!(s.unanswered, truth.lost_frames);
    assert_eq!(r.summary.unanswered_frames, truth.lost_frames);
    assert_eq!(s.wkc_events.len() as u64, truth.wkc_errors);
    let lrw = s.wkc.iter().find(|w| w.label.starts_with("LRW")).unwrap();
    assert_eq!(lrw.expected, truth.expected_lrw_wkc);
    assert_eq!(lrw.mismatches, truth.wkc_errors);
    assert!(r.findings.iter().any(|f| f.severity == Severity::Defect && f.title.contains("did not return")));
    assert!(r.findings.iter().any(|f| f.severity == Severity::Defect && f.title.contains("working-counter")));

    let problems = query(&trace, &a, &FrameFilter { problems_only: true, ..Default::default() }, 0, 10);
    assert_eq!(problems.total, truth.lost_frames + 2 * truth.wkc_errors);
    assert_eq!(problems.rows.len(), 10);
}

#[test]
fn stations_and_states() {
    let (trace, a, truth) = run(&faulty());
    let r = &a.report;
    assert_eq!(r.summary.slaves_responding, Some(truth.slaves as u16));
    assert_eq!(r.stations.len(), truth.slaves);
    for (p, st) in r.stations.iter().enumerate() {
        assert_eq!(st.station, 0x1001 + p as u16);
        assert_eq!(st.position, Some(p as u16));
        assert_eq!(st.requested_state.as_deref(), Some("OP"));
        assert_eq!(st.reported_state.as_deref(), Some("OP"));
        assert!(!st.al_error);
    }
    let reqs = r.al_events.iter().filter(|e| e.kind == AlEventKind::Requested && e.station == Some(0x1004)).count();
    assert_eq!(reqs, 3, "PREOP, SAFEOP, OP");
    assert_eq!(r.summary.paired_frames + r.summary.unanswered_frames, r.summary.tx_frames);
    assert_eq!(trace.direction_mode, flowlab_capture::DirectionMode::MacLocalBit);
}

#[test]
fn frame_detail_decodes_mailbox() {
    let (trace, a, _) = run(&faulty());
    let page = query(
        &trace,
        &a,
        &FrameFilter { command: Some("FPWR".into()), station: Some(0x1002), ..Default::default() },
        0,
        100,
    );
    let sdo = page.rows.iter().find_map(|r| {
        let d = detail(&trace, &a, r.no).unwrap();
        d.datagrams[0].mailbox.clone()
    });
    let mbx = sdo.expect("an SDO request to station 0x1002");
    assert_eq!(mbx.protocol, "COE");
    let coe = mbx.coe.unwrap();
    assert_eq!(coe.service, "SdoRequest");
    assert_eq!(coe.index, Some(0x1018));

    let cyclic =
        query(&trace, &a, &FrameFilter { stream: Some(0), direction: Some("rx".into()), ..Default::default() }, 0, 1);
    let d = detail(&trace, &a, cyclic.rows[0].no).unwrap();
    assert_eq!(d.datagrams.len(), 3);
    assert!(d.datagrams.iter().all(|g| g.expected_wkc.is_some()));
    assert_eq!(d.datagrams[2].al_state.as_deref(), Some("OP"));
    assert_eq!(d.hex.len(), 2 * d.row.len as usize);
    let covered: u32 = d.ranges.iter().map(|r| r.len).sum();
    assert_eq!(covered, d.row.len);
    assert!(detail(&trace, &a, 0).is_none());
    assert!(detail(&trace, &a, u32::MAX).is_none());
}

#[test]
fn healthy_capture_has_no_defects() {
    let (_, a, _) = run(&spec(FaultSpec::default()));
    assert!(a.report.findings.iter().all(|f| f.severity == Severity::Info), "{:#?}", a.report.findings);
    assert!(a.report.utilization.mean_tx_percent > 0.0 && a.report.utilization.mean_tx_percent < 100.0);
}

#[test]
fn report_is_deterministic() {
    let (_, a1, _) = run(&faulty());
    let (_, a2, _) = run(&faulty());
    let j1 = serde_json::to_string(&a1.report).unwrap();
    let j2 = serde_json::to_string(&a2.report).unwrap();
    assert_eq!(j1, j2);
    assert!(j1.contains("\"source\":\"synthetic\""));
}
