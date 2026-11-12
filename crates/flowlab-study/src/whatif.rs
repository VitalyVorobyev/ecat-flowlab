//! What would help: rerun a scenario with one change at a time and rank the changes.
//!
//! Each change doubles or halves one parameter, or switches one choice (mailbox policy,
//! protocol, mailbox access), so the ranking reads as a sensitivity: the parameters whose
//! change shortens the transfers most are the ones that limit them. Every run is a simulation.

use flowlab_core::{InputRef, Provenance, ReportMeta};
use flowlab_sim::{
    MAX_MAILBOX_BYTES, MailboxAccess, MailboxPolicy, SimResult, SimSpec, TransferDirection, TransferProtocol,
    TransferSpec,
};
use serde::Serialize;
use serde_json::{Value, json};
use ts_rs::TS;

use crate::StudyError;
use crate::run::par_map;

/// What part of the scenario a change touches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Lever {
    /// The master's cycle.
    Master,
    /// How the master schedules mailbox datagrams.
    MailboxPolicy,
    /// A slave's process data interface (e.g. SPI) to its ESC.
    Pdi,
    /// A slave's mailbox SyncManagers.
    Mailbox,
    /// A slave application's processing time.
    Application,
    /// How one transfer crosses the bus.
    Protocol,
    /// Two of the changes above together.
    Combined,
}

/// One value set by a change, addressed by JSON pointer into the spec.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct SpecEdit {
    /// JSON pointer, e.g. `/slaves/1/pdi/read_bps`.
    pub path: String,
    /// The new value.
    pub value: Value,
}

/// What one simulation of the scenario (changed or not) gave.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct WhatIfRun {
    /// The spec was valid and every transfer finished verified within `max_duration_ms`.
    pub feasible: bool,
    /// Why not.
    pub reason: Option<String>,
    /// Duration of each transfer, ns, in spec order; `null` when it did not finish.
    pub transfer_ns: Vec<Option<f64>>,
    /// When the last transfer finished, ns after the first cycle; `null` unless feasible.
    pub finish_ns: Option<f64>,
    /// Link utilization over the run, percent.
    pub link_utilization_percent: Option<f64>,
    /// The busiest slave PDI, percent of the simulated time.
    pub pdi_busy_percent_max: Option<f64>,
    /// The fullest ESC RAM, percent of what is available.
    pub dpram_used_percent_max: Option<f64>,
}

/// One change and its outcome.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct WhatIf {
    /// What changes, with the old and new values.
    pub label: String,
    /// What part of the scenario it touches.
    pub lever: Lever,
    /// Index of the slave it changes.
    pub slave: Option<u32>,
    /// Index of the transfer it changes.
    pub transfer: Option<u32>,
    /// The values it sets.
    pub edits: Vec<SpecEdit>,
    /// For a combined change, the labels of the single changes it combines; else empty.
    pub parts: Vec<String>,
    /// The run with the change.
    pub run: WhatIfRun,
}

/// The scenario as given and every one-change variant, best first.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct WhatIfResult {
    /// Provenance (`simulated`), the scenario's identity, the method.
    pub meta: ReportMeta,
    /// The scenario unchanged.
    pub base: WhatIfRun,
    /// Feasible changes by finish time (earliest first), then the infeasible ones. Single
    /// changes, and pairs of them (`lever` `combined`).
    pub changes: Vec<WhatIf>,
    /// Relative change below which a change counts as having no effect (0.01 = 1 %).
    pub no_effect: f64,
    /// The changes ranked by when all transfers are done (`transfer` `null`, first), then by
    /// each transfer's own duration.
    pub rankings: Vec<WhatIfRanking>,
}

/// Options of [`what_if`].
#[derive(Debug, Clone, PartialEq)]
pub struct WhatIfOptions {
    /// Relative change below which a change counts as having no effect (default 0.01).
    pub no_effect: f64,
    /// Pairs listed at most in each ranking (default 5).
    pub max_pairs: usize,
}

