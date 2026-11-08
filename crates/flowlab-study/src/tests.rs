use super::*;
use flowlab_analysis::{AnalysisOptions, analyze};
use flowlab_capture::{CaptureFile, Trace};
use flowlab_sim::{SimSpec, TransferDirection, TransferProtocol, TransferSpec};
use serde_json::json;

fn param(path: &str, values: Vec<serde_json::Value>) -> Parameter {
    Parameter {
        path: path.into(),
        also: Vec::new(),
        values,
        min: None,
        max: None,
        steps: None,
        integer: false,
        log: false,
    }
}

#[test]
fn example_grid_has_a_consistent_pareto_frontier() {
    let spec = StudySpec::example();
    let r = run_study(&spec, None).unwrap();
    assert_eq!(r.runs.len(), 4 * 3 * 2);
    assert_eq!(r.feasible, 24);
    assert!(!r.pareto.is_empty());
    let obj = |i: u32| r.runs[i as usize].objectives.iter().map(|v| v.unwrap()).collect::<Vec<_>>();
    for &p in &r.pareto {
        for q in 0..r.runs.len() as u32 {
            let (a, b) = (obj(q), obj(p));
            let dominated = a.iter().zip(&b).all(|(x, y)| x <= y) && a.iter().zip(&b).any(|(x, y)| x < y);
            assert!(!dominated, "Pareto run {p} is dominated by {q}");
        }
        assert!(r.runs[p as usize].spec.is_some(), "Pareto runs keep their spec");
    }
    // The fastest run is on the frontier and is reported as best.
    let fastest = (0..24u32).min_by(|&a, &b| obj(a)[0].total_cmp(&obj(b)[0])).unwrap();
    assert_eq!(r.best, Some(fastest));
    // Larger mailboxes and a faster PDI never make the FoE write slower at the same policy.
    let t = |mbx: u64, rate: u64, policy: &str| {
        r.runs
            .iter()
            .find(|x| x.values[0] == mbx.to_string() && x.values[1] == rate.to_string() && x.values[2].contains(policy))
            .unwrap()
            .objectives[0]
            .unwrap()
    };
    assert!(t(1024, 80_000_000, "fixed") <= t(128, 80_000_000, "fixed"));
    assert!(t(512, 80_000_000, "opportunistic") <= t(512, 5_000_000, "opportunistic"));
    assert_eq!(r.meta.source, flowlab_core::Provenance::Simulated);
}

#[test]
fn constraints_reject_runs_with_their_reason() {
    let mut spec = StudySpec::example();
    spec.constraints = vec![Constraint { metric: Metric::DpramUsedPercentMax, max: Some(40.0), min: None }];
    let r = run_study(&spec, None).unwrap();
    let big: Vec<_> = r.runs.iter().filter(|x| x.values[0] == "1024").collect();
    assert!(big.iter().all(|x| !x.feasible && x.reason.as_deref().unwrap().contains("ESC RAM")));
    assert!(r.pareto.iter().all(|&p| r.runs[p as usize].values[0] != "1024"));
    // A spec the simulator rejects is an infeasible run, not a failed study.
    let mut spec = StudySpec::example();
    spec.parameters = vec![param("/slaves/2/dpram_bytes", vec![json!(100), json!(4096)])];
    let r = run_study(&spec, None).unwrap();
    assert!(!r.runs[0].feasible && r.runs[0].reason.as_deref().unwrap().contains("ESC RAM"));
    assert!(r.runs[1].feasible);
}

#[test]
fn sampling_rules() {
    let mut spec = StudySpec::example();
    spec.max_runs = 10;
    assert!(run_study(&spec, None).unwrap_err().to_string().contains("max_runs"));
    spec.sampling = Sampling::LatinHypercube { samples: 4, seed: 7 };
    spec.parameters.truncate(1); // 4 mailbox sizes, 4 samples: each used once
    let r = run_study(&spec, None).unwrap();
    let mut used: Vec<_> = r.runs.iter().map(|x| x.values[0].clone()).collect();
    used.sort();
    assert_eq!(used, ["1024", "128", "256", "512"]);
    let again = run_study(&spec, None).unwrap();
    assert_eq!(r, again, "deterministic");
    let mut bad = StudySpec::example();
    bad.parameters = vec![param("/no/such/field", vec![json!(1)])];
    assert!(run_study(&bad, None).unwrap_err().to_string().contains("not in the base spec"));
    let mut range = StudySpec::example();
    range.parameters = vec![Parameter {
        min: Some(1e6),
        max: Some(1e8),
        steps: Some(3),
        integer: true,
        log: true,
        values: Vec::new(),
        ..param("/slaves/2/pdi/read_bps", Vec::new())
    }];
    let r = run_study(&range, None).unwrap();
    let v: Vec<_> = r.runs.iter().map(|x| x.values[0].as_str()).collect();
    assert_eq!(v, ["1000000", "10000000", "100000000"]);
}

