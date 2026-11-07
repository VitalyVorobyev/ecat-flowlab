//! Datagram commands (NOP … FRMW) and how each one interprets its address field.

/// EtherCAT datagram command (ETG.1000.4, table "EtherCAT command types").
///
/// Serializes as its mnemonic string (`"LRW"`, or `"CMD15"` for unknown codes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Command {
    /// No operation.
    Nop,
    /// Auto-increment physical read.
    Aprd,
    /// Auto-increment physical write.
    Apwr,
    /// Auto-increment physical read/write.
    Aprw,
    /// Configured-address physical read.
    Fprd,
    /// Configured-address physical write.
    Fpwr,
    /// Configured-address physical read/write.
    Fprw,
    /// Broadcast read.
    Brd,
    /// Broadcast write.
    Bwr,
    /// Broadcast read/write.
    Brw,
    /// Logical memory read.
    Lrd,
    /// Logical memory write.
    Lwr,
    /// Logical memory read/write.
    Lrw,
    /// Auto-increment physical read, multiple write.
    Armw,
    /// Configured-address physical read, multiple write.
    Frmw,
    /// A command code this crate does not know (reserved values 15..=255).
    Unknown(u8),
}

/// How a datagram's 32-bit address field is interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum AddressMode {
    /// No addressing (NOP).
    None,
    /// Auto-increment position (ADP) + physical offset (ADO).
    Position,
    /// Configured station address (ADP) + physical offset (ADO).
    Fixed,
    /// Every slave; ADP is incremented, ADO is the physical offset.
    Broadcast,
    /// 32-bit logical address mapped by FMMUs.
    Logical,
}

impl Command {
    /// Decode a command byte.
    pub const fn from_code(code: u8) -> Self {
        match code {
            0 => Self::Nop,
            1 => Self::Aprd,
            2 => Self::Apwr,
            3 => Self::Aprw,
            4 => Self::Fprd,
            5 => Self::Fpwr,
            6 => Self::Fprw,
            7 => Self::Brd,
            8 => Self::Bwr,
            9 => Self::Brw,
            10 => Self::Lrd,
            11 => Self::Lwr,
            12 => Self::Lrw,
            13 => Self::Armw,
            14 => Self::Frmw,
            other => Self::Unknown(other),
        }
    }

    /// The command byte.
    pub const fn code(self) -> u8 {
        match self {
            Self::Nop => 0,
            Self::Aprd => 1,
            Self::Apwr => 2,
            Self::Aprw => 3,
            Self::Fprd => 4,
            Self::Fpwr => 5,
            Self::Fprw => 6,
            Self::Brd => 7,
            Self::Bwr => 8,
            Self::Brw => 9,
            Self::Lrd => 10,
            Self::Lwr => 11,
            Self::Lrw => 12,
            Self::Armw => 13,
            Self::Frmw => 14,
            Self::Unknown(c) => c,
        }
    }

    /// Mnemonic as used in ETG documents and Wireshark (`"LRW"`, `"FPRD"`, ...).
    pub const fn mnemonic(self) -> &'static str {
        match self {
            Self::Nop => "NOP",
            Self::Aprd => "APRD",
            Self::Apwr => "APWR",
            Self::Aprw => "APRW",
            Self::Fprd => "FPRD",
            Self::Fpwr => "FPWR",
            Self::Fprw => "FPRW",
            Self::Brd => "BRD",
            Self::Bwr => "BWR",
            Self::Brw => "BRW",
            Self::Lrd => "LRD",
            Self::Lwr => "LWR",
            Self::Lrw => "LRW",
            Self::Armw => "ARMW",
            Self::Frmw => "FRMW",
            Self::Unknown(_) => "UNKNOWN",
        }
    }

    /// Addressing mode implied by the command.
    pub const fn address_mode(self) -> AddressMode {
        match self {
            Self::Nop | Self::Unknown(_) => AddressMode::None,
            Self::Aprd | Self::Apwr | Self::Aprw | Self::Armw => AddressMode::Position,
            Self::Fprd | Self::Fpwr | Self::Fprw | Self::Frmw => AddressMode::Fixed,
            Self::Brd | Self::Bwr | Self::Brw => AddressMode::Broadcast,
            Self::Lrd | Self::Lwr | Self::Lrw => AddressMode::Logical,
        }
    }

    /// Whether every slave increments the ADP field as the datagram passes: true for
    /// position-addressed (`APxx`, `ARMW`) and broadcast (`Bxx`) commands.
    pub const fn increments_adp(self) -> bool {
        matches!(self.address_mode(), AddressMode::Position | AddressMode::Broadcast)
    }

    /// The raw 32-bit address field after the datagram passed `slaves` slaves: ADP grows by one
    /// per slave for position and broadcast commands; other addresses are unchanged.
    pub const fn address_after(self, address: u32, slaves: u16) -> u32 {
        if self.increments_adp() {
            (address & 0xFFFF_0000) | ((address as u16).wrapping_add(slaves) as u32)
        } else {
            address
        }
    }

    /// The part of the address field that is the same on the way out and on the way back:
    /// ADP is cleared for commands whose ADP the slaves increment.
    pub const fn stable_address(self, address: u32) -> u32 {
        if self.increments_adp() { address & 0xFFFF_0000 } else { address }
    }

    /// Whether the command reads slave memory (the returning frame carries slave data).
    pub const fn reads(self) -> bool {
        matches!(
            self,
            Self::Aprd
                | Self::Aprw
                | Self::Fprd
                | Self::Fprw
                | Self::Brd
                | Self::Brw
                | Self::Lrd
                | Self::Lrw
                | Self::Armw
                | Self::Frmw
        )
    }

    /// Whether the command writes slave memory.
    pub const fn writes(self) -> bool {
        matches!(
            self,
            Self::Apwr
                | Self::Aprw
                | Self::Fpwr
                | Self::Fprw
                | Self::Bwr
                | Self::Brw
                | Self::Lwr
                | Self::Lrw
                | Self::Armw
                | Self::Frmw
        )
    }

    /// All defined commands, in code order.
    pub const ALL: [Command; 15] = [
        Self::Nop,
        Self::Aprd,
        Self::Apwr,
        Self::Aprw,
        Self::Fprd,
        Self::Fpwr,
        Self::Fprw,
        Self::Brd,
        Self::Bwr,
        Self::Brw,
        Self::Lrd,
        Self::Lwr,
        Self::Lrw,
        Self::Armw,
        Self::Frmw,
    ];
}

impl std::fmt::Display for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(c) => write!(f, "CMD{c}"),
            other => f.write_str(other.mnemonic()),
        }
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for Command {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}
