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
fn sync_manager_golden_and_slicing() {
    // SM0: start 0x1000, 128 bytes, control 0x26 (mailbox, ECAT writes, interrupt), enabled.
    let sm0 = [0x00, 0x10, 0x80, 0x00, 0x26, 0x00, 0x01, 0x00];
    let c = SyncManagerConfig::parse(&sm0).unwrap();
    assert_eq!((c.start, c.length), (0x1000, 128));
    assert_eq!(c.mode(), SmMode::Mailbox);
    assert_eq!(c.direction(), SmDirection::EcatWrite);
    assert!(c.enabled());
    assert_eq!(c.encode(), sm0);
    // A 16-byte write at 0x0800 holds SM0 and SM1; one at 0x0804 holds neither fully but SM1.
    let mut two = sm0.to_vec();
    two.extend_from_slice(&[0x80, 0x10, 0x80, 0x00, 0x22, 0x00, 0x01, 0x00]);
    let sms: Vec<_> = sync_managers_in(0x0800, &two).collect();
    assert_eq!(sms.len(), 2);
    assert_eq!(sms[1].0, 1);
    assert_eq!(sms[1].1.direction(), SmDirection::EcatRead);
    let shifted: Vec<_> = sync_managers_in(0x0804, &two[..12]).collect();
    assert_eq!(shifted.len(), 1);
    assert_eq!(shifted[0].0, 1);
}

#[test]
fn sdo_command_golden() {
    use SdoCommand::*;
    assert_eq!(SdoCommand::decode(0x40, true), InitiateUploadRequest { complete_access: false });
    assert_eq!(SdoCommand::decode(0x50, true), InitiateUploadRequest { complete_access: true });
    assert_eq!(
        SdoCommand::decode(0x43, false),
        InitiateUploadResponse { complete_access: false, expedited: true, size_indicated: true, unused: 0 }
    );
    assert_eq!(
        SdoCommand::decode(0x2F, true),
        InitiateDownloadRequest { complete_access: false, expedited: true, size_indicated: true, unused: 3 }
    );
    assert_eq!(
        SdoCommand::decode(0x21, true),
        InitiateDownloadRequest { complete_access: false, expedited: false, size_indicated: true, unused: 0 }
    );
    assert_eq!(SdoCommand::decode(0x60, false), InitiateDownloadResponse);
    assert_eq!(SdoCommand::decode(0x60, true), UploadSegmentRequest { toggle: false });
    assert_eq!(SdoCommand::decode(0x70, true), UploadSegmentRequest { toggle: true });
    assert_eq!(SdoCommand::decode(0x0B, false), UploadSegmentResponse { toggle: false, unused: 5, last: true });
    assert_eq!(SdoCommand::decode(0x11, true), DownloadSegmentRequest { toggle: true, unused: 0, last: true });
    assert_eq!(SdoCommand::decode(0x30, false), DownloadSegmentResponse { toggle: true });
    assert_eq!(SdoCommand::decode(0x80, false), Abort);
    assert_eq!(sdo_abort_text(0x0602_0000), Some("Object does not exist in the object dictionary"));
    // Segment data: 7-byte segments declare unused bytes; longer ones use the mailbox length.
    assert_eq!(segment_data_len(10, 5), 2);
    assert_eq!(segment_data_len(122, 0), 119);
}

