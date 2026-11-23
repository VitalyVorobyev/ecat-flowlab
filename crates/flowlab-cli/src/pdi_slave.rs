//! `flowlab pdi-slave`: FlowLab's slave application on a real ESC (a LAN9252 over Linux
//! spidev), in place of the ESC's own application, so the mailbox sizes and the PDI access
//! pattern are FlowLab's.
//!
//! Compiled with the cargo feature `slave`. It writes AL Status and the mailbox buffers of a
//! real ESC, so it needs `--replace-slave-application`: the slave's own application must be
//! stopped first, or both serve the ESC.

#[cfg(not(feature = "slave"))]
mod imp {
    use crate::Result;

    /// Arguments of `flowlab pdi-slave` in a build without it.
    #[derive(clap::Args)]
    pub(crate) struct PdiSlaveArgs {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
        rest: Vec<String>,
    }

    pub(crate) fn run(_: PdiSlaveArgs) -> Result<()> {
        Err("this flowlab was built without the slave applications. `pdi-slave` drives a real ESC, so it is \
             opt-in: build with `cargo build --release -p flowlab-cli --features slave`. See the book's \
             \"PDI slave\" page."
            .into())
    }
}

#[cfg(feature = "slave")]
mod imp {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    use clap::ValueEnum;
    use flowlab_mailbox::msg::EoeIpParameters;
    use flowlab_mailbox::responder::ResponderLog;
    use flowlab_pdi::{Lan9252, ReadInstruction, SlaveAccess, SpiBus, SpiTuning};
    use flowlab_slave::pdi::{AlChange, MIN_MAILBOX_BYTES, PdiSlave, PdiSlaveConfig, PdiSlaveStats};
    use flowlab_slave::wire::EthernetPort;

    use crate::Result;

    #[derive(Clone, Copy, ValueEnum)]
    enum ReadArg {
        /// READ (0x03), up to 30 MHz.
        Read,
        /// FAST READ (0x0B) with one dummy byte, up to 80 MHz.
        FastRead,
    }

    /// Arguments of `flowlab pdi-slave`.
    #[derive(clap::Args)]
    pub(crate) struct PdiSlaveArgs {
        /// spidev device of the LAN9252, e.g. /dev/spidev1.0.
        #[arg(long, value_name = "PATH")]
        device: PathBuf,
        /// The slave's own application is stopped and nothing else uses the ESC: take it over.
        /// Nothing is accessed without it.
        #[arg(long)]
        replace_slave_application: bool,
        /// SPI clock, Hz (READ up to 30 MHz, FAST READ up to 80 MHz).
        #[arg(long, default_value_t = 10_000_000, value_name = "HZ")]
        clock_hz: u32,
        /// SPI mode; the LAN9252 takes 0 or 3.
        #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(u8).range(0..=3))]
        mode: u8,
        /// SPI read instruction.
        #[arg(long, value_enum, default_value_t = ReadArg::Read)]
        read_instruction: ReadArg,
        /// Chip ID expected in ID_REV bits 31:16.
        #[arg(long, default_value = "0x9252", value_parser = parse_hex16, value_name = "HEX")]
        chip_id: u16,
        /// Largest mailbox SyncManager accepted for PRE-OP, bytes.
        #[arg(long, default_value_t = 1024, value_name = "BYTES")]
        max_mailbox_bytes: u16,
        /// What the slave sends (FoE read, SDO upload, VoE from the slave): this file.
        #[arg(long, conflicts_with = "upload_bytes")]
        upload: Option<PathBuf>,
        /// What the slave sends: this many bytes of the simulator's payload for `--seed`, the
        /// bytes `flowlab master transfer --direction from-slave --bytes N --seed S` expects.
        #[arg(long)]
        upload_bytes: Option<u64>,
        /// Seed of the generated payload.
        #[arg(long, default_value_t = 0)]
        seed: u64,
        /// Stop after this long, seconds (default: until Ctrl-C).
        #[arg(long, value_name = "SECONDS")]
        duration: Option<f64>,
        /// How the SPI bus is used: `fast` (split writes, no abort), `stack` (as the slave
        /// stacks do) or a list of `split`, `batch`, `fifo`, `noabort`.
        #[arg(long, default_value = "fast", value_parser = parse_tuning, value_name = "TUNING")]
        spi_tuning: SpiTuning,
        /// Read and write whole mailbox buffers (else only the message and the buffer's last
        /// DWORD).
        #[arg(long)]
        whole_buffers: bool,
        /// Bridge EoE to this Ethernet interface (raw frames; needs root): one end of a veth
        /// pair whose other end, `--eoe-host`, is the slave's network interface.
        #[arg(long, requires = "eoe_host", value_name = "IFACE")]
        eoe_bridge: Option<String>,
        /// The slave's network interface behind `--eoe-bridge`: EoE Set IP Parameter requests
        /// are answered and applied to it (`ip link` / `ip addr`), and only frames it sent go to
        /// the master.
        #[arg(long, requires = "eoe_bridge", value_name = "IFACE")]
        eoe_host: Option<String>,
    }

