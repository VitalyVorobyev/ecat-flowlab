//! EtherCrab on a network interface: scan, a cyclic run, and mailbox transfers.
//!
//! **This sends EtherCAT frames.** Use it only on an isolated lab segment with no other
//! master. [`Session::open`] takes the [`QuietSegment`] of [`crate::safety::check_segment`],
//! so a session exists only after those checks passed. Nothing here writes a slave's EEPROM.
//!
//! EtherCrab (MIT OR Apache-2.0) initialises the network: it assigns station addresses,
//! configures the mailbox SyncManagers from each slave's EEPROM and, for `run` and `transfer
//! --op`, maps process data and moves the slaves to OP. Mailbox transfers do not use
//! EtherCrab's mailbox code: every access is an FPWR or FPRD of a SyncManager buffer issued
//! through [`ethercrab::Command`], so the engine controls counters, retries and the service
//! policy, as the simulator does.
//!
//! The TX/RX side runs on its own thread: EtherCrab's raw socket on Linux (`CAP_NET_RAW` or
//! root), Npcap on Windows; on macOS, a BPF socket of `pnet_datalink` (read/write access to
//! `/dev/bpf*`) that hands EtherCrab only frames that passed an ESC. One [`Session`] per process (EtherCrab's frame
//! storage is a static); its thread ends with the process.
//!
//! [`Session::in_process`] connects EtherCrab to a function instead of an interface, e.g. the
//! emulated ESC of `flowlab-slave`: nothing is sent on any network, and the whole layer runs
//! in a test.
//!
//! A slave whose EEPROM names it FlowLab's emulated ESC
//! ([`EMULATED_ESC_NAME`](flowlab_core::EMULATED_ESC_NAME)) is software on an ordinary
//! Ethernet port; every result that involves it says so in its assumptions.
//!
//! Timing is the host's: `std::time::Instant` around each call. It includes the operating
//! system's scheduling and the socket, and is not the wire's; a capture taken meanwhile shows
//! the wire (with hardware timestamps, if the capture tool provides them).

use std::time::{Duration, Instant};

use ecat_wire::{AlStatus, SmDirection, SmMode, SyncManagerConfig, sm_register};
use ethercrab::error::Error as EcError;
use ethercrab::subdevice_group::Op;
use ethercrab::{
    Command, DcSupport, DefaultLock, MainDevice, MainDeviceConfig, PduStorage, RetryBehaviour, SubDeviceGroup, Timeouts,
};
use flowlab_core::{EMULATED_ESC_ASSUMPTION, EMULATED_ESC_NAME, InputRef, Provenance, ReportMeta};

use crate::MasterError;
use crate::bus::{BusError, MailboxBus, MailboxLayout, Reply};
use crate::engine::{TransferJob, TransferOptions, run_transfer};
use crate::result::{
    CycleRecord, CycleTiming, MeasuredTransfer, RunResult, ScanResult, ScannedSlave, ScannedSm, Summary,
};
use crate::safety::QuietSegment;

/// Most slaves a session handles.
pub const MAX_SUBDEVICES: usize = 16;
/// Largest process image (outputs + inputs), bytes.
pub const MAX_PDI: usize = 1024;
const MAX_FRAMES: usize = 16;
const MAX_PDU_DATA: usize = PduStorage::element_size(1500);

static PDU_STORAGE: PduStorage<MAX_FRAMES, MAX_PDU_DATA> = PduStorage::new();

type OpGroup = SubDeviceGroup<MAX_SUBDEVICES, MAX_PDI, DefaultLock, Op>;

/// Settings of a [`Session`].
#[derive(Debug, Clone)]
pub struct SessionOptions {
    /// How long to wait for a frame to come back before it counts as lost.
    pub pdu_timeout: Duration,
    /// How long to wait for a state change (e.g. SAFE-OP → OP).
    pub state_transition_timeout: Duration,
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self { pdu_timeout: Duration::from_millis(30), state_transition_timeout: Duration::from_secs(5) }
    }
}

