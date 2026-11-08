//! End-to-end tests of the engine against the virtual slave.

use futures_lite::future::block_on;

use flowlab_mailbox::MailboxPolicy;
use flowlab_mailbox::msg::udp_payload;

use super::*;

fn pattern(n: usize, seed: u8) -> Vec<u8> {
    (0..n).map(|i| (i as u32).wrapping_mul(2_654_435_761).to_le_bytes()[1] ^ seed).collect()
}

fn job(slave: &VirtualSlave, protocol: TransferProtocol, payload: Payload) -> TransferJob {
    TransferJob { slave: "virtual".into(), mailbox: slave.layout(), protocol, payload, payload_name: "pattern".into() }
}

fn run(slave: &VirtualSlave, j: TransferJob, o: &TransferOptions) -> MeasuredTransfer {
    block_on(run_transfer(slave, j, o)).expect("transfer runs")
}

/// Let the slave's application catch up (it takes the last message after `processing_ns`).
fn settle(slave: &VirtualSlave) {
    block_on(slave.sleep_until_ns(slave.now_ns() + 10_000_000));
}

fn receive(expected: &[u8]) -> Payload {
    Payload::Receive {
        expected_bytes: Some(expected.len() as u64),
        expected_sha256: Some(
            sha2::Digest::finalize(sha2::Sha256::new_with_prefix(expected))
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
        ),
    }
}

use sha2::Digest as _;

#[test]
fn foe_write_is_byte_identical_at_every_boundary() {
    // 128-byte mailbox: 116 data bytes per packet.
    for len in [0usize, 1, 115, 116, 117, 232, 1000, 10_000] {
        let slave = VirtualSlave::new(VirtualSlaveConfig::default());
        let data = pattern(len, 1);
        let r =
            run(&slave, job(&slave, TransferProtocol::foe(), Payload::Send(data.clone())), &TransferOptions::default());
        assert!(r.completed, "{len}: {:?}", r.error);
        let log = slave.log();
        assert_eq!(log.files.len(), 1, "{len}");
        assert_eq!(log.files[0].0, "flowlab.bin");
        assert_eq!(log.files[0].1, data, "{len}");
        assert_eq!(r.delivered_bytes, len as u64);
        assert_eq!(r.payload_bytes, len as u64);
        // WRQ + DATA packets (a zero-length last one when the file fills the last packet).
        let packets = len / 116 + 1;
        assert_eq!(r.messages_to_slave, 1 + packets as u64, "{len}");
        assert_eq!(r.messages_from_slave, 1 + packets as u64, "{len}");
        assert_eq!(r.meta.source, flowlab_core::Provenance::Measured);
        assert!(r.exchange_ns.unwrap() <= r.duration_ns.unwrap());
    }
}

#[test]
fn foe_read_and_sdo_upload_verify_the_received_bytes() {
    for len in [0usize, 3, 4, 5, 112, 113, 119, 120, 5000] {
        let data = pattern(len, 7);
        let cfg = VirtualSlaveConfig { upload: data.clone(), ..Default::default() };
        let slave = VirtualSlave::new(cfg.clone());
        let r = run(&slave, job(&slave, TransferProtocol::foe(), receive(&data)), &TransferOptions::default());
        assert!(r.completed, "foe read {len}: {:?}", r.error);
        assert_eq!(r.verified, Some(true), "foe read {len}");
        assert_eq!(r.delivered_bytes, len as u64);

        let slave = VirtualSlave::new(cfg);
        let r = run(&slave, job(&slave, TransferProtocol::coe(), receive(&data)), &TransferOptions::default());
        assert!(r.completed, "sdo upload {len}: {:?}", r.error);
        assert_eq!(r.verified, Some(true), "sdo upload {len}");
        assert_eq!(r.payload_bytes, len as u64, "the size comes from the slave");
    }
}

#[test]
fn segmented_sdo_download_arrives_whole() {
    for len in [0usize, 1, 7, 112, 113, 119, 120, 121, 3000] {
        let slave = VirtualSlave::new(VirtualSlaveConfig::default());
        let data = pattern(len, 3);
        let r =
            run(&slave, job(&slave, TransferProtocol::coe(), Payload::Send(data.clone())), &TransferOptions::default());
        assert!(r.completed, "{len}: {:?}", r.error);
        assert_eq!(slave.log().objects, vec![data], "{len}");
        // Initiate (112 bytes) + segments of up to 119 bytes, each answered.
        let segments = len.saturating_sub(112).div_ceil(119) as u64;
        assert_eq!(r.messages_to_slave, 1 + segments, "{len}");
        assert_eq!(r.messages_from_slave, 1 + segments, "{len}");
    }
}

