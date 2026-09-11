//! Disk-backed and live log stores for the virtualized log viewer.
//!
//! - [`LiveLogStore`]: in-memory ring buffer (capped at [`MAX_LIVE_LINES`]).
//! - [`FileLogStore`]: read-only `mmap` + line-offset index + line cache.
//! - Substring filter helpers (`|` = OR) shared by GUI find / highlight.

use crossbeam_channel::Sender;
use memmap2::Mmap;
use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Soft cap for the live ring buffer (matches GUI “last 5000 lines” copy).
pub const MAX_LIVE_LINES: usize = 5_000;
/// Cap find/filter hit lists so a common substring cannot exhaust RAM.
pub const MAX_FILTER_MATCHES: usize = 50_000;

const PROGRESS_EVERY: usize = 50_000;
const LINE_CACHE_CAP: usize = 4_096;

/// One substring match inside a store line (byte offsets into the line).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextHit {
    pub row: usize,
    pub start: usize,
    pub end: usize,
}

/// Shared read API for live and file-backed stores.
pub trait LogStore: Send + Sync {
    fn line_count(&self) -> usize;
    fn line_at(&self, index: usize) -> Option<Arc<str>>;
    fn max_line_chars(&self) -> usize;
}

/// In-memory ring used by the live serial tab.
#[derive(Debug, Default)]
pub struct LiveLogStore {
    lines: Vec<Arc<str>>,
    max_line_chars: usize,
}

impl LiveLogStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append lines; when over [`MAX_LIVE_LINES`], drop oldest from the front.
    /// Returns how many leading lines were evicted.
    pub fn append_lines<I, S>(&mut self, lines: I) -> usize
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for line in lines {
            let arc: Arc<str> = Arc::from(line.as_ref());
            self.max_line_chars = self.max_line_chars.max(arc.chars().count());
            self.lines.push(arc);
        }
        let excess = self.lines.len().saturating_sub(MAX_LIVE_LINES);
        if excess > 0 {
            self.lines.drain(..excess);
            self.rescan_max_chars();
        }
        excess
    }

    pub fn clear(&mut self) {
        self.lines.clear();
        self.max_line_chars = 0;
    }

    fn rescan_max_chars(&mut self) {
        self.max_line_chars = self
            .lines
            .iter()
            .map(|line| line.chars().count())
            .max()
            .unwrap_or(0);
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

struct LineCache {
    map: HashMap<usize, Arc<str>>,
    order: VecDeque<usize>,
    capacity: usize,
}

impl LineCache {
    fn new(capacity: usize) -> Self {
        Self {
            map: HashMap::with_capacity(capacity.min(1024)),
            order: VecDeque::with_capacity(capacity.min(1024)),
            capacity: capacity.max(1),
        }
    }

    fn get(&self, index: usize) -> Option<Arc<str>> {
        self.map.get(&index).cloned()
    }

    fn insert(&mut self, index: usize, line: Arc<str>) {
        if self.map.contains_key(&index) {
            self.map.insert(index, line);
            return;
        }
        while self.map.len() >= self.capacity {
            if let Some(old) = self.order.pop_front() {
                self.map.remove(&old);
            } else {
                break;
            }
        }
        self.order.push_back(index);
        self.map.insert(index, line);
    }
}

/// Memory-mapped log file with O(1) line addressing.
pub struct FileLogStore {
    /// Kept so Windows holds a share mode compatible with read-only mapping.
    _file: File,
    map: Mmap,
    /// `line_offsets[i]` = byte start of line `i`; last entry is EOF.
    line_offsets: Vec<usize>,
    max_line_chars: usize,
    cache: Mutex<LineCache>,
}

impl FileLogStore {
    /// Index `path` into a store. `on_progress` is called with lines found so far.
    pub fn open(path: &Path, mut on_progress: impl FnMut(usize)) -> std::io::Result<Self> {
        let file = File::open(path)?;
        // SAFETY: read-only map; we only read bytes for the lifetime of `Self`.
        let map = unsafe { Mmap::map(&file)? };
        let (line_offsets, max_line_chars) = index_lines(&map, &mut on_progress);
        Ok(Self {
            _file: file,
            map,
            line_offsets,
            max_line_chars,
            cache: Mutex::new(LineCache::new(LINE_CACHE_CAP)),
        })
    }

    /// Warm the line cache for `[start, end)`.
    pub fn prefetch_range(&self, start: usize, end: usize) {
        let n = self.line_count();
        if n == 0 {
            return;
        }
        let start = start.min(n);
        let end = end.min(n).max(start);
        for i in start..end {
            let _ = self.line_at(i);
        }
    }

    fn decode_line(&self, index: usize) -> Option<Arc<str>> {
        let n = self.line_count();
        if index >= n {
            return None;
        }
        let start = *self.line_offsets.get(index)?;
        let mut end = *self.line_offsets.get(index + 1)?;
        if end > start && self.map[end - 1] == b'\n' {
            end -= 1;
        }
        if end > start && self.map[end - 1] == b'\r' {
            end -= 1;
        }
        let bytes = &self.map[start..end];
        Some(Arc::<str>::from(String::from_utf8_lossy(bytes).into_owned()))
    }
}

impl LogStore for FileLogStore {
    fn line_count(&self) -> usize {
        self.line_offsets.len().saturating_sub(1)
    }

    fn line_at(&self, index: usize) -> Option<Arc<str>> {
        if index >= self.line_count() {
            return None;
        }
        if let Ok(cache) = self.cache.lock() {
            if let Some(hit) = cache.get(index) {
                return Some(hit);
            }
        }
        let line = self.decode_line(index)?;
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(index, Arc::clone(&line));
        }
        Some(line)
    }

    fn max_line_chars(&self) -> usize {
        self.max_line_chars
    }
}

