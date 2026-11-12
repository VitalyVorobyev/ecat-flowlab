//! EoE gateway: the master's own network interface on a slave's EoE link.
//!
//! [`run_gateway`] keeps one slave's mailbox busy for as long as it runs. It sends an EoE Set
//! IP Parameter request first (if given one), then moves Ethernet frames both ways as EoE
//! fragments: every pass writes the next fragment for the slave (if any) and reads "mailbox
//! in", in flight together. On the master's side the frames end in a small TCP/IP stack
//! (smoltcp) with one IPv4 address on the slave's subnet. Local TCP ports on 127.0.0.1 are
//! forwarded through it to the slave's ports, so ordinary tools (curl, a browser, gRPC
//! clients) reach the slave with no host network interface and no root.
//!
//! The master's TCP is smoltcp's, not the host operating system's: its maximum segment size
//! follows the gateway's MTU, its receive window the socket buffer, and its acknowledgements
//! the configured delay (with a delay, an acknowledgement goes out at the latest when more
//! than one segment is unacknowledged, as most stacks do).

use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ecat_wire::{EoeHeader, EoeType, MAILBOX_HEADER_LEN, MailboxHeader, MailboxType};
use flowlab_core::{InputRef, Provenance, ReportMeta};
use flowlab_mailbox::msg::{self, EoeIpParameters, EoeReassembler};
use serde::Serialize;
use smoltcp::iface::{Config, Interface, SocketHandle, SocketSet};
use smoltcp::phy::{Device, DeviceCapabilities, Medium};
use smoltcp::socket::tcp;
use smoltcp::time::{Duration as SDuration, Instant as SInstant};
use smoltcp::wire::{EthernetAddress, HardwareAddress, IpAddress, IpCidr, Ipv4Address};

use crate::MasterError;
use crate::bus::{BusError, MailboxBus, MailboxLayout, read_mailbox, write_mailbox};
use crate::result::Summary;

/// Frames the master's stack may queue for the slave before it waits.
const TX_QUEUE: usize = 8;
/// Width of a timeline bucket, ns.
const BUCKET_NS: u64 = 100_000_000;
/// Most access times kept for the summaries.
const MAX_SAMPLES: usize = 2_000_000;
/// Bytes moved from a local client per read.
const BUF: usize = 16 * 1024;

/// A local TCP port (on 127.0.0.1) forwarded to a port of the slave.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PortForward {
    /// Port on 127.0.0.1.
    pub local_port: u16,
    /// Port at the slave's address.
    pub remote_port: u16,
}

/// How a gateway runs.
#[derive(Debug, Clone)]
pub struct GatewayConfig {
    /// The slave's mailbox.
    pub mailbox: MailboxLayout,
    /// Send this Set IP Parameter request first.
    pub set_ip: Option<EoeIpParameters>,
    /// The slave's IPv4 address (where forwarded connections go).
    pub slave_ip: [u8; 4],
    /// The master's MAC address on the EoE link.
    pub master_mac: [u8; 6],
    /// The master's IPv4 address on the EoE link.
    pub master_ip: [u8; 4],
    /// Prefix length of the subnet.
    pub prefix_len: u8,
    /// IP MTU of the master's interface, bytes (its TCP segments carry up to MTU − 40).
    pub mtu: u16,
    /// Forwarded ports.
    pub forwards: Vec<PortForward>,
    /// Receive and send buffer of each forwarded connection, bytes (the receive window).
    pub tcp_buffer_bytes: usize,
    /// Delay of the master's TCP acknowledgements, ms; `None` acknowledges every segment at
    /// once.
    pub ack_delay_ms: Option<u64>,
    /// One pass (a write if a message waits, and a read) per cycle of this length, ns, as a
    /// cyclic master serves the mailbox; 0: passes back to back.
    pub cycle_ns: u64,
    /// Stop after this long, ns.
    pub max_duration_ns: u64,
    /// Stop when this becomes true (e.g. on Ctrl-C).
    pub stop: Arc<AtomicBool>,
    /// Assumptions of the bus implementation, added to the result's.
    pub assumptions: Vec<String>,
}

/// One forwarded connection.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct GatewayConnection {
    /// Local port it came in on.
    pub local_port: u16,
    /// Port at the slave.
    pub remote_port: u16,
    /// When the local client connected, ns after the gateway started.
    pub opened_ns: f64,
    /// When the slave's TCP accepted the connection, ns.
    pub established_ns: Option<f64>,
    /// When it ended, ns.
    pub closed_ns: Option<f64>,
    /// Bytes the client sent to the slave.
    pub bytes_to_slave: u64,
    /// Bytes the slave sent to the client.
    pub bytes_from_slave: u64,
    /// First and last byte from the slave, ns.
    pub first_byte_from_slave_ns: Option<f64>,
    /// Last byte from the slave, ns.
    pub last_byte_from_slave_ns: Option<f64>,
    /// Bytes from the slave over the time from its first to its last byte, bit/s.
    pub goodput_from_slave_bps: Option<f64>,
    /// Why it ended, if not a normal close.
    pub error: Option<String>,
}

