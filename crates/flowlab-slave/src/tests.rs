//! The ESC model, datagram by datagram.

use ecat_wire::{
    AlState, CoeHeader, CoeService, Command, DatagramSpec, FoeHeader, FoeOpcode, FrameBuilder, MAILBOX_HEADER_LEN,
    MacAddr, MailboxHeader, MailboxType, SdoCommand, SdoHeader, SyncManagerConfig, parse_frame,
};
use flowlab_mailbox::msg;

use crate::{EMULATED_ESC_NAME, EmulatedSlave, SlaveConfig};

const MASTER: MacAddr = MacAddr([0x10; 6]);
const STATION: u16 = 0x1000;

struct Bus {
    slave: EmulatedSlave,
    t: u64,
}

impl Bus {
    fn new(cfg: SlaveConfig) -> Self {
        Self { slave: EmulatedSlave::new(cfg).unwrap(), t: 0 }
    }

    /// One datagram in one frame: the data and working counter that come back.
    fn dg(&mut self, cmd: Command, adp: u16, ado: u16, data: &[u8]) -> (Vec<u8>, u16) {
        let address = u32::from(adp) | u32::from(ado) << 16;
        self.raw(cmd, address, data)
    }

    fn raw(&mut self, cmd: Command, address: u32, data: &[u8]) -> (Vec<u8>, u16) {
        let mut b = FrameBuilder::new(MacAddr([0xff; 6]), MASTER);
        b.push(DatagramSpec::new(cmd, 0, address, data)).unwrap();
        self.t += 10_000;
        let back = self.slave.handle(&b.finish(), self.t).expect("an answer");
        let f = parse_frame(&back).unwrap().unwrap();
        let d = f.pdu.datagrams().next().unwrap().unwrap();
        (d.data.to_vec(), d.wkc)
    }

    fn fprd(&mut self, ado: u16, len: usize) -> (Vec<u8>, u16) {
        self.dg(Command::Fprd, STATION, ado, &vec![0; len])
    }

    fn fpwr(&mut self, ado: u16, data: &[u8]) -> u16 {
        self.dg(Command::Fpwr, STATION, ado, data).1
    }

    fn eeprom(&mut self, word: u16) -> [u8; 8] {
        let mut req = vec![0x00, 0x01];
        req.extend_from_slice(&u32::from(word).to_le_bytes());
        assert_eq!(self.fpwr(0x0502, &req), 1);
        let (status, _) = self.fprd(0x0502, 2);
        assert_eq!(status[1] & 0x80, 0, "not busy");
        assert_eq!(status[0] & 0x40, 0x40, "8-byte reads");
        self.fprd(0x0508, 8).0.try_into().unwrap()
    }

    fn al(&mut self, state: AlState) -> (AlState, u16) {
        self.fpwr(0x0120, &[state.code(), 0]);
        let (s, _) = self.fprd(0x0130, 6);
        (AlState::from_code(s[0] & 0x0F), u16::from_le_bytes([s[4], s[5]]))
    }

    fn configure_mailboxes(&mut self) {
        let l = self.slave.layout();
        let sm0 = SyncManagerConfig {
            start: l.mbx_out,
            length: l.mbx_len,
            control: 0x26,
            status: 0,
            activate: 1,
            pdi_control: 0,
        };
        let sm1 = SyncManagerConfig { start: l.mbx_in, control: 0x22, ..sm0 };
        assert_eq!(self.fpwr(0x0800, &sm0.encode()), 1);
        assert_eq!(self.fpwr(0x0808, &sm1.encode()), 1);
    }

    fn to_preop() -> Self {
        let mut bus = Self::new(SlaveConfig::default());
        bus.dg(Command::Apwr, 0, 0x0010, &STATION.to_le_bytes());
        bus.configure_mailboxes();
        assert_eq!(bus.al(AlState::PreOp), (AlState::PreOp, 0));
        bus
    }

    fn send(&mut self, kind: MailboxType, counter: u8, body: &[u8]) -> u16 {
        let l = self.slave.layout();
        let buf = msg::mailbox_buffer(kind, counter, 0, body, usize::from(l.mbx_len));
        self.fpwr(l.mbx_out, &buf)
    }

    fn receive(&mut self) -> Option<(MailboxHeader, Vec<u8>)> {
        let l = self.slave.layout();
        let (buf, wkc) = self.fprd(l.mbx_in, usize::from(l.mbx_len));
        if wkc == 0 {
            return None;
        }
        let h = MailboxHeader::parse(&buf).unwrap();
        let body = buf[MAILBOX_HEADER_LEN..MAILBOX_HEADER_LEN + usize::from(h.length)].to_vec();
        Some((h, body))
    }
}

