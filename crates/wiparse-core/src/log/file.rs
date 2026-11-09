//! Disk-backed log store: mmap file + line offset index + LRU chunk cache.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crossbeam_channel::Sender;
use memmap2::Mmap;

use super::store::LogStore;

/// Lines per cached chunk (tuned for ~50–150 KB per chunk).
const CHUNK_LINES: usize = 512;
/// Max chunks in RAM (~32K lines ≈ a few MB).
const MAX_CACHED_CHUNKS: usize = 64;

pub enum FileBuildEvent {
    Progress { lines: usize },
    Done(Arc<FileLogStore>),
    Err(String),
}

pub struct FileLogStore {
    path: PathBuf,
    mmap: Mmap,
    line_starts: Arc<[u64]>,
    max_line_chars: usize,
    cache: Mutex<ChunkCache>,
}

struct ChunkCache {
    map: HashMap<usize, Arc<[Arc<str>]>>,
    order: Vec<usize>,
}

impl ChunkCache {
    fn new() -> Self {
        Self {
            map: HashMap::new(),
            order: Vec::new(),
        }
    }

    fn get(&mut self, chunk_id: usize) -> Option<Arc<[Arc<str>]>> {
        if let Some(v) = self.map.get(&chunk_id) {
            // bump LRU
            self.order.retain(|&c| c != chunk_id);
            self.order.push(chunk_id);
            return Some(Arc::clone(v));
        }
        None
    }

    fn insert(&mut self, chunk_id: usize, chunk: Arc<[Arc<str>]>) {
        if self.map.contains_key(&chunk_id) {
            self.order.retain(|&c| c != chunk_id);
        } else if self.map.len() >= MAX_CACHED_CHUNKS {
            if let Some(old) = self.order.first().copied() {
                self.order.remove(0);
                self.map.remove(&old);
            }
        }
        self.map.insert(chunk_id, Arc::clone(&chunk));
        self.order.push(chunk_id);
    }
}

