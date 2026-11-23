//! The ESC: register file and process RAM, the SII interface, the AL state machine, mailbox
//! SyncManagers, FMMUs, and the datagrams that reach them.

use std::collections::VecDeque;

use ecat_wire::{AlState, Command, SyncManagerConfig};
use flowlab_mailbox::msg;
use flowlab_mailbox::responder::{Message, Responder};

use crate::sii::{SM_CONTROL_INPUTS, SM_CONTROL_MBX_IN, SM_CONTROL_MBX_OUT, SM_CONTROL_OUTPUTS};
use crate::{Layout, SlaveStats};

const MEM: usize = 0x1_0000;

const REG_STATION: usize = 0x0010;
const REG_DL_STATUS: usize = 0x0110;
const REG_AL_CONTROL: usize = 0x0120;
const REG_AL_STATUS: usize = 0x0130;
const REG_AL_STATUS_CODE: usize = 0x0134;
const REG_SII_CONTROL: usize = 0x0502;
const REG_SII_ADDRESS: usize = 0x0504;
const REG_SII_DATA: usize = 0x0508;
const REG_FMMU: usize = 0x0600;
const REG_SM: usize = 0x0800;
const FMMUS: usize = 3;
const SMS: usize = 4;

/// SII control, low byte: 8-byte reads; the write-enable bit (0) is the master's.
const SII_READ_8: u8 = 0x40;
/// SII control, high byte: read, write and reload commands.
const SII_COMMANDS: u8 = 0x07;
/// SII control, high byte: command error.
const SII_COMMAND_ERROR: u8 = 0x20;
/// SII control, high byte: error flags (checksum, device info, command, write).
const SII_ERRORS: u8 = 0x78;

/// SyncManager status: mailbox full.
const SM_FULL: u8 = 0x08;
/// SyncManager activate: repeat request; PDI control: repeat acknowledge.
const SM_REPEAT: u8 = 0x02;

/// AL status codes (ETG.1000.6).
const AL_INVALID_TRANSITION: u16 = 0x0011;
const AL_UNKNOWN_STATE: u16 = 0x0012;
const AL_BOOT_NOT_SUPPORTED: u16 = 0x0013;
const AL_INVALID_MAILBOX: u16 = 0x0016;
const AL_INVALID_OUTPUTS: u16 = 0x001D;
const AL_INVALID_INPUTS: u16 = 0x001E;

fn le16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// Whether `[a, a + n)` and `[s, s + l)` overlap.
fn overlaps(a: usize, n: usize, s: usize, l: usize) -> bool {
    n > 0 && l > 0 && a < s + l && s < a + n
}

/// Registers the master cannot write.
fn read_only(a: usize) -> bool {
    matches!(a, 0x0000..=0x000F | 0x0110..=0x0111 | 0x0130..=0x0135)
        || ((REG_SM..REG_SM + 8 * SMS).contains(&a) && matches!((a - REG_SM) % 8, 5 | 7))
}

/// A mailbox SyncManager the master configured: its index and buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Area {
    sm: usize,
    start: usize,
    len: usize,
}

pub(crate) struct Esc {
    mem: Vec<u8>,
    eeprom: Vec<u8>,
    layout: Layout,
    app: Responder,
    eoe_start_after_ns: u64,
    /// "mailbox in" holds a message the master has not read to its last byte.
    in_full: bool,
    in_area: Option<Area>,
    last_sent: Option<Vec<u8>>,
    out_counter: u8,
    outbox: VecDeque<Message>,
    preop_since_ns: Option<u64>,
    now_ns: u64,
    pub(crate) stats: SlaveStats,
}

