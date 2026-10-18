//! Schema-driven, business-neutral workspace. All disk/plugin work is out of process.
use super::*;
mod form_layout;
mod project_matrix;
use form_layout::{caption, enum_caption, ui_spec};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};

static WORKERS: AtomicUsize = AtomicUsize::new(0);
const HUB_CONTENT_MAX_W: f32 = 1680.0;
const HUB_WIDE_MIN_W: f32 = 1120.0;
const HUB_LIST_W: f32 = 320.0;
const HUB_CHECKS_W: f32 = 320.0;
const HUB_PANEL_GAP: f32 = 12.0;
#[derive(Default)]
pub(super) struct Workspace {
    key: String,
    snapshot: Value,
    document: Value,
    original: Value,
    original_revision: u64,
    original_sha: String,
    result_ref: Option<String>,
    pub(super) api_binding: Option<Value>,
    selected: HashSet<String>,
    current: String,
    query: String,
    status_filter: String,
    errors: Value,
    error: String,
    tab: usize,
    checks_tab: bool,
    form_section: usize,
    new_id: String,
    rx: Option<Receiver<(String, Result<Value, String>)>>,
    last_poll: Option<Instant>,
    reload: bool,
    pending: Option<(String, Value)>,
    focus_path: Option<String>,
    follow: bool,
    file: String,
    import_strategy: usize,
    matrix_signature: Option<(std::time::SystemTime, u64)>,
    sort: usize,
    batch_tags: String,
    config_status: String,
    list_epoch: u64,
    list_cache_key: String,
    list_cache: std::sync::Arc<Vec<Value>>,
    matrix_cache_key: String,
    matrix_cache: std::sync::Arc<Vec<Value>>,

