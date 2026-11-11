use std::{io, ops::Index, sync::Arc};

type Point = [f64; 2];
const BLOCK: usize = 8192;

// Only this process owns the unnamed file. After sealing there are no writable
// mappings or file handles. Older pages can be reclaimed by the OS.
struct Block {
    storage: Storage,
    overview: Arc<[Point]>,
}

enum Storage {
    Hot(Vec<Point>),
    Paged(memmap2::Mmap),
}
impl Block {
    fn page_out(&self) -> io::Result<Self> {
        let raw = self.points();
        let file = tempfile::tempfile()?;
        let bytes = bytemuck::cast_slice(raw);
        file.set_len(bytes.len() as u64)?;
        // SAFETY: the private tempfile cannot be truncated or modified externally.
        let mut map = unsafe { memmap2::MmapOptions::new().map_mut(&file)? };
        map.copy_from_slice(bytes);

        Ok(Self {
            storage: Storage::Paged(map.make_read_only()?),
            overview: self.overview.clone(),
        })
    }
    fn points(&self) -> &[Point] {
        match &self.storage {
            Storage::Hot(raw) => raw,
            Storage::Paged(map) => bytemuck::cast_slice(map),
        }
    }
}

fn envelope(points: &[Point]) -> Vec<Point> {
    let mut min = 0;
    let mut max = 0;
    for (i, p) in points.iter().enumerate() {
        if p[1] < points[min][1] {
            min = i;
        }
        if p[1] > points[max][1] {
            max = i;
        }
    }
    let mut indices = vec![0, min, max, points.len() - 1];
    indices.sort_unstable();
    indices.dedup();
    indices.into_iter().map(|i| points[i]).collect()
}

#[derive(Clone, Default)]
pub(super) struct Points {
    blocks: Vec<Arc<Block>>,
    paged: usize,
    tail: Vec<Point>,
}

