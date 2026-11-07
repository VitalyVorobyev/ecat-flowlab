//! Deterministic synthetic EtherCAT captures with known ground truth.
//!
//! The generator plays a master talking to a line of slaves and writes what a capture at
//! the master's NIC would show: every TX frame and, unless a fault drops it, the RX frame
//! the ring returns with working counters and input data filled in. Returning frames carry
//! the master MAC with the locally-administered bit set, as ESCs do.
//!
//! It is *not* a simulator of slave behaviour or bus timing beyond a simple round-trip model
//! (serialization + a fixed per-slave delay); its job is to produce captures whose cycle
//! time, jitter, faults and topology are known exactly, so analysis can be tested.

mod mailbox;
mod rng;
mod spec;

use std::io::Write;

use ecat_wire::{AlState, Command, DatagramSpec, FrameBuilder, MacAddr, wire_bytes};
use flowlab_capture::PcapNgWriter;
use flowlab_core::{Provenance, units};
use serde::Serialize;

pub use mailbox::{
    ABORT_NO_OBJECT, EOE_DST, EOE_ETHERTYPE, EOE_SRC, MBX_LEN, MISSING_OBJECT, MailboxTruth, SM0_START, SM1_START,
};
use mailbox::{Conversation, Step};
use rng::Rng;
pub use spec::{FaultSpec, MailboxSpec, SlaveSpec, SynthSpec};

/// Master MAC used by the generator: from the IANA documentation range (RFC 7042).
pub const MASTER_MAC: MacAddr = MacAddr([0x00, 0x00, 0x5E, 0x00, 0x53, 0x01]);
/// Station address of the first slave; slave `p` gets `FIRST_STATION + p`.
pub const FIRST_STATION: u16 = 0x1001;
/// Logical address where the process image starts.
pub const LOGICAL_BASE: u32 = 0x0001_0000;
const STARTUP_STEP_NS: u64 = 200_000;

/// What the generator actually produced. Analysis results are checked against this.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct GroundTruth {
    /// Seed used.
    pub seed: u64,
    /// Number of slaves.
    pub slaves: usize,
    /// Nominal cycle time, ns.
    pub cycle_time_ns: u64,
    /// Configured jitter standard deviation, ns.
    pub tx_jitter_ns: u32,
    /// Frames written in total.
    pub frames: u64,
    /// TX frames.
    pub tx_frames: u64,
    /// RX frames.
    pub rx_frames: u64,
    /// Frames (TX + RX) of the start-up sequence.
    pub startup_frames: u64,
    /// Cyclic TX frames sent.
    pub cyclic_tx_frames: u64,
    /// Cycles the master skipped.
    pub skipped_cycles: u64,
    /// Cyclic frames that did not return.
    pub lost_frames: u64,
    /// Cyclic frames returned with a wrong LRW working counter.
    pub wkc_errors: u64,
    /// LRW working counter of a healthy cycle.
    pub expected_lrw_wkc: u16,
    /// Process image size (outputs + inputs), bytes.
    pub process_image_bytes: u32,
    /// Expedited SDO uploads of the identity object performed.
    pub sdo_uploads: u64,
    /// Everything that went through mailboxes.
    pub mailbox: MailboxTruth,
    /// Time of the first cyclic TX frame relative to the first frame, ns.
    pub first_cyclic_ns: u64,
}

/// Why generation failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SynthError {
    /// The spec is out of range.
    #[error("invalid spec: {0}")]
    Spec(String),
    /// Writing the capture failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// A frame did not fit.
    #[error(transparent)]
    Wire(#[from] ecat_wire::WireError),
}

#[derive(Clone)]
struct Dg {
    command: Command,
    address: u32,
    data: Vec<u8>,
    wkc: u16,
}

fn dg(command: Command, adp: u16, ado: u16, data: Vec<u8>, wkc: u16) -> Dg {
    Dg { command, address: adp as u32 | (ado as u32) << 16, data, wkc }
}

struct Generator<W: Write> {
    out: PcapNgWriter<W>,
    rng: Rng,
    /// Separate stream for mailbox faults, so mailbox settings do not change cyclic faults.
    mbx_rng: Rng,
    reject_p: f64,
    response_delay: u64,
    t0_ns: u64,
    slave_delay_ns: u64,
    n: u16,
    index: u8,
    truth: GroundTruth,
}