/// An EtherCrab master on one interface.
pub struct Session {
    md: MainDevice<'static>,
    interface: String,
    origin: Instant,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session").field("interface", &self.interface).finish_non_exhaustive()
    }
}

fn ec(e: EcError) -> MasterError {
    MasterError::Bus(e.to_string())
}

/// Assumptions every hardware result carries.
fn host_assumptions() -> Vec<String> {
    vec![
        "Timing is the host's: std::time::Instant around each EtherCrab call, including the operating system's scheduling and the socket (raw socket on Linux, Npcap on Windows), not the wire's. A capture at the port shows the wire.".into(),
        "EtherCrab sends every access as its own Ethernet frame; a write and a read the policy puts in one frame are in flight together, in two frames.".into(),
        "EtherCrab initialised the network: station addresses from 0x1000, mailbox SyncManagers from the EEPROM, process data mapped over CoE where the slave supports it.".into(),
    ]
}

/// Wait until `deadline`: sleep while it is far, then spin.
fn wait_until(deadline: Instant) {
    loop {
        let now = Instant::now();
        if now >= deadline {
            return;
        }
        let left = deadline - now;
        if left > Duration::from_micros(1500) {
            std::thread::sleep(left - Duration::from_micros(1000));
        } else {
            std::thread::yield_now();
        }
    }
}

fn map_read<T>(r: Result<T, EcError>, len: u16, data: impl FnOnce(T) -> Vec<u8>) -> Result<Reply, BusError> {
    match r {
        Ok(d) => Ok(Reply { data: data(d), wkc: 1 }),
        Err(EcError::WorkingCounter { received, .. }) => Ok(Reply { data: vec![0; len as usize], wkc: received }),
        Err(EcError::Timeout(_)) => Err(BusError::Lost),
        Err(e) => Err(BusError::Fatal(e.to_string())),
    }
}

/// The engine's bus on EtherCrab, with the process data of an OP group as the cyclic exchange.
struct EthercrabBus<'a> {
    md: &'a MainDevice<'a>,
    group: Option<&'a OpGroup>,
    origin: Instant,
}

impl MailboxBus for EthercrabBus<'_> {
    fn now_ns(&self) -> u64 {
        self.origin.elapsed().as_nanos() as u64
    }

    async fn sleep_until_ns(&self, t_ns: u64) {
        wait_until(self.origin + Duration::from_nanos(t_ns));
    }

    async fn cyclic(&self) -> Result<Option<u16>, BusError> {
        let Some(g) = self.group else { return Ok(None) };
        match g.tx_rx(self.md).await {
            Ok(r) => Ok(Some(r.working_counter)),
            Err(EcError::Timeout(_)) => Err(BusError::Lost),
            Err(e) => Err(BusError::Fatal(e.to_string())),
        }
    }

    async fn fprd(&self, station: u16, register: u16, len: u16) -> Result<Reply, BusError> {
        let r = Command::fprd(station, register).receive_slice(self.md, len).await;
        map_read(r, len, |d| d.to_vec())
    }

    async fn fpwr(&self, station: u16, register: u16, data: &[u8]) -> Result<u16, BusError> {
        match Command::fpwr(station, register).send_receive_slice(self.md, data).await {
            Ok(_) => Ok(1),
            Err(EcError::WorkingCounter { received, .. }) => Ok(received),
            Err(EcError::Timeout(_)) => Err(BusError::Lost),
            Err(e) => Err(BusError::Fatal(e.to_string())),
        }
    }
}

/// Whether a slave of the group is FlowLab's emulated ESC.
fn is_emulated(group: &SubDeviceGroup<MAX_SUBDEVICES, MAX_PDI>, md: &MainDevice<'_>) -> bool {
    group.iter(md).any(|s| s.name() == EMULATED_ESC_NAME)
}

/// Mailbox protocol names of the EEPROM's mailbox protocol word (`0x001C`).
fn protocols(word: u16) -> Vec<String> {
    use ecat_wire::MailboxType as T;
    [(0x01, T::Aoe), (0x02, T::Eoe), (0x04, T::Coe), (0x08, T::Foe), (0x10, T::Soe), (0x20, T::Voe)]
        .iter()
        .filter(|(bit, _)| word & bit != 0)
        .map(|(_, t)| t.name().to_owned())
        .collect()
}