#[test]
fn counts_takes_a_station_address_and_increments_adp() {
    let mut bus = Bus::new(SlaveConfig::default());
    assert_eq!(bus.dg(Command::Brd, 0, 0x0000, &[0]).1, 1);
    let mut b = FrameBuilder::new(MacAddr([0xff; 6]), MASTER);
    b.push(DatagramSpec::new(Command::Aprd, 0, 0xFFFF, &[0, 0])).unwrap(); // position 1: not us
    let back = bus.slave.handle(&b.finish(), 1).unwrap();
    let f = parse_frame(&back).unwrap().unwrap();
    let d = f.pdu.datagrams().next().unwrap().unwrap();
    assert_eq!((d.wkc, d.address), (0, ecat_wire::DatagramAddress::Position { adp: 0, ado: 0 }));

    assert_eq!(bus.dg(Command::Apwr, 0, 0x0010, &STATION.to_le_bytes()).1, 1);
    assert_eq!(bus.slave.station(), STATION);
    assert_eq!(bus.fprd(0x0010, 2), (STATION.to_le_bytes().to_vec(), 1));
    assert_eq!(bus.dg(Command::Fprd, 0x1001, 0x0010, &[0, 0]).1, 0, "another station");
    // Read-only registers keep their value; the write still counts.
    let (dl, _) = bus.fprd(0x0110, 2);
    assert_eq!(bus.fpwr(0x0110, &[0, 0]), 1);
    assert_eq!(bus.fprd(0x0110, 2).0, dl);
}

#[test]
fn returning_and_foreign_frames_are_ignored() {
    let mut bus = Bus::new(SlaveConfig::default());
    let mut b = FrameBuilder::new(MacAddr([0xff; 6]), MacAddr([0x12; 6]));
    b.push(DatagramSpec::new(Command::Brd, 0, 0, &[0])).unwrap();
    assert!(bus.slave.handle(&b.finish(), 0).is_none(), "already passed an ESC");
    let mut ip = vec![0xff; 6];
    ip.extend_from_slice(&[0x10; 6]);
    ip.extend_from_slice(&[0x08, 0x00]);
    ip.resize(60, 0);
    assert!(bus.slave.handle(&ip, 0).is_none(), "IPv4");
}

#[test]
fn behind_an_esc_frames_with_the_mac_bit_are_answered() {
    let mut bus = Bus::new(SlaveConfig { behind_esc: true, ..SlaveConfig::default() });
    // As an upstream ESC passes it on: bit 1 of the source MAC set, position decremented to 0.
    let mut b = FrameBuilder::new(MacAddr([0xff; 6]), MacAddr([0x12; 6]));
    b.push(DatagramSpec::new(Command::Brd, 0, 0, &[0])).unwrap();
    let back = bus.slave.handle(&b.finish(), 0).expect("answered behind an ESC");
    let f = parse_frame(&back).unwrap().unwrap();
    assert_eq!(f.pdu.datagrams().next().unwrap().unwrap().wkc, 1);
    assert_eq!(back[6], 0x12);
}

#[test]
fn eeprom_holds_identity_mailbox_and_name() {
    let mut bus = Bus::new(SlaveConfig { serial: 42, ..SlaveConfig::default() });
    bus.dg(Command::Apwr, 0, 0x0010, &STATION.to_le_bytes());
    let id = bus.eeprom(0x0008);
    assert_eq!(u32::from_le_bytes(id[0..4].try_into().unwrap()), 0);
    assert_eq!(u32::from_le_bytes(id[4..8].try_into().unwrap()), 1);
    let serial = bus.eeprom(0x000E);
    assert_eq!(u32::from_le_bytes(serial[0..4].try_into().unwrap()), 42);
    let mbx = bus.eeprom(0x0018);
    let l = bus.slave.layout();
    assert_eq!(u16::from_le_bytes([mbx[0], mbx[1]]), l.mbx_out);
    assert_eq!(u16::from_le_bytes([mbx[2], mbx[3]]), l.mbx_len);
    assert_eq!(u16::from_le_bytes([mbx[4], mbx[5]]), l.mbx_in);
    let protocols = bus.eeprom(0x001C);
    assert_eq!(u16::from_le_bytes([protocols[0], protocols[1]]), 0x002E);
    // First category: strings, the device name first.
    let s = bus.eeprom(0x0040);
    assert_eq!(u16::from_le_bytes([s[0], s[1]]), 10);
    let name = [bus.eeprom(0x0042), bus.eeprom(0x0046), bus.eeprom(0x004A)].concat();
    assert_eq!(usize::from(name[1]), EMULATED_ESC_NAME.len());
    assert_eq!(&name[2..2 + EMULATED_ESC_NAME.len()], EMULATED_ESC_NAME.as_bytes());
    // Unwritten words read as an erased EEPROM.
    assert_eq!(bus.eeprom(0x3000), [0xFF; 8]);
    // Writing needs the write enable bit.
    let mut req = vec![0x00, 0x02];
    req.extend_from_slice(&4u32.to_le_bytes());
    bus.fpwr(0x0502, &req);
    assert_eq!(bus.fprd(0x0502, 2).0[1] & 0x20, 0x20, "command error");
}

