//! Unified VISA instrument workbench.

use crate::theme::{self, Tokens};
use chrono::Local;
use crossbeam_channel::{unbounded, Receiver, Sender};
use egui::{Color32, CornerRadius, Frame, Margin, RichText, Stroke};
use egui_plot::{Legend, Line, Plot, PlotPoints};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use wiparse_core::config::AppConfig;
use wiparse_core::i18n::Lang;
use wiparse_core::instrument::{
    discover_resources_with_library, export_csv, humanize_scope_reading_text, AcquisitionBuffer,
    Capabilities, ControlCommand, Identity, InstrumentDevice, InstrumentKind, MeasureFunction,
    Reading, ResourceInfo, Sample, ScopeMeasType, WaveformTrace,
};
use wiparse_core::waveform_file::{load_waveform_bytes, save_waveform_file};

enum Job {
    Scan {
        library: String,
        timeout_ms: u32,
    },
    Connect {
        id: u64,
        resource: String,
        kind: Option<InstrumentKind>,
        timeout_ms: u32,
        library: String,
    },
    /// Soft connect without VISA — opens the full workspace for UI debugging.
    ConnectDemo {
        id: u64,
        kind: InstrumentKind,
    },
    Disconnect(u64),
    Command(u64, ControlCommand),
    Measure(u64),
    /// Capture scope screen for in-app preview + clipboard (no save dialog).
    Capture { id: u64 },
    /// Read waveform source file over VISA (ISF/CSV), then UI prompts Save As.
    WaveformSource { id: u64, channel: u8 },
    Waveform {
        id: u64,
        channel: u8,
        points: usize,
    },
    Shutdown,
}

enum Event {
    Resources(Vec<ResourceInfo>),
    Connected {
        id: u64,
        resource: String,
        identity: Identity,
        kind: InstrumentKind,
        profile: String,
        capabilities: Capabilities,
    },
    Disconnected(u64),
    CommandDone {
        id: u64,
        response: Option<String>,
    },
    Measurements {
        id: u64,
        resource: String,
        readings: Vec<Reading>,
    },
    Screenshot {
        id: u64,
        width: usize,
        height: usize,
        rgba: Vec<u8>,
        png: Vec<u8>,
    },
    Waveform {
        id: u64,
        trace: WaveformTrace,
    },
    /// Raw waveform source file bytes from the instrument (ready for Save As).
    WaveformSource {
        id: u64,
        bytes: Vec<u8>,
        suggested_name: String,
    },
    Error {
        id: Option<u64>,
        message: String,
    },
}

/// Prebuilt plot series — rebuilt only when a new waveform arrives.
struct CachedWavePlot {
    channel: String,
    points: Arc<Vec<[f64; 2]>>,
}

/// Max points drawn in the scope plot (full CURVe data stays in `waveforms`).
const SCOPE_PLOT_DISPLAY_POINTS: usize = 1_024;
/// Longest edge for on-screen screenshot preview (full PNG kept for Save As).
const SCOPE_PREVIEW_MAX_EDGE: u32 = 1600;
/// Gap between the two scope rows (keep small so cards fill the screen).
const SCOPE_ROW_GAP: f32 = 8.0;
/// Gap between columns inside a row.
const SCOPE_COL_GAP: f32 = 8.0;
/// Uniform action-button size for scope toolbar / channel actions.
const SCOPE_BTN: egui::Vec2 = egui::vec2(72.0, 28.0);
const SCOPE_BTN_WIDE: egui::Vec2 = egui::vec2(88.0, 28.0);
const SCOPE_STEP_BTN: egui::Vec2 = egui::vec2(24.0, 24.0);
const SCOPE_POS_STEP: f64 = 0.25;
/// Tektronix-style vertical scale ladder (V/div).
const SCOPE_SCALE_STEPS: &[f64] = &[
    1e-3, 2e-3, 5e-3, 10e-3, 20e-3, 50e-3, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0,
];
/// Max independent DC source channels in the UI (DP832=3, modular up to 4).
const MAX_SOURCE_CHANNELS: usize = 4;

#[derive(Clone, Copy)]
enum InstrumentPanelBody {
    LoadControl,
    LoadReadings,
    LoadInfo,
    DmmSetup,
    DmmReading,
    DmmInfo,
    Scpi,
}

#[derive(Debug)]
struct ControlState {
    scope_channel: u8,
    /// CH1..=CH4 display enable (Tektronix SELect:CHx).
    scope_channel_on: [bool; 4],
    /// Per-channel vertical scale (V/div).
    scope_scales: [f64; 4],
    /// Per-channel vertical position (divisions).
    scope_positions: [f64; 4],
    /// Per-channel measure type selection.
    scope_meas_types: [ScopeMeasType; 4],
    /// Last measure readout per channel.
    scope_meas_results: [Option<String>; 4],
    scope_timebase: f64,
    trigger_source: String,
    trigger_level: f64,
    trigger_slope: String,
    /// Per-channel voltage / current setpoints and output state.
    source_voltages: [f64; MAX_SOURCE_CHANNELS],
    source_currents: [f64; MAX_SOURCE_CHANNELS],
    source_ovps: [f64; MAX_SOURCE_CHANNELS],
    source_ocps: [f64; MAX_SOURCE_CHANNELS],
    source_outputs: [bool; MAX_SOURCE_CHANNELS],
    load_mode: String,
    load_level: f64,
    load_input: bool,
    dmm_function: MeasureFunction,
    dmm_autorange: bool,
    dmm_range: f64,
    dmm_nplc: f64,
    console: String,
}

impl Default for ControlState {
    fn default() -> Self {
        Self {
            scope_channel: 1,
            scope_channel_on: [true, false, false, false],
            scope_scales: [1.0, 1.0, 1.0, 1.0],
            scope_positions: [0.0, 0.0, 0.0, 0.0],
            scope_meas_types: [ScopeMeasType::Frequency; 4],
            scope_meas_results: [None, None, None, None],
            scope_timebase: 0.001,
            trigger_source: "CH1".into(),
            trigger_level: 0.0,
            trigger_slope: "RISE".into(),
            source_voltages: [5.0; MAX_SOURCE_CHANNELS],
            source_currents: [1.0; MAX_SOURCE_CHANNELS],
            source_ovps: [6.0; MAX_SOURCE_CHANNELS],
            source_ocps: [1.2; MAX_SOURCE_CHANNELS],
            source_outputs: [false; MAX_SOURCE_CHANNELS],
            load_mode: "CC".into(),
            load_level: 1.0,
            load_input: false,
            dmm_function: MeasureFunction::DcVoltage,
            dmm_autorange: true,
            dmm_range: 10.0,
            dmm_nplc: 1.0,
            console: "*IDN?".into(),
        }
    }
}

struct DeviceUi {
    id: u64,
    resource: String,
    identity: Identity,
    kind: InstrumentKind,
    profile: String,
    capabilities: Capabilities,
    controls: ControlState,
    acquiring: bool,
    paused: bool,
}

pub struct InstrumentControlPanel {
    tx: Sender<Job>,
    rx: Receiver<Event>,
    resources: Vec<ResourceInfo>,
    resource_inputs: [String; 4],
    devices: Vec<DeviceUi>,
    selected_kind: InstrumentKind,
    selected_id: Option<u64>,
    next_id: u64,
    status: String,
    timeout_ms: u32,
    visa_library: String,
    sample_interval_ms: u64,
    max_points: usize,
    save_dir: PathBuf,
    last_sample: Instant,
    measurement_pending: HashSet<u64>,
    samples: AcquisitionBuffer,
    latest: HashMap<(u64, String), Reading>,
    waveforms: HashMap<u64, WaveformTrace>,
    wave_plots: HashMap<u64, CachedWavePlot>,
    screenshots: HashMap<u64, egui::TextureHandle>,
    /// Full PNG bytes for Save As (populated by Screenshot capture).
    screenshot_png: HashMap<u64, Vec<u8>>,
    /// Skip heavy waveform/image widgets for N frames after selecting the scope card.
    scope_heavy_defer_frames: u8,
    logs: VecDeque<String>,
    scanning: bool,
    /// When set, every instrument card is shown and missing kinds use demo sessions.
    debug_mode: bool,
}

impl InstrumentControlPanel {
    pub fn new(cfg: &AppConfig) -> Self {
        let (tx, jobs) = unbounded();
        let (events, rx) = unbounded();
        thread::spawn(move || worker_loop(jobs, events));
        let instrument_cfg = &cfg.apps.instruments;
        let mut resource_inputs: [String; 4] = std::array::from_fn(|_| String::new());
        if let Some(first) = instrument_cfg.known_tcpip_resources.first() {
            // Seed only until a scan classifies and redistributes devices.
            resource_inputs[0] = first.clone();
        }
        let panel = Self {
            tx,
            rx,
            resources: instrument_cfg
                .known_tcpip_resources
                .iter()
                .map(|address| ResourceInfo {
                    address: address.clone(),
                    transport: "TCPIP".into(),
                    kind: None,
                    identity: None,
                    probe_error: None,
                })
                .collect(),
            resource_inputs,
            devices: Vec::new(),
            selected_kind: InstrumentKind::Oscilloscope,
            selected_id: None,
            next_id: 1,
            status: local(cfg.ui.language.starts_with("zh"), "就绪", "Ready").into(),
            timeout_ms: instrument_cfg.timeout_ms,
            visa_library: instrument_cfg.visa_library.clone(),
            sample_interval_ms: instrument_cfg.sample_interval_ms.max(100),
            max_points: instrument_cfg.max_points.max(100),
            save_dir: wiparse_core::paths::project_path(&instrument_cfg.save_dir),
            last_sample: Instant::now(),
            measurement_pending: HashSet::new(),
            samples: AcquisitionBuffer::new(instrument_cfg.max_points),
            latest: HashMap::new(),
            waveforms: HashMap::new(),
            wave_plots: HashMap::new(),
            screenshots: HashMap::new(),
            screenshot_png: HashMap::new(),
            scope_heavy_defer_frames: 0,
            logs: VecDeque::new(),
            scanning: false,
            debug_mode: false,
        };
        let _ = std::fs::create_dir_all(&panel.save_dir);
        panel
    }

    /// Enable/disable debug workbench: show all instrument cards and open demo sessions.
    pub fn apply_debug_mode(&mut self, enabled: bool, lang: Lang) {
        if self.debug_mode == enabled {
            return;
        }
        self.debug_mode = enabled;
        if enabled {
            self.ensure_demo_devices(lang);
            self.status = text(
                lang,
                "调试模式：已显示全部仪表卡片（演示连接）",
                "Debug mode: all instrument cards shown (demo sessions)",
            )
            .into();
        } else {
            self.disconnect_demo_devices();
            self.status = text(lang, "调试模式已关闭", "Debug mode off").into();
        }
    }

    fn ensure_demo_devices(&mut self, lang: Lang) {
        for kind in [
            InstrumentKind::Oscilloscope,
            InstrumentKind::DcSource,
            InstrumentKind::ElectronicLoad,
            InstrumentKind::Multimeter,
        ] {
            let has_live = self.devices.iter().any(|device| device.kind == kind);
            if has_live {
                continue;
            }
            self.begin_connect_demo(kind, lang);
        }
    }

    fn disconnect_demo_devices(&mut self) {
        let demo_ids: Vec<u64> = self
            .devices
            .iter()
            .filter(|device| device.resource.starts_with("DEMO::"))
            .map(|device| device.id)
            .collect();
        for id in demo_ids {
            let _ = self.tx.send(Job::Disconnect(id));
        }
    }

    fn visible_instrument_kinds(&self) -> Vec<InstrumentKind> {
        const ALL: [InstrumentKind; 4] = [
            InstrumentKind::Oscilloscope,
            InstrumentKind::DcSource,
            InstrumentKind::ElectronicLoad,
            InstrumentKind::Multimeter,
        ];
        if self.debug_mode {
            return ALL.to_vec();
        }
        let mut kinds = Vec::new();
        for kind in ALL {
            let connected = self.devices.iter().any(|device| device.kind == kind);
            let matched = self.resources.iter().any(|item| item.kind == Some(kind));
            if connected || matched {
                kinds.push(kind);
            }
        }
        // No classified devices yet: keep every card so the user can connect manually.
        if kinds.is_empty() {
            return ALL.to_vec();
        }
        if !kinds.contains(&self.selected_kind) {
            kinds.push(self.selected_kind);
            kinds.sort_by_key(|kind| instrument_kind_slot(*kind));
        }
        kinds
    }

    pub fn pump(&mut self, ctx: &egui::Context) {
        self.pump_with_bus(ctx, None);
    }

