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
    /// Lines popped from the front for FIFO. Absolute seq = dropped + window index.
    dropped: usize,
}

impl Default for LiveLogStore {
    fn default() -> Self {
        Self {
            lines: VecDeque::new(),
            max_line_chars: 0,
            dropped: 0,
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
        self.dropped = 0;
    }

    /// Number of lines evicted from the front since the last clear.
    pub fn dropped(&self) -> usize {
        self.dropped
    }

    /// Next absolute sequence number (dropped + current window length).
    pub fn abs_len(&self) -> usize {
        self.dropped.saturating_add(self.lines.len())
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
        self.dropped = self.dropped.saturating_add(evicted);
        evicted
    }

    pub fn set_lines(&mut self, lines: impl IntoIterator<Item = impl AsRef<str>>) {
        self.clear();
        let _ = self.append_lines(lines);
    }
}

/// Page an absolute session cursor into the live FIFO window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveAbsPage {
    pub window_from: usize,
    pub take: usize,
    pub next_abs: usize,
    pub total_abs: usize,
    pub truncated: bool,
}

pub fn live_abs_page(dropped: usize, window: usize, from_abs: usize, limit: usize) -> LiveAbsPage {
    let total_abs = dropped.saturating_add(window);
    let truncated = from_abs < dropped;
    let start_abs = from_abs.max(dropped);
    if start_abs >= total_abs || limit == 0 {
        return LiveAbsPage {
            window_from: window,
            take: 0,
            next_abs: start_abs.max(total_abs),
            total_abs,
            truncated,
        };
    }
    let window_from = start_abs - dropped;
    let take = (total_abs - start_abs).min(limit);
    LiveAbsPage {
        window_from,
        take,
        next_abs: start_abs + take,
        total_abs,
        truncated,
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
        assert_eq!(store.dropped(), 1);
        assert_eq!(store.abs_len(), MAX_LIVE_LINES + 1);
    }

    #[test]
    fn live_abs_page_keeps_cursor_after_fifo() {
        let page = live_abs_page(10, 5_000, 5_000, 200);
        assert!(!page.truncated);
        assert_eq!(page.window_from, 4_990);
        assert_eq!(page.take, 10);
        assert_eq!(page.next_abs, 5_010);
        assert_eq!(page.total_abs, 5_010);
    }

    #[test]
    fn live_abs_page_skips_gap_when_cursor_was_evicted() {
        let page = live_abs_page(8_000, 5_000, 5_000, 200);
        assert!(page.truncated);
        assert_eq!(page.window_from, 0);
        assert_eq!(page.take, 200);
        assert_eq!(page.next_abs, 8_200);
        assert_eq!(page.total_abs, 13_000);
    }

    #[test]
    fn live_abs_page_empty_at_head() {
        let page = live_abs_page(0, 5_000, 5_000, 200);
        assert_eq!(page.take, 0);
        assert_eq!(page.next_abs, 5_000);
        assert_eq!(page.total_abs, 5_000);
    }
}