impl Default for WhatIfOptions {
    fn default() -> Self {
        Self { no_effect: 0.01, max_pairs: 5 }
    }
}

/// One change in a [`WhatIfRanking`].
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct RankedChange {
    /// Index into [`WhatIfResult::changes`].
    pub change: u32,
    /// The ranked time (all transfers done, or the transfer's duration), ns; `null` if the run is
    /// not feasible.
    pub time_ns: Option<f64>,
    /// Relative change of that time against the scenario as given (−0.25 = 25 % shorter).
    pub delta: Option<f64>,
}

/// The changes sorted by their effect on one time, best first in each list.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct WhatIfRanking {
    /// The transfer whose duration is ranked; `null` ranks when all transfers are done. Protocol
    /// switches of other transfers are left out.
    pub transfer: Option<u32>,
    /// Single changes that shorten the time by more than `no_effect`.
    pub helps: Vec<RankedChange>,
    /// Single changes within `no_effect`.
    pub flat: Vec<RankedChange>,
    /// Single changes that lengthen the time by more than `no_effect`.
    pub hurts: Vec<RankedChange>,
    /// Single changes whose run is not feasible, or does not finish the transfer.
    pub impossible: Vec<RankedChange>,
    /// Pairs that beat both of their parts alone, and the scenario, by more than `no_effect`.
    pub together: Vec<RankedChange>,
}

fn ranking(base: &WhatIfRun, changes: &[WhatIf], transfer: Option<u32>, o: &WhatIfOptions) -> WhatIfRanking {
    let pick = |run: &WhatIfRun| match transfer {
        None => run.finish_ns,
        Some(t) if run.feasible => run.transfer_ns.get(t as usize).copied().flatten(),
        Some(_) => None,
    };
    let b = pick(base).filter(|b| *b > 0.0);
    let mut scored: Vec<(&WhatIf, RankedChange)> = changes
        .iter()
        .enumerate()
        .filter(|(_, c)| transfer.is_none() || c.transfer.is_none() || c.transfer == transfer)
        .map(|(i, c)| {
            let time_ns = pick(&c.run);
            let delta = time_ns.zip(b).map(|(t, b)| (t - b) / b);
            (c, RankedChange { change: i as u32, time_ns, delta })
        })
        .collect();
    scored.sort_by(|x, y| x.1.delta.unwrap_or(f64::INFINITY).total_cmp(&y.1.delta.unwrap_or(f64::INFINITY)));
    let alone = |label: &str| {
        scored
            .iter()
            .find(|(c, _)| c.parts.is_empty() && c.label == label)
            .and_then(|(_, r)| r.delta)
            .unwrap_or(f64::INFINITY)
    };
    let singles = || scored.iter().filter(|(c, _)| c.parts.is_empty());
    let take =
        |keep: &dyn Fn(Option<f64>) -> bool| singles().filter(|(_, r)| keep(r.delta)).map(|(_, r)| r.clone()).collect();
    let ne = o.no_effect;
    WhatIfRanking {
        transfer,
        helps: take(&|d| d.is_some_and(|d| d < -ne)),
        flat: take(&|d| d.is_some_and(|d| d.abs() <= ne)),
        hurts: take(&|d| d.is_some_and(|d| d > ne)),
        impossible: take(&|d| d.is_none()),
        together: scored
            .iter()
            .filter(|(c, r)| {
                c.parts.len() == 2
                    && r.delta.is_some_and(|d| {
                        let best = c.parts.iter().map(|p| alone(p)).fold(f64::INFINITY, f64::min);
                        d < best.min(0.0) - ne
                    })
            })
            .take(o.max_pairs)
            .map(|(_, r)| r.clone())
            .collect(),
    }
}

/// Mailbox policy tried by a switch: the master fills up to 60 % of the cycle with mailbox
/// frames and needs 10 µs between a frame's return and the next.
const OPPORTUNISTIC: (f64, u32) = (60.0, 10_000);
/// At most this many single changes (the first that are not a protocol switch) are combined
/// in pairs.
const MAX_PAIRED: usize = 8;
/// Bytes per cycle of a PDO channel tried in place of a mailbox protocol.
const PDO_CHANNEL_BYTES: u16 = 256;