    normalized: Value,
    warnings: Value,
    close_pending: bool,
    last_edit: Option<Instant>,
    validation_candidate: Value,
    last_form_value: Value,
    draft_operation: String,
    api_epoch: u64,
    parameter_fingerprint: String,
    file_metadata: Value,
    drafts: std::collections::HashMap<String, Value>,
    focus_new_row: bool,
    matrix_current: String,
    result_details: Value,
    result_key: String,
    project_document: Value,
    project_original: Value,
    project_migration: Value,
}
impl Workspace {
    pub(super) fn dirty(&self) -> bool {
        (!self.current.is_empty() && self.document != self.original)
            || self.project_document != self.project_original
    }
    pub(super) fn binding(&self) -> Value {
        if let Some(binding) = &self.api_binding {
            return binding.clone();
        }
        let p = &self.snapshot["preflight"];
        let mut selected: Vec<_> = self.selected.iter().cloned().collect();
        selected.sort();
        json!({"config_manifest_sha256":p["manifest_sha256"],"preflight_revision":p["revision"],"selected_item_ids":selected,"filters":{"query":self.query}})
    }
}
fn run_worker(node: String, request: Value) -> Result<Value, String> {
    if WORKERS
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
            (n < 4).then_some(n + 1)
        })
        .is_err()
    {
        return Err("CONFIG.BUSY".into());
    }
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            WORKERS.fetch_sub(1, Ordering::Release);
        }
    }
    let _guard = Guard;
    let mut cmd = Command::new(node);
    cmd.arg(project_path("test-tools/hub-workspace.mjs"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let mut child = cmd.spawn().map_err(|_| "CONFIG.WORKER_UNAVAILABLE")?;
    let bytes = serde_json::to_vec(&request).map_err(|_| "CONFIG.INVALID_REQUEST")?;
    if bytes.len() > 2 * 1024 * 1024 {
        let _ = child.kill();
        let _ = child.wait();
        return Err("CONFIG.REQUEST_TOO_LARGE".into());
    }
    let mut stdin = child.stdin.take().ok_or("CONFIG.WORKER_PIPE")?;
    let writer = thread::spawn(move || {
        let result = stdin.write_all(&bytes);
        drop(stdin);
        result
    });
    let stdout = child.stdout.take().ok_or("CONFIG.WORKER_PIPE")?;
    let reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take(6 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < Duration::from_secs(60) => {
                thread::sleep(Duration::from_millis(20))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("CONFIG.WORKER_TIMEOUT".into());
            }
        }
    }
    let _ = writer.join();
    let bytes = reader
        .join()
        .map_err(|_| "CONFIG.WORKER_FAILED")?
        .map_err(|_| "CONFIG.WORKER_FAILED")?;
    if bytes.len() > 6 * 1024 * 1024 {
        return Err("CONFIG.RESPONSE_TOO_LARGE".into());
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| "CONFIG.INVALID_RESPONSE")?;
    Ok(value)
}
impl TestToolPanel {
    pub(super) fn workspace_run_gate(&self, preflight: bool) -> Result<(), String> {
        if self.workspace.dirty() {
            return Err("CONFIG.UNSAVED_CHANGES".into());
        }
        if self.workspace.error.starts_with("PLUGIN.")
            || self.workspace.error.starts_with("SCHEMA.UNSUPPORTED")
        {
            return Err(self.workspace.error.clone());
        }
        if !preflight {
            let binding = self.workspace.binding();
            if binding["config_manifest_sha256"].as_str().is_none()
                || binding["preflight_revision"].as_str().is_none()
            {
                return Err("PREFLIGHT.REQUIRED".into());
            }
            if self.workspace.api_binding.is_none()
                && self.workspace.snapshot["preflight"]["valid"] != true
            {
                return Err("PREFLIGHT.STALE".into());
            }
        }
        Ok(())
    }
    pub fn workspace_close_guard(&mut self, ctx: &egui::Context, lang: Lang) {
        self.workspace_poll(ctx);
        self.workspace_nav_dialog(ctx, lang);
        if ctx.input(|i| i.viewport().close_requested()) && self.workspace.dirty() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.workspace.close_pending = true;
        }
        if !self.workspace.close_pending {
            return;
        }
        if !self.workspace.dirty() && self.workspace.rx.is_none() {
            self.workspace.close_pending = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        let zh = matches!(lang, Lang::Zh);
        egui::Window::new(if zh {
            "关闭前保存配置？"
        } else {
            "Save configuration before closing?"
        })
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            if !self.workspace.error.is_empty() {
                ui.colored_label(egui::Color32::LIGHT_RED, &self.workspace.error);
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        self.workspace.rx.is_none(),
                        egui::Button::new(if zh {
                            "保存并关闭"
                        } else {
                            "Save and close"
                        }),
                    )
                    .clicked()
                {
                    if self.workspace.project_document != self.workspace.project_original {
                        self.workspace_send("project.save", self.project_save_params());
                    } else {
                        self.workspace_send("config.save", self.workspace_save_params(false));
                    }
                }
                if ui
                    .button(if zh {
                        "放弃并关闭"
                    } else {
                        "Discard and close"
                    })
                    .clicked()
                {
                    self.workspace_discard();
                    self.workspace.close_pending = false;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                if ui.button(if zh { "取消" } else { "Cancel" }).clicked() {
                    self.workspace.close_pending = false;
                }
            });
        });
    }
    fn workspace_request(
        &self,
        plugin: &PluginInfo,
        method: &str,
        params: Value,
        internal: bool,
    ) -> Value {
        json!({"context":{"pluginDir":plugin.dir,"dataRoot":self.data_root_resolved(),"project":self.effective_project_id(),"testId":self.hub_test_id_for(plugin),"params":self.param_values,"approvedRoots":self.approved_config_roots,"protectedRoots":[resolve_dir(self.plugins_dir.trim()),self.resolve_marketplace_root()]},"method":method,"params":params,"internal":internal})
    }
    /// API replies are delivered from a bounded background worker, never blocking Stop/UI.
    pub fn api_workspace(&self, req: crate::backend::PendingRequest) {
        let id = req
            .params
            .get("plugin")
            .or_else(|| req.params.get("plugin_id"))
            .and_then(Value::as_str)
            .or(self.selected.as_deref());
        let Some(plugin) = self.plugins.iter().find(|p| Some(p.id.as_str()) == id) else {
            let _ = req.reply.try_send(crate::backend::invoke_err(
                &req.method,
                "CONFIG.PLUGIN_NOT_FOUND",
            ));
            return;
        };
        let mut request = self.workspace_request(plugin, &req.method, req.params.clone(), false);
        if self.selected.as_deref() != Some(&plugin.id) {
            request["context"]["params"] = json!({});
            request["context"]["testId"] = json!(plugin.report_name_or_fallback());
        }
        if let Some(overrides) = req.params.get("run_params") {
            request["context"]["params"] = overrides.clone();
        }
        let node = resolve_command(self.node_path.trim());
        let epoch = self.hub_api_epoch.clone();
        thread::spawn(move || {
            if Instant::now() > req.deadline {
                let _ = req.reply.try_send(crate::backend::invoke_err(
                    &req.method,
                    "CONFIG.REQUEST_EXPIRED",
                ));
                return;
            }
            let result = run_worker(node, request);
            let reply = match result {
                Ok(v) if v["ok"] == true => {
                    crate::backend::invoke_ok(&req.method, v["data"].clone())
                }
                Ok(v) => crate::backend::InvokeReply {
                    ok: false,
                    cmd: req.method.clone(),
                    data: v["error"].clone(),
                    error: Some(
                        v["error"]["code"]
                            .as_str()
                            .unwrap_or("CONFIG.OPERATION_FAILED")
                            .into(),
                    ),
                },
                Err(e) => crate::backend::invoke_err(&req.method, &e),
            };
            if reply.ok
                && [
                    "ui.test_tool.config.save",
                    "ui.test_tool.config.archive",
                    "ui.test_tool.config.restore",
                    "ui.test_tool.config.import",
                    "ui.test_tool.config.batch",
                    "ui.test_tool.project.save",
                    "ui.test_tool.project.migration.commit",
                ]
                .contains(&req.method.as_str())
                && req.params["validate_only"] != true
            {
                epoch.fetch_add(1, Ordering::Release);
            }
            let _ = req.reply.try_send(reply);
        });
    }
    fn workspace_send(&mut self, method: &str, params: Value) {
        if self.workspace.rx.is_some() {
            return;
        }
        if method == "config.create" || method == "config.duplicate" {
            let id = params[if method == "config.create" {
                "config_id"
            } else {
                "new_id"
            }]
            .as_str()
            .unwrap_or("");
            if self.workspace.snapshot["items"]
                .as_array()
                .is_some_and(|rows| rows.len() >= 500)
            {
                self.workspace.error = "CONFIG.REPOSITORY_LIMIT".into();
                return;
            }
            if self.workspace.snapshot["items"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|r| {
                    r["id"]
                        .as_str()
                        .is_some_and(|key| key.eq_ignore_ascii_case(id))
                })
            {
                self.workspace.error = "CONFIG.ID_CONFLICT".into();
                return;
            }
        }
        let Some(p) = self.selected_plugin() else {
            return;
        };
        let request = self.workspace_request(p, method, params, true);
        let node = resolve_command(self.node_path.trim());
        let method = method.to_owned();
        let (tx, rx) = mpsc::channel();
        self.workspace.rx = Some(rx);
        let matrix_file = self.workspace.snapshot["matrix_file"]
            .as_str()
            .map(PathBuf::from);
        let prior = self.workspace.matrix_signature;
        thread::spawn(move || {
            if method == "matrix.get" {
                if let Some(file) = matrix_file.as_ref() {
                    if let Ok(meta) = fs::metadata(&file) {
                        if let Ok(at) = meta.modified() {
                            let signature = (at, meta.len());
                            if Some(signature) == prior {
                                let _ =
                                    tx.send(("matrix.unchanged".into(), Ok(json!({"ok":true}))));
                                return;
                            }
                        }
                    }
                }
            }
            let mut result = run_worker(node, request);
            if method == "matrix.get" || method == "workspace.open" {
                if let Ok(v) = &mut result {
                    if let Some(file) = v["data"]["matrix_file"]
                        .as_str()
                        .map(PathBuf::from)
                        .or(matrix_file)
                    {
                        if let Ok(meta) = fs::metadata(file) {
                            if let Ok(at) = meta.modified() {
                                v["_signature"] = json!([
                                    at.duration_since(std::time::UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_nanos()
                                        .to_string(),
                                    meta.len()
                                ]);
                            }
                        }
                    }
                }
            }
            let _ = tx.send((method, result));
        });
    }
    fn workspace_poll(&mut self, ctx: &egui::Context) {
        let result = self.workspace.rx.as_ref().and_then(|rx| rx.try_recv().ok());
        if let Some((method, result)) = result {
            self.workspace.rx = None;
            match result {
                Ok(v) if v["ok"] == true => {
                    if let (Some(nanos), Some(size)) = (
                        v["_signature"][0]
                            .as_str()
                            .and_then(|s| s.parse::<u128>().ok()),
                        v["_signature"][1].as_u64(),
                    ) {
                        self.workspace.matrix_signature = Some((
                            std::time::UNIX_EPOCH
                                + Duration::new(
                                    (nanos / 1_000_000_000) as u64,
                                    (nanos % 1_000_000_000) as u32,
                                ),
                            size,
                        ));
                    }
                    self.workspace.error.clear();
                    let data = v["data"].clone();
                    match method.as_str() {
                        "workspace.open" => {
                            let initial = self.workspace.snapshot.is_null();
                            if initial {
                                if let Some(rows) = data["items"].as_array() {
                                    self.workspace.selected = rows
                                        .iter()
                                        .filter(|r| r["enabled"] == true)
                                        .filter_map(|r| r["id"].as_str().map(str::to_owned))
                                        .collect();
                                }
                            }
                            self.workspace.list_epoch = self.workspace.list_epoch.wrapping_add(1);
                            if self.workspace.project_document == self.workspace.project_original {
                                self.workspace.project_document =
                                    data["project_config"]["value"].clone();
                                self.workspace.project_original =
                                    self.workspace.project_document.clone();
                            }
                            self.workspace.snapshot = data;
                            self.workspace.merge_drafts();
                            self.workspace.reload = false;
                            if !self.workspace.current.is_empty()
                                && self.workspace.document == self.workspace.original
                                && !self.workspace.drafts.contains_key(&self.workspace.current)
                            {
                                let changed = self.workspace.snapshot["items"]
                                    .as_array()
                                    .into_iter()
                                    .flatten()
                                    .find(|r| r["id"] == self.workspace.current)
                                    .is_some_and(|r| {
                                        r["sha256"].as_str()
                                            != Some(self.workspace.original_sha.as_str())
                                    });
                                if changed {
                                    self.workspace_send(
                                        "config.get",
                                        json!({"config_id":self.workspace.current}),
                                    );
                                }
                            }
                        }
                        "project.save" => {
                            self.workspace.project_document = data["value"].clone();
                            self.workspace.project_original =
                                self.workspace.project_document.clone();
                            self.workspace.snapshot["project_config"]["value"] =
                                data["value"].clone();
                            for key in ["revision", "sha256", "schema_hash", "saved"] {
                                self.workspace.snapshot["project_config"][key] = data[key].clone();
                            }
                            self.workspace.snapshot["preflight"]["valid"] = json!(false);
                            self.workspace.api_binding = None;
                            self.workspace.reload = true;
                        }
                        "project.migration.preview" => {
                            self.workspace.project_migration = data;
                        }
                        "project.migration.commit" => {
                            self.workspace.current.clear();
                            self.workspace.document = Value::Null;
                            self.workspace.original = Value::Null;
                            self.workspace.project_migration = Value::Null;
                            self.workspace.reload = true;
                            self.workspace.snapshot["preflight"]["valid"] = json!(false);
                            self.workspace.api_binding = None;
                        }
                        "result.get" => {
                            let expected = format!(
                                "{}:{}:{}",
                                data["config_id"], data["run_id"], data["attempt"]
                            );
                            if data["config_id"] == self.workspace.matrix_current
                                && expected == self.workspace.result_key
                            {
                                self.workspace.result_details = data;
                            }
                        }
                        "config.validate" => {
                            if self.workspace.validation_candidate == self.workspace.document {
                                self.workspace.errors = data["errors"].clone();
                            }
                        }
                        "matrix.unchanged" => {}
                        "matrix.get" => {
                            self.workspace.snapshot["matrix"] = data;
                        }
                        "run.control" => {
                            self.workspace.snapshot["matrix"]["control"] = data;
                            self.workspace.matrix_signature = None;
                        }
                        "preflight.state" => {
                            self.workspace.snapshot["preflight"] = data;
                        }
                        "config.inspect_file" => {
                            let pointer = data["pointer"].as_str().unwrap_or("");
                            if data["config_id"] == self.workspace.current
                                && self.workspace.document.pointer(pointer)
                                    == Some(&data["expected_value"])
                            {
                                insert_field(
                                    &mut self.workspace.document,
                                    pointer,
                                    data["path"].clone(),
                                );
                                if let Some(hash_field) = data["hash_field"].as_str() {
                                    insert_field(
                                        &mut self.workspace.document,
                                        &format!("/{}", hash_field.replace('.', "/")),
                                        data["sha256"].clone(),
                                    );
                                }
                                self.workspace.file_metadata = data;
                                self.workspace.snapshot["preflight"]["valid"] = json!(false);
                                self.workspace.api_binding = None;
                            } else {
                                self.workspace.error = "CONFIG.FILE_SELECTION_STALE".into();
                            }
                        }
                        "config.get" | "config.create" | "config.duplicate" => {
                            self.workspace.file_metadata = Value::Null;
                            if data["draft"] == true {
                                let id = data["config_id"].as_str().unwrap_or("").to_owned();
                                let row = json!({"id":id,"name":data["value"]["name"],"tags":data["value"]["tags"],"enabled":false,"draft":true,"status":"draft","revision":0,"sha256":"","order":data["value"]["order"]});
                                self.workspace.drafts.insert(id, row);
                                self.workspace.merge_drafts();
                                self.workspace.focus_new_row = true;
                                self.workspace.query.clear();
                                self.workspace.config_status.clear();
                            }
                            self.workspace.draft_operation = if method == "config.duplicate" {
                                "duplicate".into()
                            } else {
                                "save".into()
                            };
                            self.workspace.current =
                                data["config_id"].as_str().unwrap_or("").into();
                            self.workspace.document = data["value"].clone();
                            self.workspace.last_form_value = self.workspace.document.clone();
                            self.workspace.last_edit = None;
                            self.workspace.original = if method == "config.get" {
                                data["value"].clone()
                            } else {
                                Value::Null
                            };
                            self.workspace.original_revision =
                                data["revision"].as_u64().unwrap_or(0);
                            self.workspace.original_sha =
                                data["sha256"].as_str().unwrap_or("").into();
                            self.workspace.errors = data["errors"].clone();
                        }
                        "config.save" => {
                            self.workspace.errors = Value::Null;
                            self.workspace.normalized = data["normalized_value"].clone();
                            self.workspace.warnings = data["warnings"].clone();
                            if data["valid"] == true && data["value"].is_object() {
                                let next = data["config_id"].as_str().unwrap_or("");
                                self.workspace.drafts.remove(&self.workspace.current);
                                if let Some(rows) = self.workspace.snapshot["items"].as_array_mut()
                                {
                                    rows.retain(|r| r["id"] != self.workspace.current);
                                    rows.push(json!({"id":next,"name":data["value"]["name"],"enabled":data["value"]["enabled"],"tags":data["value"]["tags"],"order":data["value"]["order"],"status":"valid","revision":data["revision"],"sha256":data["sha256"]}));
                                }
                                self.workspace.list_epoch =
                                    self.workspace.list_epoch.wrapping_add(1);
                                if self.workspace.selected.remove(&self.workspace.current) {
                                    self.workspace.selected.insert(next.to_owned());
                                }
                                self.workspace.current =
                                    data["config_id"].as_str().unwrap_or("").into();
                                self.workspace.document = data["value"].clone();
                                self.workspace.original = data["value"].clone();
                                self.workspace.original_revision =
                                    data["revision"].as_u64().unwrap_or(0);
                                self.workspace.original_sha =
                                    data["sha256"].as_str().unwrap_or("").into();
                            }
                            if data["value"].is_object() {
                                self.workspace.snapshot["preflight"]["valid"] = json!(false);
                                self.workspace.api_binding = None;
                            }
                            self.workspace.reload = true;
                        }
                        _ => {
                            if method == "config.archive" {
                                self.workspace.current.clear();
                                self.workspace.document = Value::Null;
                                self.workspace.original = Value::Null;
                            }
                            self.workspace.snapshot["preflight"]["valid"] = json!(false);
                            self.workspace.api_binding = None;
                            self.workspace.reload = true;
                        }
                    }
                }
                Ok(v) => {
                    self.workspace.error = v["error"]["code"]
                        .as_str()
                        .unwrap_or("CONFIG.OPERATION_FAILED")
                        .into();
                    self.workspace.errors = v["error"]["errors"].clone();
                }
                Err(e) => self.workspace.error = e,
            }
        }
        if self.workspace.rx.is_some() {
            ctx.request_repaint_after(Duration::from_millis(30));
        }
    }
    fn workspace_nav_dialog(&mut self, ctx: &egui::Context, lang: Lang) {
        let zh = matches!(lang, Lang::Zh);
        let text = |cn, en| if zh { cn } else { en };
        if self.workspace.pending.is_some() {
            egui::Window::new(text("保存当前修改？", "Save changes?"))
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(text(
                        "切换前请保存或放弃修改。",
                        "Save or discard before navigating.",
                    ));
                    ui.horizontal(|ui| {
                        if ui.button(text("保存", "Save")).clicked() {
                            if self.workspace.project_document != self.workspace.project_original {
                                self.workspace_send("project.save", self.project_save_params());
                            } else {
                                self.workspace_send(
                                    "config.save",
                                    self.workspace_save_params(false),
                                );
                            }
                        }
                        if ui
                            .button(text("放弃并继续", "Discard and continue"))
                            .clicked()
                        {
                            self.workspace_discard();
                            if let Some((m, p)) = self.workspace.pending.take() {
                                self.workspace_navigate(&m, p);
                            }
                        }
                        if ui.button(text("取消", "Cancel")).clicked() {
                            self.workspace.pending = None;
                        }
                    });
                });
            if !self.workspace.dirty() && self.workspace.rx.is_none() {
                if let Some((m, p)) = self.workspace.pending.take() {
                    self.workspace_navigate(&m, p);
                }
            }
        }
    }
    pub(super) fn workspace_switch(&mut self, kind: &str, id: String) {
        self.workspace.pending = Some((kind.into(), json!({"id":id})));
    }
    fn workspace_discard(&mut self) {
        self.workspace.project_document = self.workspace.project_original.clone();
        if self.workspace.original.is_null() {
            self.workspace.drafts.remove(&self.workspace.current);
            if let Some(rows) = self.workspace.snapshot["items"].as_array_mut() {
                rows.retain(|r| r["id"] != self.workspace.current);
            }
            self.workspace.list_epoch = self.workspace.list_epoch.wrapping_add(1);
        }
        self.workspace.document = self.workspace.original.clone();
        if self.workspace.original.is_null() {
            self.workspace.current.clear();
        }
        self.workspace.merge_drafts();
        self.workspace.snapshot["total"] = json!(self.workspace.snapshot["items"]
            .as_array()
            .map_or(0, Vec::len));
    }
    fn workspace_save_params(&self, validate_only: bool) -> Value {
        json!({"config_id":self.workspace.current,"value":self.workspace.document,"expected_revision":self.workspace.original_revision,"expected_sha256":self.workspace.original_sha,"validate_only":validate_only,"operation":self.workspace.draft_operation})
    }
    fn workspace_navigate(&mut self, method: &str, params: Value) {
        if self.workspace.dirty() {
            self.workspace.pending = Some((method.into(), params));
        } else if method == "__plugin" {
            if let Some(id) = params["id"].as_str() {
                self.select_plugin(id.into());
            }
        } else if method == "__project" {
            if let Some(id) = params["id"].as_str() {
                self.set_project_id(id.into());
            }
        } else if method == "__list" {
            self.workspace.current.clear();
            self.workspace.document = Value::Null;
            self.workspace.original = Value::Null;
        } else {
            self.workspace_send(method, params);
        }
    }
    pub(super) fn paint_workspace(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        self.workspace_poll(ui.ctx());
        let Some(plugin) = self.selected_plugin().cloned() else {
            return;
        };
        let key = format!(
            "{}:{}:{}",
            plugin.id,
            self.effective_project_id(),
            self.hub_report_name(&plugin)
        );
        let ordered: std::collections::BTreeMap<_, _> = self.param_values.iter().collect();
        let fingerprint = serde_json::to_string(&ordered).unwrap_or_default();
        if self.workspace.key != key && !self.workspace.dirty() && self.workspace.rx.is_none() {
            self.workspace = Workspace {
                key,
                parameter_fingerprint: fingerprint.clone(),
                ..Default::default()
            };
            self.workspace_send("workspace.open", json!({}));
        }
        if self.workspace.parameter_fingerprint != fingerprint
            && !self.workspace.dirty()
            && self.workspace.rx.is_none()
        {
            self.workspace.parameter_fingerprint = fingerprint;
            self.workspace.reload = true;
            self.workspace.api_binding = None;
        }
        let epoch = self.hub_api_epoch.load(Ordering::Acquire);
        if self.workspace.api_epoch != epoch {
            self.workspace.api_epoch = epoch;
            self.workspace.reload = true;
        }
        if self.workspace.rx.is_none() && self.workspace.reload {
            self.workspace_send("workspace.open", json!({}));
        }
        let zh = matches!(lang, Lang::Zh);
        let text = |cn, en| if zh { cn } else { en };
        Frame::NONE
            .fill(tokens.surface_bg)
            .corner_radius(CornerRadius::same(ui_theme::RADIUS_CARD))
            .inner_margin(Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = ui_theme::SPACE_MD;
                    ui.selectable_value(&mut self.workspace.tab, 0, text("配置", "Configurations"));
                    ui.selectable_value(
                        &mut self.workspace.tab,
                        1,
                        text("测试矩阵", "Test matrix"),
                    );
                    ui.selectable_value(
                        &mut self.workspace.tab,
                        2,
                        text("本轮参数", "Run parameters"),
                    );
                    ui.selectable_value(&mut self.workspace.tab, 3, text("运行日志", "Run log"));
                    if self.workspace.snapshot["project_config"].is_object() {
                        ui.selectable_value(
                            &mut self.workspace.tab,
                            4,
                            text("工程设置", "Project settings"),
                        );
                    }
                    ui.separator();
                    if ui.button(text("刷新", "Refresh")).clicked() {
                        self.workspace.reload = true;
                    }
                    if self.workspace.rx.is_some() {
                        ui.spinner();
                    }
                    if self.workspace.dirty() {
                        ui.label(
                            egui::RichText::new(text("有未保存的修改", "Unsaved changes"))
                                .color(tokens.accent),
                        );
                    }
                });
            });
        self.paint_workspace_execution(ui, lang, tokens);
        ui.add_space(ui_theme::SPACE_MD);
        if !self.workspace.error.is_empty() {
            ui.colored_label(egui::Color32::LIGHT_RED, &self.workspace.error);
        }
        if self.workspace.tab == 2 {
            if self.workspace.dirty() {
                ui.label(text(
                    "请先保存或放弃配置修改，再调整本轮参数。",
                    "Save or discard configuration changes before editing run parameters.",
                ));
                return;
            }
            ui.collapsing(
                text(
                    "已授权的配置和证据目录",
                    "Approved configuration and evidence roots",
                ),
                |ui| {
                    let mut remove = None;
                    for (i, root) in self.approved_config_roots.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(root);
                            if ui
                                .add_enabled(self.job.is_none(), egui::Button::new("×"))
                                .clicked()
                            {
                                remove = Some(i);
                            }
                        });
                    }
                    if let Some(i) = remove {
                        self.approved_config_roots.remove(i);
                        self.persist_config();
                        self.workspace.reload = true;
                    }
                    if ui
                        .add_enabled(
                            self.job.is_none(),
                            egui::Button::new(text("添加授权目录…", "Add approved root…")),
                        )
                        .clicked()
                    {
                        if let Some(root) = rfd::FileDialog::new().pick_folder() {
                            let root = root.display().to_string();
                            if self.approved_config_roots.len() < 16
                                && !self.approved_config_roots.contains(&root)
                            {
                                self.approved_config_roots.push(root);
                                self.persist_config();
                                self.workspace.reload = true;
                            }
                        }
                    }
                },
            );
            self.paint_runner_config(
                ui,
                lang,
                tokens,
                Some(self.plugins.iter().position(|p| p.id == plugin.id).unwrap()),
            );
            return;
        }
        if self.workspace.tab == 3 {
            self.paint_runner_output(ui, lang, tokens);
            return;
        }
        if self.workspace.snapshot.is_null() {
            ui.label(text(
                "加载配置工作区；若路径未配置，请打开“本轮参数”。",
                "Loading workspace; configure paths under Run parameters if needed.",
            ));
            return;
        }
        if self.workspace.tab == 4 {
            self.paint_project_settings(ui, lang, tokens);
        } else if self.workspace.tab == 1 {
            self.paint_matrix(ui, lang, tokens);
        } else {
            self.paint_config_workspace(ui, lang, tokens);
        }
        if self.job.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(400));
            if self.workspace.rx.is_none()
                && self
                    .workspace
                    .last_poll
                    .is_none_or(|at| at.elapsed() >= Duration::from_millis(400))
            {
                self.workspace.last_poll = Some(Instant::now());
                self.workspace_send("matrix.get", json!({"limit":500}));
            }
        } else if self.workspace.last_poll.take().is_some() {
            self.workspace.reload = true;
        }
    }
    fn paint_workspace_execution(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        let zh = matches!(lang, Lang::Zh);
        let text = |cn, en| if zh { cn } else { en };
        let running = self.job.is_some();
        let busy = self.workspace.rx.is_some();
        let supported =
            self.workspace.snapshot["ui"]["execution_controls"]["pause_mode"] == "between_items";
        let state = self.workspace.snapshot["matrix"]["control"]["state"]
            .as_str()
            .unwrap_or("running")
            .to_owned();
        let run_id = self.workspace.snapshot["matrix"]["run_id"].clone();
        let ready = self.workspace.snapshot["preflight"]["valid"] == true
            && !self.workspace.dirty()
            && !self.workspace.selected.is_empty();
        let mut launch = None;
        let mut action = None;
        let mut stop = false;
        Frame::NONE
            .fill(tokens.surface_bg)
            .corner_radius(6)
            .inner_margin(Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(text("执行控制", "Execution")).strong());
                    let badge = &self.workspace.snapshot["ui"]["execution_badge"];
                    let parameter = badge["parameter"].as_str().unwrap_or("");
                    let value = self
                        .param_values
                        .get(parameter)
                        .map(String::as_str)
                        .unwrap_or_else(|| badge["default"].as_str().unwrap_or(""));
                    let mode = &badge["values"][value];
                    if mode.is_object() {
                        ui.label(RichText::new(localized_title(mode, zh)).color(
                            if mode["tone"] == "warning" {
                                tokens.warning
                            } else {
                                tokens.text_muted
                            },
                        ));
                    }
                    ui.weak(if zh {
                        format!("已选择 {} 项", self.workspace.selected.len())
                    } else {
                        format!("{} selected", self.workspace.selected.len())
                    });
                    ui.separator();
                    if ui
                        .add_enabled(
                            !running
                                && !busy
                                && !self.workspace.dirty()
                                && !self.workspace.selected.is_empty(),
                            egui::Button::new(text("预检", "Preflight")),
                        )
                        .clicked()
                    {
                        launch = Some(true);
                    }
                    if ui
                        .add_enabled(
                            !running && !busy && ready,
                            egui::Button::new(
                                RichText::new(text("▶ 启动测试", "▶ Start test"))
                                    .color(tokens.accent_text),
                            )
                            .fill(tokens.accent),
                        )
                        .on_hover_text(text(
                            "先保存配置并完成预检，再启动所选测试项",
                            "Save and preflight the selected configurations first",
                        ))
                        .clicked()
                    {
                        launch = Some(false);
                    }
                    let paused = state == "paused" || state == "pause_requested";
                    if ui
                        .add_enabled(
                            running && !self.preflight_only && supported && !busy && !paused,
                            egui::Button::new(text("Ⅱ 暂停", "Ⅱ Pause")),
                        )
                        .on_hover_text(text(
                            "完成当前测试项后暂停，不打断采集时序",
                            "Pause after the current item finishes; preserve capture timing",
                        ))
                        .clicked()
                    {
                        action = Some("pause");
                    }
                    if ui
                        .add_enabled(
                            running && supported && !busy && paused,
                            egui::Button::new(text("▶ 继续", "▶ Continue")),
                        )
                        .clicked()
                    {
                        action = Some("resume");
                    }
                    if ui
                        .add_enabled(
                            running,
                            egui::Button::new(
                                RichText::new(text("■ 停止", "■ Stop"))
                                    .color(egui::Color32::LIGHT_RED),
                            ),
                        )
                        .clicked()
                    {
                        stop = true;
                    }
                    ui.separator();
                    let label = if !running {
                        text("空闲", "Idle")
                    } else if self.preflight_only {
                        text("正在预检", "Preflighting")
                    } else {
                        match state.as_str() {
                            "pause_requested" => {
                                text("等待当前项完成后暂停", "Pausing after current item")
                            }
                            "paused" => text("已暂停，可继续或停止", "Paused · continue or stop"),
                            "resume_requested" => text("正在继续", "Resuming"),
                            _ => text("测试运行中", "Running"),
                        }
                    };
                    ui.label(RichText::new(label).color(if state == "paused" {
                        tokens.warning
                    } else {
                        tokens.text_muted
                    }));
                });
            });
        if let Some(preflight) = launch {
            self.preflight_only = preflight;
            if let Err(error) = self.start_selected(lang) {
                self.workspace.error = error;
            }
            self.workspace.matrix_signature = None;
        }
        if let Some(action) = action {
            self.workspace_send("run.control", json!({"action":action,"run_id":run_id}));
        }
        if stop {
            self.stop_job(lang);
        }
        if running {
            ui.ctx().request_repaint_after(Duration::from_millis(400));
            if self.workspace.rx.is_none()
                && self
                    .workspace
                    .last_poll
                    .is_none_or(|at| at.elapsed() >= Duration::from_millis(400))
            {
                self.workspace.last_poll = Some(Instant::now());
                self.workspace_send("matrix.get", json!({"limit":500}));
            }
        }
    }
    fn paint_config_workspace(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        let zh = matches!(lang, Lang::Zh);
        let text = |cn, en| if zh { cn } else { en };
        if self.workspace.rx.is_none()
            && self
                .workspace
                .last_edit
                .is_some_and(|at| at.elapsed() >= Duration::from_millis(600))
            && !self.workspace.current.is_empty()
        {
            self.workspace.last_edit = None;
            self.workspace.validation_candidate = self.workspace.document.clone();
            self.workspace_send("config.validate", json!({"value":self.workspace.document}));
        }
        if self.workspace.project_document != self.workspace.project_original {
            ui.label(if zh {
                "请先保存或放弃工程设置修改，再编辑测试项。"
            } else {
                "Save or discard project settings before editing test items."
            });
            if ui
                .button(if zh {
                    "返回工程设置"
                } else {
                    "Open project settings"
                })
                .clicked()
            {
                self.workspace.tab = 4;
            }
            return;
        }
        let busy = self.workspace.rx.is_some();
        let total = self.workspace.snapshot["total"]
            .as_u64()
            .unwrap_or_else(|| {
                self.workspace.snapshot["items"]
                    .as_array()
                    .map_or(0, |items| items.len() as u64)
            });
        if total == 0 && self.workspace.current.is_empty() {
            self.paint_config_empty_state(ui, lang, tokens, busy);
            return;
        }

        self.paint_config_toolbar(ui, lang, tokens, busy, total);
        ui.add_space(ui_theme::SPACE_SM);

        let wide = ui.available_width().min(HUB_CONTENT_MAX_W) >= HUB_WIDE_MIN_W;
        if wide {
            let available = ui.available_size();
            let (outer, _) = ui.allocate_exact_size(available, egui::Sense::hover());
            let content_w = outer.width().min(HUB_CONTENT_MAX_W);
            let content = egui::Rect::from_min_size(
                egui::pos2(outer.center().x - content_w * 0.5, outer.top()),
                egui::vec2(content_w, outer.height()),
            );
            let list =
                egui::Rect::from_min_size(content.min, egui::vec2(HUB_LIST_W, content.height()));
            if self.workspace.current.is_empty() {
                let form = egui::Rect::from_min_max(
                    egui::pos2(list.right() + HUB_PANEL_GAP, content.top()),
                    content.max,
                );
                workspace_card_in_rect(ui, list, tokens, |ui| self.config_list(ui, lang));
                workspace_card_in_rect(ui, form, tokens, |ui| self.config_form(ui, lang, tokens));
            } else {
                let checks = egui::Rect::from_min_max(
                    egui::pos2(content.right() - HUB_CHECKS_W, content.top()),
                    content.max,
                );
                let form = egui::Rect::from_min_max(
                    egui::pos2(list.right() + HUB_PANEL_GAP, content.top()),
                    egui::pos2(checks.left() - HUB_PANEL_GAP, content.bottom()),
                );
                workspace_card_in_rect(ui, list, tokens, |ui| self.config_list(ui, lang));
                workspace_card_in_rect(ui, form, tokens, |ui| self.config_form(ui, lang, tokens));
                workspace_card_in_rect(ui, checks, tokens, |ui| {
                    self.config_checks(ui, lang, tokens)
                });
            }
        } else {
            Frame::NONE
                .fill(tokens.surface_bg)
                .corner_radius(CornerRadius::same(ui_theme::RADIUS_CARD))
                .inner_margin(Margin::same(10))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui
                            .selectable_label(
                                self.workspace.current.is_empty(),
                                text("配置列表", "List"),
                            )
                            .clicked()
                        {
                            self.workspace.checks_tab = false;
                            self.workspace_navigate("__list", json!({}));
                        }
                        if !self.workspace.current.is_empty() {
                            ui.selectable_value(
                                &mut self.workspace.checks_tab,
                                false,
                                text("配置编辑", "Editor"),
                            );
                            ui.selectable_value(
                                &mut self.workspace.checks_tab,
                                true,
                                text("校验结果", "Checks"),
                            );
                        }
                    });
                    ui.separator();
                    if self.workspace.checks_tab && !self.workspace.current.is_empty() {
                        self.config_checks(ui, lang, tokens);
                    } else if self.workspace.current.is_empty() {
                        self.config_list(ui, lang);
                    } else {
                        self.config_form(ui, lang, tokens);
                    }
                });
        }
    }

    fn paint_config_empty_state(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        tokens: &Tokens,
        busy: bool,
    ) {
        let zh = matches!(lang, Lang::Zh);
        let text = |cn, en| if zh { cn } else { en };
        let available = ui.available_size();
        ui.add_space((available.y * 0.12).clamp(24.0, 96.0));
        let card_w = ui.available_width().min(620.0);
        ui.horizontal(|ui| {
            ui.add_space(((ui.available_width() - card_w) * 0.5).max(0.0));
            Frame::NONE
                .fill(tokens.surface_bg)
                .stroke(Stroke::new(1.0_f32, tokens.border))
                .corner_radius(CornerRadius::same(ui_theme::RADIUS_CARD))
                .inner_margin(Margin::same(24))
                .show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.set_width((card_w - 48.0).max(1.0));
                        ui.vertical_centered(|ui| {
                            ui.label(
                                RichText::new(text(
                                    "创建第一个测试配置",
                                    "Create your first test configuration",
                                ))
                                .size(ui_theme::FONT_SECTION)
                                .strong()
                                .color(tokens.text_primary),
                            );
                            ui.add_space(ui_theme::SPACE_SM);
                            ui.label(
                                RichText::new(text(
                                    "配置仪器、复位、日志、波形和判定规则，然后执行预检。",
                                    "Configure instruments, reset, logs, waveforms and verdict rules, then run preflight.",
                                ))
                                .color(tokens.text_muted),
                            );
                        });
                        ui.add_space(ui_theme::SPACE_LG);
                        ui.horizontal(|ui| {
                            let input_w = (ui.available_width() - 108.0).max(160.0);
                            ui.add_sized(
                                [input_w, ui_theme::CTRL_H],
                                egui::TextEdit::singleline(&mut self.workspace.new_id)
                                    .hint_text(text("配置 ID，例如 fpga_smoke", "Configuration ID, e.g. fpga_smoke")),
                            );
                            let create = ui.add_enabled(
                                !busy && !self.workspace.new_id.trim().is_empty(),
                                egui::Button::new(
                                    RichText::new(text("新建配置", "Create"))
                                        .color(tokens.accent_text)
                                        .strong(),
                                )
                                .fill(tokens.accent)
                                .stroke(Stroke::NONE)
                                .min_size(egui::vec2(96.0, ui_theme::CTRL_H)),
                            );
                            if create.clicked() {
                                self.workspace_navigate(
                                    "config.create",
                                    json!({"config_id":self.workspace.new_id.trim()}),
                                );
                            }
                        });
                        ui.add_space(ui_theme::SPACE_MD);
                        ui.separator();
                        self.paint_transfer_section(ui, lang, busy, false);
                    });
                });
        });
    }

    fn paint_config_toolbar(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        tokens: &Tokens,
        busy: bool,
        total: u64,
    ) {
        let zh = matches!(lang, Lang::Zh);
        let text = |cn, en| if zh { cn } else { en };
        let card_w = ui.available_width().min(HUB_CONTENT_MAX_W);
        ui.horizontal(|ui| {
            ui.add_space(((ui.available_width() - card_w) * 0.5).max(0.0));
            Frame::NONE
                .fill(tokens.surface_bg)
                .corner_radius(CornerRadius::same(ui_theme::RADIUS_CARD))
                .inner_margin(Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.set_width((card_w - 24.0).max(1.0));
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = ui_theme::SPACE_SM;
                    ui.label(
                        RichText::new(text("配置库", "Configuration library"))
                            .strong()
                            .color(tokens.text_primary),
                    );
                    ui.label(
                        RichText::new(if zh {
                            format!("共 {total} 个")
                        } else {
                            format!("{total} total")
                        })
                        .color(tokens.text_muted),
                    );
                    ui.separator();
                    ui.add_sized(
                        [180.0, ui_theme::CTRL_H],
                        egui::TextEdit::singleline(&mut self.workspace.new_id)
                            .hint_text(text("新配置 ID", "New configuration ID")),
                    );
                    if ui
                        .add_enabled(
                            !busy && !self.workspace.new_id.trim().is_empty(),
                            egui::Button::new(text("新建", "New")),
                        )
                        .clicked()
                    {
                        self.workspace_navigate(
                            "config.create",
                            json!({"config_id":self.workspace.new_id.trim()}),
                        );
                    }
                    if !self.workspace.current.is_empty() {
                        if ui
                            .add_enabled(
                                !busy && !self.workspace.new_id.trim().is_empty(),
                                egui::Button::new(text("复制为", "Duplicate as")),
                            )
                            .clicked()
                        {
                            self.workspace_navigate(
                                "config.duplicate",
                                json!({"config_id":self.workspace.current,"new_id":self.workspace.new_id.trim()}),
                            );
                        }
                        ui.separator();
                        if ui
                            .add_enabled(!busy, egui::Button::new(text("校验", "Validate")))
                            .clicked()
                        {
                            self.workspace_send("config.save", self.workspace_save_params(true));
                        }
                        if self.workspace.dirty() {
                            if ui
                                .add_enabled(
                                    !busy
                                        && self
                                            .workspace
                                            .errors
                                            .as_array()
                                            .is_none_or(|errors| errors.is_empty()),
                                    egui::Button::new(
                                        RichText::new(text("保存", "Save"))
                                            .color(tokens.accent_text)
                                            .strong(),
                                    )
                                    .fill(tokens.accent)
                                    .stroke(Stroke::NONE),
                                )
                                .clicked()
                            {
                                self.workspace_send(
                                    "config.save",
                                    self.workspace_save_params(false),
                                );
                            }
                            if ui.button(text("放弃修改", "Discard")).clicked() {
                                self.workspace_discard();
                            }
                        }
                        if ui
                            .add_enabled(
                                !busy && !self.workspace.dirty(),
                                egui::Button::new(text("归档", "Archive")),
                            )
                            .clicked()
                        {
                            self.workspace_send(
                                "config.archive",
                                self.workspace_save_params(false),
                            );
                        }
                    }
                });
                self.paint_transfer_section(ui, lang, busy, true);
                    });
                });
        });
    }

    fn paint_transfer_section(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        busy: bool,
        allow_export: bool,
    ) {
        let zh = matches!(lang, Lang::Zh);
        let text = |cn, en| if zh { cn } else { en };
        ui.collapsing(text("导入、导出与归档", "Import, export and archive"), |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(ui_theme::SPACE_SM, ui_theme::SPACE_SM);
            ui.horizontal_wrapped(|ui| {
                ui.add_sized(
                    [ui.available_width().min(360.0), ui_theme::CTRL_H],
                    egui::TextEdit::singleline(&mut self.workspace.file)
                        .hint_text(text("配置包文件路径", "Configuration bundle path")),
                );
                if ui.button("…").clicked() {
                    if let Some(file) = rfd::FileDialog::new()
                        .add_filter("JSON", &["json"])
                        .pick_file()
                    {
                        self.workspace.file = file.display().to_string();
                    }
                }
                let strategies = [
                    text("跳过重名", "Skip conflicts"),
                    text("自动重命名", "Rename"),
                    text("替换并检查版本", "Replace with revision check"),
                ];
                egui::ComboBox::from_id_salt("import_strategy")
                    .width(160.0)
                    .selected_text(strategies[self.workspace.import_strategy])
                    .show_ui(ui, |ui| {
                        for (i, strategy) in strategies.iter().enumerate() {
                            ui.selectable_value(&mut self.workspace.import_strategy, i, *strategy);
                        }
                    });
                if ui
                    .add_enabled(
                        !busy && !self.workspace.file.is_empty(),
                        egui::Button::new(text("导入", "Import")),
                    )
                    .clicked()
                {
                    let mut expected = json!({});
                    if let Some(rows) = self.workspace.snapshot["items"].as_array() {
                        for row in rows {
                            expected[row["id"].as_str().unwrap_or("")] = json!({
                                "expected_revision": row["revision"],
                                "expected_sha256": row["sha256"]
                            });
                        }
                    }
                    self.workspace_navigate(
                        "config.import",
                        json!({
                            "file":self.workspace.file,
                            "strategy":(["skip","rename","replace"][self.workspace.import_strategy]),
                            "expected":expected
                        }),
                    );
                }
                if allow_export
                    && ui
                        .add_enabled(
                            !busy
                                && !self.workspace.file.is_empty()
                                && !self.workspace.selected.is_empty(),
                            egui::Button::new(text("导出所选", "Export selected")),
                        )
                        .clicked()
                {
                    self.workspace_send(
                        "config.export",
                        json!({"file":self.workspace.file,"ids":self.workspace.selected}),
                    );
                }
            });
            let archives = self.workspace.snapshot["archives"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if !archives.is_empty() {
                ui.separator();
                ui.label(text("已归档配置", "Archived configurations"));
                egui::ScrollArea::vertical()
                    .id_salt("archives")
                    .max_height(120.0)
                    .show_rows(ui, ui_theme::CTRL_H, archives.len(), |ui, range| {
                        for i in range {
                            ui.horizontal(|ui| {
                                ui.label(archives[i]["id"].as_str().unwrap_or(""));
                                if ui.button(text("恢复", "Restore")).clicked() {
                                    self.workspace_navigate(
                                        "config.restore",
                                        json!({
                                            "archive_id":archives[i]["archive_id"],
                                            "config_id":if self.workspace.new_id.is_empty(){archives[i]["id"].as_str().unwrap_or("")}else{&self.workspace.new_id}
                                        }),
                                    );
                                }
                            });
                        }
                    });
            }
        });
    }
    fn config_list(&mut self, ui: &mut egui::Ui, lang: Lang) {
        let zh = matches!(lang, Lang::Zh);
        ui.label(
            RichText::new(if zh {
                "测试配置"
            } else {
                "Test configurations"
            })
            .size(ui_theme::FONT_SECTION)
            .strong(),
        );
        ui.add_space(ui_theme::SPACE_SM);
        ui.add(
            egui::TextEdit::singleline(&mut self.workspace.query).hint_text(if zh {
                "筛选名称、ID、标签"
            } else {
                "Filter name, ID, tags"
            }),
        );
        let cache_key = format!(
            "{}:{}:{}:{}",
            self.workspace.list_epoch,
            self.workspace.query,
            self.workspace.config_status,
            self.workspace.sort
        );
        if self.workspace.list_cache_key != cache_key {
            let query = self.workspace.query.to_lowercase();
            let status = &self.workspace.config_status;
            let mut rows: Vec<Value> = self.workspace.snapshot["items"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|r| {
                    (status.is_empty() || r["status"] == *status)
                        && format!("{} {} {}", r["id"], r["name"], r["tags"])
                            .to_lowercase()
                            .contains(&query)
                })
                .cloned()
                .collect();
            let sort = ["order", "name", "updated_at"][self.workspace.sort];
            rows.sort_by(|a, b| {
                if sort == "order" {
                    a[sort]
                        .as_i64()
                        .unwrap_or(0)
                        .cmp(&b[sort].as_i64().unwrap_or(0))
                } else {
                    a[sort]
                        .as_str()
                        .unwrap_or("")
                        .cmp(b[sort].as_str().unwrap_or(""))
                }
            });
            self.workspace.list_cache = std::sync::Arc::new(rows);
            self.workspace.list_cache_key = cache_key;
        }
        let rows = self.workspace.list_cache.clone();
        ui.add_enabled_ui(self.job.is_none(),|ui|{ui.horizontal_wrapped(|ui|{
            if ui.button(if zh{"全选"}else{"All"}).clicked(){self.workspace.selected.extend(rows.iter().filter(|r|r["draft"]!=true).filter_map(|r|r["id"].as_str().map(str::to_owned)));}
            if ui.button(if zh{"反选"}else{"Invert"}).clicked(){for row in rows.iter().filter(|r|r["draft"]!=true){let id=row["id"].as_str().unwrap_or("").to_owned();if !self.workspace.selected.remove(&id){self.workspace.selected.insert(id);}}}
            for(enabled,cn,en)in[(true,"启用","Enable"),(false,"禁用","Disable")]{if ui.add_enabled(self.workspace.rx.is_none()&&!self.workspace.selected.is_empty(),egui::Button::new(if zh{cn}else{en})).clicked(){let mut expected=json!({});for row in self.workspace.snapshot["items"].as_array().into_iter().flatten(){expected[row["id"].as_str().unwrap_or("")]=json!({"expected_revision":row["revision"],"expected_sha256":row["sha256"]});}self.workspace_send("config.batch",json!({"ids":self.workspace.selected,"expected":expected,"enabled":enabled}));}}
        });
        });
        ui.horizontal(|ui| {
            let sort_labels = if zh {
                ["默认顺序", "名称", "更新时间"]
            } else {
                ["Default order", "Name", "Updated"]
            };
            egui::ComboBox::from_id_salt("config_sort")
                .width(118.0)
                .selected_text(sort_labels[self.workspace.sort])
                .show_ui(ui, |ui| {
                    for (i, s) in sort_labels.iter().enumerate() {
                        ui.selectable_value(&mut self.workspace.sort, i, *s);
                    }
                });
            ui.label(format!(
                "{} / {}",
                rows.len(),
                self.workspace.snapshot["total"]
            ));
        });
        egui::ComboBox::from_id_salt("config_status")
            .width(ui.available_width())
            .selected_text(if self.workspace.config_status.is_empty() {
                if zh {
                    "全部状态"
                } else {
                    "All states"
                }
            } else if self.workspace.config_status == "valid" {
                if zh {
                    "有效"
                } else {
                    "Valid"
                }
            } else {
                if zh {
                    "无效"
                } else {
                    "Invalid"
                }
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut self.workspace.config_status,
                    String::new(),
                    if zh { "全部状态" } else { "All states" },
                );
                ui.selectable_value(
                    &mut self.workspace.config_status,
                    "valid".into(),
                    if zh { "有效" } else { "Valid" },
                );
                ui.selectable_value(
                    &mut self.workspace.config_status,
                    "invalid".into(),
                    if zh { "无效" } else { "Invalid" },
                );
            });
        ui.horizontal(|ui|{ui.add(egui::TextEdit::singleline(&mut self.workspace.batch_tags).hint_text(if zh{"批量标签，逗号分隔"}else{"Batch tags, comma-separated"}));if ui.add_enabled(self.workspace.rx.is_none()&&!self.workspace.selected.is_empty(),egui::Button::new(if zh{"设置"}else{"Apply"})).clicked(){let mut expected=json!({});for row in self.workspace.snapshot["items"].as_array().into_iter().flatten(){expected[row["id"].as_str().unwrap_or("")]=json!({"expected_revision":row["revision"],"expected_sha256":row["sha256"]});}let tags:Vec<_>=self.workspace.batch_tags.split(',').map(str::trim).filter(|s|!s.is_empty()).collect();self.workspace_send("config.batch",json!({"ids":self.workspace.selected,"expected":expected,"tags":tags}));}});

        let mut list_scroll = egui::ScrollArea::vertical().id_salt("config_list");
        if self.workspace.focus_new_row {
            if let Some(i) = rows.iter().position(|r| r["id"] == self.workspace.current) {
                list_scroll = list_scroll.vertical_scroll_offset(i as f32 * 54.0);
            }
            self.workspace.focus_new_row = false;
        }
        list_scroll.show_rows(ui, 54.0, rows.len(), |ui, range| {
            for i in range {
                let row = &rows[i];
                let id = row["id"].as_str().unwrap_or("");
                ui.push_id(id, |ui| {
                    ui.horizontal(|ui| {
                        let mut selected = self.workspace.selected.contains(id);
                        if ui
                            .add_enabled(
                                self.job.is_none() && row["draft"] != true,
                                egui::Checkbox::without_text(&mut selected),
                            )
                            .changed()
                        {
                            if selected {
                                self.workspace.selected.insert(id.into());
                            } else {
                                self.workspace.selected.remove(id);
                            }
                        }
                        let available = ui.available_width().max(40.0);
                        let name = row["name"].as_str().unwrap_or(id);
                        let tags = row["tags"]
                            .as_array()
                            .map(|a| {
                                a.iter()
                                    .filter_map(Value::as_str)
                                    .collect::<Vec<_>>()
                                    .join(" · ")
                            })
                            .unwrap_or_default();
                        let status = if row["draft"] == true {
                            if zh {
                                "未保存 · 不可执行"
                            } else {
                                "Unsaved · not executable"
                            }
                        } else if row["status"] == "invalid" {
                            if zh {
                                "需要修正"
                            } else {
                                "Needs attention"
                            }
                        } else if row["enabled"] == false {
                            if zh {
                                "已禁用"
                            } else {
                                "Disabled"
                            }
                        } else {
                            if zh {
                                "已启用"
                            } else {
                                "Enabled"
                            }
                        };
                        let color = if row["status"] == "invalid" {
                            egui::Color32::LIGHT_RED
                        } else {
                            ui.visuals().weak_text_color()
                        };
                        let current = self.workspace.current == id;
                        let response = Frame::NONE
                            .fill(if current {
                                ui.visuals().selection.bg_fill.gamma_multiply(0.55)
                            } else {
                                ui.visuals().faint_bg_color
                            })
                            .stroke(Stroke::new(
                                1.0_f32,
                                if current {
                                    ui.visuals().selection.stroke.color
                                } else {
                                    ui.visuals().widgets.noninteractive.bg_stroke.color
                                },
                            ))
                            .corner_radius(6)
                            .inner_margin(7)
                            .show(ui, |ui| {
                                ui.vertical(|ui| {
                                    ui.set_width((available - 12.0).max(1.0));
                                    ui.add(
                                        egui::Label::new(RichText::new(name).strong()).truncate(),
                                    );
                                    let state = self.workspace.snapshot["matrix"]["items"]
                                        .as_array()
                                        .and_then(|items| {
                                            items.iter().find(|item| item["id"] == id)
                                        })
                                        .and_then(|item| item["status"].as_str())
                                        .unwrap_or("pending");
                                    let tone = if row["status"] == "invalid" {
                                        color
                                    } else {
                                        match state {
                                            "pass" => egui::Color32::from_rgb(78, 180, 125),
                                            "fail" | "error" => egui::Color32::LIGHT_RED,
                                            "running" => ui.visuals().selection.stroke.color,
                                            _ => color,
                                        }
                                    };
                                    ui.label(
                                        RichText::new(
                                            if row["status"] == "invalid" || row["enabled"] == false
                                            {
                                                status.to_owned()
                                            } else {
                                                if matches!(
                                                    state,
                                                    "pass"
                                                        | "fail"
                                                        | "error"
                                                        | "aborted"
                                                        | "skipped"
                                                        | "inconclusive"
                                                ) {
                                                    let historical = self.workspace.snapshot
                                                        ["matrix"]["items"]
                                                        .as_array()
                                                        .and_then(|items| {
                                                            items
                                                                .iter()
                                                                .find(|item| item["id"] == id)
                                                        });
                                                    let stale = historical.is_some_and(|item| {
                                                        item["config_sha256"] != row["sha256"]
                                                    }) || (self.workspace.current
                                                        == id
                                                        && self.workspace.dirty());
                                                    format!(
                                                        "{} {}{}",
                                                        if zh { "上次" } else { "Last" },
                                                        matrix_status_label(state, zh),
                                                        if stale {
                                                            if zh {
                                                                " · 内容已变更，待测试"
                                                            } else {
                                                                " · Changed; untested"
                                                            }
                                                        } else {
                                                            ""
                                                        }
                                                    )
                                                } else {
                                                    matrix_status_label(state, zh).to_owned()
                                                }
                                            },
                                        )
                                        .small()
                                        .color(tone),
                                    );
                                });
                            })
                            .response;
                        if ui
                            .interact(response.rect, response.id, egui::Sense::click())
                            .on_hover_text(format!("{name}\n{id}\n{tags}\n{status}"))
                            .clicked()
                        {
                            self.workspace_navigate("config.get", json!({"config_id":id}));
                        }
                    });
                });
            }
        });
    }
    fn config_form(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        if self.workspace.current.is_empty() {
            let zh = matches!(lang, Lang::Zh);
            ui.vertical_centered(|ui| {
                ui.add_space((ui.available_height() * 0.28).clamp(36.0, 160.0));
                ui.label(
                    RichText::new(if zh {
                        "选择一个测试配置"
                    } else {
                        "Select a test configuration"
                    })
                    .size(ui_theme::FONT_SECTION)
                    .strong()
                    .color(tokens.text_primary),
                );
                ui.add_space(ui_theme::SPACE_SM);
                ui.label(
                    RichText::new(if zh {
                        "从左侧列表选择配置以编辑参数和查看校验结果。"
                    } else {
                        "Choose a configuration from the list to edit it and review validation."
                    })
                    .color(tokens.text_muted),
                );
            });
            return;
        }
        let schema = self.workspace.snapshot["schema"].clone();
        let docs = self.workspace.snapshot["documents"].clone();
        let ui_schema = self.workspace.snapshot["ui"].clone();
        let locked = self.workspace.snapshot["matrix"]["workspace_locked"] == true
            || (self.job.is_some() && self.workspace.selected.contains(&self.workspace.current))
            || self.workspace.snapshot["locked"]["ids"]
                .as_array()
                .is_some_and(|ids| {
                    ids.iter()
                        .any(|id| id.as_str() == Some(&self.workspace.current))
                });
        if locked {
            ui.label("CONFIG.LOCKED_BY_RUN");
        }
        let mut ui_schema = ui_schema;
        if !ui_schema["groups"].is_array() && ui_schema["layout"]["groups"].is_array() {
            ui_schema["groups"] = ui_schema["layout"]["groups"].clone();
        }
        ui_schema["_serial_ports"] = json!(self
            .serial_ports
            .iter()
            .map(|p| p.device.clone())
            .collect::<Vec<_>>());
        ui_schema["_instruments"] = json!(self
            .live_devices
            .iter()
            .map(|d| json!({"value":d.resource,"resource":d.resource,"serial":d.serial,"kind":d.kind,"model":d.model,"label":d.label()}))
            .collect::<Vec<_>>());
        ui_schema["_root"] = self.workspace.document.clone();
        ui_schema["_project_config_root"] = self.workspace.project_document.clone();
        ui_schema["_busy"] = json!(self.workspace.rx.is_some());
        let errors = self.workspace.errors.clone();
        let focus = self.workspace.focus_path.take();
        let groups = ui_schema["groups"].as_array().cloned().unwrap_or_default();
        if let Some(pointer) = focus.as_deref() {
            if let Some(index) = groups.iter().position(|g| {
                g["fields"].as_array().is_some_and(|fields| {
                    fields.iter().filter_map(Value::as_str).any(|field| {
                        let base = format!("/{}", field.replace('.', "/"));
                        pointer == base || pointer.starts_with(&(base + "/"))
                    })
                })
            }) {
                self.workspace.form_section = index;
            }
        }
        ui.horizontal(|ui| {
            let name = self.workspace.document["name"]
                .as_str()
                .filter(|name| !name.is_empty())
                .unwrap_or(&self.workspace.current);
            ui.label(
                RichText::new(name)
                    .size(ui_theme::FONT_SECTION)
                    .strong()
                    .color(tokens.text_primary),
            );
            ui.label(RichText::new(&self.workspace.current).color(tokens.text_muted));
            if self.workspace.dirty() {
                ui.label(
                    RichText::new(if matches!(lang, Lang::Zh) {
                        "已修改"
                    } else {
                        "Modified"
                    })
                    .color(tokens.accent),
                );
            }
        });
        if self.workspace.document["id"]
            .as_str()
            .is_some_and(|id| id != self.workspace.current)
            && !self.workspace.dirty()
        {
            ui.colored_label(
                tokens.warning,
                if matches!(lang, Lang::Zh) {
                    "配置文件名与标识不一致，修复后即可正常预检。"
                } else {
                    "The filename differs from the configuration ID. Repair it before preflight."
                },
            );
            if ui
                .add_enabled(
                    !locked && self.workspace.rx.is_none(),
                    egui::Button::new(if matches!(lang, Lang::Zh) {
                        "修复文件名与标识"
                    } else {
                        "Repair filename and ID"
                    }),
                )
                .clicked()
            {
                self.workspace_send("config.save", self.workspace_save_params(false));
            }
        }
        let properties = ui_schema["item_properties"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if !properties.is_empty() {
            ui.collapsing(
                if matches!(lang, Lang::Zh) {
                    "测试项属性"
                } else {
                    "Item properties"
                },
                |ui| {
                    ui.add_enabled_ui(!locked, |ui| {
                        for key in properties.iter().filter_map(Value::as_str) {
                            if let Some(value) = self.workspace.document.get_mut(key) {
                                form(
                                    ui,
                                    &schema["properties"][key],
                                    &docs,
                                    &ui_schema,
                                    value,
                                    &format!("/{key}"),
                                    lang,
                                    &errors,
                                    focus.as_deref(),
                                    0,
                                );
                            }
                        }
                    });
                },
            );
        }
        ui.separator();
        if !groups.is_empty() {
            ui.horizontal_wrapped(|ui| {
                for (index, group) in groups.iter().enumerate() {
                    let label = localized_title(group, matches!(lang, Lang::Zh));
                    if ui
                        .selectable_label(self.workspace.form_section == index, label)
                        .clicked()
                    {
                        self.workspace.form_section = index;
                    }
                }
                ui.selectable_value(
                    &mut self.workspace.form_section,
                    groups.len(),
                    if matches!(lang, Lang::Zh) {
                        "全部"
                    } else {
                        "All"
                    },
                );
            });
            ui.separator();
        }
        egui::ScrollArea::vertical()
            .id_salt((
                "config_form",
                &self.workspace.current,
                self.workspace.form_section,
            ))
            .show(ui, |ui| {
                ui.add_enabled_ui(!locked, |ui| {
                    if let Some(group) = groups.get(self.workspace.form_section) {
                        let fields: Vec<_> = group["fields"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_str)
                            .collect();
                        if fields.iter().all(|field| !field.contains('.')) {
                            let props = fields
                                .iter()
                                .filter_map(|field| {
                                    schema_at(&schema, &docs, field).map(|s| (field.to_string(), s))
                                })
                                .collect();
                            form_layout::object_fields(
                                ui,
                                &props,
                                &schema,
                                &docs,
                                &ui_schema,
                                &mut self.workspace.document,
                                "",
                                lang,
                                &errors,
                                focus.as_deref(),
                                0,
                                &HashSet::new(),
                            );
                        } else {
                            for field in fields {
                                let pointer = format!("/{}", field.replace('.', "/"));
                                if let Some(field_schema) = schema_at(&schema, &docs, field) {
                                    if let Some(value) =
                                        self.workspace.document.pointer_mut(&pointer)
                                    {
                                        form(
                                            ui,
                                            &field_schema,
                                            &docs,
                                            &ui_schema,
                                            value,
                                            &pointer,
                                            lang,
                                            &errors,
                                            focus.as_deref(),
                                            0,
                                        );
                                    }
                                }
                            }
                        }
                    } else {
                        form(
                            ui,
                            &schema,
                            &docs,
                            &ui_schema,
                            &mut self.workspace.document,
                            "",
                            lang,
                            &errors,
                            focus.as_deref(),
                            0,
                        );
                    }
                });
            });
        if self.workspace.document != self.workspace.last_form_value {
            self.workspace.last_edit = Some(Instant::now());
            self.workspace.last_form_value = self.workspace.document.clone();
            if let Some(row) = self.workspace.drafts.get_mut(&self.workspace.current) {
                row["name"] = self.workspace.document["name"].clone();
                if let Some(rows) = self.workspace.snapshot["items"].as_array_mut() {
                    if let Some(current) =
                        rows.iter_mut().find(|r| r["id"] == self.workspace.current)
                    {
                        *current = row.clone();
                    }
                }
                self.workspace.list_epoch = self.workspace.list_epoch.wrapping_add(1);
            }
            ui.ctx().request_repaint_after(Duration::from_millis(620));
        }
        if let Some(mut request) = ui
            .ctx()
            .data_mut(|d| d.remove_temp::<Value>(egui::Id::new("hub.inspect_file")))
        {
            if !locked && self.workspace.rx.is_none() {
                request["config_id"] = json!(self.workspace.current);
                self.workspace_send("config.inspect_file", request);
            }
        }
    }
    fn config_checks(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        let zh = matches!(lang, Lang::Zh);
        ui.label(
            RichText::new(if zh {
                "校验与预检"
            } else {
                "Validation & preflight"
            })
            .size(ui_theme::FONT_SECTION)
            .strong()
            .color(tokens.text_primary),
        );
        ui.add_space(ui_theme::SPACE_SM);
        let preflight_valid = self.workspace.snapshot["preflight"]["valid"] == true
            && !self.workspace.dirty()
            && self.workspace.errors.as_array().is_none_or(Vec::is_empty);
        ui.label(
            RichText::new(if preflight_valid {
                if zh {
                    "预检有效"
                } else {
                    "Preflight valid"
                }
            } else {
                if zh {
                    "需要重新预检"
                } else {
                    "Preflight required"
                }
            })
            .color(if preflight_valid {
                tokens.success
            } else {
                tokens.warning
            })
            .strong(),
        );
        ui.label(
            RichText::new(if zh {
                format!("配置版本 {}", self.workspace.original_revision)
            } else {
                format!(
                    "Configuration revision {}",
                    self.workspace.original_revision
                )
            })
            .color(tokens.text_muted),
        );
        ui.add_space(ui_theme::SPACE_MD);
        let error_count = self.workspace.errors.as_array().map_or(0, Vec::len);
        let next = if self.workspace.dirty() {
            if zh {
                "下一步：校验并保存修改"
            } else {
                "Next: validate and save changes"
            }
        } else if error_count > 0 {
            if zh {
                "下一步：修正下列配置问题"
            } else {
                "Next: resolve the configuration issues below"
            }
        } else if !preflight_valid {
            if zh {
                "下一步：确认本轮参数，然后点击预检"
            } else {
                "Next: review run parameters, then preflight"
            }
        } else {
            if zh {
                "已就绪：在测试矩阵确认选择后运行"
            } else {
                "Ready: confirm the matrix selection and run"
            }
        };
        ui.label(RichText::new(next).strong().color(if error_count > 0 {
            tokens.warning
        } else {
            tokens.text_primary
        }));
        if let Some(help) =
            self.workspace.snapshot["ui"][if zh { "help_zh" } else { "help" }].as_str()
        {
            ui.small(help);
        }
        if self.workspace.snapshot["ui"]["global_actions"]
            .as_array()
            .is_some_and(|a| a.iter().any(|v| v == "ai_settings"))
            && ui
                .button(if zh {
                    "全局 AI 设置…"
                } else {
                    "Global AI settings…"
                })
                .clicked()
        {
            ui.ctx()
                .data_mut(|d| d.insert_temp(egui::Id::new("hub.open_ai_settings"), true));
        }
        ui.separator();
        // Preview semantics are declared by widgets, never inferred from plugin IDs.
        let inspected = &self.workspace.file_metadata;
        if inspected["pointer"]
            .as_str()
            .and_then(|p| self.workspace.document.pointer(p))
            == inspected.get("path")
            && inspected.is_object()
        {
            ui.collapsing(if zh { "文件校验结果" } else { "Inspected file" },|ui| {
                ui.label(inspected["path"].as_str().unwrap_or(""));
                ui.monospace(inspected["sha256"].as_str().unwrap_or(""));
                ui.weak(format!("{} bytes",inspected["bytes"]));
                ui.small(if zh { "此结果来自显式文件检查；预检与运行会重新核验文件内容。" } else { "Explicit inspection result; preflight and execution independently recheck content." });
            });
        }
        let widgets = self.workspace.snapshot["ui"]["widgets"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        for (field, spec) in widgets {
            let kind = spec
                .as_str()
                .or(spec["type"].as_str())
                .or(spec["widget"].as_str())
                .unwrap_or("");
            let pointer = format!("/{}", field.replace('.', "/"));
            let Some(value) = self.workspace.document.pointer(&pointer) else {
                continue;
            };
            if [
                "instrument_selector",
                "serial_port_selector",
                "serial_selector",
            ]
            .contains(&kind)
            {
                let name = friendly_path(
                    &self.workspace.snapshot["schema"],
                    &self.workspace.snapshot["documents"],
                    &field,
                    zh,
                );
                ui.weak(name);
                ui.label(redact(value, "").as_str().unwrap_or("—"));
            }
            if ["timeline", "repeatable_timeline"].contains(&kind) {
                timeline(ui, value, &spec, zh);
            }
        }
        if let Some(warnings) = self.workspace.warnings.as_array() {
            for warning in warnings {
                ui.colored_label(
                    egui::Color32::YELLOW,
                    warning["message"].as_str().unwrap_or("Warning"),
                );
            }
        }
        if self.workspace.normalized.is_object()
            && self.workspace.normalized != self.workspace.document
            && ui
                .button(if zh {
                    "应用规范化建议"
                } else {
                    "Apply normalization suggestion"
                })
                .clicked()
        {
            self.workspace.document = self.workspace.normalized.clone();
        }
        if let Some(errors) = self.workspace.errors.as_array().cloned() {
            for error in errors {
                let raw_path = error["path"].as_str().unwrap_or("/");
                let path = if let Some(missing) = error["params"]["missingProperty"].as_str() {
                    format!(
                        "{}/{}",
                        raw_path.trim_end_matches('/'),
                        missing.replace('~', "~0").replace('/', "~1")
                    )
                } else {
                    raw_path.to_owned()
                };
                let label = friendly_path(
                    &self.workspace.snapshot["schema"],
                    &self.workspace.snapshot["documents"],
                    &path.trim_start_matches('/').replace('/', "."),
                    zh,
                );
                let message = diagnostic_message(&error, zh);
                if ui
                    .link(format!("{label}：{message}"))
                    .on_hover_text(&path)
                    .clicked()
                {
                    self.workspace.focus_path = Some(path.clone());
                }
                ui.collapsing(
                    if zh {
                        "诊断详情"
                    } else {
                        "Diagnostic details"
                    },
                    |ui| {
                        ui.monospace(format!(
                            "{} · {}",
                            path,
                            error["message"].as_str().unwrap_or("Invalid")
                        ));
                    },
                );
            }
        }
        if self.workspace.snapshot["ui"]["source_view"] != "hidden" {
            ui.collapsing(
                if zh {
                    "只读源码（脱敏）"
                } else {
                    "Read-only source (redacted)"
                },
                |ui| {
                    let text = serde_json::to_string_pretty(&redact(
                        &schema_redact(
                            &self.workspace.document,
                            &self.workspace.snapshot["schema"],
                            &self.workspace.snapshot["documents"],
                            0,
                        ),
                        "",
                    ))
                    .unwrap_or_default();
                    egui::ScrollArea::both()
                        .id_salt("source_view")
                        .max_height(300.0)
                        .show(ui, |ui| {
                            ui.add(
                                egui::Label::new(egui::RichText::new(text).monospace())
                                    .selectable(true),
                            );
                        });
                },
            );
        }
    }
    fn paint_matrix(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        let available = ui.available_size();
        let (outer, _) = ui.allocate_exact_size(available, egui::Sense::hover());
        let content_w = outer.width().min(HUB_CONTENT_MAX_W);
        let content = egui::Rect::from_min_size(
            egui::pos2(outer.center().x - content_w * 0.5, outer.top()),
            egui::vec2(content_w, outer.height()),
        );
        workspace_card_in_rect(ui, content, tokens, |ui| {
            self.paint_matrix_content(ui, lang)
        });
    }

    fn paint_matrix_content(&mut self, ui: &mut egui::Ui, lang: Lang) {
        self.paint_project_matrix(ui, lang);
    }
}

fn workspace_card_in_rect(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    tokens: &Tokens,
    add: impl FnOnce(&mut egui::Ui),
) {
    ui.painter().rect_filled(
        rect,
        CornerRadius::same(ui_theme::RADIUS_CARD),
        tokens.surface_bg,
    );
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(ui_theme::RADIUS_CARD),
        Stroke::new(1.0_f32, tokens.divider),
        egui::StrokeKind::Inside,
    );
    ui_theme::scope_in_allocated_rect(
        ui,
        egui::UiBuilder::new()
            .max_rect(rect.shrink(ui_theme::SPACE_MD))
            .layout(egui::Layout::top_down(egui::Align::Min)),
        |ui| {
            ui.set_clip_rect(rect.intersect(ui.clip_rect()));
            ui.spacing_mut().item_spacing = egui::vec2(ui_theme::SPACE_SM, ui_theme::SPACE_SM);
            add(ui);
        },
    );
}