    pub fn pump_with_bus(&mut self, ctx: &egui::Context, bus: Option<&crate::backend::EventBus>) {
        while let Ok(event) = self.rx.try_recv() {
            if let Some(bus) = bus {
                publish_instrument_event(bus, &event);
            }
            self.handle_event(ctx, event);
        }
        if self.live_active()
            && self.last_sample.elapsed() >= Duration::from_millis(self.sample_interval_ms)
        {
            self.last_sample = Instant::now();
            for device in self
                .devices
                .iter()
                .filter(|device| device.acquiring && !device.paused)
            {
                if self.measurement_pending.insert(device.id) {
                    let _ = self.tx.send(Job::Measure(device.id));
                }
            }
        }
        if self.scanning {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }

    pub fn device_count(&self) -> usize {
        self.devices.len()
    }

    pub fn api_list(&self) -> serde_json::Value {
        let devices: Vec<_> = self
            .devices
            .iter()
            .map(|d| {
                serde_json::json!({
                    "device_id": d.id,
                    "resource": d.resource,
                    "kind": d.kind,
                    "profile": d.profile,
                    "identity": d.identity,
                    "capabilities": d.capabilities,
                    "acquiring": d.acquiring,
                })
            })
            .collect();
        serde_json::json!({ "devices": devices, "scanning": self.scanning })
    }

    pub fn api_scan(&mut self, _params: &serde_json::Value) -> crate::backend::InvokeReply {
        use crate::backend::{invoke_err as err, invoke_ok as ok};
        self.scanning = true;
        match self.tx.send(Job::Scan {
            library: self.visa_library.clone(),
            timeout_ms: self.timeout_ms,
        }) {
            Ok(()) => ok("instrument.scan", serde_json::json!({ "accepted": true })),
            Err(e) => err("instrument.scan", &e.to_string()),
        }
    }

    pub fn api_connect(
        &mut self,
        params: &serde_json::Value,
        lang: Lang,
    ) -> crate::backend::InvokeReply {
        use crate::backend::{invoke_err as err, invoke_ok as ok};
        let resource = match params.get("resource").and_then(|v| v.as_str()) {
            Some(r) if !r.is_empty() => r.to_string(),
            _ => return err("instrument.connect", "missing resource"),
        };
        let kind = params
            .get("kind")
            .and_then(|v| serde_json::from_value::<InstrumentKind>(v.clone()).ok());
        let id = self.next_id;
        self.next_id += 1;
        match self.tx.send(Job::Connect {
            id,
            resource: resource.clone(),
            kind,
            timeout_ms: self.timeout_ms,
            library: self.visa_library.clone(),
        }) {
            Ok(()) => {
                self.status = text(lang, "正在连接…", "Connecting…").into();
                ok(
                    "instrument.connect",
                    serde_json::json!({ "accepted": true, "device_id": id, "resource": resource }),
                )
            }
            Err(e) => err("instrument.connect", &e.to_string()),
        }
    }

    pub fn api_disconnect(&mut self, params: &serde_json::Value) -> crate::backend::InvokeReply {
        use crate::backend::{invoke_err as err, invoke_ok as ok};
        let id = match params.get("device_id").and_then(|v| v.as_u64()) {
            Some(id) => id,
            None => return err("instrument.disconnect", "missing device_id"),
        };
        match self.tx.send(Job::Disconnect(id)) {
            Ok(()) => ok(
                "instrument.disconnect",
                serde_json::json!({ "accepted": true, "device_id": id }),
            ),
            Err(e) => err("instrument.disconnect", &e.to_string()),
        }
    }

    pub fn api_command(&mut self, params: &serde_json::Value) -> crate::backend::InvokeReply {
        use crate::backend::{invoke_err as err, invoke_ok as ok};
        let id = match params.get("device_id").and_then(|v| v.as_u64()) {
            Some(id) => id,
            None => return err("instrument.command", "missing device_id"),
        };
        let command = match params.get("command") {
            Some(c) => match serde_json::from_value::<ControlCommand>(c.clone()) {
                Ok(cmd) => cmd,
                Err(e) => return err("instrument.command", &format!("invalid command: {e}")),
            },
            None => return err("instrument.command", "missing command"),
        };
        match self.tx.send(Job::Command(id, command)) {
            Ok(()) => ok(
                "instrument.command",
                serde_json::json!({ "accepted": true, "device_id": id }),
            ),
            Err(e) => err("instrument.command", &e.to_string()),
        }
    }

    pub fn api_measure(&mut self, params: &serde_json::Value) -> crate::backend::InvokeReply {
        use crate::backend::{invoke_err as err, invoke_ok as ok};
        let id = match params.get("device_id").and_then(|v| v.as_u64()) {
            Some(id) => id,
            None => return err("instrument.measure", "missing device_id"),
        };
        self.measurement_pending.insert(id);
        match self.tx.send(Job::Measure(id)) {
            Ok(()) => ok(
                "instrument.measure",
                serde_json::json!({ "accepted": true, "device_id": id }),
            ),
            Err(e) => err("instrument.measure", &e.to_string()),
        }
    }

    pub fn api_capture(&mut self, params: &serde_json::Value) -> crate::backend::InvokeReply {
        use crate::backend::{invoke_err as err, invoke_ok as ok};
        let id = match params.get("device_id").and_then(|v| v.as_u64()) {
            Some(id) => id,
            None => return err("instrument.capture", "missing device_id"),
        };
        match self.tx.send(Job::Capture { id }) {
            Ok(()) => ok(
                "instrument.capture",
                serde_json::json!({ "accepted": true, "device_id": id }),
            ),
            Err(e) => err("instrument.capture", &e.to_string()),
        }
    }

    pub fn api_waveform(&mut self, params: &serde_json::Value) -> crate::backend::InvokeReply {
        use crate::backend::{invoke_err as err, invoke_ok as ok};
        let id = match params.get("device_id").and_then(|v| v.as_u64()) {
            Some(id) => id,
            None => return err("instrument.waveform", "missing device_id"),
        };
        let channel = params.get("channel").and_then(|v| v.as_u64()).unwrap_or(1) as u8;
        let points = params
            .get("points")
            .and_then(|v| v.as_u64())
            .unwrap_or(1000) as usize;
        match self.tx.send(Job::Waveform {
            id,
            channel,
            points,
        }) {
            Ok(()) => ok(
                "instrument.waveform",
                serde_json::json!({ "accepted": true, "device_id": id, "channel": channel }),
            ),
            Err(e) => err("instrument.waveform", &e.to_string()),
        }
    }

    fn handle_event(&mut self, ctx: &egui::Context, event: Event) {
        match event {
            Event::Resources(resources) => {
                self.scanning = false;
                self.apply_discovered_resources(resources);
            }
            Event::Connected {
                id,
                resource,
                identity,
                kind,
                profile,
                capabilities,
            } => {
                self.status = format!("Connected: {}", identity.raw);
                self.log(format!("CONNECT {resource} — {}", identity.raw));
                self.devices.push(DeviceUi {
                    id,
                    resource,
                    identity,
                    kind,
                    profile,
                    capabilities,
                    controls: ControlState::default(),
                    acquiring: false,
                    paused: false,
                });
                self.select_kind(kind);
                self.selected_id = Some(id);
            }
            Event::Disconnected(id) => {
                self.devices.retain(|device| device.id != id);
                self.measurement_pending.remove(&id);
                self.waveforms.remove(&id);
                self.wave_plots.remove(&id);
                self.screenshots.remove(&id);
                self.screenshot_png.remove(&id);
                self.selected_id = self
                    .selected_id
                    .filter(|selected| *selected != id)
                    .or_else(|| {
                        self.devices
                            .iter()
                            .find(|device| device.kind == self.selected_kind)
                            .map(|device| device.id)
                    });
                self.status = "Disconnected".into();
                self.log(format!("DISCONNECT #{id}"));
            }
            Event::CommandDone { id, response } => {
                if let Some(response) = response {
                    if let Some(device) = self.devices.iter_mut().find(|device| device.id == id) {
                        store_scope_measure_result(&mut device.controls, &response);
                    }
                    self.status = response.clone();
                    self.log(format!("#{id} ◀ {response}"));
                } else {
                    self.status = "Command completed".into();
                }
            }
            Event::Measurements {
                id,
                resource,
                readings,
            } => {
                self.measurement_pending.remove(&id);
                for reading in readings {
                    self.samples.push(Sample::value(
                        &resource,
                        &reading.channel,
                        reading.value,
                        &reading.unit,
                    ));
                    self.latest.insert((id, reading.channel.clone()), reading);
                }
                self.status = format!("Updated {}", Local::now().format("%H:%M:%S"));
            }
            Event::Screenshot {
                id,
                width,
                height,
                rgba,
                png,
            } => {
                let color = egui::ColorImage::from_rgba_unmultiplied([width, height], &rgba);
                ctx.copy_image(color.clone());
                self.screenshots.insert(
                    id,
                    ctx.load_texture(
                        format!("instrument-shot-{id}"),
                        color,
                        Default::default(),
                    ),
                );
                self.screenshot_png.insert(id, png);
                self.status =
                    "截图已显示并复制到剪贴板 / Screenshot copied to clipboard".into();
            }
            Event::Waveform { id, trace } => {
                self.status = format!("{}: {} points", trace.channel, trace.x.len());
                self.wave_plots
                    .insert(id, build_cached_wave_plot(&trace, SCOPE_PLOT_DISPLAY_POINTS));
                self.waveforms.insert(id, trace);
            }
            Event::WaveformSource {
                id,
                bytes,
                suggested_name,
            } => {
                let stem = PathBuf::from(&suggested_name);
                let ext = stem
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("isf")
                    .to_ascii_lowercase();
                let channel = stem
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .and_then(|s| s.strip_prefix("waveform_"))
                    .unwrap_or("CH1");

                // Parse in memory and show full screen record in ③ Waveform panel.
                if let Ok(trace) = load_waveform_bytes(&bytes, &ext, channel) {
                    self.status = format!(
                        "屏幕波形 {} · N={} / Screen waveform {} · N={}",
                        trace.channel,
                        trace.x.len(),
                        trace.channel,
                        trace.x.len()
                    );
                    self.wave_plots.insert(
                        id,
                        build_cached_wave_plot(&trace, SCOPE_PLOT_DISPLAY_POINTS),
                    );
                    self.waveforms.insert(id, trace);
                }

                // Auto Save As as soon as VISA transfer completes.
                let default = self.save_dir.join(format!(
                    "{}_{}.{}",
                    stem.file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("waveform"),
                    Local::now().format("%Y%m%d_%H%M%S"),
                    ext
                ));
                let mut dialog = rfd::FileDialog::new()
                    .set_directory(&self.save_dir)
                    .set_file_name(default.file_name().unwrap_or_default().to_string_lossy())
                    .add_filter("Tektronix ISF", &["isf"])
                    .add_filter("Tektronix WFM", &["wfm"])
                    .add_filter("CSV", &["csv"])
                    .add_filter("All", &["*"]);
                if let Some(path) = dialog.save_file() {
                    match save_waveform_file(&path, Some(&bytes), Some(&ext), None) {
                        Ok(()) => {
                            self.status = format!(
                                "已保存波形源 / Waveform source saved: {}",
                                path.display()
                            );
                            if let Some(parent) = path.parent() {
                                self.save_dir = parent.to_path_buf();
                            }
                        }
                        Err(error) => self.status = error.to_string(),
                    }
                } else if self.waveforms.contains_key(&id) {
                    self.status =
                        "已解析屏幕波形，保存已取消 / Screen waveform parsed, save cancelled"
                            .into();
                } else {
                    self.status =
                        "已取消保存波形源 / Waveform source save cancelled".into();
                }
            }
            Event::Error { id, message } => {
                if id.is_none() {
                    self.scanning = false;
                }
                if let Some(id) = id {
                    self.measurement_pending.remove(&id);
                    if self
                        .devices
                        .iter()
                        .any(|device| device.id == id && device.acquiring)
                    {
                        let resource = self
                            .devices
                            .iter()
                            .find(|device| device.id == id)
                            .map(|device| device.resource.clone())
                            .unwrap_or_default();
                        self.samples.push(Sample::error(resource, &message));
                    }
                }
                self.status = message.clone();
                self.log(format!("ERROR {message}"));
            }
        }
    }

    pub fn live_active(&self) -> bool {
        self.devices
            .iter()
            .any(|device| device.acquiring && !device.paused)
    }

    pub fn status_text(&self) -> &str {
        &self.status
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        // `pump` is called once from the app update loop while this tab is active.
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(290.0, ui.available_height()),
                egui::Layout::top_down(egui::Align::Min),
                |ui| self.device_rack(ui, lang, tokens),
            );
            ui.separator();
            ui.vertical(|ui| self.workspace(ui, lang, tokens));
        });
    }

    fn select_kind(&mut self, kind: InstrumentKind) {
        let kind_changed = self.selected_kind != kind;
        self.selected_kind = kind;
        self.selected_id = self
            .selected_id
            .filter(|id| {
                self.devices
                    .iter()
                    .any(|device| device.id == *id && device.kind == kind)
            })
            .or_else(|| {
                self.devices
                    .iter()
                    .find(|device| device.kind == kind)
                    .map(|device| device.id)
            });
        // Defer waveform/image for a couple frames so card clicks stay responsive.
        if kind_changed {
            self.scope_heavy_defer_frames = if kind == InstrumentKind::Oscilloscope {
                2
            } else {
                0
            };
        }
    }

    fn apply_discovered_resources(&mut self, resources: Vec<ResourceInfo>) {
        for resource in resources {
            if let Some(existing) = self
                .resources
                .iter_mut()
                .find(|item| item.address == resource.address)
            {
                *existing = resource;
            } else {
                self.resources.push(resource);
            }
        }
        self.resources.sort_by(|a, b| {
            kind_sort_key(a.kind)
                .cmp(&kind_sort_key(b.kind))
                .then_with(|| a.address.cmp(&b.address))
        });

        let mut assigned = [false; 4];
        for resource in &self.resources {
            let Some(kind) = resource.kind else {
                continue;
            };
            if matches!(kind, InstrumentKind::Generic) {
                continue;
            }
            let slot = instrument_kind_slot(kind);
            if assigned[slot] {
                continue;
            }
            self.resource_inputs[slot] = resource.address.clone();
            assigned[slot] = true;
        }

        // Fallback: first unidentified resource fills the selected card so Connect
        // remains usable when *IDN? probe fails.
        let selected_slot = instrument_kind_slot(self.selected_kind);
        if !assigned[selected_slot] && self.resource_inputs[selected_slot].trim().is_empty() {
            if let Some(resource) = self.resources.iter().find(|item| item.kind.is_none()) {
                self.resource_inputs[selected_slot] = resource.address.clone();
            }
        }

        let identified = self.resources.iter().filter(|item| item.kind.is_some()).count();
        let failed = self
            .resources
            .iter()
            .filter(|item| item.probe_error.is_some())
            .count();
        self.status = if identified > 0 {
            format!(
                "{} resource(s), {} identified, {} probe failed",
                self.resources.len(),
                identified,
                failed
            )
        } else {
            format!(
                "{} resource(s) — select address in card, then Connect",
                self.resources.len()
            )
        };
        self.log(format!(
            "SCAN {} resource(s); auto-assigned to cards by *IDN?",
            self.resources.len()
        ));
    }

    fn resolve_card_resource(&self, kind: InstrumentKind) -> Option<String> {
        let slot = instrument_kind_slot(kind);
        let typed = self.resource_inputs[slot].trim();
        if !typed.is_empty() {
            return Some(typed.to_owned());
        }
        self.resources
            .iter()
            .find(|item| item.kind == Some(kind))
            .or_else(|| self.resources.first())
            .map(|item| item.address.clone())
    }

    fn begin_connect(&mut self, kind: InstrumentKind, lang: Lang) {
        let Some(resource) = self.resolve_card_resource(kind) else {
            self.status = text(
                lang,
                "无可用 VISA 资源，请先扫描或手动输入地址",
                "No VISA resource available. Scan or enter an address first.",
            )
            .into();
            return;
        };
        let slot = instrument_kind_slot(kind);
        self.resource_inputs[slot] = resource.clone();
        let id = self.next_id;
        self.next_id += 1;
        self.select_kind(kind);
        self.status = text(lang, "正在连接…", "Connecting…").into();
        let _ = self.tx.send(Job::Connect {
            id,
            resource,
            // Let *IDN? classify the live session; UI then jumps to the detected card.
            kind: None,
            timeout_ms: self.timeout_ms,
            library: self.visa_library.clone(),
        });
    }

    fn begin_connect_demo(&mut self, kind: InstrumentKind, lang: Lang) {
        let id = self.next_id;
        self.next_id += 1;
        self.select_kind(kind);
        self.status = text(lang, "正在打开模拟连接…", "Opening demo connection…").into();
        let _ = self.tx.send(Job::ConnectDemo { id, kind });
    }

    fn active_device_index(&self) -> Option<usize> {
        if let Some(id) = self.selected_id {
            if let Some(index) = self
                .devices
                .iter()
                .position(|device| device.id == id && device.kind == self.selected_kind)
            {
                return Some(index);
            }
        }
        self.devices
            .iter()
            .position(|device| device.kind == self.selected_kind)
    }

    fn discovered_resources_panel(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        Frame::NONE
            .fill(tokens.surface_bg)
            .stroke(Stroke::new(1.0_f32, tokens.border))
            .corner_radius(CornerRadius::same(7))
            .inner_margin(Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.set_width(CARD_PANEL_WIDTH);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(text(lang, "已发现资源", "Discovered"))
                            .strong()
                            .size(12.5),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("{}", self.resources.len()))
                                .small()
                                .color(tokens.text_muted),
                        );
                    });
                });
                ui.add_space(6.0);
                egui::ScrollArea::vertical()
                    .id_salt("discovered-resources")
                    .max_height(150.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.set_width(CARD_PANEL_WIDTH - 8.0);
                        let mut fill_action: Option<(InstrumentKind, String)> = None;
                        let mut connect_action: Option<(InstrumentKind, String)> = None;
                        for i in 0..self.resources.len() {
                            if i > 0 {
                                ui.add_space(6.0);
                                ui.separator();
                                ui.add_space(4.0);
                            }
                            let resource = &self.resources[i];
                            let target = resource.kind.unwrap_or(self.selected_kind);
                            let kind_label = resource
                                .kind
                                .map(|kind| instrument_name(lang, kind))
                                .unwrap_or(text(lang, "未识别", "Unknown"));
                            let model = resource
                                .identity
                                .as_ref()
                                .map(|id| id.model.as_str())
                                .unwrap_or("");
                            ui.label(
                                RichText::new(kind_label)
                                    .small()
                                    .strong()
                                    .color(tokens.accent),
                            );
                            ui.label(
                                RichText::new(short_resource(&resource.address))
                                    .small()
                                    .monospace(),
                            );
                            if !model.is_empty() {
                                ui.label(
                                    RichText::new(model).small().color(tokens.text_muted),
                                );
                            }
                            if let Some(error) = &resource.probe_error {
                                ui.label(
                                    RichText::new(error)
                                        .small()
                                        .color(Color32::from_rgb(0xF5, 0x9E, 0x0B)),
                                );
                            }
                            ui.add_space(3.0);
                            ui.horizontal(|ui| {
                                if ui
                                    .add_sized(
                                        [ui.available_width() * 0.48, 22.0],
                                        egui::Button::new(text(lang, "填入", "Fill")),
                                    )
                                    .clicked()
                                {
                                    fill_action = Some((target, resource.address.clone()));
                                }
                                if ui
                                    .add_sized(
                                        [ui.available_width(), 22.0],
                                        egui::Button::new(text(lang, "连接", "Connect"))
                                            .fill(tokens.accent),
                                    )
                                    .clicked()
                                {
                                    connect_action = Some((target, resource.address.clone()));
                                }
                            });
                        }
                        if let Some((target, address)) = fill_action {
                            let slot = instrument_kind_slot(target);
                            self.resource_inputs[slot] = address;
                            self.select_kind(target);
                        }
                        if let Some((target, address)) = connect_action {
                            let slot = instrument_kind_slot(target);
                            self.resource_inputs[slot] = address;
                            self.begin_connect(target, lang);
                        }
                    });
            });
    }

    fn device_rack(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        ui.set_min_height(ui.available_height());
        Frame::NONE
            .fill(tokens.panel_bg)
            .stroke(Stroke::new(1.0_f32, tokens.border))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(Margin::same(10))
            .show(ui, |ui| {
                ui.set_min_height(ui.available_height());
                ui.heading(text(lang, "仪器设备", "Instruments"));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !self.scanning,
                            egui::Button::new(text(lang, "扫描 USB/LAN", "Scan USB/LAN")),
                        )
                        .clicked()
                    {
                        self.scanning = true;
                        self.status = text(
                            lang,
                            "正在扫描并识别设备…",
                            "Scanning and identifying…",
                        )
                        .into();
                        let _ = self.tx.send(Job::Scan {
                            library: self.visa_library.clone(),
                            timeout_ms: self.timeout_ms,
                        });
                    }
                    if self.scanning {
                        ui.spinner().on_hover_text(text(
                            lang,
                            "扫描后通过 *IDN? 识别类型并分发到对应卡片",
                            "After scan, *IDN? classifies devices onto matching cards",
                        ));
                    }
                });
                ui.label(
                    RichText::new(text(
                        lang,
                        "扫描后自动识别类型并填入对应卡片；也可手动选择地址后连接",
                        "Scan auto-classifies and fills cards; or pick an address manually",
                    ))
                    .small()
                    .color(tokens.text_muted),
                );
                if !self.resources.is_empty() {
                    ui.add_space(4.0);
                    self.discovered_resources_panel(ui, lang, tokens);
                }
                let cards_height = ui.available_height().max(160.0);
                egui::ScrollArea::vertical()
                    .id_salt("instrument-type-cards")
                    .max_height(cards_height)
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        for kind in self.visible_instrument_kinds() {
                            let matching_count = self
                                .devices
                                .iter()
                                .filter(|device| device.kind == kind)
                                .count();
                            let matching_device = self
                                .devices
                                .iter()
                                .find(|device| device.kind == kind)
                                .map(|device| (device.id, device.identity.model.clone()));
                            let input_slot = instrument_kind_slot(kind);
                            let action = instrument_type_card(
                                ui,
                                lang,
                                tokens,
                                kind,
                                self.selected_kind == kind,
                                matching_device.as_ref().map(|(_, model)| model.as_str()),
                                matching_count,
                                matching_device.as_ref().map(|(id, _)| *id),
                                &mut self.resource_inputs[input_slot],
                                &self.resources,
                            );
                            if action.selected && self.selected_kind != kind {
                                self.select_kind(kind);
                            }
                            if action.connect {
                                self.begin_connect(kind, lang);
                            }
                            if let Some(id) = action.disconnect {
                                let _ = self.tx.send(Job::Disconnect(id));
                            }
                            ui.add_space(7.0);
                        }
                    });
                ui.separator();
                ui.label(RichText::new(&self.status).small().color(tokens.text_muted));
            });
    }

    fn workspace(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        let Some(index) = self.active_device_index() else {
            self.empty_instrument_workspace(ui, lang, tokens);
            return;
        };
        let id = self.devices[index].id;
        self.selected_id = Some(id);
        let same_kind_count = self
            .devices
            .iter()
            .filter(|device| device.kind == self.selected_kind)
            .count();
        ui.horizontal(|ui| {
            let device = &self.devices[index];
            ui.heading(format!(
                "{} — {}",
                device.kind.label(),
                device.identity.model
            ));
            ui.label(RichText::new(&device.identity.manufacturer).color(tokens.text_muted));
            if same_kind_count > 1 {
                let models: Vec<(u64, String)> = self
                    .devices
                    .iter()
                    .filter(|device| device.kind == self.selected_kind)
                    .map(|device| (device.id, device.identity.model.clone()))
                    .collect();
                let selected_model = self.devices[index].identity.model.clone();
                egui::ComboBox::from_id_salt("same-kind-device")
                    .selected_text(selected_model)
                    .show_ui(ui, |ui| {
                        for (device_id, model) in models {
                            ui.selectable_value(&mut self.selected_id, Some(device_id), model);
                        }
                    });
            }
        });
        ui.separator();

        let kind = self.devices[index].kind;
        match kind {
            InstrumentKind::Oscilloscope => self.scope_workspace(ui, lang, tokens, id, index),
            InstrumentKind::DcSource => self.source_workspace(ui, lang, tokens, id, index),
            InstrumentKind::ElectronicLoad => self.load_workspace(ui, lang, tokens, id, index),
            InstrumentKind::Multimeter => self.dmm_workspace(ui, lang, tokens, id, index),
            InstrumentKind::Generic => {
                self.generic_workspace(ui, lang, tokens, id, index, kind);
            }
        }
    }

    fn dispatch_scope_commands(&mut self, id: u64, commands: Vec<ControlCommand>) {
        for command in commands {
            self.log_command(id, &command);
            let _ = self.tx.send(Job::Command(id, command));
        }
    }

    fn generic_workspace(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        tokens: &Tokens,
        id: u64,
        index: usize,
        kind: InstrumentKind,
    ) {
        egui::ScrollArea::vertical()
            .id_salt(format!("instrument-workspace-{id}"))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.columns(3, |columns| {
                    card(
                        &mut columns[0],
                        tokens,
                        text(lang, "参数设置", "Parameters"),
                        |ui| {
                            self.instrument_parameters_ui(ui, lang, tokens, id, index);
                            self.settings_ui(ui, lang, tokens, id, index);
                        },
                    );
                    card(
                        &mut columns[1],
                        tokens,
                        text(lang, "控制", "Control"),
                        |ui| {
                            let (commands, measure_once) =
                                control_ui(ui, lang, tokens, &mut self.devices[index]);
                            for command in commands {
                                self.log_command(id, &command);
                                let _ = self.tx.send(Job::Command(id, command));
                            }
                            if measure_once {
                                self.measurement_pending.insert(id);
                                let _ = self.tx.send(Job::Measure(id));
                            }
                        },
                    );
                    card(
                        &mut columns[2],
                        tokens,
                        text(lang, "数据采集", "Data Acquisition"),
                        |ui| {
                            ui.label(
                                RichText::new(instrument_acquisition_hint(lang, kind))
                                    .small()
                                    .color(tokens.text_muted),
                            );
                            ui.add_space(6.0);
                            self.acquisition_ui(ui, lang, tokens, id, index);
                        },
                    );
                });
                card(ui, tokens, "SCPI", |ui| {
                    self.console_ui(ui, lang, tokens, id, index);
                });
            });
    }

    fn scope_workspace(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        tokens: &Tokens,
        id: u64,
        index: usize,
    ) {
        let defer_heavy = self.scope_heavy_defer_frames > 0;
        if defer_heavy {
            self.scope_heavy_defer_frames = self.scope_heavy_defer_frames.saturating_sub(1);
            ui.ctx().request_repaint();
        }

        // Fill the visible workspace exactly: 2×2 grid, no outer scroll overflow.
        // Fixed column widths (shared by both rows) — avoid egui::columns content sizing drift.
        let avail = ui.available_size();
        let row1_h = ((avail.y - SCOPE_ROW_GAP) * 0.5).floor().max(1.0);
        let row2_h = (avail.y - SCOPE_ROW_GAP - row1_h).max(1.0);
        let (col_l, col_r) = instrument_row_column_widths(avail.x);

        instrument_grid_row(ui, avail.x, row1_h, col_l, col_r, |ui, left, right| {
            instrument_grid_cell(ui, left, tokens, text(lang, "① 示波器控制", "① Scope Control"), |ui| {
                egui::ScrollArea::vertical()
                    .id_salt(("scope-ctrl-scroll", id))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let commands =
                            scope_unified_controls(ui, lang, tokens, &mut self.devices[index]);
                        self.dispatch_scope_commands(id, commands);
                    });
            });
            instrument_grid_cell(ui, right, tokens, text(lang, "② 屏幕截图", "② Screen Capture"), |ui| {
                self.scope_screenshot_ui(ui, lang, tokens, id, index, defer_heavy);
            });
        });
        ui.add_space(SCOPE_ROW_GAP);
        instrument_grid_row(ui, avail.x, row2_h, col_l, col_r, |ui, left, right| {
            instrument_grid_cell(ui, left, tokens, text(lang, "③ 波形数据", "③ Waveform Samples"), |ui| {
                self.scope_waveform_data_ui(ui, lang, tokens, id, index, defer_heavy);
            });
            instrument_grid_cell(ui, right, tokens, text(lang, "④ SCPI 控制台", "④ SCPI Console"), |ui| {
                self.console_ui_compact(ui, lang, tokens, id, index);
            });
        });
    }

    fn scope_screenshot_ui(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        tokens: &Tokens,
        id: u64,
        index: usize,
        defer_heavy: bool,
    ) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let can_shot = self.devices[index].capabilities.screenshot;
            let has_png = self.screenshot_png.contains_key(&id);
            if ui
                .add_enabled(
                    can_shot,
                    egui::Button::new(text(lang, "屏幕截图", "Screenshot"))
                        .fill(tokens.accent)
                        .min_size(SCOPE_BTN_WIDE),
                )
                .clicked()
            {
                self.status = text(lang, "正在截取屏幕…", "Capturing screen…").into();
                let _ = self.tx.send(Job::Capture { id });
            }
            // Oscilloscope workbench always exposes VISA waveform-source read
            // (capability flag can be false on generic/demo profiles).
            let can_wave_src = self.devices[index].kind == InstrumentKind::Oscilloscope
                || self.devices[index].capabilities.waveform;
            if ui
                .add_enabled(
                    can_wave_src,
                    egui::Button::new(text(lang, "读取波形源文件", "Read Wave Source"))
                        .min_size(egui::vec2(128.0, 28.0)),
                )
                .on_hover_text(text(
                    lang,
                    "通过 VISA 读取屏幕完整波形（.isf/.wfm/.csv），完成后自动弹出另存为",
                    "Read full on-screen waveform via VISA (.isf/.wfm/.csv), then auto Save As",
                ))
                .clicked()
            {
                let channel = self.devices[index].controls.scope_channel;
                self.status =
                    text(lang, "正在通过 VISA 读取波形源文件…", "Reading waveform source via VISA…")
                        .into();
                let _ = self.tx.send(Job::WaveformSource { id, channel });
            }
            if ui
                .add_enabled(
                    has_png,
                    egui::Button::new(text(lang, "另存为…", "Save As…")).min_size(SCOPE_BTN),
                )
                .on_hover_text(text(
                    lang,
                    "将截图预览另存为 PNG",
                    "Save screenshot preview as PNG",
                ))
                .clicked()
            {
                let default = self.save_dir.join(format!(
                    "scope_{}.png",
                    Local::now().format("%Y%m%d_%H%M%S")
                ));
                if let Some(path) = rfd::FileDialog::new()
                    .set_directory(&self.save_dir)
                    .set_file_name(default.file_name().unwrap_or_default().to_string_lossy())
                    .add_filter("PNG", &["png"])
                    .save_file()
                {
                    if let Some(png) = self.screenshot_png.get(&id) {
                        match std::fs::write(&path, png) {
                            Ok(()) => {
                                self.status = format!(
                                    "{} {}",
                                    text(lang, "已保存", "Saved"),
                                    path.display()
                                );
                            }
                            Err(error) => self.status = error.to_string(),
                        }
                    }
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(text(
                        lang,
                        "预览 · 波形源",
                        "Preview · wave src",
                    ))
                    .small()
                    .color(tokens.text_muted),
                );
            });
        });
        ui.add_space(6.0);
        let preview_h = ui.available_height().max(80.0);
        if defer_heavy {
            media_slot_empty(ui, preview_h);
        } else if let Some(texture) = self.screenshots.get(&id) {
            paint_screenshot(ui, texture, preview_h);
        } else {
            placeholder_panel(
                ui,
                preview_h,
                text(
                    lang,
                    "点击「屏幕截图」抓取仪器画面",
                    "Click Screenshot for the scope display",
                ),
                tokens.text_muted,
            );
        }
    }

    fn scope_waveform_data_ui(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        tokens: &Tokens,
        id: u64,
        index: usize,
        defer_heavy: bool,
    ) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.label(RichText::new(text(lang, "通道", "CH")).strong());
            let max_ch = self.devices[index].capabilities.channels.max(1);
            let selected = format!("CH{}", self.devices[index].controls.scope_channel);
            egui::ComboBox::from_id_salt(("scope-wave-ch", id))
                .selected_text(selected)
                .width(72.0)
                .show_ui(ui, |ui| {
                    for n in 1..=max_ch {
                        ui.selectable_value(
                            &mut self.devices[index].controls.scope_channel,
                            n,
                            format!("CH{n}"),
                        );
                    }
                });
            if ui
                .add_sized(
                    SCOPE_BTN,
                    egui::Button::new(text(lang, "读取屏幕", "Read Screen")).fill(tokens.accent),
                )
                .on_hover_text(text(
                    lang,
                    "通过 VISA 读取屏幕完整波形（全部显示点，无抽样）",
                    "Read full on-screen waveform via VISA (all displayed points)",
                ))
                .clicked()
            {
                let channel = self.devices[index].controls.scope_channel;
                self.status = text(lang, "正在读取屏幕波形…", "Reading on-screen waveform…").into();
                let _ = self.tx.send(Job::Waveform {
                    id,
                    channel,
                    points: 0,
                });
            }
            let has_trace = self.waveforms.contains_key(&id);
            if ui
                .add_enabled(
                    has_trace,
                    egui::Button::new(text(lang, "导出…", "Export…")).min_size(SCOPE_BTN_WIDE),
                )
                .clicked()
            {
                if let Some(trace) = self.waveforms.get(&id) {
                    let default = self.save_dir.join(format!(
                        "waveform_{}_{}.isf",
                        trace.channel,
                        Local::now().format("%Y%m%d_%H%M%S")
                    ));
                    if let Some(path) = rfd::FileDialog::new()
                        .set_directory(&self.save_dir)
                        .set_file_name(default.file_name().unwrap_or_default().to_string_lossy())
                        .add_filter("Tektronix ISF", &["isf"])
                        .add_filter("Tektronix WFM", &["wfm"])
                        .add_filter("CSV", &["csv"])
                        .save_file()
                    {
                        match save_waveform_file(&path, None, None, Some(trace)) {
                            Ok(()) => {
                                self.status = format!(
                                    "{} {}",
                                    text(lang, "波形已导出", "Waveform exported"),
                                    path.display()
                                );
                            }
                            Err(error) => self.status = error.to_string(),
                        }
                    }
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(text(lang, "CURVe 采样", "CURVe samples"))
                        .small()
                        .color(tokens.text_muted),
                );
            });
        });

        if let Some(trace) = self.waveforms.get(&id) {
            let stats = waveform_stats(trace);
            ui.add_space(4.0);
            let stats_line = format!(
                "{}  N={}  Δt={:.3}{}  min={:.4}{}  max={:.4}{}  pp={:.4}{}  mean={:.4}{}",
                trace.channel,
                stats.count,
                stats.dt,
                trace.x_unit,
                stats.min,
                trace.y_unit,
                stats.max,
                trace.y_unit,
                stats.pp,
                trace.y_unit,
                stats.mean,
                trace.y_unit,
            );
            ui.add(
                egui::Label::new(
                    RichText::new(stats_line)
                        .small()
                        .monospace()
                        .color(tokens.text_muted),
                )
                .truncate(),
            );
        }

        ui.add_space(6.0);
        let preview_h = ui.available_height().max(80.0);
        if defer_heavy {
            media_slot_empty(ui, preview_h);
        } else if let Some(cached) = self.wave_plots.get(&id) {
            paint_waveform(ui, &cached.points, preview_h, tokens.accent);
        } else {
            placeholder_panel(
                ui,
                preview_h,
                text(
                    lang,
                    "读取后可看曲线、统计并导出 CSV",
                    "Read to plot, stats, and export CSV",
                ),
                tokens.text_muted,
            );
        }
    }

    fn console_ui_compact(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        tokens: &Tokens,
        id: u64,
        index: usize,
    ) {
        let panel_w = ui.available_width().max(1.0);
        ui.set_max_width(panel_w);

        let command = &mut self.devices[index].controls.console;
        let mut to_send = None;
        ui.horizontal(|ui| {
            ui.set_max_width(panel_w);
            ui.spacing_mut().item_spacing.x = 8.0;
            let send_w = SCOPE_BTN.x;
            let gap = ui.spacing().item_spacing.x;
            let edit_w = (ui.available_width() - send_w - gap).max(40.0);
            let response = ui.add(
                egui::TextEdit::singleline(command)
                    .desired_width(edit_w)
                    .hint_text("*IDN?"),
            );
            let send = ui
                .add_sized(SCOPE_BTN, egui::Button::new(text(lang, "发送", "Send")))
                .clicked()
                || (response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)));
            if send && !command.trim().is_empty() {
                let value = command.trim().to_owned();
                to_send = Some(if value.ends_with('?') {
                    ControlCommand::RawQuery(value)
                } else {
                    ControlCommand::RawWrite(value)
                });
            }
        });
        if let Some(scpi) = to_send {
            self.log_command(id, &scpi);
            let _ = self.tx.send(Job::Command(id, scpi));
        }
        ui.add_space(6.0);

        // Exact leftover viewport so ScrollArea cannot grow the card past its cell.
        let log_h = ui.available_height().max(1.0);
        let log_w = ui.available_width().min(panel_w).max(1.0);
        let (log_rect, _) =
            ui.allocate_exact_size(egui::vec2(log_w, log_h), egui::Sense::hover());
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(log_rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
            |ui| {
                ui.set_clip_rect(log_rect.intersect(ui.clip_rect()));
                ui.set_min_size(log_rect.size());
                ui.set_max_size(log_rect.size());
                egui::ScrollArea::vertical()
                    .id_salt(("scope-scpi-log", id))
                    .max_height(log_rect.height())
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        ui.set_max_width(log_rect.width());
                        if self.logs.is_empty() {
                            ui.label(
                                RichText::new(text(
                                    lang,
                                    "SCPI 收发日志显示在此",
                                    "SCPI traffic appears here",
                                ))
                                .small()
                                .color(tokens.text_muted),
                            );
                        } else {
                            for line in self.logs.iter().rev().take(80).rev() {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(line)
                                            .monospace()
                                            .small()
                                            .color(tokens.text_muted),
                                    )
                                    .truncate(),
                                );
                            }
                        }
                    });
            },
        );
    }

    fn source_workspace(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        tokens: &Tokens,
        id: u64,
        index: usize,
    ) {
        let avail = ui.available_size();
        let row1_h = ((avail.y - SCOPE_ROW_GAP) * 0.5).floor().max(1.0);
        let row2_h = (avail.y - SCOPE_ROW_GAP - row1_h).max(1.0);
        let (col_l, col_r) = instrument_row_column_widths(avail.x);

        instrument_grid_row(ui, avail.x, row1_h, col_l, col_r, |ui, left, right| {
            instrument_grid_cell(ui, left, tokens, text(lang, "① 通道输出", "① Channel Outputs"), |ui| {
                egui::ScrollArea::vertical()
                    .id_salt(("source-ch-scroll", id))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let commands =
                            source_channel_controls(ui, lang, tokens, &mut self.devices[index]);
                        self.dispatch_scope_commands(id, commands);
                    });
            });
            instrument_grid_cell(ui, right, tokens, text(lang, "② 实测读数", "② Measurements"), |ui| {
                self.source_readings_ui(ui, lang, tokens, id, index);
            });
        });
        ui.add_space(SCOPE_ROW_GAP);
        instrument_grid_row(ui, avail.x, row2_h, col_l, col_r, |ui, left, right| {
            instrument_grid_cell(ui, left, tokens, text(lang, "③ 保护设定", "③ Protection"), |ui| {
                egui::ScrollArea::vertical()
                    .id_salt(("source-prot-scroll", id))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let commands =
                            source_protection_controls(ui, lang, tokens, &mut self.devices[index]);
                        self.dispatch_scope_commands(id, commands);
                    });
            });
            instrument_grid_cell(ui, right, tokens, text(lang, "④ SCPI 控制台", "④ SCPI Console"), |ui| {
                self.console_ui_compact(ui, lang, tokens, id, index);
            });
        });
    }

    fn load_workspace(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        tokens: &Tokens,
        id: u64,
        index: usize,
    ) {
        self.instrument_two_by_two(
            ui,
            [
                (
                    text(lang, "① 负载控制", "① Load Control"),
                    "load-ctrl-scroll",
                    InstrumentPanelBody::LoadControl,
                ),
                (
                    text(lang, "② 实测读数", "② Measurements"),
                    "load-meas",
                    InstrumentPanelBody::LoadReadings,
                ),
                (
                    text(lang, "③ 设备信息", "③ Device Info"),
                    "load-info-scroll",
                    InstrumentPanelBody::LoadInfo,
                ),
                (
                    text(lang, "④ SCPI 控制台", "④ SCPI Console"),
                    "load-scpi",
                    InstrumentPanelBody::Scpi,
                ),
            ],
            lang,
            tokens,
            id,
            index,
        );
    }

    fn dmm_workspace(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        tokens: &Tokens,
        id: u64,
        index: usize,
    ) {
        self.instrument_two_by_two(
            ui,
            [
                (
                    text(lang, "① 测量配置", "① Measure Setup"),
                    "dmm-setup-scroll",
                    InstrumentPanelBody::DmmSetup,
                ),
                (
                    text(lang, "② 读数", "② Reading"),
                    "dmm-reading",
                    InstrumentPanelBody::DmmReading,
                ),
                (
                    text(lang, "③ 设备信息", "③ Device Info"),
                    "dmm-info-scroll",
                    InstrumentPanelBody::DmmInfo,
                ),
                (
                    text(lang, "④ SCPI 控制台", "④ SCPI Console"),
                    "dmm-scpi",
                    InstrumentPanelBody::Scpi,
                ),
            ],
            lang,
            tokens,
            id,
            index,
        );
    }

    fn instrument_two_by_two(
        &mut self,
        ui: &mut egui::Ui,
        panels: [(&str, &str, InstrumentPanelBody); 4],
        lang: Lang,
        tokens: &Tokens,
        id: u64,
        index: usize,
    ) {
        let avail = ui.available_size();
        let row1_h = ((avail.y - SCOPE_ROW_GAP) * 0.5).floor().max(1.0);
        let row2_h = (avail.y - SCOPE_ROW_GAP - row1_h).max(1.0);
        let (col_l, col_r) = instrument_row_column_widths(avail.x);

        for (row_idx, (row_h, pair)) in [
            (row1_h, [panels[0], panels[1]]),
            (row2_h, [panels[2], panels[3]]),
        ]
        .into_iter()
        .enumerate()
        {
            instrument_grid_row(ui, avail.x, row_h, col_l, col_r, |ui, left, right| {
                for (cell, (title, salt, body)) in [(left, pair[0]), (right, pair[1])] {
                    instrument_grid_cell(ui, cell, tokens, title, |ui| match body {
                        InstrumentPanelBody::Scpi => {
                            self.console_ui_compact(ui, lang, tokens, id, index);
                        }
                        InstrumentPanelBody::LoadReadings | InstrumentPanelBody::DmmReading => {
                            self.render_instrument_panel(ui, lang, tokens, id, index, body);
                        }
                        _ => {
                            egui::ScrollArea::vertical()
                                .id_salt((salt, id))
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    self.render_instrument_panel(
                                        ui, lang, tokens, id, index, body,
                                    );
                                });
                        }
                    });
                }
            });
            if row_idx == 0 {
                ui.add_space(SCOPE_ROW_GAP);
            }
        }
    }

    fn render_instrument_panel(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        tokens: &Tokens,
        id: u64,
        index: usize,
        body: InstrumentPanelBody,
    ) {
        match body {
            InstrumentPanelBody::LoadControl => {
                let commands = load_channel_controls(ui, lang, tokens, &mut self.devices[index]);
                self.dispatch_scope_commands(id, commands);
            }
            InstrumentPanelBody::LoadReadings => {
                self.load_readings_ui(ui, lang, tokens, id, index);
            }
            InstrumentPanelBody::LoadInfo => {
                self.load_info_ui(ui, lang, tokens, id, index);
            }
            InstrumentPanelBody::DmmSetup => {
                let commands = dmm_setup_controls(ui, lang, tokens, &mut self.devices[index]);
                self.dispatch_scope_commands(id, commands);
            }
            InstrumentPanelBody::DmmReading => {
                self.dmm_reading_ui(ui, lang, tokens, id, index);
            }
            InstrumentPanelBody::DmmInfo => {
                self.dmm_info_ui(ui, lang, tokens, id, index);
            }
            InstrumentPanelBody::Scpi => {
                self.console_ui_compact(ui, lang, tokens, id, index);
            }
        }
    }

    fn load_readings_ui(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        tokens: &Tokens,
        id: u64,
        index: usize,
    ) {
        self.measure_toolbar(ui, lang, tokens, id, index);
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(6.0);
        egui::Grid::new(("load-readings-grid", id))
            .num_columns(2)
            .spacing([16.0, 10.0])
            .striped(true)
            .show(ui, |ui| {
                for key in ["Voltage", "Current", "Power"] {
                    let label = match key {
                        "Voltage" => text(lang, "电压", "Voltage"),
                        "Current" => text(lang, "电流", "Current"),
                        _ => text(lang, "功率", "Power"),
                    };
                    let value = self
                        .latest
                        .get(&(id, key.to_owned()))
                        .map(|r| format!("{:.6} {}", r.value, r.unit))
                        .unwrap_or_else(|| "—".into());
                    ui.label(RichText::new(label).strong());
                    ui.label(RichText::new(value).monospace().size(16.0));
                    ui.end_row();
                }
            });
    }

    fn load_info_ui(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        tokens: &Tokens,
        id: u64,
        index: usize,
    ) {
        let device = &self.devices[index];
        egui::Grid::new(("load-info-grid", id))
            .num_columns(2)
            .spacing([14.0, 8.0])
            .show(ui, |ui| {
                setting_row(
                    ui,
                    text(lang, "制造商", "Manufacturer"),
                    &device.identity.manufacturer,
                );
                setting_row(ui, text(lang, "型号", "Model"), &device.identity.model);
                setting_row(ui, "VISA", &device.resource);
                setting_row(ui, text(lang, "驱动档案", "Profile"), &device.profile);
            });
        ui.add_space(10.0);
        ui.label(RichText::new(text(lang, "支持模式", "Supported modes")).strong());
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            for mode in &device.capabilities.load_modes {
                capability_badge(ui, tokens, mode);
            }
        });
        ui.add_space(10.0);
        ui.label(
            RichText::new(text(
                lang,
                "提示：先设定模式与电平，再开启负载输入。",
                "Tip: set mode and level before enabling load input.",
            ))
            .small()
            .color(tokens.text_muted),
        );
    }

    fn dmm_reading_ui(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        tokens: &Tokens,
        id: u64,
        index: usize,
    ) {
        self.measure_toolbar(ui, lang, tokens, id, index);
        ui.add_space(10.0);
        ui.separator();
        ui.add_space(10.0);

        let function = self.devices[index].controls.dmm_function;
        let channel = function.scpi().to_owned();
        // Worker stores channel as scpi string via read_measurements.
        let reading = self
            .latest
            .get(&(id, channel))
            .or_else(|| {
                // Fallback: any reading for this device.
                self.latest
                    .iter()
                    .find(|((dev, _), _)| *dev == id)
                    .map(|(_, r)| r)
            });

        ui.label(
            RichText::new(function.label())
                .strong()
                .size(14.0)
                .color(tokens.text_muted),
        );
        ui.add_space(8.0);
        if let Some(r) = reading {
            ui.label(
                RichText::new(format!("{:.6}", r.value))
                    .monospace()
                    .size(28.0)
                    .strong(),
            );
            ui.label(RichText::new(&r.unit).size(16.0).color(tokens.text_muted));
        } else {
            placeholder_panel(
                ui,
                ui.available_height().max(80.0),
                text(lang, "点击「单次测量」或开始连续采样", "Measure once or start sampling"),
                tokens.text_muted,
            );
        }
    }

    fn dmm_info_ui(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        tokens: &Tokens,
        id: u64,
        index: usize,
    ) {
        let device = &self.devices[index];
        egui::Grid::new(("dmm-info-grid", id))
            .num_columns(2)
            .spacing([14.0, 8.0])
            .show(ui, |ui| {
                setting_row(
                    ui,
                    text(lang, "制造商", "Manufacturer"),
                    &device.identity.manufacturer,
                );
                setting_row(ui, text(lang, "型号", "Model"), &device.identity.model);
                setting_row(ui, "VISA", &device.resource);
                setting_row(ui, text(lang, "驱动档案", "Profile"), &device.profile);
            });
        ui.add_space(10.0);
        ui.label(RichText::new(text(lang, "测量功能", "Functions")).strong());
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            for function in &device.capabilities.measure_functions {
                capability_badge(ui, tokens, function);
            }
        });
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            if device.capabilities.range_control {
                capability_badge(ui, tokens, text(lang, "量程", "Range"));
            }
            if device.capabilities.nplc_control {
                capability_badge(ui, tokens, "NPLC");
            }
        });
    }

    fn measure_toolbar(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        tokens: &Tokens,
        id: u64,
        index: usize,
    ) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if ui
                .add_sized(
                    SCOPE_BTN_WIDE,
                    egui::Button::new(text(lang, "单次测量", "Measure")).fill(tokens.accent),
                )
                .clicked()
            {
                self.measurement_pending.insert(id);
                let _ = self.tx.send(Job::Measure(id));
            }
            ui.label(text(lang, "周期", "Interval"));
            ui.add(
                egui::DragValue::new(&mut self.sample_interval_ms)
                    .range(100..=60_000)
                    .suffix(" ms"),
            );
            let acquiring = self.devices[index].acquiring;
            if !acquiring {
                if ui
                    .add_sized(
                        SCOPE_BTN_WIDE,
                        egui::Button::new(text(lang, "连续采样", "Continuous")),
                    )
                    .clicked()
                {
                    self.devices[index].acquiring = true;
                    self.devices[index].paused = false;
                    self.last_sample =
                        Instant::now() - Duration::from_millis(self.sample_interval_ms);
                }
            } else {
                let pause_label = if self.devices[index].paused {
                    text(lang, "继续", "Resume")
                } else {
                    text(lang, "暂停", "Pause")
                };
                if ui
                    .add_sized(SCOPE_BTN, egui::Button::new(pause_label))
                    .clicked()
                {
                    self.devices[index].paused = !self.devices[index].paused;
                    if !self.devices[index].paused {
                        self.last_sample =
                            Instant::now() - Duration::from_millis(self.sample_interval_ms);
                    }
                }
                if ui
                    .add_sized(
                        SCOPE_BTN,
                        egui::Button::new(
                            RichText::new(text(lang, "停止", "Stop"))
                                .color(tokens.accent_text)
                                .strong(),
                        )
                        .fill(tokens.stop_bg),
                    )
                    .clicked()
                {
                    self.devices[index].acquiring = false;
                    self.devices[index].paused = false;
                }
            }
        });
    }

    fn source_readings_ui(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        tokens: &Tokens,
        id: u64,
        index: usize,
    ) {
        let channels = self.devices[index].capabilities.channels.max(1).min(4) as usize;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if ui
                .add_sized(
                    SCOPE_BTN_WIDE,
                    egui::Button::new(text(lang, "读取实测", "Read Actual")).fill(tokens.accent),
                )
                .clicked()
            {
                self.measurement_pending.insert(id);
                let _ = self.tx.send(Job::Measure(id));
            }
            let acquiring = self.devices[index].acquiring;
            let label = if acquiring {
                text(lang, "停止采样", "Stop Sample")
            } else {
                text(lang, "连续采样", "Continuous")
            };
            if ui.add_sized(SCOPE_BTN_WIDE, egui::Button::new(label)).clicked() {
                self.devices[index].acquiring = !acquiring;
                if self.devices[index].acquiring {
                    self.devices[index].paused = false;
                    self.measurement_pending.insert(id);
                    let _ = self.tx.send(Job::Measure(id));
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(format!(
                        "{}: {channels}",
                        text(lang, "通道数", "Channels")
                    ))
                    .small()
                    .color(tokens.text_muted),
                );
            });
        });
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(6.0);

        egui::ScrollArea::vertical()
            .id_salt(("source-readings", id))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new(("source-readings-grid", id))
                    .num_columns(4)
                    .spacing([12.0, 8.0])
                    .striped(true)
                    .show(ui, |ui| {
                        ui.label(RichText::new("CH").strong().small());
                        ui.label(RichText::new("V").strong().small());
                        ui.label(RichText::new("A").strong().small());
                        ui.label(RichText::new("W").strong().small());
                        ui.end_row();
                        for ch in 1..=channels {
                            let v = self
                                .latest
                                .get(&(id, format!("CH{ch} Voltage")))
                                .map(|r| format!("{:.4} {}", r.value, r.unit))
                                .unwrap_or_else(|| "—".into());
                            let a = self
                                .latest
                                .get(&(id, format!("CH{ch} Current")))
                                .map(|r| format!("{:.4} {}", r.value, r.unit))
                                .unwrap_or_else(|| "—".into());
                            let p = self
                                .latest
                                .get(&(id, format!("CH{ch} Power")))
                                .map(|r| format!("{:.4} {}", r.value, r.unit))
                                .unwrap_or_else(|| "—".into());
                            ui.label(RichText::new(format!("CH{ch}")).strong().monospace());
                            ui.label(RichText::new(v).monospace().small());
                            ui.label(RichText::new(a).monospace().small());
                            ui.label(RichText::new(p).monospace().small());
                            ui.end_row();
                        }
                    });
            });
    }

    fn empty_instrument_workspace(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        ui.heading(instrument_name(lang, self.selected_kind));
        ui.label(
            RichText::new(text(
                lang,
                "当前类型尚未连接。左侧扫描或输入 VISA 地址后即可启用全部功能。",
                "No instrument of this type is connected. Scan or enter a VISA resource on the left to enable all functions.",
            ))
            .color(tokens.text_muted),
        );
        ui.add_space(12.0);
        let features = instrument_features(lang, self.selected_kind);
        ui.columns(3, |columns| {
            for (column, (title, body)) in columns.iter_mut().zip(features) {
                card(column, tokens, title, |ui| {
                    ui.label(RichText::new(body).color(tokens.text_muted));
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(text(lang, "连接后启用", "Available after connection"))
                            .small()
                            .color(tokens.accent),
                    );
                });
            }
        });
    }

    fn instrument_parameters_ui(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        tokens: &Tokens,
        id: u64,
        index: usize,
    ) {
        let kind = self.devices[index].kind;
        let capabilities = self.devices[index].capabilities.clone();
        match kind {
            InstrumentKind::Oscilloscope => {
                ui.horizontal(|ui| {
                    ui.label(text(lang, "默认波形通道", "Default waveform channel"));
                    ui.add(
                        egui::DragValue::new(&mut self.devices[index].controls.scope_channel)
                            .range(1..=capabilities.channels.max(1)),
                    );
                    ui.label(text(lang, "波形点数上限", "Waveform point limit"));
                    ui.label(format!("{}", self.max_points.min(20_000)));
                });
            }
            InstrumentKind::DcSource => {
                ui.label(
                    RichText::new(format!(
                        "{}: {}  |  {}  |  {}",
                        text(lang, "识别通道数", "Detected channels"),
                        capabilities.channels.max(1),
                        text(lang, "电压上限 0–60 V", "Voltage limit 0–60 V"),
                        if capabilities.source_protection {
                            text(lang, "支持 OVP/OCP", "OVP/OCP supported")
                        } else {
                            text(lang, "保护未报告", "Protection not reported")
                        }
                    ))
                    .small()
                    .color(tokens.text_muted),
                );
                let _ = id;
            }
            InstrumentKind::ElectronicLoad => {
                card(
                    ui,
                    tokens,
                    text(lang, "负载参数", "Load Parameters"),
                    |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(text(lang, "支持模式", "Supported modes"));
                            for mode in &capabilities.load_modes {
                                capability_badge(ui, tokens, mode);
                            }
                        });
                    },
                );
            }
            InstrumentKind::Multimeter => {
                card(
                    ui,
                    tokens,
                    text(lang, "万用表参数", "DMM Parameters"),
                    |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(text(lang, "测量功能", "Functions"));
                            for function in &capabilities.measure_functions {
                                capability_badge(ui, tokens, function);
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label(text(lang, "量程控制", "Range control"));
                            ui.label(if capabilities.range_control {
                                text(lang, "支持", "Supported")
                            } else {
                                text(lang, "未报告", "Not reported")
                            });
                            ui.label(text(lang, "NPLC 控制", "NPLC control"));
                            ui.label(if capabilities.nplc_control {
                                text(lang, "支持", "Supported")
                            } else {
                                text(lang, "未报告", "Not reported")
                            });
                        });
                    },
                );
            }
            InstrumentKind::Generic => {}
        }
    }

    fn acquisition_ui(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        _tokens: &Tokens,
        id: u64,
        index: usize,
    ) {
        ui.horizontal(|ui| {
            ui.label(text(lang, "采样周期", "Sample interval"));
            ui.add(
                egui::DragValue::new(&mut self.sample_interval_ms)
                    .range(100..=60_000)
                    .suffix(" ms"),
            );
            let acquiring = self.devices[index].acquiring;
            if !acquiring {
                if theme::accent_button(ui, _tokens, text(lang, "开始", "Start")).clicked() {
                    self.devices[index].acquiring = true;
                    self.devices[index].paused = false;
                    self.last_sample =
                        Instant::now() - Duration::from_millis(self.sample_interval_ms);
                }
            } else {
                let pause_label = if self.devices[index].paused {
                    text(lang, "继续", "Resume")
                } else {
                    text(lang, "暂停", "Pause")
                };
                if ui.button(pause_label).clicked() {
                    self.devices[index].paused = !self.devices[index].paused;
                    if !self.devices[index].paused {
                        self.last_sample =
                            Instant::now() - Duration::from_millis(self.sample_interval_ms);
                    }
                }
                if theme::stop_button(ui, _tokens, text(lang, "停止", "Stop")).clicked() {
                    self.devices[index].acquiring = false;
                    self.devices[index].paused = false;
                }
            }
            if ui.button(text(lang, "单次读取", "Read Once")).clicked() {
                self.measurement_pending.insert(id);
                let _ = self.tx.send(Job::Measure(id));
            }
            if ui.button(text(lang, "清空", "Clear")).clicked() {
                self.samples.clear();
                self.latest.retain(|(device_id, _), _| *device_id != id);
            }
            if ui
                .add_enabled(
                    !self.samples.is_empty(),
                    egui::Button::new(text(lang, "导出 CSV", "Export CSV")),
                )
                .clicked()
            {
                let default_name =
                    format!("instrument_{}.csv", Local::now().format("%Y%m%d_%H%M%S"));
                if let Some(path) = rfd::FileDialog::new()
                    .set_directory(&self.save_dir)
                    .set_file_name(&default_name)
                    .add_filter("CSV", &["csv"])
                    .save_file()
                {
                    let rows: Vec<_> = self.samples.iter().cloned().collect();
                    match export_csv(path, rows) {
                        Ok(()) => self.status = text(lang, "CSV 已导出", "CSV exported").into(),
                        Err(error) => self.status = error.to_string(),
                    }
                }
            }
        });

        ui.horizontal_wrapped(|ui| {
            for ((device_id, channel), reading) in &self.latest {
                if *device_id == id {
                    Frame::NONE
                        .stroke(Stroke::new(
                            1.0_f32,
                            ui.visuals().widgets.noninteractive.bg_stroke.color,
                        ))
                        .corner_radius(CornerRadius::same(5))
                        .inner_margin(Margin::symmetric(10, 6))
                        .show(ui, |ui| {
                            ui.label(RichText::new(channel).small());
                            ui.label(
                                RichText::new(format!("{:.6} {}", reading.value, reading.unit))
                                    .strong(),
                            );
                        });
                }
            }
        });

        let resource = self.devices[index].resource.clone();
        let mut series: HashMap<String, Vec<[f64; 2]>> = HashMap::new();
        let first = self
            .samples
            .iter()
            .find(|sample| sample.resource == resource && sample.value.is_some())
            .map(|sample| sample.timestamp.timestamp_millis())
            .unwrap_or(0);
        for sample in self
            .samples
            .iter()
            .filter(|sample| sample.resource == resource)
        {
            if let Some(value) = sample.value {
                let x = (sample.timestamp.timestamp_millis() - first) as f64 / 1000.0;
                series
                    .entry(sample.channel.clone())
                    .or_default()
                    .push([x, value]);
            }
        }
        Plot::new(format!("acquisition-{id}"))
            .height(260.0)
            .legend(Legend::default())
            .show(ui, |plot_ui| {
                for (name, points) in series {
                    plot_ui.line(Line::new(PlotPoints::from(points)).name(name));
                }
            });
    }

    fn settings_ui(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        tokens: &Tokens,
        id: u64,
        index: usize,
    ) {
        let device = &self.devices[index];
        let identity = device.identity.clone();
        let resource = device.resource.clone();
        let profile = device.profile.clone();
        let capabilities = device.capabilities.clone();
        let mut command = None;
        let mut disconnect = false;

        card(
            ui,
            tokens,
            text(lang, "设备信息", "Device Information"),
            |ui| {
                egui::Grid::new(format!("device-info-{id}"))
                    .num_columns(2)
                    .spacing([18.0, 6.0])
                    .show(ui, |ui| {
                        setting_row(
                            ui,
                            text(lang, "制造商", "Manufacturer"),
                            &identity.manufacturer,
                        );
                        setting_row(ui, text(lang, "型号", "Model"), &identity.model);
                        setting_row(ui, text(lang, "序列号", "Serial"), &identity.serial);
                        setting_row(ui, text(lang, "固件", "Firmware"), &identity.firmware);
                        setting_row(ui, "VISA", &resource);
                        setting_row(ui, text(lang, "驱动档案", "Driver Profile"), &profile);
                    });
            },
        );
        card(
            ui,
            tokens,
            text(lang, "通信与采集参数", "Communication & Acquisition"),
            |ui| {
                egui::Grid::new(format!("instrument-parameters-{id}"))
                    .num_columns(2)
                    .spacing([18.0, 8.0])
                    .show(ui, |ui| {
                        ui.label(text(lang, "VISA 超时", "VISA timeout"));
                        ui.add(
                            egui::DragValue::new(&mut self.timeout_ms)
                                .range(100..=120_000)
                                .suffix(" ms"),
                        );
                        ui.end_row();
                        ui.label(text(lang, "采样周期", "Sample interval"));
                        ui.add(
                            egui::DragValue::new(&mut self.sample_interval_ms)
                                .range(100..=60_000)
                                .suffix(" ms"),
                        );
                        ui.end_row();
                        ui.label(text(lang, "最大采样点", "Maximum samples"));
                        ui.label(self.max_points.to_string());
                        ui.end_row();
                        ui.label(text(lang, "数据目录", "Data directory"));
                        ui.label(self.save_dir.display().to_string());
                        ui.end_row();
                    });
            },
        );
        card(
            ui,
            tokens,
            text(lang, "设备能力", "Device Capabilities"),
            |ui| {
                ui.horizontal_wrapped(|ui| {
                    capability_badge(ui, tokens, format!("CH {}", capabilities.channels));
                    if capabilities.waveform {
                        capability_badge(ui, tokens, text(lang, "波形", "Waveform"));
                    }
                    if capabilities.screenshot {
                        capability_badge(ui, tokens, text(lang, "截图", "Screenshot"));
                    }
                    if capabilities.source_output {
                        capability_badge(ui, tokens, text(lang, "输出控制", "Output"));
                    }
                    for mode in &capabilities.load_modes {
                        capability_badge(ui, tokens, mode);
                    }
                    for function in &capabilities.measure_functions {
                        capability_badge(ui, tokens, function);
                    }
                });
            },
        );
        ui.horizontal(|ui| {
            if ui
                .button("*CLS")
                .on_hover_text(text(lang, "清除设备状态", "Clear device status"))
                .clicked()
            {
                command = Some(ControlCommand::Clear);
            }
            if ui
                .button("SYST:ERR?")
                .on_hover_text(text(lang, "读取错误队列", "Read error queue"))
                .clicked()
            {
                command = Some(ControlCommand::RawQuery("SYST:ERR?".into()));
            }
            if ui
                .button("*RST")
                .on_hover_text(text(lang, "恢复设备默认设置", "Reset instrument"))
                .clicked()
            {
                command = Some(ControlCommand::Reset);
            }
            if theme::stop_button(ui, tokens, text(lang, "断开设备", "Disconnect")).clicked() {
                disconnect = true;
            }
        });
        if let Some(command) = command {
            self.log_command(id, &command);
            let _ = self.tx.send(Job::Command(id, command));
        }
        if disconnect {
            let _ = self.tx.send(Job::Disconnect(id));
        }
    }

    fn console_ui(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        tokens: &Tokens,
        id: u64,
        index: usize,
    ) {
        ui.label(text(
            lang,
            "仅在确认仪表命令集后使用原始 SCPI。查询命令应以 ? 结尾。",
            "Use raw SCPI only after confirming the instrument command set. Queries should end in ?.",
        ));
        let command = &mut self.devices[index].controls.console;
        let mut to_send = None;
        ui.horizontal(|ui| {
            let response = ui.add(egui::TextEdit::singleline(command).desired_width(420.0));
            let send = ui.button(text(lang, "发送", "Send")).clicked()
                || (response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)));
            if send && !command.trim().is_empty() {
                let value = command.trim().to_owned();
                to_send = Some(if value.ends_with('?') {
                    ControlCommand::RawQuery(value)
                } else {
                    ControlCommand::RawWrite(value)
                });
            }
        });
        if let Some(scpi) = to_send {
            self.log_command(id, &scpi);
            let _ = self.tx.send(Job::Command(id, scpi));
        }
        ui.separator();
        egui::ScrollArea::vertical()
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for line in &self.logs {
                    ui.label(
                        RichText::new(line)
                            .monospace()
                            .small()
                            .color(tokens.text_muted),
                    );
                }
            });
    }

    fn log_command(&mut self, id: u64, command: &ControlCommand) {
        self.log(format!("#{id} ▶ {command:?}"));
    }

    fn log(&mut self, message: String) {
        self.logs
            .push_back(format!("{} {message}", Local::now().format("%H:%M:%S%.3f")));
        while self.logs.len() > 1_000 {
            self.logs.pop_front();
        }
    }

    pub fn on_exit(&mut self) {
        for device in &mut self.devices {
            device.acquiring = false;
        }
        let _ = self.tx.send(Job::Shutdown);
    }
}

