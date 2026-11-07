use std::collections::{BTreeMap, HashMap};

use ecat_wire::{AddressMode, AlStatus, Command, DatagramAddress, register_info, wire_bytes};
use flowlab_capture::{DatagramRecord, Direction, DirectionMode, FrameKind, Trace};
use flowlab_core::{Provenance, ReportMeta, units};

use crate::mailbox::{self, Channels, MailboxAnalysis, MailboxHint};
use crate::report::*;
use crate::stats::{Distribution, decimate, nice_step};

/// Tunables and declared thresholds.
#[derive(Debug, Clone, PartialEq)]
pub struct AnalysisOptions {
    /// Link rate used for utilization, bit/s.
    pub link_bps: u64,
    /// Minimum TX frames for a layout to count as a cyclic stream.
    pub min_stream_frames: usize,
    /// Warn when |phase jitter| p99 exceeds this fraction of the period.
    pub jitter_warn_fraction: f64,
    /// Points per time series.
    pub max_series_points: usize,
    /// Histogram bins (target).
    pub histogram_bins: usize,
    /// TX frames within this time of the capture end are not counted as unanswered.
    pub in_flight_window_ns: u64,
    /// Mailbox SyncManager addresses for stations whose configuration the capture lacks.
    pub mailbox_hints: Vec<MailboxHint>,
    /// The master's network configuration (ENI): expected working counters, mailboxes, names.
    pub network: Option<flowlab_devices::NetworkConfig>,
}

impl Default for AnalysisOptions {
    fn default() -> Self {
        Self {
            link_bps: units::FAST_ETHERNET_BPS,
            min_stream_frames: 20,
            jitter_warn_fraction: 0.10,
            max_series_points: 2000,
            histogram_bins: 40,
            in_flight_window_ns: 50_000_000,
            mailbox_hints: Vec::new(),
            network: None,
        }
    }
}

/// Sentinel for "none" in the per-frame side tables.
pub(crate) const NONE: u32 = u32::MAX;
const MAX_EVENTS: usize = 2000;

/// The report plus per-frame tables used to query frames.
#[derive(Debug)]
pub struct Analysis {
    /// The report.
    pub report: CaptureReport,
    /// For each frame, the paired frame (TX↔RX) or [`u32::MAX`].
    pub pair_of: Vec<u32>,
    /// For each frame, the stream id or [`u16::MAX`].
    pub stream_of: Vec<u16>,
    /// For each frame, whether a WKC differs from the expected value
    /// (set on the RX frame and its TX partner).
    pub wkc_mismatch: Vec<bool>,
    /// For each frame, whether it is an unanswered TX frame.
    pub unanswered: Vec<bool>,
    /// Expected WKC per (stream id, datagram position).
    pub expected_wkc: HashMap<(u16, u16), u16>,
    /// Mailbox channels, messages and reassembled EoE frames.
    pub mailbox: MailboxAnalysis,
}

type Layout = Vec<(u8, u32, u16)>;

