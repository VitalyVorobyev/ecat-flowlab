use super::*;

const ESI: &str = include_str!("../tests/fixtures/flowlab-demo.esi.xml");
const ENI: &str = include_str!("../tests/fixtures/flowlab-demo.eni.xml");

#[test]
fn esi_reads_identity_sync_managers_pdos_and_mailbox() {
    let f = parse_esi(ESI).unwrap();
    assert_eq!((f.vendor_id, f.vendor_name.as_str()), (0xFFF, "FlowLab Test Devices"));
    assert_eq!(f.devices.len(), 2);
    let di = &f.devices[0];
    assert_eq!((di.type_name.as_str(), di.product_code, di.revision), ("FL-DI16", 0x0001_0001, 0x0002_0000));
    assert!(di.mailbox.is_none() && !di.dc);
    assert_eq!((di.default_outputs_bits, di.default_inputs_bits), (0, 16));

    let bulk = &f.devices[1];
    assert_eq!(bulk.name, "FL-BULK bulk endpoint", "first <Name> wins");
    assert_eq!(bulk.sync_managers.len(), 4);
    assert_eq!(bulk.sm_size("MBoxOut"), Some(512));
    assert_eq!(bulk.sync_managers[1].start, Some(0x1200));
    assert_eq!(bulk.sync_managers[0].control, Some(0x26));
    // Default assignment: only PDOs with an Sm attribute; padding entries count.
    assert_eq!((bulk.default_outputs_bits, bulk.default_inputs_bits), (64, 64));
    assert_eq!(bulk.rx_pdos.len(), 2);
    assert!(bulk.rx_pdos[1].sm.is_none());
    let m = bulk.mailbox.as_ref().unwrap();
    assert!(m.coe && m.foe && m.eoe && !m.soe && m.coe_sdo_info && m.coe_complete_access);
    assert!(m.supports("eoe") && !m.supports("VoE"));
    assert!(bulk.dc);

    let p = PlanSlave::from_device(bulk);
    assert_eq!((p.outputs_bytes, p.inputs_bytes, p.mailbox_out_bytes, p.mailbox_in_bytes), (8, 8, 512, 512));
    assert_eq!(p.max_mailbox_bytes, Some(1024));
    assert_eq!(p.supports.as_deref(), Some(&["CoE".to_owned(), "FoE".into(), "EoE".into()][..]));
}

#[test]
fn eni_reads_slaves_mailboxes_and_expected_wkc() {
    let n = parse_eni(ENI).unwrap();
    assert_eq!(n.master.as_deref(), Some("Device 1 (EtherCAT)"));
    assert_eq!(n.cycle_time_ns, Some(1_000_000));
    assert_eq!(n.slaves.len(), 2);
    assert_eq!((n.slaves[0].station, n.slaves[0].position), (1001, Some(0)));
    let bulk = n.slave(1002).unwrap();
    assert_eq!(bulk.position, Some(1), "AutoIncAddr 65535 = position 1");
    assert_eq!((bulk.vendor_id, bulk.product_code), (Some(0xFFF), Some(0x0002_0001)));
    assert_eq!((bulk.outputs_bits, bulk.inputs_bits), (64, 64));
    let m = bulk.mailbox.as_ref().unwrap();
    assert_eq!((m.out_start, m.out_len, m.in_start, m.in_len), (4096, 512, 4608, 512));
    assert_eq!(m.protocols, ["CoE", "FoE", "EoE"]);
    let cmds = &n.cyclic[0].cmds;
    assert_eq!(cmds.len(), 3);
    assert_eq!(n.expected_wkc(14, 1001 | 0x0910 << 16, 8), Some(2));
    assert_eq!(n.expected_wkc(12, 0x0001_0000, 20), Some(4), "length from <Data>");
    assert_eq!(n.expected_wkc(7, 0x0130 << 16, 2), Some(2));
    assert_eq!(n.expected_wkc(12, 0x0001_0000, 21), None);
}