impl<W: Write> Generator<W> {
    /// Write a TX frame at `t` and, when `rx` is set, the returned frame after the round trip.
    fn exchange(&mut self, t: u64, tx: &[Dg], rx: Option<&[Dg]>) -> Result<u64, SynthError> {
        let first_index = self.index;
        let build = |src: MacAddr, dgs: &[Dg], returning: bool| -> Result<Vec<u8>, SynthError> {
            let mut b = FrameBuilder::new(MacAddr([0xFF; 6]), src);
            for (i, d) in dgs.iter().enumerate() {
                let spec = DatagramSpec::new(d.command, first_index.wrapping_add(i as u8), d.address, &d.data);
                b.push(spec.with_wkc(if returning { d.wkc } else { 0 }))?;
            }
            Ok(b.finish())
        };
        self.index = self.index.wrapping_add(tx.len() as u8);
        let tx_bytes = build(MASTER_MAC, tx, false)?;
        self.out.packet(self.t0_ns + t, &tx_bytes)?;
        self.truth.tx_frames += 1;
        let rtt = units::serialization_ns(wire_bytes(tx_bytes.len() as u32) as u64, units::FAST_ETHERNET_BPS)
            + self.n as u64 * self.slave_delay_ns;
        if let Some(rx) = rx {
            let rx_bytes = build(MASTER_MAC.with_local_bit(), rx, true)?;
            self.out.packet(self.t0_ns + t + rtt, &rx_bytes)?;
            self.truth.rx_frames += 1;
        }
        Ok(rtt)
    }

    /// TX and a healthy RX that is identical except for `wkc`/`data` from `rx_fill`.
    fn simple(&mut self, t: u64, d: Dg) -> Result<(), SynthError> {
        let tx = Dg { wkc: 0, ..d.clone() };
        self.exchange(t, &[tx], Some(&[d]))?;
        Ok(())
    }

    /// One mailbox access for conversation `c` at tick `k`: the datagram as sent and as returned.
    fn mailbox_access(&mut self, c: &mut Conversation, k: u64) -> Option<(Dg, Dg)> {
        let t = &mut self.truth.mailbox;
        match c.steps.front()? {
            Step::Send(msg) => {
                let tx = dg(Command::Fpwr, c.station, SM0_START, msg.clone(), 0);
                let mut rx = tx.clone();
                if self.mbx_rng.chance(self.reject_p) {
                    t.rejected_writes += 1;
                } else {
                    rx.wkc = 1;
                    t.messages_to_slaves += 1;
                    c.steps.pop_front();
                    c.ready_at = k + 1 + self.response_delay;
                }
                Some((tx, rx))
            }
            Step::Receive(msg) => {
                let tx = dg(Command::Fprd, c.station, SM1_START, vec![0; MBX_LEN], 0);
                let mut rx = tx.clone();
                if k < c.ready_at {
                    t.empty_polls += 1;
                } else {
                    rx.data = msg.clone();
                    rx.wkc = 1;
                    t.messages_from_slaves += 1;
                    c.steps.pop_front();
                    c.ready_at = k + 1;
                }
                Some((tx, rx))
            }
        }
    }

    /// Advance every conversation by one access; `frame_per_slave` sends each access in its
    /// own frame `step_ns` apart, otherwise all go in one frame at `t`. Returns the time used.
    fn mailbox_tick(
        &mut self,
        convs: &mut [Conversation],
        k: u64,
        t: u64,
        frame_per_slave: Option<u64>,
    ) -> Result<u64, SynthError> {
        let mut pairs = Vec::new();
        for c in convs.iter_mut() {
            if let Some(p) = self.mailbox_access(c, k) {
                pairs.push(p);
            }
        }
        if pairs.is_empty() {
            return Ok(t);
        }
        match frame_per_slave {
            Some(step) => {
                let mut at = t;
                for (tx, rx) in pairs {
                    at += step;
                    self.exchange(at, &[tx], Some(&[rx]))?;
                }
                Ok(at)
            }
            None => {
                let (tx, rx): (Vec<Dg>, Vec<Dg>) = pairs.into_iter().unzip();
                self.exchange(t, &tx, Some(&rx))?;
                Ok(t)
            }
        }
    }
}

fn busy(convs: &[Conversation]) -> bool {
    convs.iter().any(|c| !c.steps.is_empty())
}

