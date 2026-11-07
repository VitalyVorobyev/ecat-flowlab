//! Analysis checked against flowlab-synth ground truth.

use flowlab_capture::{CaptureFile, Trace};
use flowlab_synth::{FaultSpec, GroundTruth, MailboxSpec, SlaveSpec, SynthSpec, generate};

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
        mailbox: Default::default(),
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
    spec(FaultSpec {
        lost_frame_probability: 0.01,
        wkc_error_probability: 0.01,
        skipped_cycle_probability: 0.005,
        ..FaultSpec::default()
    })
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
    assert_eq!(mbx.role, "write attempt");
    assert_eq!(mbx.source, ChannelSource::SmConfig);
    assert_eq!(mbx.header.unwrap().protocol, "CoE");
    assert!(mbx.summary.unwrap().starts_with("CoE SDO upload req 0x1018:01"));
    assert!(mbx.fields.iter().any(|f| f.name == "Index" && f.value == "0x1018:01"));

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

fn mailbox_heavy() -> SynthSpec {
    let mut s = faulty();
    s.cycles = 1500;
    s.mailbox = MailboxSpec {
        response_delay_polls: 3,
        segmented_upload_bytes: 300,
        download_bytes: 250,
        aborted_uploads: 2,
        foe_write_bytes: 116 * 5, // a whole number of packets: ends with an empty DATA packet
        eoe_frames_to_slave: 4,
        eoe_frames_from_slave: 3,
        eoe_frame_bytes: 1000,
        emergencies: 2,
    };
    s.faults.mailbox_write_reject_probability = 0.1;
    s
}

#[test]
fn recovers_mailbox_ground_truth() {
    let (trace, a, truth) = run(&mailbox_heavy());
    let m = &a.report.mailbox;
    let t = &truth.mailbox;
    assert!(t.rejected_writes > 0 && t.empty_polls > 0, "{t:?}");

    assert_eq!(m.channels.len(), 3);
    for c in &m.channels {
        assert_eq!(c.source, ChannelSource::SmConfig);
        assert_eq!((c.out_start, c.out_len, c.in_start, c.in_len), (Some(0x1000), Some(128), Some(0x1080), Some(128)));
        assert_eq!(c.repeats, 0);
    }
    let sum = |f: fn(&MailboxChannel) -> u64| m.channels.iter().map(f).sum::<u64>();
    assert_eq!(sum(|c| c.messages_out), t.messages_to_slaves);
    assert_eq!(sum(|c| c.messages_in), t.messages_from_slaves);
    assert_eq!(sum(|c| c.writes_rejected), t.rejected_writes);
    assert_eq!(sum(|c| c.empty_reads), t.empty_polls);
    assert_eq!(m.malformed, 0);

    let coe = &m.coe;
    assert_eq!(coe.ok, t.sdo_ok);
    assert_eq!(coe.aborted, t.sdo_aborted);
    assert_eq!((coe.protocol_errors, coe.unfinished), (0, 0));
    let segmented: Vec<_> = coe.transactions.iter().filter(|x| x.transfer == "segmented").collect();
    assert_eq!(segmented.len() as u64, t.sdo_segmented);
    assert_eq!(coe.transactions.iter().map(|x| x.segments as u64).sum::<u64>(), t.sdo_segments);
    let up = coe.transactions.iter().find(|x| x.index == 0x2000).unwrap();
    assert_eq!((up.kind.as_str(), up.size, up.bytes), ("upload", Some(300), 300));
    let down = coe.transactions.iter().find(|x| x.index == 0x2001).unwrap();
    assert_eq!((down.kind.as_str(), down.size, down.bytes), ("download", Some(250), 250));
    let ab = coe.transactions.iter().find(|x| x.outcome == SdoOutcome::Aborted).unwrap();
    assert_eq!((ab.abort_code, ab.aborted_by.as_deref()), (Some(flowlab_synth::ABORT_NO_OBJECT), Some("slave")));
    assert_eq!(coe.emergencies_total, t.emergencies);

    assert_eq!(m.foe.len() as u64, t.foe_transfers);
    let foe = &m.foe[0];
    assert_eq!((foe.outcome.as_str(), foe.kind.as_str(), foe.file_name.as_str()), ("ok", "write", "firmware.bin"));
    assert_eq!(foe.bytes, t.foe_bytes);
    assert_eq!(foe.packets, 6);
    assert!(foe.goodput_bps.unwrap() > 0.0);

    let eoe = &m.eoe;
    assert_eq!(eoe.reassembly_errors, 0);
    assert_eq!(eoe.frames_total, t.eoe_frames_to_slave + t.eoe_frames_from_slave);
    assert_eq!((eoe.bytes_to_slaves, eoe.bytes_from_slaves), (t.eoe_bytes_to_slave, t.eoe_bytes_from_slave));
    assert!(eoe.frames.iter().all(|f| f.ethertype == Some(flowlab_synth::EOE_ETHERTYPE) && f.fragments == 11));
    assert_eq!(a.mailbox.eoe_packets.len() as u64, eoe.frames_total);

    // Mailbox polling frames neither form streams nor count as WKC errors.
    let r = &a.report;
    assert_eq!(r.streams.len(), 1);
    assert_eq!(r.streams[0].wkc_events.len() as u64, truth.wkc_errors);
    assert!(r.findings.iter().any(|f| f.severity == Severity::Warn && f.title.contains("SDO transfers aborted")));
    assert!(r.findings.iter().any(|f| f.severity == Severity::Warn && f.title.contains("emergencies")));
    assert!(r.findings.iter().any(|f| f.title.contains("mailbox writes rejected")));
    assert!(!r.findings.iter().any(|f| f.title.contains("violated") || f.title.contains("EoE")));

    // Frame queries by protocol.
    let foe_frames = query(&trace, &a, &FrameFilter { mailbox: Some("foe".into()), ..Default::default() }, 0, 1000);
    // WRQ + 6 DATA + 7 ACK, each seen as TX and RX, plus rejected write attempts that never
    // carried a message do not count: only frames with an accepted message.
    assert_eq!(foe_frames.total, 2 * 14);
    assert!(foe_frames.rows.iter().any(|r| r.summary.contains("FoE DATA #6 0 B")));
    let any = query(&trace, &a, &FrameFilter { mailbox: Some("any".into()), ..Default::default() }, 0, 1);
    assert!(any.total > foe_frames.total);
    let no = query(&trace, &a, &FrameFilter { mailbox: Some("bogus".into()), ..Default::default() }, 0, 1);
    assert_eq!(no.total, 0);
}