#[test]
fn al_states_follow_the_sync_manager_configuration() {
    let mut bus = Bus::new(SlaveConfig::default());
    bus.dg(Command::Apwr, 0, 0x0010, &STATION.to_le_bytes());
    assert_eq!(bus.al(AlState::SafeOp), (AlState::Init, 0x0011), "INIT → SAFE-OP");
    assert_eq!(bus.al(AlState::PreOp), (AlState::Init, 0x0011), "an error needs its acknowledge");
    bus.fpwr(0x0120, &[0x11, 0]);
    assert_eq!(bus.al(AlState::PreOp), (AlState::Init, 0x0016), "no mailbox configured");
    bus.fpwr(0x0120, &[0x11, 0]);
    bus.configure_mailboxes();
    assert_eq!(bus.al(AlState::PreOp), (AlState::PreOp, 0));
    assert_eq!(bus.al(AlState::SafeOp), (AlState::PreOp, 0x001D), "outputs not configured");
    bus.fpwr(0x0120, &[0x12, 0]);
    let l = bus.slave.layout();
    for (n, start, len, control) in [(2u16, l.outputs, l.output_len, 0x64u8), (3, l.inputs, l.input_len, 0x20)] {
        let sm = SyncManagerConfig { start, length: len, control, status: 0, activate: 1, pdi_control: 0 };
        bus.fpwr(0x0800 + 8 * n, &sm.encode());
    }
    assert_eq!(bus.al(AlState::SafeOp), (AlState::SafeOp, 0));
    assert_eq!(bus.al(AlState::Op), (AlState::Op, 0));
    assert_eq!(bus.al(AlState::Boot), (AlState::Op, 0x0013));
    bus.fpwr(0x0120, &[0x11, 0]);
    assert_eq!(bus.slave.al_state(), (AlState::Init, None));
}

#[test]
fn mailbox_follows_the_one_buffer_rules() {
    let mut bus = Bus::to_preop();
    assert!(bus.receive().is_none(), "empty: WKC 0");
    let (status, _) = bus.fprd(0x080D, 1);
    assert_eq!(status[0] & 0x08, 0);
    assert_eq!(bus.send(MailboxType::Foe, 1, &msg::foe(FoeOpcode::Write, 0, b"x.bin")), 1);
    assert_eq!(bus.fprd(0x080D, 1).0[0] & 0x08, 0x08, "the answer waits in mailbox in");
    // A read that stops short of the last byte leaves the buffer full.
    let l = bus.slave.layout();
    assert_eq!(bus.fprd(l.mbx_in, 8).1, 1);
    let (h, body) = bus.receive().unwrap();
    assert_eq!((h.mailbox_type, h.counter), (MailboxType::Foe, 1));
    let foe = FoeHeader::parse(&body).unwrap();
    assert_eq!((foe.opcode, foe.value), (FoeOpcode::Ack, 0));
    assert!(bus.receive().is_none(), "read to the last byte: empty again");
    // Writing into mailbox in is not the master's.
    assert_eq!(bus.fpwr(l.mbx_in, &[0; 8]), 0);

    assert_eq!(bus.send(MailboxType::Foe, 2, &msg::foe(FoeOpcode::Data, 1, b"hello")), 1);
    let (h, _) = bus.receive().unwrap();
    assert_eq!(h.counter, 2);
    assert_eq!(bus.slave.log().files, vec![("x.bin".to_string(), b"hello".to_vec())]);

    // Repeat request: the last message comes back, and the PDI acknowledges.
    let (act, _) = bus.fprd(0x080E, 2);
    assert_eq!(bus.fpwr(0x080E, &[act[0] ^ 0x02]), 1);
    assert_eq!(bus.fprd(0x080F, 1).0[0] & 0x02, 0x02);
    let (h, _) = bus.receive().unwrap();
    assert_eq!(h.counter, 2, "the same message");
    assert_eq!(bus.slave.stats().repeats, 1);
}