/// Generate a capture for `spec` into `out` (pcapng). Returns the ground truth.
pub fn generate<W: Write>(spec: &SynthSpec, out: W) -> Result<GroundTruth, SynthError> {
    spec.validate().map_err(SynthError::Spec)?;
    let comment = format!(
        "{}{}; generator=flowlab-synth {}; seed={}; {}",
        Provenance::COMMENT_PREFIX,
        Provenance::Synthetic.as_str(),
        env!("CARGO_PKG_VERSION"),
        spec.seed,
        spec.description
    );
    let writer = PcapNgWriter::new(out, concat!("flowlab-synth ", env!("CARGO_PKG_VERSION")), &comment, "synthetic0")?;
    let n = spec.slaves.len() as u16;
    let cycle_ns = spec.cycle_time_us as u64 * units::NS_PER_US;
    let mut g = Generator {
        out: writer,
        rng: Rng::new(spec.seed),
        mbx_rng: Rng::new(spec.seed ^ 0x6D62_7800_0000_0000),
        reject_p: spec.faults.mailbox_write_reject_probability,
        response_delay: spec.mailbox.response_delay_polls as u64,
        t0_ns: spec.start_unix_s * units::NS_PER_S,
        slave_delay_ns: spec.slave_delay_ns as u64,
        n,
        index: 0,
        truth: GroundTruth {
            seed: spec.seed,
            slaves: spec.slaves.len(),
            cycle_time_ns: cycle_ns,
            tx_jitter_ns: spec.tx_jitter_ns,
            ..GroundTruth::default()
        },
    };
    // One conversation per mailbox slave for the whole capture: its mailbox counters
    // continue from PREOP into OP, as a master's do.
    let mut convs: Vec<Conversation> = spec
        .slaves
        .iter()
        .enumerate()
        .filter(|(_, s)| s.mailbox)
        .map(|(p, _)| Conversation::new(FIRST_STATION + p as u16))
        .collect();
    let mut t = 0u64;
    if spec.include_startup {
        t = startup(&mut g, spec, &mut convs)?;
        g.truth.startup_frames = g.truth.tx_frames + g.truth.rx_frames;
    }

    // Cyclic phase.
    let image_out: usize = spec.slaves.iter().map(|s| s.outputs_bytes as usize).sum();
    let image_in: usize = spec.slaves.iter().map(|s| s.inputs_bytes as usize).sum();
    let image = image_out + image_in;
    let lrw_wkc: u16 = spec
        .slaves
        .iter()
        .map(|s| if s.outputs_bytes > 0 { 2 } else { 0 } + if s.inputs_bytes > 0 { 1 } else { 0 })
        .sum();
    g.truth.expected_lrw_wkc = lrw_wkc;
    g.truth.process_image_bytes = image as u32;
    let start = t + cycle_ns;
    g.truth.first_cyclic_ns = start;
    let max_jitter = (cycle_ns as f64 * 0.4) as i64;
    op_conversations(&mut g, spec, &mut convs);
    let op = &mut convs;
    for k in 0..spec.cycles as u64 {
        if busy(op) {
            g.mailbox_tick(op, k, start + k * cycle_ns + cycle_ns / 2, None)?;
        }
        if g.rng.chance(spec.faults.skipped_cycle_probability) {
            g.truth.skipped_cycles += 1;
            continue;
        }
        let jitter = ((g.rng.normal() * spec.tx_jitter_ns as f64) as i64).clamp(-max_jitter, max_jitter);
        let at = (start + k * cycle_ns).saturating_add_signed(jitter);
        let mut outputs = vec![0u8; image];
        for (i, b) in outputs.iter_mut().take(image_out).enumerate() {
            *b = (k as u8).wrapping_add(i as u8);
        }
        let mut tx = Vec::with_capacity(3);
        if spec.distributed_clocks {
            tx.push(dg(Command::Frmw, FIRST_STATION, 0x0910, vec![0; 8], 0));
        }
        tx.push(Dg { command: Command::Lrw, address: LOGICAL_BASE, data: outputs.clone(), wkc: 0 });
        tx.push(dg(Command::Brd, 0, 0x0130, vec![0, 0], 0));

        let lost = g.rng.chance(spec.faults.lost_frame_probability);
        let wkc_bad = !lost && g.rng.chance(spec.faults.wkc_error_probability);
        let mut rx = tx.clone();
        let mut i = 0;
        if spec.distributed_clocks {
            let sys_time = at + g.t0_ns - 946_684_800 * units::NS_PER_S; // DC epoch is 2000-01-01
            rx[0].data = sys_time.to_le_bytes().to_vec();
            rx[0].wkc = n;
            i = 1;
        }
        for b in rx[i].data.iter_mut().skip(image_out) {
            *b = g.rng.next_u64() as u8;
        }
        rx[i].wkc = if wkc_bad { lrw_wkc.saturating_sub(1 + g.rng.below(2) as u16) } else { lrw_wkc };
        rx[i + 1].data = vec![AlState::Op.code(), 0];
        rx[i + 1].wkc = n;
        if lost {
            g.truth.lost_frames += 1;
        }
        if wkc_bad {
            g.truth.wkc_errors += 1;
        }
        g.exchange(at, &tx, (!lost).then_some(rx.as_slice()))?;
        g.truth.cyclic_tx_frames += 1;
    }
    // Mailbox work left when the cyclic phase ends continues at the same cadence.
    let mut k = spec.cycles as u64;
    while busy(op) {
        g.mailbox_tick(op, k, start + k * cycle_ns + cycle_ns / 2, None)?;
        k += 1;
    }
    g.truth.frames = g.truth.tx_frames + g.truth.rx_frames;
    g.out.finish()?;
    Ok(g.truth)
}