/// EtherCrab's own TX/RX task on its own thread (Linux: raw socket; Windows: Npcap).
#[cfg(not(target_os = "macos"))]
fn start_ethercrab(
    interface: &str,
    tx: ethercrab::PduTx<'static>,
    rx: ethercrab::PduRx<'static>,
) -> Result<(), MasterError> {
    let (started, ready) = std::sync::mpsc::channel::<Result<(), String>>();
    let iface = interface.to_owned();
    std::thread::Builder::new()
        .name("ethercrab-tx-rx".into())
        .spawn(move || {
            #[cfg(unix)]
            match ethercrab::std::tx_rx_task(&iface, tx, rx) {
                Ok(task) => {
                    let _ = started.send(Ok(()));
                    if let Err(e) = async_io::block_on(task) {
                        tracing::error!("EtherCrab TX/RX on {iface} stopped: {e}");
                    }
                }
                Err(e) => {
                    let _ = started.send(Err(format!("cannot open {iface}: {e}")));
                }
            }
            #[cfg(windows)]
            {
                let _ = started.send(Ok(()));
                let cfg = ethercrab::std::TxRxTaskConfig { spinloop: false };
                if let Err(e) = ethercrab::std::tx_rx_task_blocking(&iface, tx, rx, cfg) {
                    tracing::error!("EtherCrab TX/RX on {iface} stopped: {e}");
                }
            }
        })
        .map_err(|e| MasterError::Bus(format!("cannot start the TX/RX thread: {e}")))?;
    ready
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| MasterError::Bus("the TX/RX thread did not start".into()))?
        .map_err(MasterError::Bus)
}

/// macOS: EtherCrab's frames through a BPF socket of `pnet_datalink`.
///
/// EtherCrab's own macOS socket tells its frames from the slaves' by the interface's MAC
/// address, which it reads through `getifaddrs`; for some adapters that read returns a wrong
/// address, so EtherCrab takes its own frame, which BPF also delivers, for the answer. Here a
/// frame goes to EtherCrab only if bit 1 of its source MAC is set: an ESC sets it, so the frame
/// has passed a slave.
#[cfg(target_os = "macos")]
mod bpf {
    use std::time::Duration;

    use ethercrab::{PduRx, PduTx};
    use pnet_datalink::{Channel, Config};

    use crate::MasterError;

    pub(super) fn start(interface: &str, mut tx: PduTx<'static>, mut rx: PduRx<'static>) -> Result<(), MasterError> {
        let iface = pnet_datalink::interfaces()
            .into_iter()
            .find(|i| i.name == interface)
            .ok_or_else(|| MasterError::Bus(format!("no interface `{interface}`")))?;
        let cfg = Config { read_timeout: Some(Duration::from_millis(100)), ..Config::default() };
        let (mut out, mut inp) = match pnet_datalink::channel(&iface, cfg) {
            Ok(Channel::Ethernet(t, r)) => (t, r),
            Ok(_) => return Err(MasterError::Bus(format!("{interface}: not an Ethernet channel"))),
            Err(e) => return Err(MasterError::Bus(format!("cannot open {interface}: {e}"))),
        };
        let name = interface.to_owned();
        std::thread::Builder::new()
            .name("ethercrab-tx".into())
            .spawn(move || {
                futures_lite::future::block_on(futures_lite::future::poll_fn(|cx| {
                    tx.replace_waker(cx.waker());
                    while let Some(frame) = tx.next_sendable_frame() {
                        let sent = frame.send_blocking(|data| match out.send_to(data, None) {
                            Some(Ok(())) => Ok(data.len()),
                            _ => Err(ethercrab::error::Error::SendFrame),
                        });
                        if let Err(e) = sent {
                            tracing::error!("send on {name}: {e}");
                        }
                    }
                    if tx.should_exit() { std::task::Poll::Ready(()) } else { std::task::Poll::Pending }
                }));
            })
            .map_err(|e| MasterError::Bus(format!("cannot start the TX thread: {e}")))?;
        let name = interface.to_owned();
        std::thread::Builder::new()
            .name("ethercrab-rx".into())
            .spawn(move || {
                loop {
                    match inp.next() {
                        Ok(f) => {
                            if f.len() > 14 && f[12..14] == [0x88, 0xA4] && f[6] & 0x02 != 0 {
                                let _ = rx.receive_frame(f);
                            }
                        }
                        Err(e) if matches!(e.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock) => {
                        }
                        Err(e) => {
                            tracing::error!("receive on {name}: {e}");
                            return;
                        }
                    }
                }
            })
            .map_err(|e| MasterError::Bus(format!("cannot start the RX thread: {e}")))?;
        Ok(())
    }
}