fn matrix_status_label(status: &str, zh: bool) -> &str {
    match (status, zh) {
        ("all", true) => "全部状态",
        ("total", true) => "总计",
        ("selected", true) => "已选择",
        ("pending", true) => "待测试",
        ("running", true) => "测试中",
        ("pass", true) => "通过",
        ("fail", true) => "失败",
        ("error", true) => "错误",
        ("skipped", true) => "已跳过",
        ("aborted", true) => "已中止",
        ("inconclusive", true) => "待确认",
        ("all", false) => "All states",
        ("total", false) => "Total",
        ("selected", false) => "Selected",
        ("pending", false) => "To be tested",
        ("running", false) => "Testing",
        ("pass", false) => "Pass",
        ("fail", false) => "Fail",
        ("error", false) => "Error",
        ("skipped", false) => "Skipped",
        ("aborted", false) => "Aborted",
        ("inconclusive", false) => "Inconclusive",
        _ => status,
    }
}
fn matrix_config_changes(snapshot: &Value, dirty_id: Option<&str>) -> usize {
    let configurations: std::collections::HashMap<_, _> = snapshot["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| row["id"].as_str().map(|id| (id, &row["sha256"])))
        .collect();
    snapshot["matrix"]["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|row| {
            let id = row["id"].as_str().unwrap_or("");
            dirty_id == Some(id)
                || configurations.get(id).is_none_or(|current| {
                    current
                        .as_str()
                        .zip(row["config_sha256"].as_str())
                        .is_none_or(|(a, b)| a != b)
                })
        })
        .count()
}

