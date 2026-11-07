use super::*;
use proptest::prelude::*;

const MASTER: MacAddr = MacAddr([0x00, 0x01, 0x05, 0x00, 0x00, 0x01]);
const BCAST: MacAddr = MacAddr([0xFF; 6]);

/// Hand-assembled from the ETG.1000.4 layout: BRD of AL status (0x0130), 2 bytes, WKC 3.
const GOLDEN_BRD: [u8; 30] = [
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, // dst
    0x02, 0x01, 0x05, 0x00, 0x00, 0x01, // src (locally administered: returning frame)
    0x88, 0xa4, // EtherType
    0x0e, 0x10, // EtherCAT header: length 14, type 1
    0x07, 0x01, // BRD, index 1
    0x00, 0x00, 0x30, 0x01, // ADP 0, ADO 0x0130
    0x02, 0x00, // len 2, last datagram
    0x00, 0x00, // IRQ
    0x08, 0x00, // data: AL status OP
    0x03, 0x00, // WKC 3
];

#[test]
fn golden_brd_parses() {
    let f = parse_frame(&GOLDEN_BRD).unwrap().unwrap();
    assert_eq!(f.encapsulation, Encapsulation::Ethernet);
    assert!(f.ethernet.src.locally_administered());
    assert_eq!(f.pdu.header.length, 14);
    assert_eq!(f.pdu.header.pdu_type, EcatPduType::Datagrams);
    let dgs: Vec<_> = f.pdu.datagrams().collect::<Result<_, _>>().unwrap();
    assert_eq!(dgs.len(), 1);
    let d = dgs[0];
    assert_eq!(d.command, Command::Brd);
    assert_eq!(d.index, 1);
    assert_eq!(d.address, DatagramAddress::Broadcast { adp: 0, ado: 0x0130 });
    assert_eq!(d.data, &[0x08, 0x00]);
    assert_eq!(d.wkc, 3);
    assert!(!d.more);
    assert_eq!(AlStatus::from_byte(d.data[0]).state, AlState::Op);
    assert_eq!(register_info(0x0130).unwrap().name, "AL status");
}

#[test]
fn builder_reproduces_golden_bytes() {
    let mut b = FrameBuilder::new(BCAST, MASTER.with_local_bit());
    b.push(DatagramSpec::new(Command::Brd, 1, 0x0130_0000, &[0x08, 0x00]).with_wkc(3)).unwrap();
    let bytes = b.finish();
    assert_eq!(bytes.len(), 60, "padded to the Ethernet minimum");
    assert_eq!(&bytes[..30], &GOLDEN_BRD);
    assert!(bytes[30..].iter().all(|&b| b == 0));
}

#[test]
fn more_flag_chains_datagrams() {
    let mut b = FrameBuilder::new(BCAST, MASTER);
    b.push(DatagramSpec::new(Command::Lrw, 0, 0x0001_0000, &[1, 2, 3, 4])).unwrap();
    b.push(DatagramSpec::new(Command::Fprd, 1, 0x0130_1001, &[0, 0])).unwrap();
    let bytes = b.finish();
    let f = parse_frame(&bytes).unwrap().unwrap();
    let dgs: Vec<_> = f.pdu.datagrams().map(Result::unwrap).collect();
    assert_eq!(dgs.len(), 2);
    assert!(dgs[0].more && !dgs[1].more);
    assert_eq!(dgs[1].address, DatagramAddress::Fixed { station: 0x1001, ado: 0x0130 });
    assert_eq!(dgs[1].offset, 16);
}

#[test]
fn position_address_decodes_ring_position() {
    let a = DatagramAddress::new(Command::Aprd, 0x0010_FFFE);
    assert_eq!(a.position(), Some(2));
    assert_eq!(a.ado(), Some(0x0010));
}

#[test]
fn non_ethercat_frames_are_skipped() {
    let mut f = GOLDEN_BRD;
    f[12] = 0x08;
    f[13] = 0x06; // ARP
    assert!(parse_frame(&f).unwrap().is_none());
}

#[test]
fn truncated_datagram_is_an_error_not_a_panic() {
    let cut = &GOLDEN_BRD[..GOLDEN_BRD.len() - 1];
    assert!(matches!(parse_frame(cut), Err(WireError::LengthMismatch { .. })));
    // Header claims fewer bytes than the datagram needs.
    let mut f = GOLDEN_BRD;
    f[14] = 0x0c;
    let pdu = parse_frame(&f).unwrap().unwrap().pdu;
    assert!(matches!(pdu.datagrams().next(), Some(Err(WireError::Truncated { .. }))));
}