fn worker_loop(jobs: Receiver<Job>, events: Sender<Event>) {
    let mut devices = HashMap::<u64, InstrumentDevice>::new();
    while let Ok(job) = jobs.recv() {
        match job {
            Job::Scan {
                library,
                timeout_ms,
            } => match discover_resources_with_library(
                (!library.trim().is_empty()).then_some(library.as_str()),
                timeout_ms,
            ) {
                Ok(resources) => {
                    let _ = events.send(Event::Resources(resources));
                }
                Err(error) => {
                    let _ = events.send(Event::Error {
                        id: None,
                        message: error.to_string(),
                    });
                }
            },
            Job::Connect {
                id,
                resource,
                kind,
                timeout_ms,
                library,
            } => match InstrumentDevice::connect_with_library(
                resource.clone(),
                timeout_ms,
                kind,
                (!library.trim().is_empty()).then_some(library.as_str()),
            ) {
                Ok(device) => {
                    let event = Event::Connected {
                        id,
                        resource,
                        identity: device.identity.clone(),
                        kind: device.profile.kind,
                        profile: device.profile.name.clone(),
                        capabilities: device.profile.capabilities.clone(),
                    };
                    devices.insert(id, device);
                    let _ = events.send(event);
                }
                Err(error) => send_error(&events, Some(id), error),
            },
            Job::ConnectDemo { id, kind } => match InstrumentDevice::connect_demo(kind) {
                Ok(device) => {
                    let event = Event::Connected {
                        id,
                        resource: device.resource.clone(),
                        identity: device.identity.clone(),
                        kind: device.profile.kind,
                        profile: device.profile.name.clone(),
                        capabilities: device.profile.capabilities.clone(),
                    };
                    devices.insert(id, device);
                    let _ = events.send(event);
                }
                Err(error) => send_error(&events, Some(id), error),
            },
            Job::Disconnect(id) => {
                devices.remove(&id);
                let _ = events.send(Event::Disconnected(id));
            }
            Job::Command(id, command) => {
                if let Some(device) = devices.get_mut(&id) {
                    match device.execute(command) {
                        Ok(response) => {
                            let _ = events.send(Event::CommandDone { id, response });
                        }
                        Err(error) => send_error(&events, Some(id), error),
                    }
                }
            }
            Job::Measure(id) => {
                if let Some(device) = devices.get_mut(&id) {
                    match device.read_measurements() {
                        Ok(readings) => {
                            let _ = events.send(Event::Measurements {
                                id,
                                resource: device.resource.clone(),
                                readings,
                            });
                        }
                        Err(error) => send_error(&events, Some(id), error),
                    }
                }
            }
            Job::Capture { id } => {
                if let Some(device) = devices.get_mut(&id) {
                    match device.capture_scope_png() {
                        Ok(png) => match prepare_screenshot_preview(&png, SCOPE_PREVIEW_MAX_EDGE) {
                            Ok((width, height, rgba)) => {
                                let _ = events.send(Event::Screenshot {
                                    id,
                                    width,
                                    height,
                                    rgba,
                                    png,
                                });
                            }
                            Err(error) => send_error(
                                &events,
                                Some(id),
                                format!("Invalid screenshot: {error}"),
                            ),
                        },
                        Err(error) => send_error(&events, Some(id), error),
                    }
                }
            }
            Job::WaveformSource { id, channel } => {
                if let Some(device) = devices.get_mut(&id) {
                    match device.capture_scope_waveform_source(channel) {
                        Ok((bytes, suggested_name)) => {
                            let _ = events.send(Event::WaveformSource {
                                id,
                                bytes,
                                suggested_name,
                            });
                        }
                        Err(error) => send_error(&events, Some(id), error),
                    }
                }
            }
            Job::Waveform {
                id,
                channel,
                points: _,
            } => {
                if let Some(device) = devices.get_mut(&id) {
                    match device.read_scope_screen_waveform(channel) {
                        Ok(trace) => {
                            let _ = events.send(Event::Waveform { id, trace });
                        }
                        Err(error) => send_error(&events, Some(id), error),
                    }
                }
            }
            Job::Shutdown => break,
        }
    }
}

