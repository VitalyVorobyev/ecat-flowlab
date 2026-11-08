//! Findings and assumptions of the mailbox analysis.

use std::collections::BTreeMap;

use ecat_wire::sdo_abort_text;

use super::*;

pub(super) fn plural(n: u64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

pub(crate) fn findings(m: &MailboxReport, out: &mut Vec<Finding>) {
    let mut add = |severity, title: String, detail: String| out.push(Finding { severity, title, detail });
    let coe = &m.coe;
    if coe.protocol_errors > 0 {
        add(
            Severity::Defect,
            format!("{} violated the protocol", plural(coe.protocol_errors, "SDO transfer", "SDO transfers")),
            "Toggle bit, size or response sequence did not match the request. Threshold: any.".into(),
        );
    }
    if coe.aborted > 0 {
        let mut codes: BTreeMap<u32, u64> = BTreeMap::new();
        for t in coe.transactions.iter().filter(|t| t.outcome == SdoOutcome::Aborted) {
            *codes.entry(t.abort_code.unwrap_or(0)).or_default() += 1;
        }
        let (code, n) = codes.iter().max_by_key(|(_, n)| **n).map(|(c, n)| (*c, *n)).unwrap_or((0, 0));
        add(
            Severity::Warn,
            format!("{} aborted", plural(coe.aborted, "SDO transfer", "SDO transfers")),
            format!(
                "Most frequent: 0x{code:08X} {} ({n}×). Masters probing optional objects cause aborts too. Threshold: any.",
                sdo_abort_text(code).unwrap_or("")
            ),
        );
    }
    if coe.emergencies_total > 0 {
        add(
            Severity::Warn,
            plural(coe.emergencies_total, "CoE emergency", "CoE emergencies"),
            "Slaves reported emergency messages; see the Mailbox view for codes. Threshold: any.".into(),
        );
    }
    if coe.unfinished > 0 {
        add(
            Severity::Info,
            format!("{} unfinished", plural(coe.unfinished, "SDO transfer", "SDO transfers")),
            "Superseded by a new request or still open at the end of the capture.".into(),
        );
    }
    let foe_bad = m.foe.iter().filter(|t| t.outcome == "error" || t.outcome == "protocol_error").count();
    if foe_bad > 0 {
        add(
            Severity::Warn,
            format!("{} failed", plural(foe_bad as u64, "FoE transfer", "FoE transfers")),
            "An FoE error reply or a packet-number gap ended the transfer. Threshold: any.".into(),
        );
    }
    if m.eoe.reassembly_errors > 0 {
        add(
            Severity::Warn,
            format!("{} could not be reassembled", plural(m.eoe.reassembly_errors, "EoE frame", "EoE frames")),
            "Fragments were missing, out of order, or the capture ended mid-frame. Threshold: any.".into(),
        );
    }
    if !m.errors.is_empty() {
        add(
            Severity::Warn,
            plural(m.errors.len() as u64, "mailbox error reply", "mailbox error replies"),
            format!("First: station 0x{:04X}, {}. Threshold: any.", m.errors[0].station, m.errors[0].text),
        );
    }
    if m.malformed > 0 {
        add(
            Severity::Warn,
            plural(m.malformed, "malformed mailbox message", "malformed mailbox messages"),
            "Data read from or written to a mailbox address is not a valid mailbox header.".into(),
        );
    }
    let rejected: u64 = m.channels.iter().map(|c| c.writes_rejected).sum();
    if rejected > 0 {
        add(
            Severity::Info,
            format!("{} rejected", plural(rejected, "mailbox write", "mailbox writes")),
            "The slave's mailbox was still full (WKC 0); masters retry. Many rejections mean the slave is slow to read its mailbox.".into(),
        );
    }
}

pub(crate) fn assumptions(m: &MailboxReport) -> Vec<String> {
    let mut a = vec![
        "Each station's mailbox SyncManagers come, in this order of preference, from SyncManager configuration writes in the capture (mailbox mode), the ENI, a user hint or a heuristic; the source is reported per channel. Messages are taken from returning datagrams at the mailbox start address with WKC > 0.".to_owned(),
        "Logically mapped mailbox access and the SyncManager repeat mechanism are not interpreted; a message whose counter equals the previous one is treated as a repeat.".into(),
    ];
    if m.channels.iter().any(|c| c.source == ChannelSource::Eni) {
        a.push("Mailboxes of stations without SyncManager configuration in the capture come from the ENI; its \"Recv\" mailbox is taken to be SM1 for status polls.".into());
    }
    if m.channels.iter().any(|c| c.source == ChannelSource::Heuristic) {
        a.push("Some stations had no SyncManager configuration in the capture; their mailboxes were inferred from FPWR/FPRD at ≥ 0x1000 whose data parses as a mailbox header.".into());
    }
    a
}
