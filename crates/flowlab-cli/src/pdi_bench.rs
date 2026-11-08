//! `flowlab pdi-bench`: time complete ESC accesses over a LAN9252's SPI and write the samples
//! `flowlab pdi-fit` reads; and the fit shared by both commands.

use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::{Args, ValueEnum};
use flowlab_core::Provenance;
use flowlab_pdi::{
    Area, BenchOptions, Clock, DEFAULT_SIZES, Lan9252, MonotonicClock, ReadInstruction, SimLan9252, SimTiming, SpiBus,
};

use crate::{Result, ensure_parent, ns};

#[derive(Clone, Copy, ValueEnum)]
enum ReadArg {
    /// READ (0x03), up to 30 MHz.
    Read,
    /// FAST READ (0x0B) with one dummy byte, up to 80 MHz.
    FastRead,
}

/// Options of `flowlab pdi-bench`.
#[derive(Args)]
pub(crate) struct PdiBenchArgs {
    /// spidev device of the LAN9252 (Linux), e.g. /dev/spidev0.0.
    #[arg(long, value_name = "PATH", required_unless_present = "sim")]
    device: Option<PathBuf>,
    /// Time a simulated LAN9252 instead (no hardware); the samples are marked simulated.
    #[arg(long, conflicts_with = "device")]
    sim: bool,
    /// SPI clock, Hz (READ up to 30 MHz, FAST READ up to 80 MHz).
    #[arg(long, default_value_t = 10_000_000, value_name = "HZ")]
    clock_hz: u32,
    /// SPI mode; the LAN9252 takes 0 or 3.
    #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(u8).range(0..=3))]
    mode: u8,
    /// SPI read instruction.
    #[arg(long, value_enum, default_value_t = ReadArg::Read)]
    read_instruction: ReadArg,
    /// Access sizes, bytes, comma-separated.
    #[arg(long, value_delimiter = ',', default_values_t = DEFAULT_SIZES, value_name = "BYTES")]
    sizes: Vec<u32>,
    /// Timed accesses per direction and size.
    #[arg(long, default_value_t = 50)]
    reps: u32,
    /// Untimed accesses per direction and size before the timed ones.
    #[arg(long, default_value_t = 3)]
    warmup: u32,
    /// ESC range the accesses use. Reads default to process RAM from 0x1000; writes need an
    /// area in process RAM that nothing uses.
    #[arg(long, value_name = "START+LEN")]
    area: Option<Area>,
    /// Also time writes, into --area (overwrites it).
    #[arg(long)]
    allow_writes: bool,
    /// Do not read AL Event Request after every access (the SOES hardware layer does).
    #[arg(long)]
    no_al_event: bool,
    /// Chip ID expected in ID_REV bits 31:16.
    #[arg(long, default_value = "0x9252", value_parser = parse_hex16, value_name = "HEX")]
    chip_id: u16,
    /// Seed of the shuffled order.
    #[arg(long, default_value_t = 1)]
    seed: u64,
    /// Samples file (CSV for `flowlab pdi-fit`).
    #[arg(short, long, value_name = "CSV")]
    out: PathBuf,
    /// Fit the samples at once (as `flowlab pdi-fit`).
    #[arg(long)]
    fit: bool,
    /// Write the fit as JSON (implies --fit).
    #[arg(long, value_name = "JSON")]
    result: Option<PathBuf>,
    /// Store the fitted values in this lab-inputs file (implies --fit; hardware runs only).
    #[arg(long, value_name = "JSON", conflicts_with = "sim")]
    lab: Option<PathBuf>,
    /// With --sim: fixed cost of every SPI transaction, ns (a chosen value).
    #[arg(long, default_value_t = 5_000, value_name = "NS")]
    sim_transaction_ns: u64,
}

fn parse_hex16(s: &str) -> std::result::Result<u16, String> {
    let h = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).unwrap_or(s);
    u16::from_str_radix(h, 16).map_err(|_| format!("`{s}` is not a 16-bit hex number (e.g. 0x9252)"))
}

