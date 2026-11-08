//! How a master schedules mailbox datagrams per cycle.

use std::fmt;
use std::str::FromStr;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// How the master decides which mailbox datagrams go on the wire. None of these is what every
/// master does; compare them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[ts(export)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MailboxPolicy {
    /// Up to `accesses_per_cycle` mailbox datagrams (writes and reads together) per cycle,
    /// shared round-robin across slaves, in one frame after the cyclic frame.
    FixedQuota {
        /// Mailbox datagrams per cycle.
        accesses_per_cycle: u32,
    },
    /// Each cycle, `slaves_per_cycle` slaves with mailbox work get one write and one read.
    RoundRobin {
        /// Slaves served per cycle.
        slaves_per_cycle: u32,
    },
    /// Mailbox frames back to back while the cycle has spare time: after each mailbox frame
    /// returns, another is sent if it would start within `bus_budget_percent` of the cycle.
    Opportunistic {
        /// Share of the cycle the master may fill with mailbox frames, percent.
        bus_budget_percent: f64,
        /// Master turnaround between a frame's return and the next frame, ns.
        turnaround_ns: u32,
    },
}

impl Default for MailboxPolicy {
    fn default() -> Self {
        Self::FixedQuota { accesses_per_cycle: 2 }
    }
}

/// The policy in words, as simulated and measured results name it.
impl fmt::Display for MailboxPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FixedQuota { accesses_per_cycle } => {
                write!(f, "fixed quota: {accesses_per_cycle} mailbox datagrams per cycle")
            }
            Self::RoundRobin { slaves_per_cycle } => {
                write!(
                    f,
                    "round robin: {slaves_per_cycle} slave{} per cycle",
                    if *slaves_per_cycle == 1 { "" } else { "s" }
                )
            }
            Self::Opportunistic { bus_budget_percent, turnaround_ns } => write!(
                f,
                "opportunistic: mailbox frames within {bus_budget_percent} % of the cycle, {turnaround_ns} ns turnaround"
            ),
        }
    }
}

/// The short form of the command line: `fixed-quota:N`, `round-robin:N` or
/// `opportunistic:PERCENT[,TURNAROUND_NS]` (turnaround 10000 ns when left out).
impl FromStr for MailboxPolicy {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let (kind, rest) = s.split_once(':').unwrap_or((s, ""));
        let bad = || format!("`{s}`: expected fixed-quota:N, round-robin:N or opportunistic:PERCENT[,TURNAROUND_NS]");
        Ok(match kind {
            "fixed-quota" => Self::FixedQuota { accesses_per_cycle: rest.parse().map_err(|_| bad())? },
            "round-robin" => Self::RoundRobin { slaves_per_cycle: rest.parse().map_err(|_| bad())? },
            "opportunistic" => {
                let (p, t) = rest.split_once(',').unwrap_or((rest, "10000"));
                Self::Opportunistic {
                    bus_budget_percent: p.parse().map_err(|_| bad())?,
                    turnaround_ns: t.parse().map_err(|_| bad())?,
                }
            }
            _ => return Err(bad()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_forms_parse_and_print() {
        assert_eq!("round-robin:1".parse(), Ok(MailboxPolicy::RoundRobin { slaves_per_cycle: 1 }));
        assert_eq!(
            "opportunistic:60".parse(),
            Ok(MailboxPolicy::Opportunistic { bus_budget_percent: 60.0, turnaround_ns: 10_000 })
        );
        assert!("greedy".parse::<MailboxPolicy>().is_err());
        assert!("fixed-quota:x".parse::<MailboxPolicy>().is_err());
        assert_eq!(MailboxPolicy::RoundRobin { slaves_per_cycle: 1 }.to_string(), "round robin: 1 slave per cycle");
        assert_eq!(MailboxPolicy::default().to_string(), "fixed quota: 2 mailbox datagrams per cycle");
    }

    #[test]
    fn json_form_is_tagged() {
        let p: MailboxPolicy = serde_json::from_str(r#"{ "kind": "round_robin", "slaves_per_cycle": 2 }"#).unwrap();
        assert_eq!(p, MailboxPolicy::RoundRobin { slaves_per_cycle: 2 });
        assert!(
            serde_json::from_str::<MailboxPolicy>(r#"{ "kind": "fixed_quota", "accesses_per_cycle": 1, "x": 1 }"#)
                .is_err()
        );
    }
}
