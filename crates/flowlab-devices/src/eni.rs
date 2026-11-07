//! EtherCAT Network Information (ENI, ETG.2100): how a master configured one network.
//!
//! Only what the analysis uses is read: slaves (station address, ring position, identity,
//! process data size, mailbox SyncManagers) and the cyclic commands with their expected
//! working counters.

use std::fmt::Write as _;

use serde::Serialize;
use ts_rs::TS;

use crate::DeviceError;
use crate::xml::{child, child_num, children, document, text};

/// A slave's mailbox as configured.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct MailboxConfig {
    /// Master → slave SyncManager start ("Send").
    pub out_start: u16,
    /// Its length.
    pub out_len: u16,
    /// Slave → master SyncManager start ("Recv").
    pub in_start: u16,
    /// Its length.
    pub in_len: u16,
    /// Protocols listed for the slave.
    pub protocols: Vec<String>,
}

/// One configured slave.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct SlaveConfig {
    /// Name.
    pub name: String,
    /// Configured station address.
    pub station: u16,
    /// Ring position (from the auto-increment address).
    pub position: Option<u16>,
    /// Vendor id.
    pub vendor_id: Option<u32>,
    /// Product code.
    pub product_code: Option<u32>,
    /// Revision.
    pub revision: Option<u32>,
    /// Output process data, bits.
    pub outputs_bits: u32,
    /// Input process data, bits.
    pub inputs_bits: u32,
    /// Mailbox.
    pub mailbox: Option<MailboxConfig>,
}

/// One datagram of a cyclic frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct CyclicCmd {
    /// Command code (e.g. 12 = LRW).
    pub command: u8,
    /// Raw 32-bit address: logical address, or ADP | ADO << 16.
    pub address: u32,
    /// Data length, bytes.
    pub length: u16,
    /// Expected working counter.
    pub expected_wkc: u16,
}

/// One cyclic frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct CyclicFrame {
    /// Its datagrams, in order.
    pub cmds: Vec<CyclicCmd>,
}

/// What an ENI file says about the network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct NetworkConfig {
    /// Master name, if given.
    pub master: Option<String>,
    /// Cycle time from `<Cyclic><CycleTime>` (µs in the file), ns.
    pub cycle_time_ns: Option<u64>,
    /// Slaves in file order.
    pub slaves: Vec<SlaveConfig>,
    /// Cyclic frames.
    pub cyclic: Vec<CyclicFrame>,
}

impl NetworkConfig {
    /// The configured expected WKC of a datagram with this command, raw address and length.
    pub fn expected_wkc(&self, command: u8, address: u32, length: u16) -> Option<u16> {
        self.cyclic
            .iter()
            .flat_map(|f| &f.cmds)
            .find(|c| c.command == command && c.address == address && c.length == length)
            .map(|c| c.expected_wkc)
    }

    /// The configured slave with this station address.
    pub fn slave(&self, station: u16) -> Option<&SlaveConfig> {
        self.slaves.iter().find(|s| s.station == station)
    }
}

