/// Application-layer state of an EtherCAT slave.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum AlState {
    /// `1`: Init.
    Init,
    /// `2`: Pre-Operational.
    PreOp,
    /// `3`: Bootstrap.
    Boot,
    /// `4`: Safe-Operational.
    SafeOp,
    /// `8`: Operational.
    Op,
    /// Any other 4-bit value.
    Other(u8),
}

impl AlState {
    /// Decode the low 4 bits of AL Control / AL Status.
    pub const fn from_code(code: u8) -> Self {
        match code & 0x0F {
            1 => Self::Init,
            2 => Self::PreOp,
            3 => Self::Boot,
            4 => Self::SafeOp,
            8 => Self::Op,
            other => Self::Other(other),
        }
    }

    /// The 4-bit code.
    pub const fn code(self) -> u8 {
        match self {
            Self::Init => 1,
            Self::PreOp => 2,
            Self::Boot => 3,
            Self::SafeOp => 4,
            Self::Op => 8,
            Self::Other(c) => c,
        }
    }

    /// Short name (`"INIT"`, `"PREOP"`, `"BOOT"`, `"SAFEOP"`, `"OP"`).
    pub const fn name(self) -> &'static str {
        match self {
            Self::Init => "INIT",
            Self::PreOp => "PREOP",
            Self::Boot => "BOOT",
            Self::SafeOp => "SAFEOP",
            Self::Op => "OP",
            Self::Other(_) => "?",
        }
    }
}

/// AL Control (0x0120) or AL Status (0x0130) low byte: state plus bit 4
/// (error acknowledge in control, error indication in status).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AlStatus {
    /// State.
    pub state: AlState,
    /// Bit 4: error indication (status) or error acknowledge (control).
    pub error: bool,
}

impl AlStatus {
    /// Decode the low byte.
    pub const fn from_byte(b: u8) -> Self {
        Self { state: AlState::from_code(b), error: b & 0x10 != 0 }
    }

    /// Encode to the low byte.
    pub const fn to_byte(self) -> u8 {
        self.state.code() | if self.error { 0x10 } else { 0 }
    }
}