fn send_error(events: &Sender<Event>, id: Option<u64>, error: impl std::fmt::Display) {
    let _ = events.send(Event::Error {
        id,
        message: error.to_string(),
    });
}

struct WaveformStats {
    count: usize,
    dt: f64,
    min: f64,
    max: f64,
    pp: f64,
    mean: f64,
}

fn waveform_stats(trace: &WaveformTrace) -> WaveformStats {
    let n = trace.x.len().min(trace.y.len());
    if n == 0 {
        return WaveformStats {
            count: 0,
            dt: 0.0,
            min: 0.0,
            max: 0.0,
            pp: 0.0,
            mean: 0.0,
        };
    }
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    let mut sum = 0.0;
    for &y in &trace.y[..n] {
        min = min.min(y);
        max = max.max(y);
        sum += y;
    }
    let dt = if n >= 2 {
        (trace.x[n - 1] - trace.x[0]) / (n as f64 - 1.0)
    } else {
        0.0
    };
    WaveformStats {
        count: n,
        dt,
        min,
        max,
        pp: max - min,
        mean: sum / n as f64,
    }
}

fn export_waveform_csv(
    path: impl AsRef<std::path::Path>,
    trace: &WaveformTrace,
) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = std::fs::File::create(path)?;
    writeln!(
        file,
        "channel,index,x({}),y({})",
        trace.x_unit, trace.y_unit
    )?;
    let n = trace.x.len().min(trace.y.len());
    for i in 0..n {
        writeln!(
            file,
            "{},{},{},{}",
            waveform_csv_cell(&trace.channel),
            i,
            trace.x[i],
            trace.y[i]
        )?;
    }
    Ok(())
}

