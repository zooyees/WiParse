//! AI-facing local text tools. The model receives opaque IDs, never a file path.
use super::*;
use serde_json::Value;
use std::sync::Mutex;
use wiparse_core::log_analysis::{self, LibrarySpec, Query, Scan};
pub(super) fn catalog() -> &'static str {
    r#"Local text tools: ONLY manual conversation with a non-none data_scope; the host restricts sources and row ranges to the user selection, read-only, no device-control permission required.
log.files {"offset":0,"limit":16}: inventory within the user-selected current log, row range, selected text, or opened disk logs and user-chosen directory recursively (.log/.txt/.jsonl/.csv and uncompressed rotations). Use opaque source IDs, never file paths. next_offset pages results. If no files, tell user to open logs or choose the visible log directory.
log.analyze {}: scan ALL registered files locally, return bounded cumulative statistics/evidence. Start with samples:0 for a cheap overview. Optional fields: source_ids:[IDs] (empty=all,max32); contains:[literals] (any by default), require_all:false, exclude:[literals] (max8 each,64 UTF-8 bytes per item); case_sensitive:false; pattern:"Rust regex" (max256 bytes); first_line:1,last_line:N (per-file,1-based); context:0..2,samples:0..8,group:true; numeric:[{"name":"VIN","pattern":"VIN=([+-]?[0-9.]+)","min":4.5,"max":5.5}] (max4,group1 is numeric value, limits only if supplied by user, units from the log).
Prefer aggregate counts first, then targeted examples around anomalies. Keyword counts are in-range before filtering; numeric statistics apply to matched lines. Grouping removes timestamps/numbers and tracks normalized prefixes, not semantic equivalence. Reports never contain the full raw input. Oversized lines, changing files, UTF-8 decoding issues and grouping limits are explicit. Unsaved disk edits are not read. Live logs use immutable retained-line snapshots; evicted rows are explicitly unavailable, never claimed as analyzed. Selection sources use relative row numbering.
The user sets a local byte budget; text scanning also yields after a 15s working slice. If continuation is non-null, call log.analyze {"cursor":"host-issued token"} ONLY (no new filters). Results are cumulative; DO NOT sum successive reports. last_analysis is a previous snapshot: start a fresh query without cursor to refresh current data. complete=false cannot establish a full-log conclusion or a clean test. Latest analysis/cursor is in local_log_library. Follow the six-tool-call limit, explain remaining coverage and continue on a later user request if needed. Local preview uses no AI tokens. No edits, shell, arbitrary paths or invented measurements."#
}

#[cfg(test)]
pub(super) static LOCAL_TEST_LOCK: Mutex<()> = Mutex::new(());

static WORKERS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
struct WorkerPermit;
impl Drop for WorkerPermit {
    fn drop(&mut self) {
        WORKERS.fetch_sub(1, Ordering::AcqRel);
    }
}
fn worker_permit() -> Result<WorkerPermit, String> {
    WORKERS
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
            (count < 2).then_some(count + 1)
        })
        .map(|_| WorkerPermit)
        .map_err(|_| "本地扫描仍在结束，请稍后重试 / Local scan workers are still finishing".into())
}

