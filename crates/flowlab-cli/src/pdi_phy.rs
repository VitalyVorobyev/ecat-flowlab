//! `flowlab pdi-phy`: the LAN9252's PHY registers through the ESC's MII management interface.

use std::path::PathBuf;
use std::time::Duration;

use clap::Args;
use flowlab_pdi::{Lan9252, PhyAccess, SimLan9252, SimTiming, SpiBus};

use crate::Result;
use crate::pdi_bench::parse_hex16;

/// Options of `flowlab pdi-phy`.
#[derive(Args)]
pub(crate) struct PdiPhyArgs {
    /// spidev device of the LAN9252 (Linux), e.g. /dev/spidev1.0.
    #[arg(long, value_name = "PATH", required_unless_present = "sim")]
    device: Option<PathBuf>,
    /// Use a simulated LAN9252 instead (no hardware).
    #[arg(long, conflicts_with = "device")]
    sim: bool,
    /// SPI clock, Hz.
    #[arg(long, default_value_t = 10_000_000, value_name = "HZ")]
    clock_hz: u32,
    /// SPI mode; the LAN9252 takes 0 or 3.
    #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(u8).range(0..=3))]
    mode: u8,
    /// Chip ID expected in ID_REV bits 31:16.
    #[arg(long, default_value = "0x9252", value_parser = parse_hex16, value_name = "HEX")]
    chip_id: u16,
    /// MII addresses of the PHYs to show, comma-separated.
    #[arg(long, value_delimiter = ',', default_values_t = [0u8, 1, 2, 3], value_name = "ADDR")]
    phys: Vec<u8>,
    /// Write VALUE to register REG of PHY PHY (`PHY:REG=VALUE`, numbers decimal or 0x hex).
    /// Repeatable; needs --allow-phy-writes.
    #[arg(long = "set", value_parser = parse_set, value_name = "PHY:REG=VALUE")]
    sets: Vec<(u8, u8, u16)>,
    /// Turn on auto-negotiation and auto-MDIX on this PHY. Repeatable; needs
    /// --allow-phy-writes.
    #[arg(long = "autoneg", value_name = "PHY")]
    autoneg: Vec<u8>,
    /// Allow --set and --autoneg: they change the EtherCAT ports' link until the next reset
    /// (or until the slave's application sets the PHYs again).
    #[arg(long)]
    allow_phy_writes: bool,
}

fn parse_number(s: &str) -> std::result::Result<u32, String> {
    let r = match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(h) => u32::from_str_radix(h, 16),
        None => s.parse(),
    };
    r.map_err(|_| format!("`{s}` is not a number"))
}

fn parse_set(s: &str) -> std::result::Result<(u8, u8, u16), String> {
    let err = || format!("`{s}`: expected PHY:REG=VALUE, e.g. 0:0=0x3300");
    let (target, value) = s.split_once('=').ok_or_else(err)?;
    let (phy, reg) = target.split_once(':').ok_or_else(err)?;
    let (phy, reg, value) = (parse_number(phy)?, parse_number(reg)?, parse_number(value)?);
    if phy > 31 || reg > 31 || value > 0xFFFF {
        return Err(format!("`{s}`: PHY and register are 0–31, the value 16 bits"));
    }
    Ok((phy as u8, reg as u8, value as u16))
}

pub(crate) fn run(a: PdiPhyArgs) -> Result<()> {
    let writes = !a.sets.is_empty() || !a.autoneg.is_empty();
    if writes && !a.allow_phy_writes {
        return Err("--set and --autoneg change the PHYs' link: pass --allow-phy-writes".into());
    }
    let (bus, target): (Box<dyn SpiBus>, String) = match &a.device {
        Some(d) => (flowlab_pdi::open_spidev(d, a.clock_hz, a.mode)?, format!("{} at {} Hz", d.display(), a.clock_hz)),
        None => (Box::new(SimLan9252::new(SimTiming::default()).0), "a simulated LAN9252".into()),
    };
    let mut phys = PhyAccess::open(Lan9252::new(bus), a.chip_id)?;
    let link = phys.link();
    println!("LAN9252 rev {} on {target}: ESC in {}", link.revision(), link.al_state());
    show(&mut phys, &a.phys)?;
    if !writes {
        return Ok(());
    }
    for &(phy, reg, value) in &a.sets {
        phys.write(phy, reg, value)?;
        println!("PHY {phy} register {reg} := 0x{value:04X}");
    }
    for &phy in &a.autoneg {
        phys.enable_autoneg(phy)?;
        println!("PHY {phy}: auto-negotiation and auto-MDIX on, restarted");
    }
    if !a.autoneg.is_empty() && a.device.is_some() {
        std::thread::sleep(Duration::from_secs(3));
    }
    println!("after:");
    show(&mut phys, &a.phys)
}

fn show<S: SpiBus>(phys: &mut PhyAccess<S>, which: &[u8]) -> Result<()> {
    for &phy in which {
        match phys.status(phy)? {
            None => println!("PHY {phy}: none"),
            Some(p) => {
                let regs = p.registers.iter().map(|(r, v)| format!("r{r}=0x{v:04X}")).collect::<Vec<_>>().join(" ");
                println!("PHY {phy}: {}\n       {regs}", p.summary());
            }
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
        args: PdiPhyArgs,
    }

    #[test]
    fn writes_need_the_opt_in() {
        let a = Cli::parse_from(["pdi-phy", "--sim", "--set", "0:0=0x3300"]).args;
        let Err(e) = run(a) else { panic!("a PHY was written without --allow-phy-writes") };
        assert!(e.to_string().contains("--allow-phy-writes"), "{e}");
        let a = Cli::parse_from(["pdi-phy", "--sim", "--autoneg", "0", "--allow-phy-writes"]).args;
        run(a).unwrap();
    }

    #[test]
    fn set_arguments_parse() {
        assert_eq!(parse_set("1:27=0xA00B"), Ok((1, 27, 0xA00B)));
        assert_eq!(parse_set("0:0=13056"), Ok((0, 0, 0x3300)));
        assert!(parse_set("0:32=1").is_err());
        assert!(parse_set("0=1").is_err());
    }
}
