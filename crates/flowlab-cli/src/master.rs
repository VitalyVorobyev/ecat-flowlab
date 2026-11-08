//! `flowlab master`: FlowLab's own EtherCAT master (EtherCrab) for an isolated lab segment.
//!
//! Compiled only with the cargo feature `master`: it sends frames, so it is opt-in, and on
//! Windows it links against the Npcap SDK. Without the feature the command only says how to
//! get it.

#[cfg(not(feature = "master"))]
mod imp {
    use crate::Result;

    /// Arguments of `flowlab master` in a build without the lab master.
    #[derive(clap::Args)]
    pub(crate) struct MasterArgs {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
        rest: Vec<String>,
    }

    pub(crate) fn run(_: MasterArgs) -> Result<()> {
        Err("this flowlab was built without the lab master. It sends EtherCAT frames, so it is opt-in: \
             build with `cargo build --release -p flowlab-cli --features master` (Windows: needs the Npcap SDK \
             to link). See the book's \"Lab master\" page."
            .into())
    }
}

#[cfg(feature = "master")]
mod imp {
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use flowlab_live::Tool;
    use flowlab_master::hardware::{Session, SessionOptions};
    use flowlab_master::safety::{SegmentCheck, check_segment};
    use flowlab_master::{
        EoeAddresses, MeasuredTransfer, Payload, PollMode, TransferJob, TransferOptions, TransferProtocol, policy_text,
        setup_like,
    };
    use flowlab_sim::MailboxPolicy;

    use crate::{Result, ensure_parent, ns, parse_station};

    /// Arguments of `flowlab master`.
    #[derive(clap::Args)]
    pub(crate) struct MasterArgs {
        #[command(subcommand)]
        cmd: MasterCmd,
    }

    /// What every command needs before it sends anything.
    #[derive(clap::Args)]
    struct Safety {
        /// Network interface the slaves are on (Linux: e.g. enp3s0; Windows: \Device\NPF_{…},
        /// as `flowlab capture --list` shows).
        #[arg(long, short = 'i')]
        interface: String,
        /// Confirm that the interface is on an isolated lab segment: no other master, no
        /// production equipment. Nothing is sent without it.
        #[arg(long)]
        isolated_lab_network: bool,
        /// Start even though no capture tool (dumpcap, tcpdump) can check for another master
        /// first.
        #[arg(long)]
        no_master_check: bool,
        /// Listen this long for another master's frames before sending, seconds (includes the
        /// capture tool's start-up).
        #[arg(long, default_value_t = 2.0, value_name = "SECONDS")]
        listen: f64,
        /// Capture tool for the check (default: dumpcap if installed, else tcpdump).
        #[arg(long, value_parser = ["dumpcap", "tcpdump"])]
        tool: Option<String>,
        /// How long to wait for a frame to come back before it counts as lost, ms.
        #[arg(long, default_value_t = 30, value_name = "MS")]
        pdu_timeout_ms: u64,
    }

    #[derive(clap::Subcommand)]
    enum MasterCmd {
        /// Find the slaves: identity, names, station addresses, AL states, mailbox
        /// SyncManagers, DC support. Leaves them in PRE-OP.
        Scan {
            #[command(flatten)]
            safety: Safety,
            /// Write the result as JSON (default: results/master/scan-UNIXTIME.json).
            #[arg(short, long)]
            out: Option<PathBuf>,
            /// Also write the network as ENI XML for `flowlab analyze --eni`.
            #[arg(long, value_name = "XML")]
            eni: Option<PathBuf>,
        },
        /// PRE-OP → SAFE-OP → OP, process data every cycle for a while; per-cycle round trip
        /// and working counter. Back to PRE-OP at the end.
        Run {
            #[command(flatten)]
            safety: Safety,
            /// Cycle time, µs.
            #[arg(long, default_value_t = 1000, value_name = "US")]
            cycle_us: u64,
            /// How long to run, seconds.
            #[arg(long, default_value_t = 10.0, value_name = "SECONDS")]
            duration: f64,
            /// Keep at most this many per-cycle records in the JSON.
            #[arg(long, default_value_t = 100_000)]
            records: usize,
            /// Write the result as JSON (default: results/master/run-UNIXTIME.json).
            #[arg(short, long)]
            out: Option<PathBuf>,
        },
        /// Move a payload to or from a slave with FoE, CoE or EoE, under a mailbox policy and
        /// cycle time, and measure it like the simulator does.
        Transfer(Box<TransferArgs>),
    }

