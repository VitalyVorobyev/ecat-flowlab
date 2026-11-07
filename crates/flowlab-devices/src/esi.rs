//! EtherCAT Slave Information (ESI, ETG.2000): what a device type can do.

use serde::Serialize;
use ts_rs::TS;

use crate::DeviceError;
use crate::xml::{attr_bool, attr_num, child, child_num, children, document, text};

/// One SyncManager as the ESI declares it.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct SmDescription {
    /// SyncManager number (order in the file).
    pub index: u8,
    /// `"MBoxOut"`, `"MBoxIn"`, `"Outputs"`, `"Inputs"` or what the file says.
    pub kind: String,
    /// Physical start address.
    pub start: Option<u16>,
    /// Default length, bytes.
    pub default_size: Option<u16>,
    /// Minimum length, bytes.
    pub min_size: Option<u16>,
    /// Maximum length, bytes.
    pub max_size: Option<u16>,
    /// Control byte.
    pub control: Option<u8>,
    /// Enabled by default.
    pub enable: bool,
}

/// One PDO entry.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct PdoEntry {
    /// Object index (0 = padding).
    pub index: u16,
    /// Subindex.
    pub subindex: u8,
    /// Size, bits.
    pub bit_len: u16,
    /// Name.
    pub name: String,
    /// Data type.
    pub data_type: Option<String>,
}

/// One RxPDO (outputs) or TxPDO (inputs).
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct PdoDescription {
    /// PDO index (0x16xx outputs, 0x1Axx inputs).
    pub index: u16,
    /// Name.
    pub name: String,
    /// SyncManager it is assigned to by default; `None` if optional.
    pub sm: Option<u8>,
    /// Content cannot be changed.
    pub fixed: bool,
    /// Must be assigned.
    pub mandatory: bool,
    /// Entries.
    pub entries: Vec<PdoEntry>,
    /// Total size, bits.
    pub bit_len: u32,
}

/// Mailbox protocols a device supports.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct MailboxSupport {
    /// CANopen over EtherCAT.
    pub coe: bool,
    /// File access over EtherCAT.
    pub foe: bool,
    /// Ethernet over EtherCAT.
    pub eoe: bool,
    /// Servo drive profile over EtherCAT.
    pub soe: bool,
    /// Vendor-specific over EtherCAT.
    pub voe: bool,
    /// ADS over EtherCAT.
    pub aoe: bool,
    /// CoE SDO Information service.
    pub coe_sdo_info: bool,
    /// CoE complete access.
    pub coe_complete_access: bool,
}

impl MailboxSupport {
    /// Whether `protocol` (`"CoE"`, `"FoE"`, `"EoE"`, `"SoE"`, `"VoE"`, `"AoE"`, any case) is
    /// supported.
    pub fn supports(&self, protocol: &str) -> bool {
        match protocol.to_ascii_lowercase().as_str() {
            "coe" => self.coe,
            "foe" => self.foe,
            "eoe" => self.eoe,
            "soe" => self.soe,
            "voe" => self.voe,
            "aoe" => self.aoe,
            _ => false,
        }
    }

    /// Supported protocol names.
    pub fn names(&self) -> Vec<String> {
        [
            ("CoE", self.coe),
            ("FoE", self.foe),
            ("EoE", self.eoe),
            ("SoE", self.soe),
            ("VoE", self.voe),
            ("AoE", self.aoe),
        ]
        .iter()
        .filter(|(_, on)| *on)
        .map(|(n, _)| (*n).to_owned())
        .collect()
    }
}

/// One device type from an ESI file.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct DeviceDescription {
    /// Vendor id.
    pub vendor_id: u32,
    /// Vendor name.
    pub vendor_name: String,
    /// Product code.
    pub product_code: u32,
    /// Revision number.
    pub revision: u32,
    /// Order code / type name (`<Type>` text).
    pub type_name: String,
    /// Display name (first `<Name>`).
    pub name: String,
    /// SyncManagers.
    pub sync_managers: Vec<SmDescription>,
    /// Output PDOs.
    pub rx_pdos: Vec<PdoDescription>,
    /// Input PDOs.
    pub tx_pdos: Vec<PdoDescription>,
    /// Mailbox protocols, if the device has a mailbox.
    pub mailbox: Option<MailboxSupport>,
    /// Distributed clocks supported.
    pub dc: bool,
    /// Outputs of the default PDO assignment, bits.
    pub default_outputs_bits: u32,
    /// Inputs of the default PDO assignment, bits.
    pub default_inputs_bits: u32,
}

impl DeviceDescription {
    /// Default length of the SyncManager of `kind`, bytes.
    pub fn sm_size(&self, kind: &str) -> Option<u16> {
        self.sync_managers.iter().find(|s| s.kind.eq_ignore_ascii_case(kind)).and_then(|s| s.default_size)
    }
}