/// OP-phase mailbox work: EoE and FoE on the first mailbox slave, emergencies from the last.
fn op_conversations<W: Write>(g: &mut Generator<W>, spec: &SynthSpec, convs: &mut [Conversation]) {
    let mb = &spec.mailbox;
    let m = &mut g.truth.mailbox;
    for c in convs.iter_mut() {
        c.ready_at = 0; // OP ticks count from 0
    }
    let Some((a, rest)) = convs.split_first_mut() else { return };
    for _ in 0..mb.eoe_frames_to_slave {
        a.eoe_frame(mb.eoe_frame_bytes as usize, true, m);
    }
    for _ in 0..mb.eoe_frames_from_slave {
        a.eoe_frame(mb.eoe_frame_bytes as usize, false, m);
    }
    if mb.foe_write_bytes > 0 {
        a.foe_write("firmware.bin", mb.foe_write_bytes as usize, m);
    }
    let e = rest.last_mut().unwrap_or(a);
    for i in 0..mb.emergencies {
        e.emergency(0xFF00 | (i as u16 & 0xFF), m);
    }
}

fn startup<W: Write>(g: &mut Generator<W>, spec: &SynthSpec, convs: &mut [Conversation]) -> Result<u64, SynthError> {
    let n = g.n;
    let mut t = 0u64;
    let mut step = || {
        t += STARTUP_STEP_NS;
        t
    };
    // Count slaves.
    g.simple(step(), dg(Command::Brd, 0, 0x0000, vec![0], n))?;
    // Assign station addresses by ring position.
    for p in 0..n {
        let station = FIRST_STATION + p;
        g.simple(step(), dg(Command::Apwr, p.wrapping_neg(), 0x0010, station.to_le_bytes().to_vec(), 1))?;
    }
    // Read ESC information, configure mailbox SyncManagers, request INIT ack.
    for (p, s) in spec.slaves.iter().enumerate() {
        let st = FIRST_STATION + p as u16;
        g.simple(step(), dg(Command::Fprd, st, 0x0000, vec![0x11, 0x00, 0x01, 0x00, 8, 8, 8, 0x0F], 1))?;
        if s.mailbox {
            let mut sm = Vec::with_capacity(16);
            for (start, ctrl) in [(SM0_START, 0x26u8), (SM1_START, 0x22u8)] {
                sm.extend_from_slice(&start.to_le_bytes());
                sm.extend_from_slice(&(MBX_LEN as u16).to_le_bytes());
                sm.extend_from_slice(&[ctrl, 0, 1, 0]);
            }
            g.simple(step(), dg(Command::Fpwr, st, 0x0800, sm, 1))?;
        }
    }
    // INIT -> PREOP.
    state_change(g, spec, &mut t, AlState::PreOp, AlState::Init)?;
    // Mailbox conversations in PREOP: identity object uploads, then the configured extras.
    for c in convs.iter_mut() {
        let m = &mut g.truth.mailbox;
        for k in 0..spec.sdo_uploads_per_slave {
            let value = g.rng.next_u64() as u32;
            c.expedited_upload(0x1018, (k % 4 + 1) as u8, value, m);
            g.truth.sdo_uploads += 1;
        }
        let mb = &spec.mailbox;
        if mb.segmented_upload_bytes > 0 {
            c.segmented_upload(0x2000, mb.segmented_upload_bytes as usize, m);
        }
        if mb.download_bytes > 0 {
            c.download(0x2001, mb.download_bytes as usize, m);
        }
        for _ in 0..mb.aborted_uploads {
            c.aborted_upload(m);
        }
    }
    let mut k = 0;
    while busy(convs) {
        t = g.mailbox_tick(convs, k, t, Some(STARTUP_STEP_NS))?;
        k += 1;
    }
    // Process data mapping: FMMU 0 per slave, then PREOP -> SAFEOP -> OP.
    let mut logical = LOGICAL_BASE;
    for (p, s) in spec.slaves.iter().enumerate() {
        let st = FIRST_STATION + p as u16;
        let len = s.outputs_bytes + s.inputs_bytes;
        let mut fmmu = Vec::with_capacity(16);
        fmmu.extend_from_slice(&logical.to_le_bytes());
        fmmu.extend_from_slice(&len.to_le_bytes());
        fmmu.extend_from_slice(&[0, 7, 0x00, 0x11, 0, 3, 1, 0, 0, 0]);
        t += STARTUP_STEP_NS;
        g.simple(t, dg(Command::Fpwr, st, 0x0600, fmmu, 1))?;
        logical += len as u32;
    }
    state_change(g, spec, &mut t, AlState::SafeOp, AlState::PreOp)?;
    state_change(g, spec, &mut t, AlState::Op, AlState::SafeOp)?;
    Ok(t)
}

