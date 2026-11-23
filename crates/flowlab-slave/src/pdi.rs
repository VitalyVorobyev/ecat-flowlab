//! FlowLab's slave application on a real ESC, through its process data interface (PDI).
//!
//! The emulated slave ([`crate::EmulatedSlave`]) is an ESC model and its application in one.
//! [`PdiSlave`] is only the application: a real ESC (a LAN9252 over SPI, [`SlaveAccess`])
//! answers the master's frames, and this loop does what a slave stack does on the PDI side,
//! with the same [`Responder`] behind the mailbox (FoE, CoE, EoE, FlowLab VoE). It takes the
//! place of the ESC's own application, which must be stopped first.
//!
//! Each [`step`](PdiSlave::step) polls once:
//!
//! * AL Event Request (0x0220) bit 0: the master wrote AL Control. Reading AL Control
//!   (0x0120) acknowledges it. INIT is always accepted; PRE-OP from INIT when SyncManager 0 is
//!   a mailbox the master writes and SyncManager 1 one it reads, both enabled, in process RAM,
//!   apart, and between 34 B and [`PdiSlaveConfig::max_mailbox_bytes`] long. Their sizes are
//!   whatever the master configured: the EEPROM's, or others (`flowlab master
//!   --mailbox-bytes`). Otherwise AL Status gets the error flag and code 0x0016. SAFE-OP and OP
//!   are refused with 0x0011 (no process data), BOOT with 0x0013. With the error flag set, a
//!   request without the acknowledge bit is ignored, as on an ESC.
//! * Entering PRE-OP clears bit 0 of both mailbox SyncManagers' PDI Control (0x0807, 0x080F),
//!   which a stack sets outside PRE-OP: while it is set, the ESC refuses the master's writes.
//!   INIT sets it again.
//! * In PRE-OP, "mailbox out" full: the message is read, and the buffer's last DWORD (reading
//!   its last byte frees it for the master), and handed to the responder.
//! * "Mailbox in" empty and an answer waits: the answer is written, then the buffer's last
//!   DWORD (writing its last byte hands it to the master).
//! * Both come from the SyncManager bits of AL Event Request (bit 8: "mailbox out" written,
//!   bit 9: "mailbox in" read), when the master enabled them (SyncManager control bit 5);
//!   otherwise, and every 64th poll as a fallback, from the SyncManager status (0x0805,
//!   0x080D). One poll without traffic is then one register read.
//! * EoE: with [`PdiSlaveConfig::answer_set_ip`], Set IP Parameter requests are answered with
//!   success and kept for the caller ([`PdiSlave::take_set_ip`]); received frames can be taken
//!   ([`PdiSlave::take_eoe_frames`]) and frames sent ([`PdiSlave::send_eoe_frame`]), to bridge
//!   EoE to a network interface.
//!
//! What it does not do: process data, the SyncManager repeat request (the master's own
//! resend, a new message with a new counter, still works), interrupts (it polls), and the
//! watchdogs. It writes AL Status, AL Status Code, the PDI Control bytes of SyncManagers 0 and
//! 1 and the mailbox buffers, never AL Control or the master's part of the SyncManager
//! registers.

use std::collections::VecDeque;

use ecat_wire::{AlState, MAILBOX_HEADER_LEN, MailboxHeader, MailboxType, SyncManagerConfig};
use flowlab_mailbox::msg::{self, EoeIpParameters};
use flowlab_mailbox::responder::{Message, Responder, ResponderConfig, ResponderLog};
use flowlab_pdi::{PdiError, SlaveAccess, SpiBus};

/// AL Control.
const REG_AL_CONTROL: u16 = 0x0120;
/// AL Event Request; bit 0: AL Control was written.
const REG_AL_EVENT: u16 = 0x0220;
/// Configured station address.
const REG_STATION: u16 = 0x0010;
/// SyncManager 0's register block; SyncManager 1's follows.
const REG_SM: u16 = 0x0800;
/// First byte of process RAM.
const RAM_START: u32 = 0x1000;
/// End of the LAN9252's process RAM.
const RAM_END: u32 = 0x2000;
/// SyncManager status: mailbox full.
const SM_FULL: u8 = 0x08;
/// SyncManager control: interrupt in AL Event Request.
const CONTROL_AL_EVENT: u8 = 0x20;
/// Polls between SyncManager status reads when the AL Event bits drive the mailbox.
const STATUS_FALLBACK_POLLS: u64 = 64;
/// Bytes read first from "mailbox out": the header and short messages.
const HEAD_BYTES: usize = 64;
/// SyncManager PDI Control: deactivated from the PDI side.
const PDI_DEACTIVATE: u8 = 0x01;
/// SyncManager control, low four bits: mailbox, the master writes.
const CONTROL_MBX_OUT: u8 = 0x06;
/// SyncManager control, low four bits: mailbox, the master reads.
const CONTROL_MBX_IN: u8 = 0x02;
/// Smallest mailbox accepted: header and the longest fixed protocol header (CoE SDO, 10 B).
pub const MIN_MAILBOX_BYTES: u16 = 34;