    #[derive(clap::Args)]
    struct TransferArgs {
        #[command(flatten)]
        safety: Safety,
        /// Station address of the slave (`flowlab master scan` lists them; EtherCrab assigns
        /// 0x1000, 0x1001, …).
        #[arg(long, value_parser = parse_station)]
        station: u16,
        /// Set up like transfer `--transfer` of this simulator spec: protocol, direction, the
        /// simulator's payload bytes (same SHA-256), cycle time and policy.
        #[arg(long, value_name = "SIM_JSON", conflicts_with_all = ["protocol", "file", "bytes"])]
        like: Option<PathBuf>,
        /// Which transfer of the `--like` spec, from 0.
        #[arg(long, default_value_t = 0, requires = "like")]
        transfer: usize,
        /// Protocol.
        #[arg(long, value_parser = ["foe", "coe", "eoe"], required_unless_present = "like")]
        protocol: Option<String>,
        /// Direction.
        #[arg(long, value_parser = ["to-slave", "from-slave"], default_value = "to-slave")]
        direction: String,
        /// To the slave: send this file.
        #[arg(long, conflicts_with = "bytes")]
        file: Option<PathBuf>,
        /// To the slave: send this many bytes of the simulator's payload for `--seed`. From
        /// the slave: wait for this many bytes (required for EoE).
        #[arg(long)]
        bytes: Option<u64>,
        /// Seed of the generated payload (`--bytes`); the expected SHA-256 for a receive.
        #[arg(long)]
        seed: Option<u64>,
        /// From the slave: the SHA-256 (hex) the received bytes should have.
        #[arg(long, value_name = "HEX")]
        expect_sha256: Option<String>,
        /// From the slave: save the received bytes here.
        #[arg(long, value_name = "FILE")]
        save_received: Option<PathBuf>,
        /// Cycle time, µs (default 1000, or the `--like` spec's).
        #[arg(long, value_name = "US")]
        cycle_us: Option<u64>,
        /// Mailbox policy: `fixed-quota:N` (N datagrams per cycle), `round-robin:N` (N slaves
        /// per cycle) or `opportunistic:PERCENT[,TURNAROUND_NS]` (default fixed-quota:2, or the
        /// `--like` spec's).
        #[arg(long, value_parser = parse_policy)]
        policy: Option<MailboxPolicy>,
        /// How "mailbox in" is polled: `read` the buffer (WKC 0 when empty) or read the
        /// SyncManager `status` first.
        #[arg(long, value_parser = ["read", "status"], default_value = "read")]
        poll: String,
        /// Put the network in OP and exchange process data at the start of every cycle (as the
        /// simulator does); otherwise stay in PRE-OP.
        #[arg(long)]
        op: bool,
        /// FoE file name.
        #[arg(long, default_value = "flowlab.bin")]
        foe_file: String,
        /// FoE password.
        #[arg(long, default_value_t = 0)]
        foe_password: u32,
        /// CoE object, INDEX:SUBINDEX.
        #[arg(long, default_value = "0x2000:0", value_parser = parse_object)]
        sdo: (u16, u8),
        /// EoE: IP MTU of the tunnelled frames, bytes.
        #[arg(long, default_value_t = 1500)]
        eoe_mtu: u16,
        /// EoE: the slave's MAC (its EoE interface).
        #[arg(long, value_parser = parse_mac)]
        eoe_slave_mac: Option<[u8; 6]>,
        /// EoE: the slave's IPv4 address.
        #[arg(long, value_parser = parse_ip)]
        eoe_slave_ip: Option<[u8; 4]>,
        /// EoE: the master side's MAC.
        #[arg(long, value_parser = parse_mac)]
        eoe_master_mac: Option<[u8; 6]>,
        /// EoE: the master side's IPv4 address.
        #[arg(long, value_parser = parse_ip)]
        eoe_master_ip: Option<[u8; 4]>,
        /// EoE: UDP destination port.
        #[arg(long)]
        eoe_port: Option<u16>,
        /// Without a reply for this long, send the last message again with a new counter, ms.
        #[arg(long, default_value_t = 1000, value_name = "MS")]
        reply_timeout_ms: u64,
        /// Resends without progress before giving up.
        #[arg(long, default_value_t = 3)]
        max_resends: u32,
        /// Give up after this long, seconds.
        #[arg(long, default_value_t = 600.0, value_name = "SECONDS")]
        max_duration: f64,
        /// Counter of the first mailbox message (1–7).
        #[arg(long, default_value_t = 1)]
        first_counter: u8,
        /// Keep at most this many per-cycle records in the JSON.
        #[arg(long, default_value_t = 100_000)]
        records: usize,
        /// Write the result as JSON (default: results/master/transfer-UNIXTIME.json).
        #[arg(short, long)]
        out: Option<PathBuf>,
    }

