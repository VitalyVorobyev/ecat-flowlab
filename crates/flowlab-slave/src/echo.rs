//! Recognising the slave's own answers when the socket hands them back.
//!
//! A macOS BPF socket delivers the frames this host sends to its own reader. With the source
//! MAC bit as the filter they are ignored anyway, but a slave [behind an
//! ESC](crate::SlaveConfig::behind_esc) answers frames that carry the bit, so it would take its
//! own answer for a new frame. [`Echoes`] remembers each answer for a short time and drops the
//! first received frame with the same bytes.

use std::collections::VecDeque;
use std::time::Duration;

/// How long a sent frame is expected back at most.
const WINDOW: Duration = Duration::from_millis(100);
/// Sent frames remembered at most.
const MAX: usize = 64;

/// The answers sent recently, oldest first, with the time they were sent.
#[derive(Debug, Default)]
pub(crate) struct Echoes {
    sent: VecDeque<(Duration, Vec<u8>)>,
}

impl Echoes {
    /// Remember a frame sent at `at` (any monotonic origin).
    pub(crate) fn sent(&mut self, at: Duration, frame: &[u8]) {
        if self.sent.len() == MAX {
            self.sent.pop_front();
        }
        self.sent.push_back((at, frame.to_vec()));
    }

    /// Whether `frame`, received at `at`, is one of the remembered answers coming back. A match
    /// is forgotten, so an identical frame received later is a new one.
    pub(crate) fn is_echo(&mut self, at: Duration, frame: &[u8]) -> bool {
        while self.sent.front().is_some_and(|(t, _)| at.saturating_sub(*t) > WINDOW) {
            self.sent.pop_front();
        }
        match self.sent.iter().position(|(_, f)| f == frame) {
            Some(i) => {
                self.sent.remove(i);
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn an_answer_is_dropped_once_and_only_while_fresh() {
        let mut e = Echoes::default();
        e.sent(MS, &[1, 2, 3]);
        assert!(!e.is_echo(2 * MS, &[1, 2, 4]), "other bytes");
        assert!(e.is_echo(2 * MS, &[1, 2, 3]));
        assert!(!e.is_echo(3 * MS, &[1, 2, 3]), "the same bytes again are a new frame");
        e.sent(10 * MS, &[5]);
        assert!(!e.is_echo(10 * MS + WINDOW + MS, &[5]), "too late to be the echo");
    }

    #[test]
    fn memory_is_bounded() {
        let mut e = Echoes::default();
        for i in 0..=MAX {
            e.sent(MS, &[(i % 256) as u8, (i / 256) as u8]);
        }
        assert_eq!(e.sent.len(), MAX);
        assert!(!e.is_echo(MS, &[0, 0]), "the oldest was forgotten");
        assert!(e.is_echo(MS, &[1, 0]));
    }
}