#[test]
fn eni_round_trips_through_the_writer() {
    let n = parse_eni(ENI).unwrap();
    let again = parse_eni(&write_eni(&n)).unwrap();
    assert_eq!(again, n);
}

#[test]
fn malformed_files_are_errors_not_panics() {
    assert!(matches!(parse_esi("<EtherCATInfo"), Err(DeviceError::Xml(_))));
    assert!(matches!(parse_esi("<Other/>"), Err(DeviceError::Format(_))));
    assert!(matches!(parse_eni("<EtherCATConfig/>"), Err(DeviceError::Format(_))));
    let dtd = "<?xml version=\"1.0\"?><!DOCTYPE x [<!ENTITY a \"aaaa\">]><EtherCATConfig/>";
    assert!(parse_eni(dtd).is_err(), "DTDs are rejected");
}

#[test]
fn plan_layout_matches_hand_arithmetic() {
    let r = plan(&PlanSpec::example());
    // Image: 2+2+12+16+12+16+8+8 = 76 bytes. Frame: 14 + 2 + (12+8 FRMW) + (12+76 LRW) + (12+2 BRD) = 138.
    assert_eq!((r.image_out_bytes, r.image_in_bytes), (34, 42));
    assert_eq!(r.frames.len(), 1);
    assert_eq!(r.frames[0].frame_bytes, 138);
    assert_eq!(r.frames[0].wire_bytes, 162);
    assert!((r.cyclic_wire_ns - 12_960.0).abs() < 1e-6, "162 B at 100 Mbit/s = 12.96 µs");
    assert!((r.ring_delay_ns - 6.0 * 700.0).abs() < 1e-6);
    assert!((r.cyclic_done_ns - (12_960.0 + 4_200.0)).abs() < 1e-6);
    assert!(r.checks.iter().all(|c| c.severity != "defect"), "{:?}", r.checks);
    // Largest mailbox pair: 14 + 2 + (12+512) * 2 = 1064 B → 1088 wire B = 87.04 µs + ring.
    assert!((r.mailbox_frame_ns.unwrap() - 87_040.0).abs() < 1e-6);
    let per: f64 = 87_040.0 + 4_200.0;
    assert_eq!(r.mailbox_frames_per_cycle, Some(((1e6 - 17_160.0) / per).floor() as u32));
}

#[test]
fn plan_checks_cycle_ram_and_capabilities() {
    let mut s = PlanSpec::example();
    s.cycle_time_us = 10;
    s.slaves[5].dpram_bytes = 1000;
    s.slaves[5].supports = Some(vec!["CoE".into(), "FoE".into()]);
    s.slaves[1].requires = vec!["FoE".into()];
    let r = plan(&s);
    let has = |subject: &str, text: &str| {
        r.checks.iter().any(|c| c.severity == "defect" && c.subject == subject && c.message.contains(text))
    };
    assert!(has("cycle", "more than"));
    assert!(has("bulk-endpoint", "ESC RAM"));
    assert!(has("bulk-endpoint", "EoE"));
    assert!(has("DI-16", "needs a mailbox"));

    s.cycle_time_us = 22;
    let r = plan(&s);
    assert!(r.checks.iter().any(|c| c.severity == "warn" && c.subject == "cycle"), "{:?}", r.checks);
}

#[test]
fn large_images_span_frames() {
    let mut s = PlanSpec::example();
    s.slaves[5].outputs_bytes = 1500;
    s.slaves[5].inputs_bytes = 1500;
    s.slaves[5].dpram_bytes = 16384;
    let r = plan(&s);
    let lrw: u32 =
        r.frames.iter().flat_map(|f| &f.datagrams).filter(|d| d.command == "LRW").map(|d| d.data_bytes).sum();
    assert_eq!(lrw, r.image_out_bytes + r.image_in_bytes);
    assert_eq!(r.frames.len(), 3);
    assert!(r.frames.iter().all(|f| f.frame_bytes <= 1514));
}

#[test]
fn plan_schema_is_draft_2020_12() {
    assert!(plan_schema()["$schema"].as_str().unwrap().contains("2020-12"));
}