impl Points {
    pub fn len(&self) -> usize {
        self.blocks.len() * BLOCK + self.tail.len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn push(&mut self, point: Point) {
        self.tail.push(point);
    }
    pub fn seal(&mut self) {
        let count = self.tail.len() / BLOCK;
        if count == 0 {
            return;
        }
        for raw in self.tail.chunks_exact(BLOCK) {
            self.blocks.push(Arc::new(Block {
                storage: Storage::Hot(raw.to_vec()),
                overview: raw.chunks(64).flat_map(envelope).collect(),
            }));
        }
        self.tail.drain(..count * BLOCK);
    }
    // Called by the checkpoint worker. The UI keeps its immutable copy until
    // both paging and the database transaction have succeeded.
    pub fn page_out(&mut self) -> io::Result<()> {
        for block in &mut self.blocks[self.paged..] {
            *block = Arc::new(block.page_out()?);
        }
        self.paged = self.blocks.len();
        Ok(())
    }
    pub fn adopt_pages(&mut self, snapshot: Self) {
        let end = snapshot.paged;
        if self.paged < end && end <= self.blocks.len() {
            self.blocks[self.paged..end].clone_from_slice(&snapshot.blocks[self.paged..end]);
            self.paged = end;
        }
    }
    pub fn get(&self, i: usize) -> Option<&Point> {
        if let Some(block) = self.blocks.get(i / BLOCK) {
            Some(&block.points()[i % BLOCK])
        } else {
            self.tail.get(i - self.blocks.len() * BLOCK)
        }
    }
    pub fn first(&self) -> Option<&Point> {
        self.get(0)
    }
    pub fn last(&self) -> Option<&Point> {
        self.len().checked_sub(1).and_then(|i| self.get(i))
    }
    pub fn iter(&self) -> impl Iterator<Item = &Point> {
        self.blocks
            .iter()
            .flat_map(|b| b.points())
            .chain(&self.tail)
    }
    pub fn visible(&self, x0: f64, x1: f64, cols: usize) -> Vec<Point> {
        let mut points = Vec::new();
        let first = self
            .blocks
            .partition_point(|b| b.overview.last().unwrap()[0] < x0)
            .saturating_sub(1);
        for b in &self.blocks[first..] {
            let first_x = b.overview[0][0];
            let last_x = b.overview.last().unwrap()[0];
            let coarse = (x1 - x0) / cols.max(1) as f64 >= (last_x - first_x) * 64. / BLOCK as f64;
            let slice = if coarse { &b.overview } else { b.points() };
            append_visible(&mut points, slice, x0, x1);
            if first_x > x1 {
                break;
            }
        }
        append_visible(&mut points, &self.tail, x0, x1);
        super::downsample_minmax(&points, x0, x1, cols)
    }
}
fn append_visible(out: &mut Vec<Point>, raw: &[Point], lo: f64, hi: f64) {
    let a = raw.partition_point(|p| p[0] < lo).saturating_sub(1);
    let b = (raw.partition_point(|p| p[0] <= hi) + 1).min(raw.len());
    out.extend_from_slice(&raw[a..b]);
}
impl From<Vec<Point>> for Points {
    fn from(tail: Vec<Point>) -> Self {
        Self {
            blocks: vec![],
            paged: 0,
            tail,
        }
    }
}
impl FromIterator<Point> for Points {
    fn from_iter<I: IntoIterator<Item = Point>>(i: I) -> Self {
        Vec::from_iter(i).into()
    }
}
impl Index<usize> for Points {
    type Output = Point;
    fn index(&self, i: usize) -> &Point {
        self.get(i).expect("point index")
    }
}
#[cfg(test)]
impl std::ops::IndexMut<usize> for Points {
    fn index_mut(&mut self, i: usize) -> &mut Point {
        assert!(self.blocks.is_empty());
        &mut self.tail[i]
    }
}
#[cfg(test)]
impl Extend<Point> for Points {
    fn extend<I: IntoIterator<Item = Point>>(&mut self, i: I) {
        self.tail.extend(i)
    }
}
#[cfg(test)]
impl PartialEq<Vec<Point>> for Points {
    fn eq(&self, v: &Vec<Point>) -> bool {
        self.iter().eq(v)
    }
}
#[cfg(test)]
impl std::fmt::Debug for Points {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Points").field("len", &self.len()).finish()
    }
}
pub(super) fn checkpoint(
    path: &std::path::Path,
    header: &super::data_extract_cache::WidaHeader,
    points: &[Points],
) -> Result<(), Box<dyn std::error::Error>> {
    use rusqlite::{params, OptionalExtension};
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut db = rusqlite::Connection::open(path)?;
    let tx = db.transaction()?;
    tx.execute_batch("CREATE TABLE IF NOT EXISTS metadata(id INTEGER PRIMARY KEY, header TEXT NOT NULL); CREATE TABLE IF NOT EXISTS samples(curve INTEGER, start INTEGER, data BLOB NOT NULL, PRIMARY KEY(curve,start));")?;
    let previous: Option<String> = tx
        .query_row("SELECT header FROM metadata WHERE id=1", [], |row| {
            row.get(0)
        })
        .optional()?;
    let previous = previous
        .as_deref()
        .map(serde_json::from_str::<super::data_extract_cache::WidaHeader>)
        .transpose()?;
    for (curve, series) in points.iter().enumerate() {
        let start = previous
            .as_ref()
            .and_then(|h| h.series.get(curve))
            .map_or(0, |s| s.n as usize);
        for lo in (start..series.len()).step_by(BLOCK) {
            let hi = (lo + BLOCK).min(series.len());
            let mut bytes = Vec::with_capacity((hi - lo) * 16);
            for i in lo..hi {
                for v in series[i] {
                    bytes.extend_from_slice(&v.to_le_bytes());
                }
            }
            tx.execute(
                "INSERT INTO samples VALUES (?1,?2,?3)",
                params![curve, lo, bytes],
            )?;
        }
    }
    tx.execute(
        "INSERT OR REPLACE INTO metadata VALUES (1,?1)",
        [serde_json::to_string(header)?],
    )?;
    tx.commit()?;
    Ok(())
}

pub(super) fn export_csv(
    path: &std::path::Path,
    curves: &[(String, String, Points)],
) -> io::Result<()> {
    use std::io::{BufWriter, Write};
    let mut tmp =
        tempfile::NamedTempFile::new_in(path.parent().unwrap_or(std::path::Path::new(".")))?;
    {
        let mut writer = BufWriter::new(tmp.as_file_mut());
        writer.write_all(b"\xef\xbb\xbfframe,series,unit,value\r\n")?;
        for (name, unit, points) in curves {
            let name = name.replace('"', "\"\"");
            let unit = unit.replace('"', "\"\"");
            for [frame, value] in points.iter() {
                writeln!(writer, "{frame},\"{name}\",\"{unit}\",{value}\r")?;
            }
        }
        writer.flush()?;
    }
    tmp.as_file().sync_all()?;
    tmp.persist(path)?;
    Ok(())
}