/// Analyze a trace.
pub fn analyze(trace: &Trace, opts: &AnalysisOptions) -> Analysis {
    let n = trace.frames.len();
    let mut order: Vec<u32> = (0..n as u32).collect();
    order.sort_by_key(|&i| trace.frames[i as usize].t_ns);

    // Layout ids.
    let mut layouts: HashMap<Layout, u32> = HashMap::new();
    let mut layout_of = vec![NONE; n];
    for (i, f) in trace.frames.iter().enumerate() {
        if f.kind == FrameKind::Ethercat && f.datagram_count > 0 {
            let l: Layout = trace.datagrams_of(f).iter().map(|d| (d.command.code(), d.address_raw, d.len)).collect();
            let next = layouts.len() as u32;
            layout_of[i] = *layouts.entry(l).or_insert(next);
        }
    }

    // TX/RX pairing.
    let end_ns = trace.duration_ns();
    let mut pair_of = vec![NONE; n];
    let mut unanswered = vec![false; n];
    let mut pending: HashMap<(u32, u8), u32> = HashMap::new();
    for &i in &order {
        let f = &trace.frames[i as usize];
        if layout_of[i as usize] == NONE {
            continue;
        }
        let key = (layout_of[i as usize], trace.datagrams[f.first_datagram as usize].index);
        match f.direction {
            Direction::Tx => {
                if let Some(old) = pending.insert(key, i) {
                    unanswered[old as usize] = true;
                }
            }
            Direction::Rx => {
                if let Some(tx) = pending.remove(&key) {
                    pair_of[tx as usize] = i;
                    pair_of[i as usize] = tx;
                }
            }
            Direction::Unknown => {}
        }
    }
    for (_, tx) in pending {
        if trace.frames[tx as usize].t_ns + opts.in_flight_window_ns <= end_ns {
            unanswered[tx as usize] = true;
        }
    }

    let directions_known = trace.direction_mode != DirectionMode::Unknown;
    let (mailbox, mailbox_report) = mailbox::analyze(
        trace,
        &order,
        &pair_of,
        &opts.mailbox_hints,
        opts.network.as_ref(),
        directions_known,
        opts.histogram_bins,
    );
    let is_sent = |i: usize| match trace.frames[i].direction {
        Direction::Tx => true,
        Direction::Rx => false,
        Direction::Unknown => !directions_known,
    };

    // Cyclic streams.
    let mut by_layout: HashMap<u32, Vec<u32>> = HashMap::new();
    for &i in &order {
        if layout_of[i as usize] != NONE && is_sent(i as usize) {
            by_layout.entry(layout_of[i as usize]).or_default().push(i);
        }
    }
    // Frames made only of mailbox accesses are polling, not process data.
    let only_mailbox =
        |i: u32| trace.datagrams_of(&trace.frames[i as usize]).iter().all(|d| mailbox.channels.access(d).is_some());
    let mut candidates: Vec<(u32, Vec<u32>)> = by_layout
        .into_iter()
        .filter(|(_, v)| v.len() >= opts.min_stream_frames && !only_mailbox(v[0]) && is_regular(trace, v))
        .collect();
    candidates.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(&b.0)));

    let mut stream_of = vec![u16::MAX; n];
    let mut wkc_mismatch = vec![false; n];
    let mut expected_wkc = HashMap::new();
    let mut streams = Vec::with_capacity(candidates.len());
    for (id, (_, tx_frames)) in candidates.iter().enumerate() {
        let id = id as u16;
        for &i in tx_frames {
            stream_of[i as usize] = id;
            if pair_of[i as usize] != NONE {
                stream_of[pair_of[i as usize] as usize] = id;
            }
        }
        streams.push(build_stream(
            trace,
            opts,
            id,
            tx_frames,
            &pair_of,
            &unanswered,
            directions_known,
            &mailbox.channels,
            &mut wkc_mismatch,
            &mut expected_wkc,
        ));
    }

    let utilization = utilization(trace, opts, directions_known);
    let (mut stations, al_events, slaves_responding) = stations(trace, &order, directions_known);
    if let Some(n) = &opts.network {
        for s in &mut stations {
            s.name = n.slave(s.station).map(|c| c.name.clone());
        }
    }
    let summary = summary(trace, &pair_of, &unanswered, slaves_responding);
    let mut findings = findings(trace, opts, &summary, &streams, &stations);
    mailbox::findings(&mailbox_report, &mut findings);
    let mut assumptions = assumptions(trace, opts);
    assumptions.extend(mailbox::assumptions(&mailbox_report));
    let meta = ReportMeta::new(trace.provenance, trace.input.clone(), assumptions);
    let report =
        CaptureReport { meta, summary, streams, utilization, stations, al_events, mailbox: mailbox_report, findings };
    Analysis { report, pair_of, stream_of, wkc_mismatch, unanswered, expected_wkc, mailbox }
}