/// A "measurement" made by the simulator with hidden PDI parameters, analysed like a capture;
/// the calibration must find the parameters that reproduce it.
#[test]
fn calibration_recovers_hidden_pdi_parameters_from_a_capture() {
    let mut truth = SimSpec::example();
    truth.transfers = vec![
        TransferSpec {
            slave: 2,
            direction: TransferDirection::MasterToSlave,
            payload_bytes: 20_000,
            protocol: TransferProtocol::Foe,
            start_ms: 0.0,
        },
        TransferSpec {
            slave: 2,
            direction: TransferDirection::MasterToSlave,
            payload_bytes: 512,
            protocol: TransferProtocol::CoeSegmented,
            start_ms: 0.0,
        },
    ];
    truth.mailbox_policy = flowlab_sim::MailboxPolicy::Opportunistic { bus_budget_percent: 90.0, turnaround_ns: 5_000 };
    let hidden_rate = 8_000_000u64;
    let hidden_overhead = 40_000u32;
    let mut measured = truth.clone();
    measured.slaves[2].pdi.read_bps = hidden_rate;
    measured.slaves[2].pdi.write_bps = hidden_rate;
    measured.slaves[2].pdi.access_overhead_ns = hidden_overhead;
    let mut bytes = Vec::new();
    flowlab_sim::simulate(&measured, Some(&mut bytes)).unwrap();
    let trace = Trace::build(CaptureFile::parse(bytes).unwrap(), "lab.pcapng");
    let report = analyze(&trace, &AnalysisOptions::default()).report;

    let spec = StudySpec {
        description: "fit PDI rate and overhead".into(),
        base: truth,
        parameters: vec![
            Parameter {
                also: vec!["/slaves/2/pdi/write_bps".into()],
                ..param(
                    "/slaves/2/pdi/read_bps",
                    [2e6, 4e6, 8e6, 16e6, 32e6].iter().map(|v| json!(*v as u64)).collect(),
                )
            },
            param("/slaves/2/pdi/access_overhead_ns", [0, 10_000, 40_000, 160_000].map(|v| json!(v)).to_vec()),
        ],
        sampling: Sampling::Grid,
        objectives: Vec::new(),
        constraints: Vec::new(),
        targets: vec![
            Target {
                metric: Metric::ExchangeTimeS { transfer: 0 },
                measured: None,
                from_capture: Some(CaptureMetric::FoeDurationS { index: 0 }),
                weight: 1.0,
            },
            Target {
                metric: Metric::ExchangeTimeS { transfer: 1 },
                measured: None,
                from_capture: Some(CaptureMetric::SdoDurationS { index: 0 }),
                weight: 1.0,
            },
        ],
        max_runs: 400,
    };
    assert!(run_study(&spec, None).is_err(), "targets read the capture");
    let r = run_study(&spec, Some(&report)).unwrap();
    let best = &r.runs[r.best.unwrap() as usize];
    assert_eq!(best.values, [hidden_rate.to_string(), hidden_overhead.to_string()], "{:#?}", r.residuals);
    // Same model, same frames: the capture's message spans are reproduced exactly.
    assert!(best.objectives[0].unwrap() < 1e-9, "fit error {:?}", best.objectives[0]);
    assert_eq!(r.residuals.len(), 2);
    assert!(r.residuals.iter().all(|x| x.relative_error.unwrap().abs() < 1e-9), "{:#?}", r.residuals);
    assert_eq!(r.calibrated_against.as_ref().unwrap().name, "lab.pcapng");
    assert!(r.meta.assumptions.iter().any(|a| a.contains("not that its parameters are the physical ones")));
}