fn waveform_csv_cell(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

fn build_cached_wave_plot(trace: &WaveformTrace, max_points: usize) -> CachedWavePlot {
    let n = trace.x.len().min(trace.y.len());
    let max_points = max_points.max(2);
    let points = if n == 0 {
        Vec::new()
    } else if n <= max_points {
        trace.x[..n]
            .iter()
            .zip(&trace.y[..n])
            .map(|(&x, &y)| [x, y])
            .collect()
    } else {
        // Min/max buckets preserve peaks without shipping 10k points to egui_plot each frame.
        let buckets = max_points / 2;
        let mut points = Vec::with_capacity(buckets * 2);
        for b in 0..buckets {
            let start = b * n / buckets;
            let end = ((b + 1) * n / buckets).max(start + 1).min(n);
            let mut min_i = start;
            let mut max_i = start;
            for i in start..end {
                if trace.y[i] < trace.y[min_i] {
                    min_i = i;
                }
                if trace.y[i] > trace.y[max_i] {
                    max_i = i;
                }
            }
            if min_i <= max_i {
                points.push([trace.x[min_i], trace.y[min_i]]);
                if max_i != min_i {
                    points.push([trace.x[max_i], trace.y[max_i]]);
                }
            } else {
                points.push([trace.x[max_i], trace.y[max_i]]);
                points.push([trace.x[min_i], trace.y[min_i]]);
            }
        }
        points
    };
    CachedWavePlot {
        channel: trace.channel.clone(),
        points: Arc::new(points),
    }
}

fn scope_panel(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    title: &str,
    add: impl FnOnce(&mut egui::Ui),
) {
    let width = ui.available_width();
    Frame::NONE
        .fill(tokens.surface_bg)
        .stroke(Stroke::new(1.0_f32, tokens.border))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_min_width(width);
            ui.set_max_width(width);
            ui.label(RichText::new(title).strong().size(13.0));
            ui.add_space(6.0);
            add(ui);
        });
}