fn median(v: &mut [f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn intervals(trace: &Trace, frames: &[u32]) -> Vec<f64> {
    frames.windows(2).map(|w| (trace.frames[w[1] as usize].t_ns - trace.frames[w[0] as usize].t_ns) as f64).collect()
}

/// At least 60 % of intervals within ±50 % of the median interval.
fn is_regular(trace: &Trace, frames: &[u32]) -> bool {
    let mut iv = intervals(trace, frames);
    let m = median(&mut iv);
    if m <= 0.0 {
        return false;
    }
    let ok = iv.iter().filter(|&&d| d >= 0.5 * m && d <= 1.5 * m).count();
    ok as f64 >= 0.6 * iv.len() as f64
}

fn frame_event(trace: &Trace, i: u32) -> FrameEvent {
    FrameEvent { frame: i + 1, t_ns: trace.frames[i as usize].t_ns as f64 }
}

fn dg_label(d: &DatagramRecord) -> String {
    match d.address() {
        DatagramAddress::Logical { address } => format!("{} 0x{address:08X}", d.command),
        DatagramAddress::Fixed { station, ado } => format!("{} 0x{station:04X}:0x{ado:04X}", d.command),
        DatagramAddress::Position { ado, .. } => {
            format!("{} pos {}:0x{ado:04X}", d.command, d.address().position().unwrap_or(0))
        }
        DatagramAddress::Broadcast { ado, .. } => format!("{} 0x{ado:04X}", d.command),
        DatagramAddress::Raw { value } => format!("{} 0x{value:08X}", d.command),
    }
}

#[allow(clippy::too_many_arguments)]
fn build_stream(
    trace: &Trace,
    opts: &AnalysisOptions,
    id: u16,
    tx: &[u32],
    pair_of: &[u32],
    unanswered: &[bool],
    directions_known: bool,
    channels: &Channels,
    wkc_mismatch: &mut [bool],
    expected_wkc: &mut HashMap<(u16, u16), u16>,
) -> CyclicStream {
    let t = |i: u32| trace.frames[i as usize].t_ns as f64;
    let first = &trace.frames[tx[0] as usize];
    let dgs = trace.datagrams_of(first);
    let signature = dgs.iter().map(dg_label).collect::<Vec<_>>().join(" · ");

    // Cycle numbers and least-squares period.
    let mut iv = intervals(trace, tx);
    let m = median(&mut iv.clone());
    let mut k = Vec::with_capacity(tx.len());
    let mut kk = 0f64;
    k.push(0.0);
    for d in &iv {
        kk += (d / m).round().max(1.0);
        k.push(kk);
    }
    let ts: Vec<f64> = tx.iter().map(|&i| t(i)).collect();
    let (period, intercept) = fit_line(&k, &ts);
    let phase: Vec<f64> = k.iter().zip(&ts).map(|(k, t)| t - (intercept + period * k)).collect();
    let (mut gaps, mut missed) = (0u64, 0u64);
    let mut regular = Vec::with_capacity(iv.len());
    for d in &iv {
        if *d > 1.5 * period {
            gaps += 1;
            missed += ((d / period).round() as u64).saturating_sub(1);
        } else {
            regular.push(*d);
        }
    }
    let interval_points: Vec<(f64, f64)> = tx.windows(2).zip(&iv).map(|(w, d)| (t(w[1]), *d)).collect();
    iv.clear();

    // Round trip.
    let rtt_points: Vec<(f64, f64)> =
        tx.iter().filter(|&&i| pair_of[i as usize] != NONE).map(|&i| (t(i), t(pair_of[i as usize]) - t(i))).collect();
    let rtts: Vec<f64> = rtt_points.iter().map(|p| p.1).collect();
    let unanswered_events: Vec<FrameEvent> =
        tx.iter().filter(|&&i| unanswered[i as usize]).map(|&i| frame_event(trace, i)).collect();

    // Working counters on returning frames.
    let rx: Vec<u32> = tx.iter().map(|&i| pair_of[i as usize]).filter(|&r| r != NONE).collect();
    let mut wkc = Vec::new();
    let mut wkc_events = Vec::new();
    if directions_known && !rx.is_empty() {
        let mut counts: Vec<BTreeMap<u16, u64>> = vec![BTreeMap::new(); dgs.len()];
        for &r in &rx {
            for (p, d) in trace.datagrams_of(&trace.frames[r as usize]).iter().enumerate() {
                *counts[p].entry(d.wkc).or_default() += 1;
            }
        }
        let configured: Vec<Option<u16>> = dgs
            .iter()
            .map(|d| opts.network.as_ref().and_then(|n| n.expected_wkc(d.command.code(), d.address_raw, d.len)))
            .collect();
        let expected: Vec<u16> = counts
            .iter()
            .zip(&configured)
            .map(|(c, conf)| {
                conf.unwrap_or_else(|| {
                    c.iter().max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0))).map(|(w, _)| *w).unwrap_or(0)
                })
            })
            .collect();
        let mut slots: Vec<WkcSlot> = dgs
            .iter()
            .enumerate()
            .map(|(p, d)| WkcSlot {
                position: p as u16,
                label: dg_label(d),
                expected: expected[p],
                mismatches: 0,
                zero: 0,
                expected_from: if configured[p].is_some() { "eni" } else { "most_frequent" }.into(),
                mailbox: channels.access(d).is_some(),
            })
            .collect();
        for (p, e) in expected.iter().enumerate() {
            expected_wkc.insert((id, p as u16), *e);
        }
        for &r in &rx {
            let mut bad = false;
            for (p, d) in trace.datagrams_of(&trace.frames[r as usize]).iter().enumerate() {
                if d.wkc != expected[p] && !slots[p].mailbox {
                    slots[p].mismatches += 1;
                    bad = true;
                }
                if d.wkc == 0 {
                    slots[p].zero += 1;
                }
            }
            if bad {
                wkc_mismatch[r as usize] = true;
                wkc_mismatch[pair_of[r as usize] as usize] = true;
                if wkc_events.len() < MAX_EVENTS {
                    wkc_events.push(frame_event(trace, r));
                }
            }
        }
        wkc = slots;
    }

    let bins = opts.histogram_bins;
    let series = |pts: &[(f64, f64)]| -> Vec<Point> {
        decimate(pts, opts.max_series_points).into_iter().map(|(t_ns, v)| Point { t_ns, v }).collect()
    };
    CyclicStream {
        id,
        signature,
        frame_bytes: first.captured_len,
        frames: tx.len() as u64,
        first_ns: ts[0],
        last_ns: ts[ts.len() - 1],
        period_ns: period,
        interval: Distribution::of(&regular, bins),
        phase_jitter_abs_p99_ns: Distribution::of(&phase.iter().map(|p| p.abs()).collect::<Vec<_>>(), bins)
            .map(|d| d.p99),
        phase_jitter: Distribution::of(&phase, bins),
        gaps,
        missed_cycles: missed,
        round_trip: Distribution::of(&rtts, bins),
        unanswered: unanswered_events.len() as u64,
        wkc,
        wkc_events,
        unanswered_events: unanswered_events.into_iter().take(MAX_EVENTS).collect(),
        interval_series: series(&interval_points),
        round_trip_series: series(&rtt_points),
    }
}

