//! Common log store interface.

use std::sync::Arc;

/// Read-only view of log lines (live ring buffer or disk-backed file).
pub trait LogStore: Send + Sync {
    fn line_count(&self) -> usize;
    fn line_at(&self, index: usize) -> Option<Arc<str>>;
    /// Longest line in chars (for horizontal scroll width).
    fn max_line_chars(&self) -> usize;
}

impl<T: LogStore + ?Sized> LogStore for Arc<T> {
    fn line_count(&self) -> usize {
        (**self).line_count()
    }

    fn line_at(&self, index: usize) -> Option<Arc<str>> {
        (**self).line_at(index)
    }

    fn max_line_chars(&self) -> usize {
        (**self).max_line_chars()
    }
}