    fn parse_policy(s: &str) -> std::result::Result<MailboxPolicy, String> {
        let (kind, rest) = s.split_once(':').unwrap_or((s, ""));
        let bad = || format!("`{s}`: expected fixed-quota:N, round-robin:N or opportunistic:PERCENT[,TURNAROUND_NS]");
        Ok(match kind {
            "fixed-quota" => MailboxPolicy::FixedQuota { accesses_per_cycle: rest.parse().map_err(|_| bad())? },
            "round-robin" => MailboxPolicy::RoundRobin { slaves_per_cycle: rest.parse().map_err(|_| bad())? },
            "opportunistic" => {
                let (p, t) = rest.split_once(',').unwrap_or((rest, "10000"));
                MailboxPolicy::Opportunistic {
                    bus_budget_percent: p.parse().map_err(|_| bad())?,
                    turnaround_ns: t.parse().map_err(|_| bad())?,
                }
            }
            _ => return Err(bad()),
        })
    }

    fn parse_object(s: &str) -> std::result::Result<(u16, u8), String> {
        let bad = || format!("`{s}`: expected INDEX:SUBINDEX, e.g. 0x2000:0");
        let (i, sub) = s.split_once(':').ok_or_else(bad)?;
        let num = |v: &str| match v.strip_prefix("0x").or_else(|| v.strip_prefix("0X")) {
            Some(h) => u32::from_str_radix(h, 16).ok(),
            None => v.parse().ok(),
        };
        let index = num(i).and_then(|v| u16::try_from(v).ok()).ok_or_else(bad)?;
        let sub = num(sub).and_then(|v| u8::try_from(v).ok()).ok_or_else(bad)?;
        Ok((index, sub))
    }

    fn parse_mac(s: &str) -> std::result::Result<[u8; 6], String> {
        let parts: Vec<u8> = s.split([':', '-']).filter_map(|p| u8::from_str_radix(p, 16).ok()).collect();
        parts.try_into().map_err(|_| format!("`{s}` is not a MAC address (aa:bb:cc:dd:ee:ff)"))
    }

    fn parse_ip(s: &str) -> std::result::Result<[u8; 4], String> {
        s.parse::<std::net::Ipv4Addr>().map(|a| a.octets()).map_err(|_| format!("`{s}` is not an IPv4 address"))
    }

    /// The checks `flowlab_master::safety` runs before anything is sent, from the flags.
    fn segment_check(s: &Safety) -> Result<SegmentCheck> {
        if !(s.listen.is_finite() && s.listen >= 0.0) {
            return Err("--listen must be zero or more seconds".into());
        }
        let tool = match s.tool.as_deref() {
            Some("dumpcap") => Some(Tool::Dumpcap),
            Some("tcpdump") => Some(Tool::Tcpdump),
            _ => None,
        };
        Ok(SegmentCheck {
            interface: s.interface.clone(),
            isolated_lab_network: s.isolated_lab_network,
            listen: Duration::from_secs_f64(s.listen),
            tool,
            no_master_check: s.no_master_check,
        })
    }