fn mbit(bps: u64) -> String {
    let m = bps as f64 / 1e6;
    if m.fract() == 0.0 { format!("{m:.0} Mbit/s") } else { format!("{m:.2} Mbit/s") }
}

fn us(ns: u64) -> String {
    let u = ns as f64 / 1e3;
    if u.fract() == 0.0 { format!("{u:.0} µs") } else { format!("{u:.1} µs") }
}

fn protocol_name(p: &TransferProtocol) -> String {
    match p {
        TransferProtocol::Eoe { .. } => "EoE".into(),
        TransferProtocol::Foe => "FoE".into(),
        TransferProtocol::CoeSegmented => "CoE".into(),
        TransferProtocol::Voe { window } => format!("VoE (window {window})"),
        TransferProtocol::PdoChannel { bytes_per_cycle, .. } => format!("PDO channel {bytes_per_cycle} B/cycle"),
    }
}

fn transfer_name(i: usize, t: &TransferSpec) -> String {
    let arrow = if t.direction == TransferDirection::MasterToSlave { "↓" } else { "↑" };
    format!("transfer {i} ({} {arrow})", protocol_name(&t.protocol))
}

/// A change to try: label, lever, slave, transfer, edits.
type Candidate = (String, Lever, Option<u32>, Option<u32>, Vec<SpecEdit>);