impl Esc {
    pub(crate) fn new(eeprom: Vec<u8>, layout: Layout, app: Responder, eoe_start_after_ns: u64) -> Self {
        let mut mem = vec![0u8; MEM];
        mem[0x0000] = 0x00; // type: not an ESC model
        mem[0x0004] = FMMUS as u8;
        mem[0x0005] = SMS as u8;
        mem[0x0006] = ((layout.ram_end - 0x1000) / 1024) as u8;
        mem[0x0007] = 0x03; // port 0 MII, ports 1–3 not implemented
        // DL status: PDI operational, link on port 0, communication on port 0, loops 1–3 closed.
        mem[REG_DL_STATUS..REG_DL_STATUS + 2].copy_from_slice(&0x5611u16.to_le_bytes());
        mem[REG_AL_STATUS] = AlState::Init.code();
        mem[REG_SII_CONTROL] = SII_READ_8;
        Self {
            mem,
            eeprom,
            layout,
            app,
            eoe_start_after_ns,
            in_full: false,
            in_area: None,
            last_sent: None,
            out_counter: 0,
            outbox: VecDeque::new(),
            preop_since_ns: None,
            now_ns: 0,
            stats: SlaveStats::default(),
        }
    }

    pub(crate) fn app(&self) -> &Responder {
        &self.app
    }

    pub(crate) fn app_mut(&mut self) -> &mut Responder {
        &mut self.app
    }

    /// Change the EEPROM and the SyncManager layout (in INIT): the master reads the new ones
    /// when it starts again. The application keeps its log and settings.
    pub(crate) fn reconfigure(&mut self, eeprom: Vec<u8>, layout: Layout, eoe_send: bool) {
        self.eeprom = eeprom;
        self.layout = layout;
        let len = usize::from(layout.mbx_len);
        self.app.reconfigure(len, len, eoe_send);
        self.restart();
    }

    pub(crate) fn al_state(&self) -> AlState {
        AlState::from_code(self.mem[REG_AL_STATUS] & 0x0F)
    }

    pub(crate) fn al_error(&self) -> Option<u16> {
        (self.mem[REG_AL_STATUS] & 0x10 != 0).then(|| le16(&self.mem, REG_AL_STATUS_CODE))
    }

    pub(crate) fn station(&self) -> u16 {
        le16(&self.mem, REG_STATION)
    }

    fn sm(&self, n: usize) -> Option<SyncManagerConfig> {
        SyncManagerConfig::parse(&self.mem[REG_SM + 8 * n..REG_SM + 8 * n + 8]).ok()
    }

    /// The mailbox SyncManager in `direction` (true: the master writes), if configured.
    fn mailbox(&self, master_writes: bool) -> Option<Area> {
        (0..SMS).find_map(|n| {
            let sm = self.sm(n)?;
            let control = sm.control & 0x0F;
            let mailbox = control & 0x03 == 0x02;
            let writes = (control >> 2) & 0x03 == 0x01;
            (sm.enabled() && mailbox && writes == master_writes && sm.length > 0).then_some(Area {
                sm: n,
                start: usize::from(sm.start),
                len: usize::from(sm.length),
            })
        })
    }

    // ---------------------------------------------------------------- frames and datagrams

    /// Process one EtherCAT PDU in place (`f[at..]` starts at the EtherCAT header). Returns
    /// false if it is malformed (it is then not returned to the master).
    pub(crate) fn process(&mut self, f: &mut [u8], at: usize, now_ns: u64) -> bool {
        self.now_ns = now_ns;
        self.service();
        if f.len() < at + 2 {
            return false;
        }
        let word = le16(f, at);
        let (len, kind) = (usize::from(word & 0x07FF), word >> 12);
        let end = at + 2 + len;
        if kind != 1 || end > f.len() {
            return false;
        }
        let mut o = at + 2;
        loop {
            if o + 10 + 2 > end {
                return false;
            }
            let cmd = Command::from_code(f[o]);
            let lw = le16(f, o + 6);
            let n = usize::from(lw & 0x07FF);
            let d0 = o + 10;
            if d0 + n + 2 > end {
                return false;
            }
            let mut wkc = le16(f, d0 + n);
            let (head, rest) = f.split_at_mut(d0);
            let address = &mut head[o + 2..o + 6];
            self.datagram(cmd, address, &mut rest[..n], &mut wkc);
            rest[n..n + 2].copy_from_slice(&wkc.to_le_bytes());
            self.stats.datagrams += 1;
            o = d0 + n + 2;
            if lw & 0x8000 == 0 {
                break;
            }
        }
        self.service();
        true
    }