/// The devices of one ESI file.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct EsiFile {
    /// Vendor id.
    pub vendor_id: u32,
    /// Vendor name.
    pub vendor_name: String,
    /// Devices.
    pub devices: Vec<DeviceDescription>,
}

fn pdos(dev: roxmltree::Node<'_, '_>, tag: &str) -> Vec<PdoDescription> {
    children(dev, tag)
        .map(|p| {
            let entries: Vec<PdoEntry> = children(p, "Entry")
                .map(|e| PdoEntry {
                    index: child_num(e, "Index").unwrap_or(0) as u16,
                    subindex: child_num(e, "SubIndex").unwrap_or(0) as u8,
                    bit_len: child_num(e, "BitLen").unwrap_or(0) as u16,
                    name: text(e, "Name").unwrap_or_default().to_owned(),
                    data_type: text(e, "DataType").map(str::to_owned),
                })
                .collect();
            PdoDescription {
                index: child_num(p, "Index").unwrap_or(0) as u16,
                name: text(p, "Name").unwrap_or_default().to_owned(),
                sm: attr_num(p, "Sm").map(|v| v as u8),
                fixed: attr_bool(p, "Fixed"),
                mandatory: attr_bool(p, "Mandatory"),
                bit_len: entries.iter().map(|e| e.bit_len as u32).sum(),
                entries,
            }
        })
        .collect()
}

/// Parse an ESI (`<EtherCATInfo>`) document.
pub fn parse_esi(xml: &str) -> Result<EsiFile, DeviceError> {
    let doc = document(xml, "EtherCATInfo")?;
    let root = doc.root_element();
    let vendor = child(root, "Vendor");
    let vendor_id = vendor.and_then(|v| child_num(v, "Id")).unwrap_or(0) as u32;
    let vendor_name = vendor.and_then(|v| text(v, "Name")).unwrap_or_default().to_owned();
    let devices_node = child(root, "Descriptions")
        .and_then(|d| child(d, "Devices"))
        .ok_or_else(|| DeviceError::Format("no <Descriptions><Devices>".into()))?;
    let mut devices = Vec::new();
    for dev in children(devices_node, "Device") {
        let ty = child(dev, "Type").ok_or_else(|| DeviceError::Format("a <Device> has no <Type>".into()))?;
        let sync_managers = children(dev, "Sm")
            .enumerate()
            .map(|(i, s)| SmDescription {
                index: i as u8,
                kind: s.text().map(str::trim).unwrap_or_default().to_owned(),
                start: attr_num(s, "StartAddress").map(|v| v as u16),
                default_size: attr_num(s, "DefaultSize").map(|v| v as u16),
                min_size: attr_num(s, "MinSize").map(|v| v as u16),
                max_size: attr_num(s, "MaxSize").map(|v| v as u16),
                control: attr_num(s, "ControlByte").map(|v| v as u8),
                enable: attr_bool(s, "Enable"),
            })
            .collect();
        let rx_pdos = pdos(dev, "RxPdo");
        let tx_pdos = pdos(dev, "TxPdo");
        let mailbox = child(dev, "Mailbox").map(|m| {
            let coe = child(m, "CoE");
            MailboxSupport {
                coe: coe.is_some(),
                foe: child(m, "FoE").is_some(),
                eoe: child(m, "EoE").is_some(),
                soe: child(m, "SoE").is_some(),
                voe: child(m, "VoE").is_some(),
                aoe: child(m, "AoE").is_some(),
                coe_sdo_info: coe.is_some_and(|c| attr_bool(c, "SdoInfo")),
                coe_complete_access: coe.is_some_and(|c| attr_bool(c, "CompleteAccess")),
            }
        });
        let assigned = |v: &[PdoDescription]| v.iter().filter(|p| p.sm.is_some()).map(|p| p.bit_len).sum::<u32>();
        devices.push(DeviceDescription {
            vendor_id,
            vendor_name: vendor_name.clone(),
            product_code: attr_num(ty, "ProductCode").unwrap_or(0) as u32,
            revision: attr_num(ty, "RevisionNo").unwrap_or(0) as u32,
            type_name: ty.text().map(str::trim).unwrap_or_default().to_owned(),
            name: text(dev, "Name").unwrap_or_default().to_owned(),
            sync_managers,
            default_outputs_bits: assigned(&rx_pdos),
            default_inputs_bits: assigned(&tx_pdos),
            rx_pdos,
            tx_pdos,
            mailbox,
            dc: child(dev, "Dc").is_some(),
        });
    }
    Ok(EsiFile { vendor_id, vendor_name, devices })
}
