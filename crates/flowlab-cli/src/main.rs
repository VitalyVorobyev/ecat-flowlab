//! `flowlab`: generate synthetic captures, analyze captures, and serve the UI.

use std::fs::File;
use std::io::BufWriter;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use flowlab_analysis::{AnalysisOptions, CaptureReport, Severity, analyze};
use flowlab_capture::{CaptureFile, Trace};
use flowlab_synth::SynthSpec;

#[derive(Parser)]
#[command(name = "flowlab", version, about = "ECAT FlowLab: EtherCAT capture analysis and simulation workbench")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Generate a synthetic pcapng with known ground truth from a JSON spec.
    Synth {
        /// Spec file (see `flowlab schema synth`). Omit to use the built-in demo spec.
        spec: Option<PathBuf>,
        /// Output capture.
        #[arg(short, long)]
        out: PathBuf,
        /// Also write the ground truth as JSON.
        #[arg(long)]
        truth: Option<PathBuf>,
    },
    /// Analyze a pcap/pcapng capture.
    Analyze {
        /// Capture file.
        capture: PathBuf,
        /// Write the JSON report here.
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Print the JSON report to stdout instead of the summary.
        #[arg(long)]
        json: bool,
        /// Link rate for utilization, Mbit/s.
        #[arg(long, default_value_t = 100)]
        link_mbps: u64,
        /// Minimum repetitions for a cyclic stream.
        #[arg(long, default_value_t = 20)]
        min_stream_frames: usize,
    },
    /// Serve the web UI and API on localhost.
    Serve {
        /// Captures to load at start.
        captures: Vec<PathBuf>,
        /// Port.
        #[arg(short, long, default_value_t = 7878)]
        port: u16,
        /// Address to bind. The API reads local files and has no authentication:
        /// keep the default loopback address.
        #[arg(long, default_value = "127.0.0.1")]
        host: IpAddr,
        /// Directory of the built UI.
        #[arg(long, default_value = "ui/dist")]
        ui_dir: PathBuf,
        /// Open the UI in the default browser.
        #[arg(long)]
        open: bool,
    },
    /// Print a JSON Schema.
    Schema {
        /// Which schema.
        #[arg(value_parser = ["synth"])]
        which: String,
    },
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with_writer(std::io::stderr)
        .init();
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Cmd::Synth { spec, out, truth } => {
            let spec: SynthSpec = match spec {
                Some(p) => serde_json::from_reader(File::open(&p).map_err(|e| format!("{}: {e}", p.display()))?)?,
                None => SynthSpec::example(),
            };
            ensure_parent(&out)?;
            let gt = flowlab_synth::generate(&spec, BufWriter::new(File::create(&out)?))?;
            if let Some(t) = truth {
                ensure_parent(&t)?;
                serde_json::to_writer_pretty(File::create(&t)?, &gt)?;
            }
            eprintln!(
                "wrote {} frames to {} (synthetic, seed {}): {} cyclic, {} lost, {} WKC errors, {} skipped cycles",
                gt.frames,
                out.display(),
                gt.seed,
                gt.cyclic_tx_frames,
                gt.lost_frames,
                gt.wkc_errors,
                gt.skipped_cycles
            );
        }
        Cmd::Analyze { capture, out, json, link_mbps, min_stream_frames } => {
            let file = CaptureFile::read(&capture).map_err(|e| format!("{}: {e}", capture.display()))?;
            let name = capture.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let trace = Trace::build(file, &name);
            let opts = AnalysisOptions { link_bps: link_mbps * 1_000_000, min_stream_frames, ..Default::default() };
            let a = analyze(&trace, &opts);
            if let Some(o) = &out {
                ensure_parent(o)?;
                serde_json::to_writer_pretty(BufWriter::new(File::create(o)?), &a.report)?;
            }
            if json {
                println!("{}", serde_json::to_string_pretty(&a.report)?);
            } else {
                print_summary(&a.report);
            }
        }
        Cmd::Serve { captures, port, host, ui_dir, open } => {
            if !host.is_loopback() {
                tracing::warn!("binding to {host}: the API can read local files and has no authentication");
            }
            let state = flowlab_server::AppState::new(AnalysisOptions::default());
            for c in &captures {
                let info = state.load_path(c).map_err(|e| format!("{}: {e}", c.display()))?;
                tracing::info!("loaded {} ({} frames)", info.name, info.frames);
            }
            let ui = ui_dir.join("index.html").is_file().then_some(ui_dir.clone());
            if ui.is_none() {
                tracing::warn!(
                    "no built UI at {} (run `bun run build` in ui/); serving the API only",
                    ui_dir.display()
                );
            }
            let addr = SocketAddr::new(host, port);
            if open {
                let _ = open::that(format!("http://{addr}"));
            }
            tokio::runtime::Runtime::new()?.block_on(flowlab_server::serve(state, addr, ui))?;
        }
        Cmd::Schema { which } => {
            debug_assert_eq!(which, "synth");
            println!("{}", serde_json::to_string_pretty(&flowlab_synth::spec_schema())?);
        }
    }
    Ok(())
}

fn ensure_parent(p: &Path) -> Result<()> {
    if let Some(d) = p.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(d)?;
    }
    Ok(())
}

fn ns(v: f64) -> String {
    let a = v.abs();
    if a >= 1e9 {
        format!("{:.3} s", v / 1e9)
    } else if a >= 1e6 {
        format!("{:.3} ms", v / 1e6)
    } else if a >= 1e3 {
        format!("{:.2} µs", v / 1e3)
    } else {
        format!("{v:.0} ns")
    }
}

fn print_summary(r: &CaptureReport) {
    let s = &r.summary;
    println!(
        "{}  [{}]  sha256 {}",
        r.meta.input.name,
        r.meta.source.as_str().to_uppercase(),
        &r.meta.input.sha256[..16]
    );
    println!(
        "  {} frames ({} EtherCAT, {} other, {} malformed), {} datagrams over {}",
        s.frames,
        s.ethercat_frames,
        s.other_frames,
        s.malformed_frames,
        s.datagrams,
        ns(s.duration_ns as f64)
    );
    println!(
        "  directions: {:?}; {} TX, {} RX, {} paired, {} unanswered; slaves responding: {}",
        s.direction_mode,
        s.tx_frames,
        s.rx_frames,
        s.paired_frames,
        s.unanswered_frames,
        s.slaves_responding.map_or("?".into(), |n| n.to_string())
    );
    println!(
        "  utilization TX mean {:.2} %, peak {:.2} % (bins of {})",
        r.utilization.mean_tx_percent,
        r.utilization.peak_tx_percent,
        ns(r.utilization.bin_ns)
    );
    for st in &r.streams {
        println!("  stream {}: {}", st.id, st.signature);
        println!(
            "    {} frames, period {}, |jitter| p99 {}, missed {}, unanswered {}, rtt p50 {}",
            st.frames,
            ns(st.period_ns),
            st.phase_jitter_abs_p99_ns.map_or("-".into(), ns),
            st.missed_cycles,
            st.unanswered,
            st.round_trip.as_ref().map_or("-".into(), |d| ns(d.p50)),
        );
    }
    if !r.stations.is_empty() {
        println!(
            "  stations: {}",
            r.stations.iter().map(|s| format!("0x{:04X}", s.station)).collect::<Vec<_>>().join(" ")
        );
    }
    for f in &r.findings {
        let tag = match f.severity {
            Severity::Info => "info",
            Severity::Warn => "WARN",
            Severity::Defect => "DEFECT",
        };
        println!("  [{tag}] {}", f.title);
    }
}