fn redact(value: &Value, key: &str) -> Value {
    if ["secret", "password", "token", "api_key"]
        .iter()
        .any(|s| key.to_lowercase().contains(s))
    {
        return json!("[REDACTED]");
    }
    match value {
        Value::Object(map) => {
            Value::Object(map.iter().map(|(k, v)| (k.clone(), redact(v, k))).collect())
        }
        Value::Array(a) => Value::Array(a.iter().map(|v| redact(v, key)).collect()),
        Value::String(s) if Path::new(s).is_absolute() => json!("[LOCAL_PATH]"),
        v => v.clone(),
    }
}
fn resolved(schema: &Value, docs: &Value) -> Value {
    if let Some(reference) = schema["$ref"].as_str() {
        let (key, pointer) = reference.split_once('#').unwrap_or((reference, ""));
        let doc = &docs[key];
        return if pointer.is_empty() {
            doc.clone()
        } else {
            doc.pointer(pointer).cloned().unwrap_or(Value::Null)
        };
    }
    schema.clone()
}
fn field_defaults(schema: &Value, docs: &Value, depth: usize) -> Value {
    if depth > 24 {
        return Value::Null;
    }
    let s = resolved(schema, docs);
    let required = s["required"].as_array().cloned().unwrap_or_default();
    if let Some(branch) = s["oneOf"]
        .as_array()
        .or_else(|| s["anyOf"].as_array())
        .and_then(|a| a.first())
    {
        return field_defaults(branch, docs, depth + 1);
    }
    if let Some(v) = s
        .get("default")
        .or_else(|| s.get("const"))
        .or_else(|| s["enum"].as_array().and_then(|v| v.first()))
    {
        return v.clone();
    }
    match s["type"]
        .as_str()
        .unwrap_or(if s["properties"].is_object() {
            "object"
        } else {
            "string"
        }) {
        "object" => Value::Object(
            s["properties"]
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(k, property)| {
                    property.get("default").is_some()
                        || property.get("const").is_some()
                        || required.iter().any(|v| v == *k)
                })
                .map(|(k, v)| (k.clone(), field_defaults(v, docs, depth + 1)))
                .collect(),
        ),
        "array" => json!([]),
        "boolean" => json!(false),
        "number" | "integer" => s.get("minimum").cloned().unwrap_or(json!(0)),
        "null" => Value::Null,
        _ => json!(""),
    }
}
fn form(
    ui: &mut egui::Ui,
    schema: &Value,
    docs: &Value,
    layout: &Value,
    value: &mut Value,
    pointer: &str,
    lang: Lang,
    errors: &Value,
    focus: Option<&str>,
    depth: usize,
) {
    if depth > 24 {
        ui.label("SCHEMA.DEPTH_LIMIT");
        return;
    }
    let schema = resolved(schema, docs);
    let zh = matches!(lang, Lang::Zh);
    let dot = pointer.trim_start_matches('/').replace('/', ".");
    let field_spec = ui_spec(layout, "fields", &dot);
    let title = caption(
        &schema,
        field_spec,
        pointer.rsplit('/').next().unwrap_or(""),
        zh,
    );
    let widget_spec = ui_spec(layout, "widgets", &dot);
    let widget = widget_spec
        .as_str()
        .or(widget_spec["widget"].as_str())
        .or(widget_spec["type"].as_str())
        .unwrap_or("");
    if field_spec["hidden"] == true || !visible(layout, &dot, &layout["_root"]) {
        return;
    }
    ui.push_id(pointer, |ui| {
        let width = match field_spec["width"].as_str() {
            Some("short") => 220.0,
            Some("normal") => 320.0,
            _ => ui.available_width(),
        };
        if !schema["properties"].is_object()
            && !schema["oneOf"].is_array()
            && !schema["anyOf"].is_array()
            && schema["type"] != "array"
        {
            ui.set_max_width(ui.available_width().min(width));
        }
        if !widget.is_empty()
            && ![
                "file",
                "path",
                "file_picker",
                "path_picker",
                "tags",
                "tag_input",
                "discriminated_union",
                "timeline",
                "rule_builder",
                "reference_selector",
                "serial_port",
                "serial_selector",
                "serial_port_selector",
                "instrument_selector",
                "read_only_summary",
                "json_read_only",
            ]
            .contains(&widget)
        {
            ui.colored_label(
                egui::Color32::YELLOW,
                if zh {
                    "此可选控件尚不支持，已使用类型化表单"
                } else {
                    "Optional widget unavailable; using the typed form"
                },
            );
        }
        if schema["readOnly"] == true {
            ui.horizontal(|ui| {
                ui.weak(title);
                ui.label(value.to_string());
            });
            return;
        }
        if schema.get("const").is_some() {
            if value != &schema["const"] {
                ui.colored_label(
                    egui::Color32::LIGHT_RED,
                    if zh {
                        "固定参数不合法，请查看校验结果"
                    } else {
                        "Invalid constant; review diagnostics"
                    },
                );
            }
            return;
        }
        if widget == "reference_selector" {
            let initial = value.as_str().unwrap_or("").to_owned();
            let mut text = initial.clone();
            let source = widget_spec["source_path"].as_str().unwrap_or("");
            let root=if widget_spec["source_scope"]=="project_config" || ui_spec(layout,"fields",&pointer.trim_start_matches('/').replace('/',"."))["source_scope"]=="project_config" { &layout["_project_config_root"] } else { &layout["_root"] };
            let target = root.pointer(&format!("/{}", source.replace('.', "/")));
            let value_key = widget_spec["value_field"].as_str().unwrap_or("id");
            let label_key = widget_spec["label_field"].as_str().unwrap_or("name");
            let choices: Vec<(String, String)> = match target {
                Some(Value::Array(items)) => items
                    .iter()
                    .filter_map(|item| {
                        item[value_key].as_str().map(|id| {
                            (
                                id.to_owned(),
                                item[label_key].as_str().unwrap_or(id).to_owned(),
                            )
                        })
                    })
                    .collect(),
                Some(Value::Object(items)) => items
                    .iter()
                    .map(|(key, item)| {
                        (
                            key.clone(),
                            item[label_key].as_str().unwrap_or(key).to_owned(),
                        )
                    })
                    .collect(),
                _ => Vec::new(),
            };
            ui.label(title);
            egui::ComboBox::from_id_salt("reference")
                .width(ui.available_width().min(320.0))
                .selected_text(if text.is_empty() {
                    if zh {
                        "选择引用"
                    } else {
                        "Select reference"
                    }
                } else {
                    &text
                })
                .show_ui(ui, |ui| {
                    for (id, label) in &choices {
                        ui.selectable_value(&mut text, id.clone(), label);
                    }
                });
            if ui
                .add(egui::TextEdit::singleline(&mut text).desired_width(ui.available_width()))
                .changed()
                || text != initial
            {
                *value = json!(text);
            }
            if !text.is_empty() && !choices.iter().any(|(id, _)| id == &text) {
                ui.colored_label(
                    egui::Color32::YELLOW,
                    if zh {
                        "此引用尚未解析，请在预检前确认"
                    } else {
                        "Unresolved reference; verify before preflight"
                    },
                );
            }
            return;
        }
        // Union selection remains typed; no JSON source editor is used.
        if let Some(variants) = schema["oneOf"]
            .as_array()
            .or_else(|| schema["anyOf"].as_array())
        {
            let idx = variants
                .iter()
                .position(|s| {
                    if let Some(kind) = s["type"].as_str() {
                        if kind != "object" {
                            return match kind {
                                "string" => value.is_string(),
                                "number" => value.is_number(),
                                "integer" => value.as_i64().is_some(),
                                "boolean" => value.is_boolean(),
                                "array" => value.is_array(),
                                "null" => value.is_null(),
                                _ => false,
                            };
                        }
                    }
                    s["properties"].as_object().is_some_and(|props| {
                        props
                            .iter()
                            .all(|(k, v)| v.get("const").is_none_or(|c| value.get(k) == Some(c)))
                    })
                })
                .unwrap_or(0);
            let mut chosen = idx;
            egui::ComboBox::from_id_salt("variant")
                .selected_text(localized_title(&variants[idx], zh))
                .show_ui(ui, |ui| {
                    for (i, v) in variants.iter().enumerate() {
                        ui.selectable_value(&mut chosen, i, localized_title(v, zh));
                    }
                });
            if idx != chosen {
                *value = switch_variant(value, &variants[chosen], docs, depth + 1);
            }
            let mut branch = schema.clone();
            branch.as_object_mut().map(|m| {
                m.remove("oneOf");
                m.remove("anyOf");
            });
            if let Some(fields) = variants[chosen].as_object() {
                for (k, v) in fields {
                    if k == "properties" {
                        if !branch[k].is_object() {
                            branch[k] = json!({});
                        }
                        branch[k]
                            .as_object_mut()
                            .unwrap()
                            .extend(v.as_object().cloned().unwrap_or_default());
                    } else {
                        branch[k] = v.clone();
                    }
                }
            }
            form(
                ui,
                &branch,
                docs,
                layout,
                value,
                pointer,
                lang,
                errors,
                focus,
                depth + 1,
            );
            return;
        }
        if [
            "serial_port",
            "serial_selector",
            "serial_port_selector",
            "instrument_selector",
        ]
        .contains(&widget)
        {
            let mut text = value.as_str().unwrap_or("").to_owned();
            ui.label(title);
            egui::ComboBox::from_id_salt("resource")
                .selected_text(if text.is_empty() {
                    if zh {
                        "选择资源"
                    } else {
                        "Select resource"
                    }
                } else {
                    &text
                })
                .width(ui.available_width().min(320.0))
                .show_ui(ui, |ui| {
                    let choices = if widget == "instrument_selector" {
                        &layout["_instruments"]
                    } else {
                        &layout["_serial_ports"]
                    };
                    for item in choices.as_array().into_iter().flatten() {
                        if widget == "instrument_selector"
                            && widget_spec["kind"]
                                .as_str()
                                .is_some_and(|kind| item["kind"] != kind)
                        {
                            continue;
                        }
                        let picked = widget_spec["value_field"].as_str().unwrap_or("value");
                        let raw = item.as_str().or(item[picked].as_str()).unwrap_or("");
                        ui.selectable_value(
                            &mut text,
                            raw.into(),
                            item["label"].as_str().unwrap_or(raw),
                        );
                    }
                });
            if !text.is_empty() {
                *value = json!(text);
            }
            return;
        }
        if ["tags", "tag_input"].contains(&widget) && schema["type"] == "array" {
            if !value.is_array() {
                ui.colored_label(
                    egui::Color32::LIGHT_RED,
                    if zh {
                        "标签应为列表，请修复配置"
                    } else {
                        "Expected a tag list; repair the configuration"
                    },
                );
                return;
            }
            ui.label(title);
            let items = value.as_array_mut().unwrap();
            let mut remove = None;
            ui.horizontal_wrapped(|ui| {
                for (i, item) in items.iter_mut().enumerate() {
                    let mut text = item.as_str().unwrap_or("").to_owned();
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            if ui
                                .add_sized([100.0, 24.0], egui::TextEdit::singleline(&mut text))
                                .changed()
                            {
                                *item = json!(text);
                            }
                            if ui.small_button("×").clicked() {
                                remove = Some(i);
                            }
                        });
                    });
                }
                if ui.small_button("+").clicked()
                    && items.len() < schema["maxItems"].as_u64().unwrap_or(32) as usize
                {
                    items.push(json!(""));
                }
            });
            if let Some(i) = remove {
                items.remove(i);
            }
            return;
        }
        if widget == "read_only_summary" || widget == "json_read_only" {
            let text =
                serde_json::to_string_pretty(&redact(&schema_redact(value, &schema, docs, 0), ""))
                    .unwrap_or_default();
            ui.label(egui::RichText::new(text).monospace());
            return;
        }
        if let Some(options) = schema["enum"].as_array() {
            let mut selected = value.clone();
            ui.label(title);
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("enum")
                    .width(ui.available_width())
                    .selected_text(enum_caption(&schema, field_spec, value, zh))
                    .show_ui(ui, |ui| {
                        for v in options {
                            ui.selectable_value(
                                &mut selected,
                                v.clone(),
                                enum_caption(&schema, field_spec, v, zh),
                            );
                        }
                    });
            });
            *value = selected;
        } else {
            let kind = schema["type"]
                .as_str()
                .or_else(|| {
                    schema["type"]
                        .as_array()
                        .and_then(|a| a.iter().filter_map(Value::as_str).find(|s| *s != "null"))
                })
                .unwrap_or(if schema["properties"].is_object() {
                    "object"
                } else {
                    "string"
                });
            match kind {
                "object" => {
                    if !value.is_object() {
                        ui.colored_label(
                            egui::Color32::LIGHT_RED,
                            if zh {
                                "此处应为对象，请修复配置"
                            } else {
                                "Expected an object; repair the configuration"
                            },
                        );
                        return;
                    }
                    let mut props = schema["properties"]
                        .as_object()
                        .cloned()
                        .unwrap_or_default();
                    if let Some(condition) = schema.get("if") {
                        let matches = condition["properties"].as_object().is_some_and(|p| {
                            p.iter().all(|(k, s)| {
                                s.get("const").is_none_or(|c| value.get(k) == Some(c))
                                    && s.get("enum").is_none_or(|a| {
                                        a.as_array().is_some_and(|a| a.contains(&value[k]))
                                    })
                            })
                        });
                        let branch = &schema[if matches { "then" } else { "else" }];
                        if let Some(extra) = branch["properties"].as_object() {
                            props.extend(extra.clone());
                        }
                    }
                    let unknown: Vec<String> = value
                        .as_object()
                        .unwrap()
                        .keys()
                        .filter(|k| !props.contains_key(*k))
                        .cloned()
                        .collect();
                    if schema["additionalProperties"] == false {
                        for key in unknown {
                            ui.horizontal(|ui| {
                                ui.colored_label(
                                    egui::Color32::LIGHT_RED,
                                    format!("Unknown field: {key}"),
                                );
                                if ui
                                    .small_button(if zh {
                                        "移除未知字段"
                                    } else {
                                        "Remove unknown field"
                                    })
                                    .clicked()
                                {
                                    value.as_object_mut().unwrap().remove(&key);
                                }
                            });
                        }
                    }
                    let mut grouped = HashSet::new();
                    if pointer.is_empty(){grouped.extend(layout["item_properties"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_owned));}
                    if pointer.is_empty() {
                        for group in layout["groups"].as_array().into_iter().flatten() {
                            let caption = if zh { group["label_zh"].as_str() } else { None }
                                .or(group["label"].as_str())
                                .or(group["title"].as_str())
                                .unwrap_or("Configuration");
                            egui::CollapsingHeader::new(caption)
                                .id_salt(caption)
                                .default_open(true)
                                .show(ui, |ui| {
                                    for field in group["fields"]
                                        .as_array()
                                        .into_iter()
                                        .flatten()
                                        .filter_map(Value::as_str)
                                    {
                                        let path = format!("/{}", field.replace('.', "/"));
                                        let root = field.split('.').next().unwrap_or(field);
                                        if let Some(target_schema) = schema_at(&schema, docs, field)
                                        {
                                            if let Some(target) = value.pointer_mut(&path) {
                                                form(
                                                    ui,
                                                    &target_schema,
                                                    docs,
                                                    layout,
                                                    target,
                                                    &path,
                                                    lang,
                                                    errors,
                                                    focus,
                                                    depth + 1,
                                                );
                                                grouped.insert(root.to_owned());
                                            } else if ui
                                                .small_button(format!("+ {field}"))
                                                .clicked()
                                            {
                                                if !field.contains('.') {
                                                    value[field] = field_defaults(
                                                        &target_schema,
                                                        docs,
                                                        depth + 1,
                                                    );
                                                }
                                            }
                                        }
                                    }
                                });
                        }
                    }
                    form_layout::object_fields(
                        ui, &props, &schema, docs, layout, value, pointer, lang, errors, focus,
                        depth, &grouped,
                    );
                }
                "array" => {
                    if !value.is_array() {
                        ui.colored_label(
                            egui::Color32::LIGHT_RED,
                            if zh {
                                "此处应为列表，请修复配置"
                            } else {
                                "Expected an array; repair the configuration"
                            },
                        );
                        return;
                    }
                    let items = value.as_array_mut().unwrap();
                    let mut remove = None;
                    let mut movement = None;
                    let structured = matches!(widget, "rule_builder" | "timeline");
                    let selection_id = ui.make_persistent_id((pointer, "selected_detail"));
                    let mut selected = ui
                        .ctx()
                        .data_mut(|d| d.get_temp::<usize>(selection_id))
                        .unwrap_or(usize::MAX);
                    if let Some(target) = focus.and_then(|f| f.strip_prefix(&format!("{pointer}/")))
                    {
                        if let Some(index) = target
                            .split('/')
                            .next()
                            .and_then(|p| p.parse::<usize>().ok())
                        {
                            selected = index;
                        }
                    }
                    if structured {
                        ui.label(egui::RichText::new(title).strong());
                        for (i, item) in items.iter().enumerate() {
                            let summary = form_layout::item_summary(
                                item,
                                &schema["items"],
                                docs,
                                widget_spec,
                                layout,
                                pointer,
                                zh,
                                i,
                            );
                            if ui
                                .add(egui::Button::new(summary).selected(selected == i).wrap())
                                .clicked()
                            {
                                selected = if selected == i { usize::MAX } else { i };
                            }
                        }
                        ui.add_space(8.0);
                    }
                    for (i, item) in items.iter_mut().enumerate() {
                        if structured && selected != i {
                            continue;
                        }
                        let key = item["id"]
                            .as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| i.to_string());
                        ui.push_id(key, |ui| {
                            ui.group(|ui| {
                                ui.horizontal(|ui| {
                                    ui.dnd_drag_source(
                                        egui::Id::new((pointer, i, "drag")),
                                        (pointer.to_owned(), i),
                                        |ui| {
                                            ui.label("⠿");
                                        },
                                    );
                                    ui.weak(format!("{} {}", title, i + 1));
                                    if ui.small_button("↑").clicked() && i > 0 {
                                        movement = Some((i, i - 1));
                                    }
                                    if ui.small_button("↓").clicked() {
                                        movement = Some((i, i + 1));
                                    }
                                    if ui.small_button("×").clicked() {
                                        remove = Some(i);
                                    }
                                });
                                form(
                                    ui,
                                    &schema["items"],
                                    docs,
                                    layout,
                                    item,
                                    &format!("{pointer}/{i}"),
                                    lang,
                                    errors,
                                    focus,
                                    depth + 1,
                                );
                            });
                            let response = ui.interact(
                                ui.min_rect(),
                                egui::Id::new((pointer, i, "drop")),
                                egui::Sense::hover(),
                            );
                            if let Some(payload) = response.dnd_release_payload::<(String, usize)>()
                            {
                                if payload.0 == pointer && payload.1 != i {
                                    movement = Some((payload.1, i));
                                }
                            }
                        });
                    }
                    if let Some(i) = remove {
                        let candidate = items[i].get("id").and_then(Value::as_str);
                        if candidate
                            .is_some_and(|id| has_reference(&layout["_root"], id, &items[i]))
                        {
                            ui.colored_label(
                                egui::Color32::LIGHT_RED,
                                if zh {
                                    "该项仍被引用，请先解除引用。"
                                } else {
                                    "Remove references before deleting this item."
                                },
                            );
                        } else {
                            items.remove(i);
                            selected = usize::MAX;
                        }
                    }
                    if let Some((a, b)) = movement {
                        if b < items.len() {
                            items.swap(a, b);
                            selected = b;
                        }
                    }
                    if items.len() < schema["maxItems"].as_u64().unwrap_or(500).min(500) as usize
                        && ui
                            .small_button(if zh { "+ 添加" } else { "+ Add" })
                            .clicked()
                    {
                        let mut item = field_defaults(&schema["items"], docs, depth + 1);
                        if item.is_object() && item.get("id").is_some() {
                            item["id"] = json!(format!(
                                "item_{}",
                                std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_nanos()
                            ));
                        }
                        items.push(item);
                        selected = items.len() - 1;
                    }
                    if structured {
                        ui.ctx().data_mut(|d| d.insert_temp(selection_id, selected));
                    }
                }
                "boolean" => {
                    let mut b = value.as_bool().unwrap_or(false);
                    if ui.checkbox(&mut b, title).changed() {
                        *value = json!(b);
                    }
                }
                "number" | "integer" => {
                    let mut number = value.as_f64().unwrap_or(0.0);
                    ui.label(title);
                    ui.horizontal(|ui| {
                        let control_width = (ui.available_width() - 42.0).min(220.0).max(24.0);
                        let response = ui.add_sized(
                            [control_width, 26.0],
                            egui::DragValue::new(&mut number).speed(if kind == "integer" {
                                1.0
                            } else {
                                0.01
                            }),
                        );
                        if let Some(unit) =
                            schema["x-unit"].as_str().or(widget_spec["unit"].as_str())
                        {
                            ui.weak(unit);
                        }
                        if response.changed() {
                            *value = if kind == "integer" {
                                json!(number as i64)
                            } else {
                                json!(number)
                            };
                        }
                        if focus == Some(pointer) {
                            response.scroll_to_me(Some(egui::Align::Center));
                            response.request_focus();
                        }
                    });
                }
                "null" => {
                    ui.label(title);
                }
                _ => {
                    let mut string = value.as_str().unwrap_or("").to_owned();
                    ui.label(title);
                    let mut edit =
                        egui::TextEdit::singleline(&mut string).desired_width(ui.available_width());
                    if schema["writeOnly"] == true {
                        edit = edit.password(true);
                    }
                    let response = ui.add(edit);
                    if response.changed() {
                        *value = json!(string);
                    }
                    if focus == Some(pointer) {
                        response.scroll_to_me(Some(egui::Align::Center));
                        response.request_focus();
                    }
                    if (["path", "file-path", "directory"]
                        .contains(&schema["format"].as_str().unwrap_or(""))
                        || ["path", "file", "file_picker", "path_picker"].contains(&widget))
                        && ui
                            .add_enabled(layout["_busy"] != true,egui::Button::new(if zh { "浏览…" } else { "Browse…" }).small())
                            .clicked()
                    {
                        let file = if schema["format"] == "directory" {
                            rfd::FileDialog::new().pick_folder()
                        } else {
                            rfd::FileDialog::new().pick_file()
                        };
                        if let Some(file) = file {
                            if schema["format"] != "directory" {
                                ui.ctx().data_mut(|d|d.insert_temp(egui::Id::new("hub.inspect_file"),json!({"field":dot,"path":file.display().to_string(),"expected_value":value})));
                            } else { *value = json!(file.display().to_string()); }
                        }
                    }
                    if ["path","file-path"].contains(&schema["format"].as_str().unwrap_or(""))
                        && ui.add_enabled(layout["_busy"] != true && value.as_str().is_some_and(|s|!s.is_empty()),egui::Button::new(if zh { "校验文件并更新 SHA-256" } else { "Inspect file and update SHA-256" }).small()).clicked() {
                        ui.ctx().data_mut(|d|d.insert_temp(egui::Id::new("hub.inspect_file"),json!({"field":dot,"path":value,"expected_value":value})));
                    }
                }
            }
        }
        if let Some(message) = local_field_error(value, &schema, ui.ctx(), zh) {
            ui.colored_label(egui::Color32::LIGHT_RED, message);
        }
        if let Some(help) = (if zh {
            field_spec["description_zh"]
                .as_str()
                .or(schema["x-description-zh"].as_str())
        } else {
            None
        })
        .or(field_spec["description"].as_str())
        .or(schema["description"].as_str())
        {
            ui.weak(help);
        }
        for error in errors
            .as_array()
            .into_iter()
            .flatten()
            .filter(|e| e["path"] == pointer)
        {
            ui.colored_label(
                egui::Color32::LIGHT_RED,
                friendly_error(error["message"].as_str().unwrap_or("Invalid"), zh),
            );
        }
    });
}

