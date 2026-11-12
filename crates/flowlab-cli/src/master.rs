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
    use flowlab_mailbox::MailboxPolicy;
    use flowlab_master::hardware::{Session, SessionOptions};
    use flowlab_master::safety::{SegmentCheck, check_segment};
    use flowlab_master::{
        EoeAddresses, MeasuredTransfer, Payload, PollMode, TransferJob, TransferOptions, TransferProtocol, setup_like,
    };

    use crate::{Result, ensure_parent, ns, parse_object, parse_station};

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
        /// Move a payload to or from a slave with FoE, CoE, EoE or FlowLab VoE, under a mailbox
        /// policy and cycle time, and measure it like the simulator does.
        Transfer(Box<TransferArgs>),
        /// The master's own TCP/IP stack on a slave's EoE link: optionally configure the slave's
        /// interface (EoE Set IP Parameter), then forward local TCP ports (127.0.0.1) to the
        /// slave's ports until Ctrl-C or `--duration`. No host interface, no root.
        Gateway(Box<GatewayArgs>),
    }

    #[derive(clap::Args)]
    struct GatewayArgs {
        #[command(flatten)]
        safety: Safety,
        /// Station address of the slave.
        #[arg(long, value_parser = parse_station)]
        station: u16,
        /// The slave's IPv4 address.
        #[arg(long, value_parser = parse_ip)]
        slave_ip: [u8; 4],
        /// The master's IPv4 address on the EoE link (same subnet).
        #[arg(long, value_parser = parse_ip)]
        master_ip: [u8; 4],
        /// Prefix length of the subnet.
        #[arg(long, default_value_t = 24)]
        prefix: u8,
        /// Send an EoE Set IP Parameter request first: the slave's IP, the subnet mask, and
        /// `--slave-mac` / `--slave-gateway` if given.
        #[arg(long)]
        set_ip: bool,
        /// With --set-ip: the MAC address the slave's interface gets.
        #[arg(long, value_parser = parse_mac, requires = "set_ip")]
        slave_mac: Option<[u8; 6]>,
        /// With --set-ip: the slave's default gateway.
        #[arg(long, value_parser = parse_ip, requires = "set_ip")]
        slave_gateway: Option<[u8; 4]>,
        /// The master's MAC address on the EoE link.
        #[arg(long, value_parser = parse_mac, default_value = "02:46:4c:00:00:01")]
        master_mac: [u8; 6],
        /// IP MTU of the master's side, bytes: its TCP segments and the segment size it
        /// announces carry up to MTU − 40.
        #[arg(long, default_value_t = 1500)]
        mtu: u16,
        /// Forward LOCAL:REMOTE: 127.0.0.1:LOCAL to the slave's port REMOTE (repeatable).
        #[arg(long, value_parser = parse_forward)]
        forward: Vec<flowlab_master::PortForward>,
        /// TCP receive and send buffer per connection, bytes (the receive window).
        #[arg(long, default_value_t = 65536)]
        tcp_buffer: usize,
        /// Delay of the master's TCP acknowledgements, ms (0: acknowledge every segment at
        /// once).
        #[arg(long, default_value_t = 10)]
        ack_delay_ms: u64,
        /// One mailbox pass (a write if a message waits, and a read) per cycle of this length,
        /// µs, as a cyclic master serves the mailbox; 0: passes back to back.
        #[arg(long, default_value_t = 0, value_name = "US")]
        cycle_us: u64,
        /// Set the mailbox SyncManagers to this many bytes each (INIT, reconfigure, PRE-OP)
        /// instead of the EEPROM's sizes. Nothing is written to the EEPROM; a stack that does
        /// not allow the size refuses PRE-OP, and the original SyncManagers are restored.
        #[arg(long, value_name = "BYTES")]
        mailbox_bytes: Option<u16>,
        /// Stop after this long, seconds.
        #[arg(long, default_value_t = 3600.0, value_name = "SECONDS")]
        duration: f64,
        /// Write the result as JSON (default: results/master/gateway-UNIXTIME.json).
        #[arg(short, long)]
        out: Option<PathBuf>,
    }

    fn parse_forward(s: &str) -> std::result::Result<flowlab_master::PortForward, String> {
        let (l, r) = s.split_once(':').ok_or_else(|| format!("`{s}` is not LOCAL:REMOTE"))?;
        let port = |p: &str| p.parse::<u16>().map_err(|_| format!("`{p}` is not a port"));
        Ok(flowlab_master::PortForward { local_port: port(l)?, remote_port: port(r)? })
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
        #[arg(long, value_parser = ["foe", "coe", "eoe", "voe"], required_unless_present = "like")]
        protocol: Option<String>,
        /// Direction.
        #[arg(long, value_parser = ["to-slave", "from-slave"], default_value = "to-slave")]
        direction: String,
        /// To the slave: send this file.
        #[arg(long, conflicts_with = "bytes")]
        file: Option<PathBuf>,
        /// To the slave: send this many bytes of the simulator's payload for `--seed`. From
        /// the slave: wait for this many bytes (required for EoE; FlowLab VoE announces it).
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
        #[arg(long, value_parser = |s: &str| s.parse::<MailboxPolicy>())]
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
        /// FlowLab VoE: DATA messages per acknowledgement (the slave may grant fewer).
        #[arg(long, default_value_t = flowlab_mailbox::voe::DEFAULT_WINDOW)]
        voe_window: u16,
        /// Without a reply for this long, send the last message again with a new counter, ms.
        #[arg(long, default_value_t = 1000, value_name = "MS")]
        reply_timeout_ms: u64,
        /// Resends without progress before giving up.
        #[arg(long, default_value_t = 3)]
        max_resends: u32,
        /// Give up after this long, seconds.
        #[arg(long, default_value_t = 600.0, value_name = "SECONDS")]
        max_duration: f64,
        /// Counter of the first mailbox message, 0–7 (default: 1; with --op 0, the start
        /// value: EtherCrab used the mailbox to map the process data, so the slave's last
        /// counter is unknown).
        #[arg(long, value_parser = clap::value_parser!(u8).range(0..=7))]
        first_counter: Option<u8>,
        /// Keep at most this many per-cycle records in the JSON.
        #[arg(long, default_value_t = 100_000)]
        records: usize,
        /// Write the result as JSON (default: results/master/transfer-UNIXTIME.json).
        #[arg(short, long)]
        out: Option<PathBuf>,
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
            MasterCmd::Gateway(g) => gateway(*g),
        }
    }

    fn gateway(a: GatewayArgs) -> Result<()> {
        if a.duration.is_nan() || a.duration <= 0.0 {
            return Err("--duration must be positive".into());
        }
        let mask = |p: u8| (u32::MAX << (32 - u32::from(p.min(32)))).to_be_bytes();
        let set_ip = a.set_ip.then(|| flowlab_master::EoeIpParameters {
            mac: a.slave_mac,
            ip: Some(a.slave_ip),
            subnet_mask: Some(mask(a.prefix)),
            gateway: a.slave_gateway,
        });
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = stop.clone();
        std::thread::spawn(move || {
            if let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() {
                rt.block_on(async {
                    let _ = tokio::signal::ctrl_c().await;
                });
                flag.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        });
        let cfg = flowlab_master::GatewayConfig {
            mailbox: flowlab_master::MailboxLayout {
                station: a.station,
                out_start: 0,
                out_len: 0,
                in_start: 0,
                in_len: 0,
            },
            set_ip,
            slave_ip: a.slave_ip,
            master_mac: a.master_mac,
            master_ip: a.master_ip,
            prefix_len: a.prefix,
            mtu: a.mtu,
            forwards: a.forward.clone(),
            tcp_buffer_bytes: a.tcp_buffer,
            ack_delay_ms: (a.ack_delay_ms > 0).then_some(a.ack_delay_ms),
            cycle_ns: a.cycle_us * 1000,
            max_duration_ns: (a.duration * 1e9) as u64,
            stop,
            assumptions: Vec::new(),
        };
        let session = open(&a.safety)?;
        let ip = |o: [u8; 4]| std::net::Ipv4Addr::from(o).to_string();
        eprintln!(
            "EoE gateway to {:#06x}: master {} ↔ slave {} (MTU {}); Ctrl-C stops",
            a.station,
            ip(a.master_ip),
            ip(a.slave_ip),
            a.mtu
        );
        for f in &a.forward {
            eprintln!("  127.0.0.1:{} → {}:{}", f.local_port, ip(a.slave_ip), f.remote_port);
        }
        let r = session.gateway(a.station, cfg, a.mailbox_bytes, &mut |line| eprintln!("{line}"))?;
        let secs = r.duration_ns / 1e9;
        println!(
            "EoE gateway {:#06x} \"{}\"  [MEASURED, host timing]  {:.1} s, mailbox {}/{} B",
            r.station, r.slave, secs, r.mailbox_out_bytes, r.mailbox_in_bytes
        );
        println!(
            "  messages {} to / {} from the slave ({:.0} / {:.0} per s), {} rejected writes, {} empty reads, {} lost, {} duplicates, {} other",
            r.messages_to_slave,
            r.messages_from_slave,
            r.messages_to_slave as f64 / secs,
            r.messages_from_slave as f64 / secs,
            r.rejected_writes,
            r.empty_reads,
            r.lost_frames,
            r.duplicate_messages,
            r.other_messages
        );
        println!(
            "  frames {} to ({} B) / {} from ({} B) the slave, {} broken fragments; access round trip p50 write {} read {}",
            r.frames_to_slave,
            r.frame_bytes_to_slave,
            r.frames_from_slave,
            r.frame_bytes_from_slave,
            r.broken_fragments,
            ns(r.write_access.p50_ns),
            ns(r.read_access.p50_ns)
        );
        for c in &r.connections {
            println!(
                "  :{} → :{}  {} B up, {} B down{}{}",
                c.local_port,
                c.remote_port,
                c.bytes_to_slave,
                c.bytes_from_slave,
                c.goodput_from_slave_bps.map_or(String::new(), |g| format!(", {:.1} kB/s down", g / 8e3)),
                c.error.as_ref().map_or(String::new(), |e| format!(" ({e})"))
            );
        }
        write_json(&out_path(a.out, "gateway"), serde_json::to_value(&r)?)
    }

    fn transfer(a: TransferArgs) -> Result<()> {
        let to_slave = a.direction == "to-slave";
        let (protocol, payload, payload_name, cycle_ns, policy) = match &a.like {
            Some(spec_path) => {
                let file = std::fs::File::open(spec_path).map_err(|e| format!("{}: {e}", spec_path.display()))?;
                let spec: flowlab_sim::SimSpec = serde_json::from_reader(file)?;
                let s = setup_like(&spec.transfer_plan(a.transfer)?)?;
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
                    Some("voe") => TransferProtocol::Voe { window: a.voe_window },
                    _ => TransferProtocol::Eoe { mtu_bytes: a.eoe_mtu, addresses },
                };
                let generated = |n: u64, seed: u64| flowlab_mailbox::payload(seed, n as usize);
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
                        .or_else(|| a.seed.zip(a.bytes).map(|(seed, n)| flowlab_mailbox::payload_sha256(seed, n)));
                    let name = format!("received from {:#06x}", a.station);
                    (Payload::Receive { expected_bytes: a.bytes, expected_sha256 }, name)
                };
                let policy = MailboxPolicy::default();
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
            first_counter: a.first_counter.unwrap_or(if a.op { 0 } else { 1 }),
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
        eprintln!("transfer to/from {:#06x}: {}, cycle {}", a.station, options.policy, ns(cycle_ns as f64));
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
        fn arguments_parse() {
            assert_eq!(parse_object("0x2000:1"), Ok((0x2000, 1)));
            assert_eq!(parse_mac("02:00:00:00:00:01"), Ok([2, 0, 0, 0, 0, 1]));
            assert_eq!(parse_ip("192.0.2.2"), Ok([192, 0, 2, 2]));
            assert_eq!(
                parse_forward("18080:8080"),
                Ok(flowlab_master::PortForward { local_port: 18080, remote_port: 8080 })
            );
            assert!(parse_forward("8080").is_err());
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