impl Session {
    /// Open the interface `segment` checked and start EtherCrab's TX/RX thread. Fails without
    /// the permission to open a raw socket (Linux: `CAP_NET_RAW`), or if a session was opened
    /// before in this process.
    pub fn open(segment: &QuietSegment, o: &SessionOptions) -> Result<Self, MasterError> {
        let interface = segment.interface();
        let (tx, rx, pdu_loop) =
            PDU_STORAGE.try_split().map_err(|_| MasterError::Invalid("one EtherCrab session per process".into()))?;
        #[cfg(target_os = "macos")]
        bpf::start(interface, tx, rx)?;
        #[cfg(not(target_os = "macos"))]
        start_ethercrab(interface, tx, rx)?;
        Ok(Self::with_loop(pdu_loop, interface, o))
    }

    fn with_loop(pdu_loop: ethercrab::PduLoop<'static>, interface: &str, o: &SessionOptions) -> Self {
        let timeouts =
            Timeouts { pdu: o.pdu_timeout, state_transition: o.state_transition_timeout, ..Timeouts::default() };
        let config = MainDeviceConfig { dc_static_sync_iterations: 0, retry_behaviour: RetryBehaviour::None };
        let md = MainDevice::new(pdu_loop, timeouts, config);
        Self { md, interface: interface.to_owned(), origin: Instant::now() }
    }

    /// A session whose frames go to `answer` in this process instead of a network interface:
    /// `answer` takes each Ethernet frame EtherCAT sends and returns the frame that comes back
    /// (`None`: lost). Nothing is sent on any network, so no safety check applies. Results name
    /// the interface `in-process`. Fails if a session was opened before in this process.
    pub fn in_process(
        mut answer: impl FnMut(&[u8]) -> Option<Vec<u8>> + Send + 'static,
        o: &SessionOptions,
    ) -> Result<Self, MasterError> {
        let (mut tx, mut rx, pdu_loop) =
            PDU_STORAGE.try_split().map_err(|_| MasterError::Invalid("one EtherCrab session per process".into()))?;
        std::thread::Builder::new()
            .name("ethercrab-in-process".into())
            .spawn(move || {
                futures_lite::future::block_on(futures_lite::future::poll_fn(|cx| {
                    tx.replace_waker(cx.waker());
                    while let Some(frame) = tx.next_sendable_frame() {
                        let mut back = None;
                        let sent = frame.send_blocking(|data| {
                            back = answer(data);
                            Ok(data.len())
                        });
                        if let Err(e) = sent {
                            tracing::error!("in-process TX: {e}");
                        }
                        if let Some(b) = back
                            && let Err(e) = rx.receive_frame(&b)
                        {
                            tracing::error!("in-process RX: {e}");
                        }
                    }
                    if tx.should_exit() { std::task::Poll::Ready(()) } else { std::task::Poll::Pending }
                }));
            })
            .map_err(|e| MasterError::Bus(format!("cannot start the in-process thread: {e}")))?;
        Ok(Self::with_loop(pdu_loop, "in-process", o))
    }

    fn meta(&self, emulated: bool, extra: Vec<String>) -> ReportMeta {
        let input = InputRef { name: self.interface.clone(), bytes: 0, sha256: String::new() };
        let mut a = host_assumptions();
        if emulated {
            a.push(EMULATED_ESC_ASSUMPTION.into());
        }
        a.extend(extra);
        ReportMeta::new(Provenance::Measured, input, a)
    }