/// Least-squares fit `y = a + b·x`; returns `(b, a)`.
fn fit_line(x: &[f64], y: &[f64]) -> (f64, f64) {
    let n = x.len() as f64;
    if x.len() < 2 {
        return (0.0, y.first().copied().unwrap_or(0.0));
    }
    let (mx, my) = (x.iter().sum::<f64>() / n, y.iter().sum::<f64>() / n);
    let (mut sxy, mut sxx) = (0.0, 0.0);
    for (a, b) in x.iter().zip(y) {
        sxy += (a - mx) * (b - my);
        sxx += (a - mx) * (a - mx);
    }
    let b = if sxx > 0.0 { sxy / sxx } else { 0.0 };
    (b, my - b * mx)
}

fn utilization(trace: &Trace, opts: &AnalysisOptions, directions_known: bool) -> Utilization {
    let dur = trace.duration_ns().max(1) as f64;
    let target_bins = (opts.max_series_points / 2).max(1) as f64;
    let bin_ns = nice_step((dur / target_bins).max(100_000.0));
    let nbins = (dur / bin_ns).floor() as usize + 1;
    let mut tx = vec![0f64; nbins];
    let mut rx = vec![0f64; nbins];
    for f in &trace.frames {
        let bits = wire_bytes(f.orig_len.max(f.captured_len)) as f64 * 8.0;
        let b = ((f.t_ns as f64 / bin_ns) as usize).min(nbins - 1);
        match f.direction {
            Direction::Rx => rx[b] += bits,
            Direction::Tx => tx[b] += bits,
            Direction::Unknown if !directions_known => tx[b] += bits,
            Direction::Unknown => {}
        }
    }
    let cap = opts.link_bps as f64 * bin_ns / 1e9;
    let pct = |v: &[f64]| -> Vec<Point> {
        v.iter().enumerate().map(|(i, b)| Point { t_ns: i as f64 * bin_ns, v: 100.0 * b / cap }).collect()
    };
    let total_cap = opts.link_bps as f64 * (nbins as f64 * bin_ns) / 1e9;
    let tx_series = pct(&tx);
    Utilization {
        link_bps: opts.link_bps as f64,
        bin_ns,
        mean_tx_percent: 100.0 * tx.iter().sum::<f64>() / total_cap,
        peak_tx_percent: tx_series.iter().map(|p| p.v).fold(0.0, f64::max),
        mean_rx_percent: 100.0 * rx.iter().sum::<f64>() / total_cap,
        rx_series: pct(&rx),
        tx_series,
    }
}