#[derive(Default)]
struct Store {
    jobs: std::collections::VecDeque<(String, Arc<Mutex<Scan>>)>,
    last: Option<Value>,
}
pub(super) struct Library {
    pub directory: Option<PathBuf>,
    pub budget_mib: u64,
    store: Arc<Mutex<Store>>,
    scope_key: String,
}
impl Default for Library {
    fn default() -> Self {
        Self {
            directory: None,
            budget_mib: 1024,
            scope_key: String::new(),
            store: Arc::new(Mutex::new(Store::default())),
        }
    }
}
pub(super) fn is_local(method: &str) -> bool {
    matches!(method, "log.files" | "log.analyze")
}
pub(super) fn validate(method: &str, params: &Value) -> Result<(), String> {
    let object = params.as_object().ok_or("LOG.INVALID_QUERY")?;
    if method == "log.files" {
        if object
            .keys()
            .any(|key| !matches!(key.as_str(), "offset" | "limit"))
            || object
                .get("offset")
                .is_some_and(|v| !v.as_u64().is_some_and(|n| n <= 4096))
            || object
                .get("limit")
                .is_some_and(|v| !v.as_u64().is_some_and(|n| (1..=16).contains(&n)))
        {
            return Err("LOG.INVALID_QUERY: offset 0..4096, limit 1..16".into());
        }
    } else if method == "log.analyze" {
        if let Some(cursor) = object.get("cursor") {
            if object.len() != 1
                || !cursor
                    .as_str()
                    .is_some_and(|s| uuid::Uuid::parse_str(s).is_ok())
            {
                return Err(
                    "LOG.INVALID_CURSOR: continuation accepts only its host-issued cursor".into(),
                );
            }
        } else {
            serde_json::from_value::<Query>(params.clone())
                .map_err(|e| e.to_string())?
                .validate()?;
        }
    } else {
        return Err("LOG.UNSUPPORTED_METHOD".into());
    }
    Ok(())
}
impl Library {
    pub fn bind_scope(&mut self, key: String) {
        if self.scope_key != key {
            self.reset();
            self.scope_key = key;
        }
    }
    pub fn reset(&mut self) {
        self.store = Arc::new(Mutex::new(Store::default()));
    }
    pub fn status(&self) -> Value {
        let last = self
            .store
            .try_lock()
            .ok()
            .and_then(|store| store.last.clone());
        json!({"enabled":true,"directory_selected":self.directory.is_some(),"chunk_mib":self.budget_mib,"raw_files_uploaded":false,"last_analysis":last})
    }
    pub fn preview_params(&self) -> Value {
        self.store.try_lock().ok().and_then(|store|store.last.as_ref()?.get("continuation")?.as_str().map(|cursor|json!({"cursor":cursor}))).unwrap_or_else(||json!({"contains":["error","warn","timeout","fail","错误","异常","超时","告警"],"samples":4}))
    }
    #[cfg(test)]
    pub fn start(
        &self,
        method: &str,
        params: Value,
        files: Vec<PathBuf>,
        cancel: Arc<AtomicBool>,
        ctx: egui::Context,
    ) -> Result<Receiver<crate::backend::InvokeReply>, String> {
        self.start_scoped(method, params, files, None, None, true, cancel, ctx)
    }
    pub fn start_scoped(
        &self,
        method: &str,
        mut params: Value,
        files: Vec<PathBuf>,
        memory: Option<log_analysis::Source>,
        range: Option<(u64, u64)>,
        include_directory: bool,
        cancel: Arc<AtomicBool>,
        ctx: egui::Context,
    ) -> Result<Receiver<crate::backend::InvokeReply>, String> {
        validate(method, &params)?;
        if method == "log.analyze" && params.get("cursor").is_none() {
            if let Some((first, last)) = range {
                let requested_first = params["first_line"].as_u64().unwrap_or(1).max(first);
                let requested_last = params["last_line"].as_u64().unwrap_or(last).min(last);
                if requested_first > requested_last {
                    return Err("LOG.ROW_RANGE_OUTSIDE_SCOPE".into());
                }
                params["first_line"] = json!(requested_first);
                params["last_line"] = json!(requested_last);
            }
        }
        let method = method.to_owned();
        let spec = LibrarySpec {
            directory: if include_directory {
                self.directory.clone()
            } else {
                None
            },
            files,
        };
        let store = self.store.clone();
        let budget = self.budget_mib.clamp(64, 4096) * 1024 * 1024;
        let (tx, rx) = crossbeam_channel::bounded(1);
        let permit = worker_permit()?;
        std::thread::Builder::new().name("wiparse-local-log-analysis".into()).spawn(move||{
            let _permit=permit;
   let work=||->Result<Value,String>{
    if method=="log.files"{
     let mut inventory=log_analysis::discover(&spec,&cancel)?;if let Some(source)=memory.clone(){inventory.sources.push(source);}let offset=params["offset"].as_u64().unwrap_or(0) as usize;let limit=params["limit"].as_u64().unwrap_or(16) as usize;
     let next=(offset+limit).min(inventory.sources.len());
     return Ok(json!({"files":inventory.sources.iter().skip(offset).take(limit).collect::<Vec<_>>(),"total_files":inventory.sources.len(),"next_offset":if next<inventory.sources.len(){Some(next)}else{None},"inventory_complete":!inventory.incomplete,"issues":inventory.issues,"extensions":"log/txt/jsonl/csv and uncompressed rotated logs, plus opened file tabs","raw_files_uploaded":false}));
    }
    let (id,job)=if let Some(id)=params["cursor"].as_str(){
     let job=store.lock().map_err(|_|"LOG.STORE_UNAVAILABLE")?.jobs.iter().find(|(key,_)|key==id).map(|(_,job)|job.clone()).ok_or("LOG.CURSOR_EXPIRED: begin a new analysis")?;(id.to_owned(),job)
    }else{
     let mut inventory=log_analysis::discover(&spec,&cancel)?;if let Some(source)=memory.clone(){inventory.sources.push(source);}let query:Query=serde_json::from_value(params.clone()).map_err(|e|e.to_string())?;
     let job=Arc::new(Mutex::new(Scan::new(inventory,query)?));let id=uuid::Uuid::new_v4().to_string();
     {let mut cache=store.lock().map_err(|_|"LOG.STORE_UNAVAILABLE")?;while cache.jobs.len()>=4{cache.jobs.pop_front();}cache.jobs.push_back((id.clone(),job.clone()));}(id,job)
    };
    let mut job=job.lock().map_err(|_|"LOG.JOB_UNAVAILABLE")?;job.run_chunk(budget,Duration::from_secs(15),&cancel)?;
    if cancel.load(Ordering::Acquire){return Err("LOG.ANALYSIS_CANCELLED".into());}
    let mut result=job.report();result["query_id"]=json!(id);result["continuation"]=if job.finished(){Value::Null}else{json!(id)};drop(job);
    store.lock().map_err(|_|"LOG.STORE_UNAVAILABLE")?.last=Some(result.clone());Ok(result)
   };
   let reply=match work(){Ok(data)=>crate::backend::invoke_ok(&method,data),Err(error)=>crate::backend::invoke_err(&method,&error)};
   let _=tx.send(reply);ctx.request_repaint();
  }).map_err(|e|e.to_string())?;
        Ok(rx)
    }
}
pub(super) fn readable(result: &Value, lang: Lang) -> String {
    let data = &result["data"];
    if result["ok"] == false {
        return format!(
            "{}：{}",
            label(lang, "本地处理失败", "Local analysis failed"),
            result["error"]
        );
    }
    if data.get("coverage").is_none() {
        return format!(
            "{}：{}\n\n{}",
            label(lang, "发现日志文件", "Log files found"),
            data["total_files"],
            label(
                lang,
                "AI可使用文件ID筛选来源，或使用空来源列表扫描全部。",
                "Use file IDs to select sources, or an empty list to scan all."
            )
        );
    }
    let coverage = &data["coverage"];
    let mut text = format!(
        "### {}\n\n{}：{} / {} · {}：{} · {}：{}\n\n{}\n",
        label(lang, "本地日志分析", "Local log analysis"),
        label(lang, "文件", "Files"),
        coverage["files_finished"],
        coverage["files"],
        label(lang, "已读行", "Lines read"),
        coverage["lines_read"],
        label(lang, "匹配行", "Matched lines"),
        coverage["matched_lines"],
        if coverage["complete"] == true {
            label(lang, "已完成当前范围扫描。", "Requested scope scanned.")
        } else if coverage["finished"] == false {
            label(
                lang,
                "尚未完成，可继续扫描；以下统计为累计已读部分。",
                "Not finished. Continue scanning; statistics cover the cumulative portion read.",
            )
        } else {
            label(
                lang,
                "存在未读/变化/编码异常，不能视为完整分析，请查看覆盖信息。",
                "Coverage has gaps or changed/invalid input; not a complete analysis.",
            )
        }
    );
    if let Some(missing) = coverage["preceding_rows_unavailable"]
        .as_u64()
        .filter(|n| *n > 0)
    {
        text += &format!(
            "\n{}：{missing}；{}\n",
            label(
                lang,
                "已淘汰且不在本次快照内的历史行",
                "Earlier rows unavailable in this snapshot"
            ),
            label(
                lang,
                "完整历史请选择日志库中的落盘文件。",
                "Select the recorded file in Local logs for full history."
            )
        );
    }
    text += label(
        lang,
        "\n关键词/数值命中仅作为分析线索，不是测试判定。\n",
        "\nKeyword/numeric hits are analysis clues, not test verdicts.\n",
    );
    if let Some(counts) = data["keyword_line_counts_before_filter"].as_array() {
        for count in counts {
            text += &format!(
                "\n- `{}`: {}",
                count["keyword"].as_str().unwrap_or(""),
                count["lines"]
            );
        }
    }
    if let Some(groups) = data["top_tracked_groups"].as_array() {
        for group in groups {
            text += &format!(
                "\n- {} × `{}`",
                group["statistics"]["count"],
                group["template"].as_str().unwrap_or("")
            );
        }
    }
    if let Some(numbers) = data["numeric_matched_lines"].as_array() {
        for number in numbers {
            text += &format!(
                "\n- {}：{}；min {}，max {}，mean {}；↓ {} / ↑ {}",
                number["field"].as_str().unwrap_or("field"),
                number["statistics"]["count"],
                number["statistics"]["min"],
                number["statistics"]["max"],
                number["statistics"]["mean"],
                number["statistics"]["below_min"],
                number["statistics"]["above_max"]
            );
        }
    }
    if let Some(examples) = data["examples"].as_array() {
        for example in examples {
            text += &format!(
                "\n\n**{} · {} {}**\n\n```text\n{}\n```",
                example["file"].as_str().unwrap_or("log"),
                label(lang, "行", "line"),
                example["line"],
                example["text"].as_str().unwrap_or("")
            );
        }
    }
    if let Some(errors) = data["errors"].as_array() {
        for error in errors {
            text += &format!("\n\n{}", error.as_str().unwrap_or(""));
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_tools_reject_paths_and_malformed_continuations() {
        assert!(validate("log.files", &json!({"path":"C:/secret"})).is_err());
        assert!(validate("log.analyze", &json!({"path":"C:/secret"})).is_err());
        assert!(validate("log.analyze", &json!({"cursor":"../outside"})).is_err());
        assert!(validate(
            "log.analyze",
            &json!({"cursor":uuid::Uuid::new_v4().to_string(),"contains":["new"]})
        )
        .is_err());
        assert!(validate("log.analyze", &json!({"contains":["error"],"samples":0})).is_ok());
        assert_eq!(
            super::super::platform::validate("log.analyze", &json!({})).unwrap(),
            false
        );
    }
    #[test]
    fn local_preview_is_read_only_and_does_not_require_an_ai_provider() {
        let _serial = LOCAL_TEST_LOCK.lock().unwrap();
        let directory =
            std::env::temp_dir().join(format!("wiparse-local-preview-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(directory.join("test.log"), "normal\nERROR timeout\n").unwrap();
        let library = Library {
            directory: Some(directory.clone()),
            ..Library::default()
        };
        let ctx = egui::Context::default();
        let rx = library
            .start(
                "log.analyze",
                library.preview_params(),
                vec![],
                Arc::new(AtomicBool::new(false)),
                ctx,
            )
            .unwrap();
        let reply = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(reply.ok);
        assert_eq!(reply.data["coverage"]["lines_read"], 2);
        assert_eq!(reply.data["coverage"]["matched_lines"], 1);
        assert!(library.status()["last_analysis"]["continuation"].is_null());
        let before = std::fs::read(directory.join("test.log")).unwrap();
        assert_eq!(before, b"normal\nERROR timeout\n");
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn log_scope_consent_and_manual_only_are_checked_before_dispatch() {
        let mut config = AppConfig::default();
        config.log_monitor.save_live_to_disk = false;
        config.log_monitor.open_log_files.clear();
        let mut panel = SerialToolPanel::new(&config, Lang::Zh);
        let mut chat = SerialAiChat::default();
        let ctx = egui::Context::default();
        for scope in [DataScope::None] {
            chat.scope = scope;
            let result = panel.begin_platform_call(
                &ctx,
                &mut chat,
                "log.analyze".into(),
                json!({}),
                "analyse".into(),
                false,
            );
            assert!(result.unwrap_err().contains("DATA_SCOPE"));
            assert!(chat.tool.is_none());
        }
        chat.scope = DataScope::Library;
        assert!(panel
            .begin_platform_call(
                &ctx,
                &mut chat,
                "log.analyze".into(),
                json!({}),
                "analyse".into(),
                true
            )
            .unwrap_err()
            .contains("DATA_SCOPE"));
        panel.tabs[0].append_lines(["not attached to library prompts".into()], false);
        let observation = panel.ai_scoped_observation(DataScope::Library).unwrap();
        assert_eq!(observation["data_included"], false);
        assert_eq!(observation["recent_lines"], json!([]));
    }
    fn preview_scope(panel: &mut SerialToolPanel, chat: &mut SerialAiChat, params: Value) -> Value {
        chat.tool_round = 0;
        let ctx = egui::Context::default();
        panel
            .begin_platform_call(
                &ctx,
                chat,
                "log.analyze".into(),
                params,
                "local preview".into(),
                false,
            )
            .unwrap();
        chat.tool.as_mut().unwrap().local_only = true;
        for _ in 0..1000 {
            panel.poll_platform(&ctx, chat, Lang::Zh);
            if chat.tool.is_none() {
                return chat.library.status()["last_analysis"].clone();
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        panic!("local preview did not finish");
    }
    #[test]
    fn default_current_and_explicit_range_scan_all_retained_rows_without_raw_upload() {
        let _serial = LOCAL_TEST_LOCK.lock().unwrap();
        let mut config = AppConfig::default();
        config.log_monitor.save_live_to_disk = false;
        config.log_monitor.open_log_files.clear();
        let mut panel = SerialToolPanel::new(&config, Lang::Zh);
        let mut chat = SerialAiChat::default();
        panel.tabs[0].append_lines(
            (1..=6000).map(|i| {
                if i == 1001 || i == 6000 {
                    format!("ERROR {i}")
                } else {
                    format!("normal {i}")
                }
            }),
            false,
        );
        assert_eq!(chat.scope, DataScope::Current);
        let report = preview_scope(
            &mut panel,
            &mut chat,
            json!({"contains":["ERROR"],"samples":2}),
        );
        assert_eq!(report["coverage"]["lines_read"], 5000);
        assert_eq!(report["coverage"]["matched_lines"], 2);
        assert_eq!(report["coverage"]["preceding_rows_unavailable"], 1000);
        let observation = panel.ai_scoped_observation(DataScope::Current).unwrap();
        assert_eq!(observation["recent_lines"], json!([]));
        assert_eq!(observation["selected_text"], "");
        chat.scope = DataScope::Range;
        chat.range_first = 1001;
        chat.range_last = 1010;
        let report = preview_scope(
            &mut panel,
            &mut chat,
            json!({"first_line":1,"last_line":6000,"contains":["ERROR"],"context":2}),
        );
        assert_eq!(report["scope"]["first_line"], 1001);
        assert_eq!(report["scope"]["last_line"], 1010);
        assert_eq!(report["coverage"]["matched_lines"], 1);
        assert_eq!(report["coverage"]["lines_in_range"], 10);
        assert!(panel
            .begin_platform_call(
                &egui::Context::default(),
                &mut chat,
                "log.analyze".into(),
                json!({"first_line":2000}),
                "outside".into(),
                false
            )
            .unwrap_err()
            .contains("OUTSIDE_SCOPE"));
    }
    #[test]
    fn current_disk_scope_ignores_other_tabs_and_directory_and_scans_past_100_lines() {
        let _serial = LOCAL_TEST_LOCK.lock().unwrap();
        let root = std::env::temp_dir().join(format!("wiparse-scope-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("current.log");
        std::fs::write(
            &path,
            format!("ERROR first\n{}ERROR last\n", "normal\n".repeat(1000)),
        )
        .unwrap();
        std::fs::write(root.join("excluded.log"), "ERROR private\n").unwrap();
        let mut config = AppConfig::default();
        config.log_monitor.save_live_to_disk = false;
        config.log_monitor.open_log_files.clear();
        let mut panel = SerialToolPanel::new(&config, Lang::Zh);
        panel
            .tabs
            .push(LogTabPage::file_tab_empty(path.display().to_string()));
        panel.active_tab = 1;
        let mut chat = SerialAiChat::default();
        chat.library.directory = Some(root.clone());
        let report = preview_scope(
            &mut panel,
            &mut chat,
            json!({"contains":["ERROR"],"samples":2}),
        );
        assert_eq!(report["coverage"]["files"], 1);
        assert_eq!(report["coverage"]["lines_read"], 1002);
        assert_eq!(report["coverage"]["matched_lines"], 2);
        assert_eq!(report["coverage"]["complete"], true);
        std::fs::remove_dir_all(root).unwrap();
    }
}