/// Card that fills the allocated cell exactly (no overflow); body uses leftover height.
/// Equal split for a 2×2 instrument grid — same left/right widths on every row.
fn instrument_row_column_widths(total_w: f32) -> (f32, f32) {
    let left = ((total_w - SCOPE_COL_GAP) * 0.5).floor().max(1.0);
    let right = (total_w - SCOPE_COL_GAP - left).max(1.0);
    (left, right)
}

/// Allocate one row and return absolute left/right cell rects (content cannot shift columns).
fn instrument_grid_row(
    ui: &mut egui::Ui,
    total_w: f32,
    row_h: f32,
    left_w: f32,
    right_w: f32,
    add: impl FnOnce(&mut egui::Ui, egui::Rect, egui::Rect),
) {
    let total_w = total_w.max(1.0);
    let row_h = row_h.max(1.0);
    let (row_rect, _) =
        ui.allocate_exact_size(egui::vec2(total_w, row_h), egui::Sense::hover());
    let left_rect = egui::Rect::from_min_size(row_rect.min, egui::vec2(left_w, row_h));
    let right_rect = egui::Rect::from_min_size(
        egui::pos2(row_rect.min.x + left_w + SCOPE_COL_GAP, row_rect.min.y),
        egui::vec2(right_w, row_h),
    );
    add(ui, left_rect, right_rect);
}

fn instrument_grid_cell(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    tokens: &Tokens,
    title: &str,
    add: impl FnOnce(&mut egui::Ui),
) {
    if !rect.is_positive() {
        return;
    }
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
        |ui| {
            ui.set_clip_rect(rect.intersect(ui.clip_rect()));
            ui.set_min_size(rect.size());
            ui.set_max_size(rect.size());
            scope_card_fill(ui, tokens, title, add);
        },
    );
}

fn scope_card_fill(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    title: &str,
    add: impl FnOnce(&mut egui::Ui),
) {
    // Paint fill/stroke on the *allocated* cell rect. Do not use Frame::show here:
    // Frame sizes its stroke from content min_rect; when SCPI/ScrollArea expands by
    // even a few px, the right/bottom stroke lands outside the cell clip and vanishes.
    let outer = ui.available_size();
    let (rect, _) = ui.allocate_exact_size(outer, egui::Sense::hover());
    if !rect.is_positive() {
        return;
    }

    const STROKE_W: f32 = 1.0;
    const PAD_X: f32 = 10.0;
    const PAD_Y: f32 = 8.0;
    let radius = CornerRadius::same(6);
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, radius, tokens.surface_bg);
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(STROKE_W, tokens.border),
        egui::StrokeKind::Inside,
    );

    let content = rect.shrink2(egui::vec2(STROKE_W + PAD_X, STROKE_W + PAD_Y));
    if !content.is_positive() {
        return;
    }
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(content)
            .layout(egui::Layout::top_down(egui::Align::Min)),
        |ui| {
            ui.set_clip_rect(content.intersect(ui.clip_rect()));
            ui.set_min_size(content.size());
            ui.set_max_size(content.size());
            ui.label(RichText::new(title).strong().size(13.0));
            ui.add_space(4.0);
            let body = ui.available_size();
            ui.allocate_ui_with_layout(
                body,
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_min_size(body);
                    ui.set_max_size(body);
                    ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
                    add(ui);
                },
            );
        },
    );
}

fn media_slot_empty(ui: &mut egui::Ui, height: f32) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), height), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::same(4), Color32::from_rgb(0x0B, 0x12, 0x20));
    painter.rect_stroke(
        rect,
        CornerRadius::same(4),
        Stroke::new(1.0_f32, Color32::from_rgb(0x2A, 0x36, 0x4A)),
        egui::StrokeKind::Inside,
    );
}

fn paint_waveform(ui: &mut egui::Ui, points: &[[f64; 2]], height: f32, color: Color32) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::same(4), Color32::from_rgb(0x0B, 0x12, 0x20));
    painter.rect_stroke(
        rect,
        CornerRadius::same(4),
        Stroke::new(1.0_f32, Color32::from_rgb(0x2A, 0x36, 0x4A)),
        egui::StrokeKind::Inside,
    );
    if points.len() < 2 {
        return;
    }
    let mut min_x = points[0][0];
    let mut max_x = points[0][0];
    let mut min_y = points[0][1];
    let mut max_y = points[0][1];
    for p in points.iter().skip(1) {
        min_x = min_x.min(p[0]);
        max_x = max_x.max(p[0]);
        min_y = min_y.min(p[1]);
        max_y = max_y.max(p[1]);
    }
    let dx = (max_x - min_x).max(1e-12);
    let dy = (max_y - min_y).max(1e-12);
    let pad = 6.0;
    let inner = rect.shrink(pad);
    let stroke = Stroke::new(1.4_f32, color);
    let mut prev = None;
    for p in points {
        let x = inner.left() + ((p[0] - min_x) / dx) as f32 * inner.width();
        let y = inner.bottom() - ((p[1] - min_y) / dy) as f32 * inner.height();
        let pt = egui::pos2(x, y);
        if let Some(prev) = prev {
            painter.line_segment([prev, pt], stroke);
        }
        prev = Some(pt);
    }
}

fn paint_screenshot(ui: &mut egui::Ui, texture: &egui::TextureHandle, height: f32) {
    let width = ui.available_width().max(1.0);
    let height = height.max(1.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::same(4), Color32::from_rgb(0x0B, 0x12, 0x20));
    painter.rect_stroke(
        rect,
        CornerRadius::same(4),
        Stroke::new(1.0_f32, Color32::from_rgb(0x2A, 0x36, 0x4A)),
        egui::StrokeKind::Inside,
    );
    let source = texture.size_vec2();
    if source.x <= 0.0 || source.y <= 0.0 {
        return;
    }
    // Fit inside the slot (contain), keep aspect ratio, maximize size.
    let pad = 2.0;
    let inner = rect.shrink(pad);
    let scale = (inner.width() / source.x)
        .min(inner.height() / source.y)
        .max(0.0);
    let size = source * scale;
    let image_rect = egui::Rect::from_center_size(inner.center(), size);
    egui::Image::new(texture)
        .fit_to_exact_size(size)
        .paint_at(ui, image_rect);
}

fn placeholder_panel(ui: &mut egui::Ui, height: f32, message: &str, color: Color32) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), height), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::same(4), Color32::from_rgb(0x0B, 0x12, 0x20));
    painter.rect_stroke(
        rect,
        CornerRadius::same(4),
        Stroke::new(1.0_f32, Color32::from_rgb(0x2A, 0x36, 0x4A)),
        egui::StrokeKind::Inside,
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        message,
        egui::FontId::proportional(13.0),
        color,
    );
}

/// Reject absurd PNG dimensions before full decode (runaway VISA binary).
const MAX_SCREENSHOT_BYTES: usize = 32 * 1024 * 1024;
const MAX_SCREENSHOT_PIXELS: u64 = 64_000_000;

fn png_ihdr_dimensions(png: &[u8]) -> Option<(u32, u32)> {
    if png.len() < 24 || png.get(0..8) != Some(b"\x89PNG\r\n\x1a\n") {
        return None;
    }
    if png.get(12..16) != Some(b"IHDR") {
        return None;
    }
    let width = u32::from_be_bytes(png.get(16..20)?.try_into().ok()?);
    let height = u32::from_be_bytes(png.get(20..24)?.try_into().ok()?);
    Some((width, height))
}

fn prepare_screenshot_preview(
    png: &[u8],
    max_edge: u32,
) -> Result<(usize, usize, Vec<u8>), String> {
    if png.len() > MAX_SCREENSHOT_BYTES {
        return Err(format!(
            "PNG exceeds {} MiB limit",
            MAX_SCREENSHOT_BYTES / (1024 * 1024)
        ));
    }
    match png_ihdr_dimensions(png) {
        Some((w, h)) => {
            let pixels = u64::from(w).saturating_mul(u64::from(h));
            if pixels == 0 || pixels > MAX_SCREENSHOT_PIXELS {
                return Err(format!("PNG dimensions {w}x{h} exceed safety limit"));
            }
        }
        None => {
            // Non-PNG hardcopies still go through the decoder with a size ceiling.
            if png.len() > 8 * 1024 * 1024 {
                return Err("screenshot payload is not a PNG and exceeds 8 MiB".into());
            }
        }
    }
    let image = image::load_from_memory(png)
        .map_err(|e| e.to_string())?
        .into_rgba8();
    let (width, height) = image.dimensions();
    let image = if width.max(height) > max_edge {
        let (tw, th) = if width >= height {
            (
                max_edge,
                (max_edge as u64 * height as u64 / width as u64).max(1) as u32,
            )
        } else {
            (
                (max_edge as u64 * width as u64 / height as u64).max(1) as u32,
                max_edge,
            )
        };
        image::imageops::thumbnail(&image, tw, th)
    } else {
        image
    };
    let (width, height) = image.dimensions();
    Ok((width as usize, height as usize, image.into_raw()))
}

fn control_ui(
    ui: &mut egui::Ui,
    lang: Lang,
    tokens: &Tokens,
    device: &mut DeviceUi,
) -> (Vec<ControlCommand>, bool) {
    let mut commands = Vec::new();
    let mut measure_once = false;
    match device.kind {
        InstrumentKind::Oscilloscope => {
            commands.extend(scope_unified_controls(ui, lang, tokens, device));
        }
        InstrumentKind::DcSource => {
            // Dedicated source_workspace handles DC source controls.
            commands.extend(source_channel_controls(ui, lang, tokens, device));
            if ui.button(text(lang, "读取实测", "Read Actual")).clicked() {
                measure_once = true;
            }
        }
        InstrumentKind::ElectronicLoad => {
            commands.extend(load_channel_controls(ui, lang, tokens, device));
            if ui.button(text(lang, "单次测量", "Measure")).clicked() {
                measure_once = true;
            }
        }
        InstrumentKind::Multimeter => {
            commands.extend(dmm_setup_controls(ui, lang, tokens, device));
            if ui.button(text(lang, "单次测量", "Measure")).clicked() {
                measure_once = true;
            }
        }
        InstrumentKind::Generic => {
            ui.label(text(
                lang,
                "未识别设备仅开放 SCPI 控制台，避免发送不兼容命令。",
                "Unknown instruments are limited to the SCPI console to avoid incompatible commands.",
            ));
        }
    }
    (commands, measure_once)
}

fn scope_unified_controls(
    ui: &mut egui::Ui,
    lang: Lang,
    tokens: &Tokens,
    device: &mut DeviceUi,
) -> Vec<ControlCommand> {
    let mut out = Vec::new();
    let max_ch = device.capabilities.channels.max(1).min(4) as usize;
    let trigger_source = device.controls.trigger_source.clone();
    let trigger_slope = device.controls.trigger_slope.clone();
    let panel_w = ui.available_width();

    // Row A — acquisition: four equal buttons span the full card width.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let gap = ui.spacing().item_spacing.x;
        let btn_w = ((panel_w - gap * 3.0) / 4.0).max(56.0);
        let btn = egui::vec2(btn_w, SCOPE_BTN.y);
        if ui
            .add_sized(
                btn,
                egui::Button::new(
                    RichText::new(text(lang, "运行", "Run"))
                        .color(tokens.accent_text)
                        .strong(),
                )
                .fill(tokens.accent),
            )
            .clicked()
        {
            out.push(ControlCommand::ScopeRun);
        }
        if ui
            .add_sized(
                btn,
                egui::Button::new(
                    RichText::new(text(lang, "停止", "Stop"))
                        .color(tokens.accent_text)
                        .strong(),
                )
                .fill(tokens.stop_bg),
            )
            .clicked()
        {
            out.push(ControlCommand::ScopeStop);
        }
        if ui
            .add_sized(btn, egui::Button::new(text(lang, "单次", "Single")))
            .clicked()
        {
            out.push(ControlCommand::ScopeSingle);
        }
        if ui
            .add_sized(btn, egui::Button::new(text(lang, "自动设置", "Autoset")))
            .clicked()
        {
            out.push(ControlCommand::ScopeAutoset);
        }
    });
    ui.add_space(10.0);

    // Row B — timebase / trigger as a full-width toolbar (no floating gap).
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        ui.label(RichText::new("s/div").strong().small());
        ui.allocate_ui_with_layout(
            egui::vec2(80.0, SCOPE_BTN.y),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.add(
                    egui::DragValue::new(&mut device.controls.scope_timebase)
                        .speed(0.000_01)
                        .range(1e-9..=10.0)
                        .min_decimals(0)
                        .max_decimals(9),
                );
            },
        );
        ui.separator();
        ui.label(RichText::new(text(lang, "触发", "Trig")).strong().small());
        egui::ComboBox::from_id_salt("scope-trigger-source")
            .selected_text(trigger_source)
            .width(68.0)
            .show_ui(ui, |ui| {
                for n in 1..=max_ch as u8 {
                    let name = format!("CH{n}");
                    ui.selectable_value(&mut device.controls.trigger_source, name.clone(), name);
                }
            });
        ui.allocate_ui_with_layout(
            egui::vec2(72.0, SCOPE_BTN.y),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.add(
                    egui::DragValue::new(&mut device.controls.trigger_level)
                        .suffix(" V")
                        .speed(0.01),
                );
            },
        );
        egui::ComboBox::from_id_salt("scope-slope")
            .selected_text(trigger_slope)
            .width(68.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut device.controls.trigger_slope, "RISE".into(), "RISE");
                ui.selectable_value(&mut device.controls.trigger_slope, "FALL".into(), "FALL");
            });
        if ui
            .add_sized(SCOPE_BTN, egui::Button::new(text(lang, "应用", "Apply")))
            .clicked()
        {
            out.push(ControlCommand::ScopeTimebase(
                device.controls.scope_timebase,
            ));
            out.push(ControlCommand::ScopeTrigger {
                source: device.controls.trigger_source.clone(),
                level: device.controls.trigger_level,
                slope: device.controls.trigger_slope.clone(),
            });
        }
    });
    ui.add_space(10.0);
    ui.separator();
    ui.add_space(8.0);

    // Channel table — proportional columns fill the panel; result column is widest.
    let gap = 8.0;
    let w_ch = 40.0;
    let w_on = 28.0;
    let w_scale = 96.0;
    let w_pos = 96.0;
    let w_meas = 168.0;
    let fixed = w_ch + w_on + w_scale + w_pos + w_meas + gap * 5.0;
    let w_result = (panel_w - fixed).max(160.0);
    let header_h = 20.0;
    let rows_h = (ui.available_height() - header_h - 4.0).max(0.0);
    let row_h = (rows_h / max_ch.max(1) as f32).clamp(30.0, 44.0);

    scope_channel_col_header(
        ui,
        lang,
        tokens,
        &[
            (w_ch, "CH"),
            (w_on, text(lang, "开", "On")),
            (w_scale, "Scale"),
            (w_pos, "Pos"),
            (w_meas, text(lang, "测量", "Meas")),
            (w_result, text(lang, "读数", "Result")),
        ],
        gap,
        header_h,
    );
    ui.add_space(4.0);

    for i in 0..max_ch {
        let ch = (i as u8) + 1;
        ui.allocate_ui_with_layout(
            egui::vec2(panel_w, row_h),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = gap;

                scope_col(ui, w_ch, row_h, |ui| {
                    ui.label(
                        RichText::new(format!("CH{ch}"))
                            .strong()
                            .monospace()
                            .size(12.0),
                    );
                });
                scope_col(ui, w_on, row_h, |ui| {
                    if ui
                        .checkbox(&mut device.controls.scope_channel_on[i], "")
                        .changed()
                    {
                        out.push(ControlCommand::ScopeChannel {
                            channel: ch,
                            enabled: device.controls.scope_channel_on[i],
                        });
                    }
                });
                scope_col(ui, w_scale, row_h, |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if ui
                        .add_sized(SCOPE_STEP_BTN, egui::Button::new("−"))
                        .clicked()
                    {
                        device.controls.scope_scales[i] =
                            nearest_scale_step(device.controls.scope_scales[i], -1);
                        out.push(ControlCommand::ScopeScale {
                            channel: ch,
                            volts_per_div: device.controls.scope_scales[i],
                        });
                        device.controls.scope_channel = ch;
                    }
                    ui.add_sized(
                        egui::vec2(36.0, SCOPE_STEP_BTN.y),
                        egui::Label::new(
                            RichText::new(format_scope_scale(device.controls.scope_scales[i]))
                                .small()
                                .monospace(),
                        )
                        .selectable(false),
                    );
                    if ui
                        .add_sized(SCOPE_STEP_BTN, egui::Button::new("+"))
                        .clicked()
                    {
                        device.controls.scope_scales[i] =
                            nearest_scale_step(device.controls.scope_scales[i], 1);
                        out.push(ControlCommand::ScopeScale {
                            channel: ch,
                            volts_per_div: device.controls.scope_scales[i],
                        });
                        device.controls.scope_channel = ch;
                    }
                });
                scope_col(ui, w_pos, row_h, |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if ui
                        .add_sized(SCOPE_STEP_BTN, egui::Button::new("−"))
                        .clicked()
                    {
                        device.controls.scope_positions[i] =
                            (device.controls.scope_positions[i] - SCOPE_POS_STEP).clamp(-8.0, 8.0);
                        out.push(ControlCommand::ScopePosition {
                            channel: ch,
                            divisions: device.controls.scope_positions[i],
                        });
                        device.controls.scope_channel = ch;
                    }
                    ui.add_sized(
                        egui::vec2(36.0, SCOPE_STEP_BTN.y),
                        egui::Label::new(
                            RichText::new(format!("{:.2}", device.controls.scope_positions[i]))
                                .small()
                                .monospace(),
                        )
                        .selectable(false),
                    );
                    if ui
                        .add_sized(SCOPE_STEP_BTN, egui::Button::new("+"))
                        .clicked()
                    {
                        device.controls.scope_positions[i] =
                            (device.controls.scope_positions[i] + SCOPE_POS_STEP).clamp(-8.0, 8.0);
                        out.push(ControlCommand::ScopePosition {
                            channel: ch,
                            divisions: device.controls.scope_positions[i],
                        });
                        device.controls.scope_channel = ch;
                    }
                });
                scope_col(ui, w_meas, row_h, |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let meas_label = device.controls.scope_meas_types[i].label();
                    egui::ComboBox::from_id_salt(("scope-meas-type", ch))
                        .selected_text(meas_label)
                        .width(88.0)
                        .show_ui(ui, |ui| {
                            for option in ScopeMeasType::all() {
                                ui.selectable_value(
                                    &mut device.controls.scope_meas_types[i],
                                    *option,
                                    option.label(),
                                );
                            }
                        });
                    if ui
                        .add_sized(
                            egui::vec2(60.0, SCOPE_BTN.y),
                            egui::Button::new(text(lang, "测量", "Meas")),
                        )
                        .clicked()
                    {
                        out.push(ControlCommand::ScopeMeasure {
                            channel: ch,
                            meas_type: device.controls.scope_meas_types[i],
                        });
                        device.controls.scope_channel = ch;
                    }
                });
                scope_col(ui, w_result, row_h, |ui| {
                    let full = device.controls.scope_meas_results[i]
                        .as_deref()
                        .unwrap_or("—");
                    // Row already shows CH + meas type; show human units (MHz/µs…).
                    let display = format_scope_meas_display(
                        full,
                        device.controls.scope_meas_types[i],
                    );
                    let clipped = ellipsize_to_width(ui, &display, w_result - 4.0);
                    ui.add(
                        egui::Label::new(
                            RichText::new(clipped)
                                .small()
                                .monospace()
                                .color(tokens.text_muted),
                        )
                        .truncate()
                        .selectable(false),
                    )
                    .on_hover_text(full);
                });
            },
        );
    }
    out
}

