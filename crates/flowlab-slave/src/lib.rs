//! An emulated EtherCAT slave: an ESC register model with FoE, CoE and EoE behind its mailbox,
//! for testing a master, a capture and the analysis on an ordinary Ethernet link when no
//! EtherCAT hardware is at hand.
//!
//! [`EmulatedSlave::handle`] takes an Ethernet frame as it arrived and returns the frame the
//! slave sends back, as the last slave of a line returns it through port 0: datagrams applied
//! to its memory, working counters and auto-increment addresses updated, and bit 1 of the
//! source MAC set, as an ESC sets it. A frame that already has that bit set has passed an
//! ESC: it is a returning frame and is ignored, unless the slave sits behind a real ESC
//! ([`SlaveConfig::behind_esc`]), whose downstream port passes the master's frames on with
//! the bit set.
//!
//! What the model does:
//!
//! * registers: type, FMMU and SyncManager counts, RAM size, station address, DL status (one
//!   port with link), AL control, status and status code, the SII interface (0x0500–0x050F)
//!   over the EEPROM image, FMMUs (0x0600), SyncManagers (0x0800); read-only registers ignore
//!   writes;
//! * commands: position, configured-address and broadcast reads, writes and read-writes,
//!   ARMW/FRMW, and logical LRD/LWR/LRW through its FMMUs, each with the ESC's working
//!   counter rules;
//! * the AL state machine: INIT, PRE-OP (after the mailbox SyncManagers are configured as the
//!   EEPROM says), SAFE-OP (after the process data SyncManagers are), OP, with error codes and
//!   the error acknowledge;
//! * mailbox SyncManagers with the ESC's one-buffer rules: a write to a full "mailbox out" and
//!   a read of an empty "mailbox in" get no working counter; the message is the slave's when
//!   the last byte is written, and "mailbox in" is free again when its last byte is read; the
//!   repeat request puts the last message back;
//! * the application: [`flowlab_mailbox::responder`] (FoE, CoE SDO, EoE) takes each message
//!   at once; CoE also serves the objects a master reads to map the process data (0x1C12,
//!   0x1C13, 0x1600, 0x1A00) and the identity (0x1018); the inputs echo the outputs.
//!
//! What it does not: distributed clocks (it reports none), the buffered (three-buffer) mode
//! of the process data SyncManagers (they are plain memory), the watchdogs, interrupts and
//! events, more than one port, and the forwarding delay of an ESC: frames come back when the
//! host has processed them.
//!
//! With the feature `wire`, `wire::serve` answers frames on a network interface. It sends
//! frames, so it needs the interface name and an explicit acknowledgement that the network is
//! an isolated lab network, as the lab master does.
//!
//! ```
//! use ecat_wire::{Command, DatagramSpec, FrameBuilder, MacAddr, parse_frame};
//! use flowlab_slave::{EmulatedSlave, SlaveConfig};
//!
//! let mut slave = EmulatedSlave::new(SlaveConfig::default()).unwrap();
//! let mut b = FrameBuilder::new(MacAddr([0xff; 6]), MacAddr([0x10; 6]));
//! b.push(DatagramSpec::new(Command::Brd, 0, 0x0000_0000, &[0])).unwrap();
//! let back = slave.handle(&b.finish(), 0).unwrap();
//! let f = parse_frame(&back).unwrap().unwrap();
//! assert_eq!(f.pdu.datagrams().next().unwrap().unwrap().wkc, 1); // one slave counted
//! assert_eq!(back[6], 0x12); // the ESC's source MAC bit
//! ```

#[cfg(any(feature = "wire", test))]
mod echo;
mod esc;
mod sii;
#[cfg(test)]
mod tests;
#[cfg(feature = "wire")]
pub mod wire;

use ecat_wire::AlState;
use flowlab_mailbox::responder::{Responder, ResponderConfig, ResponderLog};

pub use flowlab_core::EMULATED_ESC_NAME;

/// Setting: the size of each mailbox, bytes (u16, 32–1024). A write takes effect when the
/// master next takes the slave to INIT: the EEPROM and the SyncManager layout change with it,
/// so the master reads the new size on its next start.
pub const SETTING_MAILBOX_BYTES: (u16, u8) = (0x2001, 0);
/// Setting: send the upload over EoE after PRE-OP (u8, 0 or 1); takes effect at the next INIT.
pub const SETTING_EOE_SEND: (u16, u8) = (0x2002, 0);

/// First byte of process RAM.
const RAM_START: u16 = 0x1000;
/// Process RAM of the model, bytes.
const RAM_BYTES: u16 = 0x2000;

