//! `flowlab`: analyse captures (or capture live), generate synthetic ones, simulate, run studies, plan networks,
//! read ESI files, time a LAN9252's SPI, turn lab measurements into simulator inputs and serve the UI. `flowlab help`
//! lists the subcommands.

use std::fs::File;
use std::io::BufWriter;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

mod capture;
mod master;
mod pdi_bench;

use clap::{Parser, Subcommand};
use flowlab_analysis::mailbox::{MailboxHint, write_eoe_pcapng};
use flowlab_analysis::{AnalysisOptions, CaptureReport, Severity, analyze};
use flowlab_capture::{CaptureFile, Trace};
use flowlab_sim::{SimResult, SimSpec};
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
        /// Also write the network configuration a master would use, as ENI XML.
        #[arg(long)]
        eni: Option<PathBuf>,
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
        /// Mailbox addresses of a station whose SyncManager configuration is not in the
        /// capture: `STATION=OUT,IN`, e.g. `0x1001=0x1000,0x1080`. Repeatable.
        #[arg(long = "mailbox", value_name = "STATION=OUT,IN")]
        mailbox_hints: Vec<MailboxHint>,
        /// Write the Ethernet frames reassembled from EoE to this pcapng.
        #[arg(long, value_name = "PCAPNG")]
        eoe_out: Option<PathBuf>,
        /// Network configuration (ENI XML): expected working counters, mailboxes, names.
        #[arg(long, value_name = "XML")]
        eni: Option<PathBuf>,
    },
    /// Summarize the devices of an ESI file.
    Esi {
        /// ESI XML file.
        file: PathBuf,
        /// Print JSON instead of the summary.
        #[arg(long)]
        json: bool,
    },
    /// Run a parameter study (sweep, Pareto frontier, calibration) over the simulator.
    Study {
        /// Study spec (see `flowlab schema study`). Omit to use the built-in example.
        spec: Option<PathBuf>,
        /// Capture to calibrate against (targets with `from_capture`).
        #[arg(long)]
        capture: Option<PathBuf>,
        /// Network configuration (ENI) for analysing that capture.
        #[arg(long, value_name = "XML")]
        eni: Option<PathBuf>,
        /// Mailbox addresses for that capture, `STATION=OUT,IN`. Repeatable.
        #[arg(long = "mailbox", value_name = "STATION=OUT,IN")]
        mailbox_hints: Vec<MailboxHint>,
        /// Write the result JSON.
        #[arg(long)]
        result: Option<PathBuf>,
    },
    /// Lay out the cyclic frames of a network and check its budget.
    Plan {
        /// Plan spec (see `flowlab schema plan`). Omit to use the built-in example.
        spec: Option<PathBuf>,
        /// Print JSON instead of the summary.
        #[arg(long)]
        json: bool,
    },
    /// Run the discrete-event simulator. With several specs, run each and print its table.
    Simulate {
        /// Spec files (see `flowlab schema sim`). Omit to use the built-in example.
        specs: Vec<PathBuf>,
        /// Write the simulated frames (one spec only).
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Write the result JSON (one spec), or a JSON array of results (several).
        #[arg(long)]
        result: Option<PathBuf>,
    },
    /// Rank one-change variants of a scenario by how much they shorten its transfers.
    ///
    /// Each variant doubles or halves one parameter (cycle, PDI rate and overhead, mailboxes,
    /// mailbox policy, application time) or switches one choice (protocol, mailbox access);
    /// pairs of non-protocol changes are tried too.
    WhatIf {
        /// Spec file (see `flowlab schema sim`). Omit to use the built-in example.
        spec: Option<PathBuf>,
        /// Write the result JSON.
        #[arg(long)]
        result: Option<PathBuf>,
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
        /// Directory of the built UI. A binary built with the `embed-ui` feature (the release
        /// binaries) serves its own UI when this directory has none.
        #[arg(long, default_value = "ui/dist")]
        ui_dir: PathBuf,
        /// Open the UI in the default browser.
        #[arg(long)]
        open: bool,
        /// Mailbox addresses for stations without SyncManager configuration in the capture,
        /// `STATION=OUT,IN`. Repeatable.
        #[arg(long = "mailbox", value_name = "STATION=OUT,IN")]
        mailbox_hints: Vec<MailboxHint>,
        /// Network configuration (ENI XML) applied to every capture.
        #[arg(long, value_name = "XML")]
        eni: Option<PathBuf>,
        /// Allow live capture from network interfaces in the UI (through dumpcap or tcpdump;
        /// only listens). Replays of loaded captures work without it.
        #[arg(long)]
        live: bool,
        /// Where live captures are saved.
        #[arg(long, default_value = "results/live", value_name = "DIR")]
        live_dir: PathBuf,
    },
    /// Capture EtherCAT traffic live and analyse it as it arrives.
    ///
    /// Runs Wireshark's dumpcap (or tcpdump) on the interface; only listens, never sends.
    /// Prints one line per second: frames, cycle period, jitter and round trip of the main
    /// cyclic stream, WKC mismatches, unanswered frames, missed cycles, mailbox messages.
    /// Saves the capture for `flowlab analyze`. Ctrl-C stops.
    Capture {
        /// Interface to capture on (`--list` shows them).
        interface: Option<String>,
        /// List the interfaces the capture tool can open, then exit.
        #[arg(long)]
        list: bool,
        /// Capture tool (default: dumpcap if installed, else tcpdump).
        #[arg(long, value_parser = ["dumpcap", "tcpdump"])]
        tool: Option<String>,
        /// Save the capture here (default: results/live/live-UNIXTIME.pcapng).
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Stop after this many seconds.
        #[arg(long, value_name = "SECONDS")]
        duration: Option<f64>,
        /// Capture time analysed in full at the end (`--report`), seconds.
        #[arg(long, default_value_t = 10.0, value_name = "SECONDS")]
        window: f64,
        /// Keep all traffic on the interface, not only EtherCAT.
        #[arg(long)]
        all_traffic: bool,
        /// Play a capture file back at its recorded pace instead of capturing.
        #[arg(long, value_name = "FILE", conflicts_with = "interface")]
        replay: Option<PathBuf>,
        /// Playback speed for --replay (1 = real time).
        #[arg(long, default_value_t = 1.0)]
        speed: f64,
        /// Write the analysis of the last window as JSON at the end.
        #[arg(long, value_name = "JSON")]
        report: Option<PathBuf>,
        /// Mailbox addresses for stations whose SyncManager configuration was not captured,
        /// `STATION=OUT,IN`. Repeatable.
        #[arg(long = "mailbox", value_name = "STATION=OUT,IN")]
        mailbox_hints: Vec<MailboxHint>,
        /// Network configuration (ENI XML): expected working counters, mailboxes, names.
        #[arg(long, value_name = "XML")]
        eni: Option<PathBuf>,
        /// Link rate for utilization, Mbit/s.
        #[arg(long, default_value_t = 100)]
        link_mbps: u64,
    },
    /// FlowLab's own EtherCAT master on an isolated lab segment: scan, a cyclic run, mailbox
    /// transfers measured like the simulator's. Sends frames; needs a build with
    /// `--features master`.
    Master(master::MasterArgs),
    /// Print a JSON Schema.
    Schema {
        /// Which schema.
        #[arg(value_parser = ["synth", "sim", "plan", "study", "lab"])]
        which: String,
    },
    /// Fit the simulator's PDI model (cost per access + effective rate) to timed ESC accesses.
    PdiFit {
        /// CSV with the header `direction,bytes,duration_ns`, one access per line.
        samples: PathBuf,
        /// Write the fit (with residuals per access size) as JSON.
        #[arg(long)]
        result: Option<PathBuf>,
        /// Store the fitted values in this lab-inputs file (created or updated).
        #[arg(long, value_name = "JSON")]
        lab: Option<PathBuf>,
    },
    /// Time complete ESC reads and writes over a LAN9252's SPI (Linux spidev), as samples for
    /// `pdi-fit`.
    ///
    /// Run it on the slave's host with the slave application stopped and the ESC in INIT (no
    /// master). Checks the link (BYTE_TEST, ID_REV) first; reads process RAM; writes only with
    /// --allow-writes into an --area of process RAM that nothing uses. --sim times a simulated
    /// LAN9252 instead and marks the samples simulated.
    PdiBench(pdi_bench::PdiBenchArgs),
    /// Read the master's mailbox service pattern from a capture, as a simulator policy.
    Policy {
        /// Capture of the real master serving the mailbox (e.g. during an FoE transfer).
        capture: PathBuf,
        /// Station whose pattern to use (default: the only one with a pattern).
        #[arg(long, value_parser = parse_station)]
        station: Option<u16>,
        /// Mailbox addresses of a station whose SyncManager configuration is not in the
        /// capture, `STATION=OUT,IN`. Repeatable.
        #[arg(long = "mailbox", value_name = "STATION=OUT,IN")]
        mailbox_hints: Vec<MailboxHint>,
        /// Network configuration (ENI XML) for the analysis.
        #[arg(long, value_name = "XML")]
        eni: Option<PathBuf>,
        /// Store the policy in this lab-inputs file (created or updated).
        #[arg(long, value_name = "JSON")]
        lab: Option<PathBuf>,
    },
}