fn index_lines(map: &[u8], on_progress: &mut dyn FnMut(usize)) -> (Vec<usize>, usize) {
    let mut offsets = Vec::with_capacity(map.len() / 64 + 2);
    offsets.push(0);
    let mut max_chars = 0usize;
    let mut line_start = 0usize;
    let mut lines = 0usize;

    let mut i = 0usize;
    while i < map.len() {
        if map[i] == b'\n' {
            let mut end = i;
            if end > line_start && map[end - 1] == b'\r' {
                end -= 1;
            }
            max_chars = max_chars.max(utf8_char_count(&map[line_start..end]));
            offsets.push(i + 1);
            line_start = i + 1;
            lines += 1;
            if lines % PROGRESS_EVERY == 0 {
                on_progress(lines);
            }
        }
        i += 1;
    }

    // Content after the last newline (no trailing `\n`) is still one line.
    // A final newline is treated as a terminator, not an extra blank row.
    // Empty files expose zero lines (`offsets == [0]`).
    if line_start < map.len() {
        max_chars = max_chars.max(utf8_char_count(&map[line_start..]));
        offsets.push(map.len());
        lines += 1;
    }

    on_progress(lines);
    (offsets, max_chars)
}

fn utf8_char_count(bytes: &[u8]) -> usize {
    String::from_utf8_lossy(bytes).chars().count()
}

/// Events from [`build_file_store_worker`].
pub enum FileBuildEvent {
    Progress { lines: usize },
    Done(Arc<FileLogStore>),
    Err(String),
}

/// Background indexer used when opening / reloading a log file.
pub fn build_file_store_worker(path: PathBuf, tx: Sender<FileBuildEvent>) {
    match FileLogStore::open(&path, |lines| {
        let _ = tx.send(FileBuildEvent::Progress { lines });
    }) {
        Ok(store) => {
            let _ = tx.send(FileBuildEvent::Done(Arc::new(store)));
        }
        Err(e) => {
            let _ = tx.send(FileBuildEvent::Err(e.to_string()));
        }
    }
}

/// Split a filter string on `|` (OR). Empty / whitespace-only → `None`.
pub fn parse_filter_patterns(filter: &str) -> Option<Vec<String>> {
    let trimmed = filter.trim();
    if trimmed.is_empty() {
        return None;
    }
    let parts: Vec<String> = trimmed
        .split('|')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts)
    }
}

/// Lowercase needles when `case_sensitive` is false (ASCII fold).
pub fn prepared_needles(patterns: Option<&[String]>, case_sensitive: bool) -> Vec<String> {
    let Some(patterns) = patterns else {
        return Vec::new();
    };
    patterns
        .iter()
        .filter(|p| !p.is_empty())
        .map(|p| {
            if case_sensitive {
                p.clone()
            } else {
                p.to_ascii_lowercase()
            }
        })
        .collect()
}

