use super::*;
use flowlab_capture::{CaptureFile, Direction, Trace};

pub(crate) fn spec() -> SynthSpec {
    SynthSpec {
        seed: 7,
        description: "test".into(),
        slaves: vec![
            SlaveSpec { name: "coupler".into(), outputs_bytes: 0, inputs_bytes: 0, mailbox: false },
            SlaveSpec { name: "di".into(), outputs_bytes: 0, inputs_bytes: 2, mailbox: true },
            SlaveSpec { name: "do".into(), outputs_bytes: 2, inputs_bytes: 0, mailbox: true },
            SlaveSpec { name: "drive".into(), outputs_bytes: 12, inputs_bytes: 16, mailbox: true },
        ],
        cycle_time_us: 1000,
        cycles: 500,
        tx_jitter_ns: 5_000,
        slave_delay_ns: 700,
        include_startup: true,
        sdo_uploads_per_slave: 2,
        distributed_clocks: true,
        start_unix_s: 1_767_225_600,
        mailbox: Default::default(),
        faults: FaultSpec {
            lost_frame_probability: 0.01,
            wkc_error_probability: 0.02,
            skipped_cycle_probability: 0.005,
            mailbox_write_reject_probability: 0.0,
        },
    }
}

#[test]
fn deterministic_bytes() {
    let mut a = Vec::new();
    let mut b = Vec::new();
    let ta = generate(&spec(), &mut a).unwrap();
    let tb = generate(&spec(), &mut b).unwrap();
    assert_eq!(a, b);
    assert_eq!(ta, tb);
    let mut other = spec();
    other.seed = 8;
    let mut c = Vec::new();
    generate(&other, &mut c).unwrap();
    assert_ne!(a, c);
}

#[test]
fn counts_match_capture() {
    let mut bytes = Vec::new();
    let truth = generate(&spec(), &mut bytes).unwrap();
    assert_eq!(truth.expected_lrw_wkc, 1 + 2 + 3);
    assert_eq!(truth.cyclic_tx_frames + truth.skipped_cycles, 500);
    assert!(truth.lost_frames > 0 && truth.wkc_errors > 0, "{truth:?}");
    assert_eq!(truth.sdo_uploads, 6);
    let trace = Trace::build(CaptureFile::parse(bytes).unwrap(), "synth.pcapng");
    assert_eq!(trace.provenance, Provenance::Synthetic);
    assert!(trace.errors.is_empty(), "{:?}", trace.errors);
    assert_eq!(trace.frames.len() as u64, truth.frames);
    let tx = trace.frames.iter().filter(|f| f.direction == Direction::Tx).count() as u64;
    let rx = trace.frames.iter().filter(|f| f.direction == Direction::Rx).count() as u64;
    assert_eq!((tx, rx), (truth.tx_frames, truth.rx_frames));
}

#[test]
fn rejects_bad_spec() {
    let mut s = spec();
    s.slaves.clear();
    assert!(matches!(generate(&s, Vec::new()), Err(SynthError::Spec(_))));
    let mut s = spec();
    s.faults.lost_frame_probability = 1.5;
    assert!(generate(&s, Vec::new()).is_err());
}

#[test]
fn schema_is_draft_2020_12() {
    let schema = spec_schema();
    assert!(schema["$schema"].as_str().unwrap().contains("2020-12"));
    assert!(schema["properties"]["cycle_time_us"].is_object());
}
