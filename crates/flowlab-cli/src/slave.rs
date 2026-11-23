//! `flowlab slave`: an emulated EtherCAT slave on a network interface, for testing the lab
//! master, captures and the analysis without EtherCAT hardware.
//!
//! Compiled only with the cargo feature `slave`: it sends frames, so it is opt-in. Without the
//! feature the command only says how to get it.

#[cfg(not(feature = "slave"))]
mod imp {
    use crate::Result;

    /// Arguments of `flowlab slave` in a build without the emulated slave.
    #[derive(clap::Args)]
    pub(crate) struct SlaveArgs {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
        rest: Vec<String>,
    }

    pub(crate) fn run(_: SlaveArgs) -> Result<()> {
        Err("this flowlab was built without the emulated slave. It answers EtherCAT frames, so it is opt-in: \
             build with `cargo build --release -p flowlab-cli --features slave` (Linux, macOS). See the book's \
             \"Emulated slave\" page."
            .into())
    }
}

#[cfg(feature = "slave")]
mod imp {
    use std::path::PathBuf;
    use std::time::Duration;

    use flowlab_mailbox::responder::ResponderLog;
    use flowlab_slave::wire::{Event, ServeOptions, serve};
    use flowlab_slave::{EmulatedSlave, SlaveConfig};

    use crate::Result;

    /// Arguments of `flowlab slave`.
    #[derive(clap::Args)]
    pub(crate) struct SlaveArgs {
        /// Network interface the master is on (Linux: e.g. eth0; macOS: e.g. en7).
        #[arg(long, short = 'i')]
        interface: String,
        /// Confirm that the interface is on an isolated lab network: no production equipment,
        /// no other EtherCAT slaves. Nothing is sent without it.
        #[arg(long)]
        isolated_lab_network: bool,
        /// Bytes of each mailbox (SM0 and SM1), 32–1024.
        #[arg(long, default_value_t = 128)]
        mailbox_bytes: u16,
        /// Process data outputs (master → slave), bytes.
        #[arg(long, default_value_t = 4)]
        output_bytes: u16,
        /// Process data inputs (slave → master), bytes; they echo the outputs.
        #[arg(long, default_value_t = 4)]
        input_bytes: u16,
        /// What the slave sends (FoE read, SDO upload, EoE send): this file.
        #[arg(long, conflicts_with = "upload_bytes")]
        upload: Option<PathBuf>,
        /// What the slave sends: this many bytes of the simulator's payload for `--seed`, the
        /// bytes `flowlab master transfer --direction from-slave --bytes N --seed S` expects.
        #[arg(long)]
        upload_bytes: Option<u64>,
        /// Seed of the generated payload.
        #[arg(long, default_value_t = 0)]
        seed: u64,
        /// Send the upload over EoE (UDP frames), starting after PRE-OP.
        #[arg(long)]
        eoe_send: bool,
        /// How long after entering PRE-OP the EoE send starts, ms (after the master's CoE
        /// configuration).
        #[arg(long, default_value_t = 1000, value_name = "MS")]
        eoe_start_after_ms: u64,
        /// EoE: IP MTU of the slave's frames, bytes.
        #[arg(long, default_value_t = 1500)]
        eoe_mtu: usize,
        /// CoE transfer object, INDEX:SUBINDEX.
        #[arg(long, default_value = "0x2000:0", value_parser = crate::parse_object)]
        sdo: (u16, u8),
        /// Answer every n-th FoE DATA packet BUSY once.
        #[arg(long)]
        busy_every: Option<u32>,
        /// Serial number in the EEPROM.
        #[arg(long, default_value_t = 0)]
        serial: u32,
        /// The interface is cabled to the downstream port of a real slave (e.g. an EK1100's
        /// X2 OUT): answer the frames its ESC passes on, which already carry the ESC's source
        /// MAC bit.
        #[arg(long)]
        behind_esc: bool,
        /// Stop after this long, seconds (default: until Ctrl-C).
        #[arg(long, value_name = "SECONDS")]
        duration: Option<f64>,
    }

    /// EoE totals so far (the frames themselves are not kept).
    #[derive(Default)]
    struct Totals {
        eoe_frames: usize,
        eoe_bytes: usize,
        voe_sent: u64,
    }