const AL_INVALID_TRANSITION: u16 = 0x0011;
const AL_UNKNOWN_STATE: u16 = 0x0012;
const AL_BOOT_NOT_SUPPORTED: u16 = 0x0013;
const AL_INVALID_MAILBOX: u16 = 0x0016;

/// What the slave application needs from the ESC; [`SlaveAccess`] provides it.
pub trait EscPdi {
    /// Read ESC registers (below 0x1000) or process RAM.
    fn read(&mut self, address: u16, buf: &mut [u8]) -> Result<(), PdiError>;
    /// Write process RAM.
    fn write_ram(&mut self, address: u16, data: &[u8]) -> Result<(), PdiError>;
    /// Write AL Status Code, then AL Status.
    fn write_al_status(&mut self, status: u16, code: u16) -> Result<(), PdiError>;
    /// Write the PDI Control byte of SyncManager `sm`; bit 0 deactivates it from the PDI side.
    fn write_pdi_control(&mut self, sm: u8, value: u8) -> Result<(), PdiError>;
}

impl<S: SpiBus> EscPdi for SlaveAccess<S> {
    fn read(&mut self, address: u16, buf: &mut [u8]) -> Result<(), PdiError> {
        SlaveAccess::read(self, address, buf)
    }

    fn write_ram(&mut self, address: u16, data: &[u8]) -> Result<(), PdiError> {
        SlaveAccess::write_ram(self, address, data)
    }

    fn write_al_status(&mut self, status: u16, code: u16) -> Result<(), PdiError> {
        SlaveAccess::write_al_status(self, status, code)
    }

    fn write_pdi_control(&mut self, sm: u8, value: u8) -> Result<(), PdiError> {
        SlaveAccess::write_pdi_control(self, sm, value)
    }
}

/// Settings of a [`PdiSlave`].
#[derive(Debug, Clone)]
pub struct PdiSlaveConfig {
    /// Largest mailbox SyncManager accepted, bytes.
    pub max_mailbox_bytes: u16,
    /// The application behind the mailbox.
    pub responder: ResponderConfig,
    /// Answer EoE Set IP Parameter requests with success and keep them for the caller, which
    /// applies them (else they go to the responder, which ignores them).
    pub answer_set_ip: bool,
    /// Read and write only the message and the buffer's last DWORD, not the whole mailbox
    /// buffer.
    pub message_only: bool,
}

impl Default for PdiSlaveConfig {
    fn default() -> Self {
        Self {
            max_mailbox_bytes: 1024,
            responder: ResponderConfig::default(),
            answer_set_ip: false,
            message_only: true,
        }
    }
}

/// Where the master configured the two mailboxes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mailboxes {
    /// "Mailbox out" (SyncManager 0, the master writes): start address.
    pub out_start: u16,
    /// Its length, bytes.
    pub out_len: u16,
    /// "Mailbox in" (SyncManager 1, the master reads): start address.
    pub in_start: u16,
    /// Its length, bytes.
    pub in_len: u16,
}

/// Counters of a [`PdiSlave`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PdiSlaveStats {
    /// Polls ([`PdiSlave::step`] calls).
    pub polls: u64,
    /// AL Control requests read.
    pub al_requests: u64,
    /// Messages taken from "mailbox out".
    pub messages_in: u64,
    /// Messages put into "mailbox in".
    pub messages_out: u64,
    /// Bytes of mailbox buffers read.
    pub bytes_in: u64,
    /// Bytes of mailbox buffers written.
    pub bytes_out: u64,
    /// SyncManager status reads (none per poll while AL Event drives the mailbox).
    pub status_reads: u64,
}

/// A state change the application made, for the caller to report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlChange {
    /// The state AL Status now shows.
    pub state: AlState,
    /// The error code, when the request was refused.
    pub error: Option<u16>,
    /// The mailboxes, in PRE-OP.
    pub mailboxes: Option<Mailboxes>,
}