pub(crate) fn run(a: PdiBenchArgs) -> Result<()> {
    let options = BenchOptions {
        sizes: a.sizes.clone(),
        reps: a.reps,
        warmup: a.warmup,
        area: a.area,
        allow_writes: a.allow_writes,
        seed: a.seed,
    };
    let plan = flowlab_pdi::plan(&options)?;
    let instruction = match a.read_instruction {
        ReadArg::Read => ReadInstruction::Read,
        ReadArg::FastRead => ReadInstruction::FastRead,
    };
    let timing =
        SimTiming { clock_hz: u64::from(a.clock_hz), transaction_ns: a.sim_transaction_ns, ..SimTiming::default() };
    let (bus, clock, target): (Box<dyn SpiBus>, Box<dyn Clock>, String) = match &a.device {
        _ if a.sim => {
            let (sim, clock) = SimLan9252::new(timing);
            let target = format!(
                "a simulated LAN9252 (not a measurement; chosen timing: {} ns per transaction, {} ns CSR busy, {} ns until PRAM data)",
                timing.transaction_ns, timing.csr_busy_ns, timing.pram_ready_ns
            );
            (Box::new(sim), Box::new(clock), target)
        }
        Some(device) => {
            let bus = flowlab_pdi::open_spidev(device, a.clock_hz, a.mode)?;
            (bus, Box::new(MonotonicClock::new()), device.display().to_string())
        }
        None => return Err("name the SPI device with --device /dev/spidevB.C, or use --sim".into()),
    };
    if matches!(instruction, ReadInstruction::Read) && a.clock_hz > 30_000_000 {
        eprintln!("warning: the LAN9252 specifies READ up to 30 MHz; use --read-instruction fast-read above it");
    }
    let mut esc = Lan9252::new(bus).with_read_instruction(instruction).with_al_event_read(!a.no_al_event);
    eprintln!(
        "timing {} reads{} of {} sizes × {} on {target}",
        plan.read_area,
        plan.write_area.map_or(String::new(), |w| format!(" and writes at {w}")),
        plan.sizes.len(),
        a.reps
    );
    let bench = flowlab_pdi::run(&mut esc, &*clock, &options, a.chip_id)?;

    let provenance = if a.sim { Provenance::Simulated } else { Provenance::Measured };
    let unix_s = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let comments = [
        format!("flowlab pdi-bench {}: {target}", env!("CARGO_PKG_VERSION")),
        format!(
            "LAN9252 ID_REV 0x{:08X}, AL Status {}; SPI {} Hz, mode {}, {}; AL Event read after every access: {}",
            bench.link.id_rev,
            bench.link.al_state(),
            a.clock_hz,
            a.mode,
            instruction.name(),
            if a.no_al_event { "no" } else { "yes" }
        ),
        format!(
            "reads at {}, writes {}; {} timed after {} warm-up per direction and size, shuffled (seed {}); unix time {unix_s}",
            bench.plan.read_area,
            bench.plan.write_area.map_or("not allowed".into(), |w| format!("at {w}")),
            a.reps,
            a.warmup,
            a.seed
        ),
    ];
    let csv = flowlab_pdi::to_csv(&bench.samples, &comments, provenance);
    ensure_parent(&a.out)?;
    std::fs::write(&a.out, &csv).map_err(|e| format!("{}: {e}", a.out.display()))?;

    println!(
        "LAN9252 chip 0x{:04X} rev {}, {}; SPI {:.2} MHz mode {} {}  [{}]",
        bench.link.chip_id(),
        bench.link.revision(),
        bench.link.al_state(),
        f64::from(a.clock_hz) / 1e6,
        a.mode,
        instruction.name(),
        provenance.as_str().to_uppercase()
    );
    for s in flowlab_pdi::summarise(&bench.samples) {
        println!(
            "  {:<5} {:>6} B  ×{:<5} median {:>10}  p95 {:>10}  min {:>10}",
            s.direction.as_str(),
            s.bytes,
            s.samples,
            ns(s.median_ns),
            ns(s.p95_ns),
            ns(s.min_ns)
        );
    }
    eprintln!("wrote {} ({} samples)", a.out.display(), bench.samples.len());
    if a.fit || a.result.is_some() || a.lab.is_some() {
        fit_samples(csv.as_bytes(), &a.out.display().to_string(), &crate::file_name(&a.out), a.result, a.lab)?;
    }
    Ok(())
}

/// Fit a samples file, print the fit, and optionally write it (`result`) and store it in a
/// lab-inputs file (`lab`; measured samples only). `label` names the file in errors.
pub(crate) fn fit_samples(
    bytes: &[u8],
    label: &str,
    name: &str,
    result: Option<PathBuf>,
    lab: Option<PathBuf>,
) -> Result<()> {
    let text = String::from_utf8_lossy(bytes);
    let parsed = flowlab_core::pdi_samples::parse(&text).map_err(|e| format!("{label}: {e}"))?;
    let fit = flowlab_study::fit_pdi(&parsed, name, bytes)?;
    crate::print_pdi_fit(&fit);
    if let Some(p) = result {
        ensure_parent(&p)?;
        serde_json::to_writer_pretty(BufWriter::new(File::create(&p)?), &fit)?;
    }
    if let Some(p) = lab {
        let mut l = crate::read_lab(&p)?;
        l.set_pdi_fit(&fit).map_err(|e| format!("{e}; {} is unchanged", p.display()))?;
        crate::write_lab(&p, &l)?;
    }
    Ok(())
}
