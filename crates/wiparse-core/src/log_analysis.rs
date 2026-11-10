//! Streaming, read-only log analysis. Raw files stay local; reports and work
//! state are bounded independently of input size and can be resumed by a host.
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    fs::{self, File},
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant, SystemTime},
};
const MAX_SOURCES: usize = 4096;
const MAX_LINE: usize = 64 * 1024;
pub const REPORT_BYTES: usize = 6000;
fn clip(s: &str, n: usize) -> String {
    let clean: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let mut end = clean.len().min(n);
    while !clean.is_char_boundary(end) {
        end -= 1;
    }
    clean[..end].into()
}
fn fingerprint(s: &str) -> String {
    format!("{:x}", Sha256::digest(s.as_bytes()))[..32].into()
}
fn cancelled(flag: &AtomicBool) -> Result<(), String> {
    if flag.load(Ordering::Acquire) {
        Err("LOG.ANALYSIS_CANCELLED".into())
    } else {
        Ok(())
    }
}

#[derive(Clone, Default)]
pub struct LibrarySpec {
    pub directory: Option<PathBuf>,
    pub files: Vec<PathBuf>,
}
#[derive(Clone, Serialize)]
pub struct Source {
    pub id: String,
    pub name: String,
    pub bytes: u64,
    #[serde(skip)]
    pub path: PathBuf,
    #[serde(skip)]
    modified: Option<SystemTime>,
    #[serde(skip)]
    memory: Option<std::sync::Arc<Vec<std::sync::Arc<str>>>>,
    pub first_row: u64,
    pub preceding_rows_unavailable: u64,
}
impl Source {
    /// Immutable shared lines: no whole-log text allocation or filesystem staging.
    pub fn memory(
        name: &str,
        lines: Vec<std::sync::Arc<str>>,
        first_row: u64,
        missing: u64,
    ) -> Self {
        let bytes = lines
            .iter()
            .fold(0u64, |sum, line| sum.saturating_add(line.len() as u64 + 1));
        Self {
            id: fingerprint(&format!("memory:{name}")),
            name: clip(name, 160),
            bytes,
            path: PathBuf::new(),
            modified: None,
            memory: Some(std::sync::Arc::new(lines)),
            first_row: first_row.max(1),
            preceding_rows_unavailable: missing,
        }
    }
}
#[derive(Default)]
pub struct Inventory {
    pub sources: Vec<Source>,
    pub incomplete: bool,
    pub issues: Vec<String>,
}
fn log_extension(path: &Path) -> bool {
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    let extension = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    matches!(extension.as_str(), "log" | "txt" | "jsonl" | "csv")
        || ((name.contains(".log.") || name.contains(".txt."))
            && !matches!(extension.as_str(), "gz" | "zip" | "7z" | "bak"))
}
fn issue(inventory: &mut Inventory, message: String) {
    inventory.incomplete = true;
    if inventory.issues.len() < 6 {
        inventory.issues.push(clip(&message, 160));
    }
}
fn add_source(
    inventory: &mut Inventory,
    seen: &mut HashSet<String>,
    path: &Path,
    root: Option<&Path>,
) {
    if inventory.sources.len() >= MAX_SOURCES {
        issue(
            inventory,
            "文件数量超过4096，清单不完整，请缩小目录范围".into(),
        );
        return;
    }
    let canonical = match fs::canonicalize(path) {
        Ok(p) => p,
        Err(e) => {
            issue(
                inventory,
                format!(
                    "无法访问日志 {}: {e}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ),
            );
            return;
        }
    };
    if root.is_some_and(|root| !canonical.starts_with(root)) {
        issue(inventory, "跳过目录范围外的链接".into());
        return;
    }
    let identity = canonical.to_string_lossy();
    let identity = if cfg!(windows) {
        identity.to_lowercase()
    } else {
        identity.into_owned()
    };
    if !seen.insert(identity.clone()) {
        return;
    }
    let metadata = match fs::metadata(&canonical) {
        Ok(m) if m.is_file() => m,
        _ => {
            issue(inventory, "日志来源不是普通文件".into());
            return;
        }
    };
    let name = root
        .and_then(|root| canonical.strip_prefix(root).ok())
        .unwrap_or_else(|| {
            canonical
                .file_name()
                .map(Path::new)
                .unwrap_or(Path::new("log"))
        })
        .to_string_lossy()
        .replace('\\', "/");
    inventory.sources.push(Source {
        id: fingerprint(&identity),
        name: clip(&name, 160),
        bytes: metadata.len(),
        modified: metadata.modified().ok(),
        path: canonical,
        memory: None,
        first_row: 1,
        preceding_rows_unavailable: 0,
    });
}
pub fn discover(spec: &LibrarySpec, cancel: &AtomicBool) -> Result<Inventory, String> {
    let mut inventory = Inventory::default();
    let mut seen = HashSet::new();
    for file in &spec.files {
        cancelled(cancel)?;
        add_source(&mut inventory, &mut seen, file, None);
    }
    if let Some(directory) = &spec.directory {
        let root = fs::canonicalize(directory).map_err(|_| "无法访问已选择的日志目录")?;
        if !root.is_dir() {
            return Err("日志目录已变化，请重新选择".into());
        }
        let mut pending = VecDeque::from([root.clone()]);
        let mut directories = 0;
        let mut visited = HashSet::new();
        while let Some(directory) = pending.pop_front() {
            cancelled(cancel)?;
            let canonical = match fs::canonicalize(&directory) {
                Ok(path) if path.starts_with(&root) => path,
                _ => {
                    issue(&mut inventory, "跳过授权目录以外的目录链接".into());
                    continue;
                }
            };
            if !visited.insert(canonical) {
                continue;
            }
            directories += 1;
            if directories > MAX_SOURCES {
                issue(&mut inventory, "子目录数量超过4096，清单不完整".into());
                break;
            }
            let entries = match fs::read_dir(&directory) {
                Ok(e) => e,
                Err(_) => {
                    issue(&mut inventory, "部分子目录不可读".into());
                    continue;
                }
            };
            for entry in entries {
                cancelled(cancel)?;
                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => {
                        issue(&mut inventory, "部分目录条目不可读".into());
                        continue;
                    }
                };
                let kind = match entry.file_type() {
                    Ok(t) => t,
                    Err(_) => {
                        issue(&mut inventory, "部分文件类型不可读".into());
                        continue;
                    }
                };
                if kind.is_symlink() {
                    issue(&mut inventory, "跳过符号链接，未跨越授权目录".into());
                    continue;
                }
                if kind.is_dir() {
                    if pending.len() + directories < MAX_SOURCES {
                        pending.push_back(entry.path());
                    } else {
                        issue(&mut inventory, "目录扫描上限已达，清单不完整".into());
                    }
                } else if kind.is_file() && log_extension(&entry.path()) {
                    add_source(&mut inventory, &mut seen, &entry.path(), Some(&root));
                    if inventory.sources.len() >= MAX_SOURCES {
                        inventory.incomplete = true;
                        break;
                    }
                }
            }
            if inventory.sources.len() >= MAX_SOURCES {
                issue(
                    &mut inventory,
                    "文件清单达到4096项上限，可能有未列出的日志".into(),
                );
                break;
            }
        }
    }
    inventory
        .sources
        .sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    Ok(inventory)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NumericRule {
    pub name: String,
    pub pattern: String,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Query {
    pub source_ids: Vec<String>,
    pub contains: Vec<String>,
    pub exclude: Vec<String>,
    pub pattern: Option<String>,
    pub case_sensitive: bool,
    pub require_all: bool,
    pub first_line: u64,
    pub last_line: Option<u64>,
    pub context: usize,
    pub samples: usize,
    pub group: bool,
    pub numeric: Vec<NumericRule>,
}
impl Default for Query {
    fn default() -> Self {
        Self {
            source_ids: vec![],
            contains: vec![],
            exclude: vec![],
            pattern: None,
            case_sensitive: false,
            require_all: false,
            first_line: 1,
            last_line: None,
            context: 1,
            samples: 6,
            group: true,
            numeric: vec![],
        }
    }
}
fn compile(pattern: &str, sensitive: bool) -> Result<Regex, String> {
    if pattern.is_empty() || pattern.len() > 256 || pattern.chars().any(char::is_control) {
        return Err("正则表达式须为1–256字节".into());
    }
    RegexBuilder::new(pattern)
        .case_insensitive(!sensitive)
        .size_limit(1024 * 1024)
        .dfa_size_limit(1024 * 1024)
        .build()
        .map_err(|e| format!("无效或过大的正则表达式：{e}"))
}
impl Query {
    pub fn validate(&self) -> Result<(), String> {
        if self.source_ids.len() > 32
            || self
                .source_ids
                .iter()
                .any(|s| s.len() != 32 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err("日志来源必须使用清单中的ID（最多32个；空列表表示全部）".into());
        }
        if self.contains.len() > 8
            || self.exclude.len() > 8
            || self
                .contains
                .iter()
                .chain(&self.exclude)
                .any(|s| s.is_empty() || s.len() > 64 || s.chars().any(char::is_control))
        {
            return Err("包含/排除关键词各最多8个，每项1–64字节".into());
        }
        if self.first_line == 0
            || self.last_line.is_some_and(|n| n < self.first_line)
            || self.context > 2
            || self.samples > 8
            || self.numeric.len() > 4
        {
            return Err("日志行范围、上下文或统计项超出上限".into());
        }
        if let Some(pattern) = &self.pattern {
            compile(pattern, self.case_sensitive)?;
        }
        let mut names = HashSet::new();
        for rule in &self.numeric {
            if rule.name.is_empty()
                || rule.name.len() > 32
                || rule.name.chars().any(char::is_control)
                || !names.insert(&rule.name)
                || rule.min.is_some_and(|n| !n.is_finite())
                || rule.max.is_some_and(|n| !n.is_finite())
                || matches!((rule.min,rule.max),(Some(a),Some(b)) if a>b)
            {
                return Err("数值字段名称/限值无效或重复".into());
            }
            if compile(&rule.pattern, self.case_sensitive)?.captures_len() < 2 {
                return Err("数值正则须提供第一个捕获组作为数值".into());
            }
        }
        Ok(())
    }
}
#[derive(Default, Serialize)]
struct Numbers {
    count: u64,
    invalid: u64,
    min: Option<f64>,
    max: Option<f64>,
    mean: Option<f64>,
    below_min: u64,
    above_max: u64,
}
impl Numbers {
    fn push(&mut self, text: &str, rule: &NumericRule) {
        let value = match text.parse::<f64>() {
            Ok(v) if v.is_finite() => v,
            _ => {
                self.invalid += 1;
                return;
            }
        };
        self.count += 1;
        self.min = Some(self.min.map_or(value, |v| v.min(value)));
        self.max = Some(self.max.map_or(value, |v| v.max(value)));
        self.mean = Some(self.mean.map_or(value, |mean| {
            mean * (1. - 1. / self.count as f64) + value / self.count as f64
        }));
        if rule.min.is_some_and(|v| value < v) {
            self.below_min += 1;
        }
        if rule.max.is_some_and(|v| value > v) {
            self.above_max += 1;
        }
    }
}
#[derive(Clone, Serialize)]
struct Example {
    source_id: String,
    file: String,
    line: u64,
    text: String,
    before: Vec<(u64, String)>,
    after: Vec<(u64, String)>,
}
#[derive(Serialize)]
struct Group {
    count: u64,
    first_file: String,
    first_source_id: String,
    first_line: u64,
    last_file: String,
    last_source_id: String,
    last_line: u64,
}

pub struct Scan {
    sources: Vec<Source>,
    query: Query,
    pattern: Option<Regex>,
    numeric: Vec<Regex>,
    numbers: Vec<Numbers>,
    keywords: Vec<u64>,
    source: usize,
    offset: u64,
    row: u64,
    pending: Vec<u8>,
    oversize: bool,
    before: VecDeque<(u64, String)>,
    bytes: u64,
    lines: u64,
    in_range: u64,
    matched: u64,
    oversized: u64,
    invalid_utf8: u64,
    nul_lines: u64,
    partial_tails: u64,
    growing_sources: HashSet<String>,
    diagnostics: HashSet<(usize, u8)>,
    failed: usize,
    errors: Vec<String>,
    inventory_incomplete: bool,
    groups: BTreeMap<String, Group>,
    groups_omitted: u64,
    examples: Vec<Example>,
    normal_time: Regex,
    normal_number: Regex,
}
impl Scan {
    pub fn new(inventory: Inventory, query: Query) -> Result<Self, String> {
        query.validate()?;
        let ids: HashSet<_> = query.source_ids.iter().collect();
        if ids
            .iter()
            .any(|id| !inventory.sources.iter().any(|source| &source.id == *id))
        {
            return Err("日志来源ID已变化，请重新获取清单".into());
        }
        let sources: Vec<_> = inventory
            .sources
            .into_iter()
            .filter(|s| ids.is_empty() || ids.contains(&s.id))
            .collect();
        if sources.is_empty() {
            return Err("没有可分析的日志，请打开文件或选择日志目录".into());
        }
        let pattern = query
            .pattern
            .as_ref()
            .map(|p| compile(p, query.case_sensitive))
            .transpose()?;
        let numeric = query
            .numeric
            .iter()
            .map(|r| compile(&r.pattern, query.case_sensitive))
            .collect::<Result<_, _>>()?;
        Ok(Self{sources,pattern,numeric,numbers:(0..query.numeric.len()).map(|_|Numbers::default()).collect(),keywords:vec![0;query.contains.len()],query,
   source:0,offset:0,row:1,pending:vec![],oversize:false,before:VecDeque::new(),bytes:0,lines:0,in_range:0,matched:0,oversized:0,invalid_utf8:0,nul_lines:0,partial_tails:0,growing_sources:HashSet::new(),diagnostics:HashSet::new(),failed:0,errors:inventory.issues,inventory_incomplete:inventory.incomplete,groups:BTreeMap::new(),groups_omitted:0,examples:vec![],
   normal_time:Regex::new(r"^(?:\[\d{2}:\d{2}:\d{2}(?:\.\d+)?\]\s*|\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(?:\.\d+)?\s*)").unwrap(),
   normal_number:Regex::new(r"\b(?:0[xX][0-9a-fA-F]+|\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)\b").unwrap()})
    }
    fn fail(&mut self, error: &str) {
        self.failed += 1;
        if self.errors.len() < 6 {
            self.errors.push(clip(error, 160));
        }
        self.next_source();
    }
    fn growing(&mut self, source: &Source) {
        if self.growing_sources.insert(source.id.clone()) && self.errors.len() < 6 {
            self.errors.push(clip(
                &format!(
                    "{} 正在增长：仅扫描初始字节范围，无法确认前缀一致性；不作为完整日志结论",
                    source.name
                ),
                160,
            ));
        }
    }
    fn next_source(&mut self) {
        self.source += 1;
        self.offset = 0;
        self.row = self
            .sources
            .get(self.source)
            .map_or(1, |source| source.first_row);
        self.pending.clear();
        self.oversize = false;
        self.before.clear();
    }
    pub fn finished(&self) -> bool {
        self.source >= self.sources.len()
    }
    /// The host retains this state and issues opaque continuation IDs. A chunk
    /// may stop in the middle of a line without dropping or recounting that line.
    pub fn run_chunk(
        &mut self,
        budget: u64,
        time: Duration,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        if budget == 0 || time.is_zero() {
            return Err("扫描预算必须大于零".into());
        }
        let started = Instant::now();
        let mut consumed = 0;
        while !self.finished() && consumed < budget && started.elapsed() < time {
            cancelled(cancel)?;
            let source = self.sources[self.source].clone();
            if let Some(lines) = &source.memory {
                if self.offset == 0 {
                    self.row = source.first_row;
                }
                while (self.offset as usize) < lines.len()
                    && consumed < budget
                    && started.elapsed() < time
                {
                    cancelled(cancel)?;
                    let line = &lines[self.offset as usize];
                    let length = line.len();
                    self.pending
                        .extend_from_slice(&line.as_bytes()[..length.min(MAX_LINE)]);
                    self.oversize = length > MAX_LINE;
                    self.offset += 1;
                    self.bytes = self.bytes.saturating_add(length as u64 + 1);
                    consumed = consumed.saturating_add(length as u64 + 1);
                    self.process_line();
                }
                if self.offset as usize == lines.len() {
                    self.next_source();
                }
                continue;
            }
            let mut file = match File::open(&source.path) {
                Ok(f) => f,
                Err(e) => {
                    self.fail(&format!("{} 无法读取：{e}", source.name));
                    continue;
                }
            };
            let metadata = file.metadata().map_err(|e| e.to_string())?;
            if metadata.len() < source.bytes
                || (metadata.len() == source.bytes && metadata.modified().ok() != source.modified)
            {
                self.fail(&format!(
                    "{} 在分析期间变化，请重新开始；已有统计仅为已读部分",
                    source.name
                ));
                continue;
            }
            if metadata.len() > source.bytes {
                self.growing(&source);
            }
            file.seek(SeekFrom::Start(self.offset))
                .map_err(|e| e.to_string())?;
            let mut reader = BufReader::with_capacity(
                64 * 1024,
                file.take(source.bytes.saturating_sub(self.offset)),
            );
            loop {
                cancelled(cancel)?;
                if consumed >= budget || started.elapsed() >= time {
                    return Ok(());
                }
                let buffer = reader.fill_buf().map_err(|e| e.to_string())?;
                if buffer.is_empty() {
                    let metadata = reader
                        .get_ref()
                        .get_ref()
                        .metadata()
                        .map_err(|e| e.to_string())?;
                    if metadata.len() < source.bytes
                        || (metadata.len() == source.bytes
                            && metadata.modified().ok() != source.modified)
                    {
                        self.fail(&format!(
                            "{} 读取期间被修改或截断，不能视为稳定快照",
                            source.name
                        ));
                    } else {
                        if metadata.len() > source.bytes {
                            self.growing(&source);
                        }
                        if !self.pending.is_empty() || self.oversize {
                            if self.growing_sources.contains(&source.id) {
                                self.partial_tails += 1;
                            } else {
                                self.process_line();
                            }
                        }
                        self.next_source();
                    }
                    break;
                }
                let available = buffer
                    .len()
                    .min((budget - consumed).min(usize::MAX as u64) as usize);
                let length = buffer[..available]
                    .iter()
                    .position(|b| *b == b'\n')
                    .map_or(available, |i| i + 1);
                let newline = buffer[length - 1] == b'\n';
                let content = &buffer[..length];
                let keep = (MAX_LINE.saturating_sub(self.pending.len())).min(content.len());
                self.pending.extend_from_slice(&content[..keep]);
                self.oversize |= keep < content.len();
                self.offset += length as u64;
                self.bytes += length as u64;
                consumed += length as u64;
                reader.consume(length);
                if newline {
                    self.process_line();
                }
            }
        }
        Ok(())
    }
    fn process_line(&mut self) {
        self.lines += 1;
        let row = self.row;
        self.row += 1;
        if self.oversize {
            self.oversized += 1;
            if self.diagnostics.insert((self.source, 1)) && self.errors.len() < 6 {
                self.errors.push(clip(
                    &format!(
                        "{} 第{}行超过64 KiB，跳过该行，统计不完整",
                        self.sources[self.source].name, row
                    ),
                    160,
                ));
            }
            self.pending.clear();
            self.oversize = false;
            self.before.clear();
            return;
        }
        while self
            .pending
            .last()
            .is_some_and(|b| *b == b'\n' || *b == b'\r')
        {
            self.pending.pop();
        }
        let nul = self.pending.contains(&0);
        let invalid = std::str::from_utf8(&self.pending).is_err();
        if nul {
            self.nul_lines += 1;
        }
        if invalid {
            self.invalid_utf8 += 1;
        }
        if (nul || invalid) && self.diagnostics.insert((self.source, 0)) && self.errors.len() < 6 {
            self.errors.push(clip(
                &format!(
                    "{} 第{}行含非UTF-8或NUL数据，请转换编码后重试；现有统计不完整",
                    self.sources[self.source].name, row
                ),
                160,
            ));
        }
        let text = String::from_utf8_lossy(&self.pending)
            .trim_start_matches('\u{feff}')
            .to_owned();
        self.pending.clear();
        let source = &self.sources[self.source];
        let in_scope =
            row >= self.query.first_line && self.query.last_line.is_none_or(|last| row <= last);
        for example in &mut self.examples {
            if in_scope
                && example.source_id == source.id
                && row > example.line
                && row - example.line <= self.query.context as u64
            {
                example.after.push((row, clip(&text, 160)));
            }
        }
        if row >= self.query.first_line && self.query.last_line.is_none_or(|last| row <= last) {
            self.in_range += 1;
            let folded = if self.query.case_sensitive {
                text.clone()
            } else {
                text.to_lowercase()
            };
            let hits: Vec<_> = self
                .query
                .contains
                .iter()
                .map(|key| {
                    folded.contains(&if self.query.case_sensitive {
                        key.clone()
                    } else {
                        key.to_lowercase()
                    })
                })
                .collect();
            for (i, hit) in hits.iter().enumerate() {
                if *hit {
                    self.keywords[i] += 1;
                }
            }
            let keywords = hits.is_empty()
                || if self.query.require_all {
                    hits.iter().all(|v| *v)
                } else {
                    hits.iter().any(|v| *v)
                };
            let excluded = self.query.exclude.iter().any(|key| {
                folded.contains(&if self.query.case_sensitive {
                    key.clone()
                } else {
                    key.to_lowercase()
                })
            });
            if keywords
                && !excluded
                && self
                    .pattern
                    .as_ref()
                    .is_none_or(|pattern| pattern.is_match(&text))
            {
                self.matched += 1;
                for (i, pattern) in self.numeric.iter().enumerate() {
                    for captures in pattern.captures_iter(&text) {
                        if let Some(value) = captures.get(1) {
                            self.numbers[i].push(value.as_str(), &self.query.numeric[i]);
                        }
                    }
                }
                if self.query.group {
                    let without_time = self.normal_time.replace(&text, "");
                    let key = clip(&self.normal_number.replace_all(&without_time, "<n>"), 240);
                    if let Some(group) = self.groups.get_mut(&key) {
                        group.count += 1;
                        group.last_file = source.name.clone();
                        group.last_source_id = source.id.clone();
                        group.last_line = row;
                    } else if self.groups.len() < 4096 {
                        self.groups.insert(
                            key,
                            Group {
                                count: 1,
                                first_file: source.name.clone(),
                                first_source_id: source.id.clone(),
                                first_line: row,
                                last_file: source.name.clone(),
                                last_source_id: source.id.clone(),
                                last_line: row,
                            },
                        );
                    } else {
                        self.groups_omitted += 1;
                    }
                }
                if self.query.samples > 0 {
                    if self.examples.len() >= self.query.samples {
                        self.examples.remove(self.query.samples / 2);
                    }
                    self.examples.push(Example {
                        source_id: source.id.clone(),
                        file: source.name.clone(),
                        line: row,
                        text: crate::engineering_agent::match_excerpt(&text,
                            self.pattern.as_ref().and_then(|p|p.find(&text)).map(|m|m.as_str())
                            .or_else(||self.numeric.iter().find_map(|p|p.find(&text).map(|m|m.as_str())))
                            .or_else(||self.query.contains.iter().zip(&hits).find(|(_,hit)|**hit).map(|(s,_)|s.as_str())).unwrap_or(""),240),
                        before: self.before.iter().cloned().collect(),
                        after: vec![],
                    });
                }
            }
        }
        if in_scope {
            self.before.push_back((row, clip(&text, 160)));
        }
        while self.before.len() > self.query.context {
            self.before.pop_front();
        }
    }
    pub fn report(&self) -> Value {
        let mut groups: Vec<_> = self.groups.iter().collect();
        groups.sort_by(|a, b| b.1.count.cmp(&a.1.count).then(a.0.cmp(b.0)));
        let mut value = json!({"coverage":{"finished":self.finished(),"complete":self.finished()&&!self.inventory_incomplete&&self.sources.iter().all(|s|s.preceding_rows_unavailable==0)&&self.failed==0&&self.oversized==0&&self.invalid_utf8==0&&self.nul_lines==0&&self.partial_tails==0&&self.growing_sources.is_empty(),"preceding_rows_unavailable":self.sources.iter().map(|s|s.preceding_rows_unavailable).sum::<u64>(),"source_windows":self.sources.iter().take(16).map(|s|json!({"id":s.id,"first_row":s.first_row,"memory_snapshot":s.memory.is_some(),"preceding_rows_unavailable":s.preceding_rows_unavailable})).collect::<Vec<_>>(),"files":self.sources.len(),"files_finished":self.source,"files_failed":self.failed,"inventory_incomplete":self.inventory_incomplete,"bytes_read":self.bytes,"snapshot_bytes":self.sources.iter().fold(0u64,|sum,source|sum.saturating_add(source.bytes)),"lines_read":self.lines,"lines_in_range":self.in_range,"matched_lines":self.matched,"oversized_lines_skipped":self.oversized,"lossy_utf8_lines":self.invalid_utf8,"nul_or_utf16_like_lines":self.nul_lines,"incomplete_tail_lines_skipped":self.partial_tails,"growing_files":self.growing_sources.len()},
   "scope":{"first_line":self.query.first_line,"last_line":self.query.last_line,"contains":self.query.contains,"exclude":self.query.exclude,"require_all":self.query.require_all,"case_sensitive":self.query.case_sensitive,"pattern":self.query.pattern},
   "keyword_line_counts_before_filter":self.query.contains.iter().zip(&self.keywords).map(|(key,count)|json!({"keyword":key,"lines":count})).collect::<Vec<_>>(),
   "numeric_matched_lines":self.query.numeric.iter().zip(&self.numbers).map(|(rule,numbers)|json!({"field":rule.name,"pattern":rule.pattern,"min_limit":rule.min,"max_limit":rule.max,"statistics":numbers})).collect::<Vec<_>>(),
   "grouping":"timestamp removed; numbers replaced; first 240 UTF-8 bytes; counts are exact only for tracked prefixes, not semantic equivalence",
   "grouping_complete":self.groups_omitted==0,"untracked_group_lines":self.groups_omitted,
   "top_tracked_groups":groups.into_iter().take(8).map(|(template,stats)|json!({"template":template,"statistics":stats})).collect::<Vec<_>>(),"examples":self.examples,"examples_are_bounded_first_and_latest":true,"errors":self.errors,"unsaved_edits_included":false,"raw_files_uploaded":false});
        let mut omitted = 0;
        value["report_items_omitted_for_size"] = json!(0);
        while value.to_string().len() > REPORT_BYTES {
            if value["examples"].as_array().is_some_and(|a| !a.is_empty()) {
                value["examples"].as_array_mut().unwrap().pop();
            } else if value["top_tracked_groups"]
                .as_array()
                .is_some_and(|a| !a.is_empty())
            {
                value["top_tracked_groups"].as_array_mut().unwrap().pop();
            } else {
                value["scope"] = json!({"query_fingerprint":fingerprint(&serde_json::to_string(&self.query).unwrap_or_default()),"full_filter_details_omitted_for_size":true});
                break;
            }
            omitted += 1;
            value["report_items_omitted_for_size"] = json!(omitted);
        }
        value["report_items_omitted_for_size"] = json!(omitted);
        if value.to_string().len() > REPORT_BYTES {
            if let Some(fields) = value["numeric_matched_lines"].as_array_mut() {
                for field in fields {
                    field.as_object_mut().unwrap().remove("pattern");
                }
            }
            value["numeric_patterns_omitted_for_size"] = json!(true);
        }
        if value.to_string().len() > REPORT_BYTES {
            let count = value["errors"].as_array().map_or(0, Vec::len);
            if let Some(errors) = value["errors"].as_array_mut() {
                errors.truncate(1);
            }
            value["error_messages_omitted_for_size"] = json!(count.saturating_sub(1));
        }
        while value.to_string().len() > REPORT_BYTES
            && value["coverage"]["source_windows"]
                .as_array()
                .is_some_and(|a| !a.is_empty())
        {
            value["coverage"]["source_windows"]
                .as_array_mut()
                .unwrap()
                .pop();
            value["source_windows_omitted_for_size"] = json!(true);
        }
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("wiparse-text-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&root).unwrap();
            Self(root)
        }
        fn inventory(&self) -> Inventory {
            discover(
                &LibrarySpec {
                    directory: Some(self.0.clone()),
                    files: vec![],
                },
                &AtomicBool::new(false),
            )
            .unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn finish(scan: &mut Scan, budget: u64) {
        for _ in 0..10000 {
            scan.run_chunk(budget, Duration::from_secs(2), &AtomicBool::new(false))
                .unwrap();
            if scan.finished() {
                return;
            }
        }
        panic!("scan did not complete");
    }
    #[test]
    fn full_directory_filters_numeric_thresholds_and_exact_source_rows() {
        let fixture = Fixture::new();
        fs::create_dir(fixture.0.join("day")).unwrap();
        fs::write(fixture.0.join("day/a.log"),"[12:01:00.000] INFO VIN=5.0\n[12:01:00.001] ERROR VIN=4.0\n[12:01:00.002] ERROR ignore VIN=9.0\n[12:01:00.003] WARN VIN=6.0\n").unwrap();
        fs::write(fixture.0.join("b.log.1"), "前文\nERROR VIN=3.0\n后文\n").unwrap();
        fs::write(fixture.0.join("ignored.exe"), "ERROR VIN=99").unwrap();
        let mut query = Query::default();
        query.contains = vec!["error".into(), "warn".into()];
        query.exclude = vec!["ignore".into()];
        query.numeric = vec![NumericRule {
            name: "VIN".into(),
            pattern: r"VIN=([0-9.]+)".into(),
            min: Some(4.5),
            max: Some(5.5),
        }];
        let mut scan = Scan::new(fixture.inventory(), query).unwrap();
        finish(&mut scan, 7);
        let report = scan.report();
        assert_eq!(report["coverage"]["files"], 2);
        assert_eq!(report["coverage"]["lines_read"], 7);
        assert_eq!(report["coverage"]["matched_lines"], 3);
        assert_eq!(report["coverage"]["complete"], true);
        let values = &report["numeric_matched_lines"][0]["statistics"];
        assert_eq!(values["count"], 3);
        assert_eq!(values["min"], 3.);
        assert_eq!(values["max"], 6.);
        assert!((values["mean"].as_f64().unwrap() - 13. / 3.).abs() < 1e-12);
        assert_eq!(values["below_min"], 2);
        assert_eq!(values["above_max"], 1);
        assert_eq!(report["keyword_line_counts_before_filter"][0]["lines"], 3);
        let evidence = report["examples"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["file"] == "b.log.1")
            .unwrap();
        assert_eq!(evidence["line"], 2);
        assert_eq!(evidence["before"][0][1], "前文");
        assert_eq!(evidence["after"][0][1], "后文");
    }
    #[test]
    fn byte_chunks_resume_in_middle_of_utf8_and_do_not_recount_lines() {
        let fixture = Fixture::new();
        let text = "时间 1\r\n时间 2\r\n最后一行";
        fs::write(fixture.0.join("test.txt"), text).unwrap();
        let mut scan = Scan::new(fixture.inventory(), Query::default()).unwrap();
        finish(&mut scan, 1);
        let report = scan.report();
        assert_eq!(report["coverage"]["lines_read"], 3);
        assert_eq!(report["coverage"]["bytes_read"], text.len());
        assert_eq!(report["coverage"]["lossy_utf8_lines"], 0);
        assert_eq!(report["coverage"]["complete"], true);
        assert_eq!(report["top_tracked_groups"][0]["statistics"]["count"], 2);
    }
    #[test]
    fn regex_and_ranges_and_selected_ids_are_respected() {
        let fixture = Fixture::new();
        fs::write(fixture.0.join("a.log"), "keep A=1\nKEEP A=2\nkeep A=3\n").unwrap();
        fs::write(fixture.0.join("b.log"), "keep A=4\n").unwrap();
        let inventory = fixture.inventory();
        let id = inventory
            .sources
            .iter()
            .find(|s| s.name == "a.log")
            .unwrap()
            .id
            .clone();
        let mut query = Query::default();
        query.source_ids = vec![id];
        query.first_line = 2;
        query.last_line = Some(2);
        query.pattern = Some(r"KEEP A=\d+".into());
        query.case_sensitive = true;
        let mut scan = Scan::new(inventory, query).unwrap();
        finish(&mut scan, 4096);
        let report = scan.report();
        assert_eq!(report["coverage"]["files"], 1);
        assert_eq!(report["coverage"]["lines_in_range"], 1);
        assert_eq!(report["coverage"]["matched_lines"], 1);
        assert_eq!(report["examples"][0]["line"], 2);
    }
    #[test]
    fn changed_sources_and_cancellation_are_explicit() {
        let fixture = Fixture::new();
        let path = fixture.0.join("a.log");
        fs::write(&path, "first\nsecond\n").unwrap();
        let mut scan = Scan::new(fixture.inventory(), Query::default()).unwrap();
        scan.run_chunk(2, Duration::from_secs(2), &AtomicBool::new(false))
            .unwrap();
        assert!(!scan.finished());
        fs::write(&path, "short\n").unwrap();
        finish(&mut scan, 100);
        let report = scan.report();
        assert_eq!(report["coverage"]["complete"], false);
        assert_eq!(report["coverage"]["files_failed"], 1);
        let mut scan = Scan::new(fixture.inventory(), Query::default()).unwrap();
        assert!(scan
            .run_chunk(100, Duration::from_secs(2), &AtomicBool::new(true))
            .unwrap_err()
            .contains("CANCELLED"));
        assert_eq!(scan.report()["coverage"]["bytes_read"], 0);
    }
    #[test]
    fn oversized_and_invalid_utf8_never_claim_complete() {
        let fixture = Fixture::new();
        let mut bytes = vec![b'x'; MAX_LINE + 100];
        bytes.extend_from_slice(b"\n\xff\n");
        fs::write(fixture.0.join("a.log"), bytes).unwrap();
        let mut scan = Scan::new(fixture.inventory(), Query::default()).unwrap();
        finish(&mut scan, 8192);
        let report = scan.report();
        assert_eq!(report["coverage"]["oversized_lines_skipped"], 1);
        assert_eq!(report["coverage"]["lossy_utf8_lines"], 1);
        assert_eq!(report["coverage"]["complete"], false);
        assert!(report.to_string().len() <= REPORT_BYTES);
    }
    #[test]
    fn duplicate_sources_are_not_counted_twice_and_response_is_bounded() {
        let fixture = Fixture::new();
        let path = fixture.0.join("a.log");
        let text = (0..100)
            .map(|i| format!("[12:00:00.000] 中文 value={i} {}\n", "长文本".repeat(400)))
            .collect::<String>();
        fs::write(&path, text).unwrap();
        let inventory = discover(
            &LibrarySpec {
                directory: Some(fixture.0.clone()),
                files: vec![path.clone(), path],
            },
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(inventory.sources.len(), 1);
        let mut scan = Scan::new(
            inventory,
            Query {
                context: 2,
                samples: 8,
                ..Query::default()
            },
        )
        .unwrap();
        finish(&mut scan, 1024 * 1024);
        let report = scan.report();
        assert_eq!(report["coverage"]["lines_read"], 100);
        assert!(report.to_string().len() <= REPORT_BYTES);
        assert!(report["report_items_omitted_for_size"].as_u64().unwrap() > 0);
    }
    #[test]
    fn invalid_regex_and_unregistered_sources_are_rejected() {
        let mut query = Query::default();
        query.pattern = Some("(".into());
        assert!(query.validate().is_err());
        query.pattern = None;
        query.numeric = vec![NumericRule {
            name: "VIN".into(),
            pattern: "VIN".into(),
            min: None,
            max: None,
        }];
        assert!(query.validate().is_err());
        let fixture = Fixture::new();
        fs::write(fixture.0.join("a.log"), "a").unwrap();
        let query = Query {
            source_ids: vec!["0".repeat(32)],
            ..Query::default()
        };
        assert!(Scan::new(fixture.inventory(), query).is_err());
    }
    #[test]
    fn full_file_analysis_finds_evidence_outside_recent_display_and_bounds_output() {
        let fixture = Fixture::new();
        let path = fixture.0.join("large.log");
        let text = format!(
            "ERROR start\n{}ERROR end\n",
            (0..10000)
                .map(|i| format!("INFO sequence={i} value=5.0\n"))
                .collect::<String>()
        );
        fs::write(&path, &text).unwrap();
        let mut query = Query::default();
        query.contains = vec!["ERROR".into()];
        query.samples = 2;
        let mut scan = Scan::new(fixture.inventory(), query).unwrap();
        finish(&mut scan, 64 * 1024);
        let report = scan.report();
        assert_eq!(report["coverage"]["lines_read"], 10002);
        assert_eq!(report["coverage"]["matched_lines"], 2);
        assert_eq!(report["examples"][0]["line"], 1);
        assert_eq!(report["examples"][1]["line"], 10002);
        assert!(report.to_string().len() <= REPORT_BYTES);
        assert!(text.len() > report.to_string().len() * 20);
    }
    #[test]
    fn grouping_memory_overflow_is_disclosed_without_losing_match_counts() {
        let fixture = Fixture::new();
        let mut text = String::new();
        for i in 0..4200 {
            let key = format!(
                "{}{}{}",
                (b'a' + (i / 676) as u8) as char,
                (b'a' + ((i / 26) % 26) as u8) as char,
                (b'a' + (i % 26) as u8) as char
            );
            text += &format!("event_{key}\n");
        }
        fs::write(fixture.0.join("diverse.log"), text).unwrap();
        let mut scan = Scan::new(
            fixture.inventory(),
            Query {
                samples: 0,
                ..Query::default()
            },
        )
        .unwrap();
        finish(&mut scan, 1024 * 1024);
        let report = scan.report();
        assert_eq!(scan.groups.len(), 4096);
        assert_eq!(report["coverage"]["matched_lines"], 4200);
        assert_eq!(report["grouping_complete"], false);
        assert_eq!(report["untracked_group_lines"], 104);
    }
    #[test]
    fn growing_log_uses_initial_range_without_treating_partial_tail_as_a_measurement() {
        let fixture = Fixture::new();
        let path = fixture.0.join("live.log");
        let original = "ERROR stable\nVIN=4";
        fs::write(&path, original).unwrap();
        let mut scan = Scan::new(fixture.inventory(), Query::default()).unwrap();
        scan.run_chunk(2, Duration::from_secs(2), &AtomicBool::new(false))
            .unwrap();
        use std::io::Write;
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(b".9\nERROR new\n").unwrap();
        drop(file);
        finish(&mut scan, 1024);
        let report = scan.report();
        assert_eq!(report["coverage"]["growing_files"], 1);
        assert_eq!(report["coverage"]["bytes_read"], original.len());
        assert_eq!(report["coverage"]["lines_read"], 1);
        assert_eq!(report["coverage"]["incomplete_tail_lines_skipped"], 1);
        assert_eq!(report["coverage"]["complete"], false);
        assert!(scan.finished());
    }
    #[test]
    fn bomless_utf16_like_input_is_not_reported_as_clean_utf8_text() {
        let fixture = Fixture::new();
        fs::write(fixture.0.join("wide.log"), b"E\0R\0R\0O\0R\0\n\0").unwrap();
        let mut scan = Scan::new(fixture.inventory(), Query::default()).unwrap();
        finish(&mut scan, 100);
        let report = scan.report();
        assert!(
            report["coverage"]["nul_or_utf16_like_lines"]
                .as_u64()
                .unwrap()
                > 0
        );
        assert_eq!(report["coverage"]["complete"], false);
    }
    #[test]
    fn retained_live_snapshot_scans_every_row_resumes_and_preserves_absolute_line_numbers() {
        let lines = (0..5000)
            .map(|i| {
                std::sync::Arc::<str>::from(if i == 0 || i == 4999 {
                    "ERROR boundary"
                } else {
                    "normal"
                })
            })
            .collect();
        let source = Source::memory("live", lines, 1001, 1000);
        assert_eq!(source.id,Source::memory("live",vec![],1,0).id);
        let mut scan = Scan::new(
            Inventory {
                sources: vec![source],
                ..Default::default()
            },
            Query {
                contains: vec!["ERROR".into()],
                samples: 2,
                ..Default::default()
            },
        )
        .unwrap();
        finish(&mut scan, 16);
        let report = scan.report();
        assert_eq!(report["coverage"]["lines_read"], 5000);
        assert_eq!(report["coverage"]["matched_lines"], 2);
        assert_eq!(report["coverage"]["complete"], false);
        assert_eq!(report["coverage"]["preceding_rows_unavailable"], 1000);
        assert_eq!(report["examples"][0]["line"], 1001);
        assert_eq!(report["examples"][1]["line"], 6000);
        assert!(report.to_string().len() <= REPORT_BYTES);
    }
    #[test]
    fn row_scope_never_leaks_outside_context_into_evidence() {
        let source = Source::memory(
            "selection",
            (1..=30)
                .map(|i| std::sync::Arc::<str>::from(format!("private {i}")))
                .collect(),
            1,
            0,
        );
        let mut scan = Scan::new(
            Inventory {
                sources: vec![source],
                ..Default::default()
            },
            Query {
                first_line: 10,
                last_line: Some(12),
                context: 2,
                samples: 3,
                ..Default::default()
            },
        )
        .unwrap();
        finish(&mut scan, 5);
        let report = scan.report();
        assert_eq!(report["coverage"]["lines_in_range"], 3);
        assert_eq!(report["coverage"]["matched_lines"], 3);
        assert_eq!(report["examples"][0]["before"], json!([]));
        assert_eq!(report["examples"][2]["after"], json!([]));
        for sample in report["examples"].as_array().unwrap() {
            for key in ["before", "after"] {
                for pair in sample[key].as_array().unwrap() {
                    let row = pair[0].as_u64().unwrap();
                    assert!((10..=12).contains(&row));
                }
            }
        }
    }
}

/// Opaque canonical file identity, matching the scanner's source_id; never a path.
pub fn source_id_for_path(path:&Path)->Result<String,String>{let p=std::fs::canonicalize(path).map_err(|_|"LOG.SOURCE_UNAVAILABLE")?;let id=p.to_string_lossy();Ok(fingerprint(&if cfg!(windows){id.to_lowercase()}else{id.into_owned()}))}