fn scope_channel_col_header(
    ui: &mut egui::Ui,
    _lang: Lang,
    tokens: &Tokens,
    cols: &[(f32, &str)],
    gap: f32,
    height: f32,
) {
    let width: f32 =
        cols.iter().map(|(w, _)| *w).sum::<f32>() + gap * (cols.len().saturating_sub(1) as f32);
    ui.allocate_ui_with_layout(
        egui::vec2(width.max(ui.available_width()), height),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (w, label) in cols {
                scope_col(ui, *w, height, |ui| {
                    ui.label(
                        RichText::new(*label)
                            .strong()
                            .small()
                            .color(tokens.text_muted),
                    );
                });
            }
        },
    );
}

fn scope_col(ui: &mut egui::Ui, width: f32, height: f32, add: impl FnOnce(&mut egui::Ui)) {
    ui.allocate_ui_with_layout(
        egui::vec2(width, height),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_width(width);
            ui.set_max_width(width);
            add(ui);
        },
    );
}

/// Prefer human-readable "value unit" from "CHx Type: …" for the readout column.
fn format_scope_meas_display(full: &str, meas_type: ScopeMeasType) -> String {
    let payload = if let Some((_, rest)) = full.split_once(": ") {
        rest.trim()
    } else {
        full.trim()
    };
    if payload.is_empty() || payload == "—" {
        return "—".into();
    }
    humanize_scope_reading_text(payload, meas_type)
}

fn ellipsize_to_width(ui: &egui::Ui, text: &str, max_width: f32) -> String {
    if max_width <= 8.0 || text.is_empty() {
        return text.to_owned();
    }
    let font = egui::TextStyle::Small.resolve(ui.style());
    let galley = ui.fonts(|f| f.layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE));
    if galley.size().x <= max_width {
        return text.to_owned();
    }
    let ellipsis = "…";
    let mut lo = 0usize;
    let mut hi = text.chars().count();
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        let candidate: String = text.chars().take(mid).collect::<String>() + ellipsis;
        let g = ui.fonts(|f| f.layout_no_wrap(candidate, font.clone(), Color32::WHITE));
        if g.size().x <= max_width {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    if lo == 0 {
        ellipsis.to_owned()
    } else {
        text.chars().take(lo).collect::<String>() + ellipsis
    }
}

fn nearest_scale_step(value: f64, direction: i32) -> f64 {
    let steps = SCOPE_SCALE_STEPS;
    if value <= 0.0 || steps.is_empty() {
        return steps.first().copied().unwrap_or(1e-3);
    }
    let mut best_i = 0usize;
    let mut best_d = f64::MAX;
    for (i, step) in steps.iter().enumerate() {
        let d = (*step - value).abs();
        if d < best_d {
            best_d = d;
            best_i = i;
        }
    }
    if direction > 0 {
        steps[(best_i + 1).min(steps.len() - 1)]
    } else if direction < 0 {
        steps[best_i.saturating_sub(1)]
    } else {
        steps[best_i]
    }
}

fn format_scope_scale(volts_per_div: f64) -> String {
    if volts_per_div >= 1.0 {
        format!("{volts_per_div:.3}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    } else if volts_per_div >= 1e-3 {
        format!("{:.0}m", volts_per_div * 1e3)
    } else {
        format!("{:.0}u", volts_per_div * 1e6)
    }
}

fn store_scope_measure_result(controls: &mut ControlState, response: &str) {
    let trimmed = response.trim();
    if let Some(rest) = trimmed.strip_prefix("CH") {
        if let Some(ch_char) = rest.chars().next() {
            if let Some(ch) = ch_char.to_digit(10) {
                let idx = (ch as usize).saturating_sub(1);
                if idx < controls.scope_meas_results.len() {
                    controls.scope_meas_results[idx] = Some(trimmed.to_owned());
                }
            }
        }
    }
}

fn source_channel_controls(
    ui: &mut egui::Ui,
    lang: Lang,
    tokens: &Tokens,
    device: &mut DeviceUi,
) -> Vec<ControlCommand> {
    let mut out = Vec::new();
    let channels = device.capabilities.channels.max(1).min(4) as usize;
    ui.label(
        RichText::new(format!(
            "{} — {} {}",
            text(lang, "按通道设置电压/限流并开关输出", "Set V/I and output per channel"),
            channels,
            text(lang, "个通道", "channels"),
        ))
        .small()
        .color(tokens.text_muted),
    );
    ui.add_space(8.0);

    let panel_w = ui.available_width();
    let gap = 8.0;
    let w_ch = 40.0;
    let w_v = 96.0;
    let w_i = 96.0;
    let w_apply = 72.0;
    let w_out = (panel_w - w_ch - w_v - w_i - w_apply - gap * 4.0).max(88.0);
    let row_h = 34.0;

    ui.allocate_ui_with_layout(
        egui::vec2(panel_w, 20.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (w, label) in [
                (w_ch, "CH"),
                (w_v, "V"),
                (w_i, text(lang, "限流 A", "Limit A")),
                (w_apply, text(lang, "设定", "Apply")),
                (w_out, text(lang, "输出", "Output")),
            ] {
                scope_col(ui, w, 20.0, |ui| {
                    ui.label(
                        RichText::new(label)
                            .strong()
                            .small()
                            .color(tokens.text_muted),
                    );
                });
            }
        },
    );
    ui.add_space(4.0);

    for i in 0..channels {
        let ch = (i as u8) + 1;
        ui.allocate_ui_with_layout(
            egui::vec2(panel_w, row_h),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = gap;
                scope_col(ui, w_ch, row_h, |ui| {
                    ui.label(
                        RichText::new(format!("CH{ch}"))
                            .strong()
                            .monospace()
                            .size(12.0),
                    );
                });
                scope_col(ui, w_v, row_h, |ui| {
                    ui.add(
                        egui::DragValue::new(&mut device.controls.source_voltages[i])
                            .range(0.0..=60.0)
                            .speed(0.01)
                            .suffix(" V"),
                    );
                });
                scope_col(ui, w_i, row_h, |ui| {
                    ui.add(
                        egui::DragValue::new(&mut device.controls.source_currents[i])
                            .range(0.0..=20.0)
                            .speed(0.01)
                            .suffix(" A"),
                    );
                });
                scope_col(ui, w_apply, row_h, |ui| {
                    if ui
                        .add_sized(SCOPE_BTN, egui::Button::new(text(lang, "应用", "Apply")))
                        .clicked()
                    {
                        out.push(ControlCommand::SourceVoltage {
                            channel: ch,
                            value: device.controls.source_voltages[i],
                        });
                        out.push(ControlCommand::SourceCurrent {
                            channel: ch,
                            value: device.controls.source_currents[i],
                        });
                    }
                });
                scope_col(ui, w_out, row_h, |ui| {
                    let on = device.controls.source_outputs[i];
                    let label = if on {
                        text(lang, "关闭", "OFF")
                    } else {
                        text(lang, "开启", "ON")
                    };
                    let btn = if on {
                        egui::Button::new(
                            RichText::new(label)
                                .color(tokens.accent_text)
                                .strong(),
                        )
                        .fill(tokens.stop_bg)
                    } else {
                        egui::Button::new(
                            RichText::new(label)
                                .color(tokens.accent_text)
                                .strong(),
                        )
                        .fill(tokens.accent)
                    };
                    if ui.add_sized(egui::vec2(w_out.min(100.0), SCOPE_BTN.y), btn).clicked()
                    {
                        device.controls.source_outputs[i] = !on;
                        out.push(ControlCommand::SourceOutput {
                            channel: ch,
                            enabled: device.controls.source_outputs[i],
                        });
                    }
                });
            },
        );
    }
    out
}

fn source_protection_controls(
    ui: &mut egui::Ui,
    lang: Lang,
    tokens: &Tokens,
    device: &mut DeviceUi,
) -> Vec<ControlCommand> {
    let mut out = Vec::new();
    let channels = device.capabilities.channels.max(1).min(4) as usize;
    ui.label(
        RichText::new(text(
            lang,
            "每通道 OVP / OCP（过压 / 过流保护）",
            "Per-channel OVP / OCP protection",
        ))
        .small()
        .color(tokens.text_muted),
    );
    ui.add_space(8.0);

    let panel_w = ui.available_width();
    let gap = 8.0;
    let w_ch = 40.0;
    let w_ovp = 100.0;
    let w_ocp = 100.0;
    let w_apply = (panel_w - w_ch - w_ovp - w_ocp - gap * 3.0).max(72.0);
    let row_h = 34.0;

    ui.allocate_ui_with_layout(
        egui::vec2(panel_w, 20.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (w, label) in [
                (w_ch, "CH"),
                (w_ovp, "OVP"),
                (w_ocp, "OCP"),
                (w_apply, text(lang, "设定", "Apply")),
            ] {
                scope_col(ui, w, 20.0, |ui| {
                    ui.label(
                        RichText::new(label)
                            .strong()
                            .small()
                            .color(tokens.text_muted),
                    );
                });
            }
        },
    );
    ui.add_space(4.0);

    for i in 0..channels {
        let ch = (i as u8) + 1;
        ui.allocate_ui_with_layout(
            egui::vec2(panel_w, row_h),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = gap;
                scope_col(ui, w_ch, row_h, |ui| {
                    ui.label(
                        RichText::new(format!("CH{ch}"))
                            .strong()
                            .monospace()
                            .size(12.0),
                    );
                });
                scope_col(ui, w_ovp, row_h, |ui| {
                    ui.add(
                        egui::DragValue::new(&mut device.controls.source_ovps[i])
                            .range(0.0..=60.0)
                            .speed(0.01)
                            .suffix(" V"),
                    );
                });
                scope_col(ui, w_ocp, row_h, |ui| {
                    ui.add(
                        egui::DragValue::new(&mut device.controls.source_ocps[i])
                            .range(0.0..=20.0)
                            .speed(0.01)
                            .suffix(" A"),
                    );
                });
                scope_col(ui, w_apply, row_h, |ui| {
                    if ui
                        .add_sized(SCOPE_BTN, egui::Button::new(text(lang, "应用", "Apply")))
                        .clicked()
                    {
                        out.push(ControlCommand::SourceOvp {
                            channel: ch,
                            value: device.controls.source_ovps[i],
                        });
                        out.push(ControlCommand::SourceOcp {
                            channel: ch,
                            value: device.controls.source_ocps[i],
                        });
                    }
                });
            },
        );
    }
    out
}

fn load_channel_controls(
    ui: &mut egui::Ui,
    lang: Lang,
    tokens: &Tokens,
    device: &mut DeviceUi,
) -> Vec<ControlCommand> {
    let mut out = Vec::new();
    ui.label(
        RichText::new(text(
            lang,
            "设定工作模式与电平，再控制负载输入开关。",
            "Set mode and level, then toggle load input.",
        ))
        .small()
        .color(tokens.text_muted),
    );
    ui.add_space(10.0);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        ui.label(RichText::new(text(lang, "模式", "Mode")).strong());
        let selected = device.controls.load_mode.clone();
        egui::ComboBox::from_id_salt("load-mode")
            .selected_text(selected)
            .width(88.0)
            .show_ui(ui, |ui| {
                for mode in device.capabilities.load_modes.clone() {
                    ui.selectable_value(&mut device.controls.load_mode, mode.clone(), mode);
                }
            });
        let unit = match device.controls.load_mode.as_str() {
            "CV" => " V",
            "CR" => " Ω",
            "CP" => " W",
            _ => " A",
        };
        ui.label(RichText::new(text(lang, "电平", "Level")).strong());
        ui.add(
            egui::DragValue::new(&mut device.controls.load_level)
                .speed(0.01)
                .suffix(unit),
        );
        if ui
            .add_sized(SCOPE_BTN, egui::Button::new(text(lang, "应用", "Apply")))
            .clicked()
        {
            out.push(ControlCommand::LoadMode(device.controls.load_mode.clone()));
            out.push(ControlCommand::LoadLevel {
                mode: device.controls.load_mode.clone(),
                value: device.controls.load_level,
            });
        }
    });
    ui.add_space(14.0);
    ui.separator();
    ui.add_space(10.0);

    let on = device.controls.load_input;
    let label = if on {
        text(lang, "关闭负载输入", "Input OFF")
    } else {
        text(lang, "开启负载输入", "Input ON")
    };
    let btn = if on {
        egui::Button::new(RichText::new(label).color(tokens.accent_text).strong())
            .fill(tokens.stop_bg)
    } else {
        egui::Button::new(RichText::new(label).color(tokens.accent_text).strong())
            .fill(tokens.accent)
    };
    if ui
        .add_sized(egui::vec2(ui.available_width().min(220.0), 32.0), btn)
        .clicked()
    {
        device.controls.load_input = !on;
        out.push(ControlCommand::LoadInput(device.controls.load_input));
    }
    out
}

fn dmm_setup_controls(
    ui: &mut egui::Ui,
    lang: Lang,
    tokens: &Tokens,
    device: &mut DeviceUi,
) -> Vec<ControlCommand> {
    let mut out = Vec::new();
    ui.label(
        RichText::new(text(
            lang,
            "配置测量功能、量程与 NPLC，然后应用到仪表。",
            "Configure function, range and NPLC, then apply.",
        ))
        .small()
        .color(tokens.text_muted),
    );
    ui.add_space(10.0);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        ui.label(RichText::new(text(lang, "功能", "Function")).strong());
        egui::ComboBox::from_id_salt("dmm-function")
            .selected_text(device.controls.dmm_function.label())
            .width(88.0)
            .show_ui(ui, |ui| {
                for function in MeasureFunction::all() {
                    ui.selectable_value(
                        &mut device.controls.dmm_function,
                        *function,
                        function.label(),
                    );
                }
            });
        ui.checkbox(
            &mut device.controls.dmm_autorange,
            text(lang, "自动量程", "Auto range"),
        );
    });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        ui.label(RichText::new(text(lang, "量程", "Range")).strong());
        ui.add_enabled(
            !device.controls.dmm_autorange,
            egui::DragValue::new(&mut device.controls.dmm_range)
                .speed(0.1)
                .suffix(format!(" {}", device.controls.dmm_function.unit())),
        );
        ui.label(RichText::new("NPLC").strong());
        ui.add(
            egui::DragValue::new(&mut device.controls.dmm_nplc)
                .range(0.001..=100.0)
                .speed(0.01),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add_sized(SCOPE_BTN, egui::Button::new(text(lang, "应用", "Apply")))
                .clicked()
            {
                out.push(ControlCommand::DmmFunction(device.controls.dmm_function));
                out.push(ControlCommand::DmmAutoRange {
                    function: device.controls.dmm_function,
                    enabled: device.controls.dmm_autorange,
                });
                if !device.controls.dmm_autorange {
                    out.push(ControlCommand::DmmRange {
                        function: device.controls.dmm_function,
                        value: device.controls.dmm_range,
                    });
                }
                out.push(ControlCommand::DmmNplc {
                    function: device.controls.dmm_function,
                    value: device.controls.dmm_nplc,
                });
            }
        });
    });
    let _ = tokens;
    out
}

