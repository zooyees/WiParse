use std::{io, ops::Index, sync::Arc};

type Point = [f64; 2];
const BLOCK: usize = 8192;

// Only this process owns the unnamed file. After sealing there are no writable
// mappings or file handles. Older pages can be reclaimed by the OS.
struct Block {
    identity: Arc<()>,
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
            identity: self.identity.clone(),
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

#[derive(Default)]
pub(super) struct Points {
    // Stable while this instance only appends or pages immutable blocks.
    // Every independent clone gets its own identity to prevent divergent
    // append histories from being mistaken for the same captured prefix.
    identity: Arc<()>,
    // Imported points are an immutable prefix, independent of the bounded
    // live tail. Appending never copies the imported vector.
    prefix: Option<Arc<Vec<Point>>>,
    blocks: Vec<Arc<Block>>,
    paged: usize,
    // Clones freeze this prefix in O(1); only a later mutation copies it.
    // Live ingestion seals after each batch, keeping this tail below BLOCK.
    tail: Arc<Vec<Point>>,
}

impl Clone for Points {
    fn clone(&self) -> Self {
        Self { identity: Arc::new(()), prefix:self.prefix.clone(), blocks:self.blocks.clone(), paged:self.paged, tail:self.tail.clone() }
    }
}

impl Points {
    pub fn identity(&self) -> Arc<()> { self.identity.clone() }
    pub fn has_identity(&self, identity: &Arc<()>) -> bool { Arc::ptr_eq(&self.identity,identity) }
    pub fn len(&self) -> usize {
        self.prefix.as_ref().map_or(0,|prefix|prefix.len()) + self.blocks.len() * BLOCK + self.tail.len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn push(&mut self, point: Point) {
        Arc::make_mut(&mut self.tail).push(point);
    }
    pub fn seal(&mut self) {
        let count = self.tail.len() / BLOCK;
        if count == 0 {
            return;
        }
        for raw in self.tail.chunks_exact(BLOCK) {
            self.blocks.push(Arc::new(Block {
                identity: Arc::new(()),
                storage: Storage::Hot(raw.to_vec()),
                overview: raw.chunks(64).flat_map(envelope).collect(),
            }));
        }
        // Keep a frozen tail alive without cloning it just to discard the sealed prefix.
        self.tail = Arc::new(self.tail[count * BLOCK..].to_vec());
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
    pub fn can_adopt_pages(&self, snapshot: &Self) -> bool {
        let end = snapshot.paged;
        let same_prefix=match (&self.prefix,&snapshot.prefix) {
            (None,None)=>true,(Some(a),Some(b))=>Arc::ptr_eq(a,b),_=>false,
        };
        let tail_start = snapshot.prefix.as_ref().map_or(0,|prefix|prefix.len()).saturating_add(snapshot.blocks.len().saturating_mul(BLOCK));
        same_prefix && end <= self.blocks.len() && end == snapshot.blocks.len() && snapshot.len() <= self.len()
            && self.blocks[..end].iter().zip(&snapshot.blocks[..end])
                .all(|(current, saved)| Arc::ptr_eq(&current.identity, &saved.identity))
            && snapshot.tail.iter().enumerate().all(|(index, saved)| {
                self.get(tail_start + index).is_some_and(|current|
                    current[0].to_bits() == saved[0].to_bits() && current[1].to_bits() == saved[1].to_bits())
            })
    }
    pub fn adopt_pages(&mut self, snapshot: Self) -> bool {
        if !self.can_adopt_pages(&snapshot) { return false; }
        let end = snapshot.paged;
        if self.paged < end && end <= self.blocks.len() {
            self.blocks[self.paged..end].clone_from_slice(&snapshot.blocks[self.paged..end]);
            self.paged = end;
        }
        true
    }
    pub fn get(&self, i: usize) -> Option<&Point> {
        let prefix_len=self.prefix.as_ref().map_or(0,|prefix|prefix.len());
        if i<prefix_len {return self.prefix.as_ref().and_then(|prefix|prefix.get(i));}
        let i=i-prefix_len;
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
        self.prefix.iter().flat_map(|prefix|prefix.iter())
            .chain(self.blocks.iter().flat_map(|b| b.points()))
            .chain(self.tail.iter())
    }
    pub fn visible(&self, x0: f64, x1: f64, cols: usize) -> Vec<Point> {
        let mut points = Vec::new();
        if let Some(prefix)=&self.prefix {append_visible(&mut points,prefix,x0,x1);}
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

#[cfg(test)]
mod identity_tests {
    use super::*;
    fn full_block(value: f64) -> Points {
        let mut points = Points::default();
        for i in 0..BLOCK { points.push([i as f64, value]); }
        points.seal();
        points
    }
    #[test]
    fn large_unsealed_clone_shares_storage_and_preserves_fixed_cutoff() {
        let mut points:Points=(0..1_000_000).map(|i|[i as f64,i as f64]).collect();
        let snapshot=points.clone();assert!(Arc::ptr_eq(points.prefix.as_ref().unwrap(),snapshot.prefix.as_ref().unwrap()));
        let worker=std::thread::spawn(move||{
            assert_eq!(snapshot.len(),1_000_000);assert_eq!(snapshot[999_999],[999_999.,999_999.]);snapshot
        });
        // Appending uses a separate bounded tail; the large prefix remains shared.
        points.push([1_000_000.,1.]);let snapshot=worker.join().unwrap();
        assert!(Arc::ptr_eq(points.prefix.as_ref().unwrap(),snapshot.prefix.as_ref().unwrap()));
        assert_eq!(snapshot.len(),1_000_000);assert_eq!(points.len(),1_000_001);
    }
    #[test]
    fn imported_prefix_and_live_blocks_remain_ordered_across_snapshot_and_checkpoint() {
        let mut points:Points=vec![[0.,3.],[1.,4.],[2.,5.]].into();
        for i in 0..BLOCK {points.push([(i+3) as f64,6.]);}
        points.seal();points.push([(BLOCK+3) as f64,7.]);
        let mut snapshot=points.clone();snapshot.paged=snapshot.blocks.len();
        points.push([(BLOCK+4) as f64,8.]);assert!(points.adopt_pages(snapshot.clone()));
        assert_eq!(snapshot.len(),BLOCK+4);assert_eq!(points.len(),BLOCK+5);
        assert_eq!(snapshot[2],[2.,5.]);assert_eq!(snapshot[3],[3.,6.]);
        assert_eq!(snapshot[BLOCK+2],[(BLOCK+2) as f64,6.]);
        assert_eq!(snapshot[BLOCK+3],[(BLOCK+3) as f64,7.]);
        assert_eq!(snapshot.iter().count(),snapshot.len());
        let visible=snapshot.visible(1.,4.,100);assert!(visible.iter().any(|p|p[0]==2.));assert!(visible.iter().any(|p|p[0]==3.));
    }
    #[test]
    fn shared_live_tail_stays_stable_across_append_and_seal() {
        let mut points=Points::default();
        for i in 0..BLOCK-1 {points.push([i as f64,1.]);}
        let snapshot=points.clone();assert!(Arc::ptr_eq(&points.tail,&snapshot.tail));
        points.push([(BLOCK-1) as f64,2.]);points.seal();
        assert_eq!(snapshot.len(),BLOCK-1);assert_eq!(snapshot.last().unwrap()[1],1.);
        assert_eq!(points.len(),BLOCK);assert_eq!(points.last().unwrap()[1],2.);
        assert_eq!(snapshot.iter().count(),BLOCK-1);assert_eq!(points.iter().count(),BLOCK);
    }
    #[test]
    fn equal_sized_replacement_is_not_the_original_block() {
        let mut current = full_block(2.0);
        let mut stale = full_block(1.0);
        stale.paged = 1;
        assert!(!current.adopt_pages(stale));
        assert_eq!(current[0], [0.0, 2.0]);
    }
    #[test]
    fn original_block_is_accepted_after_new_tail_is_appended() {
        let mut current = full_block(1.0);
        let mut snapshot = current.clone();
        snapshot.paged = 1;
        current.push([BLOCK as f64, 3.0]);
        assert!(current.adopt_pages(snapshot));
        assert_eq!(current.len(), BLOCK + 1);
        assert_eq!(current[BLOCK], [BLOCK as f64, 3.0]);
    }
    #[test]
    fn changed_unsealed_tail_is_rejected_even_at_same_length() {
        let mut current = Points::default();
        current.push([1.0, 2.0]);
        let snapshot = current.clone();
        Arc::make_mut(&mut current.tail)[0] = [1.0, 9.0];
        assert!(!current.adopt_pages(snapshot));
        assert_eq!(current[0], [1.0, 9.0]);
    }
    #[test]
    fn original_tail_remains_valid_after_becoming_a_sealed_block() {
        let mut current = Points::default();
        current.push([0.0, 1.0]);
        let snapshot = current.clone();
        for i in 1..BLOCK { current.push([i as f64, 1.0]); }
        current.seal();
        assert!(current.adopt_pages(snapshot));
    }
}
impl From<Vec<Point>> for Points {
    fn from(tail: Vec<Point>) -> Self {
        Self {
            identity: Arc::new(()),
            prefix: Some(Arc::new(tail)),
            blocks: vec![],
            paged: 0,
            tail: Arc::new(Vec::new()),
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
        self.identity = Arc::new(());
        let prefix_len=self.prefix.as_ref().map_or(0,|prefix|prefix.len());
        if i<prefix_len { &mut Arc::make_mut(self.prefix.as_mut().unwrap())[i] }
        else { &mut Arc::make_mut(&mut self.tail)[i-prefix_len] }
    }
}
#[cfg(test)]
impl Extend<Point> for Points {
    fn extend<I: IntoIterator<Item = Point>>(&mut self, i: I) {
        Arc::make_mut(&mut self.tail).extend(i)
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