/// The changes worth trying on `spec`: only slaves and transfers that carry a transfer.
fn candidates(spec: &SimSpec) -> Vec<Candidate> {
    let mut out = Vec::new();
    let edit = |path: String, value: Value| SpecEdit { path, value };

    let cycle = spec.cycle_time_us;
    if cycle >= 2 {
        out.push((
            format!("Cycle time {} → {}", us(u64::from(cycle) * 1000), us(u64::from(cycle / 2) * 1000)),
            Lever::Master,
            None,
            None,
            vec![edit("/cycle_time_us".into(), json!(cycle / 2))],
        ));
    }

    let mailbox_slaves = spec.slaves.iter().filter(|s| s.mailbox_out_bytes > 0).count() as u32;
    let policy = |label: String, value: Value| {
        (label, Lever::MailboxPolicy, None, None, vec![edit("/mailbox_policy".into(), value)])
    };
    let opportunistic = || {
        policy(
            format!(
                "Opportunistic mailbox policy ({:.0} % of the cycle, {} turnaround)",
                OPPORTUNISTIC.0,
                us(OPPORTUNISTIC.1.into())
            ),
            json!({ "kind": "opportunistic", "bus_budget_percent": OPPORTUNISTIC.0, "turnaround_ns": OPPORTUNISTIC.1 }),
        )
    };
    match &spec.mailbox_policy {
        MailboxPolicy::FixedQuota { accesses_per_cycle: n } => {
            out.push(policy(
                format!("Mailbox datagrams per cycle {n} → {}", n * 2),
                json!({ "kind": "fixed_quota", "accesses_per_cycle": n * 2 }),
            ));
            out.push(opportunistic());
        }
        MailboxPolicy::RoundRobin { slaves_per_cycle: n } => {
            if *n < mailbox_slaves {
                out.push(policy(
                    format!("Mailbox slaves served per cycle {n} → {}", (n * 2).min(mailbox_slaves)),
                    json!({ "kind": "round_robin", "slaves_per_cycle": (n * 2).min(mailbox_slaves) }),
                ));
            }
            out.push(opportunistic());
        }
        MailboxPolicy::Opportunistic { bus_budget_percent: b, turnaround_ns: t } => {
            if *b < 90.0 {
                let nb = (b * 1.5).min(90.0);
                out.push(policy(
                    format!("Mailbox share of the cycle {b:.0} % → {nb:.0} %"),
                    json!({ "kind": "opportunistic", "bus_budget_percent": nb, "turnaround_ns": t }),
                ));
            }
            if *t >= 2 {
                out.push(policy(
                    format!("Master turnaround {} → {}", us((*t).into()), us((t / 2).into())),
                    json!({ "kind": "opportunistic", "bus_budget_percent": b, "turnaround_ns": t / 2 }),
                ));
            }
        }
    }

    let mut busy: Vec<usize> = spec.transfers.iter().map(|t| t.slave).collect();
    busy.sort_unstable();
    busy.dedup();
    for &i in busy.iter().filter(|&&i| i < spec.slaves.len()) {
        let s = &spec.slaves[i];
        let at = |field: &str| format!("/slaves/{i}/{field}");
        let si = Some(i as u32);
        let mailbox =
            spec.transfers.iter().any(|t| t.slave == i && !matches!(t.protocol, TransferProtocol::PdoChannel { .. }));
        let p = &s.pdi;
        out.push((
            format!(
                "PDI rate ×2 on {} (read {} → {}, write {} → {})",
                s.name,
                mbit(p.read_bps),
                mbit(p.read_bps * 2),
                mbit(p.write_bps),
                mbit(p.write_bps * 2)
            ),
            Lever::Pdi,
            si,
            None,
            vec![edit(at("pdi/read_bps"), json!(p.read_bps * 2)), edit(at("pdi/write_bps"), json!(p.write_bps * 2))],
        ));
        if p.access_overhead_ns >= 2 {
            out.push((
                format!(
                    "PDI access overhead on {} {} → {}",
                    s.name,
                    us(p.access_overhead_ns.into()),
                    us((p.access_overhead_ns / 2).into())
                ),
                Lever::Pdi,
                si,
                None,
                vec![edit(at("pdi/access_overhead_ns"), json!(p.access_overhead_ns / 2))],
            ));
        }
        if mailbox && p.mailbox_access == MailboxAccess::WholeBuffer {
            out.push((
                format!("{} reads and writes only the message, not the whole mailbox buffer", s.name),
                Lever::Pdi,
                si,
                None,
                vec![edit(at("pdi/mailbox_access"), json!("message_only"))],
            ));
        }
        let grown = |b: u16| b.saturating_mul(2).min(MAX_MAILBOX_BYTES).max(b);
        let (out_b, in_b) = (grown(s.mailbox_out_bytes), grown(s.mailbox_in_bytes));
        if mailbox && s.mailbox_out_bytes > 0 && (out_b, in_b) != (s.mailbox_out_bytes, s.mailbox_in_bytes) {
            let how = if out_b == MAX_MAILBOX_BYTES || in_b == MAX_MAILBOX_BYTES { "up to one frame" } else { "×2" };
            out.push((
                format!(
                    "Mailboxes on {} {how} (out {} → {out_b} B, in {} → {in_b} B)",
                    s.name, s.mailbox_out_bytes, s.mailbox_in_bytes
                ),
                Lever::Mailbox,
                si,
                None,
                vec![edit(at("mailbox_out_bytes"), json!(out_b)), edit(at("mailbox_in_bytes"), json!(in_b))],
            ));
        }
        if s.app_processing_ns >= 2 {
            out.push((
                format!(
                    "Application processing on {} {} → {}",
                    s.name,
                    us(s.app_processing_ns.into()),
                    us((s.app_processing_ns / 2).into())
                ),
                Lever::Application,
                si,
                None,
                vec![edit(at("app_processing_ns"), json!(s.app_processing_ns / 2))],
            ));
        }
    }

    for (k, t) in spec.transfers.iter().enumerate() {
        let tk = Some(k as u32);
        let si = Some(t.slave as u32);
        let path = format!("/transfers/{k}/protocol");
        let name = transfer_name(k, t);
        let mut proto =
            |label: String, value: Value| out.push((label, Lever::Protocol, si, tk, vec![edit(path.clone(), value)]));
        let alternatives = [
            json!({ "kind": "eoe", "mtu_bytes": 1500 }),
            json!({ "kind": "foe" }),
            json!({ "kind": "coe_segmented" }),
            json!({ "kind": "voe", "window": 16 }),
            json!({ "kind": "pdo_channel", "bytes_per_cycle": PDO_CHANNEL_BYTES, "window": 2, "retransmit_after_cycles": 8 }),
        ];
        let kind = serde_json::to_value(&t.protocol).ok().and_then(|v| v.get("kind").cloned());
        for alt in alternatives {
            if alt.get("kind") == kind.as_ref() {
                continue;
            }
            let Ok(p) = serde_json::from_value::<TransferProtocol>(alt.clone()) else { continue };
            proto(format!("{name} by {} instead", protocol_name(&p)), alt);
        }
        match &t.protocol {
            TransferProtocol::Eoe { mtu_bytes } if *mtu_bytes < 1500 => {
                proto(format!("{name}: MTU {mtu_bytes} → 1500 B"), json!({ "kind": "eoe", "mtu_bytes": 1500 }));
            }
            TransferProtocol::PdoChannel { bytes_per_cycle, window, retransmit_after_cycles } => {
                if *bytes_per_cycle <= u16::MAX / 2 {
                    proto(
                        format!("{name}: {bytes_per_cycle} → {} B per cycle", bytes_per_cycle * 2),
                        json!({ "kind": "pdo_channel", "bytes_per_cycle": bytes_per_cycle * 2, "window": window, "retransmit_after_cycles": retransmit_after_cycles }),
                    );
                }
                proto(
                    format!("{name}: window {window} → {} chunks", window * 2),
                    json!({ "kind": "pdo_channel", "bytes_per_cycle": bytes_per_cycle, "window": window * 2, "retransmit_after_cycles": retransmit_after_cycles }),
                );
            }
            _ => {}
        }
    }
    out
}