/// Settings of an [`EmulatedSlave`].
#[derive(Debug, Clone)]
pub struct SlaveConfig {
    /// Vendor ID in the EEPROM and in object 0x1018. 0: none assigned.
    pub vendor_id: u32,
    /// Product code.
    pub product_code: u32,
    /// Revision.
    pub revision: u32,
    /// Serial number.
    pub serial: u32,
    /// Bytes of each mailbox ("mailbox out" and "mailbox in"), 32–1024.
    pub mailbox_bytes: u16,
    /// Process data outputs (master → slave), bytes.
    pub output_bytes: u16,
    /// Process data inputs (slave → master), bytes.
    pub input_bytes: u16,
    /// With an EoE send: how long after entering PRE-OP the first frame is offered, ns. A
    /// master configures the slave over CoE right after PRE-OP; frames in "mailbox in" before
    /// that would answer its requests.
    pub eoe_start_after_ns: u64,
    /// The application behind the mailbox. Its `objects` are extended by the mapping and
    /// identity objects.
    pub responder: ResponderConfig,
    /// The slave sits behind a real ESC (cabled to its downstream port): frames arrive with
    /// bit 1 of the source MAC already set by that ESC and are answered. Without it, such
    /// frames are taken for returning frames and ignored.
    pub behind_esc: bool,
}

impl Default for SlaveConfig {
    fn default() -> Self {
        Self {
            vendor_id: 0,
            product_code: 0x0000_0001,
            revision: 0x0000_0001,
            serial: 0,
            mailbox_bytes: 128,
            output_bytes: 4,
            input_bytes: 4,
            eoe_start_after_ns: 1_000_000_000,
            responder: ResponderConfig::default(),
            behind_esc: false,
        }
    }
}

impl SlaveConfig {
    fn identity(&self) -> sii::Identity<'static> {
        sii::Identity {
            name: EMULATED_ESC_NAME,
            vendor_id: self.vendor_id,
            product_code: self.product_code,
            revision: self.revision,
            serial: self.serial,
        }
    }
}

/// Why a configuration cannot be emulated.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SlaveError {
    /// The configuration or a setting is invalid.
    #[error("{0}")]
    Invalid(String),
    /// A safety check refused to start.
    #[error("{0}")]
    Refused(String),
    /// The network interface failed.
    #[error("{0}")]
    Io(String),
}

/// Where the SyncManager buffers are in process RAM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    /// "mailbox out" (SM0), start address.
    pub mbx_out: u16,
    /// "mailbox in" (SM1), start address.
    pub mbx_in: u16,
    /// Bytes of each mailbox.
    pub mbx_len: u16,
    /// Outputs (SM2), start address.
    pub outputs: u16,
    /// Bytes of outputs.
    pub output_len: u16,
    /// Inputs (SM3), start address.
    pub inputs: u16,
    /// Bytes of inputs.
    pub input_len: u16,
    /// First address after process RAM.
    pub ram_end: usize,
}

impl Layout {
    fn of(c: &SlaveConfig) -> Result<Self, SlaveError> {
        if !(32..=1024).contains(&c.mailbox_bytes) {
            return Err(SlaveError::Invalid(format!("mailbox of {} bytes: must be 32–1024", c.mailbox_bytes)));
        }
        for (what, n) in [("outputs", c.output_bytes), ("inputs", c.input_bytes)] {
            if n > sii::MAX_PROCESS_DATA {
                return Err(SlaveError::Invalid(format!("{n} bytes of {what}: at most {}", sii::MAX_PROCESS_DATA)));
            }
        }
        let m = c.mailbox_bytes;
        let outputs = RAM_START + 2 * m;
        let inputs = outputs + c.output_bytes.next_multiple_of(8);
        let ram_end = usize::from(RAM_START) + usize::from(RAM_BYTES);
        if usize::from(inputs) + usize::from(c.input_bytes) > ram_end {
            return Err(SlaveError::Invalid("mailboxes and process data do not fit the process RAM".into()));
        }
        Ok(Self {
            mbx_out: RAM_START,
            mbx_in: RAM_START + m,
            mbx_len: m,
            outputs,
            output_len: c.output_bytes,
            inputs,
            input_len: c.input_bytes,
            ram_end,
        })
    }
}

/// Counters of what the slave did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SlaveStats {
    /// EtherCAT frames answered.
    pub frames: u64,
    /// Datagrams processed.
    pub datagrams: u64,
    /// Messages the master wrote into "mailbox out".
    pub messages_to_slave: u64,
    /// Messages the master read out of "mailbox in" (including repeats).
    pub messages_from_slave: u64,
    /// Repeat requests served.
    pub repeats: u64,
    /// AL state changes.
    pub al_changes: u64,
}

/// The emulated slave (see the [crate docs](crate)).
pub struct EmulatedSlave {
    esc: esc::Esc,
    layout: Layout,
    behind_esc: bool,
    /// The configuration the slave runs with now (its responder part is not kept).
    shape: SlaveConfig,
}

impl std::fmt::Debug for EmulatedSlave {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmulatedSlave").field("layout", &self.layout).finish_non_exhaustive()
    }
}