#[test]
fn sdo_abort_ends_the_transfer_with_its_text() {
    let slave = VirtualSlave::new(VirtualSlaveConfig::default());
    let p = TransferProtocol::Coe { index: 0x6000, subindex: 1 };
    let r = run(&slave, job(&slave, p, Payload::Send(vec![1, 2, 3])), &TransferOptions::default());
    assert!(!r.completed);
    let e = r.error.unwrap();
    assert!(e.contains("0x06020000") && e.contains("does not exist"), "{e}");
}

#[test]
fn eoe_frames_reassemble_at_the_slave_and_the_master() {
    let data = pattern(10_000, 5);
    let slave = VirtualSlave::new(VirtualSlaveConfig::default());
    let r = run(&slave, job(&slave, TransferProtocol::eoe(), Payload::Send(data.clone())), &TransferOptions::default());
    assert!(r.completed, "{:?}", r.error);
    settle(&slave);
    let log = slave.log();
    assert_eq!(log.eoe_broken, 0);
    let a = EoeAddresses::default();
    let got: Vec<u8> =
        log.eoe_frames.iter().flat_map(|f| udp_payload(f, a.destination_port).unwrap().to_vec()).collect();
    assert_eq!(got, data);
    assert_eq!(log.eoe_frames.len(), 10_000usize.div_ceil(1472));
    assert_eq!(r.messages_from_slave, 0, "EoE sends without acknowledgements");

    // Slave → master: the slave streams UDP datagrams in EoE fragments.
    let cfg = VirtualSlaveConfig { upload: data.clone(), eoe_send: true, eoe_mtu: 576, ..Default::default() };
    let slave = VirtualSlave::new(cfg);
    let r = run(&slave, job(&slave, TransferProtocol::eoe(), receive(&data)), &TransferOptions::default());
    assert!(r.completed, "{:?}", r.error);
    assert_eq!(r.verified, Some(true));
}

#[test]
fn eoe_receive_needs_a_byte_count() {
    let slave = VirtualSlave::new(VirtualSlaveConfig::default());
    let p = Payload::Receive { expected_bytes: None, expected_sha256: None };
    let e = block_on(run_transfer(&slave, job(&slave, TransferProtocol::eoe(), p), &TransferOptions::default()));
    assert!(matches!(e, Err(MasterError::Invalid(_))));
}

#[test]
fn a_full_mailbox_rejects_writes_and_an_empty_one_reads_empty() {
    // The slave takes 3 cycles per message: EoE fragments queue up behind it.
    let cfg = VirtualSlaveConfig { processing_ns: 3_000_000, ..Default::default() };
    let slave = VirtualSlave::new(cfg);
    let r =
        run(&slave, job(&slave, TransferProtocol::eoe(), Payload::Send(pattern(2000, 0))), &TransferOptions::default());
    assert!(r.completed);
    assert!(r.rejected_writes >= 2 * (r.messages_to_slave - 1), "{} rejected", r.rejected_writes);

    let cfg = VirtualSlaveConfig { processing_ns: 3_000_000, ..Default::default() };
    let slave = VirtualSlave::new(cfg);
    let r =
        run(&slave, job(&slave, TransferProtocol::foe(), Payload::Send(pattern(500, 0))), &TransferOptions::default());
    assert!(r.completed);
    assert!(r.empty_reads >= 2 * r.messages_from_slave, "{} empty", r.empty_reads);
}

fn foe_with(policy: MailboxPolicy, poll: PollMode) -> MeasuredTransfer {
    let cfg = VirtualSlaveConfig { processing_ns: 20_000, ..Default::default() };
    let slave = VirtualSlave::new(cfg);
    let data = pattern(20_000, 9);
    let o = TransferOptions { policy, poll, ..Default::default() };
    let r = run(&slave, job(&slave, TransferProtocol::foe(), Payload::Send(data.clone())), &o);
    assert!(r.completed, "{:?}", r.error);
    assert_eq!(slave.log().files[0].1, data);
    r
}