impl FileLogStore {
    fn open(path: PathBuf, line_starts: Vec<u64>, max_line_chars: usize) -> Result<Self, String> {
        let file = File::open(&path).map_err(|e| e.to_string())?;
        let mmap = unsafe { Mmap::map(&file).map_err(|e| e.to_string())? };
        Ok(Self {
            path,
            mmap,
            line_starts: line_starts.into(),
            max_line_chars,
            cache: Mutex::new(ChunkCache::new()),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn slice_line_bytes(&self, index: usize) -> Option<&[u8]> {
        let start = *self.line_starts.get(index)? as usize;
        let end = if index + 1 < self.line_starts.len() {
            self.line_starts[index + 1] as usize
        } else {
            self.mmap.len()
        };
        if start > end || end > self.mmap.len() {
            return None;
        }
        let mut slice = &self.mmap[start..end];
        while slice.last().copied() == Some(b'\n') || slice.last().copied() == Some(b'\r') {
            slice = &slice[..slice.len().saturating_sub(1)];
        }
        Some(slice)
    }

    fn decode_line(bytes: &[u8]) -> Arc<str> {
        let decoded = String::from_utf8_lossy(bytes);
        if decoded.chars().any(|ch| {
            matches!(ch, '\u{0085}' | '\u{2028}' | '\u{2029}') || (ch.is_control() && ch != '\t')
        }) {
            Arc::from(
                decoded
                    .chars()
                    .map(|ch| {
                        if matches!(ch, '\u{0085}' | '\u{2028}' | '\u{2029}')
                            || (ch.is_control() && ch != '\t')
                        {
                            ' '
                        } else {
                            ch
                        }
                    })
                    .collect::<String>(),
            )
        } else {
            Arc::from(decoded.as_ref())
        }
    }

    fn load_chunk(&self, chunk_id: usize) -> Arc<[Arc<str>]> {
        let start_line = chunk_id * CHUNK_LINES;
        let end_line = (start_line + CHUNK_LINES).min(self.line_starts.len());
        let mut lines = Vec::with_capacity(end_line.saturating_sub(start_line));
        for i in start_line..end_line {
            if let Some(bytes) = self.slice_line_bytes(i) {
                lines.push(Self::decode_line(bytes));
            } else {
                lines.push(Arc::from(""));
            }
        }
        lines.into()
    }

    fn chunk_for_line(&self, index: usize) -> Option<Arc<[Arc<str>]>> {
        if index >= self.line_starts.len() {
            return None;
        }
        let chunk_id = index / CHUNK_LINES;
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(c) = cache.get(chunk_id) {
            return Some(c);
        }
        let chunk = self.load_chunk(chunk_id);
        cache.insert(chunk_id, Arc::clone(&chunk));
        Some(chunk)
    }

    /// Warm the LRU chunk cache for lines in `[start, end)`.
    pub fn prefetch_range(&self, start: usize, end: usize) {
        if self.line_starts.is_empty() {
            return;
        }
        let end = end.min(self.line_starts.len());
        let start = start.min(end);
        if start >= end {
            let _ = self.chunk_for_line(start.min(self.line_starts.len().saturating_sub(1)));
            return;
        }
        let first = start / CHUNK_LINES;
        let last = (end - 1) / CHUNK_LINES;
        for chunk_id in first..=last {
            let _ = self.chunk_for_line(chunk_id * CHUNK_LINES);
        }
    }
}

impl LogStore for FileLogStore {
    fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    fn line_at(&self, index: usize) -> Option<Arc<str>> {
        let chunk = self.chunk_for_line(index)?;
        let local = index % CHUNK_LINES;
        chunk.get(local).cloned()
    }

    fn max_line_chars(&self) -> usize {
        self.max_line_chars
    }
}

fn sidecar_path(path: &Path) -> PathBuf {
    let mut p = path.to_path_buf();
    let name = format!(
        "{}.wiparse-v3.idx",
        path.file_name().and_then(|s| s.to_str()).unwrap_or("log")
    );
    p.set_file_name(name);
    p
}

// Version 3 also records subsecond file modification time.
const IDX_MAGIC: &[u8; 8] = b"WPRIDX03";

fn read_u64_le(r: &mut impl Read) -> std::io::Result<u64> {
    let mut b = [0u8; 8];
    r.read_exact(&mut b)?;
    Ok(u64::from_le_bytes(b))
}

fn read_u32_le(r: &mut impl Read) -> std::io::Result<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}

fn write_u64_le(w: &mut impl Write, v: u64) -> std::io::Result<()> {
    w.write_all(&v.to_le_bytes())
}

fn write_u32_le(w: &mut impl Write, v: u32) -> std::io::Result<()> {
    w.write_all(&v.to_le_bytes())
}

fn try_load_sidecar(path: &Path, meta: &std::fs::Metadata) -> Option<(Vec<u64>, usize)> {
    let side = sidecar_path(path);
    let mut f = File::open(&side).ok()?;
    let mut magic = [0u8; 8];
    f.read_exact(&mut magic).ok()?;
    if &magic != IDX_MAGIC {
        return None;
    }
    let mtime = read_u64_le(&mut f).ok()?;
    let size = read_u64_le(&mut f).ok()?;
    let max_chars = read_u32_le(&mut f).ok()? as usize;
    let count = usize::try_from(read_u64_le(&mut f).ok()?).ok()?;
    let file_mtime = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    let file_mtime=u64::try_from(file_mtime).ok()?;
    if mtime != file_mtime || size != meta.len() {
        return None;
    }
    let encoded_bytes=(count as u64).checked_mul(8)?.checked_add(36)?;
    if count as u64>size || max_chars as u64>size || f.metadata().ok()?.len()!=encoded_bytes{return None;}
    let mut starts = Vec::with_capacity(count);
    for _ in 0..count {
        let offset=read_u64_le(&mut f).ok()?;
        if offset>size||starts.last().is_some_and(|last|*last>=offset){return None;}
        starts.push(offset);
    }
    Some((starts, max_chars))
}

fn write_sidecar(path: &Path, meta: &std::fs::Metadata, starts: &[u64], max_chars: usize) {
    let side = sidecar_path(path);
    // A sidecar can be an existing user file or a link. Never truncate it just
    // because the cached index is stale or malformed.
    let Ok(mut f) = std::fs::OpenOptions::new().write(true).create_new(true).open(&side) else {
        return;
    };
    let _ = f.write_all(IDX_MAGIC);
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d|u64::try_from(d.as_nanos()).ok())
        .unwrap_or(0);
    let _ = write_u64_le(&mut f, mtime);
    let _ = write_u64_le(&mut f, meta.len());
    let _ = write_u32_le(&mut f, max_chars.min(u32::MAX as usize) as u32);
    let _ = write_u64_le(&mut f, starts.len() as u64);
    for &off in starts {
        let _ = write_u64_le(&mut f, off);
    }
}

fn build_index(
    path: &Path,
    progress: Option<&Sender<FileBuildEvent>>,
) -> Result<(Vec<u64>, usize), String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if let Some(cached) = try_load_sidecar(path, &meta) {
        return Ok(cached);
    }

    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(file);
    let mut starts = Vec::new();
    let mut max_chars = 0usize;
    let mut offset: u64 = 0;
    let mut first = true;
    let mut line_buf = Vec::new();

