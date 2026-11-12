use flowlab_core::Provenance;

use super::*;

fn esc() -> (Lan9252<SimLan9252>, SimClock) {
    let (sim, clock) = SimLan9252::new(SimTiming::default());
    (Lan9252::new(sim), clock)
}

fn assert_clean(esc: &Lan9252<SimLan9252>) {
    assert!(esc.bus().errors().is_empty(), "protocol errors: {:?}", esc.bus().errors());
}

#[test]
fn spi_framing_matches_the_data_sheet() {
    // A recording bus: READ is instruction, big-endian address, then four bytes in.
    struct Rec(Vec<Vec<u8>>);
    impl SpiBus for Rec {
        fn transaction(&mut self, segments: &mut [Segment<'_>]) -> std::io::Result<()> {
            let mut t = Vec::new();
            for s in segments.iter_mut() {
                match s {
                    Segment::Write(b) => t.extend_from_slice(b),
                    Segment::Read(b) => {
                        b.copy_from_slice(&[0x21, 0x43, 0x65, 0x87][..b.len()]);
                        t.push(0xEE);
                    }
                }
            }
            self.0.push(t);
            Ok(())
        }
    }
    let mut e = Lan9252::new(Rec(Vec::new()));
    assert_eq!(e.read_register(reg::BYTE_TEST).unwrap(), 0x8765_4321);
    let mut f = Lan9252::new(Rec(Vec::new())).with_read_instruction(ReadInstruction::FastRead);
    f.read_register(reg::ECAT_CSR_CMD).unwrap();
    assert_eq!(e.bus().0, [vec![0x03, 0x00, 0x64, 0xEE]]);
    assert_eq!(f.bus().0, [vec![0x0B, 0x03, 0x04, 0x01, 0xEE]]);
}

#[test]
fn csr_reads_follow_the_size_rules_and_return_esc_registers() {
    assert_eq!([0u16, 1, 2, 3, 4].map(|a| csr_chunk(a, 4)), [4, 1, 2, 1, 4]);
    assert_eq!([csr_chunk(0, 3), csr_chunk(2, 1), csr_chunk(0, 2)], [1, 1, 2]);
    for instruction in [ReadInstruction::Read, ReadInstruction::FastRead] {
        let (sim, _) = SimLan9252::new(SimTiming::default());
        let mut e = Lan9252::new(sim).with_read_instruction(instruction).with_al_event_read(false);
        let mut all = [0u8; 16];
        // ESC registers 0x0130.. (AL Status first) through the CSR interface, in odd pieces.
        for (start, len) in [(0x0130u16, 16usize), (0x0131, 7), (0x0133, 3), (0x0132, 2)] {
            let mut buf = vec![0u8; len];
            e.read(start, &mut buf).unwrap();
            let a = usize::from(start);
            assert_eq!(buf, e.bus().esc()[a..a + len], "{len} bytes at 0x{start:04X}");
            if start == 0x0130 {
                all.copy_from_slice(&buf);
            }
        }
        assert_eq!(all[0], 0x01, "AL Status reads INIT");
        assert!(e.bus().accesses().iter().all(|a| !a.write && !a.via_fifo && [1, 2, 4].contains(&a.len)));
        assert_clean(&e);
    }
}

#[test]
fn register_writes_and_protected_ranges_are_refused() {
    let (mut e, _) = esc();
    assert!(matches!(e.write(0x0130, &[0]), Err(PdiError::Refused(_))));
    assert!(matches!(e.write(0x0120, &[0x02, 0]), Err(PdiError::Refused(_))));
    let mut b = [0u8; 4];
    assert!(matches!(e.read(0x011E, &mut b), Err(PdiError::Refused(_))), "overlaps AL Control");
    assert!(matches!(e.read(0x0806, &mut b), Err(PdiError::Refused(_))), "SyncManager 0");
    assert!(matches!(e.read(0x0FFE, &mut b), Err(PdiError::Refused(_))), "crosses into process RAM");
    assert!(matches!(e.read(0x1FFE, &mut b), Err(PdiError::Refused(_))), "beyond 4 KiB");
    assert_eq!(e.bus().transactions(), 0, "refused before the bus");
}

#[test]
fn process_ram_round_trips_through_the_fifos() {
    for instruction in [ReadInstruction::Read, ReadInstruction::FastRead] {
        let (sim, _) = SimLan9252::new(SimTiming::default());
        let mut e = Lan9252::new(sim).with_read_instruction(instruction);
        // Sizes that are not DWORD multiples, larger than the 64-byte FIFO, unaligned starts.
        for (k, (addr, len)) in
            [(0x1000u16, 1usize), (0x1001, 3), (0x1003, 5), (0x1002, 63), (0x1100, 64), (0x1101, 65), (0x1203, 1000)]
                .into_iter()
                .enumerate()
        {
            let data: Vec<u8> = (0..len).map(|i| (i * 31 + k) as u8).collect();
            let a = usize::from(addr);
            let before = e.bus().esc().to_vec();
            e.write(addr, &data).unwrap();
            let after = e.bus().esc();
            assert_eq!(after[a..a + len], data[..], "{len} bytes at 0x{addr:04X}");
            assert_eq!(after[..a], before[..a], "untouched below");
            assert_eq!(after[a + len..], before[a + len..], "untouched above");
            let mut back = vec![0u8; len];
            e.read(addr, &mut back).unwrap();
            assert_eq!(back, data);
        }
        // Reading the whole process RAM takes 64 FIFO bursts.
        let mut all = vec![0u8; 4096];
        e.read(0x1000, &mut all).unwrap();
        assert_eq!(all, e.bus().esc()[0x1000..]);
        assert_clean(&e);
    }
}

#[test]
fn a_broken_link_or_wrong_chip_is_refused_before_any_esc_access() {
    for (byte_test, id_rev, needle) in [
        (0xFFFF_FFFF, 0x9252_0001, "nothing answers"),
        (0x2143_6587, 0x9252_0001, "bytes swapped"),
        (0x8765_4321, 0x9253_0001, "chip 0x9253"),
    ] {
        let (mut sim, clock) = SimLan9252::new(SimTiming::default());
        sim.set_byte_test(byte_test);
        sim.set_id_rev(id_rev);
        let mut e = Lan9252::new(sim);
        let err = run(&mut e, &clock, &BenchOptions::default(), 0x9252).unwrap_err();
        assert!(matches!(&err, PdiError::Link(m) if m.contains(needle)), "{err}");
        assert!(e.bus().accesses().is_empty(), "no ESC access after a failed check");
    }
}

#[test]
fn an_esc_outside_init_is_refused() {
    let (mut sim, clock) = SimLan9252::new(SimTiming::default());
    sim.set_al_status(0x0002);
    let mut e = Lan9252::new(sim);
    let err = run(&mut e, &clock, &BenchOptions::default(), 0x9252).unwrap_err();
    assert!(err.to_string().contains("PREOP, not INIT"), "{err}");
    assert!(e.bus().accesses().iter().all(|a| a.address == reg::ESC_AL_STATUS));
}

#[test]
fn a_stopped_application_allows_reads_from_an_explicit_area_in_any_state() {
    let area = Some("0x1800+256".parse::<Area>().unwrap());
    let stopped = BenchOptions { app_stopped: true, sizes: vec![4, 128], reps: 3, ..BenchOptions::default() };
    // Without an area, or with writes, it is refused before any access.
    assert!(matches!(plan(&stopped), Err(PdiError::Refused(_))));
    let writes = BenchOptions { allow_writes: true, area, ..stopped.clone() };
    assert!(matches!(plan(&writes), Err(PdiError::Refused(_))));

    let (mut sim, clock) = SimLan9252::new(SimTiming::default());
    sim.set_al_status(0x0002);
    let mut e = Lan9252::new(sim);
    let r = run(&mut e, &clock, &BenchOptions { area, ..stopped }, 0x9252).expect("reads in PREOP");
    assert_eq!(r.samples.len(), 6);
    assert_eq!(r.plan.write_area, None);
}

#[test]
fn writes_need_permission_and_an_area_in_process_ram() {
    let area = |s: &str| Some(s.parse::<Area>().unwrap());
    let o = |allow_writes, a| BenchOptions { allow_writes, area: a, ..BenchOptions::default() };
    assert!(matches!(plan(&o(true, None)), Err(PdiError::Refused(_))));
    assert!(matches!(plan(&o(true, area("0x0F00+1024"))), Err(PdiError::Refused(_))));
    assert!(matches!(plan(&o(true, area("0x1D00+1024"))), Err(PdiError::Refused(_))), "ends past 0x2000");
    assert!(matches!(plan(&o(true, area("0x1800+512"))), Err(PdiError::Options(_))), "1024-byte accesses");
    assert!(matches!(plan(&o(false, area("0x0100+1024"))), Err(PdiError::Refused(_))), "reads AL Control");
    assert_eq!(plan(&o(false, None)).unwrap().write_area, None);
    assert_eq!(plan(&o(true, area("0x1800+1024"))).unwrap().write_area, area("6144+1024"));
    assert!("0x1800".parse::<Area>().is_err() && "0x1800+0".parse::<Area>().is_err());

    // Without --allow-writes the run only reads.
    let (mut e, clock) = esc();
    let before = e.bus().esc().to_vec();
    let small = BenchOptions { sizes: vec![4, 64], reps: 3, ..BenchOptions::default() };
    let r = run(&mut e, &clock, &small, 0x9252).unwrap();
    assert!(r.samples.iter().all(|s| s.direction == Direction::Read));
    assert!(e.bus().accesses().iter().all(|a| !a.write));
    assert_eq!(e.bus().esc(), &before[..]);
}

#[test]
fn a_run_with_writes_stays_inside_its_area_and_away_from_protected_registers() {
    let (mut e, clock) = esc();
    let o = BenchOptions {
        sizes: vec![1, 2, 7, 100, 512],
        reps: 4,
        warmup: 1,
        area: Some("0x1800+512".parse().unwrap()),
        allow_writes: true,
        app_stopped: false,
        seed: 9,
    };
    let r = run(&mut e, &clock, &o, 0x9252).unwrap();
    assert_eq!(r.samples.len(), 2 * 5 * 4);
    for a in e.bus().accesses() {
        let (s, end) = (u32::from(a.address), u32::from(a.address) + u32::from(a.len));
        if a.write {
            assert!(a.via_fifo && s >= 0x1800 && end <= 0x1A00, "write {a:?}");
        }
        for (ps, pe, name) in PROTECTED {
            assert!(end <= u32::from(ps) || s >= u32::from(pe), "{name} touched by {a:?}");
        }
    }
    // The order is shuffled, not grouped by size.
    let first: Vec<u32> = r.samples.iter().take(8).map(|s| s.bytes).collect();
    assert!(first.windows(2).any(|w| w[0] != w[1]), "{first:?}");
    assert_clean(&e);
}

#[test]
fn samples_fit_back_to_the_simulated_bus() {
    let timing = SimTiming::default();
    let (sim, clock) = SimLan9252::new(timing);
    let mut e = Lan9252::new(sim);
    let o = BenchOptions {
        sizes: vec![64, 128, 256, 512, 1024],
        reps: 5,
        area: Some("0x1400+1024".parse().unwrap()),
        allow_writes: true,
        ..BenchOptions::default()
    };
    let r = run(&mut e, &clock, &o, 0x9252).unwrap();
    let csv = to_csv(&r.samples, &["simulated LAN9252".into()], Provenance::Simulated);
    let parsed = flowlab_core::pdi_samples::parse(&csv).unwrap();
    assert_eq!(parsed.len(), r.samples.len());
    let fit = flowlab_study::fit_pdi(&parsed, "sim.csv", csv.as_bytes()).unwrap();
    assert_eq!(fit.meta.source, Provenance::Simulated);

    // Every 16 DWORDs of a long access cost one poll (7 bytes) and one burst (3 + 64 bytes),
    // two transactions: that is the effective rate the line must find.
    let ns_per_64 = 2.0 * timing.transaction_ns as f64 + 74.0 * 8e9 / timing.clock_hz as f64;
    let expected_bps = 8e9 * 64.0 / ns_per_64;
    for line in [fit.read.as_ref().unwrap(), fit.write.as_ref().unwrap()] {
        assert!((line.rate_bps / expected_bps - 1.0).abs() < 0.01, "{} vs {expected_bps}", line.rate_bps);
        assert!(line.max_relative_error < 0.01, "{}", line.max_relative_error);
        assert!(line.overhead_ns > 4.0 * timing.transaction_ns as f64, "{}", line.overhead_ns);
    }
    // The simulator is deterministic: every repetition takes the same time.
    for s in summarise(&r.samples) {
        assert_eq!(s.median_ns, s.p95_ns, "{s:?}");
    }
}