/// FlowLab's slave application on a real ESC (see the [module docs](self)).
#[derive(Debug)]
pub struct PdiSlave<E> {
    esc: E,
    max_mailbox: u16,
    answer_set_ip: bool,
    message_only: bool,
    /// Both mailbox SyncManagers raise their AL Event bit.
    sm_events: bool,
    /// "Mailbox in" is empty, as far as known.
    in_free: bool,
    set_ip: Vec<EoeIpParameters>,
    app: Responder,
    state: AlState,
    error: Option<u16>,
    mailboxes: Option<Mailboxes>,
    station: u16,
    outbox: VecDeque<Message>,
    counter: u8,
    buf: Vec<u8>,
    stats: PdiSlaveStats,
}

impl<E: EscPdi> PdiSlave<E> {
    /// Start in INIT: AL Status INIT, no error.
    pub fn new(mut esc: E, cfg: PdiSlaveConfig) -> Result<Self, PdiError> {
        esc.write_pdi_control(0, PDI_DEACTIVATE)?;
        esc.write_pdi_control(1, PDI_DEACTIVATE)?;
        esc.write_al_status(u16::from(AlState::Init.code()), 0)?;
        let len = usize::from(cfg.max_mailbox_bytes);
        Ok(Self {
            esc,
            max_mailbox: cfg.max_mailbox_bytes,
            answer_set_ip: cfg.answer_set_ip,
            message_only: cfg.message_only,
            sm_events: false,
            in_free: false,
            set_ip: Vec::new(),
            app: Responder::new(cfg.responder, len, len),
            state: AlState::Init,
            error: None,
            mailboxes: None,
            station: 0,
            outbox: VecDeque::new(),
            counter: 0,
            buf: Vec::with_capacity(len),
            stats: PdiSlaveStats::default(),
        })
    }

    /// Poll once (see the [module docs](self)). Returns the state change, if the master
    /// requested one, and whether anything moved.
    pub fn step(&mut self) -> Result<(Option<AlChange>, bool), PdiError> {
        self.stats.polls += 1;
        let mut event = [0u8; 4];
        self.esc.read(REG_AL_EVENT, &mut event)?;
        let mut change = None;
        if event[0] & 0x01 != 0 {
            let mut control = [0u8; 2];
            self.esc.read(REG_AL_CONTROL, &mut control)?;
            self.stats.al_requests += 1;
            change = self.al_control(control[0])?;
        }
        let Some(m) = self.mailboxes else { return Ok((change, change.is_some())) };
        let mut moved = change.is_some();
        let fallback = !self.sm_events || self.stats.polls.is_multiple_of(STATUS_FALLBACK_POLLS);
        let mut out_full = self.sm_events && event[1] & 0x01 != 0;
        if !out_full && fallback {
            out_full = self.sm_status(0)? & SM_FULL != 0;
        }
        if out_full {
            self.read_message(m)?;
            moved = true;
        }
        if self.outbox.is_empty() && self.app.eoe_pending() {
            self.outbox.extend(self.app.next_eoe_frame());
        }
        if !self.outbox.is_empty() {
            if !self.in_free && self.sm_events && event[1] & 0x02 != 0 {
                self.in_free = true;
            }
            if !self.in_free && fallback {
                self.in_free = self.sm_status(1)? & SM_FULL == 0;
            }
            if self.in_free
                && let Some(answer) = self.outbox.pop_front()
            {
                self.write_message(m, &answer)?;
                moved = true;
            }
        }
        Ok((change, moved))
    }

    /// Queue an Ethernet frame for the master, as EoE fragments.
    pub fn send_eoe_frame(&mut self, frame: &[u8]) {
        if self.mailboxes.is_some() {
            let messages = self.app.eoe_messages(frame);
            self.outbox.extend(messages);
        }
    }

    /// The Ethernet frames the master sent by EoE since the last call.
    pub fn take_eoe_frames(&mut self) -> Vec<Vec<u8>> {
        self.app.take_eoe_frames()
    }

    /// The EoE Set IP Parameter requests answered since the last call (with
    /// [`PdiSlaveConfig::answer_set_ip`]).
    pub fn take_set_ip(&mut self) -> Vec<EoeIpParameters> {
        std::mem::take(&mut self.set_ip)
    }

    /// Messages waiting for "mailbox in".
    pub fn outbox_len(&self) -> usize {
        self.outbox.len()
    }

    /// The status byte of mailbox SyncManager `n` (0x0805 + 8·n).
    fn sm_status(&mut self, n: u16) -> Result<u8, PdiError> {
        let mut sm = [0u8; 4];
        self.esc.read(REG_SM + 8 * n + 4, &mut sm)?;
        self.stats.status_reads += 1;
        Ok(sm[1])
    }