    loop {
        line_buf.clear();
        let n = reader
            .read_until(b'\n', &mut line_buf)
            .map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        let has_lf = line_buf.last() == Some(&b'\n');
        let mut content_end = line_buf.len() - usize::from(has_lf);
        // Treat the CR in CRLF as part of the same terminator. Other CR bytes
        // below are independent line endings.
        if has_lf && content_end > 0 && line_buf[content_end - 1] == b'\r' {
            content_end -= 1;
        }

        let mut segment_start = 0usize;
        for segment_end in (0..content_end)
            .filter(|&index| line_buf[index] == b'\r')
            .chain(std::iter::once(content_end))
        {
            let is_empty_tail = segment_start == content_end && !has_lf;
            if !is_empty_tail {
                let mut absolute_start = offset + segment_start as u64;
                let mut content = &line_buf[segment_start..segment_end];
                if first {
                    first = false;
                    if content.starts_with(&[0xEF, 0xBB, 0xBF]) {
                        absolute_start += 3;
                        content = &content[3..];
                    }
                }
                starts.push(absolute_start);
                max_chars = max_chars.max(String::from_utf8_lossy(content).chars().count());
            }
            segment_start = segment_end.saturating_add(1);
        }
        offset += n as u64;
        if starts.len().is_multiple_of(50_000) {
            if let Some(tx) = progress {
                if tx.send(FileBuildEvent::Progress {
                    lines: starts.len(),
                }).is_err() { return Err("log indexing cancelled".into()); }
            }
        }
    }

    write_sidecar(path, &meta, &starts, max_chars);
    Ok((starts, max_chars))
}