#[test]
fn mailbox_without_configuration_uses_hints_or_heuristic() {
    let mut s = mailbox_heavy();
    s.include_startup = false; // no SyncManager configuration in the capture
    let mut bytes = Vec::new();
    let truth = generate(&s, &mut bytes).unwrap();
    let trace = Trace::build(CaptureFile::parse(bytes).unwrap(), "t.pcapng");
    let a = analyze(&trace, &AnalysisOptions::default());
    let m = &a.report.mailbox;
    assert!(m.channels.iter().all(|c| c.source == ChannelSource::Heuristic));
    assert_eq!(m.eoe.frames_total, truth.mailbox.eoe_frames_to_slave + truth.mailbox.eoe_frames_from_slave);
    assert!(a.report.meta.assumptions.iter().any(|x| x.contains("inferred")));

    let heuristic_out = m.channels.iter().find(|c| c.station == 0x1002).unwrap().messages_out;
    assert!(heuristic_out > 0);
    let hint: crate::mailbox::MailboxHint = "0x1002=0x1000,0x1080".parse().unwrap();
    let a = analyze(&trace, &AnalysisOptions { mailbox_hints: vec![hint], ..AnalysisOptions::default() });
    let c = a.report.mailbox.channels.iter().find(|c| c.station == 0x1002).unwrap();
    assert_eq!(c.source, ChannelSource::Hint);
    assert_eq!(c.messages_out, heuristic_out);
    assert!("0x1002=0x1000".parse::<crate::mailbox::MailboxHint>().is_err());
}

#[test]
fn example_spec_mailbox_matches_truth() {
    let (_, a, truth) = run(&SynthSpec::example());
    let m = &a.report.mailbox;
    let t = &truth.mailbox;
    assert!(m.channels.iter().all(|c| c.repeats == 0), "counters continue across PREOP and OP");
    assert_eq!(m.channels.iter().map(|c| c.messages_out).sum::<u64>(), t.messages_to_slaves);
    assert_eq!(m.channels.iter().map(|c| c.messages_in).sum::<u64>(), t.messages_from_slaves);
    assert_eq!((m.coe.ok, m.coe.aborted, m.coe.emergencies_total), (t.sdo_ok, t.sdo_aborted, t.emergencies));
    assert_eq!(m.eoe.frames_total, t.eoe_frames_to_slave + t.eoe_frames_from_slave);
    assert_eq!(m.eoe.reassembly_errors, 0);
    assert_eq!(m.foe.iter().map(|f| f.bytes).sum::<u64>(), t.foe_bytes);
}