    fn open(s: &Safety) -> Result<Session> {
        let segment = check_segment(&segment_check(s)?, |m| eprintln!("{m}"))?;
        for note in segment.notes() {
            eprintln!("{note}");
        }
        eprintln!(
            "tip: `flowlab capture {} -o results/master/run.pcapng` in another terminal records this master's frames",
            s.interface
        );
        let o = SessionOptions { pdu_timeout: Duration::from_millis(s.pdu_timeout_ms), ..SessionOptions::default() };
        Session::open(&segment, &o).map_err(|e| {
            format!(
                "{e}\n(Linux: run as root or `sudo setcap cap_net_raw,cap_net_admin=eip $(which flowlab)`; \
                 Windows: install Npcap and use the \\Device\\NPF_{{…}} name)"
            )
            .into()
        })
    }

    fn out_path(out: Option<PathBuf>, what: &str) -> PathBuf {
        out.unwrap_or_else(|| {
            let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
            Path::new("results/master").join(format!("{what}-{secs}.json"))
        })
    }

    fn write_json(path: &Path, v: serde_json::Value) -> Result<()> {
        ensure_parent(path)?;
        serde_json::to_writer_pretty(std::io::BufWriter::new(std::fs::File::create(path)?), &v)?;
        eprintln!("wrote {} (provenance: measured)", path.display());
        Ok(())
    }

    pub(crate) fn run(a: MasterArgs) -> Result<()> {
        match a.cmd {
            MasterCmd::Scan { safety, out, eni } => {
                let session = open(&safety)?;
                let r = session.scan()?;
                println!("{} slave(s) on {}  [MEASURED]", r.slaves.len(), r.interface);
                for s in &r.slaves {
                    let mbx = s
                        .sync_managers
                        .iter()
                        .filter(|m| m.mode == "mailbox")
                        .map(|m| format!("SM{} {:#06x}+{}", m.index, m.start, m.length))
                        .collect::<Vec<_>>()
                        .join(", ");
                    println!(
                        "  #{} {:#06x}  {:<24} vendor {:#010x} product {:#010x} rev {:#x}  {}  mailbox [{}] {}  DC {}",
                        s.position,
                        s.station,
                        s.name,
                        s.vendor_id,
                        s.product_code,
                        s.revision,
                        s.al_state,
                        mbx,
                        s.mailbox_protocols.join("/"),
                        s.dc_support
                    );
                }
                write_json(&out_path(out, "scan"), serde_json::to_value(&r)?)?;
                if let Some(p) = eni {
                    ensure_parent(&p)?;
                    std::fs::write(&p, flowlab_devices::write_eni(&r.network_config()))?;
                    eprintln!("wrote {} (`flowlab analyze CAPTURE --eni {}`)", p.display(), p.display());
                }
                Ok(())
            }
            MasterCmd::Run { safety, cycle_us, duration, records, out } => {
                if duration.is_nan() || duration <= 0.0 {
                    return Err("--duration must be positive".into());
                }
                let session = open(&safety)?;
                let r = session.run(Duration::from_micros(cycle_us), Duration::from_secs_f64(duration), records)?;
                let t = &r.timing;
                println!(
                    "{} slave(s), cycle {}, {} cycles  [MEASURED, host timing]",
                    r.slaves,
                    ns(r.cycle_time_ns),
                    t.cycles
                );
                println!(
                    "  round trip p50 {}  p99 {}  max {};  start late p99 {}  max {}",
                    ns(t.cyclic.p50_ns),
                    ns(t.cyclic.p99_ns),
                    ns(t.cyclic.max_ns),
                    ns(t.start_late.p99_ns),
                    ns(t.start_late.max_ns)
                );
                println!(
                    "  WKC expected {}: {} mismatches, {} lost, {} overruns, {} skipped cycles; final states {}",
                    r.expected_wkc,
                    r.wkc_mismatches,
                    r.lost_frames,
                    t.overruns,
                    t.skipped_cycles,
                    r.final_states.join(", ")
                );
                write_json(&out_path(out, "run"), serde_json::to_value(&r)?)
            }
            MasterCmd::Transfer(t) => transfer(*t),
        }
    }