fn localized_title(value: &Value, zh: bool) -> &str {
    (if zh {
        value["x-title-zh"].as_str().or(value["label_zh"].as_str())
    } else {
        None
    })
    .or(value["title"].as_str())
    .or(value["label"].as_str())
    .unwrap_or(if zh { "选项" } else { "Option" })
}
fn friendly_error(message: &str, zh: bool) -> &str {
    if !zh {
        return message;
    }
    match message {
        "Filename and id differ" => "文件名与配置标识不一致，请使用上方修复按钮",
        "Invalid" => "请检查此字段的填写内容",
        "must have required property 'pattern'" => "请填写匹配正则",
        _ => message,
    }
}
fn diagnostic_message(error: &Value, zh: bool) -> String {
    if !zh {
        return error["message"].as_str().unwrap_or("Invalid").to_owned();
    }
    match error["keyword"].as_str().unwrap_or("") {
        "required" => "请补齐必需字段".into(),
        "type" => "字段类型不符合要求，请重新填写".into(),
        "enum" => "请选择支持的选项".into(),
        "const" => "固定参数不符合要求，请修复配置".into(),
        "additionalProperties" => "存在不支持的字段，请检查并移除".into(),
        "minimum" | "exclusiveMinimum" => format!("数值低于允许下限 {}", error["params"]["limit"]),
        "maximum" | "exclusiveMaximum" => format!("数值高于允许上限 {}", error["params"]["limit"]),
        "minLength" => "内容过短，请补充填写".into(),
        "maxLength" => "内容过长，请缩短".into(),
        "minItems" => "项目数量不足，请添加".into(),
        "maxItems" => "项目数量超过限制，请减少".into(),
        "pattern" | "format" => "内容格式不符合要求".into(),
        "oneOf" | "anyOf" => "请选择有效类型，并补齐该类型的参数".into(),
        _ => friendly_error(error["message"].as_str().unwrap_or("Invalid"), true).into(),
    }
}
fn friendly_path(schema: &Value, docs: &Value, dot: &str, zh: bool) -> String {
    let parts: Vec<_> = dot.split('.').collect();
    let mut labels = Vec::new();
    for end in 1..=parts.len() {
        if (end == parts.len() || end == 2) && parts[end - 1] != "selector" {
            if let Some(s) = schema_at(schema, docs, &parts[..end].join(".")) {
                labels.push(localized_title(&s, zh).to_owned());
            }
        }
    }
    if labels.is_empty() {
        if zh {
            "配置内容".into()
        } else {
            "Configuration".into()
        }
    } else {
        labels.join(" · ")
    }
}
fn insert_field(root: &mut Value, pointer: &str, value: Value) {
    let parts: Vec<_> = pointer.trim_start_matches('/').split('/').collect();
    let mut current = root;
    for part in &parts[..parts.len() - 1] {
        if !current[*part].is_object() {
            current[*part] = json!({});
        }
        current = &mut current[*part];
    }
    current[parts[parts.len() - 1]] = value;
}
fn switch_variant(previous: &Value, branch: &Value, docs: &Value, depth: usize) -> Value {
    let mut next = field_defaults(branch, docs, depth);
    if let (Some(old), Some(props)) = (previous.as_object(), branch["properties"].as_object()) {
        for (key, s) in props {
            if s.get("const").is_none() {
                if let Some(value) = old.get(key) {
                    next[key] = value.clone();
                }
            }
        }
    }
    next
}
fn schema_at(schema: &Value, docs: &Value, dot: &str) -> Option<Value> {
    let mut s = schema.clone();
    for key in dot.split('.') {
        s = resolved(&s, docs);
        if key.parse::<usize>().is_ok() && s["items"].is_object() {
            s = s["items"].clone();
            continue;
        }
        if s["properties"].get(key).is_none() {
            if let Some(branch) = s["oneOf"]
                .as_array()
                .or_else(|| s["anyOf"].as_array())
                .and_then(|a| {
                    a.iter()
                        .find(|branch| branch["properties"].get(key).is_some())
                })
            {
                s = branch.clone();
            }
        }
        s = s["properties"].get(key)?.clone();
    }
    Some(s)
}
fn visible(layout: &Value, path: &str, root: &Value) -> bool {
    let mut has_show = false;
    let mut shown = false;
    for condition in layout["conditional_fields"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let matches_path = |key: &str| {
            condition[key].as_array().is_some_and(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .any(|p| path == p || path.starts_with(&format!("{p}.")))
            })
        };
        let reference = condition["path"].as_str().unwrap_or("");
        let active =
            root.pointer(&format!("/{}", reference.replace('.', "/"))) == condition.get("value");
        if matches_path("hide") && active {
            return false;
        }
        if matches_path("show") {
            has_show = true;
            shown |= active;
        }
    }
    !has_show || shown
}