#[test]
fn policies_change_access_counts_and_time() {
    let one = foe_with(MailboxPolicy::FixedQuota { accesses_per_cycle: 1 }, PollMode::Read);
    let two = foe_with(MailboxPolicy::FixedQuota { accesses_per_cycle: 2 }, PollMode::Read);
    let rr = foe_with(MailboxPolicy::RoundRobin { slaves_per_cycle: 1 }, PollMode::Read);
    let opp =
        foe_with(MailboxPolicy::Opportunistic { bus_budget_percent: 60.0, turnaround_ns: 10_000 }, PollMode::Read);
    assert_eq!(one.timing.max_accesses_per_cycle, 1);
    assert_eq!(two.timing.max_accesses_per_cycle, 2);
    assert!(opp.timing.max_accesses_per_cycle > 2, "{}", opp.timing.max_accesses_per_cycle);
    // One slave: round robin serves it with a write and a read every cycle, like a quota of 2.
    assert_eq!(rr.timing.cycles, two.timing.cycles);
    assert!(one.timing.cycles > two.timing.cycles, "{} vs {}", one.timing.cycles, two.timing.cycles);
    assert!(two.timing.cycles > 2 * opp.timing.cycles, "{} vs {}", two.timing.cycles, opp.timing.cycles);
    assert!(opp.goodput_bps.unwrap() > two.goodput_bps.unwrap());
    // Opportunistic frames stay within 60 % of the 1 ms cycle.
    assert!(opp.timing.busy.max_ns <= 600_000.0 + 2.0 * 20_000.0, "{}", opp.timing.busy.max_ns);
    assert_eq!(one.policy, "fixed quota: 1 mailbox datagrams per cycle");
}

#[test]
fn status_polling_reads_the_buffer_only_when_full() {
    let direct = foe_with(MailboxPolicy::FixedQuota { accesses_per_cycle: 2 }, PollMode::Read);
    let status = foe_with(MailboxPolicy::FixedQuota { accesses_per_cycle: 2 }, PollMode::Status);
    assert_eq!(direct.status_reads, 0);
    assert!(status.status_reads > 0);
    assert_eq!(status.messages_from_slave, direct.messages_from_slave);
    assert_eq!(status.poll, "status");
}

#[test]
fn lost_frames_are_recovered_with_counters_and_repeat_requests() {
    for every in [5u64, 7, 11, 13] {
        let cfg = VirtualSlaveConfig { lose_every: Some(every), ..Default::default() };
        let data = pattern(5000, 2);
        let slave = VirtualSlave::new(cfg.clone());
        let r =
            run(&slave, job(&slave, TransferProtocol::foe(), Payload::Send(data.clone())), &TransferOptions::default());
        assert!(r.completed, "every {every}: {:?}", r.error);
        assert_eq!(slave.log().files[0].1, data, "every {every}");
        assert!(r.lost_frames > 0);
        assert!(r.repeat_requests > 0, "every {every}");
        assert!(slave.log().repeats > 0);

        let slave = VirtualSlave::new(cfg.clone());
        let r =
            run(&slave, job(&slave, TransferProtocol::coe(), Payload::Send(data.clone())), &TransferOptions::default());
        assert!(r.completed, "sdo every {every}: {:?}", r.error);
        assert_eq!(slave.log().objects, vec![data.clone()], "sdo every {every}");

        let up = VirtualSlaveConfig { upload: data.clone(), ..cfg };
        let slave = VirtualSlave::new(up);
        let r = run(&slave, job(&slave, TransferProtocol::coe(), receive(&data)), &TransferOptions::default());
        assert!(r.completed, "upload every {every}: {:?}", r.error);
        assert_eq!(r.verified, Some(true), "upload every {every}");
    }
}

#[test]
fn a_counter_the_slave_saw_last_is_resent_with_a_new_one() {
    // The slave's last message before the transfer had counter 1, as the engine's first.
    let cfg = VirtualSlaveConfig { previous_counter: 1, ..Default::default() };
    let slave = VirtualSlave::new(cfg);
    let o = TransferOptions { reply_timeout_ns: 5_000_000, ..Default::default() };
    let data = pattern(300, 4);
    let r = run(&slave, job(&slave, TransferProtocol::foe(), Payload::Send(data.clone())), &o);
    assert!(r.completed, "{:?}", r.error);
    assert_eq!(r.resent_messages, 1);
    assert_eq!(slave.log().duplicates_dropped, 1);
    assert_eq!(slave.log().files[0].1, data);
}