    async fn init(&self) -> Result<SubDeviceGroup<MAX_SUBDEVICES, MAX_PDI>, MasterError> {
        let g = self.md.init_single_group::<MAX_SUBDEVICES, MAX_PDI>(ethercrab::std::ethercat_now).await.map_err(ec)?;
        if g.is_empty() {
            return Err(MasterError::Bus(format!("no slaves answered on {}", self.interface)));
        }
        Ok(g)
    }

    async fn sync_managers(&self, station: u16) -> Result<Vec<(u8, SyncManagerConfig)>, MasterError> {
        let mut out = Vec::new();
        for n in 0..8u8 {
            let r = Command::fprd(station, sm_register(n)).receive_slice(&self.md, 8).await;
            let Ok(d) = r else { continue };
            if let Ok(sm) = SyncManagerConfig::parse(&d)
                && sm.enabled()
                && sm.length > 0
            {
                out.push((n, sm));
            }
        }
        Ok(out)
    }

    async fn al_status(&self, station: u16) -> (String, u16) {
        let s = Command::fprd(station, 0x0130).receive_slice(&self.md, 2).await;
        let c = Command::fprd(station, 0x0134).receive_slice(&self.md, 2).await;
        let state = match s {
            Ok(d) if !d.is_empty() => {
                let st = AlStatus::from_byte(d[0]);
                format!("{}{}", st.state.name(), if st.error { " (error)" } else { "" })
            }
            _ => "unknown".into(),
        };
        let code = c.ok().filter(|d| d.len() >= 2).map_or(0, |d| u16::from_le_bytes([d[0], d[1]]));
        (state, code)
    }

    /// Initialise the network and read what is on it. The slaves stay in PRE-OP.
    pub fn scan(&self) -> Result<ScanResult, MasterError> {
        async_io::block_on(async {
            let group = self.init().await?;
            let mut slaves = Vec::new();
            for (position, sd) in group.iter(&self.md).enumerate() {
                let station = sd.configured_address();
                let id = sd.identity();
                let (al_state, al_status_code) = self.al_status(station).await;
                let word = sd.eeprom_read::<u16>(&self.md, 0x001C).await.unwrap_or(0);
                let sync_managers = self
                    .sync_managers(station)
                    .await?
                    .into_iter()
                    .map(|(index, sm)| ScannedSm {
                        index,
                        start: sm.start,
                        length: sm.length,
                        control: sm.control,
                        mode: match sm.mode() {
                            SmMode::Mailbox => "mailbox",
                            _ => "buffered",
                        }
                        .into(),
                        direction: match sm.direction() {
                            SmDirection::EcatWrite => "master_to_slave",
                            _ => "slave_to_master",
                        }
                        .into(),
                    })
                    .collect();
                slaves.push(ScannedSlave {
                    position: position as u16,
                    station,
                    alias: sd.alias_address(),
                    name: sd.name().to_owned(),
                    vendor_id: id.vendor_id,
                    product_code: id.product_id,
                    revision: id.revision,
                    serial: id.serial,
                    al_state,
                    al_status_code,
                    mailbox_protocols: protocols(word),
                    sync_managers,
                    dc_support: match sd.dc_support() {
                        DcSupport::None => "none",
                        DcSupport::RefOnly => "ref_only",
                        DcSupport::Bits32 => "32 bit",
                        DcSupport::Bits64 => "64 bit",
                    }
                    .into(),
                    propagation_delay_ns: sd.propagation_delay(),
                });
            }
            let emulated = slaves.iter().any(|s| s.name == EMULATED_ESC_NAME);
            Ok(ScanResult {
                meta: self.meta(emulated, vec![
                    "Identity, name and mailbox protocols are read from each slave's EEPROM; SyncManagers from registers 0x0800 + 8·n after EtherCrab configured them.".into(),
                ]),
                interface: self.interface.clone(),
                slaves,
            })
        })
    }

