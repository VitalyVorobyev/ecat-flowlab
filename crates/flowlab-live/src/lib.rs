//! Live EtherCAT capture, analysed while it runs.
//!
//! FlowLab does not open network interfaces itself. A [`LiveSession`] runs a capture tool
//! (Wireshark's `dumpcap`, or `tcpdump`) that writes pcap or pcapng to its standard output,
//! cuts the stream into blocks with [`StreamSplitter`], saves it to a file, and analyses it
//! with the same [`analyze`] as a capture file:
//!
//! * every tick (default 1 s), the frames that arrived since the previous tick become one
//!   point of a time series ([`LivePoint`]: frames, cycle period, jitter, round trip, WKC
//!   mismatches, mailbox messages) and add to the [`LiveTotals`];
//! * the last window of traffic (default 10 s) is analysed in full, giving a
//!   [`CaptureReport`] with streams, stations, AL states and mailbox channels.
//!
//! Capturing only listens: nothing is sent on the interface. A [`Source::Replay`] plays a
//! capture file back at its recorded pace instead, for trying the live view without hardware.
//!
//! ```
//! use flowlab_live::{LiveOptions, LiveSession, LiveState, Source};
//!
//! let mut bytes = Vec::new();
//! flowlab_synth::generate(&flowlab_synth::SynthSpec::example(), &mut bytes).unwrap();
//! let source = Source::Replay { bytes, name: "example.pcapng".into(), speed: 1000.0 };
//! let session = LiveSession::start(source, LiveOptions::default()).unwrap();
//! let done = session.wait();
//! assert_eq!(done.state, LiveState::Finished);
//! assert!(done.totals.ethercat_frames > 0);
//! ```

mod tool;

use std::collections::{BTreeMap, VecDeque};
use std::ffi::OsString;
use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use flowlab_analysis::mailbox::MailboxHint;
use flowlab_analysis::{AnalysisOptions, CaptureReport, ChannelSource, analyze};
use flowlab_capture::{Block, BlockKind, CaptureFile, StreamSplitter, Trace};
use flowlab_core::Provenance;
use serde::Serialize;
use thiserror::Error;
use ts_rs::TS;

pub use tool::{CaptureInterface, ETHERCAT_FILTER, Tool, find_tool, list_interfaces, parse_interface_list};

/// The packets of the analysis window are capped at this many bytes, whatever the window's
/// duration.
pub const MAX_WINDOW_BYTES: usize = 256 << 20;
/// Characters of the capture tool's error output kept for messages.
const STDERR_TAIL: usize = 4000;

/// Why a live session could not start.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum LiveError {
    /// No capture tool is installed.
    #[error("{0}")]
    NoTool(String),
    /// The capture tool could not be started.
    #[error("cannot start {0}: {1}")]
    Spawn(String, #[source] std::io::Error),
    /// The capture tool reported an error.
    #[error("{0}")]
    Tool(String),
    /// A bad option.
    #[error("{0}")]
    Option(String),
    /// The output file could not be written.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// `DIR/live-<unix seconds>.pcapng` (`.pcap` for tcpdump, which writes classic pcap).
pub fn default_save_path(dir: &Path, tool: Tool) -> PathBuf {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let ext = if tool == Tool::Tcpdump { "pcap" } else { "pcapng" };
    dir.join(format!("live-{secs}.{ext}"))
}

/// Where the packets come from.
#[derive(Debug, Clone)]
pub enum Source {
    /// A program writing pcap or pcapng to standard output, normally a capture tool
    /// ([`Source::interface`]).
    Command {
        /// The program.
        program: PathBuf,
        /// Its arguments.
        args: Vec<OsString>,
        /// What to call the source, e.g. `"dumpcap on eth1"`.
        label: String,
    },
    /// A capture file played back at its recorded pace.
    Replay {
        /// The file's bytes (pcap or pcapng).
        bytes: Vec<u8>,
        /// Its name.
        name: String,
        /// Playback speed: 1 is real time, 10 ten times faster.
        speed: f64,
    },
}

impl Source {
    /// Capture on `interface` with `tool` (at `program`), keeping only EtherCAT unless
    /// `filter` is `None`.
    pub fn interface(tool: Tool, program: PathBuf, interface: &str, filter: Option<&str>) -> Self {
        Source::Command {
            args: tool.capture_args(interface, filter),
            program,
            label: format!("{} on {interface}", tool.name()),
        }
    }

    fn label(&self) -> String {
        match self {
            Source::Command { label, .. } => label.clone(),
            Source::Replay { name, speed, .. } => format!("replay of {name} at {speed}×"),
        }
    }
}

/// How a live session analyses and keeps what it receives.
#[derive(Debug, Clone)]
pub struct LiveOptions {
    /// Traffic analysed in full each tick: the last this-much of capture time.
    pub window: Duration,
    /// Wall-clock time between analyses, and the width of one [`LivePoint`].
    pub tick: Duration,
    /// Points kept in the series.
    pub max_points: usize,
    /// Save everything received to this file (created, with its directory).
    pub save_to: Option<PathBuf>,
    /// Analysis options. Mailbox addresses learned during the session are added to
    /// `mailbox_hints`.
    pub analysis: AnalysisOptions,
}

impl Default for LiveOptions {
    fn default() -> Self {
        Self {
            window: Duration::from_secs(10),
            tick: Duration::from_secs(1),
            max_points: 600,
            save_to: None,
            analysis: AnalysisOptions { max_series_points: 300, ..AnalysisOptions::default() },
        }
    }
}

/// Whether a session is still receiving.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum LiveState {
    /// Receiving packets.
    Running,
    /// Stopped, or the source ended.
    Finished,
    /// The capture tool failed (see the message).
    Failed,
}