fn card(ui: &mut egui::Ui, tokens: &Tokens, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    Frame::NONE
        .fill(tokens.surface_bg)
        .stroke(Stroke::new(1.0_f32, tokens.border))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::same(10))
        .show(ui, |ui| {
            ui.label(RichText::new(title).strong());
            ui.add_space(5.0);
            add(ui);
        });
    ui.add_space(7.0);
}

const CARD_PANEL_WIDTH: f32 = 258.0;
const CARD_CONTROL_HEIGHT: f32 = 26.0;
const CARD_ICON_COLUMN_WIDTH: f32 = 42.0;

#[derive(Default)]
struct InstrumentCardAction {
    selected: bool,
    connect: bool,
    disconnect: Option<u64>,
}

fn instrument_type_card(
    ui: &mut egui::Ui,
    lang: Lang,
    tokens: &Tokens,
    kind: InstrumentKind,
    selected: bool,
    model: Option<&str>,
    connected_count: usize,
    connected_id: Option<u64>,
    resource_input: &mut String,
    resources: &[ResourceInfo],
) -> InstrumentCardAction {
    let status = if connected_count == 0 {
        text(lang, "未连接", "Disconnected").to_owned()
    } else if connected_count == 1 {
        model
            .unwrap_or(text(lang, "已连接", "Connected"))
            .to_owned()
    } else {
        format!("{} ×{connected_count}", text(lang, "已连接", "Connected"))
    };
    let mut action = InstrumentCardAction::default();
    let matching_count = resources
        .iter()
        .filter(|item| item.kind == Some(kind))
        .count();
    let can_connect = !resource_input.trim().is_empty() || !resources.is_empty();
    // True when an interactive control handled the pointer this frame — blank-area
    // selection must not run in that case (Connect/Disconnect/Combo/TextEdit).
    let mut controls_used = false;

    let frame = Frame::NONE
        .fill(if selected {
            tokens.surface_bg
        } else {
            tokens.panel_bg
        })
        .stroke(Stroke::new(
            if selected { 2.0_f32 } else { 1.0_f32 },
            if selected {
                tokens.accent
            } else {
                tokens.border
            },
        ))
        .corner_radius(CornerRadius::same(7))
        .inner_margin(Margin::symmetric(11, 9))
        .show(ui, |ui| {
            ui.set_min_width(CARD_PANEL_WIDTH);
            ui.set_max_width(CARD_PANEL_WIDTH);
            let content_width =
                (CARD_PANEL_WIDTH - ui.spacing().indent * 2.0).max(CARD_PANEL_WIDTH - 22.0);

            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(CARD_ICON_COLUMN_WIDTH, 0.0),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        ui.label(RichText::new(kind_icon(kind)).monospace().strong());
                    },
                );
                ui.vertical(|ui| {
                    ui.set_width(content_width - CARD_ICON_COLUMN_WIDTH);
                    if ui
                        .selectable_label(
                            selected,
                            RichText::new(instrument_name(lang, kind)).strong(),
                        )
                        .clicked()
                    {
                        action.selected = true;
                        controls_used = true;
                    }
                    ui.label(RichText::new(status).small().color(if connected_count > 0 {
                        tokens.accent
                    } else {
                        tokens.text_muted
                    }));
                    if matching_count > 0 {
                        ui.label(
                            RichText::new(format!(
                                "{}: {matching_count}",
                                text(lang, "已匹配", "Matched"),
                            ))
                            .small()
                            .color(tokens.accent),
                        );
                    }
                });
            });
            ui.separator();
            ui.vertical(|ui| {
                ui.set_width(content_width);
                let display = if resource_input.trim().is_empty() {
                    text(lang, "选择或输入 VISA 地址", "Select or enter VISA address")
                        .to_owned()
                } else {
                    short_resource(resource_input)
                };
                let combo = egui::ComboBox::from_id_salt(("card-visa-resource", kind.label()))
                    .selected_text(display)
                    .width(content_width)
                    .show_ui(ui, |ui| {
                        let mut wrote_matching = false;
                        for resource in resources.iter().filter(|item| item.kind == Some(kind)) {
                            if !wrote_matching {
                                ui.label(
                                    RichText::new(text(lang, "匹配本类型", "Matching this type"))
                                        .small()
                                        .color(tokens.text_muted),
                                );
                                wrote_matching = true;
                            }
                            let label = resource
                                .identity
                                .as_ref()
                                .map(|id| {
                                    format!(
                                        "{}  {} ({})",
                                        resource.transport,
                                        short_resource(&resource.address),
                                        id.model
                                    )
                                })
                                .unwrap_or_else(|| {
                                    format!(
                                        "{}  {}",
                                        resource.transport,
                                        short_resource(&resource.address)
                                    )
                                });
                            ui.selectable_value(resource_input, resource.address.clone(), label);
                        }
                        let mut wrote_others = false;
                        for resource in resources.iter().filter(|item| item.kind != Some(kind)) {
                            if !wrote_others {
                                if wrote_matching {
                                    ui.separator();
                                }
                                ui.label(
                                    RichText::new(text(lang, "其他资源", "Other resources"))
                                        .small()
                                        .color(tokens.text_muted),
                                );
                                wrote_others = true;
                            }
                            let kind_tag = resource.kind.map(|k| k.label()).unwrap_or("?");
                            ui.selectable_value(
                                resource_input,
                                resource.address.clone(),
                                format!(
                                    "[{kind_tag}] {}  {}",
                                    resource.transport,
                                    short_resource(&resource.address)
                                ),
                            );
                        }
                    });
                if combo.response.clicked() {
                    controls_used = true;
                    action.selected = true;
                }
                let edit = ui.add_sized(
                    [content_width, CARD_CONTROL_HEIGHT],
                    egui::TextEdit::singleline(resource_input)
                        .hint_text("USB0::0x0699::...::INSTR")
                        .margin(egui::vec2(6.0, 4.0)),
                );
                if edit.clicked() || edit.gained_focus() {
                    controls_used = true;
                    action.selected = true;
                }
                ui.horizontal(|ui| {
                    ui.set_width(content_width);
                    let gap = ui.spacing().item_spacing.x;
                    let button_width = if connected_id.is_some() {
                        (content_width - gap) * 0.5
                    } else {
                        content_width
                    };
                    if ui
                        .add_enabled(
                            can_connect,
                            egui::Button::new(text(lang, "连接", "Connect"))
                                .fill(tokens.accent)
                                .min_size(egui::vec2(button_width, CARD_CONTROL_HEIGHT)),
                        )
                        .clicked()
                    {
                        controls_used = true;
                        action.connect = true;
                        action.selected = true;
                    }
                    if let Some(id) = connected_id {
                        if ui
                            .add(
                                egui::Button::new(text(lang, "断开", "Disconnect"))
                                    .min_size(egui::vec2(button_width, CARD_CONTROL_HEIGHT)),
                            )
                            .clicked()
                        {
                            controls_used = true;
                            action.disconnect = Some(id);
                        }
                    }
                });
            });
        });

    // Blank areas (icon, status, padding, separator): select the card.
    // Interactive controls set `controls_used` so Connect/edit/combo are unaffected.
    let bg = ui.interact(
        frame.response.rect,
        ui.id().with(("instrument-type-card", kind.label())),
        egui::Sense::click(),
    );
    if bg.clicked() && !controls_used {
        action.selected = true;
    }
    if bg.hovered() && !controls_used {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    action
}

fn kind_sort_key(kind: Option<InstrumentKind>) -> u8 {
    match kind {
        Some(InstrumentKind::Oscilloscope) => 0,
        Some(InstrumentKind::DcSource) => 1,
        Some(InstrumentKind::ElectronicLoad) => 2,
        Some(InstrumentKind::Multimeter) => 3,
        Some(InstrumentKind::Generic) => 4,
        None => 5,
    }
}

fn instrument_kind_slot(kind: InstrumentKind) -> usize {
    match kind {
        InstrumentKind::Oscilloscope => 0,
        InstrumentKind::DcSource => 1,
        InstrumentKind::ElectronicLoad => 2,
        InstrumentKind::Multimeter => 3,
        InstrumentKind::Generic => 0,
    }
}

fn instrument_name(lang: Lang, kind: InstrumentKind) -> &'static str {
    match kind {
        InstrumentKind::Oscilloscope => text(lang, "示波器", "Oscilloscope"),
        InstrumentKind::DcSource => text(lang, "直流电源", "DC Source"),
        InstrumentKind::ElectronicLoad => text(lang, "电子负载", "Electronic Load"),
        InstrumentKind::Multimeter => text(lang, "数字万用表", "Digital Multimeter"),
        InstrumentKind::Generic => text(lang, "通用 SCPI", "Generic SCPI"),
    }
}

fn instrument_control_hint(lang: Lang, kind: InstrumentKind) -> &'static str {
    match kind {
        InstrumentKind::Oscilloscope => text(
            lang,
            "运行/停止/单次/自动设置，以及通道、时基与触发控制。",
            "Run/stop/single/autoset plus channel, timebase and trigger controls.",
        ),
        InstrumentKind::DcSource => text(
            lang,
            "设置电压、电流、OVP/OCP，并控制输出开关。",
            "Set voltage, current, OVP/OCP and control the output state.",
        ),
        InstrumentKind::ElectronicLoad => text(
            lang,
            "选择 CC/CV/CR/CP 模式、设定电平并控制负载输入。",
            "Select CC/CV/CR/CP mode, set level and control load input.",
        ),
        InstrumentKind::Multimeter => text(
            lang,
            "配置测量功能、量程、NPLC，并执行单次测量。",
            "Configure function, range, NPLC and trigger a single reading.",
        ),
        InstrumentKind::Generic => text(
            lang,
            "请使用下方 SCPI 控制台发送命令。",
            "Use the SCPI console below to send commands.",
        ),
    }
}

fn instrument_acquisition_hint(lang: Lang, kind: InstrumentKind) -> &'static str {
    match kind {
        InstrumentKind::Oscilloscope => text(
            lang,
            "截图看仪器画面；波形数据（CURVe）用于统计与 CSV 导出。",
            "Screenshot for the scope display; CURVe samples for stats and CSV export.",
        ),
        InstrumentKind::DcSource | InstrumentKind::ElectronicLoad => text(
            lang,
            "连续读取电压、电流、功率并绘制实时曲线，可导出 CSV。",
            "Continuously read voltage, current and power with live plots and CSV export.",
        ),
        InstrumentKind::Multimeter => text(
            lang,
            "按设定功能连续采样读数，并绘制实时曲线与 CSV 导出。",
            "Continuously sample readings for the selected function with plots and CSV export.",
        ),
        InstrumentKind::Generic => text(
            lang,
            "可通过 READ? 或自定义 SCPI 查询进行采样。",
            "Sample using READ? or custom SCPI queries.",
        ),
    }
}

fn instrument_settings_hint(lang: Lang, kind: InstrumentKind) -> &'static str {
    match kind {
        InstrumentKind::Oscilloscope => text(
            lang,
            "查看设备信息、波形参数、通信超时与诊断命令。",
            "View identity, waveform parameters, timeout and diagnostics.",
        ),
        InstrumentKind::DcSource => text(
            lang,
            "查看电源能力、保护参数、通信设置与诊断命令。",
            "View source capabilities, protection, communication and diagnostics.",
        ),
        InstrumentKind::ElectronicLoad => text(
            lang,
            "查看负载模式能力、通信设置与诊断命令。",
            "View load-mode capabilities, communication and diagnostics.",
        ),
        InstrumentKind::Multimeter => text(
            lang,
            "查看测量能力、量程/NPLC 支持、通信设置与诊断命令。",
            "View measurement capabilities, range/NPLC support, communication and diagnostics.",
        ),
        InstrumentKind::Generic => text(
            lang,
            "查看设备信息与通信诊断命令。",
            "View device identity, communication and diagnostics.",
        ),
    }
}

fn instrument_features(lang: Lang, kind: InstrumentKind) -> [(&'static str, &'static str); 3] {
    let control = match kind {
        InstrumentKind::Oscilloscope => text(
            lang,
            "通道、时基、触发与运行控制",
            "Channels, timebase, trigger and run control",
        ),
        InstrumentKind::DcSource => text(
            lang,
            "电压、电流、保护与输出控制",
            "Voltage, current, protection and output control",
        ),
        InstrumentKind::ElectronicLoad => text(
            lang,
            "CC/CV/CR/CP 模式与输入控制",
            "CC/CV/CR/CP modes and input control",
        ),
        InstrumentKind::Multimeter => text(
            lang,
            "测量功能、量程、分辨率与 NPLC",
            "Functions, range, resolution and NPLC",
        ),
        InstrumentKind::Generic => text(lang, "原始 SCPI 控制", "Raw SCPI control"),
    };
    [
        (
            text(lang, "参数设置", "Parameters"),
            text(
                lang,
                "设备信息、通信参数、能力与诊断",
                "Identity, communication, capabilities and diagnostics",
            ),
        ),
        (text(lang, "控制", "Control"), control),
        (
            text(lang, "数据采集", "Data Acquisition"),
            text(
                lang,
                "单次/连续采样、实时曲线与 CSV",
                "Single/continuous sampling, live plots and CSV",
            ),
        ),
    ]
}

fn setting_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.label(RichText::new(label).strong());
    ui.label(value);
    ui.end_row();
}

fn capability_badge(ui: &mut egui::Ui, tokens: &Tokens, label: impl Into<String>) {
    Frame::NONE
        .fill(tokens.surface_bg)
        .stroke(Stroke::new(1.0_f32, tokens.border))
        .corner_radius(CornerRadius::same(4))
        .inner_margin(Margin::symmetric(7, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(label.into()).small());
        });
}

fn kind_icon(kind: InstrumentKind) -> &'static str {
    match kind {
        InstrumentKind::Oscilloscope => "OSC",
        InstrumentKind::DcSource => "SRC",
        InstrumentKind::ElectronicLoad => "LOAD",
        InstrumentKind::Multimeter => "DMM",
        InstrumentKind::Generic => "SCPI",
    }
}

fn short_resource(resource: &str) -> String {
    const MAX: usize = 34;
    if resource.chars().count() <= MAX {
        resource.to_owned()
    } else {
        format!("{}…", resource.chars().take(MAX - 1).collect::<String>())
    }
}

fn publish_instrument_event(bus: &crate::backend::EventBus, event: &Event) {
    match event {
        Event::Resources(resources) => {
            bus.publish(
                "instrument.resources",
                serde_json::json!({ "count": resources.len(), "resources": resources }),
                None,
            );
        }
        Event::Connected {
            id,
            resource,
            identity,
            kind,
            profile,
            ..
        } => {
            bus.publish(
                "instrument.connected",
                serde_json::json!({
                    "device_id": id,
                    "resource": resource,
                    "identity": identity,
                    "kind": kind,
                    "profile": profile,
                }),
                None,
            );
        }
        Event::Disconnected(id) => {
            bus.publish(
                "instrument.disconnected",
                serde_json::json!({ "device_id": id }),
                None,
            );
        }
        Event::CommandDone { id, response } => {
            bus.publish(
                "instrument.command_done",
                serde_json::json!({ "device_id": id, "response": response }),
                None,
            );
        }
        Event::Measurements {
            id,
            resource,
            readings,
        } => {
            bus.publish(
                "instrument.measurements",
                serde_json::json!({
                    "device_id": id,
                    "resource": resource,
                    "readings": readings,
                }),
                None,
            );
        }
        Event::Screenshot {
            id, width, height, ..
        } => {
            bus.publish(
                "instrument.screenshot",
                serde_json::json!({
                    "device_id": id,
                    "width": width,
                    "height": height,
                }),
                None,
            );
        }
        Event::Waveform { id, trace } => {
            let mut data = serde_json::to_value(trace).unwrap_or_else(|_| serde_json::json!({}));
            if let Some(obj) = data.as_object_mut() {
                obj.insert("device_id".into(), serde_json::json!(id));
            }
            bus.publish("instrument.waveform", data, None);
        }
        Event::WaveformSource {
            id,
            bytes,
            suggested_name,
        } => {
            bus.publish(
                "instrument.waveform_source",
                serde_json::json!({
                    "device_id": id,
                    "bytes": bytes.len(),
                    "suggested_name": suggested_name,
                }),
                None,
            );
        }
        Event::Error { id, message } => {
            bus.publish(
                "instrument.error",
                serde_json::json!({ "device_id": id, "message": message }),
                None,
            );
        }
    }
}

fn text(lang: Lang, zh: &'static str, en: &'static str) -> &'static str {
    if lang == Lang::Zh {
        zh
    } else {
        en
    }
}

fn local(zh: bool, zh_text: &'static str, en_text: &'static str) -> &'static str {
    if zh {
        zh_text
    } else {
        en_text
    }
}