fn has_reference(root: &Value, id: &str, item: &Value) -> bool {
    match root {
        Value::Object(map) if root != item => map
            .iter()
            .any(|(k, v)| k != "id" && has_reference(v, id, item)),
        Value::Array(a) => a.iter().any(|v| has_reference(v, id, item)),
        Value::String(s) => s == id,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn fixture(count: usize) -> TestToolPanel {
        let mut cfg = AppConfig::default();
        cfg.apps.test_tool.plugins_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-tools/plugins")
            .display()
            .to_string();
        cfg.apps.test_tool.data_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tmp/hub-layout")
            .display()
            .to_string();
        let mut panel = TestToolPanel::new(&cfg);
        panel.prepare_layout_audit();
        panel.select_plugin("hub-workspace-demo".into());
        let plugin = panel.selected_plugin().unwrap().clone();
        let schema: Value =
            serde_json::from_slice(&fs::read(plugin.dir.join("config.schema.json")).unwrap())
                .unwrap();
        let layout: Value =
            serde_json::from_slice(&fs::read(plugin.dir.join("config.ui.json")).unwrap()).unwrap();
        let items:Vec<_>=(0..count).map(|n|json!({"id":format!("case_{n:03}"),"name":format!("测试配置 / Test {n:03}"),"revision":3,"enabled":true,"tags":["smoke"],"status":"valid","sha256":"abc"})).collect();
        let matrix:Vec<_>=(0..count).map(|n|json!({"id":format!("case_{n:03}"),"name":format!("测试配置 / Test {n:03}"),"tags":["smoke"],"order":n,"selected":true,"status":if n<5{"pass"}else{"pending"},"phase":"inspect","progress":if n<5{1.0}else{0.0},"attempt":1,"elapsed_s":0.02,"message":"","result_ref":null})).collect();
        panel.workspace = Workspace {
            key: format!(
                "{}:{}:{}",
                plugin.id,
                panel.effective_project_id(),
                panel.hub_report_name(&plugin)
            ),
            snapshot: json!({"schema":schema,"documents":{},"ui":layout,"items":items,"total":count,"archives":[],"preflight":{"valid":true,"revision":"frozen-manifest"},"matrix":{"available":true,"revision":7,"run_id":"demo","items":matrix,"summary":{"total":count,"selected":count,"pass":count.min(5),"pending":count.saturating_sub(5),"running":0,"fail":0,"error":0,"aborted":0}}}),
            current: "case_000".into(),
            original_revision: 3,
            document: json!({"id":"case_000","name":"启动检查 / Startup check","enabled":true,"tags":["smoke","regression"],"editor_revision":3,"updated_at":"2026-10-18T08:00:00Z","duration_s":0.05,"steps":[{"id":"step_1","name":"检查状态 / Inspect status","mode":"inspect"}]}),
            ..Default::default()
        };
        panel.workspace.original = panel.workspace.document.clone();
        panel.workspace.last_form_value = panel.workspace.document.clone();
        panel.workspace.parameter_fingerprint = serde_json::to_string(
            &panel
                .param_values
                .iter()
                .collect::<std::collections::BTreeMap<_, _>>(),
        )
        .unwrap();
        if count == 0 {
            panel.workspace.current.clear();
            panel.workspace.document = Value::Null;
            panel.workspace.original = Value::Null;
        }
        panel
    }
    #[test]
    fn hub_workspace_virtualized_layout_and_dirty_state() {
        let mut timings = Vec::new();
        for lang in [Lang::Zh, Lang::En] {
            for light in [false, true] {
                for width in [640.0, 1100.0, 1920.0] {
                    for tab in [0, 1] {
                        let mut panel = fixture(500);
                        panel.workspace.tab = tab;
                        let ctx = egui::Context::default();
                        crate::fonts::install_cjk_fonts(&ctx);
                        let tokens = ui_theme::apply_theme(&ctx, light);
                        let size = egui::vec2(width, 800.0);
                        let started = Instant::now();
                        let output = ctx.run(
                            egui::RawInput {
                                screen_rect: Some(egui::Rect::from_min_size(
                                    egui::Pos2::ZERO,
                                    size,
                                )),
                                ..Default::default()
                            },
                            |ctx| {
                                egui::CentralPanel::default()
                                    .show(ctx, |ui| panel.paint_workspace(ui, lang, &tokens));
                            },
                        );
                        timings.push(started.elapsed().as_secs_f64() * 1000.0);
                        assert!(
                            panel.workspace.rx.is_none(),
                            "render must not launch IO for unchanged snapshots"
                        );
                        for primitive in
                            ctx.tessellate(output.shapes.clone(), output.pixels_per_point)
                        {
                            if let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive {
                                assert!(mesh.is_valid());
                                assert!(mesh.vertices.iter().all(|v| v.pos.is_finite()));
                            }
                        }
                        if !light {
                            if let Ok(directory) = std::env::var("WIPARSE_UI_AUDIT_OUTPUT") {
                                crate::ui_audit::export_frame(
                                    &ctx,
                                    output,
                                    &PathBuf::from(directory),
                                    &format!(
                                        "hub-{}-{}-{}",
                                        if matches!(lang, Lang::Zh) { "zh" } else { "en" },
                                        width as u32,
                                        tab
                                    ),
                                    size,
                                );
                            }
                        }
                    }
                }
            }
        }
        let mut panel = fixture(500);
        let ctx = egui::Context::default();
        let tokens = ui_theme::apply_theme(&ctx, false);
        let mut steady = Vec::new();
        for iteration in 0..40 {
            let start = Instant::now();
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1100.0, 800.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default()
                        .show(ctx, |ui| panel.paint_workspace(ui, Lang::En, &tokens));
                },
            );
            if iteration > 4 {
                steady.push(start.elapsed().as_secs_f64() * 1000.0);
            }
        }
        steady.sort_by(f64::total_cmp);
        println!(
            "Hub 500-row cached frame ms: p50={:.2}, p95={:.2}",
            steady[steady.len() / 2],
            steady[steady.len() * 95 / 100]
        );
        assert!(!panel.workspace.dirty());
        panel.workspace.document["name"] = json!("changed");
        assert!(panel.workspace.dirty());
        panel.workspace_navigate("config.get", json!({"config_id":"case_001"}));
        assert!(panel.workspace.pending.is_some());
        assert!(panel.workspace.rx.is_none());
        panel.workspace_discard();
        assert!(!panel.workspace.dirty());
        timings.sort_by(f64::total_cmp);
        println!(
            "Hub 500-row first-frame ms: p50={:.2}, p95={:.2}, max={:.2}",
            timings[timings.len() / 2],
            timings[timings.len() * 95 / 100],
            timings[timings.len() - 1]
        );
    }
    #[test]
    fn typed_defaults_references_and_sensitive_preview() {
        let docs = json!({"local":{"type":"integer","minimum":2}});
        assert_eq!(field_defaults(&json!({"$ref":"local"}), &docs, 0), json!(2));
        assert_eq!(
            redact(&json!({"api_key":"private","normal":1}), "")["api_key"],
            "[REDACTED]"
        );
        let root = json!({"captures":[{"id":"a"}],"rules":[{"capture":"a"}]});
        assert!(has_reference(&root, "a", &json!({"id":"a"})));
        assert!(!has_reference(
            &json!({"captures":[{"id":"a"}]}),
            "a",
            &json!({"id":"a"})
        ));
    }
    #[test]
    fn background_ipc_and_api_keep_stop_independent() {
        let mut panel = fixture(0);
        let root = std::env::temp_dir().join(format!(
            "wiparse-hub-ipc-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(root.clone());
        panel.data_root = root.display().to_string();
        let plugin = panel.selected_plugin().unwrap().clone();
        let call = |method: &str, params: Value| {
            run_worker(
                "node".into(),
                panel.workspace_request(&plugin, method, params, true),
            )
            .unwrap()
        };
        let draft = call("config.create", json!({"config_id":"five_case"}));
        assert_eq!(draft["ok"], true, "{draft}");
        let saved = call(
            "config.save",
            json!({"config_id":"five_case","value":draft["data"]["value"],"expected_revision":0,"expected_sha256":""}),
        );
        assert_eq!(saved["ok"], true, "{saved}");
        let (tx, rx) = crossbeam_channel::bounded(1);
        panel.api_workspace(crate::backend::PendingRequest {
            method: "ui.test_tool.config.list".into(),
            params: json!({"plugin":"hub-workspace-demo","limit":5}),
            reply: tx,
            deadline: Instant::now() + Duration::from_secs(20),
        });
        assert!(
            panel.api_stop().ok,
            "Stop is immediately callable while a configuration worker runs"
        );
        let reply = rx.recv_timeout(Duration::from_secs(20)).unwrap();
        assert!(reply.ok, "{:?}", reply.error);
        assert_eq!(reply.data["total"], 1);
        assert_eq!(reply.data["items"].as_array().unwrap().len(), 1);
        assert!(!reply.data.to_string().contains(&root.display().to_string()));
    }
}

fn schema_redact(value: &Value, schema: &Value, docs: &Value, depth: usize) -> Value {
    if depth > 32 {
        return json!("[REDACTED]");
    }
    let schema = resolved(schema, docs);
    if schema["writeOnly"] == true {
        return json!("[REDACTED]");
    }
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        schema_redact(v, &schema["properties"][k], docs, depth + 1),
                    )
                })
                .collect(),
        ),
        Value::Array(a) => Value::Array(
            a.iter()
                .map(|v| schema_redact(v, &schema["items"], docs, depth + 1))
                .collect(),
        ),
        _ => value.clone(),
    }
}
fn timeline(ui: &mut egui::Ui, value: &Value, spec: &Value, zh: bool) {
    let Some(items) = value.as_array() else {
        return;
    };
    ui.strong(if zh { "时间轴" } else { "Timeline" });
    let time = spec["time_field"].as_str();
    let label = spec["label_field"].as_str().unwrap_or("id");
    let points: Vec<_> = items
        .iter()
        .enumerate()
        .filter_map(|(i, v)| {
            let t = time.map(|key| v[key].as_f64()).unwrap_or(Some(i as f64))?;
            (t.is_finite() && t >= 0.0).then_some((t, v[label].as_str().unwrap_or("")))
        })
        .collect();
    if points.is_empty() {
        ui.weak(if zh { "暂无项目" } else { "No items" });
        return;
    }
    let max = points.iter().map(|p| p.0).fold(1.0, f64::max);
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 40.0), egui::Sense::hover());
    let y = rect.center().y;
    let painter = ui.painter();
    painter.line_segment(
        [
            egui::pos2(rect.left() + 5.0, y),
            egui::pos2(rect.right() - 5.0, y),
        ],
        egui::Stroke::new(1.0_f32, ui.visuals().weak_text_color()),
    );
    for (t, _) in &points {
        let x = rect.left() + 5.0 + (*t / max) as f32 * (rect.width() - 10.0);
        painter.circle_filled(egui::pos2(x, y), 3.0, ui.visuals().selection.bg_fill);
    }
    let unit = spec["unit"]
        .as_str()
        .unwrap_or(if time.is_some() { "s" } else { "order" });
    for (t, label) in points.iter().take(12) {
        ui.weak(format!("{t:.2} {unit} · {label}"));
    }
}

