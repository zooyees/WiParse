//! In-memory ring buffer for the live serial tab (Python `_MAX_LIVE_LINES` parity).

use std::collections::VecDeque;
use std::sync::Arc;

use super::store::LogStore;

/// Live tab line cap (display FIFO). Full capture is on disk when save-to-disk is on.
pub const MAX_LIVE_LINES: usize = 5_000;

type LogLine = Arc<str>;

fn intern(s: &str) -> LogLine {
    Arc::from(s)
}

pub struct LiveLogStore {
    lines: VecDeque<LogLine>,
    max_line_chars: usize,
}

impl Default for LiveLogStore {
    fn default() -> Self {
        Self {
            lines: VecDeque::new(),
            max_line_chars: 0,
        }
    }
}

impl LiveLogStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.lines.clear();
        self.max_line_chars = 0;
    }

    /// Append lines and return how many rows were evicted from the front.
    pub fn append_lines(&mut self, lines: impl IntoIterator<Item = impl AsRef<str>>) -> usize {
        let mut evicted = 0usize;
        let mut recalc_max = false;
        for line in lines {
            let s = line.as_ref();
            while self.lines.len() >= MAX_LIVE_LINES {
                if let Some(old) = self.lines.pop_front() {
                    recalc_max |= old.chars().count() == self.max_line_chars;
                    evicted += 1;
                }
            }
            let n = s.chars().count();
            if n > self.max_line_chars {
                self.max_line_chars = n;
            }
            self.lines.push_back(intern(s));
        }
        if recalc_max {
            self.max_line_chars = self
                .lines
                .iter()
                .map(|line| line.chars().count())
                .max()
                .unwrap_or(0);
        }
        evicted
    }

    pub fn set_lines(&mut self, lines: impl IntoIterator<Item = impl AsRef<str>>) {
        self.clear();
        let _ = self.append_lines(lines);
    }
}

impl LogStore for LiveLogStore {
    fn line_count(&self) -> usize {
        self.lines.len()
    }

    fn line_at(&self, index: usize) -> Option<Arc<str>> {
        self.lines.get(index).cloned()
    }

    fn max_line_chars(&self) -> usize {
        self.max_line_chars
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_reports_front_eviction_and_refreshes_max_width() {
        let mut store = LiveLogStore::new();
        let long = "x".repeat(80);
        store.append_lines(std::iter::once(long));
        store.append_lines((1..MAX_LIVE_LINES).map(|_| "short"));
        assert_eq!(store.max_line_chars(), 80);

        let evicted = store.append_lines(["new"]);
        assert_eq!(evicted, 1);
        assert_eq!(store.line_count(), MAX_LIVE_LINES);
        assert_eq!(store.max_line_chars(), 5);
    }
}