#[derive(Default)]
struct StationAcc {
    position: Option<u16>,
    datagrams: u64,
    reads: u64,
    writes: u64,
    no_response: u64,
    areas: HashMap<&'static str, u64>,
    requested: Option<AlStatus>,
    reported: Option<AlStatus>,
}

fn stations(trace: &Trace, order: &[u32], directions_known: bool) -> (Vec<StationSummary>, Vec<AlEvent>, Option<u16>) {
    let mut acc: BTreeMap<u16, StationAcc> = BTreeMap::new();
    let mut positions: HashMap<u16, u16> = HashMap::new();
    let mut events = Vec::new();
    let mut broadcast_req: Option<AlStatus> = None;
    let mut broadcast_rep: Option<AlStatus> = None;
    let mut slaves: Option<u16> = None;
    for &i in order {
        let f = &trace.frames[i as usize];
        if f.kind != FrameKind::Ethercat {
            continue;
        }
        for d in trace.datagrams_of(f) {
            let sent = match f.direction {
                Direction::Tx => true,
                Direction::Rx => false,
                Direction::Unknown => !directions_known && d.wkc == 0,
            };
            let returned = f.direction != Direction::Tx && d.wkc > 0;
            let addr = d.address();
            let data = trace.datagram_data(d);
            let first = data.first().copied();
            let mut push = |station: Option<u16>, kind: AlEventKind, s: AlStatus| {
                events.push(AlEvent {
                    t_ns: f.t_ns as f64,
                    frame: i + 1,
                    station,
                    kind,
                    state: s.state.name().to_owned(),
                    error: s.error,
                });
            };
            match addr {
                DatagramAddress::Fixed { station, ado } => {
                    let a = acc.entry(station).or_default();
                    if sent {
                        a.datagrams += 1;
                        a.reads += d.command.reads() as u64;
                        a.writes += d.command.writes() as u64;
                        let area = register_info(ado).map(|r| r.area).unwrap_or("Other");
                        *a.areas.entry(area).or_default() += 1;
                        if ado == 0x0120
                            && d.command.writes()
                            && let Some(b) = first
                            && let Some(s) = changed(&mut a.requested, b)
                        {
                            push(Some(station), AlEventKind::Requested, s);
                        }
                    }
                    if f.direction == Direction::Rx && d.wkc == 0 {
                        a.no_response += 1;
                    }
                    if returned
                        && ado == 0x0130
                        && d.command.reads()
                        && let Some(b) = first
                        && let Some(s) = changed(&mut a.reported, b)
                    {
                        push(Some(station), AlEventKind::Reported, s);
                    }
                }
                DatagramAddress::Position { ado: 0x0010, .. } if sent && d.command.writes() && data.len() >= 2 => {
                    let station = u16::from_le_bytes([data[0], data[1]]);
                    if let Some(p) = addr.position() {
                        positions.insert(station, p);
                    }
                }
                DatagramAddress::Broadcast { ado, .. } => {
                    if returned && d.command == Command::Brd {
                        slaves = Some(slaves.unwrap_or(0).max(d.wkc));
                    }
                    if ado == 0x0120
                        && sent
                        && d.command.writes()
                        && let Some(b) = first
                        && let Some(s) = changed(&mut broadcast_req, b)
                    {
                        push(None, AlEventKind::Requested, s);
                    }
                    if ado == 0x0130
                        && returned
                        && d.command.reads()
                        && let Some(b) = first
                        && let Some(s) = changed(&mut broadcast_rep, b)
                    {
                        push(None, AlEventKind::Reported, s);
                    }
                }
                _ => {}
            }
            debug_assert!(
                d.command.address_mode() != AddressMode::Fixed || matches!(addr, DatagramAddress::Fixed { .. })
            );
        }
    }
    let summaries = acc
        .into_iter()
        .map(|(station, a)| {
            let mut areas: Vec<AreaCount> =
                a.areas.into_iter().map(|(area, count)| AreaCount { area: area.to_owned(), count }).collect();
            areas.sort_by(|x, y| y.count.cmp(&x.count).then(x.area.cmp(&y.area)));
            StationSummary {
                station,
                name: None,
                position: positions.get(&station).copied().or(a.position),
                datagrams: a.datagrams,
                reads: a.reads,
                writes: a.writes,
                no_response: a.no_response,
                areas,
                requested_state: a.requested.map(|s| s.state.name().to_owned()),
                reported_state: a.reported.map(|s| s.state.name().to_owned()),
                al_error: a.reported.is_some_and(|s| s.error),
            }
        })
        .collect();
    (summaries, events, slaves)
}

