//! Answering EtherCAT frames on a network interface.
//!
//! **This sends frames.** [`serve`] opens a raw socket on the named interface (Linux:
//! `AF_PACKET`, needs `CAP_NET_RAW` or root; macOS: BPF) through `pnet_datalink` (MIT OR
//! Apache-2.0), and answers every EtherCAT frame from a master with the frame the emulated
//! slave returns. It refuses to start without the acknowledgement that the interface is on an
//! isolated lab network: on a real segment it would answer a real master's frames a second
//! time. Its own answers, which a macOS BPF socket hands back to it, are recognised by their
//! bytes and not answered again; a Linux packet socket does not return its own frames.

use std::time::{Duration, Instant};

use ecat_wire::AlState;
use pnet_datalink::{Channel, Config};

use crate::echo::Echoes;
use crate::{EmulatedSlave, SlaveError};

/// Whether the socket hands this host's own frames back: BPF does, `AF_PACKET` does not.
const OWN_FRAMES_COME_BACK: bool = !cfg!(target_os = "linux");

/// Settings of [`serve`].
#[derive(Debug, Clone)]
pub struct ServeOptions {
    /// Network interface (Linux: e.g. `eth0`; macOS: e.g. `en7`).
    pub interface: String,
    /// The acknowledgement that the interface is on an isolated lab network: no production
    /// equipment, no EtherCAT slaves besides this one.
    pub isolated_lab_network: bool,
    /// Stop after this long; `None`: run until the process ends.
    pub duration: Option<Duration>,
}

/// What [`serve`] reports while it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// The socket is open; frames are answered from now on.
    Listening,
    /// The AL state changed (state, AL status code if the error flag is set).
    State(AlState, Option<u16>),
    /// A second passed with frames answered.
    Second,
}

/// The names of the interfaces this host has, for error messages.
pub fn interface_names() -> Vec<String> {
    pnet_datalink::interfaces().into_iter().map(|i| i.name).collect()
}

/// Answer frames on `o.interface` until `o.duration` has passed (see the [module docs](self)).
pub fn serve(
    slave: &mut EmulatedSlave,
    o: &ServeOptions,
    mut on_event: impl FnMut(&EmulatedSlave, Event),
) -> Result<(), SlaveError> {
    if !o.isolated_lab_network {
        return Err(SlaveError::Refused(
            "the emulated slave answers EtherCAT frames, so it sends frames: confirm that the interface is on an \
             isolated lab network (no production equipment, no other EtherCAT slaves) with --isolated-lab-network"
                .into(),
        ));
    }
    let Some(iface) = pnet_datalink::interfaces().into_iter().find(|i| i.name == o.interface) else {
        return Err(SlaveError::Invalid(format!(
            "no interface `{}` (this host has: {})",
            o.interface,
            interface_names().join(", ")
        )));
    };
    let cfg = Config {
        read_timeout: Some(Duration::from_millis(100)),
        write_buffer_size: 1 << 16,
        read_buffer_size: 1 << 16,
        promiscuous: true,
        ..Config::default()
    };
    let (mut tx, mut rx) = match pnet_datalink::channel(&iface, cfg) {
        Ok(Channel::Ethernet(tx, rx)) => (tx, rx),
        Ok(_) => return Err(SlaveError::Io(format!("{}: not an Ethernet channel", o.interface))),
        Err(e) => return Err(SlaveError::Io(format!("cannot open {}: {e}", o.interface))),
    };
    on_event(slave, Event::Listening);
    let origin = Instant::now();
    let mut state = slave.al_state();
    let mut second = origin;
    let mut answered_since = 0u64;
    let mut echoes = Echoes::default();
    loop {
        if o.duration.is_some_and(|d| origin.elapsed() >= d) {
            return Ok(());
        }
        match rx.next() {
            Ok(frame) => {
                let at = origin.elapsed();
                let echo = OWN_FRAMES_COME_BACK && echoes.is_echo(at, frame);
                let back = if echo { None } else { slave.handle(frame, at.as_nanos() as u64) };
                if let Some(back) = back {
                    match tx.send_to(&back, None) {
                        Some(Ok(())) => {
                            if OWN_FRAMES_COME_BACK {
                                echoes.sent(origin.elapsed(), &back);
                            }
                            answered_since += 1;
                        }
                        Some(Err(e)) => return Err(SlaveError::Io(format!("send on {}: {e}", o.interface))),
                        None => return Err(SlaveError::Io(format!("send on {}: no buffer", o.interface))),
                    }
                    let s = slave.al_state();
                    if s != state {
                        state = s;
                        on_event(slave, Event::State(s.0, s.1));
                    }
                }
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock) => {}
            Err(e) => return Err(SlaveError::Io(format!("receive on {}: {e}", o.interface))),
        }
        if second.elapsed() >= Duration::from_secs(1) {
            second = Instant::now();
            if answered_since > 0 {
                answered_since = 0;
                on_event(slave, Event::Second);
            }
        }
    }
}