    fn parse_tuning(s: &str) -> std::result::Result<SpiTuning, String> {
        SpiTuning::parse(s).map_err(|e| e.to_string())
    }

    /// The MAC address of interface `name`, from sysfs.
    fn interface_mac(name: &str) -> Result<[u8; 6]> {
        let path = format!("/sys/class/net/{name}/address");
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))?;
        let mut mac = [0u8; 6];
        let parts: Vec<&str> = text.trim().split(':').collect();
        if parts.len() != 6 {
            return Err(format!("{path}: `{}` is not a MAC address", text.trim()).into());
        }
        for (m, p) in mac.iter_mut().zip(parts) {
            *m = u8::from_str_radix(p, 16).map_err(|_| format!("{path}: `{}` is not a MAC address", text.trim()))?;
        }
        Ok(mac)
    }

    /// Apply an EoE Set IP Parameter request to interface `name` with `ip`.
    fn apply_set_ip(name: &str, p: &EoeIpParameters) -> Result<()> {
        let ip = |args: &[&str]| -> Result<()> {
            let out = std::process::Command::new("ip").args(args).output().map_err(|e| format!("ip: {e}"))?;
            if !out.status.success() {
                return Err(format!("ip {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()).into());
            }
            Ok(())
        };
        if let Some(m) = p.mac {
            let mac = m.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":");
            ip(&["link", "set", "dev", name, "down"])?;
            ip(&["link", "set", "dev", name, "address", &mac])?;
        }
        if let Some(a) = p.ip {
            let prefix = p.subnet_mask.map_or(24, |m| u32::from_be_bytes(m).count_ones());
            let addr = format!("{}.{}.{}.{}/{prefix}", a[0], a[1], a[2], a[3]);
            ip(&["addr", "flush", "dev", name])?;
            ip(&["addr", "add", &addr, "dev", name])?;
        }
        ip(&["link", "set", "dev", name, "up"])
    }

    fn parse_hex16(s: &str) -> std::result::Result<u16, String> {
        let h = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).unwrap_or(s);
        u16::from_str_radix(h, 16).map_err(|_| format!("`{s}` is not a 16-bit hex number (e.g. 0x9252)"))
    }

    fn report(log: ResponderLog) {
        let sha = flowlab_core::sha256_hex;
        for (name, data) in &log.files {
            eprintln!("FoE file `{name}`: {} B, SHA-256 {}", data.len(), sha(data));
        }
        for data in &log.objects {
            eprintln!("SDO download: {} B, SHA-256 {}", data.len(), sha(data));
        }
        for data in &log.voe_received {
            eprintln!("VoE payload: {} B, SHA-256 {}", data.len(), sha(data));
        }
        if log.voe_sent > 0 {
            eprintln!("VoE upload sent");
        }
        if !log.eoe_frames.is_empty() {
            eprintln!("EoE: {} frame(s)", log.eoe_frames.len());
        }
    }

    fn print_change(c: AlChange) {
        let mbx = c.mailboxes.map_or(String::new(), |m| {
            format!(": mailbox out {} B at {:#06x}, in {} B at {:#06x}", m.out_len, m.out_start, m.in_len, m.in_start)
        });
        match c.error {
            Some(code) => eprintln!("AL {} (refused, status code {code:#06x})", c.state.name()),
            None => eprintln!("AL {}{mbx}", c.state.name()),
        }
    }

    fn print_rate(now: PdiSlaveStats, before: PdiSlaveStats, s: f64) {
        let d = |a: u64, b: u64| (a - b) as f64 / s;
        eprintln!(
            "{:>8.0} polls/s  {:>6.0} msg/s in ({:>7.1} kB/s)  {:>6.0} msg/s out ({:>7.1} kB/s)",
            d(now.polls, before.polls),
            d(now.messages_in, before.messages_in),
            d(now.bytes_in, before.bytes_in) / 1000.0,
            d(now.messages_out, before.messages_out),
            d(now.bytes_out, before.bytes_out) / 1000.0
        );
    }

    /// Set the returned flag on Ctrl-C or SIGTERM.
    fn stop_flag() -> Arc<AtomicBool> {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        std::thread::spawn(move || {
            let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() else { return };
            rt.block_on(async {
                #[cfg(unix)]
                {
                    use tokio::signal::unix::{SignalKind, signal};
                    let Ok(mut term) = signal(SignalKind::terminate()) else { return };
                    tokio::select! {
                        _ = tokio::signal::ctrl_c() => {}
                        _ = term.recv() => {}
                    }
                }
                #[cfg(not(unix))]
                let _ = tokio::signal::ctrl_c().await;
            });
            flag.store(true, Ordering::Relaxed);
        });
        stop
    }

    pub(crate) fn run(a: PdiSlaveArgs) -> Result<()> {
        if !a.replace_slave_application {
            return Err("pdi-slave takes the ESC over from the slave's own application: stop that application \
                        first (e.g. `docker stop` its container), then pass --replace-slave-application"
                .into());
        }
        if a.max_mailbox_bytes < MIN_MAILBOX_BYTES {
            return Err(format!("--max-mailbox-bytes must be at least {MIN_MAILBOX_BYTES}").into());
        }
        let duration = match a.duration {
            Some(d) if !(d.is_finite() && d > 0.0) => return Err("--duration must be positive".into()),
            d => d.map(Duration::from_secs_f64),
        };
        let upload = match (&a.upload, a.upload_bytes) {
            (Some(p), _) => std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))?,
            (None, Some(n)) => flowlab_mailbox::payload(a.seed, n as usize),
            (None, None) => Vec::new(),
        };
        let instruction = match a.read_instruction {
            ReadArg::Read => ReadInstruction::Read,
            ReadArg::FastRead => ReadInstruction::FastRead,
        };
        if matches!(instruction, ReadInstruction::Read) && a.clock_hz > 30_000_000 {
            eprintln!("warning: the LAN9252 specifies READ up to 30 MHz; use --read-instruction fast-read above it");
        }
        let mut bridge = match (&a.eoe_bridge, &a.eoe_host) {
            (Some(b), Some(h)) => Some((EthernetPort::open(b)?, h.clone(), interface_mac(h)?)),
            _ => None,
        };
        let bus = flowlab_pdi::open_spidev(&a.device, a.clock_hz, a.mode)?;
        let esc = Lan9252::new(bus).with_read_instruction(instruction).with_tuning(a.spi_tuning);
        let esc = SlaveAccess::take_over(esc, a.chip_id)?;
        let link = esc.link();
        eprintln!(
            "LAN9252 rev {} on {} at {} Hz (SPI tuning {}, {}): ESC was in {}; FlowLab's slave application \
             takes over (mailboxes up to {} B; FoE, CoE, EoE, VoE). Ctrl-C stops.",
            link.revision(),
            a.device.display(),
            a.clock_hz,
            a.spi_tuning.name(),
            if a.whole_buffers { "whole buffers" } else { "message-only buffer accesses" },
            link.al_state(),
            a.max_mailbox_bytes
        );
        if let Some((_, host, _)) = &bridge {
            eprintln!(
                "EoE bridged to {} (host side {host}); Set IP Parameter requests are applied to {host}",
                a.eoe_bridge.as_deref().unwrap_or_default()
            );
        }
        if !upload.is_empty() {
            eprintln!("upload: {} B, SHA-256 {}", upload.len(), flowlab_core::sha256_hex(&upload));
        }
        let mut cfg = PdiSlaveConfig {
            max_mailbox_bytes: a.max_mailbox_bytes,
            answer_set_ip: bridge.is_some(),
            message_only: !a.whole_buffers,
            ..PdiSlaveConfig::default()
        };
        cfg.responder.upload = upload;
        let mut slave = PdiSlave::new(esc, cfg)?;
        let stop = stop_flag();
        let origin = Instant::now();
        let mut second = Instant::now();
        let mut before = slave.stats();
        let mut eoe = BridgeStats::default();
        let mut eoe_before = eoe;
        let result = loop {
            if stop.load(Ordering::Relaxed) || duration.is_some_and(|d| origin.elapsed() >= d) {
                break Ok(());
            }
            match slave.step() {
                Ok((Some(change), _)) => {
                    print_change(change);
                    report(slave.drain_log());
                }
                Ok(_) => {}
                Err(e) => break Err(e.into()),
            }
            if let Some((port, host, host_mac)) = bridge.as_mut()
                && let Err(e) = pump_bridge(&mut slave, port, host, host_mac, &mut eoe)
            {
                break Err(e);
            }
            if second.elapsed() >= Duration::from_secs(1) {
                let now = slave.stats();
                if now.messages_in != before.messages_in || now.messages_out != before.messages_out {
                    print_rate(now, before, second.elapsed().as_secs_f64());
                    if bridge.is_some() {
                        eprintln!(
                            "         EoE bridge: {} frames ({} B) to the host, {} frames ({} B) to the master",
                            eoe.to_host - eoe_before.to_host,
                            eoe.to_host_bytes - eoe_before.to_host_bytes,
                            eoe.to_master - eoe_before.to_master,
                            eoe.to_master_bytes - eoe_before.to_master_bytes
                        );
                    }
                }
                eoe_before = eoe;
                report(slave.drain_log());
                before = now;
                second = Instant::now();
            }
        };
        let stopped = slave.stop();
        report(slave.drain_log());
        let st = slave.stats();
        eprintln!(
            "{} polls, {} AL requests, {} messages in, {} out; AL Status set to INIT",
            st.polls, st.al_requests, st.messages_in, st.messages_out
        );
        result?;
        stopped?;
        Ok(())
    }

    /// Frames moved by the EoE bridge.
    #[derive(Debug, Clone, Copy, Default)]
    struct BridgeStats {
        to_host: u64,
        to_host_bytes: u64,
        to_master: u64,
        to_master_bytes: u64,
    }

    /// Messages waiting for "mailbox in" before the bridge stops taking frames from the host.
    const BRIDGE_OUTBOX: usize = 8;

    /// Move frames between the slave's EoE and the bridge port; apply Set IP requests.
    fn pump_bridge<S: SpiBus>(
        slave: &mut PdiSlave<SlaveAccess<S>>,
        port: &mut EthernetPort,
        host: &str,
        host_mac: &mut [u8; 6],
        st: &mut BridgeStats,
    ) -> Result<()> {
        for p in slave.take_set_ip() {
            apply_set_ip(host, &p)?;
            *host_mac = interface_mac(host)?;
            eprintln!("EoE Set IP Parameter applied to {host}: {p:?}");
        }
        for f in slave.take_eoe_frames() {
            port.send(&f)?;
            st.to_host += 1;
            st.to_host_bytes += f.len() as u64;
        }
        while slave.outbox_len() < BRIDGE_OUTBOX {
            let Some(f) = port.try_recv()? else { break };
            // The port also sees the frames it sent; only the host's go to the master.
            if f.len() >= 14 && f[6..12] == host_mac[..] {
                st.to_master += 1;
                st.to_master_bytes += f.len() as u64;
                slave.send_eoe_frame(&f);
            }
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use clap::Parser;

        use super::*;

        #[derive(Parser)]
        struct Cli {
            #[command(flatten)]
            args: PdiSlaveArgs,
        }

        #[test]
        fn nothing_is_accessed_without_the_acknowledgement() {
            let a = Cli::parse_from(["pdi-slave", "--device", "/dev/null"]).args;
            let Err(e) = run(a) else { panic!("the ESC was taken over without the acknowledgement") };
            assert!(e.to_string().contains("--replace-slave-application"), "{e}");
        }
    }
}

pub(crate) use imp::{PdiSlaveArgs, run};