#[test]
fn mailbox_protocol_headers_golden() {
    // EoE: fragment, last, fragment 0, complete size 3 blocks, frame number 5.
    let eoe = EoeHeader::parse(&[0x00, 0x01, 0xC0, 0x50]).unwrap();
    assert_eq!(eoe.frame_type, EoeType::Fragment);
    assert!(eoe.last_fragment && !eoe.time_appended);
    assert_eq!((eoe.fragment, eoe.offset_blocks, eoe.frame_no), (0, 3, 5));
    // FoE DATA packet 1.
    let foe = FoeHeader::parse(&[0x03, 0x00, 0x01, 0x00, 0x00, 0x00]).unwrap();
    assert_eq!((foe.opcode, foe.value), (FoeOpcode::Data, 1));
    assert_eq!(foe_error_text(0x8001), Some("Not found"));
    // SoE read request of IDN 1, value element.
    let soe = SoeHeader::parse(&[0x01, 0x40, 0x01, 0x00]).unwrap();
    assert_eq!((soe.opcode, soe.elements, soe.idn, soe.is_request()), (1, 0x40, 1, true));
    // Emergency 0x8130 (life guard), error register 0x11.
    let e = CoeEmergency::parse(&[0x30, 0x81, 0x11, 1, 2, 3, 4, 5]).unwrap();
    assert_eq!((e.error_code, e.error_register, e.data), (0x8130, 0x11, [1, 2, 3, 4, 5]));
    let err = MailboxErrorReply::parse(&[0x01, 0x00, 0x02, 0x00]).unwrap();
    assert_eq!(err.detail_text(), "Mailbox protocol not supported");
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

    #[test]
    fn sdo_command_decode_encode_is_canonical(byte in any::<u8>(), request in any::<bool>()) {
        let c = SdoCommand::decode(byte, request);
        prop_assert_eq!(SdoCommand::decode(c.encode(), request), c);
    }

    #[test]
    fn mailbox_headers_round_trip(
        len in any::<u16>(), addr in any::<u16>(), ch in 0u8..64, prio in 0u8..4, ty in 0u8..16, ctr in 0u8..8,
        start in any::<u16>(), sm_len in any::<u16>(), ctrl in any::<u8>(), st in any::<u8>(), act in any::<u8>(), pdi in any::<u8>(),
        v32 in any::<u32>(), v16 in any::<u16>(), b8 in any::<u8>(), flags in any::<[bool; 3]>(),
        f6a in 0u8..64, f6b in 0u8..64, f4 in 0u8..16, f3 in 0u8..8, data5 in any::<[u8; 5]>(),
    ) {
        let mh = MailboxHeader { length: len, address: addr, channel: ch, priority: prio, mailbox_type: MailboxType::from_code(ty), counter: ctr };
        prop_assert_eq!(MailboxHeader::parse(&mh.encode()).unwrap(), mh);
        let sm = SyncManagerConfig { start, length: sm_len, control: ctrl, status: st, activate: act, pdi_control: pdi };
        prop_assert_eq!(SyncManagerConfig::parse(&sm.encode()).unwrap(), sm);
        let foe = FoeHeader { opcode: FoeOpcode::from_code(b8), value: v32 };
        prop_assert_eq!(FoeHeader::parse(&foe.encode()).unwrap(), foe);
        let eoe = EoeHeader {
            frame_type: EoeType::from_code(f4), port: b8 & 0x0F, last_fragment: flags[0], time_appended: flags[1],
            time_request: flags[2], fragment: f6a, offset_blocks: f6b, frame_no: f4,
        };
        prop_assert_eq!(EoeHeader::parse(&eoe.encode()).unwrap(), eoe);
        let soe = SoeHeader { opcode: f3, incomplete: flags[0], error: flags[1], drive: b8 >> 5, elements: b8, idn: v16 };
        prop_assert_eq!(SoeHeader::parse(&soe.encode()).unwrap(), soe);
        let voe = VoeHeader { vendor_id: v32, vendor_type: v16 };
        prop_assert_eq!(VoeHeader::parse(&voe.encode()).unwrap(), voe);
        let em = CoeEmergency { error_code: v16, error_register: b8, data: data5 };
        prop_assert_eq!(CoeEmergency::parse(&em.encode()).unwrap(), em);
        let info = SdoInfoHeader { opcode: b8 & 0x7F, incomplete: flags[0], fragments_left: v16 };
        prop_assert_eq!(SdoInfoHeader::parse(&info.encode()).unwrap(), info);
        let err = MailboxErrorReply { service: v16, detail: len };
        prop_assert_eq!(MailboxErrorReply::parse(&err.encode()).unwrap(), err);
    }

    #[test]
    fn sm_slicing_never_panics(ado in 0x07F0u16..0x0890, data in prop::collection::vec(any::<u8>(), 0..160)) {
        for (n, _) in sync_managers_in(ado, &data) { prop_assert!(n < 16); }
    }
}

#[test]
fn slaves_increment_adp_of_position_and_broadcast_datagrams() {
    // APRD to position −1 (ADP 0xFFFF) after three slaves: 0x0002; ADO untouched.
    assert_eq!(Command::Aprd.address_after(0x0130_FFFF, 3), 0x0130_0002);
    assert_eq!(Command::Brd.address_after(0x0130_0000, 6), 0x0130_0006);
    assert_eq!(Command::Fprd.address_after(0x0130_1001, 6), 0x0130_1001);
    assert_eq!(Command::Lrw.address_after(0x0001_0000, 6), 0x0001_0000);
    for c in [Command::Aprd, Command::Brd, Command::Fprd, Command::Lrw] {
        for a in [0u32, 0x0130_FFFF, 0x1234_5678] {
            assert_eq!(c.stable_address(c.address_after(a, 5)), c.stable_address(a), "{c:?} {a:#x}");
        }
    }
}