fn parse_station(s: &str) -> std::result::Result<u16, String> {
    let r = match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(h) => u16::from_str_radix(h, 16),
        None => s.parse(),
    };
    r.map_err(|_| format!("`{s}` is not a station address (e.g. 0x1003)"))
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
        Cmd::Synth { spec, out, truth, eni } => {
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
            if let Some(e) = eni {
                ensure_parent(&e)?;
                std::fs::write(&e, flowlab_devices::write_eni(&flowlab_synth::network_config(&spec)))?;
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
        Cmd::Analyze { capture, out, json, link_mbps, min_stream_frames, mailbox_hints, eoe_out, eni } => {
            let file = CaptureFile::read(&capture).map_err(|e| format!("{}: {e}", capture.display()))?;
            let name = capture.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let trace = Trace::build(file, &name);
            let opts = AnalysisOptions {
                link_bps: link_mbps * 1_000_000,
                min_stream_frames,
                mailbox_hints,
                network: eni.as_deref().map(read_eni).transpose()?,
                ..Default::default()
            };
            let a = analyze(&trace, &opts);
            if let Some(p) = &eoe_out {
                ensure_parent(p)?;
                let n = write_eoe_pcapng(&a.mailbox, trace.provenance, &name, BufWriter::new(File::create(p)?))?;
                eprintln!("wrote {n} EoE-reassembled frames to {}", p.display());
            }
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
        Cmd::Serve { captures, port, host, ui_dir, open, mailbox_hints, eni, live, live_dir } => {
            if !host.is_loopback() {
                tracing::warn!("binding to {host}: the API can read local files and has no authentication");
            }
            let network = eni.as_deref().map(read_eni).transpose()?;
            let state = flowlab_server::AppState::new(AnalysisOptions { mailbox_hints, network, ..Default::default() })
                .with_live(flowlab_server::LiveConfig { interfaces: live, dir: live_dir });
            for c in &captures {
                let info = state.load_path(c).map_err(|e| format!("{}: {e}", c.display()))?;
                tracing::info!("loaded {} ({} frames)", info.name, info.frames);
            }
            let ui = flowlab_server::Ui::find(&ui_dir);
            match &ui {
                Some(flowlab_server::Ui::Dir(d)) => tracing::info!("serving the UI from {}", d.display()),
                Some(flowlab_server::Ui::Embedded) => tracing::info!("serving the UI built into this binary"),
                None => tracing::warn!(
                    "no built UI at {} (run `bun run build` in ui/, or build with `--features embed-ui`); \
                     serving the API only",
                    ui_dir.display()
                ),
            }
            let addr = SocketAddr::new(host, port);
            if open {
                let _ = open::that(format!("http://{addr}"));
            }
            tokio::runtime::Runtime::new()?.block_on(flowlab_server::serve(state, addr, ui))?;
        }
        Cmd::Capture {
            interface,
            list,
            tool,
            out,
            duration,
            window,
            all_traffic,
            replay,
            speed,
            report,
            mailbox_hints,
            eni,
            link_mbps,
        } => capture::run(capture::CaptureArgs {
            interface,
            list,
            tool,
            out,
            duration,
            window,
            all_traffic,
            replay,
            speed,
            report,
            analysis: AnalysisOptions {
                link_bps: link_mbps * 1_000_000,
                mailbox_hints,
                network: eni.as_deref().map(read_eni).transpose()?,
                ..Default::default()
            },
        })?,
        Cmd::Master(m) => master::run(m)?,
        Cmd::Schema { which } => {
            let schema = match which.as_str() {
                "sim" => flowlab_sim::spec_schema(),
                "plan" => flowlab_devices::plan_schema(),
                "study" => flowlab_study::spec_schema(),
                "lab" => flowlab_study::lab::schema(),
                _ => flowlab_synth::spec_schema(),
            };
            println!("{}", serde_json::to_string_pretty(&schema)?);
        }
        Cmd::Esi { file, json } => {
            let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            let esi = flowlab_devices::parse_esi(&text).map_err(|e| format!("{}: {e}", file.display()))?;
            if json {
                println!("{}", serde_json::to_string_pretty(&esi)?);
            } else {
                println!("{} (vendor 0x{:08X}): {} devices", esi.vendor_name, esi.vendor_id, esi.devices.len());
                for d in &esi.devices {
                    let mbx = d.mailbox.as_ref().map_or("no mailbox".to_owned(), |m| m.names().join("/"));
                    println!(
                        "  {:<14} 0x{:08X} rev 0x{:08X}  out {:>4} B  in {:>4} B  mailbox {:>4}/{:<4} B  {}{}  {}",
                        d.type_name,
                        d.product_code,
                        d.revision,
                        d.default_outputs_bits.div_ceil(8),
                        d.default_inputs_bits.div_ceil(8),
                        d.sm_size("MBoxOut").unwrap_or(0),
                        d.sm_size("MBoxIn").unwrap_or(0),
                        mbx,
                        if d.dc { ", DC" } else { "" },
                        d.name
                    );
                }
            }
        }
        Cmd::Study { spec, capture, eni, mailbox_hints, result } => {
            let spec: flowlab_study::StudySpec = match spec {
                Some(p) => serde_json::from_reader(File::open(&p).map_err(|e| format!("{}: {e}", p.display()))?)
                    .map_err(|e| format!("{}: {e}", p.display()))?,
                None => flowlab_study::StudySpec::example(),
            };
            let report = match &capture {
                Some(c) => {
                    let file = CaptureFile::read(c).map_err(|e| format!("{}: {e}", c.display()))?;
                    let name = c.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let opts = AnalysisOptions {
                        network: eni.as_deref().map(read_eni).transpose()?,
                        mailbox_hints,
                        ..Default::default()
                    };
                    Some(analyze(&Trace::build(file, &name), &opts).report)
                }
                None => None,
            };
            let r = flowlab_study::run_study(&spec, report.as_ref())?;
            print_study(&r);
            if let Some(p) = result {
                ensure_parent(&p)?;
                serde_json::to_writer_pretty(BufWriter::new(File::create(&p)?), &r)?;
            }
        }
        Cmd::PdiFit { samples, result, lab } => {
            let bytes = std::fs::read(&samples).map_err(|e| format!("{}: {e}", samples.display()))?;
            pdi_bench::fit_samples(&bytes, &samples.display().to_string(), &file_name(&samples), result, lab)?;
        }
        Cmd::PdiBench(args) => pdi_bench::run(args)?,
        Cmd::Policy { capture, station, mailbox_hints, eni, lab } => {
            let file = CaptureFile::read(&capture).map_err(|e| format!("{}: {e}", capture.display()))?;
            let name = file_name(&capture);
            let trace = Trace::build(file, &name);
            let opts = AnalysisOptions {
                mailbox_hints,
                network: eni.as_deref().map(read_eni).transpose()?,
                ..Default::default()
            };
            let report = analyze(&trace, &opts).report;
            println!("{name}  [{}]", report.meta.source.as_str().to_uppercase());
            let served: Vec<_> = report.mailbox.channels.iter().filter(|c| c.service.is_some()).collect();
            if served.is_empty() {
                return Err("no mailbox service pattern: the capture needs known directions, a cyclic stream and \
                    mailbox accesses in at least 10 cycles (pass --mailbox if the SyncManager setup is not in it)"
                    .into());
            }
            for c in &served {
                print_service(c.station, c.service.as_ref().expect("filtered"));
            }
            let chosen = match station {
                Some(st) => {
                    served.iter().find(|c| c.station == st).ok_or_else(|| format!("no pattern for 0x{st:04X}"))?
                }
                None if served.len() == 1 => served[0],
                None => return Err("several stations have a pattern: choose one with --station".into()),
            };
            let policy = flowlab_study::mailbox_policy(&chosen.service.as_ref().expect("filtered").pattern);
            println!("simulator policy: {}", serde_json::to_string(&policy)?);
            if let Some(p) = lab {
                let mut l = read_lab(&p)?;
                l.mailbox_policy = Some(policy);
                l.sources.push(format!(
                    "mailbox_policy: flowlab policy {name}, station 0x{:04X} (sha256 {})",
                    chosen.station,
                    &report.meta.input.sha256[..16]
                ));
                write_lab(&p, &l)?;
            }
        }
        Cmd::Plan { spec, json } => {
            let spec: flowlab_devices::PlanSpec = match spec {
                Some(p) => serde_json::from_reader(File::open(&p).map_err(|e| format!("{}: {e}", p.display()))?)?,
                None => flowlab_devices::PlanSpec::example(),
            };
            let r = flowlab_devices::plan(&spec);
            if json {
                println!("{}", serde_json::to_string_pretty(&r)?);
            } else {
                print_plan(&r);
            }
        }
        Cmd::WhatIf { spec, result } => {
            let spec: SimSpec = match &spec {
                Some(p) => serde_json::from_reader(File::open(p).map_err(|e| format!("{}: {e}", p.display()))?)
                    .map_err(|e| format!("{}: {e}", p.display()))?,
                None => SimSpec::example(),
            };
            let r = flowlab_study::what_if(&spec)?;
            print_what_if(&spec, &r);
            if let Some(p) = result {
                ensure_parent(&p)?;
                serde_json::to_writer_pretty(BufWriter::new(File::create(&p)?), &r)?;
            }
        }
        Cmd::Simulate { specs, out, result } => {
            if specs.len() > 1 && out.is_some() {
                return Err("--out takes one spec; run the specs separately to keep their captures".into());
            }
            let specs: Vec<(String, SimSpec)> = if specs.is_empty() {
                vec![("example".into(), SimSpec::example())]
            } else {
                specs
                    .iter()
                    .map(|p| -> Result<(String, SimSpec)> {
                        let f = File::open(p).map_err(|e| format!("{}: {e}", p.display()))?;
                        let s = serde_json::from_reader(f).map_err(|e| format!("{}: {e}", p.display()))?;
                        Ok((p.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), s))
                    })
                    .collect::<Result<_>>()?
            };
            let mut results = Vec::new();
            for (name, spec) in &specs {
                let r = match &out {
                    Some(o) => {
                        ensure_parent(o)?;
                        flowlab_sim::simulate(spec, Some(BufWriter::new(File::create(o)?)))?
                    }
                    None => flowlab_sim::simulate(spec, None::<std::io::Sink>)?,
                };
                print_sim(name, &r);
                results.push(r);
            }
            if let Some(p) = result {
                ensure_parent(&p)?;
                let w = BufWriter::new(File::create(&p)?);
                match results.as_slice() {
                    [one] => serde_json::to_writer_pretty(w, one)?,
                    many => serde_json::to_writer_pretty(w, many)?,
                }
            }
            if let Some(o) = &out {
                eprintln!("wrote {} frames to {} (simulated)", results[0].capture_frames, o.display());
            }
        }
    }
    Ok(())
}