/// The shipped calibration example finds the parameters a capture was made with, when that
/// capture comes from its own model (here: simulated, with values from its grid).
#[test]
fn shipped_calibration_example_recovers_its_own_capture() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../configs/calibration/foe-span.json");
    let spec: StudySpec = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut hidden = spec.base.clone();
    hidden.slaves[2].pdi.read_bps = 10_000_000;
    hidden.slaves[2].pdi.write_bps = 10_000_000;
    hidden.slaves[2].pdi.access_overhead_ns = 20_000;
    hidden.slaves[2].app_processing_ns = 40_000;
    hidden.capture.enabled = true;
    let mut bytes = Vec::new();
    flowlab_sim::simulate(&hidden, Some(&mut bytes)).unwrap();
    let trace = Trace::build(CaptureFile::parse(bytes).unwrap(), "hidden.pcapng");
    let report = analyze(&trace, &AnalysisOptions::default()).report;
    assert_eq!(report.meta.source, flowlab_core::Provenance::Simulated);

    let r = run_study(&spec, Some(&report)).unwrap();
    let best = &r.runs[r.best.unwrap() as usize];
    assert_eq!(best.values, ["10000000", "20000", "40000"], "{:#?}", r.residuals);
    assert!(r.residuals.iter().all(|x| x.relative_error.unwrap().abs() < 1e-9), "{:#?}", r.residuals);
}

#[test]
fn shipped_study_is_the_example() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../configs/studies/bulk-transfer/foe-sweep.json");
    let spec: StudySpec = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(spec, StudySpec::example());
}

/// The simulator's master policy, captured and analysed, is read back as the same policy.
#[test]
fn analysis_reads_the_master_policy_back_from_a_simulated_capture() {
    use flowlab_sim::MailboxPolicy;
    let run = |policy: MailboxPolicy| {
        let mut spec = SimSpec::example();
        spec.transfers = vec![TransferSpec {
            slave: 2,
            direction: TransferDirection::MasterToSlave,
            payload_bytes: 60_000,
            protocol: TransferProtocol::Foe,
            start_ms: 0.0,
        }];
        spec.mailbox_policy = policy;
        let mut bytes = Vec::new();
        flowlab_sim::simulate(&spec, Some(&mut bytes)).unwrap();
        let trace = Trace::build(CaptureFile::parse(bytes).unwrap(), "lab.pcapng");
        let report = analyze(&trace, &AnalysisOptions::default()).report;
        let services: Vec<_> = report.mailbox.channels.iter().filter_map(|c| c.service.clone()).collect();
        assert_eq!(services.len(), 1, "one mailbox slave is busy");
        assert!(report.meta.assumptions.iter().any(|a| a.contains("Mailbox service pattern")));
        services.into_iter().next().unwrap()
    };

    let s = run(MailboxPolicy::FixedQuota { accesses_per_cycle: 2 });
    assert_eq!(s.pattern, MailboxPolicy::FixedQuota { accesses_per_cycle: 2 }, "{s:#?}");
    assert_eq!(s.policy, s.pattern.to_string());
    assert_eq!(s.frames_per_cycle.p50, 1.0);

    let s = run(MailboxPolicy::Opportunistic { bus_budget_percent: 60.0, turnaround_ns: 10_000 });
    let MailboxPolicy::Opportunistic { bus_budget_percent, turnaround_ns } = s.pattern else {
        panic!("expected back-to-back polling: {s:#?}")
    };
    // The last frame of a cycle starts before the budget ends: the estimate is a little below it.
    assert!((50.0..=60.0).contains(&bus_budget_percent), "{bus_budget_percent}");
    assert_eq!(turnaround_ns, 10_000);
}

fn edited(spec: &SimSpec, change: &WhatIf) -> SimSpec {
    let mut v = serde_json::to_value(spec).unwrap();
    for e in &change.edits {
        *v.pointer_mut(&e.path).unwrap() = e.value.clone();
    }
    serde_json::from_value(v).unwrap()
}