    fn datagram(&mut self, cmd: Command, address: &mut [u8], data: &mut [u8], wkc: &mut u16) {
        let adp = le16(address, 0);
        let ado = usize::from(le16(address, 2));
        let station = self.station();
        let (addressed, increment) = match cmd {
            Command::Aprd | Command::Apwr | Command::Aprw | Command::Armw => (adp == 0, true),
            Command::Fprd | Command::Fpwr | Command::Fprw | Command::Frmw => (adp == station, false),
            Command::Brd | Command::Bwr | Command::Brw => (true, true),
            Command::Lrd | Command::Lwr | Command::Lrw => {
                self.logical(cmd, le32(address, 0), data, wkc);
                return;
            }
            Command::Nop | Command::Unknown(_) => return,
        };
        if increment {
            address[..2].copy_from_slice(&adp.wrapping_add(1).to_le_bytes());
        }
        let (read, write) = match cmd {
            Command::Armw | Command::Frmw => (addressed, !addressed),
            _ if !addressed => return,
            _ => (cmd.reads(), cmd.writes()),
        };
        let or = matches!(cmd, Command::Brd | Command::Brw);
        if read && write && !matches!(cmd, Command::Armw | Command::Frmw) {
            // Read-write: the frame carries the old value back, the memory takes the new.
            let new = data.to_vec();
            if self.read(ado, data, or) && self.write(ado, &new) {
                *wkc = wkc.wrapping_add(3);
            }
        } else if read {
            if self.read(ado, data, or) {
                *wkc = wkc.wrapping_add(1);
            }
        } else if write && self.write(ado, data) {
            *wkc = wkc.wrapping_add(1);
        }
    }

    /// A physical read; false if a mailbox blocks it ("mailbox in" empty).
    fn read(&mut self, ado: usize, data: &mut [u8], or: bool) -> bool {
        let n = data.len();
        if ado + n > MEM {
            return false;
        }
        let inbox = self.mailbox(false);
        if let Some(a) = inbox
            && overlaps(ado, n, a.start, a.len)
            && !self.in_full
        {
            return false;
        }
        self.refresh_status();
        for (d, m) in data.iter_mut().zip(&self.mem[ado..ado + n]) {
            *d = if or { *d | *m } else { *m };
        }
        if let Some(a) = inbox
            && overlaps(ado, n, a.start, a.len)
            && ado + n >= a.start + a.len
        {
            // The last byte was read: the buffer is the master's, the slave may fill it again.
            self.in_full = false;
            self.stats.messages_from_slave += 1;
        }
        true
    }

    /// A physical write; false if a mailbox blocks it.
    fn write(&mut self, ado: usize, data: &[u8]) -> bool {
        let n = data.len();
        if ado + n > MEM {
            return false;
        }
        if self.mailbox(false).is_some_and(|a| overlaps(ado, n, a.start, a.len)) {
            return false;
        }
        let outbox_area = self.mailbox(true);
        let activates: Vec<u8> = (0..SMS).map(|k| self.mem[REG_SM + 8 * k + 6]).collect();
        for (i, b) in data.iter().enumerate() {
            if !read_only(ado + i) {
                self.mem[ado + i] = *b;
            }
        }
        let touches = |reg: usize, len: usize| overlaps(ado, n, reg, len);
        if touches(REG_AL_CONTROL, 1) {
            self.al_control(self.mem[REG_AL_CONTROL]);
        }
        if touches(REG_SII_CONTROL, 2) {
            self.sii_command();
        }
        if touches(REG_SM, 8 * SMS) {
            self.sync_managers_written(&activates);
        }
        if let Some(a) = outbox_area
            && overlaps(ado, n, a.start, a.len)
            && ado + n >= a.start + a.len
        {
            // The last byte arrived: the slave takes the message at once.
            let buf = self.mem[a.start..a.start + a.len].to_vec();
            self.stats.messages_to_slave += 1;
            let replies = self.app.take_message(&buf);
            self.outbox.extend(replies);
        }
        true
    }