    /// Move the network to OP and exchange process data every `cycle` for `duration`.
    /// Afterwards the slaves go back to PRE-OP.
    pub fn run(&self, cycle: Duration, duration: Duration, max_records: usize) -> Result<RunResult, MasterError> {
        if cycle.is_zero() {
            return Err(MasterError::Invalid("cycle time must be positive".into()));
        }
        async_io::block_on(async {
            let group = self.init().await?;
            let emulated = is_emulated(&group, &self.md);
            let group = group.into_op(&self.md).await.map_err(ec)?;
            let mut expected = 0u16;
            let mut image = 0u32;
            for sd in group.iter(&self.md) {
                let (i, o) = (sd.inputs_raw().len(), sd.outputs_raw().len());
                image += (i + o) as u32;
                expected += if o > 0 { 2 } else { 0 } + if i > 0 { 1 } else { 0 };
            }
            let cycle_ns = cycle.as_nanos() as u64;
            let n_cycles = (duration.as_nanos() as u64).div_ceil(cycle_ns).max(1);
            let bus = EthercrabBus { md: &self.md, group: Some(&group), origin: Instant::now() };
            let t0 = bus.now_ns();
            let (mut late, mut periods, mut rtts, mut busy) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
            let (mut mismatches, mut lost, mut skipped, mut overruns) = (0u64, 0u64, 0u64, 0u64);
            let mut records = Vec::new();
            let mut truncated = false;
            let mut prev = None;
            let mut k = 0u64;
            let mut cycles = 0u64;
            while k < n_cycles {
                let planned = t0 + k * cycle_ns;
                bus.sleep_until_ns(planned).await;
                let start = bus.now_ns();
                cycles += 1;
                late.push((start - planned) as f64);
                if let Some(p) = prev {
                    periods.push((start - p) as f64);
                }
                prev = Some(start);
                let r = bus.cyclic().await;
                let end = bus.now_ns();
                let rtt = (end - start) as f64;
                let wkc = match r {
                    Ok(w) => {
                        rtts.push(rtt);
                        if w != Some(expected) {
                            mismatches += 1;
                        }
                        w
                    }
                    Err(BusError::Lost) => {
                        lost += 1;
                        None
                    }
                    Err(BusError::Fatal(e)) => return Err(MasterError::Bus(e)),
                };
                busy.push(rtt);
                if end > planned + cycle_ns {
                    overruns += 1;
                }
                if records.len() < max_records {
                    records.push(CycleRecord {
                        index: k,
                        start_ns: (start - t0) as f64,
                        late_ns: (start - planned) as f64,
                        cyclic_ns: Some(rtt),
                        cyclic_wkc: wkc,
                        mailbox_accesses: 0,
                        busy_ns: rtt,
                    });
                } else {
                    truncated = true;
                }
                let now_k = (end - t0) / cycle_ns;
                k = if now_k > k + 1 {
                    skipped += now_k - k - 1;
                    now_k
                } else {
                    k + 1
                };
            }
            let group = group.into_safe_op(&self.md).await.map_err(ec)?.into_pre_op(&self.md).await.map_err(ec)?;
            let mut final_states = Vec::new();
            for sd in group.iter(&self.md) {
                final_states.push(self.al_status(sd.configured_address()).await.0);
            }
            Ok(RunResult {
                meta: self.meta(emulated, vec![
                    "The expected working counter assumes one LRW per cycle over the process image: 2 per slave with outputs plus 1 per slave with inputs.".into(),
                    "The round trip is from before EtherCrab's tx_rx call to its return, so it includes EtherCrab's AL status reads in the same exchange.".into(),
                ]),
                interface: self.interface.clone(),
                slaves: group.len() as u16,
                cycle_time_ns: cycle_ns as f64,
                process_image_bytes: image,
                expected_wkc: expected,
                wkc_mismatches: mismatches,
                lost_frames: lost,
                final_states,
                timing: CycleTiming {
                    cycles,
                    skipped_cycles: skipped,
                    overruns,
                    start_late: Summary::of(&late),
                    period: Summary::of(&periods),
                    cyclic: Summary::of(&rtts),
                    mailbox_access: Summary::default(),
                    busy: Summary::of(&busy),
                    max_accesses_per_cycle: 0,
                },
                cycle_records: records,
                cycle_records_truncated: truncated,
            })
        })
    }