#[cfg(test)]
#[test]
fn actual_item_count_is_not_capacity() {
    for count in [0, 5, 500] {
        let mut panel = tests::fixture(count);
        let ctx = egui::Context::default();
        crate::fonts::install_cjk_fonts(&ctx);
        let tokens = ui_theme::apply_theme(&ctx, false);
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1440.0, 800.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .show(ctx, |ui| panel.paint_workspace(ui, Lang::En, &tokens));
            },
        );
        let labels: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.job.text.as_str()),
                _ => None,
            })
            .collect();
        if count == 0 {
            assert!(labels.contains(&"Create your first test configuration"));
            assert!(!labels.contains(&"Validation & preflight"));
            assert!(!labels.contains(&"Configuration library"));
        }
        if count == 0 || count == 5 {
            if let Ok(directory) = std::env::var("WIPARSE_UI_AUDIT_OUTPUT") {
                crate::ui_audit::export_frame(
                    &ctx,
                    output,
                    &PathBuf::from(directory),
                    if count == 0 {
                        "hub-empty"
                    } else {
                        "hub-five-items"
                    },
                    egui::vec2(1440.0, 800.0),
                );
            }
        }
        assert_eq!(panel.workspace.list_cache.len(), count);
        assert_eq!(
            panel.workspace.snapshot["matrix"]["items"]
                .as_array()
                .unwrap()
                .len(),
            count
        );
    }
}