    fn logical(&mut self, cmd: Command, address: u32, data: &mut [u8], wkc: &mut u16) {
        let (lo, n) = (u64::from(address), data.len() as u64);
        let mut maps = Vec::new();
        for k in 0..FMMUS {
            let f = &self.mem[REG_FMMU + 16 * k..REG_FMMU + 16 * k + 16];
            if f[12] & 0x01 == 0 {
                continue;
            }
            let (start, len, phys, kind) = (u64::from(le32(f, 0)), u64::from(le16(f, 4)), le16(f, 8), f[11]);
            let (a, b) = (lo.max(start), (lo + n).min(start + len));
            if a < b {
                let at = (a - lo) as usize;
                let p = usize::from(phys) + (a - start) as usize;
                maps.push((at, p, (b - a) as usize, kind));
            }
        }
        let (mut read, mut wrote) = (false, false);
        if cmd != Command::Lrd {
            for &(at, p, len, kind) in &maps {
                if kind & 0x02 != 0 && p + len <= MEM {
                    self.mem[p..p + len].copy_from_slice(&data[at..at + len]);
                    wrote = true;
                }
            }
        }
        if cmd != Command::Lwr {
            for &(at, p, len, kind) in &maps {
                if kind & 0x01 != 0 && p + len <= MEM {
                    data[at..at + len].copy_from_slice(&self.mem[p..p + len]);
                    read = true;
                }
            }
        }
        if wrote {
            // The application: inputs echo the outputs.
            let l = self.layout;
            let k = usize::from(l.output_len.min(l.input_len));
            let (o, i) = (usize::from(l.outputs), usize::from(l.inputs));
            self.mem.copy_within(o..o + k, i);
        }
        *wkc = wkc.wrapping_add(u16::from(read) + 2 * u16::from(wrote));
    }

    // ---------------------------------------------------------------- registers

    /// SyncManager status bytes as the master reads them.
    fn refresh_status(&mut self) {
        let inbox = self.mailbox(false).map(|a| a.sm);
        for n in 0..SMS {
            let full = Some(n) == inbox && self.in_full;
            let s = &mut self.mem[REG_SM + 8 * n + 5];
            *s = (*s & !SM_FULL) | if full { SM_FULL } else { 0 };
        }
    }

    fn sync_managers_written(&mut self, activates_before: &[u8]) {
        let inbox = self.mailbox(false);
        if inbox != self.in_area {
            // Reconfigured or disabled: the buffer is empty.
            self.in_area = inbox;
            self.in_full = false;
        }
        if let Some(a) = inbox {
            let act = REG_SM + 8 * a.sm + 6;
            if (self.mem[act] ^ activates_before[a.sm]) & SM_REPEAT != 0 {
                // Repeat request: put the last message back and acknowledge.
                if let Some(buf) = self.last_sent.clone() {
                    let len = buf.len().min(a.len);
                    self.mem[a.start..a.start + len].copy_from_slice(&buf[..len]);
                    self.in_full = true;
                    self.stats.repeats += 1;
                }
                let pdi = REG_SM + 8 * a.sm + 7;
                self.mem[pdi] = (self.mem[pdi] & !SM_REPEAT) | (self.mem[act] & SM_REPEAT);
            }
        }
    }

    fn sii_command(&mut self) {
        let lo = REG_SII_CONTROL;
        self.mem[lo] = (self.mem[lo] & 0x01) | SII_READ_8;
        let command = self.mem[lo + 1] & SII_COMMANDS;
        self.mem[lo + 1] &= !(SII_COMMANDS | SII_ERRORS | 0x80);
        let word = le32(&self.mem, REG_SII_ADDRESS) as usize;
        match command {
            0x01 => {
                for i in 0..8 {
                    self.mem[REG_SII_DATA + i] = self.eeprom.get(word * 2 + i).copied().unwrap_or(0xFF);
                }
            }
            0x02 if self.mem[lo] & 0x01 != 0 => {
                if self.eeprom.len() < word * 2 + 2 {
                    self.eeprom.resize(word * 2 + 2, 0xFF);
                }
                let (a, b) = (self.mem[REG_SII_DATA], self.mem[REG_SII_DATA + 1]);
                self.eeprom[word * 2] = a;
                self.eeprom[word * 2 + 1] = b;
            }
            0x02 => self.mem[lo + 1] |= SII_COMMAND_ERROR,
            _ => {}
        }
    }