/// Summarise a simulation as a [`WhatIfRun`].
fn summarise(r: Result<SimResult, String>) -> WhatIfRun {
    let r = match r {
        Ok(r) => r,
        Err(reason) => {
            return WhatIfRun {
                feasible: false,
                reason: Some(reason),
                transfer_ns: Vec::new(),
                finish_ns: None,
                link_utilization_percent: None,
                pdi_busy_percent_max: None,
                dpram_used_percent_max: None,
            };
        }
    };
    let reason = r.transfers.iter().find(|t| !t.verified).map(|t| {
        if t.completed {
            format!("transfer {} arrived corrupt", t.index)
        } else {
            format!("transfer {} does not finish within {} ms", t.index, r.spec.max_duration_ms)
        }
    });
    let finish = r.transfers.iter().filter_map(|t| t.end_ns).fold(0.0, f64::max);
    WhatIfRun {
        feasible: reason.is_none(),
        finish_ns: reason.is_none().then_some(finish),
        reason,
        transfer_ns: r.transfers.iter().map(|t| t.duration_ns).collect(),
        link_utilization_percent: Some(r.link_utilization_percent),
        pdi_busy_percent_max: r.slaves.iter().map(|s| s.pdi_busy_percent).reduce(f64::max),
        dpram_used_percent_max: r
            .slaves
            .iter()
            .filter(|s| s.dpram_bytes > 0)
            .map(|s| 100.0 * f64::from(s.dpram_used_bytes) / f64::from(s.dpram_bytes))
            .reduce(f64::max),
    }
}

fn simulate(spec: &Value) -> Result<SimResult, String> {
    let sim: SimSpec = serde_json::from_value(spec.clone()).map_err(|e| format!("not a valid spec: {e}"))?;
    flowlab_sim::simulate(&sim, None::<std::io::Sink>).map_err(|e| e.to_string())
}

