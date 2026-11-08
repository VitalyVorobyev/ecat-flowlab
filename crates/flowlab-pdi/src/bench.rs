//! The benchmark: complete ESC reads and writes of several sizes, in random order, each timed
//! on its own, written as the samples file `flowlab pdi-fit` reads.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use flowlab_core::pdi_samples::{Direction, Sample};
use flowlab_core::rng::SplitMix64;
use flowlab_core::stats::quantile;

use crate::PdiError;
use crate::lan9252::{Lan9252, LinkInfo, check_range, reg};
use crate::spi::{Clock, SpiBus};

/// Access sizes timed by default, bytes.
pub const DEFAULT_SIZES: [u32; 11] = [1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024];

/// A range of the ESC address space, `START+LEN` (`0x1800+1024`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Area {
    /// First byte.
    pub start: u16,
    /// Length, bytes.
    pub len: u16,
}

impl Area {
    /// One past the last byte.
    pub fn end(&self) -> u32 {
        u32::from(self.start) + u32::from(self.len)
    }
}

impl fmt::Display for Area {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{:04X}+{}", self.start, self.len)
    }
}

fn number(s: &str) -> Result<u16, String> {
    let s = s.trim();
    let r = match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(h) => u16::from_str_radix(h, 16),
        None => s.parse(),
    };
    r.map_err(|_| format!("`{s}` is not a 16-bit number"))
}

impl FromStr for Area {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (a, l) = s.split_once('+').ok_or_else(|| format!("`{s}` is not START+LEN (e.g. 0x1800+1024)"))?;
        let area = Self { start: number(a)?, len: number(l)? };
        if area.len == 0 {
            return Err("the area is empty".into());
        }
        Ok(area)
    }
}

/// What to time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BenchOptions {
    /// Access sizes, bytes.
    pub sizes: Vec<u32>,
    /// Timed accesses per direction and size.
    pub reps: u32,
    /// Untimed accesses per direction and size before the timed ones.
    pub warmup: u32,
    /// Where accesses go. Reads default to the start of process RAM; writes need it set.
    pub area: Option<Area>,
    /// Also time writes (into `area`, which must lie in process RAM).
    pub allow_writes: bool,
    /// Seed of the order shuffle.
    pub seed: u64,
}

impl Default for BenchOptions {
    fn default() -> Self {
        Self { sizes: DEFAULT_SIZES.to_vec(), reps: 50, warmup: 3, area: None, allow_writes: false, seed: 1 }
    }
}

/// Checked options: where reads and writes go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// Sizes, ascending, without duplicates.
    pub sizes: Vec<u32>,
    /// Area of the reads.
    pub read_area: Area,
    /// Area of the writes, if writes are allowed.
    pub write_area: Option<Area>,
}

/// Check the options against the safety rules before touching the bus: reads stay out of the
/// AL Control and SyncManager registers; writes happen only with `allow_writes` and an explicit
/// area in process RAM; every size fits its area.
pub fn plan(o: &BenchOptions) -> Result<Plan, PdiError> {
    let mut sizes = o.sizes.clone();
    sizes.sort_unstable();
    sizes.dedup();
    let Some(&max) = sizes.last() else { return Err(PdiError::Options("no access sizes".into())) };
    if sizes[0] == 0 || max > 4096 {
        return Err(PdiError::Options("access sizes must be 1 to 4096 bytes".into()));
    }
    if o.reps == 0 {
        return Err(PdiError::Options("at least one repetition".into()));
    }
    let read_area = o.area.unwrap_or(Area { start: reg::PROCESS_RAM_START, len: max as u16 });
    check_range(read_area.start, usize::from(read_area.len))?;
    if u32::from(read_area.len) < max {
        return Err(PdiError::Options(format!("{max}-byte accesses do not fit the area {read_area}")));
    }
    let write_area = match (o.allow_writes, o.area) {
        (false, _) => None,
        (true, None) => {
            return Err(PdiError::Refused(
                "writes need --area START+LEN: a range of process RAM (0x1000 and up) that nothing uses".into(),
            ));
        }
        (true, Some(a)) if a.start < reg::PROCESS_RAM_START => {
            return Err(PdiError::Refused(format!(
                "writes go only to process RAM (0x1000 and up); the area {a} starts below it"
            )));
        }
        (true, Some(a)) => Some(a),
    };
    Ok(Plan { sizes, read_area, write_area })
}