    fn set_al(&mut self, state: AlState, error: Option<u16>) {
        self.mem[REG_AL_STATUS] = state.code() | if error.is_some() { 0x10 } else { 0 };
        self.mem[REG_AL_STATUS_CODE..REG_AL_STATUS_CODE + 2].copy_from_slice(&error.unwrap_or(0).to_le_bytes());
    }

    /// Whether SyncManager `n` is configured as the EEPROM says (`len` bytes, enabled if any).
    fn sm_matches(&self, n: usize, start: u16, len: u16, control: u8) -> bool {
        let Some(sm) = self.sm(n) else { return false };
        if len == 0 {
            return true;
        }
        sm.enabled() && sm.start == start && sm.length == len && sm.control & 0x0F == control & 0x0F
    }

    fn al_control(&mut self, v: u8) {
        let requested = AlState::from_code(v & 0x0F);
        let ack = v & 0x10 != 0;
        let current = self.al_state();
        if self.al_error().is_some() && !ack {
            return;
        }
        let l = self.layout;
        use AlState::{Boot, Init, Op, PreOp, SafeOp};
        let result = match (current, requested) {
            (_, Init) => Ok(Init),
            (Init, PreOp) => {
                if self.sm_matches(0, l.mbx_out, l.mbx_len, SM_CONTROL_MBX_OUT)
                    && self.sm_matches(1, l.mbx_in, l.mbx_len, SM_CONTROL_MBX_IN)
                {
                    Ok(PreOp)
                } else {
                    Err(AL_INVALID_MAILBOX)
                }
            }
            (PreOp | SafeOp | Op, PreOp) => Ok(PreOp),
            (PreOp, SafeOp) => {
                if !self.sm_matches(2, l.outputs, l.output_len, SM_CONTROL_OUTPUTS) {
                    Err(AL_INVALID_OUTPUTS)
                } else if !self.sm_matches(3, l.inputs, l.input_len, SM_CONTROL_INPUTS) {
                    Err(AL_INVALID_INPUTS)
                } else {
                    Ok(SafeOp)
                }
            }
            (SafeOp | Op, SafeOp) => Ok(SafeOp),
            (SafeOp | Op, Op) => Ok(Op),
            (_, Boot) => Err(AL_BOOT_NOT_SUPPORTED),
            (_, AlState::Other(_)) => Err(AL_UNKNOWN_STATE),
            _ => Err(AL_INVALID_TRANSITION),
        };
        match result {
            Ok(next) => {
                if next == Init {
                    self.restart();
                } else if current == Init && next == PreOp {
                    self.preop_since_ns = Some(self.now_ns);
                }
                if next != current {
                    self.stats.al_changes += 1;
                }
                self.set_al(next, None);
            }
            Err(code) => self.set_al(current, Some(code)),
        }
    }

    /// Back to INIT: mailboxes empty, the application starts over.
    fn restart(&mut self) {
        self.in_full = false;
        self.outbox.clear();
        self.last_sent = None;
        self.preop_since_ns = None;
        self.app.restart();
    }

    // ---------------------------------------------------------------- the application

    /// Put the next message into "mailbox in" when it is free; start the EoE send when due.
    fn service(&mut self) {
        let state = self.al_state();
        if matches!(state, AlState::Init | AlState::Boot) {
            return;
        }
        if self.outbox.is_empty()
            && self.app.eoe_pending()
            && self.preop_since_ns.is_some_and(|t| self.now_ns >= t.saturating_add(self.eoe_start_after_ns))
        {
            let frames = self.app.next_eoe_frame();
            self.outbox.extend(frames);
        }
        let Some(a) = self.mailbox(false) else { return };
        if self.in_full {
            return;
        }
        let Some(m) = self.outbox.pop_front() else { return };
        self.out_counter = msg::next_counter(self.out_counter);
        let buf = msg::mailbox_buffer(m.kind, self.out_counter, self.station(), &m.body, a.len);
        let len = buf.len().min(a.len);
        self.mem[a.start..a.start + len].copy_from_slice(&buf[..len]);
        self.last_sent = Some(buf);
        self.in_full = true;
    }
}