    /// Print what the application received since the last call, and forget it there, so a
    /// long run neither grows nor stalls on copying what it received.
    fn report_new(log: ResponderLog, t: &mut Totals) {
        let sha = flowlab_core::sha256_hex;
        for (name, data) in &log.files {
            eprintln!("FoE file `{name}`: {} B, SHA-256 {}", data.len(), sha(data));
        }
        for data in &log.objects {
            eprintln!("SDO download: {} B, SHA-256 {}", data.len(), sha(data));
        }
        if !log.eoe_frames.is_empty() {
            t.eoe_frames += log.eoe_frames.len();
            t.eoe_bytes += log.eoe_frames.iter().map(Vec::len).sum::<usize>();
            eprintln!("EoE: {} frame(s), {} B so far", t.eoe_frames, t.eoe_bytes);
        }
        for data in &log.voe_received {
            eprintln!("VoE payload: {} B, SHA-256 {}", data.len(), sha(data));
        }
        if log.voe_sent > 0 {
            t.voe_sent += log.voe_sent;
            eprintln!("VoE upload sent ({} so far)", t.voe_sent);
        }
        for (index, sub, value) in &log.settings {
            eprintln!("setting {index:#06x}:{sub} written: {value:02x?} (applies at the next INIT)");
        }
    }

    pub(crate) fn run(a: SlaveArgs) -> Result<()> {
        let upload = match (&a.upload, a.upload_bytes) {
            (Some(p), _) => std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))?,
            (None, Some(n)) => flowlab_mailbox::payload(a.seed, n as usize),
            (None, None) => Vec::new(),
        };
        if a.eoe_send && upload.is_empty() {
            return Err("--eoe-send needs --upload or --upload-bytes".into());
        }
        let duration = match a.duration {
            Some(d) if !(d.is_finite() && d > 0.0) => return Err("--duration must be positive".into()),
            d => d.map(Duration::from_secs_f64),
        };
        let mut cfg = SlaveConfig {
            serial: a.serial,
            mailbox_bytes: a.mailbox_bytes,
            output_bytes: a.output_bytes,
            input_bytes: a.input_bytes,
            eoe_start_after_ns: a.eoe_start_after_ms * 1_000_000,
            behind_esc: a.behind_esc,
            ..SlaveConfig::default()
        };
        cfg.responder.sdo_index = a.sdo.0;
        cfg.responder.sdo_subindex = a.sdo.1;
        cfg.responder.busy_every = a.busy_every;
        cfg.responder.eoe_send = a.eoe_send;
        cfg.responder.eoe_mtu = a.eoe_mtu;
        if !upload.is_empty() {
            eprintln!("upload: {} B, SHA-256 {}", upload.len(), flowlab_core::sha256_hex(&upload));
        }
        cfg.responder.upload = upload;
        let mut slave = EmulatedSlave::new(cfg)?;
        let l = slave.layout();
        let options =
            ServeOptions { interface: a.interface.clone(), isolated_lab_network: a.isolated_lab_network, duration };
        let mut totals = Totals::default();
        let mut mailbox = l.mbx_len;
        serve(&mut slave, &options, |s, e| match e {
            Event::Listening => eprintln!(
                "emulated ESC on {}{}: mailbox {} B at {:#06x}/{:#06x}, outputs {} B, inputs {} B; FoE, CoE, EoE, VoE. \
                 Not an ESC: frames come back when this host has processed them. Ctrl-C stops.",
                a.interface,
                if a.behind_esc { " behind an ESC" } else { "" },
                l.mbx_len,
                l.mbx_out,
                l.mbx_in,
                l.output_len,
                l.input_len
            ),
            Event::State(state, code) => {
                match code {
                    Some(c) => {
                        eprintln!("AL {} (error, status code {c:#06x}), station {:#06x}", state.name(), s.station())
                    }
                    None => eprintln!("AL {}, station {:#06x}", state.name(), s.station()),
                }
                if s.layout().mbx_len != mailbox {
                    mailbox = s.layout().mbx_len;
                    let l = s.layout();
                    eprintln!("mailbox now {mailbox} B at {:#06x}/{:#06x} (setting)", l.mbx_out, l.mbx_in);
                }
                report_new(s.drain_log(), &mut totals);
            }
            Event::Second => report_new(s.drain_log(), &mut totals),
        })?;
        let st = slave.stats();
        eprintln!(
            "{} frames, {} datagrams, {} messages in, {} out, {} repeats",
            st.frames, st.datagrams, st.messages_to_slave, st.messages_from_slave, st.repeats
        );
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use clap::Parser;

        use super::*;

        #[derive(Parser)]
        struct Cli {
            #[command(flatten)]
            args: SlaveArgs,
        }

        #[test]
        fn nothing_is_sent_without_the_acknowledgement() {
            let a = Cli::parse_from(["slave", "-i", "lo", "--duration", "1"]).args;
            let Err(e) = run(a) else { panic!("the slave answered without the acknowledgement") };
            let e = e.to_string();
            assert!(e.contains("--isolated-lab-network"), "{e}");
            let a = Cli::parse_from(["slave", "-i", "lo", "--isolated-lab-network", "--eoe-send"]).args;
            assert!(run(a).is_err(), "an EoE send needs an upload");
        }
    }
}

pub(crate) use imp::{SlaveArgs, run};