/// EtherType of EtherCAT and of a VLAN tag.
const ETHERTYPE_ETHERCAT: u16 = 0x88A4;
const ETHERTYPE_VLAN: u16 = 0x8100;

impl EmulatedSlave {
    /// A slave in INIT with station address 0.
    pub fn new(cfg: SlaveConfig) -> Result<Self, SlaveError> {
        let layout = Layout::of(&cfg)?;
        let mut rc = cfg.responder;
        let shape = SlaveConfig { responder: ResponderConfig::default(), ..cfg };
        let id = shape.identity();
        rc.objects.extend(sii::objects(&id, &layout));
        rc.settings.insert(SETTING_MAILBOX_BYTES, shape.mailbox_bytes.to_le_bytes().to_vec());
        rc.settings.insert(SETTING_EOE_SEND, vec![u8::from(rc.eoe_send)]);
        let len = usize::from(layout.mbx_len);
        let app = Responder::new(rc, len, len);
        let esc = esc::Esc::new(sii::image(&id, &layout), layout, app, shape.eoe_start_after_ns);
        Ok(Self { esc, layout, behind_esc: shape.behind_esc, shape })
    }

    /// In INIT, apply the settings the master wrote ([`SETTING_MAILBOX_BYTES`],
    /// [`SETTING_EOE_SEND`]). A mailbox size that does not fit is put back to the current one.
    fn apply_settings(&mut self) {
        let app = self.esc.app();
        let mailbox = app.setting(SETTING_MAILBOX_BYTES.0, SETTING_MAILBOX_BYTES.1).and_then(|v| v.get(..2));
        let mailbox = mailbox.map_or(self.shape.mailbox_bytes, |v| u16::from_le_bytes([v[0], v[1]]));
        let eoe_send = app.setting(SETTING_EOE_SEND.0, SETTING_EOE_SEND.1).and_then(|v| v.first()).map(|b| *b != 0);
        let eoe_send = eoe_send.unwrap_or(app.config().eoe_send);
        if mailbox == self.shape.mailbox_bytes && eoe_send == app.config().eoe_send {
            return;
        }
        let shape = SlaveConfig { mailbox_bytes: mailbox, ..self.shape.clone() };
        let layout = match Layout::of(&shape) {
            Ok(l) => l,
            Err(_) => {
                let current = self.shape.mailbox_bytes.to_le_bytes().to_vec();
                self.esc.app_mut().put_setting(SETTING_MAILBOX_BYTES, current);
                return;
            }
        };
        let eeprom = sii::image(&shape.identity(), &layout);
        self.esc.reconfigure(eeprom, layout, eoe_send);
        self.layout = layout;
        self.shape = shape;
    }

    /// Process one Ethernet frame that arrived at `now_ns` (any monotonic origin). Returns the
    /// frame to send back, or `None` if it is not an EtherCAT frame for this slave: another
    /// EtherType, a malformed one, or one that already passed an ESC, unless the slave is
    /// [behind an ESC](SlaveConfig::behind_esc).
    pub fn handle(&mut self, frame: &[u8], now_ns: u64) -> Option<Vec<u8>> {
        if frame.len() < 16 {
            return None;
        }
        let mut at = 12;
        let mut ethertype = u16::from_be_bytes([frame[12], frame[13]]);
        if ethertype == ETHERTYPE_VLAN && frame.len() >= 20 {
            at = 16;
            ethertype = u16::from_be_bytes([frame[16], frame[17]]);
        }
        if ethertype != ETHERTYPE_ETHERCAT || (frame[6] & 0x02 != 0 && !self.behind_esc) {
            return None;
        }
        let mut f = frame.to_vec();
        if !self.esc.process(&mut f, at + 2, now_ns) {
            return None;
        }
        if self.esc.al_state() == AlState::Init {
            self.apply_settings();
        }
        f[6] |= 0x02;
        self.esc.stats.frames += 1;
        Some(f)
    }

    /// Where the buffers are.
    pub fn layout(&self) -> Layout {
        self.layout
    }

    /// The AL state and, if the error flag is set, the AL status code.
    pub fn al_state(&self) -> (AlState, Option<u16>) {
        (self.esc.al_state(), self.esc.al_error())
    }

    /// The station address the master assigned (0 before).
    pub fn station(&self) -> u16 {
        self.esc.station()
    }

    /// What the application received.
    pub fn log(&self) -> ResponderLog {
        self.esc.app().log()
    }

    /// What the application received since the last call; it is forgotten here
    /// ([`Responder::drain_log`]).
    pub fn drain_log(&mut self) -> ResponderLog {
        self.esc.app_mut().drain_log()
    }

    /// Counters.
    pub fn stats(&self) -> SlaveStats {
        self.esc.stats.clone()
    }

    /// The application (its settings and log).
    pub fn responder(&self) -> &Responder {
        self.esc.app()
    }
}
