//! Names of common ESC registers (ETG.1000.4 / ESC datasheets, e.g. Beckhoff ET1100,
//! Microchip LAN9252). Only the widely implemented registers are named; vendor-specific
//! and optional ones fall back to a generic area name.

/// A named ESC register or register area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct RegisterInfo {
    /// First address of the register.
    pub start: u16,
    /// Register length in bytes.
    pub len: u16,
    /// Short name.
    pub name: &'static str,
    /// Area this register belongs to.
    pub area: &'static str,
}

const REGS: &[(u16, u16, &str, &str)] = &[
    (0x0000, 1, "Type", "ESC information"),
    (0x0001, 1, "Revision", "ESC information"),
    (0x0002, 2, "Build", "ESC information"),
    (0x0004, 1, "FMMUs supported", "ESC information"),
    (0x0005, 1, "SyncManagers supported", "ESC information"),
    (0x0006, 1, "RAM size", "ESC information"),
    (0x0007, 1, "Port descriptor", "ESC information"),
    (0x0008, 2, "ESC features supported", "ESC information"),
    (0x0010, 2, "Configured station address", "Station address"),
    (0x0012, 2, "Configured station alias", "Station address"),
    (0x0020, 1, "Register write enable", "Write protection"),
    (0x0021, 1, "Register write protection", "Write protection"),
    (0x0030, 1, "ESC write enable", "Write protection"),
    (0x0031, 1, "ESC write protection", "Write protection"),
    (0x0040, 1, "ESC reset ECAT", "Reset"),
    (0x0041, 1, "ESC reset PDI", "Reset"),
    (0x0100, 4, "DL control", "Data link layer"),
    (0x0108, 2, "Physical read/write offset", "Data link layer"),
    (0x0110, 2, "DL status", "Data link layer"),
    (0x0120, 2, "AL control", "Application layer"),
    (0x0130, 2, "AL status", "Application layer"),
    (0x0134, 2, "AL status code", "Application layer"),
    (0x0138, 1, "RUN LED override", "Application layer"),
    (0x0139, 1, "ERR LED override", "Application layer"),
    (0x0140, 1, "PDI control", "PDI"),
    (0x0141, 1, "ESC configuration", "PDI"),
    (0x0150, 4, "PDI configuration", "PDI"),
    (0x0200, 2, "ECAT event mask", "Interrupts"),
    (0x0204, 4, "AL event mask", "Interrupts"),
    (0x0210, 2, "ECAT event request", "Interrupts"),
    (0x0220, 4, "AL event request", "Interrupts"),
    (0x0300, 8, "RX error counters", "Error counters"),
    (0x0308, 4, "Forwarded RX error counters", "Error counters"),
    (0x030C, 1, "ECAT processing unit error counter", "Error counters"),
    (0x030D, 1, "PDI error counter", "Error counters"),
    (0x0310, 4, "Lost link counters", "Error counters"),
    (0x0400, 2, "Watchdog divider", "Watchdogs"),
    (0x0410, 2, "Watchdog time PDI", "Watchdogs"),
    (0x0420, 2, "Watchdog time process data", "Watchdogs"),
    (0x0440, 2, "Watchdog status process data", "Watchdogs"),
    (0x0442, 1, "Watchdog counter process data", "Watchdogs"),
    (0x0443, 1, "Watchdog counter PDI", "Watchdogs"),
    (0x0500, 2, "SII EEPROM configuration", "SII EEPROM"),
    (0x0502, 2, "SII EEPROM control/status", "SII EEPROM"),
    (0x0504, 4, "SII EEPROM address", "SII EEPROM"),
    (0x0508, 8, "SII EEPROM data", "SII EEPROM"),
    (0x0510, 2, "MII management control/status", "MII management"),
    (0x0900, 4, "DC receive time port 0", "Distributed clocks"),
    (0x0904, 4, "DC receive time port 1", "Distributed clocks"),
    (0x0908, 4, "DC receive time port 2", "Distributed clocks"),
    (0x090C, 4, "DC receive time port 3", "Distributed clocks"),
    (0x0910, 8, "DC system time", "Distributed clocks"),
    (0x0918, 8, "DC receive time ECAT processing unit", "Distributed clocks"),
    (0x0920, 8, "DC system time offset", "Distributed clocks"),
    (0x0928, 4, "DC system time delay", "Distributed clocks"),
    (0x092C, 4, "DC system time difference", "Distributed clocks"),
    (0x0930, 2, "DC speed counter start", "Distributed clocks"),
    (0x0934, 1, "DC system time difference filter depth", "Distributed clocks"),
    (0x0980, 1, "DC cyclic unit control", "Distributed clocks"),
    (0x0981, 1, "DC activation", "Distributed clocks"),
    (0x098E, 1, "DC SYNC0 status", "Distributed clocks"),
    (0x0990, 8, "DC start time cyclic operation", "Distributed clocks"),
    (0x09A0, 4, "DC SYNC0 cycle time", "Distributed clocks"),
    (0x09A4, 4, "DC SYNC1 cycle time", "Distributed clocks"),
];

/// Describe the register at physical address `ado`.
///
/// FMMU (`0x0600..0x0700`) and SyncManager (`0x0800..0x0880`) channels are named by
/// channel number; addresses `>= 0x1000` are reported as process / mailbox RAM.
pub fn register_info(ado: u16) -> Option<RegisterInfo> {
    if let Some(&(start, len, name, area)) = REGS.iter().find(|(s, l, _, _)| ado >= *s && ado < s + l) {
        return Some(RegisterInfo { start, len, name, area });
    }
    match ado {
        0x0600..=0x06FF => Some(RegisterInfo {
            start: ado & !0x0F,
            len: 16,
            name: FMMU_NAMES[((ado - 0x0600) / 16) as usize],
            area: "FMMU",
        }),
        0x0800..=0x087F => Some(RegisterInfo {
            start: ado & !0x07,
            len: 8,
            name: SM_NAMES[((ado - 0x0800) / 8) as usize],
            area: "SyncManager",
        }),
        0x1000..=0xFFFF => Some(RegisterInfo { start: 0x1000, len: 0, name: "Process/mailbox RAM", area: "RAM" }),
        _ => None,
    }
}

const FMMU_NAMES: [&str; 16] = [
    "FMMU 0", "FMMU 1", "FMMU 2", "FMMU 3", "FMMU 4", "FMMU 5", "FMMU 6", "FMMU 7", "FMMU 8", "FMMU 9", "FMMU 10",
    "FMMU 11", "FMMU 12", "FMMU 13", "FMMU 14", "FMMU 15",
];
const SM_NAMES: [&str; 16] = [
    "SyncManager 0",
    "SyncManager 1",
    "SyncManager 2",
    "SyncManager 3",
    "SyncManager 4",
    "SyncManager 5",
    "SyncManager 6",
    "SyncManager 7",
    "SyncManager 8",
    "SyncManager 9",
    "SyncManager 10",
    "SyncManager 11",
    "SyncManager 12",
    "SyncManager 13",
    "SyncManager 14",
    "SyncManager 15",
];