#[test]
fn coe_serves_the_mapping_objects() {
    let mut bus = Bus::to_preop();
    let upload = SdoCommand::InitiateUploadRequest { complete_access: false };
    let read = |bus: &mut Bus, index: u16, sub: u8, counter: u8| {
        bus.send(MailboxType::Coe, counter, &msg::sdo(CoeService::SdoRequest, upload, index, sub, 0, &[]));
        let (_, body) = bus.receive().unwrap();
        SdoHeader::parse(&body[CoeHeader::LEN..]).unwrap().data
    };
    assert_eq!(read(&mut bus, 0x1C12, 0, 1), 1);
    assert_eq!(read(&mut bus, 0x1C12, 1, 2), 0x1600);
    assert_eq!(read(&mut bus, 0x1600, 0, 3), 1);
    assert_eq!(read(&mut bus, 0x1600, 1, 4), 0x7000_0120); // 0x7000:01, 32 bits
    assert_eq!(read(&mut bus, 0x1A00, 1, 5), 0x6000_0120);
    assert_eq!(read(&mut bus, 0x1018, 0, 6), 4);
}

#[test]
fn logical_exchange_counts_like_an_esc_and_echoes_the_outputs() {
    let mut bus = Bus::to_preop();
    let l = bus.slave.layout();
    let fmmu = |logical: u32, len: u16, phys: u16, kind: u8| {
        let mut f = [0u8; 16];
        f[0..4].copy_from_slice(&logical.to_le_bytes());
        f[4..6].copy_from_slice(&len.to_le_bytes());
        f[7] = 7;
        f[8..10].copy_from_slice(&phys.to_le_bytes());
        f[11] = kind;
        f[12] = 1;
        f
    };
    bus.fpwr(0x0600, &fmmu(0x100, 4, l.outputs, 2));
    bus.fpwr(0x0610, &fmmu(0x104, 4, l.inputs, 1));
    let (_, wkc) = bus.raw(Command::Lrw, 0x100, &[1, 2, 3, 4, 0, 0, 0, 0]);
    assert_eq!(wkc, 3);
    let (data, wkc) = bus.raw(Command::Lrw, 0x100, &[5, 6, 7, 8, 0, 0, 0, 0]);
    assert_eq!(wkc, 3);
    assert_eq!(&data[4..], &[1, 2, 3, 4], "inputs echo the previous outputs");
    assert_eq!(bus.raw(Command::Lrd, 0x104, &[0; 4]), (vec![5, 6, 7, 8], 1));
    assert_eq!(bus.raw(Command::Lwr, 0x100, &[0; 4]).1, 2);
    assert_eq!(bus.raw(Command::Lrw, 0x200, &[0; 4]).1, 0, "no FMMU there");
}

#[test]
fn eoe_send_starts_after_preop_and_restarts_in_init() {
    let mut cfg = SlaveConfig { eoe_start_after_ns: 50_000, ..SlaveConfig::default() };
    cfg.responder.eoe_send = true;
    cfg.responder.upload = vec![7; 300];
    let mut bus = Bus::new(cfg);
    bus.dg(Command::Apwr, 0, 0x0010, &STATION.to_le_bytes());
    bus.configure_mailboxes();
    bus.al(AlState::PreOp);
    assert!(bus.receive().is_none(), "not yet");
    bus.t += 100_000;
    let (h, _) = bus.receive().unwrap();
    assert_eq!(h.mailbox_type, MailboxType::Eoe);
    bus.fpwr(0x0120, &[0x01, 0]);
    bus.configure_mailboxes();
    bus.al(AlState::PreOp);
    assert!(bus.receive().is_none(), "restarted: waits again");
}

#[test]
fn layouts_are_checked() {
    assert!(EmulatedSlave::new(SlaveConfig { mailbox_bytes: 16, ..SlaveConfig::default() }).is_err());
    assert!(EmulatedSlave::new(SlaveConfig { output_bytes: 2000, ..SlaveConfig::default() }).is_err());
    let s =
        EmulatedSlave::new(SlaveConfig { mailbox_bytes: 1024, output_bytes: 10, ..SlaveConfig::default() }).unwrap();
    let l = s.layout();
    assert_eq!((l.mbx_out, l.mbx_in, l.outputs, l.inputs), (0x1000, 0x1400, 0x1800, 0x1810));
}