/// What arrived during one tick.
#[derive(Debug, Clone, Default, Serialize, TS)]
#[ts(export)]
pub struct LivePoint {
    /// End of the tick, seconds of wall-clock time since the session started.
    pub t_s: f64,
    /// EtherCAT frames.
    pub frames: u64,
    /// Mean link utilization by sent frames, percent.
    pub tx_percent: f64,
    /// Period of the main cyclic stream, ns.
    pub period_ns: Option<f64>,
    /// 99th percentile of its |phase jitter|, ns.
    pub jitter_p99_ns: Option<f64>,
    /// 99th percentile of its round trip, ns.
    pub round_trip_p99_ns: Option<f64>,
    /// Frames of cyclic streams with a working-counter mismatch.
    pub wkc_mismatches: u64,
    /// Sent frames that did not come back.
    pub unanswered: u64,
    /// Cycles missing in gaps of cyclic streams.
    pub missed_cycles: u64,
    /// Mailbox messages, both directions.
    pub mailbox_messages: u64,
    /// Frames that claim EtherCAT but do not parse.
    pub malformed: u64,
}

/// Sums over the whole session.
#[derive(Debug, Clone, Default, Serialize, TS)]
#[ts(export)]
pub struct LiveTotals {
    /// EtherCAT frames.
    pub ethercat_frames: u64,
    /// Frames of cyclic streams with a working-counter mismatch.
    pub wkc_mismatches: u64,
    /// Sent frames that did not come back.
    pub unanswered: u64,
    /// Cycles missing in gaps of cyclic streams.
    pub missed_cycles: u64,
    /// Mailbox messages, both directions.
    pub mailbox_messages: u64,
    /// Frames that claim EtherCAT but do not parse.
    pub malformed: u64,
}

/// The state of a live session.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct LiveSnapshot {
    /// Running, finished or failed.
    pub state: LiveState,
    /// Where the packets come from.
    pub source: String,
    /// Why it failed or ended, or the capture tool's last words.
    pub message: Option<String>,
    /// Provenance of the traffic (`measured` for an interface; a replay keeps the file's).
    pub provenance: Option<Provenance>,
    /// Seconds since the session started.
    pub elapsed_s: f64,
    /// Packets received.
    pub packets: u64,
    /// Bytes of packet blocks received.
    pub bytes: u64,
    /// The file everything is saved to.
    pub saved_to: Option<String>,
    /// Duration of the analysis window, ns.
    pub window_ns: f64,
    /// Sums over the session.
    pub totals: LiveTotals,
    /// One point per tick, oldest first.
    pub series: Vec<LivePoint>,
    /// Full analysis of the last window. Its times are relative to the window's first frame.
    pub window: Option<CaptureReport>,
    /// How the live figures are made; read with the report's own assumptions.
    pub assumptions: Vec<String>,
}