fn print_what_if(spec: &SimSpec, r: &flowlab_study::WhatIfResult) {
    let secs = |v: Option<f64>| v.map_or("–".to_owned(), |ns| format!("{:.3} s", ns / 1e9));
    let singles = r.changes.iter().filter(|c| c.parts.is_empty()).count();
    println!("{}  [SIMULATED]  {singles} single changes, {} pairs", spec.description, r.changes.len() - singles);
    match r.base.finish_ns {
        Some(f) => println!("  as given: all transfers done after {}", secs(Some(f))),
        None => println!("  as given: {}", r.base.reason.as_deref().unwrap_or("not feasible")),
    }
    println!("  {:>10}  {:>8}  change", "done after", "vs. now");
    for c in r.changes.iter().filter(|c| c.parts.is_empty()) {
        let delta = match (c.run.finish_ns, r.base.finish_ns) {
            (Some(x), Some(b)) if b > 0.0 => format!("{:+.1} %", 100.0 * (x - b) / b),
            _ => "–".into(),
        };
        let tail = c.run.reason.as_deref().map(|r| format!("  ({r})")).unwrap_or_default();
        println!("  {:>10}  {:>8}  {}{tail}", secs(c.run.finish_ns), delta, c.label);
    }
    // Pairs worth showing: better than either change alone by more than 1 % of the base.
    let alone = |label: &str| {
        r.changes.iter().find(|c| c.label == label).and_then(|c| c.run.finish_ns).unwrap_or(f64::INFINITY)
    };
    let together: Vec<_> = r
        .changes
        .iter()
        .filter(|c| c.parts.len() == 2)
        .filter(|c| match (c.run.finish_ns, r.base.finish_ns) {
            (Some(x), Some(b)) => x < alone(&c.parts[0]).min(alone(&c.parts[1])) - 0.01 * b,
            _ => false,
        })
        .take(5)
        .collect();
    if !together.is_empty() {
        println!("  two changes together, better than either alone:");
        for c in together {
            let delta = match (c.run.finish_ns, r.base.finish_ns) {
                (Some(x), Some(b)) if b > 0.0 => format!("{:+.1} %", 100.0 * (x - b) / b),
                _ => "–".into(),
            };
            println!("  {:>10}  {:>8}  {}", secs(c.run.finish_ns), delta, c.label);
        }
    }
}