/// Parse an ENI (`<EtherCATConfig>`) document.
pub fn parse_eni(xml: &str) -> Result<NetworkConfig, DeviceError> {
    let doc = document(xml, "EtherCATConfig")?;
    let config = child(doc.root_element(), "Config").ok_or_else(|| DeviceError::Format("no <Config>".into()))?;
    let master =
        child(config, "Master").and_then(|m| child(m, "Info")).and_then(|i| text(i, "Name")).map(str::to_owned);
    let mut slaves = Vec::new();
    for s in children(config, "Slave") {
        let info = child(s, "Info").ok_or_else(|| DeviceError::Format("a <Slave> has no <Info>".into()))?;
        let station =
            child_num(info, "PhysAddr").ok_or_else(|| DeviceError::Format("a slave has no <PhysAddr>".into()))? as u16;
        let position = child_num(info, "AutoIncAddr").map(|a| (a as u16).wrapping_neg());
        let pd = child(s, "ProcessData");
        let bits = |dir: &str| -> u32 {
            pd.map(|p| children(p, dir).filter_map(|d| child_num(d, "BitLength")).sum::<u64>() as u32).unwrap_or(0)
        };
        let mailbox = child(s, "Mailbox").and_then(|m| {
            let send = child(m, "Send")?;
            let recv = child(m, "Recv")?;
            Some(MailboxConfig {
                out_start: child_num(send, "Start")? as u16,
                out_len: child_num(send, "Length")? as u16,
                in_start: child_num(recv, "Start")? as u16,
                in_len: child_num(recv, "Length")? as u16,
                protocols: children(m, "Protocol").filter_map(|p| p.text()).map(|t| t.trim().to_owned()).collect(),
            })
        });
        slaves.push(SlaveConfig {
            name: text(info, "Name").unwrap_or_default().to_owned(),
            station,
            position,
            vendor_id: child_num(info, "VendorId").map(|v| v as u32),
            product_code: child_num(info, "ProductCode").map(|v| v as u32),
            revision: child_num(info, "RevisionNo").map(|v| v as u32),
            outputs_bits: bits("Send"),
            inputs_bits: bits("Recv"),
            mailbox,
        });
    }
    let mut cyclic = Vec::new();
    let mut cycle_time_ns = None;
    for c in children(config, "Cyclic") {
        if let Some(t) = child_num(c, "CycleTime") {
            cycle_time_ns = Some(t * 1000);
        }
        for f in children(c, "Frame") {
            let mut cmds = Vec::new();
            for cmd in children(f, "Cmd") {
                let Some(code) = child_num(cmd, "Cmd") else { continue };
                let address = match child_num(cmd, "Addr") {
                    Some(a) => a as u32,
                    None => {
                        let adp = child_num(cmd, "Adp").unwrap_or(0) as u16 as u32;
                        let ado = child_num(cmd, "Ado").unwrap_or(0) as u16 as u32;
                        adp | ado << 16
                    }
                };
                let length = child_num(cmd, "DataLength")
                    .or_else(|| text(cmd, "Data").map(|d| (d.len() / 2) as u64))
                    .unwrap_or(0) as u16;
                cmds.push(CyclicCmd {
                    command: code as u8,
                    address,
                    length,
                    expected_wkc: child_num(cmd, "Cnt").unwrap_or(0) as u16,
                });
            }
            cyclic.push(CyclicFrame { cmds });
        }
    }
    Ok(NetworkConfig { master, cycle_time_ns, slaves, cyclic })
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Write `cfg` as a minimal ENI document that [`parse_eni`] reads back.
pub fn write_eni(cfg: &NetworkConfig) -> String {
    let mut x =
        String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<EtherCATConfig Version=\"1.3\">\n  <Config>\n");
    let _ = writeln!(
        x,
        "    <Master>\n      <Info>\n        <Name>{}</Name>\n      </Info>\n    </Master>",
        esc(cfg.master.as_deref().unwrap_or("master"))
    );
    for s in &cfg.slaves {
        x.push_str("    <Slave>\n      <Info>\n");
        let _ = writeln!(x, "        <Name>{}</Name>", esc(&s.name));
        let _ = writeln!(x, "        <PhysAddr>{}</PhysAddr>", s.station);
        if let Some(p) = s.position {
            let _ = writeln!(x, "        <AutoIncAddr>{}</AutoIncAddr>", p.wrapping_neg());
        }
        for (tag, v) in [("VendorId", s.vendor_id), ("ProductCode", s.product_code), ("RevisionNo", s.revision)] {
            if let Some(v) = v {
                let _ = writeln!(x, "        <{tag}>{v}</{tag}>");
            }
        }
        x.push_str("      </Info>\n      <ProcessData>\n");
        if s.outputs_bits > 0 {
            let _ = writeln!(x, "        <Send>\n          <BitLength>{}</BitLength>\n        </Send>", s.outputs_bits);
        }
        if s.inputs_bits > 0 {
            let _ = writeln!(x, "        <Recv>\n          <BitLength>{}</BitLength>\n        </Recv>", s.inputs_bits);
        }
        x.push_str("      </ProcessData>\n");
        if let Some(m) = &s.mailbox {
            let _ = writeln!(
                x,
                "      <Mailbox DataLinkLayer=\"true\">\n        <Send>\n          <Start>{}</Start>\n          <Length>{}</Length>\n        </Send>\n        <Recv>\n          <Start>{}</Start>\n          <Length>{}</Length>\n        </Recv>",
                m.out_start, m.out_len, m.in_start, m.in_len
            );
            for p in &m.protocols {
                let _ = writeln!(x, "        <Protocol>{}</Protocol>", esc(p));
            }
            x.push_str("      </Mailbox>\n");
        }
        x.push_str("    </Slave>\n");
    }
    x.push_str("    <Cyclic>\n");
    if let Some(t) = cfg.cycle_time_ns {
        let _ = writeln!(x, "      <CycleTime>{}</CycleTime>", t / 1000);
    }
    for f in &cfg.cyclic {
        x.push_str("      <Frame>\n");
        for c in &f.cmds {
            let _ = writeln!(
                x,
                "        <Cmd>\n          <State>SAFEOP</State>\n          <State>OP</State>\n          <Cmd>{}</Cmd>",
                c.command
            );
            if ecat_wire::Command::from_code(c.command).address_mode() == ecat_wire::AddressMode::Logical {
                let _ = writeln!(x, "          <Addr>{}</Addr>", c.address);
            } else {
                let _ = writeln!(
                    x,
                    "          <Adp>{}</Adp>\n          <Ado>{}</Ado>",
                    c.address & 0xFFFF,
                    c.address >> 16
                );
            }
            let _ = writeln!(
                x,
                "          <DataLength>{}</DataLength>\n          <Cnt>{}</Cnt>\n        </Cmd>",
                c.length, c.expected_wkc
            );
        }
        x.push_str("      </Frame>\n");
    }
    x.push_str("    </Cyclic>\n  </Config>\n</EtherCATConfig>\n");
    x
}