/// Traffic in one timeline bucket.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct GatewayBucket {
    /// Mailbox messages the slave accepted.
    pub messages_to_slave: u64,
    /// Mailbox messages read from the slave.
    pub messages_from_slave: u64,
    /// Ethernet frame bytes completed towards the slave (all fragments accepted).
    pub frame_bytes_to_slave: u64,
    /// Ethernet frame bytes reassembled from the slave.
    pub frame_bytes_from_slave: u64,
}

/// What a gateway run did, measured.
#[derive(Debug, Clone, Serialize)]
pub struct GatewayResult {
    /// Provenance `measured`.
    pub meta: ReportMeta,
    /// Station address of the slave.
    pub station: u16,
    /// Slave name.
    pub slave: String,
    /// "Mailbox out" length, bytes.
    pub mailbox_out_bytes: u16,
    /// "Mailbox in" length, bytes.
    pub mailbox_in_bytes: u16,
    /// The master's address on the link.
    pub master_ip: String,
    /// The slave's address.
    pub slave_ip: String,
    /// IP MTU of the master's interface, bytes.
    pub mtu_bytes: u16,
    /// Cycle of the passes, ns (0: back to back).
    pub cycle_ns: f64,
    /// Delay of the master's TCP acknowledgements, ms (`None`: immediate).
    pub ack_delay_ms: Option<u64>,
    /// TCP buffer per connection, bytes.
    pub tcp_buffer_bytes: usize,
    /// Result code of the Set IP Parameter response (0: success), if one came.
    pub set_ip_result: Option<u16>,
    /// When it came, ns after the start.
    pub set_ip_reply_ns: Option<f64>,
    /// How long the gateway ran, ns.
    pub duration_ns: f64,
    /// Mailbox messages the slave accepted.
    pub messages_to_slave: u64,
    /// Mailbox messages read from the slave.
    pub messages_from_slave: u64,
    /// Writes the slave did not take (WKC 0: "mailbox out" still full).
    pub rejected_writes: u64,
    /// Reads that found "mailbox in" empty.
    pub empty_reads: u64,
    /// Accesses that did not come back.
    pub lost_frames: u64,
    /// Messages from the slave that repeated the previous one's counter (dropped).
    pub duplicate_messages: u64,
    /// Messages of other protocols, or EoE messages other than fragments and the Set IP
    /// response.
    pub other_messages: u64,
    /// Ethernet frames sent to the slave.
    pub frames_to_slave: u64,
    /// Ethernet frames reassembled from the slave.
    pub frames_from_slave: u64,
    /// Bytes of those frames.
    pub frame_bytes_to_slave: u64,
    /// Bytes of those frames.
    pub frame_bytes_from_slave: u64,
    /// Fragments that did not continue a frame, and frames abandoned (dropped).
    pub broken_fragments: u64,
    /// Round trip of the writes, host timing.
    pub write_access: Summary,
    /// Round trip of the reads, host timing.
    pub read_access: Summary,
    /// Every forwarded connection.
    pub connections: Vec<GatewayConnection>,
    /// Width of a timeline bucket, ns.
    pub timeline_bucket_ns: f64,
    /// Traffic per bucket, from the start.
    pub timeline: Vec<GatewayBucket>,
}

/// The EoE link as smoltcp's network device: frames from the slave wait in `rx`, frames for
/// it in `tx`.
struct EoeDevice {
    rx: VecDeque<Vec<u8>>,
    tx: VecDeque<Vec<u8>>,
    mtu: usize,
}

struct RxFrame(Vec<u8>);
struct TxSlot<'a>(&'a mut VecDeque<Vec<u8>>);

impl smoltcp::phy::RxToken for RxFrame {
    fn consume<R, F: FnOnce(&[u8]) -> R>(self, f: F) -> R {
        f(&self.0)
    }
}

impl smoltcp::phy::TxToken for TxSlot<'_> {
    fn consume<R, F: FnOnce(&mut [u8]) -> R>(self, len: usize, f: F) -> R {
        let mut b = vec![0; len];
        let r = f(&mut b);
        self.0.push_back(b);
        r
    }
}

impl Device for EoeDevice {
    type RxToken<'a> = RxFrame;
    type TxToken<'a> = TxSlot<'a>;

    fn receive(&mut self, _: SInstant) -> Option<(RxFrame, TxSlot<'_>)> {
        let f = self.rx.pop_front()?;
        Some((RxFrame(f), TxSlot(&mut self.tx)))
    }

    fn transmit(&mut self, _: SInstant) -> Option<TxSlot<'_>> {
        (self.tx.len() < TX_QUEUE).then_some(TxSlot(&mut self.tx))
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut c = DeviceCapabilities::default();
        c.medium = Medium::Ethernet;
        c.max_transmission_unit = self.mtu + 14;
        c
    }
}

struct Conn {
    stream: TcpStream,
    handle: SocketHandle,
    rec: GatewayConnection,
    client_eof: bool,
    shut_client: bool,
}