    /// Read the message in "mailbox out" (which frees it) and hand it on.
    fn read_message(&mut self, m: Mailboxes) -> Result<(), PdiError> {
        let len = usize::from(m.out_len);
        self.buf.resize(len, 0);
        let mut read = len;
        if self.message_only && len > HEAD_BYTES + 8 {
            self.esc.read(m.out_start, &mut self.buf[..HEAD_BYTES])?;
            let body = MailboxHeader::parse(&self.buf).map_or(len, |h| usize::from(h.length));
            let end = (MAILBOX_HEADER_LEN + body).min(len).div_ceil(4) * 4;
            if end + HEAD_BYTES < len {
                if end > HEAD_BYTES {
                    self.esc.read(m.out_start + HEAD_BYTES as u16, &mut self.buf[HEAD_BYTES..end])?;
                }
                self.esc.read(m.out_start + (len - 4) as u16, &mut self.buf[len - 4..])?;
                read = end.max(HEAD_BYTES) + 4;
            } else {
                self.esc.read(m.out_start + HEAD_BYTES as u16, &mut self.buf[HEAD_BYTES..])?;
            }
        } else {
            self.esc.read(m.out_start, &mut self.buf)?;
        }
        self.stats.messages_in += 1;
        self.stats.bytes_in += read as u64;
        if self.answer_set_ip
            && let Ok(h) = MailboxHeader::parse(&self.buf)
            && h.mailbox_type == MailboxType::Eoe
        {
            let end = (MAILBOX_HEADER_LEN + usize::from(h.length)).min(len);
            if let Some(p) = msg::eoe_set_ip_parse(&self.buf[MAILBOX_HEADER_LEN..end]) {
                self.set_ip.push(p);
                self.outbox.push_back(Message { kind: MailboxType::Eoe, body: msg::eoe_set_ip_response(0) });
                return Ok(());
            }
        }
        let answers = self.app.take_message(&self.buf);
        self.outbox.extend(answers);
        Ok(())
    }

    /// Write `answer` into "mailbox in" (writing its last byte hands it to the master).
    fn write_message(&mut self, m: Mailboxes, answer: &Message) -> Result<(), PdiError> {
        self.counter = msg::next_counter(self.counter);
        let len = usize::from(m.in_len);
        let mut buf = msg::mailbox_buffer(answer.kind, self.counter, self.station, &answer.body, len);
        buf.truncate(len);
        let end = (MAILBOX_HEADER_LEN + answer.body.len()).min(len).div_ceil(4) * 4;
        if self.message_only && end + HEAD_BYTES < len {
            self.esc.write_ram(m.in_start, &buf[..end])?;
            self.esc.write_ram(m.in_start + (len - 4) as u16, &buf[len - 4..])?;
            self.stats.bytes_out += end as u64 + 4;
        } else {
            self.esc.write_ram(m.in_start, &buf)?;
            self.stats.bytes_out += len as u64;
        }
        self.in_free = false;
        self.stats.messages_out += 1;
        Ok(())
    }

    /// Show INIT in AL Status, as when the application stops.
    pub fn stop(&mut self) -> Result<(), PdiError> {
        self.restart();
        self.state = AlState::Init;
        self.error = None;
        self.esc.write_pdi_control(0, PDI_DEACTIVATE)?;
        self.esc.write_pdi_control(1, PDI_DEACTIVATE)?;
        self.esc.write_al_status(u16::from(AlState::Init.code()), 0)
    }

    /// The state AL Status shows and its error code.
    pub fn al_state(&self) -> (AlState, Option<u16>) {
        (self.state, self.error)
    }

    /// The mailboxes, in PRE-OP.
    pub fn mailboxes(&self) -> Option<Mailboxes> {
        self.mailboxes
    }

    /// Counters so far.
    pub fn stats(&self) -> PdiSlaveStats {
        self.stats
    }

    /// What the application received since the last call (see [`Responder::drain_log`]).
    pub fn drain_log(&mut self) -> ResponderLog {
        self.app.drain_log()
    }

    /// The ESC access back.
    pub fn into_inner(self) -> E {
        self.esc
    }

    fn restart(&mut self) {
        self.mailboxes = None;
        self.outbox.clear();
        self.counter = 0;
        self.app.restart();
    }

