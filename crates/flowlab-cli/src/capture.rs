//! `flowlab capture`: live capture through dumpcap or tcpdump, analysed while it runs.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use flowlab_analysis::AnalysisOptions;
use flowlab_live::{ETHERCAT_FILTER, LiveOptions, LivePoint, LiveSession, LiveState, Source, Tool};

use crate::{Result, ensure_parent, ns};

/// What `flowlab capture` was asked to do.
pub(crate) struct CaptureArgs {
    pub(crate) interface: Option<String>,
    pub(crate) list: bool,
    pub(crate) tool: Option<String>,
    pub(crate) out: Option<PathBuf>,
    pub(crate) duration: Option<f64>,
    pub(crate) window: f64,
    pub(crate) all_traffic: bool,
    pub(crate) replay: Option<PathBuf>,
    pub(crate) speed: f64,
    pub(crate) report: Option<PathBuf>,
    pub(crate) analysis: AnalysisOptions,
}

fn tool(name: Option<&str>) -> Option<Tool> {
    match name {
        Some("dumpcap") => Some(Tool::Dumpcap),
        Some("tcpdump") => Some(Tool::Tcpdump),
        _ => None,
    }
}

pub(crate) fn run(a: CaptureArgs) -> Result<()> {
    if a.list {
        let (t, path) = flowlab_live::find_tool(tool(a.tool.as_deref()))?;
        eprintln!("interfaces {} can open ({}):", t.name(), path.display());
        for i in flowlab_live::list_interfaces(t, &path)? {
            match i.description {
                Some(d) => println!("{}\t{d}", i.name),
                None => println!("{}", i.name),
            }
        }
        return Ok(());
    }
    let (source, default_out) = match (&a.replay, &a.interface) {
        (Some(file), _) => {
            let bytes = std::fs::read(file).map_err(|e| format!("{}: {e}", file.display()))?;
            let name = crate::file_name(file);
            (Source::Replay { bytes, name, speed: a.speed }, None)
        }
        (None, Some(interface)) => {
            let (t, path) = flowlab_live::find_tool(tool(a.tool.as_deref()))?;
            let filter = (!a.all_traffic).then_some(ETHERCAT_FILTER);
            let out = flowlab_live::default_save_path(Path::new("results/live"), t);
            (Source::interface(t, path, interface, filter), Some(out))
        }
        (None, None) => {
            return Err("name an interface (`flowlab capture --list` shows them) or use --replay FILE".into());
        }
    };
    let save_to = a.out.or(default_out);
    if let Some(p) = &save_to {
        ensure_parent(p)?;
    }
    if a.window.is_nan() || a.window <= 0.0 {
        return Err("--window must be positive".into());
    }
    let options = LiveOptions {
        window: Duration::from_secs_f64(a.window),
        save_to: save_to.clone(),
        analysis: AnalysisOptions { max_series_points: 300, ..a.analysis },
        ..LiveOptions::default()
    };
    let session = LiveSession::start(source, options)?;
    eprintln!("{}; Ctrl-C stops", session.snapshot().source);
    if let Some(p) = &save_to {
        eprintln!("saving to {}", p.display());
    }
    println!(
        "{:>6}  {:>7}  {:>10}  {:>10}  {:>10}  {:>5}  {:>5}  {:>6}  {:>7}",
        "time", "frames", "cycle", "jitter p99", "RTT p99", "WKC", "unans", "missed", "mailbox"
    );
    let deadline = a.duration.map(|d| Instant::now() + Duration::from_secs_f64(d.max(0.0)));
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    let mut last = f64::NEG_INFINITY;
    rt.block_on(async {
        let mut ctrl_c = std::pin::pin!(tokio::signal::ctrl_c());
        loop {
            tokio::select! {
                _ = &mut ctrl_c => break,
                _ = tokio::time::sleep(Duration::from_millis(250)) => {}
            }
            for p in session.points_after(last) {
                print_point(&p);
                last = p.t_s;
            }
            if session.is_done() || deadline.is_some_and(|d| Instant::now() >= d) {
                break;
            }
        }
    });
    let done = session.stop();
    for p in done.series.iter().filter(|p| p.t_s > last) {
        print_point(p);
    }
    if let Some(m) = &done.message {
        eprintln!("{m}");
    }
    let t = &done.totals;
    eprintln!(
        "{}: {} packets in {:.1} s; {} EtherCAT frames, {} WKC mismatches, {} unanswered, {} missed cycles, {} mailbox messages",
        match done.state {
            LiveState::Failed => "failed",
            _ => "stopped",
        },
        done.packets,
        done.elapsed_s,
        t.ethercat_frames,
        t.wkc_mismatches,
        t.unanswered,
        t.missed_cycles,
        t.mailbox_messages
    );
    if let (Some(path), Some(report)) = (&a.report, &done.window) {
        ensure_parent(path)?;
        serde_json::to_writer_pretty(std::io::BufWriter::new(std::fs::File::create(path)?), report)?;
        eprintln!("wrote the last window's report to {}", path.display());
    }
    if let Some(p) = &done.saved_to {
        if done.packets == 0 {
            // Nothing but headers, if anything: not worth keeping.
            let _ = std::fs::remove_file(p);
        } else {
            eprintln!("saved {p}; `flowlab analyze {p}` reads it in full");
        }
    }
    if done.state == LiveState::Failed { Err("capture failed".into()) } else { Ok(()) }
}

fn print_point(p: &LivePoint) {
    let opt = |v: Option<f64>| v.map_or_else(|| "–".to_owned(), ns);
    println!(
        "{:>5.0}s  {:>7}  {:>10}  {:>10}  {:>10}  {:>5}  {:>5}  {:>6}  {:>7}",
        p.t_s,
        p.frames,
        opt(p.period_ns),
        opt(p.jitter_p99_ns),
        opt(p.round_trip_p99_ns),
        p.wkc_mismatches,
        p.unanswered,
        p.missed_cycles,
        p.mailbox_messages
    );
}