    fn transfer(a: TransferArgs) -> Result<()> {
        let to_slave = a.direction == "to-slave";
        let (protocol, payload, payload_name, cycle_ns, policy) = match &a.like {
            Some(spec_path) => {
                let file = std::fs::File::open(spec_path).map_err(|e| format!("{}: {e}", spec_path.display()))?;
                let spec: flowlab_sim::SimSpec = serde_json::from_reader(file)?;
                let s = setup_like(&spec, a.transfer)?;
                eprintln!(
                    "set up like transfer {} of {}: {} B, SHA-256 {}",
                    a.transfer,
                    spec_path.display(),
                    match &s.payload {
                        Payload::Send(d) => d.len() as u64,
                        Payload::Receive { expected_bytes, .. } => expected_bytes.unwrap_or(0),
                    },
                    s.sha256
                );
                (s.protocol, s.payload, s.payload_name, s.cycle_time_ns, s.policy)
            }
            None => {
                let mut addresses = EoeAddresses::default();
                if let Some(m) = a.eoe_slave_mac {
                    addresses.slave_mac = m;
                }
                if let Some(m) = a.eoe_master_mac {
                    addresses.master_mac = m;
                }
                if let Some(ip) = a.eoe_slave_ip {
                    addresses.slave_ip = ip;
                }
                if let Some(ip) = a.eoe_master_ip {
                    addresses.master_ip = ip;
                }
                if let Some(p) = a.eoe_port {
                    addresses.destination_port = p;
                }
                let protocol = match a.protocol.as_deref() {
                    Some("foe") => TransferProtocol::Foe { file_name: a.foe_file.clone(), password: a.foe_password },
                    Some("coe") => TransferProtocol::Coe { index: a.sdo.0, subindex: a.sdo.1 },
                    _ => TransferProtocol::Eoe { mtu_bytes: a.eoe_mtu, addresses },
                };
                let generated = |n: u64, seed: u64| {
                    let mut v = vec![0u8; n as usize];
                    flowlab_sim::payload_bytes(seed, 0, &mut v);
                    v
                };
                let (payload, name) = if to_slave {
                    match (&a.file, a.bytes) {
                        (Some(f), _) => (
                            Payload::Send(std::fs::read(f).map_err(|e| format!("{}: {e}", f.display()))?),
                            crate::file_name(f),
                        ),
                        (None, Some(n)) => {
                            let seed = a.seed.unwrap_or(0);
                            (Payload::Send(generated(n, seed)), format!("{n} B of flowlab-sim payload, seed {seed}"))
                        }
                        (None, None) => return Err("name the payload: --file FILE or --bytes N".into()),
                    }
                } else {
                    let expected_sha256 = a
                        .expect_sha256
                        .clone()
                        .or_else(|| a.seed.zip(a.bytes).map(|(seed, n)| flowlab_sim::payload_sha256(seed, n)));
                    let name = format!("received from {:#06x}", a.station);
                    (Payload::Receive { expected_bytes: a.bytes, expected_sha256 }, name)
                };
                let policy = MailboxPolicy::FixedQuota { accesses_per_cycle: 2 };
                (protocol, payload, name, 1_000_000, policy)
            }
        };
        if a.like.is_some() && a.direction != "to-slave" {
            eprintln!("note: --direction is taken from the --like spec");
        }
        let policy = a.policy.clone().unwrap_or(policy);
        let cycle_ns = a.cycle_us.map_or(cycle_ns, |us| us * 1000);
        if a.max_duration.is_nan() || a.max_duration <= 0.0 {
            return Err("--max-duration must be positive".into());
        }
        let options = TransferOptions {
            cycle_time_ns: cycle_ns,
            policy,
            poll: if a.poll == "status" { PollMode::Status } else { PollMode::Read },
            reply_timeout_ns: a.reply_timeout_ms * 1_000_000,
            max_resends: a.max_resends,
            max_duration_ns: (a.max_duration * 1e9) as u64,
            first_counter: a.first_counter,
            max_cycle_records: a.records,
            ..TransferOptions::default()
        };
        let job = TransferJob {
            slave: String::new(),
            mailbox: flowlab_master::MailboxLayout {
                station: a.station,
                out_start: 0,
                out_len: 0,
                in_start: 0,
                in_len: 0,
            },
            protocol,
            payload,
            payload_name,
        };
        let session = open(&a.safety)?;
        eprintln!(
            "transfer to/from {:#06x}: {}, cycle {}",
            a.station,
            policy_text(&options.policy),
            ns(cycle_ns as f64)
        );
        let r = session.transfer(a.station, job, &options, a.op)?;
        print_transfer(&r);
        if let (Some(p), Some(data)) = (&a.save_received, &r.received) {
            ensure_parent(p)?;
            std::fs::write(p, data)?;
            eprintln!("saved {} received bytes to {}", data.len(), p.display());
        }
        write_json(&out_path(a.out, "transfer"), serde_json::to_value(&r)?)?;
        if r.completed {
            Ok(())
        } else {
            Err(format!("the transfer did not complete: {}", r.error.unwrap_or_default()).into())
        }
    }

