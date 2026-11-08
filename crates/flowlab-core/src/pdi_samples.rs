//! The PDI timing samples file: timed ESC accesses by a slave application, one per line.
//! `flowlab-pdi` writes it (`flowlab pdi-bench`), `flowlab-study` reads it (`flowlab pdi-fit`),
//! and a slave stack can log it itself.
//!
//! ```text
//! # Verdin, LAN9252, spidev at 30 MHz
//! # flowlab:provenance=measured
//! direction,bytes,duration_ns
//! read,128,7350
//! write,64,5100
//! ```
//!
//! Lines starting with `#` are comments; a `flowlab:provenance=` marker in one labels the
//! samples. `r` and `w` are accepted for the direction, and durations may have a fraction.
//!
//! ```
//! use flowlab_core::Provenance;
//! use flowlab_core::pdi_samples::{Direction, Sample, parse, provenance, to_csv};
//!
//! let s = [Sample { direction: Direction::Read, bytes: 128, duration_ns: 7350.0 }];
//! let text = to_csv(&s, &["a test".into()], Provenance::Simulated);
//! assert_eq!(parse(&text).unwrap(), s);
//! assert_eq!(provenance(&text), Provenance::Simulated);
//! ```

use std::fmt;

use crate::Provenance;

/// The header line.
pub const HEADER: &str = "direction,bytes,duration_ns";

/// Direction of a timed access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Direction {
    /// ESC → application.
    Read,
    /// Application → ESC.
    Write,
}

impl Direction {
    /// `read` or `write`, as in the file.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
        }
    }
}

/// One timed ESC access.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    /// Read or write.
    pub direction: Direction,
    /// Bytes moved by the access.
    pub bytes: u32,
    /// Wall time of the complete access, ns.
    pub duration_ns: f64,
}

/// A line that is not a sample.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SampleError {
    /// 1-based line number.
    pub line: usize,
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for SampleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for SampleError {}

/// The file: `comments` as `#` lines, the provenance marker, the header, one line per sample.
pub fn to_csv(samples: &[Sample], comments: &[String], provenance: Provenance) -> String {
    let mut s = String::new();
    for c in comments {
        for line in c.lines() {
            s += &format!("# {line}\n");
        }
    }
    s += &format!("# {}{}\n{HEADER}\n", Provenance::COMMENT_PREFIX, provenance.as_str());
    for x in samples {
        s += &format!("{},{},{}\n", x.direction.as_str(), x.bytes, x.duration_ns);
    }
    s
}

/// Read a file: the header, then one access per line. Blank and `#` lines are skipped.
pub fn parse(text: &str) -> Result<Vec<Sample>, SampleError> {
    let mut out = Vec::new();
    let mut header = false;
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let err = |message: String| SampleError { line: n + 1, message };
        let cols: Vec<&str> = line.split(',').map(str::trim).collect();
        if !header {
            if cols.join(",") != HEADER {
                return Err(err(format!("expected the header `{HEADER}`, found `{line}`")));
            }
            header = true;
            continue;
        }
        let [d, b, t] = cols[..] else { return Err(err(format!("expected 3 columns, found {}", cols.len()))) };
        let direction = match d.to_ascii_lowercase().as_str() {
            "read" | "r" => Direction::Read,
            "write" | "w" => Direction::Write,
            _ => return Err(err(format!("direction `{d}` is neither read nor write"))),
        };
        let bytes: u32 = b.parse().map_err(|_| err(format!("bytes `{b}` is not a whole number")))?;
        let duration_ns: f64 = t.parse().map_err(|_| err(format!("duration `{t}` is not a number")))?;
        if bytes == 0 || !(duration_ns.is_finite() && duration_ns > 0.0) {
            return Err(err("bytes and duration must be positive".into()));
        }
        out.push(Sample { direction, bytes, duration_ns });
    }
    if !header {
        return Err(SampleError { line: 1, message: "empty file".into() });
    }
    Ok(out)
}

/// Provenance of a file: the `flowlab:provenance=…` marker in a `#` comment line, else
/// `measured`.
pub fn provenance(text: &str) -> Provenance {
    text.lines()
        .map(str::trim)
        .filter(|l| l.starts_with('#'))
        .find_map(Provenance::from_comment)
        .unwrap_or(Provenance::Measured)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bad_lines_are_rejected_with_their_number() {
        assert_eq!(parse("bytes,duration\n").unwrap_err().line, 1);
        let e = parse("direction,bytes,duration_ns\nread,12\n").unwrap_err();
        assert_eq!(e, SampleError { line: 2, message: "expected 3 columns, found 2".into() });
        assert!(parse("direction,bytes,duration_ns\nsideways,1,2\n").is_err());
        assert!(parse("direction,bytes,duration_ns\nread,0,2\n").is_err());
        assert!(parse("").is_err());
        let s = parse("# x\n\ndirection, bytes, duration_ns\nW,64,10.5\n").unwrap();
        assert_eq!(s, [Sample { direction: Direction::Write, bytes: 64, duration_ns: 10.5 }]);
        assert_eq!(provenance("direction,bytes,duration_ns\n"), Provenance::Measured);
    }

    #[test]
    fn whole_nanoseconds_are_written_without_a_fraction() {
        let s = [Sample { direction: Direction::Write, bytes: 4, duration_ns: 1200.0 }];
        assert!(to_csv(&s, &[], Provenance::Measured).ends_with("\nwrite,4,1200\n"));
    }
}