/// Result of [`run`].
#[derive(Debug, Clone, PartialEq)]
pub struct BenchRun {
    /// The link check.
    pub link: LinkInfo,
    /// The checked plan.
    pub plan: Plan,
    /// Timed accesses, in the order they ran.
    pub samples: Vec<Sample>,
}

/// SplitMix64, for a reproducible order.
const STATE_INIT: u16 = 1;

/// Check the link, refuse unless the ESC is in INIT, then time every access of the plan in a
/// shuffled order. With writes, one write and read-back of the largest size checks the area
/// first (untimed). AL Status is read again at the end; a change discards the run.
pub fn run<S: SpiBus, C: Clock + ?Sized>(
    esc: &mut Lan9252<S>,
    clock: &C,
    o: &BenchOptions,
    chip_id: u16,
) -> Result<BenchRun, PdiError> {
    let plan = plan(o)?;
    let link = esc.check_link(chip_id)?;
    if link.al_status & 0x0F != STATE_INIT {
        return Err(PdiError::Refused(format!(
            "the ESC is in {}, not INIT: stop the master and the slave application first",
            link.al_state()
        )));
    }
    let max = *plan.sizes.last().unwrap_or(&1) as usize;
    let mut rng = SplitMix64::new(o.seed);
    let pattern: Vec<u8> = (0..max).map(|_| rng.next_u64() as u8).collect();
    let mut buf = vec![0u8; max];
    if let Some(a) = plan.write_area {
        esc.write(a.start, &pattern)?;
        esc.read(a.start, &mut buf)?;
        if buf != pattern {
            return Err(PdiError::Link(format!("process RAM at {a} does not read back what was written")));
        }
    }
    let mut jobs: Vec<(Direction, u32)> = Vec::new();
    for &size in &plan.sizes {
        jobs.extend(std::iter::repeat_n((Direction::Read, size), o.reps as usize));
        if plan.write_area.is_some() {
            jobs.extend(std::iter::repeat_n((Direction::Write, size), o.reps as usize));
        }
    }
    for i in (1..jobs.len()).rev() {
        jobs.swap(i, rng.below(i as u64 + 1) as usize);
    }
    let mut access = |esc: &mut Lan9252<S>, d: Direction, n: usize| match d {
        Direction::Read => esc.read(plan.read_area.start, &mut buf[..n]),
        Direction::Write => esc.write(plan.write_area.map_or(0, |a| a.start), &pattern[..n]),
    };
    for &size in &plan.sizes {
        for _ in 0..o.warmup {
            access(esc, Direction::Read, size as usize)?;
            if plan.write_area.is_some() {
                access(esc, Direction::Write, size as usize)?;
            }
        }
    }
    let mut samples = Vec::with_capacity(jobs.len());
    for (direction, bytes) in jobs {
        let t0 = clock.now_ns();
        access(esc, direction, bytes as usize)?;
        let t1 = clock.now_ns();
        samples.push(Sample { direction, bytes, duration_ns: t1.saturating_sub(t0).max(1) as f64 });
    }
    let after = esc.check_link(chip_id)?;
    if after.al_status != link.al_status {
        return Err(PdiError::Refused(format!(
            "AL Status changed during the run (to {}): something else drives the ESC; samples discarded",
            after.al_state()
        )));
    }
    Ok(BenchRun { link, plan, samples })
}

/// Timing of one direction at one size.
#[derive(Debug, Clone, PartialEq)]
pub struct SizeSummary {
    /// Read or write.
    pub direction: Direction,
    /// Access size, bytes.
    pub bytes: u32,
    /// Timed accesses.
    pub samples: u64,
    /// Fastest access, ns.
    pub min_ns: f64,
    /// Median, ns.
    pub median_ns: f64,
    /// 95th percentile, ns.
    pub p95_ns: f64,
}

/// Median and 95th percentile (linear interpolation, as `flowlab pdi-fit`) per direction and size.
pub fn summarise(samples: &[Sample]) -> Vec<SizeSummary> {
    let mut by: BTreeMap<(Direction, u32), Vec<f64>> = BTreeMap::new();
    for s in samples {
        by.entry((s.direction, s.bytes)).or_default().push(s.duration_ns);
    }
    by.into_iter()
        .map(|((direction, bytes), mut v)| {
            v.sort_by(f64::total_cmp);
            SizeSummary {
                direction,
                bytes,
                samples: v.len() as u64,
                min_ns: v[0],
                median_ns: quantile(&v, 0.5),
                p95_ns: quantile(&v, 0.95),
            }
        })
        .collect()
}