struct Shared {
    source: String,
    started: Instant,
    state: LiveState,
    message: Option<String>,
    stderr: String,
    /// The tool's error output has ended (or there is none to read).
    stderr_done: bool,
    header: Vec<u8>,
    window: VecDeque<Vec<u8>>,
    window_bytes: usize,
    fresh: usize,
    packets: u64,
    bytes: u64,
    reader_done: bool,
    provenance: Option<Provenance>,
    totals: LiveTotals,
    series: VecDeque<LivePoint>,
    report: Option<CaptureReport>,
}

type SharedRef = Arc<Mutex<Shared>>;

fn lock(s: &SharedRef) -> MutexGuard<'_, Shared> {
    s.lock().unwrap_or_else(|e| e.into_inner())
}

/// A running capture and its rolling analysis. Dropping it stops the capture.
pub struct LiveSession {
    shared: SharedRef,
    stop: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
    threads: Vec<JoinHandle<()>>,
    options: LiveOptions,
}

impl std::fmt::Debug for LiveSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveSession").field("source", &lock(&self.shared).source).finish_non_exhaustive()
    }
}

impl LiveSession {
    /// Start capturing from `source`.
    pub fn start(source: Source, options: LiveOptions) -> Result<Self, LiveError> {
        if options.tick.is_zero() || options.window.is_zero() {
            return Err(LiveError::Option("tick and window must be longer than zero".into()));
        }
        let save = match &options.save_to {
            Some(p) => {
                if let Some(dir) = p.parent().filter(|d| !d.as_os_str().is_empty()) {
                    std::fs::create_dir_all(dir)?;
                }
                Some(BufWriter::new(File::create(p)?))
            }
            None => None,
        };
        let shared = Arc::new(Mutex::new(Shared {
            source: source.label(),
            started: Instant::now(),
            state: LiveState::Running,
            message: None,
            stderr: String::new(),
            stderr_done: true,
            header: Vec::new(),
            window: VecDeque::new(),
            window_bytes: 0,
            fresh: 0,
            packets: 0,
            bytes: 0,
            reader_done: false,
            provenance: None,
            totals: LiveTotals::default(),
            series: VecDeque::new(),
            report: None,
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let child = Arc::new(Mutex::new(None));
        let mut threads = Vec::new();
        match source {
            Source::Command { program, args, label } => {
                let mut c = Command::new(&program)
                    .args(&args)
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .map_err(|e| LiveError::Spawn(label.clone(), e))?;
                let stdout = c.stdout.take().ok_or_else(|| LiveError::Tool("no standard output".into()))?;
                let stderr = c.stderr.take();
                *child.lock().unwrap_or_else(|e| e.into_inner()) = Some(c);
                if let Some(mut err) = stderr {
                    let sh = shared.clone();
                    lock(&sh).stderr_done = false;
                    threads.push(std::thread::spawn(move || {
                        let mut buf = [0u8; 1024];
                        while let Ok(n @ 1..) = err.read(&mut buf) {
                            let mut s = lock(&sh);
                            s.stderr.push_str(&String::from_utf8_lossy(&buf[..n]));
                            if s.stderr.len() > STDERR_TAIL {
                                let mut cut = s.stderr.len() - STDERR_TAIL;
                                while !s.stderr.is_char_boundary(cut) {
                                    cut += 1;
                                }
                                s.stderr.drain(..cut);
                            }
                        }
                        lock(&sh).stderr_done = true;
                    }));
                }
                let (sh, st, ch) = (shared.clone(), stop.clone(), child.clone());
                threads.push(std::thread::spawn(move || read_command(stdout, save, &sh, &st, &ch)));
            }
            Source::Replay { bytes, speed, .. } => {
                if speed.is_nan() || speed <= 0.0 {
                    return Err(LiveError::Option(format!("replay speed must be positive, not {speed}")));
                }
                let (sh, st) = (shared.clone(), stop.clone());
                threads.push(std::thread::spawn(move || replay(bytes, speed, save, &sh, &st)));
            }
        }
        let (sh, st, opts) = (shared.clone(), stop.clone(), options.clone());
        threads.push(std::thread::spawn(move || analyse_loop(&sh, &st, &opts)));
        Ok(Self { shared, stop, child, threads, options })
    }

    /// The current state, series and window analysis.
    pub fn snapshot(&self) -> LiveSnapshot {
        let s = lock(&self.shared);
        let window = self.options.window;
        let tick = self.options.tick;
        LiveSnapshot {
            state: s.state,
            source: s.source.clone(),
            message: s.message.clone().or_else(|| {
                let tail = s.stderr.trim();
                (s.state != LiveState::Running && !tail.is_empty()).then(|| tail.to_owned())
            }),
            provenance: s.provenance,
            elapsed_s: s.started.elapsed().as_secs_f64(),
            packets: s.packets,
            bytes: s.bytes,
            saved_to: self.options.save_to.as_ref().map(|p| p.display().to_string()),
            window_ns: window.as_nanos() as f64,
            totals: s.totals.clone(),
            series: s.series.iter().cloned().collect(),
            window: s.report.clone(),
            assumptions: vec![
                format!(
                    "Rolling analysis: the report covers the last {} s of capture time; each series point covers the frames that arrived in {} s of wall-clock time.",
                    window.as_secs_f64(),
                    tick.as_secs_f64()
                ),
                "Cyclic streams, pairing and working counters are found per point and per window, so a point holds about one tick's worth of frames; frames sent within the in-flight window before a tick's end are not counted as unanswered.".into(),
                "Mailbox addresses learned earlier in the session (from SyncManager configuration or the ENI) are passed on as hints once that configuration has left the window.".into(),
                "Timestamps are the capture tool's. Software timestamps include the capturing host's scheduling jitter.".into(),
            ],
        }
    }

    /// Series points that end after `t_s` (cheaper than a [`snapshot`](Self::snapshot)).
    pub fn points_after(&self, t_s: f64) -> Vec<LivePoint> {
        lock(&self.shared).series.iter().filter(|p| p.t_s > t_s).cloned().collect()
    }

    /// The file everything is saved to, if any.
    pub fn save_path(&self) -> Option<&Path> {
        self.options.save_to.as_deref()
    }

    /// Whether the source has ended (stopped, finished or failed).
    pub fn is_done(&self) -> bool {
        lock(&self.shared).reader_done
    }

    /// Stop capturing, finish the analysis and return the final state.
    pub fn stop(mut self) -> LiveSnapshot {
        self.shutdown();
        self.snapshot()
    }

    /// Wait for the source to end by itself (a replay, or a tool that exits), then return the
    /// final state.
    pub fn wait(mut self) -> LiveSnapshot {
        while !self.is_done() {
            std::thread::sleep(Duration::from_millis(20));
        }
        self.shutdown();
        self.snapshot()
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(c) = self.child.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            let _ = c.kill();
        }
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

impl Drop for LiveSession {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Take one block: save it and add it to the window.
fn ingest(shared: &SharedRef, save: &mut Option<BufWriter<File>>, block: Block) -> std::io::Result<()> {
    if let Some(f) = save.as_mut() {
        f.write_all(&block.bytes)?;
    }
    let mut s = lock(shared);
    match block.kind {
        BlockKind::Section => {
            s.header = block.bytes;
            s.window.clear();
            s.window_bytes = 0;
            s.fresh = 0;
        }
        BlockKind::Interface => s.header.extend(&block.bytes),
        BlockKind::Packet => {
            s.packets += 1;
            s.bytes += block.bytes.len() as u64;
            s.window_bytes += block.bytes.len();
            s.window.push_back(block.bytes);
            s.fresh += 1;
            while s.window_bytes > MAX_WINDOW_BYTES {
                let Some(old) = s.window.pop_front() else { break };
                s.window_bytes -= old.len();
            }
            s.fresh = s.fresh.min(s.window.len());
        }
        BlockKind::Other => {}
    }
    Ok(())
}

fn finish(shared: &SharedRef, state: LiveState, message: Option<String>) {
    let mut s = lock(shared);
    if s.state == LiveState::Running {
        s.state = state;
        if message.is_some() {
            s.message = message;
        }
    }
    s.reader_done = true;
}

fn read_command(
    mut out: impl Read,
    mut save: Option<BufWriter<File>>,
    shared: &SharedRef,
    stop: &AtomicBool,
    child: &Mutex<Option<Child>>,
) {
    let mut splitter = StreamSplitter::default();
    let mut buf = vec![0u8; 64 * 1024];
    let mut error = None;
    'read: loop {
        let n = match out.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                error = Some(format!("reading the capture tool's output: {e}"));
                break;
            }
        };
        splitter.push(&buf[..n]);
        loop {
            match splitter.next_block() {
                Ok(Some(b)) => {
                    if let Err(e) = ingest(shared, &mut save, b) {
                        error = Some(format!("saving the capture: {e}"));
                        break 'read;
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    error = Some(format!("the capture tool's output is not a capture: {e}"));
                    break 'read;
                }
            }
        }
        if let Some(f) = save.as_mut()
            && let Err(e) = f.flush()
        {
            error = Some(format!("saving the capture: {e}"));
            break;
        }
    }
    if let Some(f) = save.as_mut() {
        let _ = f.flush();
    }
    let mut c = child.lock().unwrap_or_else(|e| e.into_inner());
    if error.is_some()
        && let Some(c) = c.as_mut()
    {
        let _ = c.kill();
    }
    let status = c.as_mut().and_then(|c| c.wait().ok());
    drop(c);
    if stop.load(Ordering::SeqCst) {
        finish(shared, LiveState::Finished, None);
    } else if let Some(e) = error {
        finish(shared, LiveState::Failed, Some(e));
    } else {
        match status {
            Some(s) if s.success() => finish(shared, LiveState::Finished, Some("the capture tool exited".into())),
            Some(s) => {
                // The tool has exited; give the error-output reader a moment to finish.
                let deadline = Instant::now() + Duration::from_millis(500);
                while !lock(shared).stderr_done && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(5));
                }
                let tail = lock(shared).stderr.trim().to_owned();
                let msg = if tail.is_empty() { format!("the capture tool exited with {s}") } else { tail };
                finish(shared, LiveState::Failed, Some(msg));
            }
            None => finish(shared, LiveState::Failed, Some("the capture tool vanished".into())),
        }
    }
}

fn replay(bytes: Vec<u8>, speed: f64, mut save: Option<BufWriter<File>>, shared: &SharedRef, stop: &AtomicBool) {
    let file = match CaptureFile::parse(bytes.clone()) {
        Ok(f) => f,
        Err(e) => return finish(shared, LiveState::Failed, Some(e.to_string())),
    };
    let mut splitter = StreamSplitter::default();
    splitter.push(&bytes);
    let t0 = file.packets.iter().map(|p| p.ts_ns).find(|&t| t > 0).unwrap_or(0);
    let start = Instant::now();
    let mut packet = 0usize;
    loop {
        let block = match splitter.next_block() {
            Ok(Some(b)) => b,
            Ok(None) => break,
            Err(e) => return finish(shared, LiveState::Failed, Some(e.to_string())),
        };
        if block.kind == BlockKind::Packet {
            let ts = file.packets.get(packet).map_or(t0, |p| p.ts_ns);
            packet += 1;
            let due = Duration::from_secs_f64(ts.saturating_sub(t0) as f64 / 1e9 / speed);
            while let Some(wait) = due.checked_sub(start.elapsed()).filter(|w| *w > Duration::from_millis(1)) {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                std::thread::sleep(wait.min(Duration::from_millis(50)));
            }
        }
        if stop.load(Ordering::SeqCst) {
            break;
        }
        if let Err(e) = ingest(shared, &mut save, block) {
            return finish(shared, LiveState::Failed, Some(format!("saving the capture: {e}")));
        }
    }
    if let Some(f) = save.as_mut() {
        let _ = f.flush();
    }
    let msg = (!stop.load(Ordering::SeqCst)).then(|| "end of the replayed file".to_owned());
    finish(shared, LiveState::Finished, msg);
}

fn analyse_loop(shared: &SharedRef, stop: &AtomicBool, options: &LiveOptions) {
    let mut learned: BTreeMap<u16, MailboxHint> = BTreeMap::new();
    let mut next = Instant::now() + options.tick;
    loop {
        while Instant::now() < next {
            if stop.load(Ordering::SeqCst) || lock(shared).reader_done {
                break;
            }
            std::thread::sleep((next - Instant::now()).min(Duration::from_millis(20)));
        }
        let last = stop.load(Ordering::SeqCst) || lock(shared).reader_done;
        tick(shared, options, &mut learned);
        if last {
            return;
        }
        next += options.tick;
        let now = Instant::now();
        if next < now {
            next = now + options.tick;
        }
    }
}

fn point(t_s: f64, r: &CaptureReport) -> LivePoint {
    let s0 = r.streams.first();
    LivePoint {
        t_s,
        frames: r.summary.ethercat_frames,
        tx_percent: r.utilization.mean_tx_percent,
        period_ns: s0.map(|s| s.period_ns),
        jitter_p99_ns: s0.and_then(|s| s.phase_jitter_abs_p99_ns),
        round_trip_p99_ns: s0.and_then(|s| s.round_trip.as_ref().map(|d| d.p99)),
        wkc_mismatches: r.streams.iter().map(|s| s.wkc_events.len() as u64).sum(),
        unanswered: r.summary.unanswered_frames,
        missed_cycles: r.streams.iter().map(|s| s.missed_cycles).sum(),
        mailbox_messages: r.mailbox.channels.iter().map(|c| c.messages_out + c.messages_in).sum(),
        malformed: r.summary.malformed_frames,
    }
}

fn concat(header: &[u8], blocks: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::with_capacity(header.len() + blocks.iter().map(Vec::len).sum::<usize>());
    out.extend_from_slice(header);
    for b in blocks {
        out.extend(b);
    }
    out
}

fn tick(shared: &SharedRef, options: &LiveOptions, learned: &mut BTreeMap<u16, MailboxHint>) {
    let (window, chunk, name, t_s) = {
        let mut s = lock(shared);
        let t_s = s.started.elapsed().as_secs_f64();
        if s.header.is_empty() {
            return;
        }
        let fresh = std::mem::take(&mut s.fresh);
        let blocks: Vec<Vec<u8>> = s.window.iter().cloned().collect();
        let chunk = concat(&s.header, &blocks[blocks.len() - fresh..]);
        ((s.header.clone(), blocks), (fresh > 0).then_some(chunk), s.source.clone(), t_s)
    };
    let (header, blocks) = window;
    let mut opts = options.analysis.clone();
    for h in learned.values() {
        if !opts.mailbox_hints.iter().any(|u| u.station == h.station) {
            opts.mailbox_hints.push(*h);
        }
    }
    let p = match chunk.map(CaptureFile::parse) {
        Some(Ok(f)) => point(t_s, &analyze(&Trace::build(f, &name), &opts).report),
        Some(Err(e)) => {
            lock(shared).message = Some(format!("cannot read the received packets: {e}"));
            return;
        }
        None => LivePoint { t_s, ..LivePoint::default() },
    };
    // Drop what is older than the window before analysing it.
    let parse = |blocks: &[Vec<u8>]| CaptureFile::parse(concat(&header, blocks));
    let file = match parse(&blocks) {
        Ok(f) => f,
        Err(e) => {
            lock(shared).message = Some(format!("cannot read the received packets: {e}"));
            return;
        }
    };
    let newest = file.packets.iter().map(|p| p.ts_ns).max().unwrap_or(0);
    let cutoff = newest.saturating_sub(options.window.as_nanos() as u64);
    let expired = file.packets.iter().take_while(|p| p.ts_ns < cutoff).count();
    let file = if expired == 0 { file } else { parse(&blocks[expired..]).unwrap_or(file) };
    let trace = Trace::build(file, &name);
    let report = analyze(&trace, &opts).report;
    for c in &report.mailbox.channels {
        if matches!(c.source, ChannelSource::SmConfig | ChannelSource::Eni)
            && let (Some(out_start), Some(in_start)) = (c.out_start, c.in_start)
        {
            learned.insert(c.station, MailboxHint { station: c.station, out_start, in_start });
        }
    }
    let mut s = lock(shared);
    for _ in 0..expired {
        let Some(old) = s.window.pop_front() else { break };
        s.window_bytes -= old.len();
    }
    s.fresh = s.fresh.min(s.window.len());
    s.provenance = Some(trace.provenance);
    let t = &mut s.totals;
    t.ethercat_frames += p.frames;
    t.wkc_mismatches += p.wkc_mismatches;
    t.unanswered += p.unanswered;
    t.missed_cycles += p.missed_cycles;
    t.mailbox_messages += p.mailbox_messages;
    t.malformed += p.malformed;
    s.series.push_back(p);
    while s.series.len() > options.max_points {
        s.series.pop_front();
    }
    s.report = Some(report);
}

#[cfg(test)]
mod tests;