    fn al_control(&mut self, v: u8) -> Result<Option<AlChange>, PdiError> {
        use AlState::{Boot, Init, Op, PreOp, SafeOp};
        let requested = AlState::from_code(v & 0x0F);
        let ack = v & 0x10 != 0;
        if self.error.is_some() && !ack {
            return Ok(None);
        }
        let result = match (self.state, requested) {
            (_, Init) => Ok(Init),
            (Init, PreOp) => match self.open_mailboxes() {
                Ok(()) => {
                    self.esc.write_pdi_control(0, 0)?;
                    self.esc.write_pdi_control(1, 0)?;
                    Ok(PreOp)
                }
                Err(code) => Err(code),
            },
            (PreOp, PreOp) => Ok(PreOp),
            (_, SafeOp | Op) => Err(AL_INVALID_TRANSITION),
            (_, Boot) => Err(AL_BOOT_NOT_SUPPORTED),
            (_, AlState::Other(_)) => Err(AL_UNKNOWN_STATE),
            _ => Err(AL_INVALID_TRANSITION),
        };
        let result = match result {
            Ok(next) => {
                if next == Init {
                    self.restart();
                    self.esc.write_pdi_control(0, PDI_DEACTIVATE)?;
                    self.esc.write_pdi_control(1, PDI_DEACTIVATE)?;
                }
                self.state = next;
                self.error = None;
                Ok(())
            }
            Err(code) => {
                self.error = Some(code);
                Err(code)
            }
        };
        let status = u16::from(self.state.code()) | if result.is_err() { 0x10 } else { 0 };
        self.esc.write_al_status(status, self.error.unwrap_or(0))?;
        Ok(Some(AlChange { state: self.state, error: self.error, mailboxes: self.mailboxes }))
    }