/// Update `last` with the AL byte `b`; returns the new status if it changed.
fn changed(last: &mut Option<AlStatus>, b: u8) -> Option<AlStatus> {
    let s = AlStatus::from_byte(b);
    (*last != Some(s)).then(|| {
        *last = Some(s);
        s
    })
}

fn summary(trace: &Trace, pair_of: &[u32], unanswered: &[bool], slaves: Option<u16>) -> CaptureSummary {
    let count = |k: FrameKind| trace.frames.iter().filter(|f| f.kind == k).count() as u64;
    let dir =
        |d: Direction| trace.frames.iter().filter(|f| f.kind == FrameKind::Ethercat && f.direction == d).count() as u64;
    let mut cmds: HashMap<Command, u64> = HashMap::new();
    for d in &trace.datagrams {
        *cmds.entry(d.command).or_default() += 1;
    }
    let mut commands: Vec<CommandCount> =
        cmds.into_iter().map(|(c, count)| CommandCount { command: c.to_string(), count }).collect();
    commands.sort_by(|a, b| b.count.cmp(&a.count).then(a.command.cmp(&b.command)));
    CaptureSummary {
        format: trace.file.format,
        frames: trace.frames.len() as u64,
        ethercat_frames: count(FrameKind::Ethercat),
        other_frames: count(FrameKind::Other),
        malformed_frames: count(FrameKind::Malformed),
        datagrams: trace.datagrams.len() as u64,
        duration_ns: trace.duration_ns(),
        start_unix_ms: trace.start_ts_ns as f64 / 1e6,
        direction_mode: trace.direction_mode,
        tx_frames: dir(Direction::Tx),
        rx_frames: dir(Direction::Rx),
        paired_frames: trace
            .frames
            .iter()
            .enumerate()
            .filter(|(i, f)| f.direction == Direction::Tx && pair_of[*i] != NONE)
            .count() as u64,
        unanswered_frames: unanswered.iter().filter(|u| **u).count() as u64,
        commands,
        slaves_responding: slaves,
    }
}

fn fmt_ns(ns: f64) -> String {
    let a = ns.abs();
    if a >= 1e9 {
        format!("{:.3} s", ns / 1e9)
    } else if a >= 1e6 {
        format!("{:.3} ms", ns / 1e6)
    } else if a >= 1e3 {
        format!("{:.2} µs", ns / 1e3)
    } else {
        format!("{ns:.0} ns")
    }
}

