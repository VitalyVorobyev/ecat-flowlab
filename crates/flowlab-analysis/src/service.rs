//! How the master serves each slave's mailbox: accesses and frames per cycle, the turnaround
//! between back-to-back mailbox frames, and how far into the cycle mailbox frames are still
//! sent. From that pattern it names the simulator policy that would produce it, so a capture
//! of the real master can stand in for the policy the simulator would otherwise have to
//! assume.
//!
//! Cycles are those of cyclic stream 0: a cycle starts with a frame of that stream and ends
//! with the next one. Only sent copies of reads and writes of the mailbox SyncManager buffers
//! count (status-register polls do not), so directions must be known.

use std::collections::BTreeMap;

use ecat_wire::DatagramAddress;
use flowlab_capture::{Direction, FrameKind, Trace};

use crate::mailbox::{Access, Channels};
use crate::report::{MailboxService, ServicePattern};
use crate::stats::Distribution;

const NONE: u32 = u32::MAX;

/// Frames per cycle (median) above which the pattern counts as back-to-back polling.
const BACK_TO_BACK_FRAMES: f64 = 1.5;

#[derive(Default)]
struct Cycle {
    accesses: u32,
    frames: u32,
    last_frame: Option<u32>,
    last_send_ns: f64,
}

/// Cycles with mailbox accesses a station needs before its pattern is reported.
pub(crate) const MIN_CYCLES: u64 = 10;

/// Service pattern per station, for stations whose mailbox was accessed in at least
/// [`MIN_CYCLES`] cycles of stream 0, given as its sent frames in time order and its period.
pub(crate) fn analyze(
    trace: &Trace,
    order: &[u32],
    pair_of: &[u32],
    stream_of: &[u16],
    channels: &Channels,
    (cycle_frames, period_ns): (&[u32], f64),
    bins: usize,
) -> BTreeMap<u16, MailboxService> {
    let mut out = BTreeMap::new();
    if cycle_frames.len() < 2 || period_ns <= 0.0 {
        return out;
    }
    let starts: Vec<f64> = cycle_frames.iter().map(|&i| trace.frames[i as usize].t_ns as f64).collect();
    let cycle_of = |t: f64| -> Option<usize> {
        let k = starts.partition_point(|&s| s <= t);
        // Before the first cycle (start-up) or after the last one: not part of a cycle.
        (k > 0 && k < starts.len()).then(|| k - 1)
    };

    let mut cycles: BTreeMap<u16, BTreeMap<usize, Cycle>> = BTreeMap::new();
    let mut in_cyclic: BTreeMap<u16, u64> = BTreeMap::new();
    // Per station: (send of this frame, return of the previous frame in the same cycle).
    let mut turnaround: BTreeMap<u16, Vec<f64>> = BTreeMap::new();
    let mut last_return: BTreeMap<u16, (usize, f64)> = BTreeMap::new();

    for &i in order {
        let f = &trace.frames[i as usize];
        if f.kind != FrameKind::Ethercat || f.direction != Direction::Tx {
            continue;
        }
        let t = f.t_ns as f64;
        let Some(k) = cycle_of(t) else { continue };
        let ret = pair_of[i as usize];
        let ret_t = (ret != NONE).then(|| trace.frames[ret as usize].t_ns as f64);
        for d in trace.datagrams_of(f) {
            let Some(Access::Out | Access::In) = channels.access(d) else { continue };
            let DatagramAddress::Fixed { station, .. } = d.address() else { continue };
            let c = cycles.entry(station).or_default().entry(k).or_default();
            c.accesses += 1;
            if c.last_frame != Some(i) {
                c.frames += 1;
                c.last_frame = Some(i);
                c.last_send_ns = t - starts[k];
                if let Some(&(pk, pt)) = last_return.get(&station)
                    && pk == k
                    && t >= pt
                {
                    turnaround.entry(station).or_default().push(t - pt);
                }
                match ret_t {
                    Some(rt) => last_return.insert(station, (k, rt)),
                    None => last_return.remove(&station),
                };
            }
            if stream_of[i as usize] != u16::MAX {
                *in_cyclic.entry(station).or_default() += 1;
            }
        }
    }

    for (station, per_cycle) in cycles {
        if (per_cycle.len() as u64) < MIN_CYCLES {
            continue;
        }
        let accesses: Vec<f64> = per_cycle.values().map(|c| c.accesses as f64).collect();
        let frames: Vec<f64> = per_cycle.values().map(|c| c.frames as f64).collect();
        let last_send: Vec<f64> = per_cycle.values().map(|c| c.last_send_ns / period_ns * 100.0).collect();
        let (Some(accesses), Some(frames), Some(last_send)) =
            (Distribution::of(&accesses, bins), Distribution::of(&frames, bins), Distribution::of(&last_send, bins))
        else {
            continue;
        };
        let turnaround = turnaround.get(&station).and_then(|v| Distribution::of(v, bins));
        let pattern = if frames.p50 > BACK_TO_BACK_FRAMES {
            ServicePattern::Opportunistic {
                bus_budget_percent: last_send.p95.ceil().clamp(1.0, 100.0),
                turnaround_ns: turnaround.as_ref().map_or(0.0, |d| d.p50.round()),
            }
        } else {
            ServicePattern::FixedQuota { accesses_per_cycle: (accesses.p95.round() as u32).max(1) }
        };
        out.insert(
            station,
            MailboxService {
                cycles: per_cycle.len() as u64,
                accesses_per_cycle: accesses,
                frames_per_cycle: frames,
                in_cyclic_frames: in_cyclic.get(&station).copied().unwrap_or(0),
                turnaround,
                last_send_percent: last_send,
                pattern,
            },
        );
    }
    out
}

/// Assumption text for reports that contain a service pattern.
pub(crate) const ASSUMPTION: &str = "Mailbox service pattern: cycles are those of cyclic stream 0; only sent \
    reads and writes of the mailbox SyncManager buffers count, for slaves accessed in at least 10 cycles. A median above 1.5 mailbox frames per cycle is \
    read as back-to-back polling (budget = 95th percentile of the last mailbox send in the cycle, turnaround = \
    median return-to-send gap); otherwise as a fixed quota (95th percentile of accesses per cycle). The quota \
    is per slave; a master that shares one quota among several slaves needs the sum.";