/// Simulate `spec` and every one-change variant of it (see the module docs), best first, and
/// rank the changes by their effect on each transfer and on the whole run.
pub fn what_if(spec: &SimSpec, o: &WhatIfOptions) -> Result<WhatIfResult, StudyError> {
    if !(0.0..1.0).contains(&o.no_effect) {
        return Err(StudyError::Spec(format!("no_effect must be at least 0 and below 1, not {}", o.no_effect)));
    }
    if spec.transfers.is_empty() {
        return Err(StudyError::Spec("the scenario has no transfers to speed up".into()));
    }
    let mut base = serde_json::to_value(spec).map_err(|e| StudyError::Spec(e.to_string()))?;
    if let Some(c) = base.pointer_mut("/capture/enabled") {
        *c = Value::Bool(false);
    }
    let mut changes = candidates(spec);
    let mut parts: Vec<Vec<String>> = vec![Vec::new(); changes.len()];
    // Two limits can coincide (a PDI that needs most of a cycle per message, and a cycle per
    // message): then no single change helps, but a pair does. Try pairs of non-protocol changes
    // that set different values.
    let paired: Vec<usize> = (0..changes.len()).filter(|&i| changes[i].1 != Lever::Protocol).take(MAX_PAIRED).collect();
    for (k, &a) in paired.iter().enumerate() {
        for &b in &paired[k + 1..] {
            let (ca, cb) = (&changes[a], &changes[b]);
            if ca.4.iter().any(|x| cb.4.iter().any(|y| x.path == y.path)) {
                continue;
            }
            let slave = if ca.2 == cb.2 { ca.2 } else { None };
            let edits = ca.4.iter().chain(&cb.4).cloned().collect();
            let label = format!("{} + {}", ca.0, cb.0);
            parts.push(vec![ca.0.clone(), cb.0.clone()]);
            changes.push((label, Lever::Combined, slave, None, edits));
        }
    }
    let mut specs = vec![base.clone()];
    for (_, _, _, _, edits) in &changes {
        let mut v = base.clone();
        for e in edits {
            if let Some(slot) = v.pointer_mut(&e.path) {
                *slot = e.value.clone();
            }
        }
        specs.push(v);
    }
    let mut runs = par_map(&specs, |s| summarise(simulate(s))).into_iter();
    let base_run = runs.next().expect("the base run");
    let mut changes: Vec<WhatIf> = changes
        .into_iter()
        .zip(parts)
        .zip(runs)
        .map(|(((label, lever, slave, transfer, edits), parts), run)| WhatIf {
            label,
            lever,
            slave,
            transfer,
            edits,
            parts,
            run,
        })
        .collect();
    changes.sort_by(|a, b| match (a.run.finish_ns, b.run.finish_ns) {
        (Some(x), Some(y)) => x.total_cmp(&y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });

    let json = serde_json::to_vec(spec).unwrap_or_default();
    let input = InputRef::of("scenario.json", &json);
    let assumptions = vec![
        "Each change is a separate simulation of the scenario with one parameter doubled or halved, or one choice switched; the simulator's own assumptions apply to each run.".to_owned(),
        format!("Pairs of the first {MAX_PAIRED} changes that are not protocol switches are also simulated together, since two limits can coincide; larger combinations are not tried."),
        format!(
            "The opportunistic policy tried uses {:.0} % of the cycle and a {} master turnaround; a PDO channel tried in place of a mailbox protocol carries {PDO_CHANNEL_BYTES} B per cycle with a window of 2. These are model choices, not properties of a particular master.",
            OPPORTUNISTIC.0,
            us(OPPORTUNISTIC.1.into())
        ),
        "Whether a slave or master supports a change (a faster SPI clock, larger mailboxes, another protocol) is not checked beyond the ESC RAM the SyncManagers need.".into(),
        format!(
            "A change within {} % of the scenario's time counts as having no effect; a pair is listed when it beats both of its parts alone, and the scenario, by more than that.",
            100.0 * o.no_effect
        ),
    ];
    let rankings = std::iter::once(None)
        .chain((0..spec.transfers.len() as u32).map(Some))
        .map(|t| ranking(&base_run, &changes, t, o))
        .collect();
    Ok(WhatIfResult {
        meta: ReportMeta::new(Provenance::Simulated, input, assumptions),
        base: base_run,
        changes,
        no_effect: o.no_effect,
        rankings,
    })
}
