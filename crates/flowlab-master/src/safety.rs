//! The checks before anything is sent: an acknowledged isolated segment on a named interface,
//! and no other master heard on it.
//!
//! [`Session::open`](crate::hardware::Session::open) takes a [`QuietSegment`], and only
//! [`check_segment`] makes one, so no caller can send frames without these checks:
//!
//! 1. the interface is named and the user has stated that it is on an isolated lab segment
//!    ([`SegmentCheck::isolated_lab_network`]);
//! 2. a capture tool (dumpcap or tcpdump, through `flowlab-live`) listens on it with the
//!    EtherCAT filter for [`SegmentCheck::listen`], and any EtherCAT frame heard refuses the
//!    start: another master, or something else sending EtherCAT, is active;
//! 3. without a working capture tool the start is refused unless
//!    [`SegmentCheck::no_master_check`] is set.
//!
//! The listening check only hears a master that sends while it listens; it is a guard, not a
//! proof.

use std::time::Duration;

use flowlab_live::{ETHERCAT_FILTER, LiveOptions, LiveSession, LiveState, Source, Tool};

use crate::MasterError;

/// What the user asked for before anything is sent.
#[derive(Debug, Clone, PartialEq)]
pub struct SegmentCheck {
    /// The network interface the slaves are on.
    pub interface: String,
    /// The user's statement that the interface is on an isolated lab segment: no other master,
    /// no production equipment.
    pub isolated_lab_network: bool,
    /// How long to listen for another master's frames, including the capture tool's start-up.
    pub listen: Duration,
    /// Capture tool for the check; `None` picks dumpcap if installed, else tcpdump.
    pub tool: Option<Tool>,
    /// Start even though no capture tool can check for another master.
    pub no_master_check: bool,
}

/// Evidence that [`check_segment`] passed for one interface. Only [`check_segment`] creates
/// it; [`Session::open`](crate::hardware::Session::open) requires it.
#[derive(Debug)]
pub struct QuietSegment {
    interface: String,
    notes: Vec<String>,
}

impl QuietSegment {
    /// The interface that was checked.
    pub fn interface(&self) -> &str {
        &self.interface
    }

    /// What the check did, or why it was skipped (`no_master_check`), for the user to read.
    pub fn notes(&self) -> &[String] {
        &self.notes
    }
}

/// Run the checks of the module documentation. `progress` receives a line before the listening
/// starts, since it takes [`SegmentCheck::listen`].
pub fn check_segment(c: &SegmentCheck, mut progress: impl FnMut(&str)) -> Result<QuietSegment, MasterError> {
    let refuse = |m: String| Err(MasterError::Refused(m));
    if c.interface.trim().is_empty() {
        return refuse("refusing to send: no network interface named".into());
    }
    if !c.isolated_lab_network {
        return refuse(format!(
            "refusing to send on {}: the lab master sends EtherCAT frames. Confirm (--isolated-lab-network) that this \
             interface is on an isolated lab segment with no other master and no production equipment.",
            c.interface
        ));
    }
    let quiet = |notes: Vec<String>| Ok(QuietSegment { interface: c.interface.clone(), notes });
    let (tool, path) = match flowlab_live::find_tool(c.tool) {
        Ok(found) => found,
        Err(e) if c.no_master_check => {
            return quiet(vec![format!("not checking for another master ({e}); --no-master-check given")]);
        }
        Err(e) => {
            return refuse(format!(
                "cannot check {} for another master: {e}. Install Wireshark's dumpcap or tcpdump, or pass \
                 --no-master-check if you are certain the segment is quiet.",
                c.interface
            ));
        }
    };
    progress(&format!(
        "listening on {} for {:.1} s with {} for another master's frames…",
        c.interface,
        c.listen.as_secs_f64(),
        tool.name()
    ));
    let options = LiveOptions { tick: Duration::from_millis(250), save_to: None, ..LiveOptions::default() };
    let session = LiveSession::start(Source::interface(tool, path, &c.interface, Some(ETHERCAT_FILTER)), options)
        .map_err(|e| MasterError::Refused(format!("cannot check {} for another master: {e}", c.interface)))?;
    std::thread::sleep(c.listen);
    let done = session.stop();
    if done.packets > 0 {
        return refuse(format!(
            "{} EtherCAT frame(s) on {} in {:.1} s: another master (or something sending EtherCAT) is active. \
             The lab master does not start next to another one; stop it first.",
            done.packets,
            c.interface,
            c.listen.as_secs_f64()
        ));
    }
    if done.state == LiveState::Failed {
        let why = done.message.unwrap_or_else(|| "the capture tool failed".into());
        if !c.no_master_check {
            return refuse(format!(
                "cannot check {} for another master: {why}. Fix the capture permissions, or pass \
                 --no-master-check if you are certain the segment is quiet.",
                c.interface
            ));
        }
        return quiet(vec![format!("the check for another master failed ({why}); --no-master-check given")]);
    }
    quiet(vec!["no EtherCAT frames heard; starting".into()])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(interface: &str, isolated_lab_network: bool) -> SegmentCheck {
        SegmentCheck {
            interface: interface.into(),
            isolated_lab_network,
            listen: Duration::ZERO,
            tool: None,
            no_master_check: true,
        }
    }

    #[test]
    fn nothing_is_sent_without_the_acknowledgement_or_an_interface() {
        let mut heard = Vec::new();
        let e = check_segment(&check("eth9", false), |m| heard.push(m.to_owned())).unwrap_err().to_string();
        assert!(e.contains("--isolated-lab-network"), "{e}");
        let e = check_segment(&check(" ", true), |m| heard.push(m.to_owned())).unwrap_err().to_string();
        assert!(e.contains("no network interface"), "{e}");
        assert!(heard.is_empty(), "nothing listened: {heard:?}");
    }
}