#[test]
fn foe_busy_resends_the_packet() {
    let cfg = VirtualSlaveConfig { busy_every: Some(3), ..Default::default() };
    let slave = VirtualSlave::new(cfg);
    let data = pattern(2000, 8);
    let r = run(&slave, job(&slave, TransferProtocol::foe(), Payload::Send(data.clone())), &TransferOptions::default());
    assert!(r.completed, "{:?}", r.error);
    assert!(r.resent_messages >= 5, "{}", r.resent_messages);
    assert_eq!(slave.log().files[0].1, data);
}

#[test]
fn a_silent_slave_fails_after_the_resends() {
    // The slave never answers: wrong station.
    let slave = VirtualSlave::new(VirtualSlaveConfig::default());
    let mut j = job(&slave, TransferProtocol::foe(), Payload::Send(vec![1; 10]));
    j.mailbox.station = 0x2000;
    let o = TransferOptions { reply_timeout_ns: 2_000_000, max_resends: 2, ..Default::default() };
    let r = run(&slave, j, &o);
    assert!(!r.completed);
    assert!(r.error.unwrap().contains("nothing from the slave") || r.rejected_writes > 0);
    let o = TransferOptions { max_duration_ns: 50_000_000, ..Default::default() };
    let mut j = job(&slave, TransferProtocol::foe(), Payload::Send(vec![1; 10]));
    j.mailbox.station = 0x2000;
    let r = run(&slave, j, &o);
    assert!(r.error.unwrap().contains("time limit"));
}

#[test]
fn cycles_keep_their_plan_and_record_the_cyclic_exchange() {
    let cfg = VirtualSlaveConfig { cyclic_ns: Some(30_000), ..Default::default() };
    let slave = VirtualSlave::new(cfg);
    let o = TransferOptions { cycle_time_ns: 500_000, ..Default::default() };
    let r = run(&slave, job(&slave, TransferProtocol::foe(), Payload::Send(pattern(3000, 1))), &o);
    assert!(r.completed);
    let t = &r.timing;
    assert_eq!(t.cycles as usize, r.cycle_records.len());
    assert_eq!(t.skipped_cycles, 0);
    assert_eq!(t.overruns, 0);
    assert_eq!(t.start_late.max_ns, 0.0, "virtual time starts every cycle on plan");
    assert_eq!(t.period.p50_ns, 500_000.0);
    assert_eq!(t.cyclic.p50_ns, 30_000.0);
    assert!(r.cycle_records.iter().all(|c| c.cyclic_wkc == Some(3)));
    // Cycles too short for their work overrun and are skipped.
    let cfg = VirtualSlaveConfig { access_ns: 400_000, ..Default::default() };
    let slave = VirtualSlave::new(cfg);
    let o = TransferOptions { cycle_time_ns: 500_000, ..Default::default() };
    let r = run(&slave, job(&slave, TransferProtocol::foe(), Payload::Send(pattern(1000, 1))), &o);
    assert!(r.completed);
    assert!(r.timing.overruns > 0 && r.timing.skipped_cycles > 0);
}

#[test]
fn stale_messages_are_drained_first() {
    let cfg = VirtualSlaveConfig { upload: pattern(50, 1), ..Default::default() };
    let slave = VirtualSlave::new(cfg);
    // An SDO upload answered but never read.
    let j = job(&slave, TransferProtocol::coe(), Payload::Receive { expected_bytes: None, expected_sha256: None });
    let o = TransferOptions { max_duration_ns: 1, ..Default::default() };
    let _ =
        run(&slave, j.clone(), &TransferOptions { policy: MailboxPolicy::FixedQuota { accesses_per_cycle: 1 }, ..o });
    settle(&slave);
    // The stale response would otherwise be taken as this transfer's.
    let r =
        run(&slave, job(&slave, TransferProtocol::foe(), Payload::Send(pattern(100, 0))), &TransferOptions::default());
    assert!(r.completed, "{:?}", r.error);
    assert_eq!(r.drained_messages, 1);
}

#[test]
fn results_name_the_policy_as_the_simulator_does() {
    let mut spec = flowlab_sim::SimSpec::example();
    spec.transfers.truncate(1);
    spec.capture.enabled = false;
    let p = MailboxPolicy::RoundRobin { slaves_per_cycle: 1 };
    spec.mailbox_policy = p.clone();
    let sim = flowlab_sim::simulate(&spec, None::<Vec<u8>>).unwrap();
    let slave = VirtualSlave::new(VirtualSlaveConfig::default());
    let o = TransferOptions { policy: p, ..TransferOptions::default() };
    let r = run(&slave, job(&slave, TransferProtocol::foe(), Payload::Send(pattern(100, 0))), &o);
    assert_eq!(r.policy, sim.policy);
}