/// The local side: listeners, connections, the master's TCP/IP stack.
struct Host {
    iface: Interface,
    dev: EoeDevice,
    sockets: SocketSet<'static>,
    listeners: Vec<(TcpListener, PortForward)>,
    conns: Vec<Conn>,
    done: Vec<GatewayConnection>,
    next_port: u16,
    slave_ip: Ipv4Address,
    buffer: usize,
    ack_delay: Option<SDuration>,
}

fn nsf(ns: u64) -> f64 {
    ns as f64
}

impl Host {
    fn accept(&mut self, now: f64, log: &mut dyn FnMut(String)) {
        for (l, fwd) in &self.listeners {
            loop {
                let stream = match l.accept() {
                    Ok((s, _)) => s,
                    Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                    Err(e) => {
                        log(format!("accept on 127.0.0.1:{}: {e}", fwd.local_port));
                        break;
                    }
                };
                let _ = stream.set_nonblocking(true);
                let _ = stream.set_nodelay(true);
                let mut s = tcp::Socket::new(
                    tcp::SocketBuffer::new(vec![0; self.buffer]),
                    tcp::SocketBuffer::new(vec![0; self.buffer]),
                );
                s.set_ack_delay(self.ack_delay);
                s.set_nagle_enabled(false);
                let local = self.next_port;
                self.next_port = if self.next_port >= 65000 { 49152 } else { self.next_port + 1 };
                let mut rec = GatewayConnection {
                    local_port: fwd.local_port,
                    remote_port: fwd.remote_port,
                    opened_ns: now,
                    ..GatewayConnection::default()
                };
                if let Err(e) =
                    s.connect(self.iface.context(), (IpAddress::Ipv4(self.slave_ip), fwd.remote_port), local)
                {
                    rec.error = Some(format!("connect: {e}"));
                    rec.closed_ns = Some(now);
                    self.done.push(rec);
                    continue;
                }
                let handle = self.sockets.add(s);
                log(format!("connection: 127.0.0.1:{} → {}:{}", fwd.local_port, self.slave_ip, fwd.remote_port));
                self.conns.push(Conn { stream, handle, rec, client_eof: false, shut_client: false });
            }
        }
    }

    /// Move bytes between the local clients and the master's sockets; retire finished
    /// connections.
    fn shuttle(&mut self, now: f64, log: &mut dyn FnMut(String)) {
        let mut buf = [0u8; BUF];
        let mut i = 0;
        while i < self.conns.len() {
            let c = &mut self.conns[i];
            let s = self.sockets.get_mut::<tcp::Socket>(c.handle);
            if c.rec.established_ns.is_none() && s.state() == tcp::State::Established {
                c.rec.established_ns = Some(now);
            }
            let mut failed: Option<String> = None;
            // client → slave
            if !c.client_eof && s.may_send() {
                let room = s.send_capacity() - s.send_queue();
                if room > 0 {
                    match c.stream.read(&mut buf[..room.min(BUF)]) {
                        Ok(0) => {
                            c.client_eof = true;
                            s.close();
                        }
                        Ok(n) => {
                            let _ = s.send_slice(&buf[..n]);
                            c.rec.bytes_to_slave += n as u64;
                        }
                        Err(e) if e.kind() == ErrorKind::WouldBlock => {}
                        Err(e) => failed = Some(format!("client: {e}")),
                    }
                }
            }
            // slave → client
            while failed.is_none() && s.can_recv() {
                let stream = &mut c.stream;
                let r = s.recv(|b| match stream.write(b) {
                    Ok(n) => (n, Ok(n)),
                    Err(e) if e.kind() == ErrorKind::WouldBlock => (0, Ok(0)),
                    Err(e) => (0, Err(e)),
                });
                match r {
                    Ok(Ok(0)) => break,
                    Ok(Ok(n)) => {
                        c.rec.bytes_from_slave += n as u64;
                        c.rec.first_byte_from_slave_ns.get_or_insert(now);
                        c.rec.last_byte_from_slave_ns = Some(now);
                    }
                    Ok(Err(e)) => failed = Some(format!("client: {e}")),
                    Err(e) => failed = Some(format!("socket: {e}")),
                }
            }
            let st = s.state();
            if failed.is_none()
                && !c.shut_client
                && matches!(
                    st,
                    tcp::State::CloseWait
                        | tcp::State::LastAck
                        | tcp::State::Closing
                        | tcp::State::TimeWait
                        | tcp::State::Closed
                )
                && s.recv_queue() == 0
                && c.rec.established_ns.is_some()
            {
                let _ = c.stream.shutdown(Shutdown::Write);
                c.shut_client = true;
            }
            if c.client_eof && st == tcp::State::CloseWait {
                s.close();
            }
            let refused = st == tcp::State::Closed && c.rec.established_ns.is_none();
            if failed.is_some() || matches!(st, tcp::State::Closed | tcp::State::TimeWait) {
                if let Some(e) = &failed {
                    s.abort();
                    c.rec.error = Some(e.clone());
                } else if refused {
                    c.rec.error = Some("the slave refused the connection or did not answer".into());
                }
                let mut c = self.conns.swap_remove(i);
                c.rec.closed_ns = Some(now);
                if let (Some(a), Some(b)) = (c.rec.first_byte_from_slave_ns, c.rec.last_byte_from_slave_ns)
                    && b > a
                {
                    c.rec.goodput_from_slave_bps = Some(c.rec.bytes_from_slave as f64 * 8.0 / ((b - a) / 1e9));
                }
                let _ = c.stream.shutdown(Shutdown::Both);
                if st == tcp::State::TimeWait || st == tcp::State::Closed {
                    self.sockets.remove(c.handle);
                }
                log(format!(
                    "closed: 127.0.0.1:{} → :{}, {} B to the slave, {} B from it{}{}",
                    c.rec.local_port,
                    c.rec.remote_port,
                    c.rec.bytes_to_slave,
                    c.rec.bytes_from_slave,
                    c.rec.goodput_from_slave_bps.map_or(String::new(), |g| format!(", {:.1} kB/s", g / 8e3)),
                    c.rec.error.as_ref().map_or(String::new(), |e| format!(" ({e})"))
                ));
                self.done.push(c.rec);
                continue;
            }
            i += 1;
        }
    }
}