/// Request `to` from every slave, then poll AL status twice: first still `from`, then `to`.
fn state_change<W: Write>(
    g: &mut Generator<W>,
    spec: &SynthSpec,
    t: &mut u64,
    to: AlState,
    from: AlState,
) -> Result<(), SynthError> {
    for p in 0..spec.slaves.len() as u16 {
        *t += STARTUP_STEP_NS;
        g.simple(*t, dg(Command::Fpwr, FIRST_STATION + p, 0x0120, vec![to.code(), 0], 1))?;
    }
    for state in [from, to] {
        for p in 0..spec.slaves.len() as u16 {
            *t += STARTUP_STEP_NS;
            g.simple(*t, dg(Command::Fprd, FIRST_STATION + p, 0x0130, vec![state.code(), 0], 1))?;
        }
    }
    Ok(())
}

/// The network configuration (ENI subset) a master would use for `spec`: station addresses,
/// mailboxes and the cyclic commands with their expected working counters.
pub fn network_config(spec: &SynthSpec) -> flowlab_devices::NetworkConfig {
    use flowlab_devices::{CyclicCmd, CyclicFrame, MailboxConfig, NetworkConfig, SlaveConfig};
    let n = spec.slaves.len() as u16;
    let image: u32 = spec.slaves.iter().map(|s| s.outputs_bytes as u32 + s.inputs_bytes as u32).sum();
    let lrw_wkc: u16 = spec
        .slaves
        .iter()
        .map(|s| if s.outputs_bytes > 0 { 2 } else { 0 } + if s.inputs_bytes > 0 { 1 } else { 0 })
        .sum();
    let mut cmds = Vec::new();
    if spec.distributed_clocks {
        cmds.push(CyclicCmd {
            command: Command::Frmw.code(),
            address: FIRST_STATION as u32 | 0x0910 << 16,
            length: 8,
            expected_wkc: n,
        });
    }
    cmds.push(CyclicCmd {
        command: Command::Lrw.code(),
        address: LOGICAL_BASE,
        length: image as u16,
        expected_wkc: lrw_wkc,
    });
    cmds.push(CyclicCmd { command: Command::Brd.code(), address: 0x0130 << 16, length: 2, expected_wkc: n });
    NetworkConfig {
        master: Some(concat!("flowlab-synth ", env!("CARGO_PKG_VERSION")).to_owned()),
        cycle_time_ns: Some(spec.cycle_time_us as u64 * 1000),
        slaves: spec
            .slaves
            .iter()
            .enumerate()
            .map(|(p, s)| SlaveConfig {
                name: s.name.clone(),
                station: FIRST_STATION + p as u16,
                position: Some(p as u16),
                vendor_id: None,
                product_code: None,
                revision: None,
                outputs_bits: s.outputs_bytes as u32 * 8,
                inputs_bits: s.inputs_bytes as u32 * 8,
                mailbox: s.mailbox.then(|| MailboxConfig {
                    out_start: SM0_START,
                    out_len: MBX_LEN as u16,
                    in_start: SM1_START,
                    in_len: MBX_LEN as u16,
                    protocols: vec!["CoE".into(), "FoE".into(), "EoE".into()],
                }),
            })
            .collect(),
        cyclic: vec![CyclicFrame { cmds }],
    }
}

/// JSON Schema of [`SynthSpec`] (draft 2020-12), for editors and the UI.
pub fn spec_schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(SynthSpec)).unwrap_or_default()
}

#[cfg(test)]
mod tests;