#[test]
fn scan_becomes_an_eni_the_analysis_reads() {
    let meta = flowlab_core::ReportMeta::new(
        flowlab_core::Provenance::Measured,
        flowlab_core::InputRef { name: "eth1".into(), bytes: 0, sha256: String::new() },
        Vec::new(),
    );
    let sm = |index, start, length, mode: &str, direction: &str| ScannedSm {
        index,
        start,
        length,
        control: 0,
        mode: mode.into(),
        direction: direction.into(),
    };
    let scan = ScanResult {
        meta,
        interface: "eth1".into(),
        slaves: vec![ScannedSlave {
            position: 0,
            station: 0x1001,
            alias: 0,
            name: "LAN9252-SOES".into(),
            vendor_id: 0x1337,
            product_code: 0x9252,
            revision: 1,
            serial: 0,
            al_state: "PRE-OP".into(),
            al_status_code: 0,
            mailbox_protocols: vec!["CoE".into(), "FoE".into(), "EoE".into()],
            sync_managers: vec![
                sm(0, 0x1000, 128, "mailbox", "master_to_slave"),
                sm(1, 0x1080, 128, "mailbox", "slave_to_master"),
                sm(2, 0x1100, 4, "buffered", "master_to_slave"),
            ],
            dc_support: "64 bit".into(),
            propagation_delay_ns: 0,
        }],
    };
    let xml = flowlab_devices::write_eni(&scan.network_config());
    let back = flowlab_devices::parse_eni(&xml).unwrap();
    let s = back.slave(0x1001).unwrap();
    assert_eq!(s.name, "LAN9252-SOES");
    assert_eq!(s.vendor_id, Some(0x1337));
    let m = s.mailbox.as_ref().unwrap();
    assert_eq!((m.out_start, m.out_len, m.in_start, m.in_len), (0x1000, 128, 0x1080, 128));
    assert_eq!(m.protocols, vec!["CoE", "FoE", "EoE"]);
}

#[test]
fn a_transfer_set_up_like_a_simulated_one_moves_the_same_bytes() {
    let spec = flowlab_sim::SimSpec::example();
    let mut quiet = spec.clone();
    quiet.capture.enabled = false;
    let sim = flowlab_sim::simulate(&quiet, None::<Vec<u8>>).unwrap();
    for (k, t) in spec.transfers.iter().enumerate() {
        let Ok(s) = spec.transfer_plan(k).map_err(MasterError::Invalid).and_then(|p| setup_like(&p)) else {
            assert!(matches!(t.protocol, flowlab_sim::TransferProtocol::PdoChannel { .. }));
            continue;
        };
        let st = &sim.transfers[k];
        assert_eq!(s.sha256, st.sha256, "transfer {k}");
        let (out_len, in_len) = s.mailbox_bytes;
        let mailbox = MailboxLayout { station: 0x1001, out_start: 0x1000, out_len, in_start: 0x1000 + out_len, in_len };
        let cfg = VirtualSlaveConfig { mailbox, ..Default::default() };
        let cfg = match &s.payload {
            Payload::Send(_) => cfg,
            Payload::Receive { expected_bytes, .. } => {
                let v = flowlab_mailbox::payload(
                    flowlab_mailbox::transfer_seed(spec.seed, k),
                    expected_bytes.unwrap() as usize,
                );
                let eoe = matches!(s.protocol, TransferProtocol::Eoe { .. });
                VirtualSlaveConfig { upload: v, eoe_send: eoe, ..cfg }
            }
        };
        let slave = VirtualSlave::new(cfg);
        let j = TransferJob {
            slave: "virtual".into(),
            mailbox,
            protocol: s.protocol.clone(),
            payload: s.payload.clone(),
            payload_name: s.payload_name.clone(),
        };
        let o = TransferOptions { cycle_time_ns: s.cycle_time_ns, policy: s.policy.clone(), ..Default::default() };
        let r = run(&slave, j, &o);
        assert!(r.completed, "transfer {k}: {:?}", r.error);
        assert_eq!(r.sha256, st.sha256, "transfer {k}");
        assert_eq!(r.policy, sim.policy);
        assert_eq!((r.protocol.as_str(), r.direction.as_str()), (st.protocol.as_str(), st.direction.as_str()));
        assert_eq!(r.payload_bytes, st.payload_bytes);
        assert_eq!(r.physical_bound_ns, st.physical_bound_ns);
    }
}