#[derive(Default)]
struct Link {
    /// Messages for the slave not yet encoded: kind and data after the mailbox header.
    queue: VecDeque<(MailboxType, Vec<u8>, Option<u64>)>,
    pending: Option<(Vec<u8>, Option<u64>)>,
    counter: u8,
    frame_no: u8,
    last_in_counter: Option<u8>,
    reasm: EoeReassembler,
}

/// Run the gateway over `bus` until `cfg.stop` turns true or `cfg.max_duration_ns` passes.
/// `log` gets one line per event (Set IP reply, connections).
pub async fn run_gateway<B: MailboxBus>(
    bus: &B,
    slave: &str,
    cfg: &GatewayConfig,
    log: &mut dyn FnMut(String),
) -> Result<GatewayResult, MasterError> {
    let m = cfg.mailbox;
    let min = MAILBOX_HEADER_LEN + EoeHeader::LEN + EoeHeader::BLOCK;
    if (m.out_len as usize) < min.max(MAILBOX_HEADER_LEN + 4 + 58) || (m.in_len as usize) < min {
        return Err(MasterError::Invalid(format!(
            "mailboxes of {} and {} bytes are too small for EoE",
            m.out_len, m.in_len
        )));
    }
    if !(68..=1500).contains(&cfg.mtu) {
        return Err(MasterError::Invalid(format!("MTU {} is outside 68–1500 bytes", cfg.mtu)));
    }
    if !(1..=30).contains(&cfg.prefix_len) {
        return Err(MasterError::Invalid(format!("prefix length {} is outside 1–30", cfg.prefix_len)));
    }
    let mut listeners = Vec::new();
    for f in &cfg.forwards {
        let l = TcpListener::bind(("127.0.0.1", f.local_port))
            .map_err(|e| MasterError::Invalid(format!("cannot listen on 127.0.0.1:{}: {e}", f.local_port)))?;
        l.set_nonblocking(true).map_err(|e| MasterError::Invalid(e.to_string()))?;
        listeners.push((l, *f));
    }

    let t0 = bus.now_ns();
    let ts = |ns: u64| SInstant::from_micros(((ns.saturating_sub(t0)) / 1000) as i64);
    let mut dev = EoeDevice { rx: VecDeque::new(), tx: VecDeque::new(), mtu: cfg.mtu as usize };
    let mut ic = Config::new(HardwareAddress::Ethernet(EthernetAddress(cfg.master_mac)));
    ic.random_seed = t0 ^ 0x5DEE_CE66;
    let mut iface = Interface::new(ic, &mut dev, ts(t0));
    let master_ip = Ipv4Address::from(cfg.master_ip);
    iface.update_ip_addrs(|a| {
        let _ = a.push(IpCidr::new(IpAddress::Ipv4(master_ip), cfg.prefix_len));
    });
    let mut host = Host {
        iface,
        dev,
        sockets: SocketSet::new(Vec::new()),
        listeners,
        conns: Vec::new(),
        done: Vec::new(),
        next_port: 49152 + (t0 % 1000) as u16,
        slave_ip: Ipv4Address::from(cfg.slave_ip),
        buffer: cfg.tcp_buffer_bytes.max(1024),
        ack_delay: cfg.ack_delay_ms.map(SDuration::from_millis),
    };

    let cap = msg::eoe_capacity(m.out_len as usize);
    let mut link = Link { counter: 1, frame_no: 1, ..Link::default() };
    if let Some(p) = &cfg.set_ip {
        link.queue.push_back((MailboxType::Eoe, msg::eoe_set_ip_request(p), None));
    }
    let mut r = GatewayResult {
        meta: ReportMeta::new(
            Provenance::Measured,
            InputRef { name: "EoE gateway".into(), bytes: 0, sha256: String::new() },
            Vec::new(),
        ),
        station: m.station,
        slave: slave.to_owned(),
        mailbox_out_bytes: m.out_len,
        mailbox_in_bytes: m.in_len,
        master_ip: master_ip.to_string(),
        slave_ip: host.slave_ip.to_string(),
        mtu_bytes: cfg.mtu,
        cycle_ns: cfg.cycle_ns as f64,
        ack_delay_ms: cfg.ack_delay_ms,
        tcp_buffer_bytes: host.buffer,
        set_ip_result: None,
        set_ip_reply_ns: None,
        duration_ns: 0.0,
        messages_to_slave: 0,
        messages_from_slave: 0,
        rejected_writes: 0,
        empty_reads: 0,
        lost_frames: 0,
        duplicate_messages: 0,
        other_messages: 0,
        frames_to_slave: 0,
        frames_from_slave: 0,
        frame_bytes_to_slave: 0,
        frame_bytes_from_slave: 0,
        broken_fragments: 0,
        write_access: Summary::default(),
        read_access: Summary::default(),
        connections: Vec::new(),
        timeline_bucket_ns: BUCKET_NS as f64,
        timeline: Vec::new(),
    };
    let (mut w_rtt, mut r_rtt) = (Vec::new(), Vec::new());
    let bucket = |tl: &mut Vec<GatewayBucket>, now: u64| -> usize {
        let k = ((now - t0) / BUCKET_NS) as usize;
        if tl.len() <= k {
            tl.resize(k + 1, GatewayBucket::default());
        }
        k
    };

    let mut pass = 0u64;
    let mut last_from_slave = t0;
    let mut silent_logged = false;
    loop {
        if cfg.cycle_ns > 0 {
            bus.sleep_until_ns(t0 + pass * cfg.cycle_ns).await;
            pass = (bus.now_ns() - t0) / cfg.cycle_ns + 1;
        }
        let now = bus.now_ns();
        if cfg.stop.load(Ordering::Relaxed) || now - t0 >= cfg.max_duration_ns {
            break;
        }
        let rel = nsf(now - t0);
        // The slave silent for 3 s while a connection is open: show what its ESC says.
        if !silent_logged && !host.conns.is_empty() && now - last_from_slave > 3_000_000_000 {
            silent_logged = true;
            log(esc_snapshot(bus, m.station, (now - last_from_slave) as f64 / 1e9).await);
        }
        host.accept(rel, log);
        host.shuttle(rel, log);
        host.iface.poll(ts(now), &mut host.dev, &mut host.sockets);

        // Next Ethernet frame into fragments once the previous one is out.
        if link.queue.is_empty()
            && let Some(f) = host.dev.tx.pop_front()
        {
            let frags = msg::eoe_fragments(&f, cap, link.frame_no);
            let last = frags.len() - 1;
            for (i, body) in frags.into_iter().enumerate() {
                link.queue.push_back((MailboxType::Eoe, body, (i == last).then_some(f.len() as u64)));
            }
            link.frame_no = msg::next_eoe_frame_no(link.frame_no);
        }
        if link.pending.is_none()
            && let Some((kind, body, frame_end)) = link.queue.pop_front()
        {
            link.pending = Some((msg::mailbox_buffer(kind, link.counter, 0, &body, m.out_len as usize), frame_end));
            link.counter = msg::next_counter(link.counter);
        }

        let wbuf = link.pending.as_ref().map(|p| p.0.clone());
        let wf = async {
            match wbuf {
                Some(b) => {
                    let t = bus.now_ns();
                    let x = write_mailbox(bus, &m, &b).await;
                    Some((x, bus.now_ns() - t))
                }
                None => None,
            }
        };
        let rf = async {
            let t = bus.now_ns();
            let x = read_mailbox(bus, &m).await;
            (x, bus.now_ns() - t)
        };
        let (w, (rd, rd_ns)) = futures_lite::future::zip(wf, rf).await;
        let after = bus.now_ns();
        let k = bucket(&mut r.timeline, after);

        if let Some((w, w_ns)) = w {
            if w_rtt.len() < MAX_SAMPLES {
                w_rtt.push(w_ns as f64);
            }
            match w {
                Ok(wkc) if wkc > 0 => {
                    let (_, frame_end) = link.pending.take().unwrap_or_default();
                    r.messages_to_slave += 1;
                    r.timeline[k].messages_to_slave += 1;
                    if let Some(n) = frame_end {
                        r.frames_to_slave += 1;
                        r.frame_bytes_to_slave += n;
                        r.timeline[k].frame_bytes_to_slave += n;
                    }
                }
                Ok(_) => r.rejected_writes += 1,
                Err(BusError::Lost) => r.lost_frames += 1,
                Err(BusError::Fatal(e)) => return Err(MasterError::Bus(e)),
            }
        }
        if r_rtt.len() < MAX_SAMPLES {
            r_rtt.push(rd_ns as f64);
        }
        match rd {
            Ok(x) if x.wkc > 0 => {
                let Ok(h) = MailboxHeader::parse(&x.data) else {
                    r.other_messages += 1;
                    continue;
                };
                if h.counter != 0 && link.last_in_counter == Some(h.counter) {
                    r.duplicate_messages += 1;
                    continue;
                }
                link.last_in_counter = Some(h.counter);
                last_from_slave = after;
                silent_logged = false;
                r.messages_from_slave += 1;
                r.timeline[k].messages_from_slave += 1;
                let end = (MAILBOX_HEADER_LEN + h.length as usize).min(x.data.len());
                let body = &x.data[MAILBOX_HEADER_LEN..end];
                let eoe = (h.mailbox_type == MailboxType::Eoe).then(|| EoeHeader::parse(body).ok()).flatten();
                match eoe.map(|e| e.frame_type) {
                    Some(EoeType::Fragment) => {
                        if let Some(f) = link.reasm.push(body) {
                            r.frames_from_slave += 1;
                            r.frame_bytes_from_slave += f.len() as u64;
                            r.timeline[k].frame_bytes_from_slave += f.len() as u64;
                            host.dev.rx.push_back(f);
                        }
                    }
                    Some(EoeType::SetIpResponse) => {
                        let code = msg::eoe_set_ip_result(body).unwrap_or(u16::MAX);
                        r.set_ip_result = Some(code);
                        r.set_ip_reply_ns = Some(nsf(after - t0));
                        log(format!(
                            "Set IP Parameter response: {} after {:.1} ms",
                            if code == 0 { "success".to_owned() } else { format!("error {code:#06x}") },
                            (after - t0) as f64 / 1e6
                        ));
                    }
                    _ => r.other_messages += 1,
                }
            }
            Ok(_) => r.empty_reads += 1,
            Err(BusError::Lost) => r.lost_frames += 1,
            Err(BusError::Fatal(e)) => return Err(MasterError::Bus(e)),
        }
    }

    let end = bus.now_ns();
    let rel = nsf(end - t0);
    for mut c in host.conns.drain(..) {
        c.rec.closed_ns = Some(rel);
        c.rec.error.get_or_insert_with(|| "still open when the gateway stopped".into());
        if let (Some(a), Some(b)) = (c.rec.first_byte_from_slave_ns, c.rec.last_byte_from_slave_ns)
            && b > a
        {
            c.rec.goodput_from_slave_bps = Some(c.rec.bytes_from_slave as f64 * 8.0 / ((b - a) / 1e9));
        }
        host.done.push(c.rec);
    }
    host.done.sort_by(|a, b| a.opened_ns.total_cmp(&b.opened_ns));
    r.connections = host.done;
    r.duration_ns = rel;
    r.broken_fragments = link.reasm.broken;
    r.write_access = Summary::of(&w_rtt);
    r.read_access = Summary::of(&r_rtt);
    let mut a = vec![
        format!(
            "Every pass writes the next EoE fragment for the slave (if any) and reads \"mailbox in\", the two in flight together; {}.",
            if cfg.cycle_ns > 0 { format!("one pass per {} µs cycle", cfg.cycle_ns / 1000) } else { "no pause between passes".into() }
        ),
        "A rejected write (WKC 0) or a lost one is repeated with the same counter; a lost read is not recovered (the slave's TCP resends what was in it).".into(),
        format!(
            "The master's TCP/IP is smoltcp on the EoE link ({} on a /{} subnet, IP MTU {} B), not the host's; acknowledgements {}; TCP buffer {} B per connection.",
            master_ip,
            cfg.prefix_len,
            cfg.mtu,
            match cfg.ack_delay_ms {
                Some(d) => format!("delayed up to {d} ms or until two segments are unacknowledged"),
                None => "immediate".into(),
            },
            host.buffer
        ),
        "Local TCP ports on 127.0.0.1 are forwarded to the slave; each connection's goodput counts its bytes from the slave between its first and last byte.".into(),
    ];
    a.extend(cfg.assumptions.iter().cloned());
    r.meta = ReportMeta::new(
        Provenance::Measured,
        InputRef { name: "EoE gateway".into(), bytes: r.frame_bytes_from_slave, sha256: String::new() },
        a,
    );
    Ok(r)
}