    /// Run a transfer to the slave with configured address `station`. Its mailbox comes from
    /// its SyncManager 0 and 1 registers. With `op`, the network goes to OP and process data
    /// is exchanged at the start of every cycle (as in the simulator); otherwise the slaves
    /// stay in PRE-OP and cycles carry only mailbox traffic. The job's `mailbox` is replaced
    /// by what the registers say.
    pub fn transfer(
        &self,
        station: u16,
        mut job: TransferJob,
        options: &TransferOptions,
        op: bool,
    ) -> Result<MeasuredTransfer, MasterError> {
        async_io::block_on(async {
            let group = self.init().await?;
            let Some(sd) = group.iter(&self.md).find(|s| s.configured_address() == station) else {
                let found: Vec<String> =
                    group.iter(&self.md).map(|s| format!("{:#06x}", s.configured_address())).collect();
                return Err(MasterError::Invalid(format!(
                    "no slave with station address {station:#06x} (found {})",
                    found.join(", ")
                )));
            };
            if job.slave.is_empty() {
                job.slave = sd.name().to_owned();
            }
            let sms = self.sync_managers(station).await?;
            let find = |d: SmDirection| sms.iter().find(|(_, s)| s.mode() == SmMode::Mailbox && s.direction() == d);
            let (Some((_, out)), Some((_, inp))) = (find(SmDirection::EcatWrite), find(SmDirection::EcatRead)) else {
                return Err(MasterError::Invalid(format!(
                    "slave {station:#06x} has no mailbox SyncManagers configured (no mailbox in its EEPROM?)"
                )));
            };
            job.mailbox = MailboxLayout {
                station,
                out_start: out.start,
                out_len: out.length,
                in_start: inp.start,
                in_len: inp.length,
            };
            let mut o = options.clone();
            o.assumptions.extend(host_assumptions());
            if sd.name() == EMULATED_ESC_NAME {
                o.assumptions.push(EMULATED_ESC_ASSUMPTION.into());
            }
            o.assumptions.push(format!(
                "Mailbox from the slave's SyncManager registers: SM0 {:#06x} ({} B), SM1 {:#06x} ({} B).",
                out.start, out.length, inp.start, inp.length
            ));
            if op {
                o.assumptions
                    .push("The network was in OP: every cycle starts with EtherCrab's process data exchange.".into());
                let group = group.into_op(&self.md).await.map_err(ec)?;
                let bus = EthercrabBus { md: &self.md, group: Some(&group), origin: self.origin };
                let r = run_transfer(&bus, job, &o).await;
                let back = group.into_safe_op(&self.md).await.map_err(ec)?.into_pre_op(&self.md).await;
                let r = r?;
                back.map_err(ec)?;
                Ok(r)
            } else {
                o.assumptions.push("The network was in PRE-OP: cycles carry only mailbox traffic.".into());
                let bus = EthercrabBus { md: &self.md, group: None, origin: self.origin };
                run_transfer(&bus, job, &o).await
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eeprom_protocol_word() {
        assert_eq!(protocols(0x0E), vec!["EoE", "CoE", "FoE"]);
        assert!(protocols(0).is_empty());
    }

    #[test]
    fn working_counter_errors_become_replies() {
        let r = map_read::<Vec<u8>>(Err(EcError::WorkingCounter { expected: 1, received: 0 }), 4, |d| d);
        assert_eq!(r, Ok(Reply { data: vec![0; 4], wkc: 0 }));
        let r = map_read(Ok(vec![1u8, 2]), 2, |d| d);
        assert_eq!(r, Ok(Reply { data: vec![1, 2], wkc: 1 }));
    }

    #[test]
    fn waiting_reaches_the_deadline() {
        let t = Instant::now() + Duration::from_millis(3);
        wait_until(t);
        assert!(Instant::now() >= t);
    }
}