fn print_study(r: &flowlab_study::StudyResult) {
    println!(
        "{}  [SIMULATED]  {} runs, {} feasible, {} Pareto-optimal",
        r.description,
        r.runs.len(),
        r.feasible,
        r.pareto.len()
    );
    let head: Vec<String> = r.parameters.iter().chain(&r.objectives).cloned().collect();
    println!("  {:>4}  {}", "run", head.join("  |  "));
    let mut shown: Vec<u32> = r.pareto.clone();
    shown.sort_by(|a, b| {
        let k = |i: &u32| r.runs[*i as usize].objectives.first().copied().flatten().unwrap_or(f64::INFINITY);
        k(a).total_cmp(&k(b))
    });
    for i in shown.iter().take(15) {
        let run = &r.runs[*i as usize];
        let objs: Vec<String> = run.objectives.iter().map(|v| v.map_or("-".into(), |v| format!("{v:.4}"))).collect();
        let mark = if r.best == Some(*i) { "*" } else { " " };
        println!("  {:>3}{mark}  {}  |  {}", i, run.values.join("  |  "), objs.join("  |  "));
    }
    if r.pareto.len() > 15 {
        println!("  … {} more Pareto-optimal runs in the result JSON", r.pareto.len() - 15);
    }
    for x in r.runs.iter().filter(|x| !x.feasible).take(3) {
        println!("  infeasible run {}: {}", x.index, x.reason.as_deref().unwrap_or("?"));
    }
    for res in &r.residuals {
        println!(
            "  fit {}: measured {:.6} ({}), simulated {}, error {}",
            res.label,
            res.measured,
            res.source,
            res.simulated.map_or("-".into(), |v| format!("{v:.6}")),
            res.relative_error.map_or("-".into(), |e| format!("{:+.2} %", 100.0 * e))
        );
    }
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn read_lab(p: &Path) -> Result<flowlab_study::LabInputs> {
    if !p.exists() {
        return Ok(flowlab_study::LabInputs::default());
    }
    let text = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(serde_json::from_str(&text).map_err(|e| format!("{}: {e}", p.display()))?)
}

fn write_lab(p: &Path, l: &flowlab_study::LabInputs) -> Result<()> {
    ensure_parent(p)?;
    std::fs::write(p, serde_json::to_string_pretty(l)? + "\n")?;
    eprintln!("updated {}", p.display());
    Ok(())
}

fn print_pdi_fit(f: &flowlab_study::PdiFit) {
    println!("{}  [{}]", f.meta.input.name, f.meta.source.as_str().to_uppercase());
    for (dir, line) in [("read", &f.read), ("write", &f.write)] {
        let Some(l) = line else { continue };
        println!(
            "  {dir}: {} accesses, cost per access {}, effective rate {}, worst size off the line by {:.1} %",
            l.samples,
            ns(l.overhead_ns),
            rate(l.rate_bps),
            100.0 * l.max_relative_error
        );
        for s in &l.sizes {
            println!(
                "    {:>6} B  ×{:<5} median {:>10}  p95 {:>10}  line {:>10}  {:+.1} %",
                s.bytes,
                s.samples,
                ns(s.median_ns),
                ns(s.p95_ns),
                ns(s.fitted_ns),
                100.0 * s.relative_error
            );
        }
    }
    println!("simulator pdi: {}", serde_json::to_string(&f.pdi).unwrap_or_default());
}

fn print_service(station: u16, s: &flowlab_analysis::MailboxService) {
    println!(
        "  station 0x{station:04X}: {} cycles with mailbox accesses; per cycle p50 {} accesses in {} frames, \
         last send at {:.0} % of the cycle (p95); {} accesses in cyclic frames{}",
        s.cycles,
        s.accesses_per_cycle.p50,
        s.frames_per_cycle.p50,
        s.last_send_percent.p95,
        s.in_cyclic_frames,
        s.turnaround.as_ref().map_or(String::new(), |t| format!("; turnaround p50 {}", ns(t.p50)))
    );
}

fn read_eni(p: &Path) -> Result<flowlab_devices::NetworkConfig> {
    let text = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(flowlab_devices::parse_eni(&text).map_err(|e| format!("{}: {e}", p.display()))?)
}

fn print_plan(r: &flowlab_devices::PlanResult) {
    println!(
        "cycle {}  link {:.0} Mbit/s  process image {} B out / {} B in",
        ns(r.cycle_ns),
        r.link_bps / 1e6,
        r.image_out_bytes,
        r.image_in_bytes
    );
    for (i, f) in r.frames.iter().enumerate() {
        let dgs: Vec<String> = f.datagrams.iter().map(|d| format!("{} {} B", d.command, d.data_bytes)).collect();
        println!("  frame {i}: {} B, {} on the wire  [{}]", f.frame_bytes, ns(f.wire_ns), dgs.join(", "));
    }
    println!(
        "  cyclic frames back after {} ({:.1} % of the cycle on the wire, ring {}); idle {} ({:.1} %)",
        ns(r.cyclic_done_ns),
        r.cyclic_percent,
        ns(r.ring_delay_ns),
        ns(r.idle_ns),
        r.idle_percent
    );
    if let (Some(f), Some(n), Some(b)) = (r.mailbox_frame_ns, r.mailbox_frames_per_cycle, r.mailbox_bytes_per_s) {
        println!("  mailbox: {} per frame, {n} frames fit per cycle, ≤ {:.0} kB/s per direction", ns(f), b / 1e3);
    }
    for c in &r.checks {
        println!("  [{}] {}: {}", c.severity.to_uppercase(), c.subject, c.message);
    }
}

fn ensure_parent(p: &Path) -> Result<()> {
    if let Some(d) = p.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(d)?;
    }
    Ok(())
}