#[test]
fn what_if_ranks_one_change_runs_and_its_edits_reproduce_them() {
    let mut spec = SimSpec::example();
    spec.transfers.truncate(1);
    spec.transfers[0].payload_bytes = 64_000;
    let r = what_if(&spec, &WhatIfOptions::default()).unwrap();
    assert!(r.base.feasible, "{:?}", r.base.reason);
    assert!(!r.changes.is_empty());
    // Feasible first, earliest finish first.
    let finishes: Vec<f64> = r.changes.iter().map_while(|c| c.run.finish_ns).collect();
    assert!(finishes.windows(2).all(|w| w[0] <= w[1]));
    assert!(r.changes[finishes.len()..].iter().all(|c| !c.run.feasible && c.run.reason.is_some()));
    // The best change helps, and its edits alone reproduce its run.
    let best = &r.changes[0];
    assert!(best.run.finish_ns.unwrap() < r.base.finish_ns.unwrap(), "{}", best.label);
    let again = flowlab_sim::simulate(&edited(&spec, best), None::<std::io::Sink>).unwrap();
    assert_eq!(again.transfers[0].duration_ns, best.run.transfer_ns[0]);
    // Every lever the scenario has is tried: PDI, mailbox, policy, protocol, master.
    for lever in [Lever::Pdi, Lever::Mailbox, Lever::MailboxPolicy, Lever::Protocol, Lever::Master] {
        assert!(r.changes.iter().any(|c| c.lever == lever), "{lever:?} not tried");
    }
    // Slaves without transfers are left alone.
    assert!(r.changes.iter().all(|c| c.slave.is_none_or(|s| s == 2)));
    // One ranking for the whole run, one per transfer; the best change helps.
    assert_eq!(r.rankings.len(), 2);
    let all = &r.rankings[0];
    assert_eq!(all.transfer, None);
    let first_single = r.changes.iter().position(|c| c.parts.is_empty()).unwrap() as u32;
    assert_eq!(all.helps[0].change, first_single);
    let deltas: Vec<f64> = all.helps.iter().map(|h| h.delta.unwrap()).collect();
    assert!(deltas.windows(2).all(|w| w[0] <= w[1]) && deltas.iter().all(|d| *d < -r.no_effect));
    assert!(all.flat.iter().all(|f| f.delta.unwrap().abs() <= r.no_effect));
    let n = all.helps.len() + all.flat.len() + all.hurts.len() + all.impossible.len();
    assert_eq!(n, r.changes.iter().filter(|c| c.parts.is_empty()).count());
}

#[test]
fn what_if_lists_a_pair_only_when_it_beats_both_parts_and_the_scenario() {
    let mut spec = SimSpec::example();
    spec.transfers.truncate(1);
    spec.transfers[0].payload_bytes = 64_000;
    let r = what_if(&spec, &WhatIfOptions::default()).unwrap();
    let alone = |label: &str| {
        let i = r.changes.iter().position(|c| c.parts.is_empty() && c.label == label).unwrap() as u32;
        let all = &r.rankings[0];
        [&all.helps, &all.flat, &all.hurts, &all.impossible]
            .iter()
            .flat_map(|l| l.iter())
            .find(|x| x.change == i)
            .and_then(|x| x.delta)
            .unwrap_or(f64::INFINITY)
    };
    for p in &r.rankings[0].together {
        let c = &r.changes[p.change as usize];
        let d = p.delta.unwrap();
        assert!(d < -r.no_effect, "{}: {d}", c.label);
        assert!(c.parts.iter().all(|part| d < alone(part).min(0.0) - r.no_effect), "{}", c.label);
    }
    assert!(what_if(&spec, &WhatIfOptions { no_effect: 1.5, ..Default::default() }).is_err());
}

#[test]
fn what_if_reports_why_a_change_does_not_fit() {
    let mut spec = SimSpec::example();
    spec.transfers.truncate(2);
    spec.slaves[2].dpram_bytes = 1200; // 2 × 512 B mailboxes + process data fit; doubled they do not
    let r = what_if(&spec, &WhatIfOptions::default()).unwrap();
    let mbx = r.changes.iter().find(|c| c.lever == Lever::Mailbox).unwrap();
    assert!(!mbx.run.feasible);
    assert!(mbx.run.reason.as_deref().unwrap().contains("RAM"), "{:?}", mbx.run.reason);
    assert!(mbx.run.finish_ns.is_none());
}