    fn print_transfer(r: &MeasuredTransfer) {
        let opt = |v: Option<f64>| v.map_or_else(|| "–".to_owned(), ns);
        println!(
            "{} {} {:#06x} \"{}\"  [MEASURED, host timing]  {}, cycle {}",
            r.protocol,
            r.direction.replace('_', " "),
            r.station,
            r.slave,
            r.policy,
            ns(r.cycle_time_ns)
        );
        println!(
            "  {} of {} B in {} (exchange {}), goodput {}, link bound {}",
            r.delivered_bytes,
            r.payload_bytes,
            opt(r.duration_ns),
            opt(r.exchange_ns),
            r.goodput_bps.map_or_else(|| "–".into(), |g| format!("{:.2} Mbit/s", g / 1e6)),
            ns(r.physical_bound_ns)
        );
        println!(
            "  messages {} to / {} from the slave, {} rejected writes, {} empty reads, {} lost frames, {} repeats, {} resent",
            r.messages_to_slave,
            r.messages_from_slave,
            r.rejected_writes,
            r.empty_reads,
            r.lost_frames,
            r.repeat_requests,
            r.resent_messages
        );
        println!(
            "  {} cycles, {} overruns; access round trip p50 {} p99 {}; SHA-256 {}{}",
            r.timing.cycles,
            r.timing.overruns,
            ns(r.timing.mailbox_access.p50_ns),
            ns(r.timing.mailbox_access.p99_ns),
            r.sha256,
            match r.verified {
                Some(true) => " (verified)",
                Some(false) => " (MISMATCH)",
                None => "",
            }
        );
        if let Some(e) = &r.error {
            println!("  stopped: {e}");
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn policies_parse_like_the_simulator_names_them() {
            assert_eq!(parse_policy("fixed-quota:3"), Ok(MailboxPolicy::FixedQuota { accesses_per_cycle: 3 }));
            assert_eq!(parse_policy("round-robin:1"), Ok(MailboxPolicy::RoundRobin { slaves_per_cycle: 1 }));
            assert_eq!(
                parse_policy("opportunistic:60"),
                Ok(MailboxPolicy::Opportunistic { bus_budget_percent: 60.0, turnaround_ns: 10_000 })
            );
            assert!(parse_policy("greedy").is_err());
            assert_eq!(parse_object("0x2000:1"), Ok((0x2000, 1)));
            assert_eq!(parse_mac("02:00:00:00:00:01"), Ok([2, 0, 0, 0, 0, 1]));
            assert_eq!(parse_ip("192.0.2.2"), Ok([192, 0, 2, 2]));
        }

        #[test]
        fn nothing_is_sent_without_the_acknowledgement() {
            let mut s = Safety {
                interface: "lo".into(),
                isolated_lab_network: false,
                no_master_check: true,
                listen: 0.0,
                tool: None,
                pdu_timeout_ms: 30,
            };
            let Err(e) = open(&s) else { panic!("a session opened without the acknowledgement") };
            let e = e.to_string();
            assert!(e.contains("--isolated-lab-network"), "{e}");
            s.listen = f64::NAN;
            assert!(segment_check(&s).is_err());
        }
    }
}

pub(crate) use imp::{MasterArgs, run};