fn ns(v: f64) -> String {
    // Thresholds just below the unit, so 999.996 µs prints as 1.000 ms, not 1000.00 µs.
    let a = v.abs();
    if a >= 999_999_500.0 {
        format!("{:.3} s", v / 1e9)
    } else if a >= 999_995.0 {
        format!("{:.3} ms", v / 1e6)
    } else if a >= 999.5 {
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
    let m = &r.mailbox;
    if !m.channels.is_empty() {
        let protos: Vec<String> =
            m.protocols.iter().map(|p| format!("{} {}↓/{}↑", p.protocol, p.messages_out, p.messages_in)).collect();
        println!("  mailbox: {} stations; messages {}", m.channels.len(), protos.join(", "));
        let c = &m.coe;
        if c.transactions_total > 0 {
            println!(
                "    SDO: {} transfers, {} ok, {} aborted, {} unfinished, {} protocol errors; duration p50 {}",
                c.transactions_total,
                c.ok,
                c.aborted,
                c.unfinished,
                c.protocol_errors,
                c.duration.as_ref().map_or("-".into(), |d| ns(d.p50))
            );
        }
        for t in &m.foe {
            println!(
                "    FoE {} '{}' station 0x{:04X}: {} B in {} packets, {}{}",
                t.kind,
                t.file_name,
                t.station,
                t.bytes,
                t.packets,
                t.outcome,
                t.goodput_bps.map_or(String::new(), |b| format!(", {:.1} kbit/s", b / 1e3))
            );
        }
        for c in &m.channels {
            if let Some(sv) = &c.service {
                print_service(c.station, sv);
            }
        }
        if m.eoe.frames_total > 0 || m.eoe.reassembly_errors > 0 {
            println!(
                "    EoE: {} frames reassembled ({} B to slaves, {} B from slaves), {} errors",
                m.eoe.frames_total, m.eoe.bytes_to_slaves, m.eoe.bytes_from_slaves, m.eoe.reassembly_errors
            );
        }
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

fn rate(bps: f64) -> String {
    if bps >= 1e6 { format!("{:.2} Mbit/s", bps / 1e6) } else { format!("{:.1} kbit/s", bps / 1e3) }
}

fn print_sim(name: &str, r: &SimResult) {
    println!("{name}  [SIMULATED]  seed {}  {}", r.seed, r.description);
    println!(
        "  {} cycles of {}, {} frames, link {:.2} %, {}",
        r.cycles,
        ns(r.cycle_time_ns),
        r.frames,
        r.link_utilization_percent,
        r.policy
    );
    println!(
        "  {:<3} {:<12} {:<16} {:>12} {:>12} {:>14} {:>12} {:>12}  integrity",
        "#", "protocol", "direction", "payload", "time", "goodput", "link bound", "TCP ref."
    );
    for t in &r.transfers {
        println!(
            "  {:<3} {:<12} {:<16} {:>10} B {:>12} {:>14} {:>12} {:>12}  {}",
            t.index,
            t.protocol,
            t.direction.replace('_', " "),
            t.payload_bytes,
            t.duration_ns.map_or("unfinished".into(), ns),
            t.goodput_bps.map_or("-".into(), rate),
            ns(t.physical_bound_ns),
            ns(t.ethernet_tcp_reference_ns),
            if t.verified { "SHA-256 ok" } else { "NOT VERIFIED" }
        );
    }
    for s in r.slaves.iter().filter(|s| s.pdi_busy_percent > 0.5) {
        println!(
            "  slave 0x{:04X} {}: PDI busy {:.1} %, ESC RAM {}/{} B",
            s.station, s.name, s.pdi_busy_percent, s.dpram_used_bytes, s.dpram_bytes
        );
    }
}
