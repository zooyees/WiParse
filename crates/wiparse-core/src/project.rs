//! Project workspace under `{data_root}/projects/{id}/`.
//!
//! Index and on-disk layout: project → test case (`tests/{id}`) →
//! test result (`runs/{stamp}` with wave / report / capture artifacts).
//! Testing Hub is the only runner.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub const PROJECT_SCHEMA: &str = "wiparse.project/v1";
pub const MAX_GROUP_FILES: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArtifactView {
    Wave,
    Log,
    Report,
    External,
}

impl ArtifactView {
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "wave" | "waveform" => Self::Wave,
            "log" => Self::Log,
            "report" | "md" | "markdown" => Self::Report,
            _ => Self::External,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Wave => "wave",
            Self::Log => "log",
            Self::Report => "report",
            Self::External => "external",
        }
    }

    pub fn from_filename(name: &str) -> Self {
        let ext = Path::new(name)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match ext.as_str() {
            "isf" | "wfm" | "csv" => Self::Wave,
            "txt" | "log" => Self::Log,
            "md" | "markdown" => Self::Report,
            _ => Self::External,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectTestRef {
    pub id: String,
    /// Plugin that produced this test. Empty on legacy rows where `id` was the plugin id.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub plugin_id: String,
    /// Display name (Hub report / file_prefix). Empty → use `id` (directory slug).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
}

impl<'de> Deserialize<'de> for ProjectTestRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum De {
            Id(String),
            Full {
                id: String,
                #[serde(default)]
                plugin_id: String,
                #[serde(default)]
                name: String,
            },
        }
        Ok(match De::deserialize(deserializer)? {
            De::Id(id) => Self {
                id,
                plugin_id: String::new(),
                name: String::new(),
            },
            De::Full {
                id,
                plugin_id,
                name,
            } => Self {
                id,
                plugin_id,
                name,
            },
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectDoc {
    #[serde(default = "default_schema")]
    pub schema: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub tests: Vec<ProjectTestRef>,
}

fn default_schema() -> String {
    PROJECT_SCHEMA.into()
}

impl ProjectDoc {
    pub fn new(id: &str, name: Option<&str>) -> Self {
        let id = sanitize_id(id, "default");
        Self {
            schema: PROJECT_SCHEMA.into(),
            name: name
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| id.clone()),
            id,
            tests: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct OutputSpec {
    pub id: String,
    pub label: String,
    pub label_zh: String,
    pub view: ArtifactView,
    pub include: Vec<String>,
    pub always: bool,
}

impl OutputSpec {
    pub fn from_plugin_json(v: &Value) -> Vec<Self> {
        let Some(arr) = v.get("outputs").and_then(|x| x.as_array()) else {
            return Vec::new();
        };
        arr.iter()
            .filter_map(|o| {
                let id = o.get("id")?.as_str()?.trim().to_owned();
                if id.is_empty() || !is_valid_id(&id) {
                    return None;
                }
                let include = o
                    .get("include")
                    .and_then(|x| x.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().map(|s| s.to_owned()))
                            .collect::<Vec<_>>()
                    })
                    .filter(|v| !v.is_empty())
                    .unwrap_or_else(|| vec!["*".into()]);
                Some(OutputSpec {
                    label: o
                        .get("label")
                        .and_then(|x| x.as_str())
                        .unwrap_or(&id)
                        .to_owned(),
                    label_zh: o
                        .get("label_zh")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_owned(),
                    view: ArtifactView::parse(
                        o.get("view").and_then(|x| x.as_str()).unwrap_or(""),
                    ),
                    include,
                    always: o.get("always").and_then(|x| x.as_bool()).unwrap_or(false),
                    id,
                })
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct PluginMeta {
    pub id: String,
    pub name: String,
    pub name_zh: String,
    pub outputs: Vec<OutputSpec>,
    /// Default `tests/{test_id}/` slug (`paths.file_prefix`). Empty → `report_name`.
    pub file_prefix: String,
    /// Plugin-manifest report name shown as the Test Report case title.
    pub report_name: String,
    pub report_name_zh: String,
}

impl PluginMeta {
    pub fn report_label(&self, zh: bool) -> &str {
        if zh && !self.report_name_zh.trim().is_empty() {
            return self.report_name_zh.as_str();
        }
        if !self.report_name.trim().is_empty() {
            return self.report_name.as_str();
        }
        if !self.file_prefix.trim().is_empty() {
            return self.file_prefix.as_str();
        }
        ""
    }

    pub fn report_slug(&self) -> &str {
        let prefix = self.file_prefix.trim();
        if !prefix.is_empty() {
            return prefix;
        }
        let name = self.report_name.trim();
        if !name.is_empty() {
            return name;
        }
        self.id.as_str()
    }
}

#[derive(Debug, Clone)]
pub struct IndexedFile {
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
    pub mtime: Option<SystemTime>,
    pub view: ArtifactView,
    pub output_id: String,
}

#[derive(Debug, Clone)]
pub struct IndexedGroup {
    pub id: String,
    pub label: String,
    pub label_zh: String,
    pub view: ArtifactView,
    pub files: Vec<IndexedFile>,
}

/// One Hub `run` (stamp folder under the test case).
#[derive(Debug, Clone)]
pub struct IndexedResult {
    pub stamp: String,
    pub ok: Option<bool>,
    pub groups: Vec<IndexedGroup>,
}

impl IndexedResult {
    pub fn file_count(&self) -> usize {
        self.groups.iter().map(|g| g.files.len()).sum()
    }

    pub fn latest_report(&self) -> Option<&IndexedFile> {
        self.groups
            .iter()
            .filter(|g| g.view == ArtifactView::Report)
            .flat_map(|g| g.files.iter())
            .filter(|f| f.path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("markdown")))
            .max_by_key(|f| f.mtime)
    }

    pub fn latest_any(&self) -> Option<&IndexedFile> {
        self.groups
            .iter()
            .flat_map(|g| g.files.iter())
            .max_by_key(|f| f.mtime)
    }
}

/// Hub run stamp folder: `YYYYMMDD_HHMMSS`.
pub fn is_run_stamp(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 15
        && b[8] == b'_'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 8 || c.is_ascii_digit())
}

/// Format a Hub stamp folder (`20260915_120000`) for the report tree.
pub fn format_run_stamp(stamp: &str) -> String {
    if is_run_stamp(stamp) {
        format!(
            "{}-{}-{} {}:{}:{}",
            &stamp[0..4],
            &stamp[4..6],
            &stamp[6..8],
            &stamp[9..11],
            &stamp[11..13],
            &stamp[13..15]
        )
    } else {
        stamp.to_owned()
    }
}

/// A test case (`tests/{id}/`), usually named by Hub report / file_prefix.
#[derive(Debug, Clone)]
pub struct IndexedTest {
    pub id: String,
    pub name: String,
    pub name_zh: String,
    pub plugin_id: String,
    pub installed: bool,
    pub results: Vec<IndexedResult>,
}

impl IndexedTest {
    pub fn latest_report(&self) -> Option<&IndexedFile> {
        self.results
            .iter()
            .flat_map(|r| r.groups.iter())
            .filter(|g| g.view == ArtifactView::Report)
            .flat_map(|g| g.files.iter())
            .filter(|f| f.path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("markdown")))
            .max_by_key(|f| f.mtime)
    }

    pub fn latest_any(&self) -> Option<&IndexedFile> {
        self.results
            .iter()
            .flat_map(|r| r.groups.iter())
            .flat_map(|g| g.files.iter())
            .max_by_key(|f| f.mtime)
    }

    pub fn file_count(&self) -> usize {
        self.results.iter().map(|r| r.file_count()).sum()
    }
}

#[derive(Debug, Clone)]
pub struct IndexedProject {
    pub id: String,
    pub name: String,
    pub tests: Vec<IndexedTest>,
}

pub fn is_valid_id(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphanumeric()
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

pub fn sanitize_id(raw: &str, fallback: &str) -> String {
    let s = raw.trim();
    if is_valid_id(s) {
        s.to_owned()
    } else if is_valid_id(fallback) {
        fallback.to_owned()
    } else {
        "default".into()
    }
}

/// Directory name for a Hub test. Prefer the report / `file_prefix` slug;
/// fall back to `plugin.id` when the prefix is empty or illegal.
pub fn sanitize_test_id(raw: &str, fallback: &str) -> String {
    let mut s = String::new();
    for c in raw.trim().chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
            s.push(c);
        } else if c.is_whitespace() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        {
            if !s.ends_with('_') {
                s.push('_');
            }
        }
    }
    let s = s.trim_matches(|c: char| c == '_' || c == '.').to_owned();
    if is_valid_id(&s) {
        s
    } else {
        sanitize_id(fallback, "default")
    }
}

impl ProjectTestRef {
    pub fn plugin_key(&self) -> &str {
        if self.plugin_id.is_empty() {
            &self.id
        } else {
            &self.plugin_id
        }
    }

    /// Stored label if it is not empty and not a run stamp.
    pub fn display_name(&self) -> &str {
        let name = self.name.trim();
        if name.is_empty() || is_run_stamp(name) {
            &self.id
        } else {
            name
        }
    }
}

fn is_generic_case_label(s: &str, plugin_id: &str, plugin: Option<&PluginMeta>) -> bool {
    let s = s.trim();
    if s.is_empty() || is_run_stamp(s) {
        return true;
    }
    if s == plugin_id {
        return true;
    }
    let Some(p) = plugin else {
        return false;
    };
    s == p.id
        || (!p.name.is_empty() && s == p.name)
        || (!p.name_zh.is_empty() && s == p.name_zh)
        || (!p.report_name.is_empty() && s == p.report_name)
        || (!p.report_name_zh.is_empty() && s == p.report_name_zh)
        || (!p.file_prefix.is_empty() && s == p.file_prefix)
}

/// Test Report case title: plugin `report_name`, never a run stamp.
pub fn test_case_label(
    test_name: &str,
    test_id: &str,
    plugin_id: &str,
    plugin: Option<&PluginMeta>,
    zh: bool,
) -> String {
    let pid = if plugin_id.trim().is_empty() {
        test_id
    } else {
        plugin_id
    };
    if !is_generic_case_label(test_name, pid, plugin) {
        return test_name.trim().to_owned();
    }
    if let Some(p) = plugin {
        let label = p.report_label(zh).trim();
        if !label.is_empty() {
            return label.to_owned();
        }
    }
    if !is_generic_case_label(test_id, pid, plugin) {
        return test_id.to_owned();
    }
    if let Some(p) = plugin {
        if zh && !p.name_zh.is_empty() {
            return p.name_zh.clone();
        }
        if !p.name.is_empty() {
            return p.name.clone();
        }
    }
    if !is_run_stamp(test_id) {
        test_id.to_owned()
    } else {
        pid.to_owned()
    }
}

/// Register a Hub test from plugin id + report name (folder slug and display name).
pub fn test_ref_from_report(plugin_id: &str, report_name: &str) -> ProjectTestRef {
    let label = report_name.trim();
    if label.is_empty() {
        ProjectTestRef {
            id: sanitize_id(plugin_id, "default"),
            plugin_id: plugin_id.to_owned(),
            name: String::new(),
        }
    } else {
        ProjectTestRef {
            id: sanitize_test_id(label, plugin_id),
            plugin_id: plugin_id.to_owned(),
            name: label.to_owned(),
        }
    }
}

pub fn projects_root(data_root: &Path) -> PathBuf {
    data_root.join("projects")
}

pub fn project_dir(data_root: &Path, project_id: &str) -> PathBuf {
    projects_root(data_root).join(sanitize_id(project_id, "default"))
}

pub fn project_json_path(data_root: &Path, project_id: &str) -> PathBuf {
    project_dir(data_root, project_id).join("project.json")
}

pub fn test_dir(data_root: &Path, project_id: &str, test_id: &str) -> PathBuf {
    project_dir(data_root, project_id)
        .join("tests")
        .join(test_id)
}

fn is_inside(child: &Path, parent: &Path) -> bool {
    let parent = match parent.canonicalize() {
        Ok(p) => p,
        Err(_) => return false,
    };
    let child = match child.canonicalize() {
        Ok(p) => p,
        Err(_) => return false,
    };
    child.starts_with(&parent)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProjectLoad {
    Strict,
    Salvaged,
    DiskOnly,
}

fn normalize_project_doc(doc: &mut ProjectDoc, fallback_id: &str) {
    doc.id = sanitize_id(&doc.id, fallback_id);
    if doc.name.trim().is_empty() {
        doc.name = doc.id.clone();
    }
    doc.tests.retain(|t| is_valid_id(&t.id));
}

fn salvage_project_doc(v: &Value, fallback_id: &str) -> Option<ProjectDoc> {
    let obj = v.as_object()?;
    let id = obj
        .get("id")
        .and_then(|x| x.as_str())
        .unwrap_or(fallback_id);
    let name = obj.get("name").and_then(|x| x.as_str());
    let mut doc = ProjectDoc::new(id, name);
    if let Some(arr) = obj.get("tests").and_then(|x| x.as_array()) {
        for item in arr {
            match item {
                Value::String(s) if is_valid_id(s) => {
                    doc.tests.push(ProjectTestRef {
                        id: s.clone(),
                        plugin_id: String::new(),
                        name: String::new(),
                    });
                }
                other => {
                    if let Ok(t) = serde_json::from_value::<ProjectTestRef>(other.clone()) {
                        if is_valid_id(&t.id) {
                            doc.tests.push(t);
                        }
                    }
                }
            }
        }
    }
    Some(doc)
}

fn try_load_project(data_root: &Path, project_id: &str) -> Option<(ProjectDoc, ProjectLoad)> {
    let fallback = sanitize_id(project_id, "default");
    let dir = project_dir(data_root, &fallback);
    let path = dir.join("project.json");
    if let Ok(text) = fs::read_to_string(&path) {
        if let Ok(mut doc) = serde_json::from_str::<ProjectDoc>(&text) {
            normalize_project_doc(&mut doc, &fallback);
            return Some((doc, ProjectLoad::Strict));
        }
        if let Ok(v) = serde_json::from_str::<Value>(&text) {
            if let Some(mut doc) = salvage_project_doc(&v, &fallback) {
                normalize_project_doc(&mut doc, &fallback);
                return Some((doc, ProjectLoad::Salvaged));
            }
        }
    }
    if dir.is_dir() {
        let mut doc = ProjectDoc::new(&fallback, None);
        doc.tests = merge_testcase_refs(data_root, &doc);
        return Some((doc, ProjectLoad::DiskOnly));
    }
    None
}

/// Canonical child must live under `projects/{id}`. Missing files are rejected.
pub fn path_in_project_jail(data_root: &Path, project_id: &str, candidate: &Path) -> bool {
    is_inside(candidate, &project_dir(data_root, project_id))
}

pub fn load_project(data_root: &Path, project_id: &str) -> Option<ProjectDoc> {
    try_load_project(data_root, project_id).map(|(doc, _)| doc)
}

pub fn save_project(data_root: &Path, doc: &ProjectDoc) -> std::io::Result<PathBuf> {
    let dir = project_dir(data_root, &doc.id);
    fs::create_dir_all(&dir)?;
    let path = dir.join("project.json");
    let mut out = doc.clone();
    out.schema = PROJECT_SCHEMA.into();
    out.id = sanitize_id(&out.id, "default");
    if out.name.trim().is_empty() {
        out.name = out.id.clone();
    }
    let text = serde_json::to_string_pretty(&out).unwrap_or_else(|_| "{}".into());
    fs::write(&path, format!("{text}\n"))?;
    Ok(path)
}

pub fn ensure_project(data_root: &Path, project_id: &str, name: Option<&str>) -> ProjectDoc {
    let id = sanitize_id(project_id, "default");
    match try_load_project(data_root, &id) {
        Some((doc, ProjectLoad::Strict)) => doc,
        Some((mut doc, kind)) => {
            if doc.name.trim().is_empty() {
                if let Some(n) = name.map(str::trim).filter(|s| !s.is_empty()) {
                    doc.name = n.to_owned();
                }
            }
            if kind == ProjectLoad::DiskOnly {
                let src = project_json_path(data_root, &id);
                if src.is_file() {
                    let _ = fs::copy(&src, src.with_extension("json.bak"));
                }
            }
            let _ = save_project(data_root, &doc);
            doc
        }
        None => {
            let doc = ProjectDoc::new(&id, name);
            let _ = save_project(data_root, &doc);
            doc
        }
    }
}

pub fn list_projects(data_root: &Path) -> Vec<ProjectDoc> {
    let root = projects_root(data_root);
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(&root) else {
        return out;
    };
    for ent in entries.flatten() {
        let path = ent.path();
        if !path.is_dir() {
            continue;
        }
        let id = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        if !is_valid_id(id) {
            continue;
        }
        if let Some(doc) = load_project(data_root, id) {
            out.push(doc);
        }
    }
    out.sort_by(|a, b| a.id.to_ascii_lowercase().cmp(&b.id.to_ascii_lowercase()));
    out
}

pub fn add_test(data_root: &Path, project_id: &str, test_id: &str) -> std::io::Result<ProjectDoc> {
    add_test_entry(
        data_root,
        project_id,
        ProjectTestRef {
            id: test_id.trim().to_owned(),
            plugin_id: String::new(),
            name: String::new(),
        },
    )
}

pub fn add_test_entry(
    data_root: &Path,
    project_id: &str,
    entry: ProjectTestRef,
) -> std::io::Result<ProjectDoc> {
    let raw = entry.id.trim();
    let id = if is_valid_id(raw) {
        raw.to_owned()
    } else {
        sanitize_test_id(raw, entry.plugin_key())
    };
    if !is_valid_id(&id) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid test id",
        ));
    }
    let mut doc = ensure_project(data_root, project_id, None);
    if let Some(existing) = doc.tests.iter_mut().find(|t| t.id == id) {
        if existing.plugin_id.is_empty() && !entry.plugin_id.is_empty() {
            existing.plugin_id = entry.plugin_id;
        }
        if existing.name.is_empty() && !entry.name.is_empty() {
            existing.name = entry.name;
        }
        save_project(data_root, &doc)?;
        return Ok(doc);
    }
    doc.tests.push(ProjectTestRef {
        id,
        plugin_id: entry.plugin_id,
        name: entry.name,
    });
    save_project(data_root, &doc)?;
    Ok(doc)
}

pub fn remove_test(data_root: &Path, project_id: &str, test_id: &str) -> std::io::Result<ProjectDoc> {
    let id = test_id.trim();
    let mut doc = ensure_project(data_root, project_id, None);
    let before = doc.tests.len();
    doc.tests.retain(|t| t.id != id);
    if doc.tests.len() != before {
        save_project(data_root, &doc)?;
    }
    Ok(doc)
}

fn match_simple_glob(name: &str, pattern: &str) -> bool {
    let g = pattern.trim();
    if g.is_empty() || g == "*" {
        return true;
    }
    if let Some(ext) = g.strip_prefix("*.") {
        if !ext.contains('*') {
            return name.to_ascii_lowercase().ends_with(&format!(".{}", ext.to_ascii_lowercase()));
        }
    }
    name.eq_ignore_ascii_case(g)
}

fn file_meta(path: &Path) -> Option<(u64, Option<SystemTime>)> {
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    Some((meta.len(), meta.modified().ok()))
}

fn push_file(
    map: &mut HashMap<String, Vec<IndexedFile>>,
    seen: &mut HashSet<PathBuf>,
    output_id: &str,
    view: ArtifactView,
    path: PathBuf,
) {
    let key = path.canonicalize().unwrap_or_else(|_| path.clone());
    if !seen.insert(key) {
        return;
    }
    let Some((size, mtime)) = file_meta(&path) else {
        return;
    };
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    map.entry(output_id.to_owned()).or_default().push(IndexedFile {
        name,
        path,
        size,
        mtime,
        view,
        output_id: output_id.to_owned(),
    });
}

fn ingest_run_json(
    map: &mut HashMap<String, Vec<IndexedFile>>,
    seen: &mut HashSet<PathBuf>,
    run_dir: &Path,
    outputs: &[OutputSpec],
) {
    let path = run_dir.join("run.json");
    let Ok(text) = fs::read_to_string(&path) else {
        return;
    };
    let Ok(v) = serde_json::from_str::<Value>(&text) else {
        return;
    };
    let items = v
        .get("artifacts")
        .and_then(|a| a.get("items"))
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    for item in items {
        let rel = item.get("path").and_then(|x| x.as_str()).unwrap_or("");
        if rel.is_empty() {
            continue;
        }
        let p = PathBuf::from(rel);
        if !is_inside(&p, run_dir) {
            continue;
        }
        let output = item
            .get("output")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .trim();
        let spec = outputs.iter().find(|o| o.id == output);
        let view = item
            .get("view")
            .and_then(|x| x.as_str())
            .map(ArtifactView::parse)
            .or_else(|| spec.map(|s| s.view))
            .unwrap_or_else(|| ArtifactView::from_filename(&p.display().to_string()));
        let oid = if output.is_empty() {
            spec.map(|s| s.id.as_str()).unwrap_or("_other")
        } else {
            output
        };
        push_file(map, seen, oid, view, p);
    }
}

fn glob_output_dir(
    map: &mut HashMap<String, Vec<IndexedFile>>,
    seen: &mut HashSet<PathBuf>,
    run_dir: &Path,
    spec: &OutputSpec,
) {
    let dir = run_dir.join("artifacts").join(&spec.id);
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };
    for ent in entries.flatten() {
        let path = ent.path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        if !spec.include.iter().any(|g| match_simple_glob(name, g)) {
            continue;
        }
        push_file(map, seen, &spec.id, spec.view, path);
    }
}

fn finish_groups(
    outputs: &[OutputSpec],
    mut map: HashMap<String, Vec<IndexedFile>>,
) -> Vec<IndexedGroup> {
    let mut groups = Vec::new();
    for spec in outputs {
        let mut files = map.remove(&spec.id).unwrap_or_default();
        if files.is_empty() && !spec.always {
            continue;
        }
        files.sort_by(|a, b| b.mtime.cmp(&a.mtime).then_with(|| a.name.cmp(&b.name)));
        files.truncate(MAX_GROUP_FILES);
        groups.push(IndexedGroup {
            id: spec.id.clone(),
            label: spec.label.clone(),
            label_zh: spec.label_zh.clone(),
            view: spec.view,
            files,
        });
    }
    let mut leftover: Vec<IndexedFile> = map.into_values().flatten().collect();
    if leftover.is_empty() {
        return groups;
    }
    if outputs.is_empty() {
        leftover.sort_by(|a, b| b.mtime.cmp(&a.mtime).then_with(|| a.name.cmp(&b.name)));
        leftover.truncate(MAX_GROUP_FILES);
        groups.push(IndexedGroup {
            id: "run".into(),
            label: "Run output".into(),
            label_zh: "运行产物".into(),
            view: leftover
                .first()
                .map(|f| f.view)
                .unwrap_or(ArtifactView::External),
            files: leftover,
        });
    } else {
        leftover.sort_by(|a, b| b.mtime.cmp(&a.mtime));
        leftover.truncate(MAX_GROUP_FILES);
        groups.push(IndexedGroup {
            id: "_other".into(),
            label: "Other".into(),
            label_zh: "其它".into(),
            view: ArtifactView::External,
            files: leftover,
        });
    }
    groups
}

fn peek_run_ok(run_dir: &Path) -> Option<bool> {
    let text = fs::read_to_string(run_dir.join("run.json")).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    v.get("ok").and_then(|x| x.as_bool())
}

fn peek_plugin_id(test_dir: &Path) -> Option<String> {
    let runs = test_dir.join("runs");
    let entries = fs::read_dir(&runs).ok()?;
    for ent in entries.flatten() {
        let text = fs::read_to_string(ent.path().join("run.json")).ok()?;
        let v: Value = serde_json::from_str(&text).ok()?;
        if let Some(p) = v.get("plugin").and_then(|x| x.as_str()) {
            let p = p.trim();
            if !p.is_empty() {
                return Some(p.to_owned());
            }
        }
    }
    None
}

fn index_one_run(run_dir: &Path, outputs: &[OutputSpec]) -> IndexedResult {
    let stamp = run_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut map: HashMap<String, Vec<IndexedFile>> = HashMap::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    ingest_run_json(&mut map, &mut seen, run_dir, outputs);
    for spec in outputs {
        glob_output_dir(&mut map, &mut seen, run_dir, spec);
    }
    IndexedResult {
        stamp,
        ok: peek_run_ok(run_dir),
        groups: finish_groups(outputs, map),
    }
}

fn merge_testcase_refs(data_root: &Path, doc: &ProjectDoc) -> Vec<ProjectTestRef> {
    let mut refs = doc.tests.clone();
    let mut seen: HashSet<String> = refs.iter().map(|t| t.id.clone()).collect();
    let tests_root = project_dir(data_root, &doc.id).join("tests");
    if let Ok(entries) = fs::read_dir(&tests_root) {
        for ent in entries.flatten() {
            let path = ent.path();
            if !path.is_dir() {
                continue;
            }
            let id = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            if !is_valid_id(id) || !seen.insert(id.to_owned()) {
                continue;
            }
            let plugin_id = peek_plugin_id(&path).unwrap_or_default();
            refs.push(ProjectTestRef {
                id: id.to_owned(),
                plugin_id,
                name: if is_run_stamp(id) {
                    String::new()
                } else {
                    id.to_owned()
                },
            });
        }
    }
    refs.sort_by(|a, b| a.id.to_ascii_lowercase().cmp(&b.id.to_ascii_lowercase()));
    refs
}

pub fn index_project(
    data_root: &Path,
    doc: &ProjectDoc,
    plugins: &[PluginMeta],
) -> IndexedProject {
    let mut tests = Vec::new();
    for tref in merge_testcase_refs(data_root, doc) {
        let plugin_id = tref.plugin_key().to_owned();
        let plugin = plugins.iter().find(|p| p.id == plugin_id);
        let name = test_case_label(&tref.name, &tref.id, &plugin_id, plugin, false);
        let name_zh = test_case_label(&tref.name, &tref.id, &plugin_id, plugin, true);
        let outputs = plugin.map(|p| p.outputs.as_slice()).unwrap_or(&[]);
        let mut results = Vec::new();
        let runs = test_dir(data_root, &doc.id, &tref.id).join("runs");
        if let Ok(entries) = fs::read_dir(&runs) {
            for ent in entries.flatten() {
                let run_dir = ent.path();
                if !run_dir.is_dir() {
                    continue;
                }
                let result = index_one_run(&run_dir, outputs);
                if result.groups.iter().all(|g| g.files.is_empty()) && result.ok.is_none() {
                    continue;
                }
                results.push(result);
            }
        }
        results.sort_by(|a, b| b.stamp.cmp(&a.stamp));
        tests.push(IndexedTest {
            id: tref.id.clone(),
            name,
            name_zh,
            plugin_id,
            installed: plugin.is_some(),
            results,
        });
    }
    IndexedProject {
        id: doc.id.clone(),
        name: doc.name.clone(),
        tests,
    }
}

fn group_snapshot(g: &IndexedGroup) -> Value {
    let files: Vec<Value> = g
        .files
        .iter()
        .map(|f| {
            serde_json::json!({
                "name": f.name,
                "path": f.path.display().to_string(),
                "size": f.size,
                "view": f.view.as_str(),
                "output": f.output_id,
            })
        })
        .collect();
    serde_json::json!({
        "id": g.id,
        "label": g.label,
        "label_zh": g.label_zh,
        "view": g.view.as_str(),
        "file_count": files.len(),
        "files": files,
    })
}

pub fn tree_snapshot(index: &IndexedProject) -> Value {
    let tests: Vec<Value> = index
        .tests
        .iter()
        .map(|t| {
            let results: Vec<Value> = t
                .results
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "stamp": r.stamp,
                        "ok": r.ok,
                        "file_count": r.file_count(),
                        "groups": r.groups.iter().map(group_snapshot).collect::<Vec<_>>(),
                    })
                })
                .collect();
            serde_json::json!({
                "id": t.id,
                "name": t.name,
                "name_zh": t.name_zh,
                "plugin_id": t.plugin_id,
                "installed": t.installed,
                "file_count": t.file_count(),
                "results": results,
            })
        })
        .collect();
    serde_json::json!({
        "id": index.id,
        "name": index.name,
        "tests": tests,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp() -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "wiparse-proj-{}-{}",
            std::process::id(),
uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn latest_report_ignores_newer_json_in_report_group() {
        let file = |name: &str, seconds| IndexedFile {
            name: name.into(), path: PathBuf::from(name), size: 1,
            mtime: Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(seconds)),
            view: ArtifactView::Report, output_id: "reports".into(),
        };
        let result = IndexedResult {stamp: "run".into(), ok: Some(true), groups: vec![IndexedGroup {
            id:"reports".into(), label:"Reports".into(), label_zh:String::new(), view:ArtifactView::Report,
            files:vec![file("summary.md",1),file("manifest.json",2)]
        }]};
        assert_eq!(result.latest_report().unwrap().name, "summary.md");
        let case = IndexedTest {id:"case".into(), name:String::new(), name_zh:String::new(), plugin_id:String::new(), installed:true, results:vec![result]};
        assert_eq!(case.latest_report().unwrap().name, "summary.md");
    }

    #[test]
    fn sanitize_and_jail() {
        assert_eq!(sanitize_id("qi-lot-42", "default"), "qi-lot-42");
        assert_eq!(sanitize_id("../etc", "default"), "default");
        let root = tmp();
        let doc = ensure_project(&root, "default", Some("Default"));
        assert_eq!(doc.id, "default");
        let inside = project_dir(&root, "default").join("tests").join("x");
        fs::create_dir_all(&inside).unwrap();
        let file = inside.join("a.md");
        fs::write(&file, "hi").unwrap();
        assert!(path_in_project_jail(&root, "default", &file));
        let outside = root.join("nope.txt");
        fs::write(&outside, "x").unwrap();
        assert!(!path_in_project_jail(&root, "default", &outside));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn index_skips_empty_wave_group() {
        let root = tmp();
        let mut doc = ensure_project(&root, "lot1", Some("Lot 1"));
        doc.tests.push(ProjectTestRef {
            id: "example-smoke".into(),
            plugin_id: String::new(),
            name: String::new(),
        });
        save_project(&root, &doc).unwrap();
        let run = test_dir(&root, "lot1", "example-smoke")
            .join("runs")
            .join("20260101_000000");
        fs::create_dir_all(run.join("artifacts").join("logs")).unwrap();
        let log = run.join("artifacts").join("logs").join("hub.log");
        let mut f = fs::File::create(&log).unwrap();
        writeln!(f, "ok").unwrap();
        let run_json = serde_json::json!({
            "schema": "wiparse.run/v1",
            "plugin": "example-smoke",
            "artifacts": { "items": [{ "output": "logs", "path": log.display().to_string(), "view": "log" }] }
        });
        fs::write(run.join("run.json"), run_json.to_string()).unwrap();
        let plugins = vec![PluginMeta {
            id: "example-smoke".into(),
            name: "Example Smoke".into(),
            name_zh: "示例冒烟".into(),
            file_prefix: String::new(),
            report_name: "ExampleSmoke".into(),
            report_name_zh: "示例冒烟".into(),
            outputs: vec![OutputSpec {
                id: "logs".into(),
                label: "Log".into(),
                label_zh: "运行日志".into(),
                view: ArtifactView::Log,
                include: vec!["*.log".into(), "*.txt".into()],
                always: false,
            }],
        }];
        let idx = index_project(&root, &load_project(&root, "lot1").unwrap(), &plugins);
        assert_eq!(idx.tests[0].name, "ExampleSmoke");
        assert_eq!(idx.tests[0].name_zh, "示例冒烟");
        assert_eq!(idx.tests[0].results.len(), 1);
        assert_eq!(idx.tests[0].results[0].stamp, "20260101_000000");
        assert_eq!(idx.tests[0].results[0].groups.len(), 1);
        assert_eq!(idx.tests[0].results[0].groups[0].id, "logs");
        assert!(idx.tests[0].results[0].groups.iter().all(|g| g.id != "waves"));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn remove_test_keeps_other_ids() {
        let root = tmp();
        add_test(&root, "lot1", "example-smoke").unwrap();
        add_test(&root, "lot1", "scope-serial-monitor").unwrap();
        let doc = remove_test(&root, "lot1", "example-smoke").unwrap();
        assert_eq!(doc.tests.len(), 1);
        assert_eq!(doc.tests[0].id, "scope-serial-monitor");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn sanitize_test_id_from_report_name() {
        assert_eq!(sanitize_test_id("ScopeSerial", "plug"), "ScopeSerial");
        assert_eq!(sanitize_test_id("My Lot", "plug"), "My_Lot");
        assert_eq!(sanitize_test_id("  ", "scope-serial-monitor"), "scope-serial-monitor");
    }

    #[test]
    fn test_ref_follows_report_name_not_plugin_title() {
        let named = test_ref_from_report("scope-serial-monitor", "ScopeSerial");
        assert_eq!(named.id, "ScopeSerial");
        assert_eq!(named.plugin_id, "scope-serial-monitor");
        assert_eq!(named.name, "ScopeSerial");
        assert_eq!(named.display_name(), "ScopeSerial");
        let fallback = test_ref_from_report("example-smoke", "ExampleSmoke");
        assert_eq!(fallback.id, "ExampleSmoke");
        assert_eq!(fallback.plugin_id, "example-smoke");
        assert_eq!(fallback.name, "ExampleSmoke");
        let empty = test_ref_from_report("example-smoke", "");
        assert_eq!(empty.id, "example-smoke");
        assert!(empty.name.is_empty());
    }

    #[test]
    fn test_case_label_prefers_report_name_over_stamp() {
        let plugin = PluginMeta {
            id: "wiparse-dap".into(),
            name: "CMSIS-DAP Probe".into(),
            name_zh: "CMSIS-DAP 调试通道".into(),
            outputs: Vec::new(),
            file_prefix: "WiparseDap".into(),
            report_name: "WiparseDap".into(),
            report_name_zh: "DAP 通道".into(),
        };
        assert_eq!(
            test_case_label("wiparse-dap", "wiparse-dap", "wiparse-dap", Some(&plugin), true),
            "DAP 通道"
        );
        assert_eq!(
            test_case_label(
                "20260916_143758",
                "20260916_143758",
                "wiparse-dap",
                Some(&plugin),
                false
            ),
            "WiparseDap"
        );
        assert_eq!(
            test_case_label("MyLot", "MyLot", "wiparse-dap", Some(&plugin), true),
            "MyLot"
        );
    }

    #[test]
    fn index_does_not_use_plugin_marketing_name() {
        let root = tmp();
        add_test_entry(
            &root,
            "lot1",
            ProjectTestRef {
                id: "scope-serial-monitor".into(),
                plugin_id: "scope-serial-monitor".into(),
                name: String::new(),
            },
        )
        .unwrap();
        let plugins = vec![PluginMeta {
            id: "scope-serial-monitor".into(),
            name: "Scope & Serial Monitor".into(),
            name_zh: "示波器与串口监控".into(),
            file_prefix: "ScopeSerial".into(),
            report_name: "ScopeSerial".into(),
            report_name_zh: "示波器串口".into(),
            outputs: Vec::new(),
        }];
        let idx = index_project(&root, &load_project(&root, "lot1").unwrap(), &plugins);
        assert_eq!(idx.tests[0].name, "ScopeSerial");
        assert_eq!(idx.tests[0].name_zh, "示波器串口");
        assert_ne!(idx.tests[0].name, "Scope & Serial Monitor");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn index_uses_report_name_and_plugin_id() {
        let root = tmp();
        add_test_entry(
            &root,
            "lot1",
            ProjectTestRef {
                id: "ScopeSerial".into(),
                plugin_id: "example-smoke".into(),
                name: "ScopeSerial".into(),
            },
        )
        .unwrap();
        let run = test_dir(&root, "lot1", "ScopeSerial")
            .join("runs")
            .join("20260101_000000");
        fs::create_dir_all(run.join("artifacts").join("logs")).unwrap();
        let log = run.join("artifacts").join("logs").join("out.log");
        fs::write(&log, "x").unwrap();
        let run_json = serde_json::json!({
            "schema": "wiparse.run/v1",
            "plugin": "example-smoke",
            "artifacts": { "items": [{ "output": "logs", "path": log.display().to_string(), "view": "log" }] }
        });
        fs::write(run.join("run.json"), run_json.to_string()).unwrap();
        let plugins = vec![PluginMeta {
            id: "example-smoke".into(),
            name: "Example Smoke".into(),
            name_zh: "示例冒烟".into(),
            file_prefix: "ScopeSerial".into(),
            report_name: "ScopeSerial".into(),
            report_name_zh: "示波器串口".into(),
            outputs: vec![OutputSpec {
                id: "logs".into(),
                label: "Log".into(),
                label_zh: "运行日志".into(),
                view: ArtifactView::Log,
                include: vec!["*.log".into()],
                always: false,
            }],
        }];
        let idx = index_project(&root, &load_project(&root, "lot1").unwrap(), &plugins);
        assert_eq!(idx.tests[0].id, "ScopeSerial");
        assert_eq!(idx.tests[0].name, "ScopeSerial");
        assert_eq!(idx.tests[0].plugin_id, "example-smoke");
        assert!(idx.tests[0].installed);
        assert_eq!(idx.tests[0].results[0].groups[0].id, "logs");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn format_run_stamp_pretty() {
        assert_eq!(format_run_stamp("20260915_120000"), "2026-09-15 12:00:00");
        assert_eq!(format_run_stamp("odd"), "odd");
    }

    #[test]
    fn index_keeps_two_stamps_as_results() {
        let root = tmp();
        add_test(&root, "lot1", "example-smoke").unwrap();
        let plugins = vec![PluginMeta {
            id: "example-smoke".into(),
            name: "Example Smoke".into(),
            name_zh: "示例冒烟".into(),
            file_prefix: String::new(),
            report_name: "ExampleSmoke".into(),
            report_name_zh: "示例冒烟".into(),
            outputs: vec![OutputSpec {
                id: "logs".into(),
                label: "Log".into(),
                label_zh: "运行日志".into(),
                view: ArtifactView::Log,
                include: vec!["*.log".into()],
                always: false,
            }],
        }];
        for stamp in ["20260101_000000", "20260102_120000"] {
            let run = test_dir(&root, "lot1", "example-smoke")
                .join("runs")
                .join(stamp);
            fs::create_dir_all(run.join("artifacts").join("logs")).unwrap();
            let log = run.join("artifacts").join("logs").join("out.log");
            fs::write(&log, stamp).unwrap();
            let run_json = serde_json::json!({
                "schema": "wiparse.run/v1",
                "plugin": "example-smoke",
                "ok": stamp.ends_with("120000"),
                "artifacts": { "items": [{ "output": "logs", "path": log.display().to_string(), "view": "log" }] }
            });
            fs::write(run.join("run.json"), run_json.to_string()).unwrap();
        }
        let idx = index_project(&root, &load_project(&root, "lot1").unwrap(), &plugins);
        assert_eq!(idx.tests[0].results.len(), 2);
        assert_eq!(idx.tests[0].results[0].stamp, "20260102_120000");
        assert_eq!(idx.tests[0].results[0].ok, Some(true));
        assert_eq!(idx.tests[0].results[1].stamp, "20260101_000000");
        assert_eq!(idx.tests[0].results[1].ok, Some(false));
        let snap = tree_snapshot(&idx);
        assert_eq!(snap["tests"][0]["results"].as_array().unwrap().len(), 2);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn index_discovers_testcase_dir_missing_from_json() {
        let root = tmp();
        let _ = ensure_project(&root, "lot1", Some("Lot 1"));
        let run = test_dir(&root, "lot1", "mpp25w_test")
            .join("runs")
            .join("20260915_010000");
        fs::create_dir_all(run.join("artifacts").join("waves")).unwrap();
        let wave = run.join("artifacts").join("waves").join("a.isf");
        fs::write(&wave, b"isf").unwrap();
        let run_json = serde_json::json!({
            "schema": "wiparse.run/v1",
            "plugin": "example-smoke",
            "ok": true,
            "artifacts": { "items": [{ "output": "waves", "path": wave.display().to_string(), "view": "wave" }] }
        });
        fs::write(run.join("run.json"), run_json.to_string()).unwrap();
        let plugins = vec![PluginMeta {
            id: "example-smoke".into(),
            name: "Example Smoke".into(),
            name_zh: "示例冒烟".into(),
            file_prefix: String::new(),
            report_name: "ExampleSmoke".into(),
            report_name_zh: "示例冒烟".into(),
            outputs: vec![OutputSpec {
                id: "waves".into(),
                label: "Waves".into(),
                label_zh: "波形".into(),
                view: ArtifactView::Wave,
                include: vec!["*.isf".into()],
                always: false,
            }],
        }];
        let idx = index_project(&root, &load_project(&root, "lot1").unwrap(), &plugins);
        assert_eq!(idx.tests.len(), 1);
        assert_eq!(idx.tests[0].id, "mpp25w_test");
        assert_eq!(idx.tests[0].plugin_id, "example-smoke");
        assert_eq!(idx.tests[0].results[0].groups[0].id, "waves");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn list_projects_keeps_legacy_after_create() {
        let root = tmp();
        let def = project_dir(&root, "default");
        fs::create_dir_all(def.join("tests").join("scope-serial-monitor")).unwrap();
        fs::write(
            def.join("project.json"),
            r#"{
  "schema": "wiparse.project/v1",
  "id": "default",
  "name": "default",
  "tests": ["scope-serial-monitor"]
}
"#,
        )
        .unwrap();
        ensure_project(&root, "lot2", Some("lot2"));
        let ids: Vec<String> = list_projects(&root).into_iter().map(|p| p.id).collect();
        assert!(ids.contains(&"default".to_string()), "{ids:?}");
        assert!(ids.contains(&"lot2".to_string()), "{ids:?}");
        let def_doc = load_project(&root, "default").unwrap();
        assert_eq!(def_doc.tests[0].id, "scope-serial-monitor");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn list_projects_includes_dir_without_json() {
        let root = tmp();
        fs::create_dir_all(project_dir(&root, "oldlot").join("tests").join("t1")).unwrap();
        ensure_project(&root, "newlot", None);
        let ids: Vec<String> = list_projects(&root).into_iter().map(|p| p.id).collect();
        assert!(ids.contains(&"oldlot".to_string()), "{ids:?}");
        assert!(ids.contains(&"newlot".to_string()), "{ids:?}");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn ensure_project_does_not_wipe_legacy_string_tests() {
        let root = tmp();
        let def = project_dir(&root, "default");
        fs::create_dir_all(def.join("tests").join("scope-serial-monitor")).unwrap();
        fs::write(
            def.join("project.json"),
            r#"{"id":"default","tests":["scope-serial-monitor"]}"#,
        )
        .unwrap();
        let doc = ensure_project(&root, "default", None);
        assert_eq!(doc.tests.len(), 1);
        assert_eq!(doc.tests[0].id, "scope-serial-monitor");
        fs::remove_dir_all(&root).ok();
    }
}