fn findings(
    trace: &Trace,
    opts: &AnalysisOptions,
    summary: &CaptureSummary,
    streams: &[CyclicStream],
    stations: &[StationSummary],
) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut add = |severity, title: String, detail: String| out.push(Finding { severity, title, detail });
    if summary.ethercat_frames == 0 {
        add(
            Severity::Info,
            "No EtherCAT frames".into(),
            "The capture contains no frames with EtherType 0x88A4 or UDP port 0x88A4.".into(),
        );
    }
    match trace.direction_mode {
        DirectionMode::Unknown if summary.ethercat_frames > 0 => add(
            Severity::Info,
            "Frame directions unknown".into(),
            "Source MACs do not split by the locally-administered bit and working counters do not separate TX from RX. Round trip, lost frames and WKC checks are unavailable.".into(),
        ),
        DirectionMode::WkcHeuristic => add(
            Severity::Info,
            "Directions inferred from working counters".into(),
            "No source-MAC split was found; frames whose working counters are all 0 are treated as TX. A returning frame no slave answered is then misread as TX.".into(),
        ),
        _ => {}
    }
    if summary.malformed_frames > 0 {
        add(
            Severity::Warn,
            format!("{} malformed EtherCAT frames", summary.malformed_frames),
            "Frames claim EtherCAT but their headers or datagram chain do not parse. Check the capture tool (truncation, snap length).".into(),
        );
    }
    if streams.is_empty() && summary.ethercat_frames > 0 {
        add(
            Severity::Info,
            "No cyclic stream detected".into(),
            format!("No datagram layout was sent ≥ {} times at a regular interval.", opts.min_stream_frames),
        );
    }
    for s in streams {
        let name = format!("Stream {} ({})", s.id, fmt_ns(s.period_ns));
        if s.unanswered > 0 {
            add(
                Severity::Defect,
                format!("{name}: {} frames did not return", s.unanswered),
                format!(
                    "{} of {} TX frames ({:.3} %) have no returning frame. Threshold: any.",
                    s.unanswered,
                    s.frames,
                    100.0 * s.unanswered as f64 / s.frames as f64
                ),
            );
        }
        let mism: u64 = s.wkc.iter().map(|w| w.mismatches).sum();
        if mism > 0 {
            let worst = s.wkc.iter().max_by_key(|w| w.mismatches).map(|w| w.label.clone()).unwrap_or_default();
            add(Severity::Defect, format!("{name}: {} working-counter mismatches", s.wkc_events.len().max(1)), {
                let basis = if s.wkc.iter().all(|w| w.expected_from == "eni") {
                    "the value configured in the ENI"
                } else if s.wkc.iter().any(|w| w.expected_from == "eni") {
                    "the ENI value where configured, else the most frequent value"
                } else {
                    "the most frequent value"
                };
                format!(
                    "Returning frames whose WKC differs from {basis}; most affected datagram: {worst}. Threshold: any."
                )
            });
        }
        if s.missed_cycles > 0 {
            add(
                Severity::Warn,
                format!("{name}: {} missed cycles", s.missed_cycles),
                format!("{} gaps longer than 1.5 periods in the master's send times.", s.gaps),
            );
        }
        if let Some(p99) = s.phase_jitter_abs_p99_ns
            && p99 > opts.jitter_warn_fraction * s.period_ns
        {
            add(
                Severity::Warn,
                format!("{name}: send jitter p99 {}", fmt_ns(p99)),
                format!(
                    "|phase jitter| p99 exceeds {:.0} % of the period. Timestamps come from the capture tool; host timestamping can add its own jitter.",
                    opts.jitter_warn_fraction * 100.0
                ),
            );
        }
    }
    for st in stations.iter().filter(|s| s.al_error) {
        add(
            Severity::Defect,
            format!("Station 0x{:04X} reports an AL error", st.station),
            format!(
                "Last AL status read had the error indication bit set (state {}).",
                st.reported_state.as_deref().unwrap_or("?")
            ),
        );
    }
    if trace.provenance == Provenance::Synthetic {
        add(
            Severity::Info,
            "Synthetic capture".into(),
            "Generated by flowlab-synth with known ground truth; not a measurement.".into(),
        );
    }
    out
}

fn assumptions(trace: &Trace, opts: &AnalysisOptions) -> Vec<String> {
    let mut a = vec![
        format!(
            "Utilization assumes a {} Mbit/s link; wire bytes include padding to 64 bytes, preamble, FCS and the 12-byte inter-frame gap.",
            opts.link_bps / 1_000_000
        ),
        format!(
            "A cyclic stream is a datagram layout (command, address, length) sent at least {} times with ≥ 60 % of intervals within ±50 % of the median.",
            opts.min_stream_frames
        ),
        match &opts.network {
            None => "Expected working counters are the most frequent value per datagram position; no ENI network configuration was given.".into(),
            Some(_) => "Expected working counters come from the ENI where it lists the datagram (same command, address and length); other datagrams use the most frequent value.".into(),
        },
        "Timestamps are taken as recorded by the capture tool; their accuracy is not known to FlowLab.".into(),
    ];
    a.push(match trace.direction_mode {
        DirectionMode::MacLocalBit => "Directions: source MAC without the locally-administered bit is TX, with it RX (ESCs set the bit on returning frames).".into(),
        DirectionMode::WkcHeuristic => "Directions: frames with all working counters 0 are TX, others RX.".into(),
        DirectionMode::Unknown => "Directions could not be inferred; all EtherCAT frames are treated as sent.".into(),
    });
    a
}