#[test]
fn udp_encapsulation() {
    let ecat = &GOLDEN_BRD[14..];
    let mut f = Vec::new();
    f.extend_from_slice(&GOLDEN_BRD[..12]);
    f.extend_from_slice(&[0x08, 0x00]);
    let ip_total = 20 + 8 + ecat.len();
    f.extend_from_slice(&[0x45, 0, (ip_total >> 8) as u8, ip_total as u8, 0, 0, 0, 0, 64, 17, 0, 0]);
    f.extend_from_slice(&[10, 0, 0, 1, 10, 0, 0, 2]);
    f.extend_from_slice(&[0x88, 0xa4, 0x88, 0xa4, 0, (8 + ecat.len()) as u8, 0, 0]);
    f.extend_from_slice(ecat);
    let p = parse_frame(&f).unwrap().unwrap();
    assert_eq!(p.encapsulation, Encapsulation::Udp);
    assert_eq!(p.pdu.datagrams().count(), 1);
}

#[test]
fn mailbox_and_sdo_round_trip() {
    let h =
        MailboxHeader { length: 10, address: 0, channel: 0, priority: 0, mailbox_type: MailboxType::Coe, counter: 3 };
    assert_eq!(MailboxHeader::parse(&h.encode()).unwrap(), h);
    let c = CoeHeader { number: 0, service: CoeService::SdoRequest };
    assert_eq!(CoeHeader::parse(&c.encode()).unwrap(), c);
    let s = SdoHeader { command: 0x40, index: 0x1018, subindex: 1, data: 0 };
    let back = SdoHeader::parse(&s.encode()).unwrap();
    assert_eq!(back, s);
    assert_eq!(back.specifier(), 2);
}

#[test]
fn wire_bytes_includes_overhead_and_padding() {
    assert_eq!(wire_bytes(30), 60 + 24);
    assert_eq!(wire_bytes(1514), 1514 + 24);
}

fn arb_command() -> impl Strategy<Value = Command> {
    (0u8..=14).prop_map(Command::from_code)
}

proptest! {
    #[test]
    fn encode_parse_round_trip(
        dgs in prop::collection::vec(
            (arb_command(), any::<u8>(), any::<u32>(), prop::collection::vec(any::<u8>(), 0..200), any::<u16>(), any::<u16>(), any::<bool>()),
            1..6,
        ),
        src_local in any::<bool>(),
    ) {
        let src = if src_local { MASTER.with_local_bit() } else { MASTER };
        let mut b = FrameBuilder::new(BCAST, src);
        for (cmd, idx, addr, data, irq, wkc, circ) in &dgs {
            b.push(DatagramSpec { command: *cmd, index: *idx, address: *addr, data, irq: *irq, wkc: *wkc, circulating: *circ }).unwrap();
        }
        let bytes = b.finish();
        let f = parse_frame(&bytes).unwrap().unwrap();
        prop_assert_eq!(f.ethernet.src, src);
        let parsed: Vec<_> = f.pdu.datagrams().collect::<Result<_, _>>().unwrap();
        prop_assert_eq!(parsed.len(), dgs.len());
        for (i, (p, (cmd, idx, addr, data, irq, wkc, circ))) in parsed.iter().zip(&dgs).enumerate() {
            prop_assert_eq!(p.command, *cmd);
            prop_assert_eq!(p.index, *idx);
            prop_assert_eq!(p.address.raw(), *addr);
            prop_assert_eq!(p.data, data.as_slice());
            prop_assert_eq!(p.irq, *irq);
            prop_assert_eq!(p.wkc, *wkc);
            prop_assert_eq!(p.circulating, *circ);
            prop_assert_eq!(p.more, i + 1 < dgs.len());
        }
    }

    #[test]
    fn parser_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..400)) {
        if let Ok(Some(f)) = parse_frame(&bytes) {
            for d in f.pdu.datagrams() { let _ = d; }
        }
    }

    #[test]
    fn parser_never_panics_on_ethercat_type(mut bytes in prop::collection::vec(any::<u8>(), 16..400)) {
        bytes[12] = 0x88; bytes[13] = 0xa4;
        if let Ok(Some(f)) = parse_frame(&bytes) {
            for d in f.pdu.datagrams() { let _ = d; }
        }
    }
}