/// SyncManager 0 and 1 registers, AL status and the AL event request, as one log line.
async fn esc_snapshot<B: MailboxBus>(bus: &B, station: u16, silent_s: f64) -> String {
    let hex = |r: Result<crate::bus::Reply, BusError>| match r {
        Ok(x) if x.wkc > 0 => x.data.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" "),
        Ok(_) => "WKC 0".into(),
        Err(e) => format!("{e:?}"),
    };
    let sm0 = hex(bus.fprd(station, 0x0800, 8).await);
    let sm1 = hex(bus.fprd(station, 0x0808, 8).await);
    let al = hex(bus.fprd(station, 0x0130, 6).await);
    let ev = hex(bus.fprd(station, 0x0220, 4).await);
    format!(
        "the slave sent nothing for {silent_s:.1} s with a connection open: SM0 [{sm0}] SM1 [{sm1}] AL status/code [{al}] AL event request [{ev}]"
    )
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::time::Instant;

    use ecat_wire::EoeHeader;
    use flowlab_mailbox::msg::EoeIpParameters;

    use super::*;
    use crate::bus::Reply;

    const STATION: u16 = 0x1001;
    const SLAVE_IP: [u8; 4] = [192, 168, 2, 13];
    const ECHO_PORT: u16 = 7;

    /// A slave with ESC mailbox semantics whose EoE frames end in its own TCP/IP stack with a
    /// TCP echo server; it answers the Set IP Parameter request and keeps what it set.
    struct EchoSlave {
        t0: Instant,
        layout: MailboxLayout,
        s: RefCell<State>,
    }

    struct State {
        sm1: Option<Vec<u8>>,
        outbox: VecDeque<Vec<u8>>,
        counter: u8,
        frame_no: u8,
        reasm: EoeReassembler,
        iface: Interface,
        dev: EoeDevice,
        sockets: SocketSet<'static>,
        server: SocketHandle,
        set_ip: Option<Vec<u8>>,
    }

    impl EchoSlave {
        fn new(len: u16) -> Self {
            let mut dev = EoeDevice { rx: VecDeque::new(), tx: VecDeque::new(), mtu: 1500 };
            let mut iface = Interface::new(
                Config::new(HardwareAddress::Ethernet(EthernetAddress([0x02, 1, 5, 0x20, 3, 0xeb]))),
                &mut dev,
                SInstant::ZERO,
            );
            iface.update_ip_addrs(|a| {
                let _ = a.push(IpCidr::new(IpAddress::Ipv4(Ipv4Address::from(SLAVE_IP)), 24));
            });
            let mut sockets = SocketSet::new(Vec::new());
            let mut s =
                tcp::Socket::new(tcp::SocketBuffer::new(vec![0; 65536]), tcp::SocketBuffer::new(vec![0; 65536]));
            s.listen(ECHO_PORT).unwrap();
            let server = sockets.add(s);
            Self {
                t0: Instant::now(),
                layout: MailboxLayout {
                    station: STATION,
                    out_start: 0x1000,
                    out_len: len,
                    in_start: 0x1000 + len,
                    in_len: len,
                },
                s: RefCell::new(State {
                    sm1: None,
                    outbox: VecDeque::new(),
                    counter: 1,
                    frame_no: 1,
                    reasm: EoeReassembler::default(),
                    iface,
                    dev,
                    sockets,
                    server,
                    set_ip: None,
                }),
            }
        }

        /// Run the slave's stack: echo, frames into fragments, the next message into SM1.
        fn step(&self, st: &mut State) {
            let now = SInstant::from_micros(self.t0.elapsed().as_micros() as i64);
            st.iface.poll(now, &mut st.dev, &mut st.sockets);
            let s = st.sockets.get_mut::<tcp::Socket>(st.server);
            let mut buf = [0u8; 4096];
            while s.can_recv() && s.send_capacity() - s.send_queue() >= buf.len() {
                let n = s.recv_slice(&mut buf).unwrap();
                s.send_slice(&buf[..n]).unwrap();
            }
            if s.state() == tcp::State::CloseWait && s.send_queue() == 0 {
                s.close();
            }
            if !s.is_open() {
                s.listen(ECHO_PORT).unwrap();
            }
            st.iface.poll(now, &mut st.dev, &mut st.sockets);
            while let Some(f) = st.dev.tx.pop_front() {
                let cap = msg::eoe_capacity(self.layout.in_len as usize);
                st.outbox.extend(msg::eoe_fragments(&f, cap, st.frame_no));
                st.frame_no = msg::next_eoe_frame_no(st.frame_no);
            }
            if st.sm1.is_none()
                && let Some(body) = st.outbox.pop_front()
            {
                st.sm1 = Some(msg::mailbox_buffer(MailboxType::Eoe, st.counter, 0, &body, self.layout.in_len as usize));
                st.counter = msg::next_counter(st.counter);
            }
        }
    }

    impl MailboxBus for EchoSlave {
        fn now_ns(&self) -> u64 {
            self.t0.elapsed().as_nanos() as u64
        }

        async fn sleep_until_ns(&self, _: u64) {}

        async fn cyclic(&self) -> Result<Option<u16>, BusError> {
            Ok(None)
        }

        async fn fprd(&self, station: u16, register: u16, len: u16) -> Result<Reply, BusError> {
            let mut st = self.s.borrow_mut();
            if station != STATION || register != self.layout.in_start {
                return Ok(Reply { data: vec![0; len as usize], wkc: 0 });
            }
            self.step(&mut st);
            Ok(match st.sm1.take() {
                Some(data) => Reply { data, wkc: 1 },
                None => Reply { data: vec![0; len as usize], wkc: 0 },
            })
        }

        async fn fpwr(&self, station: u16, register: u16, data: &[u8]) -> Result<u16, BusError> {
            let mut st = self.s.borrow_mut();
            if station != STATION || register != self.layout.out_start {
                return Ok(0);
            }
            let h = MailboxHeader::parse(data).unwrap();
            let body = &data[MAILBOX_HEADER_LEN..MAILBOX_HEADER_LEN + h.length as usize];
            match EoeHeader::parse(body).unwrap().frame_type {
                EoeType::SetIpRequest => {
                    st.set_ip = Some(body.to_vec());
                    let mut r = EoeHeader::parse(body).unwrap();
                    r.frame_type = EoeType::SetIpResponse;
                    st.outbox.push_back(r.encode().to_vec());
                }
                EoeType::Fragment => {
                    if let Some(f) = st.reasm.push(body) {
                        st.dev.rx.push_back(f);
                    }
                }
                _ => {}
            }
            self.step(&mut st);
            Ok(1)
        }
    }

    fn config(slave: &EchoSlave, local_port: u16, stop: Arc<AtomicBool>) -> GatewayConfig {
        GatewayConfig {
            mailbox: slave.layout,
            set_ip: Some(EoeIpParameters {
                mac: Some([0x02, 1, 5, 0x20, 3, 0xeb]),
                ip: Some(SLAVE_IP),
                subnet_mask: Some([255, 255, 255, 0]),
                gateway: None,
            }),
            slave_ip: SLAVE_IP,
            master_mac: [0x02, 0x46, 0x4c, 0, 0, 1],
            master_ip: [192, 168, 2, 10],
            prefix_len: 24,
            mtu: 1500,
            forwards: vec![PortForward { local_port, remote_port: ECHO_PORT }],
            tcp_buffer_bytes: 65536,
            ack_delay_ms: Some(10),
            cycle_ns: 0,
            max_duration_ns: 30_000_000_000,
            stop,
            assumptions: Vec::new(),
        }
    }

    fn free_port() -> u16 {
        TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port()
    }

    #[test]
    fn a_forwarded_connection_reaches_the_slave_through_eoe_fragments() {
        let slave = EchoSlave::new(128);
        let port = free_port();
        let stop = Arc::new(AtomicBool::new(false));
        let cfg = config(&slave, port, stop.clone());
        let data: Vec<u8> = (0..50_000u32).map(|i| (i * 7 + i / 251) as u8).collect();
        let sent = data.clone();
        let client = std::thread::spawn(move || {
            let mut c = None;
            for _ in 0..200 {
                if let Ok(s) = TcpStream::connect(("127.0.0.1", port)) {
                    c = Some(s);
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            let mut c = c.expect("the gateway listens");
            c.set_read_timeout(Some(std::time::Duration::from_secs(20))).unwrap();
            let mut writer = c.try_clone().unwrap();
            let w = std::thread::spawn(move || writer.write_all(&sent));
            let mut back = vec![0; data.len()];
            let got = c.read_exact(&mut back);
            w.join().unwrap().unwrap();
            drop(c);
            std::thread::sleep(std::time::Duration::from_millis(200));
            stop.store(true, Ordering::Relaxed);
            got.map(|()| back == data)
        });
        let mut lines = Vec::new();
        let r = futures_lite::future::block_on(run_gateway(&slave, "echo", &cfg, &mut |l| lines.push(l))).unwrap();
        assert_eq!(client.join().unwrap().ok(), Some(true), "the echo comes back unchanged; log: {lines:?}");

        // Set IP: answered, and the request carried the slave's address, least significant first.
        assert_eq!(r.set_ip_result, Some(0));
        let req = slave.s.borrow().set_ip.clone().unwrap();
        assert_eq!(&req[14..18], &[13, 2, 168, 192]);
        // Both directions in fragments of at most a mailbox each, nothing lost or broken.
        assert!(r.frames_to_slave > 30 && r.frames_from_slave > 30, "{} / {}", r.frames_to_slave, r.frames_from_slave);
        assert!(r.messages_to_slave > r.frames_to_slave, "full-size frames take several fragments");
        assert!(r.frame_bytes_from_slave > 50_000);
        assert_eq!((r.broken_fragments, r.lost_frames, r.duplicate_messages), (0, 0, 0));
        assert_eq!(r.connections.len(), 1);
        let c = &r.connections[0];
        assert_eq!((c.bytes_to_slave, c.bytes_from_slave), (50_000, 50_000));
        assert!(c.established_ns.is_some() && c.error.is_none(), "{c:?}");
        assert_eq!(r.meta.source, Provenance::Measured);
    }

    #[test]
    fn a_closed_port_at_the_slave_ends_the_connection_with_an_error() {
        let slave = EchoSlave::new(128);
        let port = free_port();
        let stop = Arc::new(AtomicBool::new(false));
        let mut cfg = config(&slave, port, stop.clone());
        cfg.forwards[0].remote_port = ECHO_PORT + 1;
        let client = std::thread::spawn(move || {
            let mut c = None;
            for _ in 0..200 {
                if let Ok(s) = TcpStream::connect(("127.0.0.1", port)) {
                    c = Some(s);
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            let mut c = c.expect("the gateway listens");
            c.set_read_timeout(Some(std::time::Duration::from_secs(10))).unwrap();
            let mut b = [0u8; 8];
            let n = c.read(&mut b);
            stop.store(true, Ordering::Relaxed);
            n.ok()
        });
        let r = futures_lite::future::block_on(run_gateway(&slave, "echo", &cfg, &mut |_| {})).unwrap();
        assert_eq!(client.join().unwrap(), Some(0), "the client sees the connection end");
        assert!(r.connections[0].error.is_some(), "{:?}", r.connections[0]);
    }

    #[test]
    fn mailboxes_too_small_for_set_ip_are_refused() {
        let slave = EchoSlave::new(48);
        let cfg = config(&slave, free_port(), Arc::new(AtomicBool::new(false)));
        assert!(matches!(
            futures_lite::future::block_on(run_gateway(&slave, "echo", &cfg, &mut |_| {})),
            Err(MasterError::Invalid(_))
        ));
    }
}