#[cfg(test)]
#[test]
fn matrix_retains_historical_verdict_but_discloses_changed_configuration() {
    let mut panel = tests::fixture(1);
    let id = panel.workspace.snapshot["items"][0]["id"].clone();
    panel.workspace.snapshot["items"][0]["sha256"] = json!("original-content");
    panel.workspace.snapshot["matrix"]["items"][0]["id"] = id.clone();
    panel.workspace.snapshot["matrix"]["items"][0]["config_sha256"] = json!("original-content");
    panel.workspace.snapshot["matrix"]["items"][0]["status"] = json!("pass");
    assert_eq!(matrix_config_changes(&panel.workspace.snapshot, None), 0);
    panel.workspace.snapshot["items"][0]["sha256"] = json!("new-content");
    let ctx = egui::Context::default();
    let output = ctx.run(Default::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| panel.paint_matrix_content(ui, Lang::En));
    });
    let text = output
        .shapes
        .iter()
        .filter_map(|shape| {
            if let egui::Shape::Text(t) = &shape.shape {
                Some(t.galley.job.text.as_str())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("Latest run snapshot"));
    assert!(text.contains("do not certify current content"));
    assert_eq!(
        panel.workspace.snapshot["matrix"]["items"][0]["status"], "pass",
        "A history warning must not rewrite the original verdict"
    );
    panel.workspace.snapshot["items"][0]["sha256"] = json!("original-content");
    assert_eq!(
        matrix_config_changes(&panel.workspace.snapshot, id.as_str()),
        1,
        "Unsaved changes invalidate applicability too"
    );
}

fn local_field_error(
    value: &Value,
    schema: &Value,
    ctx: &egui::Context,
    zh: bool,
) -> Option<String> {
    let invalid = if zh {
        "字段值不符合约束"
    } else {
        "Value does not meet constraints"
    };
    if let Some(s) = value.as_str() {
        let count = s.chars().count() as u64;
        if schema["minLength"].as_u64().is_some_and(|n| count < n)
            || schema["maxLength"].as_u64().is_some_and(|n| count > n)
        {
            return Some(invalid.into());
        }
        if let Some(pattern) = schema["pattern"].as_str() {
            let key = egui::Id::new(("hub_pattern", pattern));
            let regex = ctx.data_mut(|data| {
                if let Some(cached) = data.get_temp::<std::sync::Arc<regex::Regex>>(key) {
                    Some(cached)
                } else {
                    regex::Regex::new(pattern).ok().map(|r| {
                        let r = std::sync::Arc::new(r);
                        data.insert_temp(key, r.clone());
                        r
                    })
                }
            });
            if regex.is_some_and(|r| !r.is_match(s)) {
                return Some(invalid.into());
            }
        }
    }
    if let Some(n) = value.as_f64() {
        if schema["minimum"].as_f64().is_some_and(|v| n < v)
            || schema["maximum"].as_f64().is_some_and(|v| n > v)
            || schema["exclusiveMinimum"].as_f64().is_some_and(|v| n <= v)
            || schema["exclusiveMaximum"].as_f64().is_some_and(|v| n >= v)
        {
            return Some(invalid.into());
        }
    }
    None
}

#[cfg(test)]
#[test]
#[ignore = "requires generated FPGA simulation workspace fixture"]
fn fpga_plugin_workspace_visual_verification() {
    let path = std::env::var("WIPARSE_FPGA_UI_FIXTURE").expect("set WIPARSE_FPGA_UI_FIXTURE");
    let fixture: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    let scenarios: Vec<_> = (0..6)
        .map(|section| (0, section, false, Lang::Zh, false))
        .chain([
            (1, 0, false, Lang::Zh, false),
            (4, 0, false, Lang::Zh, false),
            (4, 0, true, Lang::En, false),
            (0, 7, false, Lang::Zh, true),
            (0, 7, true, Lang::Zh, true),
            (0, 2, true, Lang::En, false),
            (0, 4, false, Lang::En, false),
        ])
        .collect();
    for (tab, section, light, language, detail) in scenarios {
        for width in [640.0, 1100.0, 1440.0, 1920.0] {
            let mut panel = tests::fixture(0);
            panel.select_plugin("fpga-firmware-test".into());
            panel.param_values = fixture["params"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        v.as_str().map(str::to_owned).unwrap_or(v.to_string()),
                    )
                })
                .collect();
            let plugin = panel.selected_plugin().unwrap().clone();
            panel.workspace = Workspace {
                key: format!(
                    "{}:{}:{}",
                    plugin.id,
                    panel.effective_project_id(),
                    panel.hub_report_name(&plugin)
                ),
                snapshot: fixture["workspace"].clone(),
                matrix_current: fixture["record"]["config_id"].as_str().unwrap().into(),
                result_details: fixture["details"].clone(),
                result_key: format!(
                    "{}:{}:{}",
                    fixture["record"]["config_id"],
                    fixture["workspace"]["matrix"]["run_id"],
                    fixture["workspace"]["matrix"]["items"][0]["attempt"]
                ),
                project_document: fixture["workspace"]["project_config"]["value"].clone(),
                project_original: fixture["workspace"]["project_config"]["value"].clone(),
                current: fixture["record"]["config_id"].as_str().unwrap().into(),
                document: fixture["record"]["value"].clone(),
                original: fixture["record"]["value"].clone(),
                original_revision: fixture["record"]["revision"].as_u64().unwrap(),
                original_sha: fixture["record"]["sha256"].as_str().unwrap_or("").into(),
                focus_path: detail.then(|| "/verdict/rules/0/text".into()),
                form_section: section,
                selected: fixture["workspace"]["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|row| row["enabled"] != false)
                    .filter_map(|row| row["id"].as_str().map(str::to_owned))
                    .collect(),
                tab,
                parameter_fingerprint: serde_json::to_string(
                    &panel
                        .param_values
                        .iter()
                        .collect::<std::collections::BTreeMap<_, _>>(),
                )
                .unwrap(),
                ..Default::default()
            };
            panel.workspace.last_form_value = panel.workspace.document.clone();
            let ctx = egui::Context::default();
            crate::fonts::install_cjk_fonts(&ctx);
            let tokens = ui_theme::apply_theme(&ctx, light);
            let size = egui::vec2(width, 900.0);
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default()
                        .show(ctx, |ui| panel.paint_workspace(ui, language, &tokens));
                },
            );
            assert_eq!(
                panel.workspace.snapshot["items"].as_array().unwrap().len(),
                fixture["workspace"]["items"].as_array().unwrap().len()
            );
            assert_eq!(
                panel.workspace.snapshot["matrix"]["items"]
                    .as_array()
                    .unwrap()
                    .len(),
                fixture["workspace"]["matrix"]["items"]
                    .as_array()
                    .unwrap()
                    .len()
            );
            assert!(panel.workspace.rx.is_none());
            assert!(
                !panel.workspace.dirty(),
                "Painting must not mutate the delivered plugin configuration"
            );
            if matches!(language, Lang::Zh) {
                fn schema_keys(value: &Value, keys: &mut HashSet<String>) {
                    if let Some(props) = value["properties"].as_object() {
                        keys.extend(props.keys().filter(|k| k.contains('_')).cloned());
                    }
                    match value {
                        Value::Object(map) => {
                            for v in map.values() {
                                schema_keys(v, keys);
                            }
                        }
                        Value::Array(a) => {
                            for v in a {
                                schema_keys(v, keys);
                            }
                        }
                        _ => {}
                    }
                }
                let mut raw_keys = HashSet::new();
                schema_keys(&fixture["workspace"]["documents"], &mut raw_keys);
                for shape in &output.shapes {
                    if let egui::Shape::Text(text) = &shape.shape {
                        let label = &text.galley.job.text;
                        assert!(
                            !raw_keys.contains(label),
                            "Unlocalized schema key in section {section}: {label}"
                        );
                        assert_ne!(label, "选项", "Unnamed union variant in section {section}");
                    }
                }
            }
            if let Ok(dir) = std::env::var("WIPARSE_UI_AUDIT_OUTPUT") {
                crate::ui_audit::export_frame(
                    &ctx,
                    output,
                    &PathBuf::from(dir),
                    &format!(
                        "fpga-{width}-{tab}-{section}-{}-{}-{}",
                        if light { "light" } else { "dark" },
                        if matches!(language, Lang::Zh) {
                            "zh"
                        } else {
                            "en"
                        },
                        if detail { "detail" } else { "summary" }
                    ),
                    size,
                );
            }
        }
    }
}

#[cfg(test)]
#[test]
fn portable_hub_paths_are_absolute_before_runner_changes_working_directory() {
    let mut cfg = AppConfig::default();
    cfg.apps.test_tool.plugins_dir = "test-tools/plugins".into();
    cfg.apps.test_tool.data_root = ".".into();
    cfg.apps.test_tool.cli_path = "bin/WiParse-CLI.exe".into();
    let panel = TestToolPanel::new(&cfg);
    assert_eq!(panel.plugins_dir, "test-tools/plugins");
    assert_eq!(panel.cli_path, "bin/WiParse-CLI.exe");
    assert!(Path::new(&resolve_command(&panel.cli_path)).is_absolute());
    assert!(Path::new(
        panel.workspace_request(
            panel
                .plugins
                .iter()
                .find(|p| p.id == "fpga-firmware-test")
                .unwrap(),
            "workspace.open",
            json!({}),
            true
        )["context"]["pluginDir"]
            .as_str()
            .unwrap()
    )
    .is_absolute());
    assert_eq!(
        Path::new(&portable_path(&resolve_dir("test-tools/plugins"))),
        Path::new("test-tools/plugins")
    );
    assert!(Path::new(&panel.data_root_resolved()).is_absolute());
    assert_eq!(panel.node_path, "node");
}
#[cfg(test)]
#[test]
fn workspace_typed_sections_and_variant_changes_preserve_common_fields() {
    let schema = json!({"type":"object","properties":{"id":{"type":"string"},"type":{"const":"log_contains"},"required":{"type":"boolean","default":true},"text":{"type":"string"}},"required":["id","type","required","text"]});
    let old = json!({"id":"boot","type":"log_regex","required":true,"pattern":"old"});
    let switched = switch_variant(&old, &schema, &json!({}), 0);
    assert_eq!(switched["id"], "boot");
    assert_eq!(switched["type"], "log_contains");
    assert!(switched.get("pattern").is_none());
    let union = json!({"oneOf":[schema.clone()]});
    assert_eq!(
        field_defaults(&union, &json!({}), 0)["type"],
        "log_contains"
    );
    let mut document = json!({"verdict":{"mode":"all"}});
    insert_field(
        &mut document,
        "/verdict/ai_assist",
        json!({"enabled":false}),
    );
    assert_eq!(document["verdict"]["mode"], "all");
    assert_eq!(document["verdict"]["ai_assist"]["enabled"], false);
    assert_eq!(
        friendly_error("Filename and id differ", true),
        "文件名与配置标识不一致，请使用上方修复按钮"
    );
}