/// Byte ranges of all needle hits in `line`, merged when overlapping.
pub fn match_ranges_in_line(
    line: &str,
    needles: &[String],
    case_sensitive: bool,
) -> Vec<(usize, usize)> {
    if needles.is_empty() {
        return Vec::new();
    }
    let haystack;
    let search: &str = if case_sensitive {
        line
    } else {
        haystack = line.to_ascii_lowercase();
        &haystack
    };
    let mut ranges = Vec::new();
    for needle in needles {
        if needle.is_empty() {
            continue;
        }
        for (start, matched) in search.match_indices(needle.as_str()) {
            ranges.push((start, start + matched.len()));
        }
    }
    if ranges.is_empty() {
        return ranges;
    }
    ranges.sort_unstable_by_key(|(s, e)| (*s, *e));
    merge_overlapping(ranges)
}

fn merge_overlapping(ranges: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::with_capacity(ranges.len());
    for (start, end) in ranges {
        if let Some((_, last_end)) = out.last_mut() {
            if start <= *last_end {
                *last_end = (*last_end).max(end);
                continue;
            }
        }
        out.push((start, end));
    }
    out
}

/// Unique master-row indices from ordered hits (preserves first-seen order).
pub fn unique_match_rows(hits: &[TextHit]) -> Vec<usize> {
    let mut rows = Vec::new();
    let mut last = None;
    for hit in hits {
        if last != Some(hit.row) {
            rows.push(hit.row);
            last = Some(hit.row);
        }
    }
    rows
}

/// Scan an entire store for filter hits; stop after [`MAX_FILTER_MATCHES`].
pub fn collect_match_hits<S: LogStore + ?Sized>(
    store: &S,
    needles: &[String],
    case_sensitive: bool,
) -> (Vec<TextHit>, bool) {
    let mut hits = Vec::new();
    if needles.is_empty() {
        return (hits, false);
    }
    let n = store.line_count();
    for row in 0..n {
        let Some(line) = store.line_at(row) else {
            continue;
        };
        for (start, end) in match_ranges_in_line(&line, needles, case_sensitive) {
            if hits.len() >= MAX_FILTER_MATCHES {
                return (hits, true);
            }
            hits.push(TextHit { row, start, end });
        }
    }
    (hits, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_temp(contents: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "wiparse-log-test-{}-{}.txt",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn live_ring_evicts_from_front() {
        let mut store = LiveLogStore::new();
        let mut batch = Vec::new();
        for i in 0..(MAX_LIVE_LINES + 3) {
            batch.push(format!("line-{i}"));
        }
        let evicted = store.append_lines(batch);
        assert_eq!(evicted, 3);
        assert_eq!(store.line_count(), MAX_LIVE_LINES);
        assert_eq!(store.line_at(0).as_deref(), Some("line-3"));
    }

    #[test]
    fn filter_or_and_casefold() {
        let patterns = parse_filter_patterns(" ASK | FSK ").unwrap();
        let needles = prepared_needles(Some(patterns.as_slice()), false);
        assert_eq!(needles, vec!["ask".to_string(), "fsk".to_string()]);
        let ranges = match_ranges_in_line("xx ASKyy", &needles, false);
        assert_eq!(ranges, vec![(3, 6)]);
    }

    #[test]
    fn overlapping_needles_merge() {
        let needles = vec!["ASK".to_string(), "SK ".to_string()];
        assert_eq!(
            match_ranges_in_line("ASK packet", &needles, true),
            vec![(0, 4)]
        );
    }

    #[test]
    fn file_store_indexes_crlf_and_worker() {
        let path = write_temp(b"a\r\nb\r\nc");
        let store = FileLogStore::open(&path, |_| {}).unwrap();
        assert_eq!(store.line_count(), 3);
        assert_eq!(store.line_at(0).as_deref(), Some("a"));
        assert_eq!(store.line_at(1).as_deref(), Some("b"));
        assert_eq!(store.line_at(2).as_deref(), Some("c"));
        store.prefetch_range(0, 3);

        let (tx, rx) = crossbeam_channel::unbounded();
        build_file_store_worker(path.clone(), tx);
        let mut done = None;
        while let Ok(ev) = rx.recv() {
            match ev {
                FileBuildEvent::Done(s) => {
                    done = Some(s);
                    break;
                }
                FileBuildEvent::Err(e) => panic!("{e}"),
                FileBuildEvent::Progress { .. } => {}
            }
        }
        let store = done.expect("Done");
        let needles = prepared_needles(parse_filter_patterns("b").as_deref(), true);
        let (hits, truncated) = collect_match_hits(store.as_ref(), &needles, true);
        assert!(!truncated);
        assert_eq!(hits, vec![TextHit { row: 1, start: 0, end: 1 }]);
        assert_eq!(unique_match_rows(&hits), vec![1]);
        let _ = std::fs::remove_file(path);
    }
}