/// Background worker: build / load index then mmap the file.
pub fn build_file_store_worker(path: PathBuf, tx: Sender<FileBuildEvent>) {
    let result: Result<Arc<FileLogStore>, String> = (|| {
        let (starts, max_chars) = build_index(&path, Some(&tx))?;
        let store = FileLogStore::open(path, starts, max_chars)?;
        Ok(Arc::new(store))
    })();
    match result {
        Ok(store) => {
            let _ = tx.send(FileBuildEvent::Done(store));
        }
        Err(e) => {
            let _ = tx.send(FileBuildEvent::Err(e));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn indexing_does_not_replace_an_existing_sidecar() {
        let dir=std::env::temp_dir().join(format!("wiparse-index-preserve-{}",uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();let path=dir.join("sample.log");let side=sidecar_path(&path);
        std::fs::write(&path,b"one\ntwo\n").unwrap();std::fs::write(&side,b"existing content").unwrap();
        let (starts,_)=build_index(&path,None).unwrap();assert_eq!(starts.len(),2);
        assert_eq!(std::fs::read(&side).unwrap(),b"existing content");assert_eq!(std::fs::read(&path).unwrap(),b"one\ntwo\n");
        std::fs::remove_file(side).unwrap();std::fs::remove_file(path).unwrap();std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn malformed_index_count_is_rejected_without_allocating_it() {
        let dir=std::env::temp_dir().join(format!("wiparse-index-invalid-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&dir).unwrap();
        let path=dir.join("sample.log");let side=sidecar_path(&path);std::fs::write(&path,b"a\n").unwrap();
        let meta=std::fs::metadata(&path).unwrap();let timestamp=u64::try_from(meta.modified().unwrap().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()).unwrap();
        let mut file=File::create(&side).unwrap();file.write_all(IDX_MAGIC).unwrap();write_u64_le(&mut file,timestamp).unwrap();write_u64_le(&mut file,meta.len()).unwrap();write_u32_le(&mut file,1).unwrap();write_u64_le(&mut file,u64::MAX).unwrap();drop(file);
        assert!(try_load_sidecar(&path,&meta).is_none());
        std::fs::remove_file(side).unwrap();std::fs::remove_file(path).unwrap();std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn fresh_index_is_reused_and_offsets_are_checked() {
        let dir=std::env::temp_dir().join(format!("wiparse-index-valid-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&dir).unwrap();
        let path=dir.join("sample.log");let side=sidecar_path(&path);std::fs::write(&path,b"first\nsecond\n").unwrap();
        let old=dir.join("sample.log.wiparse.idx");std::fs::write(&old,b"previous index").unwrap();
        let original=build_index(&path,None).unwrap();let meta=std::fs::metadata(&path).unwrap();
        assert_eq!(try_load_sidecar(&path,&meta).unwrap(),original);
        assert_eq!(std::fs::read(&old).unwrap(),b"previous index");
        let mut bytes=std::fs::read(&side).unwrap();let last=bytes.len()-8;bytes[last..].copy_from_slice(&(meta.len()+1).to_le_bytes());std::fs::write(&side,bytes).unwrap();
        assert!(try_load_sidecar(&path,&meta).is_none());
        std::fs::remove_file(old).unwrap();std::fs::remove_file(side).unwrap();std::fs::remove_file(path).unwrap();std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn index_supports_a_bom_only_file() {
        let dir=std::env::temp_dir().join(format!("wiparse-index-bom-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&dir).unwrap();
        let path=dir.join("sample.log");let side=sidecar_path(&path);std::fs::write(&path,b"\xEF\xBB\xBF").unwrap();
        let built=build_index(&path,None).unwrap();assert_eq!(try_load_sidecar(&path,&std::fs::metadata(&path).unwrap()).unwrap(),built);
        std::fs::remove_file(side).unwrap();std::fs::remove_file(path).unwrap();std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn closing_the_log_consumer_stops_the_indexing_worker() {
        let dir=std::env::temp_dir().join(format!("wiparse-index-cancel-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&dir).unwrap();
        let path=dir.join("many.log");std::fs::write(&path,"x\n".repeat(100_000)).unwrap();
        let (tx,rx)=crossbeam_channel::unbounded();drop(rx);
        assert!(build_index(&path,Some(&tx)).unwrap_err().contains("cancelled"));assert!(!sidecar_path(&path).exists());
        std::fs::remove_file(path).unwrap();std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn file_store_reads_lines() {
        let dir = std::env::temp_dir().join("wiparse_log_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("sample.log");
        {
            let mut f = File::create(&path).unwrap();
            writeln!(f, "line one").unwrap();
            writeln!(f, "line two").unwrap();
        }
        let (starts, max) = build_index(&path, None).unwrap();
        let store = FileLogStore::open(path, starts, max).unwrap();
        assert_eq!(store.line_count(), 2);
        assert_eq!(store.line_at(0).as_deref(), Some("line one"));
        assert_eq!(store.line_at(1).as_deref(), Some("line two"));
    }

    #[test]
    fn file_store_splits_mixed_and_lone_cr_line_endings() {
        let dir =
            std::env::temp_dir().join(format!("wiparse_mixed_lines_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("mixed.log");
        let _ = std::fs::remove_file(sidecar_path(&path));
        std::fs::write(
            &path,
            b"\xEF\xBB\xBFfirst\rsecond\r\nthird\nfourth\rfifth\x08!",
        )
        .unwrap();

        let (starts, max) = build_index(&path, None).unwrap();
        let store = FileLogStore::open(path, starts, max).unwrap();

        assert_eq!(store.line_count(), 5);
        assert_eq!(store.line_at(0).as_deref(), Some("first"));
        assert_eq!(store.line_at(1).as_deref(), Some("second"));
        assert_eq!(store.line_at(2).as_deref(), Some("third"));
        assert_eq!(store.line_at(3).as_deref(), Some("fourth"));
        assert_eq!(store.line_at(4).as_deref(), Some("fifth !"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn dropping_file_store_allows_rewrite() {
        let dir = std::env::temp_dir().join(format!("wiparse_rewrite_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("locked.log");
        std::fs::write(&path, "before\n").unwrap();
        let (starts, max) = build_index(&path, None).unwrap();
        let store = FileLogStore::open(path.clone(), starts, max).unwrap();
        assert_eq!(store.line_at(0).as_deref(), Some("before"));
        drop(store);
        std::fs::write(&path, "after\n").unwrap();
        let (starts, max) = build_index(&path, None).unwrap();
        let store = FileLogStore::open(path, starts, max).unwrap();
        assert_eq!(store.line_at(0).as_deref(), Some("after"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
