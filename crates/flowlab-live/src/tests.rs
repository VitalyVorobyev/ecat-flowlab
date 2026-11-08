use std::time::Duration;

use flowlab_analysis::{AnalysisOptions, ChannelSource, analyze};
use flowlab_capture::{CaptureFile, Trace};
use flowlab_core::Provenance;

use super::*;

fn demo() -> Vec<u8> {
    let mut bytes = Vec::new();
    flowlab_synth::generate(&flowlab_synth::SynthSpec::example(), &mut bytes).unwrap();
    bytes
}

fn fast() -> LiveOptions {
    LiveOptions { tick: Duration::from_millis(40), ..LiveOptions::default() }
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("flowlab-live-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

#[test]
fn a_replay_is_counted_completely_and_saved_byte_for_byte() {
    let bytes = demo();
    let whole = analyze(&Trace::build(CaptureFile::parse(bytes.clone()).unwrap(), "demo"), &AnalysisOptions::default());
    let path = scratch("replay.pcapng");
    let options = LiveOptions { save_to: Some(path.clone()), ..fast() };
    // Slow enough for several ticks even where one tick's analysis takes a few hundred ms.
    let source = Source::Replay { bytes: bytes.clone(), name: "demo.pcapng".into(), speed: 5.0 };
    let done = LiveSession::start(source, options).unwrap().wait();

    assert_eq!(done.state, LiveState::Finished);
    assert_eq!(done.message.as_deref(), Some("end of the replayed file"));
    assert_eq!(done.provenance, Some(Provenance::Synthetic));
    // Every frame lands in exactly one tick.
    assert_eq!(done.totals.ethercat_frames, whole.report.summary.ethercat_frames);
    assert_eq!(done.packets, whole.report.summary.frames);
    assert!(done.series.len() >= 3, "a 1 s replay spans several 40 ms ticks");
    assert!(done.totals.mailbox_messages > 0);
    let period = done.series.iter().filter_map(|p| p.period_ns).next().unwrap();
    assert!((period - 1e6).abs() < 1e3, "1 ms cycle, got {period} ns");
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}

#[test]
fn the_window_keeps_recent_traffic_and_mailbox_addresses_outlive_it() {
    let options = LiveOptions { window: Duration::from_millis(500), ..fast() };
    // About 0.08 s of capture per tick, 2.5 s in all: several ticks even on a slow machine.
    let source = Source::Replay { bytes: demo(), name: "demo.pcapng".into(), speed: 2.0 };
    let done = LiveSession::start(source, options).unwrap().wait();
    let report = done.window.unwrap();
    let span = report.summary.duration_ns as f64;
    assert!(span <= 0.5e9 + 2e6, "window spans {span} ns");
    assert!(span > 0.3e9);
    // The SyncManager configuration is in the first tick only; later ticks find the mailbox
    // messages through the addresses learned from it.
    let whole = analyze(&Trace::build(CaptureFile::parse(demo()).unwrap(), "demo"), &AnalysisOptions::default());
    let messages: u64 = whole.report.mailbox.channels.iter().map(|c| c.messages_in + c.messages_out).sum();
    let later: u64 = done.series[1..].iter().map(|p| p.mailbox_messages).sum();
    assert!(later > 0);
    assert!(done.totals.mailbox_messages.abs_diff(messages) <= 2, "{} vs {messages}", done.totals.mailbox_messages);
    assert!(report.mailbox.channels.iter().all(|c| c.source == ChannelSource::Hint));
}

#[test]
fn bad_options_are_rejected() {
    let source = Source::Replay { bytes: demo(), name: "x".into(), speed: 0.0 };
    assert!(matches!(LiveSession::start(source, fast()), Err(LiveError::Option(_))));
    let source = Source::Replay { bytes: demo(), name: "x".into(), speed: 1.0 };
    let options = LiveOptions { tick: Duration::ZERO, ..fast() };
    assert!(matches!(LiveSession::start(source, options), Err(LiveError::Option(_))));
}

#[test]
fn stopping_a_replay_ends_it_early() {
    let source = Source::Replay { bytes: demo(), name: "x".into(), speed: 1.0 };
    let session = LiveSession::start(source, fast()).unwrap();
    std::thread::sleep(Duration::from_millis(150));
    let done = session.stop();
    assert_eq!(done.state, LiveState::Finished);
    assert!(done.message.is_none());
    assert!(done.packets > 0);
    assert!(done.totals.ethercat_frames < 9000, "stopped after about 0.15 s of a 5 s file");
}

#[cfg(unix)]
mod command {
    use super::*;

    #[test]
    fn a_tool_writing_a_capture_to_stdout_is_read_and_saved() {
        let bytes = demo();
        let input = scratch("input.pcapng");
        std::fs::write(&input, &bytes).unwrap();
        let out = scratch("cat.pcapng");
        let source = Source::Command { program: "cat".into(), args: vec![input.into()], label: "cat".into() };
        let done = LiveSession::start(source, LiveOptions { save_to: Some(out.clone()), ..fast() }).unwrap().wait();
        assert_eq!(done.state, LiveState::Finished);
        assert_eq!(done.message.as_deref(), Some("the capture tool exited"));
        assert!(done.totals.ethercat_frames > 9000);
        assert_eq!(std::fs::read(&out).unwrap(), bytes);
    }

    #[test]
    fn a_failing_tool_reports_its_error_output() {
        let source = Source::Command {
            program: "sh".into(),
            args: vec!["-c".into(), "echo 'eth9: no such device' >&2; exit 2".into()],
            label: "sh".into(),
        };
        let done = LiveSession::start(source, fast()).unwrap().wait();
        assert_eq!(done.state, LiveState::Failed);
        assert_eq!(done.message.as_deref(), Some("eth9: no such device"));
    }

    #[test]
    fn output_that_is_not_a_capture_fails_the_session() {
        let source = Source::Command { program: "echo".into(), args: vec!["hello".into()], label: "echo".into() };
        let done = LiveSession::start(source, fast()).unwrap().wait();
        assert_eq!(done.state, LiveState::Failed);
        assert!(done.message.unwrap().contains("not a capture"));
    }

    #[test]
    fn a_missing_program_does_not_start() {
        let source =
            Source::Command { program: "/nonexistent/dumpcap".into(), args: vec![], label: "dumpcap on eth1".into() };
        assert!(matches!(LiveSession::start(source, fast()), Err(LiveError::Spawn(..))));
    }
}