    /// Check SyncManagers 0 and 1 and set the mailboxes up, or refuse with 0x0016.
    fn open_mailboxes(&mut self) -> Result<(), u16> {
        let mut regs = [0u8; 16];
        let mut station = [0u8; 2];
        let read = self.esc.read(REG_SM, &mut regs).and_then(|()| self.esc.read(REG_STATION, &mut station));
        if read.is_err() {
            return Err(AL_INVALID_MAILBOX);
        }
        let parse = |at: usize| SyncManagerConfig::parse(&regs[at..at + 8]).ok();
        let (Some(out), Some(inbox)) = (parse(0), parse(8)) else { return Err(AL_INVALID_MAILBOX) };
        let fits = |sm: &SyncManagerConfig, control: u8| {
            sm.activate & 0x01 != 0
                && sm.control & 0x0F == control
                && (MIN_MAILBOX_BYTES..=self.max_mailbox).contains(&sm.length)
                && u32::from(sm.start) >= RAM_START
                && u32::from(sm.start) + u32::from(sm.length) <= RAM_END
        };
        let apart = u32::from(out.start) + u32::from(out.length) <= u32::from(inbox.start)
            || u32::from(inbox.start) + u32::from(inbox.length) <= u32::from(out.start);
        if !(fits(&out, CONTROL_MBX_OUT) && fits(&inbox, CONTROL_MBX_IN) && apart) {
            return Err(AL_INVALID_MAILBOX);
        }
        self.station = u16::from_le_bytes(station);
        self.sm_events = out.control & CONTROL_AL_EVENT != 0 && inbox.control & CONTROL_AL_EVENT != 0;
        self.in_free = true;
        let m = Mailboxes { out_start: out.start, out_len: out.length, in_start: inbox.start, in_len: inbox.length };
        let eoe_send = self.app.config().eoe_send;
        self.app.reconfigure(usize::from(m.out_len), usize::from(m.in_len), eoe_send);
        self.outbox.clear();
        self.counter = 0;
        self.mailboxes = Some(m);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use ecat_wire::{FoeOpcode, MailboxHeader, MailboxType};

    use super::*;

    /// ESC memory with the mailbox rules seen from both sides: the master's write of the last
    /// byte of "mailbox out" fills it, the PDI's read of that byte empties it; the PDI's write
    /// of the last byte of "mailbox in" fills it, the master's read empties it.
    struct FakeEsc {
        mem: Vec<u8>,
        al_event: bool,
        out_full: bool,
        in_full: bool,
        /// AL Event bit 9: the master read "mailbox in"; the PDI's next write clears it.
        in_read: bool,
        /// SyncManager control bit 5 (AL Event) set by the master.
        events: bool,
        writes: Vec<u16>,
    }

    impl FakeEsc {
        /// As the vendor stack leaves it when it stops: both mailboxes deactivated by the PDI.
        fn new() -> Self {
            let mut mem = vec![0; 0x2000];
            mem[0x807] = PDI_DEACTIVATE;
            mem[0x80F] = PDI_DEACTIVATE;
            Self {
                mem,
                al_event: false,
                out_full: false,
                in_full: false,
                in_read: false,
                events: true,
                writes: Vec::new(),
            }
        }

        fn sm(&self, n: usize) -> SyncManagerConfig {
            SyncManagerConfig::parse(&self.mem[0x800 + 8 * n..0x808 + 8 * n]).unwrap()
        }

        /// The master configures both mailboxes with `len` bytes each.
        fn configure(&mut self, len: u16) {
            // The master writes start, length, control and activate; PDI Control stays the PDI's.
            let (pdi0, pdi1) = (self.mem[0x807], self.mem[0x80F]);
            let out = SyncManagerConfig {
                start: 0x1000,
                length: len,
                control: if self.events { 0x26 } else { 0x06 },
                status: 0,
                activate: 1,
                pdi_control: pdi0,
            };
            let inbox = SyncManagerConfig {
                start: 0x1000 + len,
                control: if self.events { 0x22 } else { 0x02 },
                pdi_control: pdi1,
                ..out
            };
            self.mem[0x800..0x808].copy_from_slice(&out.encode());
            self.mem[0x808..0x810].copy_from_slice(&inbox.encode());
            self.mem[0x10..0x12].copy_from_slice(&0x1000u16.to_le_bytes());
        }

        fn request(&mut self, control: u8) {
            self.mem[0x120] = control;
            self.al_event = true;
        }

        fn status(&self) -> (u8, u16) {
            (self.mem[0x130], u16::from_le_bytes([self.mem[0x134], self.mem[0x135]]))
        }

        fn master_write(&mut self, buf: &[u8]) -> bool {
            let sm = self.sm(0);
            if self.out_full || sm.pdi_control & PDI_DEACTIVATE != 0 || buf.len() != usize::from(sm.length) {
                return false;
            }
            let s = usize::from(sm.start);
            self.mem[s..s + buf.len()].copy_from_slice(buf);
            self.out_full = true;
            true
        }

        fn master_read(&mut self) -> Option<Vec<u8>> {
            let sm = self.sm(1);
            let s = usize::from(sm.start);
            let full = std::mem::take(&mut self.in_full);
            self.in_read |= full;
            full.then(|| self.mem[s..s + usize::from(sm.length)].to_vec())
        }
    }

    impl EscPdi for FakeEsc {
        fn read(&mut self, address: u16, buf: &mut [u8]) -> Result<(), PdiError> {
            let a = usize::from(address);
            self.mem[0x220] = u8::from(self.al_event);
            self.mem[0x221] = if self.events { u8::from(self.out_full) | u8::from(self.in_read) << 1 } else { 0 };
            self.mem[0x805] = if self.out_full { SM_FULL } else { 0 };
            self.mem[0x80D] = if self.in_full { SM_FULL } else { 0 };
            buf.copy_from_slice(&self.mem[a..a + buf.len()]);
            if (a..a + buf.len()).contains(&0x120) {
                self.al_event = false;
            }
            let sm = self.sm(0);
            let last = usize::from(sm.start) + usize::from(sm.length) - 1;
            if sm.length > 0 && (a..a + buf.len()).contains(&last) {
                self.out_full = false;
            }
            Ok(())
        }

        fn write_ram(&mut self, address: u16, data: &[u8]) -> Result<(), PdiError> {
            assert!(address >= 0x1000, "registers are written only through write_al_status");
            let a = usize::from(address);
            self.mem[a..a + data.len()].copy_from_slice(data);
            self.writes.push(address);
            let sm = self.sm(1);
            let last = usize::from(sm.start) + usize::from(sm.length) - 1;
            assert!(!self.in_full, "written while the master had not read the last message");
            if a >= usize::from(sm.start) && a < last {
                self.in_read = false;
            }
            if (a..a + data.len()).contains(&last) {
                self.in_full = true;
            }
            Ok(())
        }

        fn write_al_status(&mut self, status: u16, code: u16) -> Result<(), PdiError> {
            self.mem[0x134..0x136].copy_from_slice(&code.to_le_bytes());
            self.mem[0x130..0x132].copy_from_slice(&status.to_le_bytes());
            self.writes.push(0x130);
            Ok(())
        }

        fn write_pdi_control(&mut self, sm: u8, value: u8) -> Result<(), PdiError> {
            let a = 0x807 + 8 * usize::from(sm);
            self.mem[a] = value;
            self.writes.push(a as u16);
            Ok(())
        }
    }

    fn preop(len: u16) -> PdiSlave<FakeEsc> {
        let mut s = PdiSlave::new(FakeEsc::new(), PdiSlaveConfig::default()).unwrap();
        s.esc.configure(len);
        s.esc.request(0x02);
        let (change, _) = s.step().unwrap();
        assert_eq!(change.unwrap().state, AlState::PreOp);
        s
    }

    fn run(s: &mut PdiSlave<FakeEsc>, steps: usize) {
        for _ in 0..steps {
            s.step().unwrap();
        }
    }

    #[test]
    fn pre_op_takes_the_mailboxes_the_master_configured() {
        let s = preop(1024);
        assert_eq!(s.esc.status(), (0x02, 0));
        let m = s.mailboxes().unwrap();
        assert_eq!((m.out_start, m.out_len, m.in_start, m.in_len), (0x1000, 1024, 0x1400, 1024));
        assert_eq!(s.station, 0x1000);
    }

    #[test]
    fn mailboxes_out_of_bounds_are_refused_until_acknowledged() {
        let mut s =
            PdiSlave::new(FakeEsc::new(), PdiSlaveConfig { max_mailbox_bytes: 512, ..Default::default() }).unwrap();
        s.esc.configure(1024);
        s.esc.request(0x02);
        s.step().unwrap();
        assert_eq!(s.esc.status(), (0x11, AL_INVALID_MAILBOX));
        s.esc.configure(256);
        s.esc.request(0x02);
        assert!(s.step().unwrap().0.is_none(), "ignored without the acknowledge bit");
        s.esc.request(0x12);
        s.step().unwrap();
        assert_eq!(s.esc.status(), (0x02, 0));
    }

    #[test]
    fn safe_op_and_boot_are_refused() {
        let mut s = preop(128);
        s.esc.request(0x04);
        s.step().unwrap();
        assert_eq!(s.esc.status(), (0x12, AL_INVALID_TRANSITION));
        s.esc.request(0x13);
        s.step().unwrap();
        assert_eq!(s.esc.status(), (0x12, AL_BOOT_NOT_SUPPORTED));
        s.esc.request(0x11);
        s.step().unwrap();
        assert_eq!(s.esc.status(), (0x01, 0));
        assert!(s.mailboxes().is_none());
    }

    #[test]
    fn a_full_1024_byte_mailbox_carries_foe_both_ways() {
        let len = 1024;
        let mut s = preop(len);
        let write = msg::foe(FoeOpcode::Write, 0, b"big.bin");
        assert!(s.esc.master_write(&msg::mailbox_buffer(MailboxType::Foe, 1, 0, &write, usize::from(len))));
        run(&mut s, 2);
        let ack = s.esc.master_read().expect("ACK 0");
        let h = MailboxHeader::parse(&ack).unwrap();
        assert_eq!((h.mailbox_type, h.address), (MailboxType::Foe, 0x1000));
        let payload: Vec<u8> = (0..1018 - 6).map(|i| i as u8).collect();
        let data = msg::foe(FoeOpcode::Data, 1, &payload);
        assert_eq!(data.len() + 6, usize::from(len), "one DATA packet fills the mailbox");
        assert!(s.esc.master_write(&msg::mailbox_buffer(MailboxType::Foe, 2, 0, &data, usize::from(len))));
        run(&mut s, 2);
        assert!(s.esc.master_read().is_some(), "ACK 1");
        let short = msg::foe(FoeOpcode::Data, 2, &[0xAB; 10]);
        assert!(s.esc.master_write(&msg::mailbox_buffer(MailboxType::Foe, 3, 0, &short, usize::from(len))));
        run(&mut s, 2);
        assert!(s.esc.master_read().is_some(), "ACK 2");
        let files = s.drain_log().files;
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].1.len(), payload.len() + 10);
        let st = s.stats();
        assert_eq!((st.messages_in, st.messages_out), (3, 3));
        assert!(
            s.esc.writes.iter().all(|&a| matches!(a, 0x130 | 0x807 | 0x80F | 0x1400 | 0x17FC)),
            "only AL Status, PDI Control and mailbox in (message and last DWORD)"
        );
    }

    #[test]
    fn answers_wait_until_the_master_has_read_the_last_one() {
        let mut s = preop(128);
        let write = msg::foe(FoeOpcode::Write, 0, b"a");
        s.esc.master_write(&msg::mailbox_buffer(MailboxType::Foe, 1, 0, &write, 128));
        run(&mut s, 1);
        let data = msg::foe(FoeOpcode::Data, 1, b"x");
        s.esc.master_write(&msg::mailbox_buffer(MailboxType::Foe, 2, 0, &data, 128));
        run(&mut s, 3);
        assert_eq!(s.stats().messages_out, 1, "the second ACK waits for the first to be read");
        assert!(s.esc.master_read().is_some());
        run(&mut s, 1);
        assert_eq!(s.stats().messages_out, 2);
    }

    #[test]
    fn without_al_events_the_status_registers_drive_the_mailbox() {
        let mut fake = FakeEsc::new();
        fake.events = false;
        let mut s = PdiSlave::new(fake, PdiSlaveConfig::default()).unwrap();
        s.esc.configure(256);
        s.esc.request(0x02);
        s.step().unwrap();
        assert!(!s.sm_events);
        let write = msg::foe(FoeOpcode::Write, 0, b"a");
        s.esc.master_write(&msg::mailbox_buffer(MailboxType::Foe, 1, 0, &write, 256));
        run(&mut s, 2);
        assert!(s.esc.master_read().is_some(), "ACK 0");
        assert!(s.stats().status_reads >= 2);
    }

    #[test]
    fn with_al_events_an_idle_poll_reads_only_al_event() {
        let mut s = preop(1024);
        assert!(s.sm_events);
        let before = s.stats().status_reads;
        run(&mut s, 10);
        assert_eq!(s.stats().status_reads, before, "no status reads between fallbacks");
    }

    #[test]
    fn short_answers_write_the_message_and_the_last_dword_only() {
        let mut s = preop(1024);
        let write = msg::foe(FoeOpcode::Write, 0, b"a");
        s.esc.master_write(&msg::mailbox_buffer(MailboxType::Foe, 1, 0, &write, 1024));
        s.esc.writes.clear();
        run(&mut s, 1);
        assert_eq!(s.esc.writes, [0x1400, 0x1400 + 1020], "message, then the last DWORD");
        let ack = s.esc.master_read().unwrap();
        let h = MailboxHeader::parse(&ack).unwrap();
        assert_eq!((h.mailbox_type, h.length), (MailboxType::Foe, 6));
        assert!(s.stats().bytes_in < 200 && s.stats().bytes_out < 200, "{:?}", s.stats());
    }

    #[test]
    fn eoe_set_ip_is_answered_and_frames_cross_both_ways() {
        let cfg = PdiSlaveConfig { answer_set_ip: true, ..Default::default() };
        let mut s = PdiSlave::new(FakeEsc::new(), cfg).unwrap();
        s.esc.configure(1024);
        s.esc.request(0x02);
        s.step().unwrap();
        let p = EoeIpParameters { ip: Some([10, 0, 0, 2]), ..Default::default() };
        let req = msg::eoe_set_ip_request(&p);
        s.esc.master_write(&msg::mailbox_buffer(MailboxType::Eoe, 1, 0, &req, 1024));
        run(&mut s, 2);
        assert_eq!(s.take_set_ip(), [p]);
        let r = s.esc.master_read().unwrap();
        assert_eq!(msg::eoe_set_ip_result(&r[MAILBOX_HEADER_LEN..]), Some(0));
        // A 1514-byte frame from the master in two fragments, and one back.
        let frame: Vec<u8> = (0..1514).map(|i| i as u8).collect();
        for (k, f) in msg::eoe_fragments(&frame, msg::eoe_capacity(1024), 1).iter().enumerate() {
            assert!(s.esc.master_write(&msg::mailbox_buffer(MailboxType::Eoe, 2 + k as u8, 0, f, 1024)));
            run(&mut s, 1);
        }
        assert_eq!(s.take_eoe_frames(), std::slice::from_ref(&frame));
        s.send_eoe_frame(&frame);
        assert_eq!(s.outbox_len(), 2);
        let mut back = msg::EoeReassembler::default();
        let mut got = None;
        for _ in 0..4 {
            run(&mut s, 1);
            if let Some(b) = s.esc.master_read() {
                let h = MailboxHeader::parse(&b).unwrap();
                got = got.or(back.push(&b[MAILBOX_HEADER_LEN..MAILBOX_HEADER_LEN + usize::from(h.length)]));
            }
        }
        assert_eq!(got, Some(frame));
    }

    #[test]
    fn stop_shows_init() {
        let mut s = preop(128);
        assert_eq!((s.esc.mem[0x807], s.esc.mem[0x80F]), (0, 0), "PRE-OP activates both mailboxes");
        s.stop().unwrap();
        assert_eq!(s.esc.status(), (0x01, 0));
        assert_eq!((s.esc.mem[0x807], s.esc.mem[0x80F]), (PDI_DEACTIVATE, PDI_DEACTIVATE));
        assert!(s.mailboxes().is_none());
    }
}
