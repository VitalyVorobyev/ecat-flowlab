//! EoE to an Ethernet interface: the frames the master sends by EoE go out on a port, and the
//! frames the slave's network interface sends come back to the master as EoE.
//!
//! [`EoeBridge::pump`] moves frames between a [`PdiSlave`] and a [`FramePort`]. On Linux the
//! port is one end of a veth pair (`wire::EthernetPort`, feature `wire`) whose other
//! end, the *host* interface, carries the slave's IP address, so the host's ordinary TCP/IP
//! stack and services answer the master. The port also sees the frames it sent itself; only
//! frames with the host interface's source MAC go to the master.
//!
//! ```
//! use flowlab_slave::bridge::{EoeBridge, FramePort};
//! use flowlab_slave::SlaveError;
//!
//! /// A port that has nothing to say.
//! struct Quiet;
//! impl FramePort for Quiet {
//!     fn try_recv(&mut self) -> Result<Option<Vec<u8>>, SlaveError> { Ok(None) }
//!     fn send(&mut self, _: &[u8]) -> Result<(), SlaveError> { Ok(()) }
//! }
//! let bridge = EoeBridge::new(Quiet, [0x02, 0, 0, 0, 0, 0x02]);
//! assert_eq!(bridge.stats().to_master, 0);
//! ```

use flowlab_mailbox::msg::EoeIpParameters;

use crate::SlaveError;
use crate::pdi::{EscPdi, PdiSlave};

/// Messages waiting for "mailbox in" before the bridge stops taking frames from the port.
pub const BRIDGE_OUTBOX: usize = 8;

/// Where the bridge sends the master's frames and takes the host's from.
pub trait FramePort {
    /// A frame, if one is waiting; never blocks.
    fn try_recv(&mut self) -> Result<Option<Vec<u8>>, SlaveError>;
    /// Send a frame.
    fn send(&mut self, frame: &[u8]) -> Result<(), SlaveError>;
}

/// Frames the bridge has moved.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BridgeStats {
    /// Frames from the master to the port.
    pub to_host: u64,
    /// Their bytes.
    pub to_host_bytes: u64,
    /// Frames from the host to the master.
    pub to_master: u64,
    /// Their bytes.
    pub to_master_bytes: u64,
}

/// An EoE bridge between a [`PdiSlave`] and a [`FramePort`] (see the [module docs](self)).
#[derive(Debug)]
pub struct EoeBridge<P> {
    port: P,
    host_mac: [u8; 6],
    stats: BridgeStats,
}

impl<P: FramePort> EoeBridge<P> {
    /// A bridge to `port` that forwards to the master the frames with source MAC `host_mac`.
    pub fn new(port: P, host_mac: [u8; 6]) -> Self {
        Self { port, host_mac, stats: BridgeStats::default() }
    }

    /// The host interface's MAC changed (after an EoE Set IP Parameter request set it).
    pub fn set_host_mac(&mut self, mac: [u8; 6]) {
        self.host_mac = mac;
    }

    /// Frames moved so far.
    pub fn stats(&self) -> BridgeStats {
        self.stats
    }

    /// The port.
    pub fn port(&self) -> &P {
        &self.port
    }

    /// Move the master's EoE frames to the port and the host's frames from the port to the
    /// slave, at most until [`BRIDGE_OUTBOX`] messages wait for "mailbox in". Returns the EoE
    /// Set IP Parameter requests the slave answered since the last call, for the caller to
    /// apply to the host interface.
    pub fn pump<E: EscPdi>(&mut self, slave: &mut PdiSlave<E>) -> Result<Vec<EoeIpParameters>, SlaveError> {
        let set_ip = slave.take_set_ip();
        for f in slave.take_eoe_frames() {
            self.port.send(&f)?;
            self.stats.to_host += 1;
            self.stats.to_host_bytes += f.len() as u64;
        }
        while slave.outbox_len() < BRIDGE_OUTBOX {
            let Some(f) = self.port.try_recv()? else { break };
            if f.len() >= 14 && f[6..12] == self.host_mac {
                self.stats.to_master += 1;
                self.stats.to_master_bytes += f.len() as u64;
                slave.send_eoe_frame(&f);
            }
        }
        Ok(set_ip)
    }
}

/// A MAC address in the `aa:bb:cc:dd:ee:ff` form.
pub fn parse_mac(text: &str) -> Option<[u8; 6]> {
    let mut mac = [0u8; 6];
    let mut parts = text.trim().split(':');
    for m in &mut mac {
        let p = parts.next()?;
        if p.is_empty() || p.len() > 2 {
            return None;
        }
        *m = u8::from_str_radix(p, 16).ok()?;
    }
    parts.next().is_none().then_some(mac)
}

/// The MAC address of network interface `name`, from `/sys/class/net/NAME/address` (Linux).
pub fn interface_mac(name: &str) -> Result<[u8; 6], SlaveError> {
    let path = format!("/sys/class/net/{name}/address");
    let text = std::fs::read_to_string(&path).map_err(|e| SlaveError::Io(format!("{path}: {e}")))?;
    parse_mac(&text).ok_or_else(|| SlaveError::Io(format!("{path}: `{}` is not a MAC address", text.trim())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macs_parse_in_the_colon_form_only() {
        assert_eq!(parse_mac("02:00:00:00:00:0a\n"), Some([2, 0, 0, 0, 0, 10]));
        assert_eq!(parse_mac("02:00:00:00:00"), None);
        assert_eq!(parse_mac("02:00:00:00:00:00:01"), None);
        assert_eq!(parse_mac("02-00-00-00-00-00"), None);
        assert_eq!(parse_mac("002:00:00:00:00:00"), None);
    }
}
