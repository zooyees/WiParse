//! Unified VISA instrument workbench.

use crate::plot::paint_envelope_columns;
use crate::test_tool::LiveDevice;
use crate::theme::{self, Tokens};
use chrono::Local;
use crossbeam_channel::{unbounded, Receiver, Sender};
use egui::{Color32, CornerRadius, Frame, Margin, Rect, RichText, Stroke};
use egui_plot::{Legend, Line, Plot, PlotPoints};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use wiparse_core::config::AppConfig;
use wiparse_core::i18n::Lang;
use wiparse_core::bridge::Ft4222Session;
use wiparse_core::instrument::{
    discover_resources_with_library, export_csv, humanize_scope_reading_text, parse_control_command,
    AcquisitionBuffer, Capabilities, ControlCommand, Identity, InstrumentDevice, InstrumentKind,
    MeasureFunction, Reading, ResourceInfo, Sample, ScopeMeasType, WaveformTrace,
};
use wiparse_core::probe::ProbeSession;
use wiparse_core::usb::{is_usb_session_address, parse_usb_resource, UsbIface};
use wiparse_core::wave_display::{build_overview_envelope, envelope_bounds, ScopeEnvelopeColumn};
use wiparse_core::waveform_file::{
    join_tek_isf_channels, load_waveform_bytes, load_waveform_bytes_all, save_waveform_file,
    sniff_waveform_ext, waveforms_to_spreadsheet_csv,
};

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
    Command {
        id: u64,
        command: ControlCommand,
        job_id: Option<u64>,
    },
    PollRtt(u64),
    Measure(u64),
    /// Capture scope screen for in-app preview + clipboard (no save dialog).
    Capture {
        id: u64,
        job_id: Option<u64>,
        save_path: Option<PathBuf>,
    },
    /// Read waveform source for all displayed scope channels.
    /// `auto_dir` set → write file (no Save As). Empty → UI Save As dialog.
    WaveformSource {
        id: u64,
        job_id: Option<u64>,
        auto_dir: Option<PathBuf>,
        auto_filename: Option<String>,
        overwrite: bool,
    },
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
        job_id: Option<u64>,
        response: Option<String>,
    },
    SessionOutput {
        id: u64,
        text: Option<String>,
    },
    Measurements {
        id: u64,
        resource: String,
        readings: Vec<Reading>,
    },
    Screenshot {
        id: u64,
        job_id: Option<u64>,
        save_path: Option<PathBuf>,
        width: usize,
        height: usize,
        rgba: Vec<u8>,
        png: Vec<u8>,
    },
    Waveform {
        id: u64,
        trace: WaveformTrace,
    },
    /// Raw waveform source file bytes from the instrument (ready for Save As or auto-save).
    WaveformSource {
        id: u64,
        job_id: Option<u64>,
        auto_dir: Option<PathBuf>,
        auto_filename: Option<String>,
        overwrite: bool,
        bytes: Vec<u8>,
        suggested_name: String,
        /// Parsed on the worker thread so the UI does not freeze on multi‑Mpoint ISF.
        trace: Option<WaveformTrace>,
        /// All displayed channels (for multi-file / multi-column save).
        traces: Vec<WaveformTrace>,
        /// Per-channel native blobs `(CHx, bytes, ext)`.
        channel_files: Vec<(u8, Vec<u8>, String)>,
        parse_error: Option<String>,
    },
    /// Long-running job progress (screenshot / waveform source transfer).
    Progress {
        id: Option<u64>,
        message: String,
    },
    Error {
        id: Option<u64>,
        job_id: Option<u64>,
        message: String,
    },
}

/// Deferred Save As after waveform-source parse (open dialog next frame so the
/// plot/status update is visible first; avoids rfd failing mid-event).
struct PendingWaveformSave {
    id: u64,
    job_id: Option<u64>,
    bytes: Vec<u8>,
    effective_ext: String,
    file_stem: String,
    vendor: String,
    parsed_trace: Option<WaveformTrace>,
    traces: Vec<WaveformTrace>,
    /// Per-channel native blobs `(channel, bytes, ext)`.
    channel_files: Vec<(u8, Vec<u8>, String)>,
}

/// Completion of an API / test-plan instrument job (path recorded, not point arrays).
#[derive(Debug, Clone)]
pub struct InstrumentJobResult {
    pub job_id: u64,
    pub ok: bool,
    pub kind: String,
    pub device_id: Option<u64>,
    pub detail: serde_json::Value,
}

/// Prebuilt plot series — rebuilt only when a new waveform arrives.
struct CachedWavePlot {
    channel: String,
    columns: Arc<Vec<ScopeEnvelopeColumn>>,
    /// Precomputed plot bounds (display columns only).
    bounds: (f64, f64, f64, f64),
    stats: WaveformStats,
}

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
    probe_chip: String,
    probe_flash_path: String,
    probe_flash_verify: bool,
    probe_mem_addr: String,
    probe_mem_len: u32,
    probe_mem_write_hex: String,
    probe_rtt_on: bool,
    probe_rtt_channel: u32,
    probe_speed_khz: u32,
    io_log: String,
    bridge_tab: u8,
    bridge_spi_mode: u8,
    bridge_spi_hz: u32,
    bridge_spi_cs: u8,
    bridge_write_hex: String,
    bridge_read_len: u32,
    bridge_i2c_addr: String,
    bridge_i2c_hz: u32,
    bridge_gpio_pin: u8,
    bridge_gpio_out: bool,
    bridge_gpio_value: bool,
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
            probe_chip: String::new(),
            probe_flash_path: String::new(),
            probe_flash_verify: true,
            probe_mem_addr: "08000000".into(),
            probe_mem_len: 64,
            probe_mem_write_hex: String::new(),
            probe_rtt_on: false,
            probe_rtt_channel: 0,
            probe_speed_khz: 4_000,
            io_log: String::new(),
            bridge_tab: 0,
            bridge_spi_mode: 0,
            bridge_spi_hz: 1_000_000,
            bridge_spi_cs: 0,
            bridge_write_hex: String::new(),
            bridge_read_len: 4,
            bridge_i2c_addr: "50".into(),
            bridge_i2c_hz: 100_000,
            bridge_gpio_pin: 0,
            bridge_gpio_out: true,
            bridge_gpio_value: false,
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
    last_activity: String,
}

pub struct InstrumentControlPanel {
    tx: Sender<Job>,
    rx: Receiver<Event>,
    resources: Vec<ResourceInfo>,
    resource_inputs: [String; 6],
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
    /// Left-rail "control mesh" card — shows every live session on the right.
    overview_open: bool,
    /// Optional device photos loaded from `{save_dir}/device_photos/`.
    device_photos: HashMap<u64, egui::TextureHandle>,
    /// Device ids already probed for a photo (avoid disk I/O every frame).
    device_photo_tried: HashSet<u64>,
    /// When set, every instrument card is shown and missing kinds use demo sessions.
    debug_mode: bool,
    /// Device id currently running Capture / WaveformSource (for status + UI busy).
    busy_device: Option<u64>,
    busy_started: Option<Instant>,
    busy_label: String,
    /// Waveform source Save As, opened after a short UI settle delay.
    pending_waveform_save: Option<PendingWaveformSave>,
    pending_save_delay_frames: u8,
    next_job_id: u64,
    job_results: Vec<InstrumentJobResult>,
    last_rtt: Instant,
    rtt_pending: HashSet<u64>,
}

fn kind_wire(kind: InstrumentKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("{kind:?}").to_ascii_lowercase())
}

impl InstrumentControlPanel {
    pub fn new(cfg: &AppConfig) -> Self {
        let (tx, jobs) = unbounded();
        let (events, rx) = unbounded();
        thread::spawn(move || worker_loop(jobs, events));
        let instrument_cfg = &cfg.apps.instruments;
        let mut resource_inputs: [String; 6] = std::array::from_fn(|_| String::new());
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
            overview_open: true,
            device_photos: HashMap::new(),
            device_photo_tried: HashSet::new(),
            debug_mode: false,
            busy_device: None,
            busy_started: None,
            busy_label: String::new(),
            pending_waveform_save: None,
            pending_save_delay_frames: 0,
            next_job_id: 1,
            job_results: Vec::new(),
            last_rtt: Instant::now(),
            rtt_pending: HashSet::new(),
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
            InstrumentKind::DebugProbe,
            InstrumentKind::UsbBridge,
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
        const ALL: [InstrumentKind; 6] = [
            InstrumentKind::Oscilloscope,
            InstrumentKind::DcSource,
            InstrumentKind::ElectronicLoad,
            InstrumentKind::Multimeter,
            InstrumentKind::DebugProbe,
            InstrumentKind::UsbBridge,
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
        let results_before = self.job_results.len();
        while let Ok(event) = self.rx.try_recv() {
            if let Some(bus) = bus {
                publish_instrument_event(bus, &event);
            }
            self.handle_event(ctx, event);
        }
        if let Some(bus) = bus {
            for r in self.job_results.iter().skip(results_before) {
                bus.publish(
                    "instrument.job_done",
                    serde_json::json!({
                        "job_id": r.job_id,
                        "ok": r.ok,
                        "kind": r.kind,
                        "device_id": r.device_id,
                        "detail": r.detail,
                    }),
                    None,
                );
            }
        }
        // Open Save As one frame after parse so the waveform preview is already painted.
        if self.pending_save_delay_frames > 0 {
            self.pending_save_delay_frames -= 1;
            ctx.request_repaint();
        } else if self.pending_waveform_save.is_some() {
            self.run_pending_waveform_save();
        }
        if let (Some(_), Some(started)) = (self.busy_device, self.busy_started) {
            let secs = started.elapsed().as_secs();
            if !self.busy_label.is_empty() {
                self.status = format!("{}… {}s", self.busy_label, secs);
            }
            ctx.request_repaint_after(Duration::from_millis(200));
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
        if self.last_rtt.elapsed() >= Duration::from_millis(150) {
            self.last_rtt = Instant::now();
            let rtt_ids: Vec<u64> = self
                .devices
                .iter()
                .filter(|device| {
                    device.kind == InstrumentKind::DebugProbe && device.controls.probe_rtt_on
                })
                .map(|device| device.id)
                .collect();
            for id in rtt_ids {
                if self.rtt_pending.insert(id) {
                    let _ = self.tx.send(Job::PollRtt(id));
                }
            }
            if !self.rtt_pending.is_empty() {
                ctx.request_repaint_after(Duration::from_millis(150));
            }
        }
    }

    fn clear_busy(&mut self) {
        self.busy_device = None;
        self.busy_started = None;
        self.busy_label.clear();
    }

    fn begin_busy(&mut self, id: u64, label: impl Into<String>) {
        self.busy_device = Some(id);
        self.busy_started = Some(Instant::now());
        self.busy_label = label.into();
    }

    fn run_pending_waveform_save(&mut self) {
        let Some(pending) = self.pending_waveform_save.take() else {
            return;
        };
        let default = self.save_dir.join(format!(
            "{}_{}.{}",
            pending.file_stem,
            Local::now().format("%Y%m%d_%H%M%S"),
            pending.effective_ext
        ));
        let dialog = scope_waveform_save_dialog(
            &pending.vendor,
            &self.save_dir,
            default
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .as_ref(),
            &pending.effective_ext,
        );
                if let Some(path) = dialog.save_file() {
            match save_pending_waveform_source(&path, &pending) {
                Ok(saved) => {
                    self.status = if saved.len() == 1 {
                        format!(
                            "已保存波形源 / Waveform source saved: {}",
                            saved[0].display()
                        )
                    } else {
                        format!(
                            "已保存 {} 个通道波形源 / Saved {} channel sources → {}",
                            saved.len(),
                            saved.len(),
                            path.parent()
                                .unwrap_or_else(|| std::path::Path::new("."))
                                .display()
                        )
                    };
                    if let Some(parent) = path.parent() {
                        self.save_dir = parent.to_path_buf();
                    }
                    self.complete_job(
                        pending.job_id,
                        true,
                        "waveform_source",
                        Some(pending.id),
                        serde_json::json!({
                            "path": saved.first().map(|p| p.display().to_string()),
                            "files": saved.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
                            "bytes": pending.bytes.len(),
                        }),
                    );
                }
                Err(error) => {
                    self.status = error.to_string();
                    self.complete_job(
                        pending.job_id,
                        false,
                        "waveform_source",
                        Some(pending.id),
                        serde_json::json!({ "error": error }),
                    );
                }
            }
        } else {
            if let Some(trace) = self.waveforms.get(&pending.id) {
                self.status = format!(
                    "波形已显示（{} 点），另存为已取消 / Waveform shown ({} pts), Save As cancelled",
                    trace.x.len(),
                    trace.x.len()
                );
            } else if !pending.bytes.is_empty() {
                self.status = format!(
                    "另存为已取消（已收到 {} KB 原始数据）/ Save As cancelled ({} KB raw kept)",
                    pending.bytes.len() / 1024,
                    pending.bytes.len() / 1024
                );
            }
            self.complete_job(
                pending.job_id,
                false,
                "waveform_source",
                Some(pending.id),
                serde_json::json!({ "error": "save cancelled" }),
            );
        }
    }

    fn auto_save_waveform_source(
        &mut self,
        pending: PendingWaveformSave,
        dir: &std::path::Path,
        filename: Option<&str>,
        overwrite: bool,
    ) {
        if let Err(e) = std::fs::create_dir_all(dir) {
            self.status = format!("waveform_source dir: {e}");
            self.complete_job(
                pending.job_id,
                false,
                "waveform_source",
                Some(pending.id),
                serde_json::json!({ "error": e.to_string() }),
            );
            return;
        }
        let name = filename
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("{}.{}", pending.file_stem, pending.effective_ext));
        let path = wiparse_core::paths::unique_file_path(dir, &name, overwrite);
        match save_pending_waveform_source(&path, &pending) {
            Ok(saved) => {
                self.status = format!(
                    "已保存波形源 / Waveform source saved: {}",
                    saved
                        .first()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| path.display().to_string())
                );
                self.log(format!("WaveformSource auto-save → {}", path.display()));
                self.complete_job(
                    pending.job_id,
                    true,
                    "waveform_source",
                    Some(pending.id),
                    serde_json::json!({
                        "path": saved.first().map(|p| p.display().to_string()),
                        "files": saved.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
                        "bytes": pending.bytes.len(),
                    }),
                );
            }
            Err(error) => {
                self.status = error.clone();
                self.complete_job(
                    pending.job_id,
                    false,
                    "waveform_source",
                    Some(pending.id),
                    serde_json::json!({ "error": error }),
                );
            }
        }
    }

    fn alloc_job_id(&mut self) -> u64 {
        let id = self.next_job_id;
        self.next_job_id = self.next_job_id.saturating_add(1);
        id
    }

    fn complete_job(
        &mut self,
        job_id: Option<u64>,
        ok: bool,
        kind: &str,
        device_id: Option<u64>,
        detail: serde_json::Value,
    ) {
        let Some(job_id) = job_id else {
            return;
        };
        self.job_results.push(InstrumentJobResult {
            job_id,
            ok,
            kind: kind.into(),
            device_id,
            detail,
        });
    }

    pub fn take_job_results(&mut self) -> Vec<InstrumentJobResult> {
        std::mem::take(&mut self.job_results)
    }

    pub fn first_oscilloscope_id(&self) -> Option<u64> {
        if let Some(id) = self.selected_id {
            if self
                .devices
                .iter()
                .any(|d| d.id == id && d.kind == InstrumentKind::Oscilloscope)
            {
                return Some(id);
            }
        }
        self.devices
            .iter()
            .find(|d| d.kind == InstrumentKind::Oscilloscope)
            .map(|d| d.id)
    }

    pub fn device_count(&self) -> usize {
        self.devices.len()
    }

    pub fn live_devices_for_hub(&self) -> Vec<LiveDevice> {
        self.devices
            .iter()
            .map(|d| LiveDevice {
                device_id: d.id,
                resource: d.resource.clone(),
                kind: kind_wire(d.kind),
                model: d.identity.model.clone(),
                manufacturer: d.identity.manufacturer.clone(),
                serial: d.identity.serial.clone(),
            })
            .collect()
    }

    pub fn selected_device_id(&self) -> Option<u64> {
        self.selected_id
    }

    pub fn overview_open(&self) -> bool {
        self.overview_open
    }

    pub fn is_scanning(&self) -> bool {
        self.scanning
    }

    pub fn api_select(&mut self, params: &serde_json::Value) -> crate::backend::InvokeReply {
        use crate::backend::{invoke_err as err, invoke_ok as ok};
        let overview = params
            .get("overview")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            || params
                .get("kind")
                .and_then(|v| v.as_str())
                .is_some_and(|k| k.eq_ignore_ascii_case("overview") || k.eq_ignore_ascii_case("mesh"));
        if overview {
            self.select_overview();
            return ok(
                "ui.instrument.select",
                serde_json::json!({
                    "overview": true,
                    "device_id": self.selected_id,
                }),
            );
        }
        if let Some(kind_val) = params.get("kind") {
            if let Ok(kind) = serde_json::from_value::<InstrumentKind>(kind_val.clone()) {
                self.select_kind(kind);
            } else if let Some(s) = kind_val.as_str() {
                return err(
                    "ui.instrument.select",
                    &format!("unknown kind '{s}'"),
                );
            }
        }
        let id = params
            .get("device_id")
            .or_else(|| params.get("id"))
            .and_then(|v| v.as_u64());
        if let Some(id) = id {
            let Some(kind) = self.devices.iter().find(|d| d.id == id).map(|d| d.kind) else {
                return err("ui.instrument.select", "device not found");
            };
            self.select_kind(kind);
            self.selected_id = Some(id);
            return ok(
                "ui.instrument.select",
                serde_json::json!({ "device_id": id, "kind": kind, "overview": false }),
            );
        }
        if params.get("kind").is_some() {
            return ok(
                "ui.instrument.select",
                serde_json::json!({
                    "device_id": self.selected_id,
                    "kind": self.selected_kind,
                    "overview": false,
                }),
            );
        }
        err(
            "ui.instrument.select",
            "missing device_id (or pass overview=true / kind)",
        )
    }

    pub fn api_list(&self, lang: Lang, params: &serde_json::Value) -> serde_json::Value {
        let kind_filter = params
            .get("kind")
            .and_then(|v| serde_json::from_value::<InstrumentKind>(v.clone()).ok());
        self.api_inventory(lang, kind_filter, false)
    }

    pub fn api_overview(&self, lang: Lang) -> serde_json::Value {
        self.api_inventory(lang, None, true)
    }

    fn api_inventory(
        &self,
        lang: Lang,
        kind_filter: Option<InstrumentKind>,
        twin: bool,
    ) -> serde_json::Value {
        let devices: Vec<_> = self
            .devices
            .iter()
            .filter(|d| kind_filter.map(|k| d.kind == k).unwrap_or(true))
            .map(|d| self.api_device_snapshot(lang, d, twin))
            .collect();
        let resources: Vec<_> = self
            .resources
            .iter()
            .filter(|r| kind_filter.map(|k| r.kind == Some(k)).unwrap_or(true))
            .map(|r| {
                serde_json::json!({
                    "address": r.address,
                    "transport": r.transport,
                    "kind": r.kind,
                    "identity": r.identity,
                    "probe_error": r.probe_error,
                    "connected": self.devices.iter().any(|d| d.resource == r.address),
                })
            })
            .collect();
        let live = self
            .devices
            .iter()
            .filter(|d| d.acquiring && !d.paused)
            .count();
        serde_json::json!({
            "devices": devices,
            "scanning": self.scanning,
            "resources": resources,
            "overview_open": self.overview_open,
            "selected_id": self.selected_id,
            "sessions": self.devices.len(),
            "live": live,
            "busy_device": self.busy_device,
            "busy_label": self.busy_label,
        })
    }

    fn api_device_snapshot(&self, lang: Lang, d: &DeviceUi, twin: bool) -> serde_json::Value {
        let (status, status_color) =
            overview_runtime_status(lang, d, self.busy_device, self.busy_label.as_str());
        let params = overview_param_lines(lang, d, &self.latest);
        let readings: Vec<_> = self
            .latest
            .iter()
            .filter(|((id, _), _)| *id == d.id)
            .map(|((_, ch), r)| {
                serde_json::json!({
                    "channel": ch,
                    "value": r.value,
                    "unit": r.unit,
                })
            })
            .collect();
        let mut obj = serde_json::json!({
            "device_id": d.id,
            "resource": d.resource,
            "kind": d.kind,
            "profile": d.profile,
            "identity": d.identity,
            "capabilities": d.capabilities,
            "acquiring": d.acquiring,
            "paused": d.paused,
            "live": d.acquiring && !d.paused,
            "last_activity": d.last_activity,
            "status": status,
            "status_key": overview_status_key(d, self.busy_device),
            "status_color": format!(
                "#{:02X}{:02X}{:02X}",
                status_color.r(),
                status_color.g(),
                status_color.b()
            ),
            "channels": d.capabilities.channels,
            "params": params.iter().map(|(k, v)| serde_json::json!({ "key": k, "value": v })).collect::<Vec<_>>(),
            "readings": readings,
        });
        if twin {
            let face = twin_face_of(d, &self.latest, self.waveforms.get(&d.id));
            obj["name"] = serde_json::json!(if d.identity.model.trim().is_empty() {
                instrument_name(lang, d.kind).to_owned()
            } else {
                d.identity.model.clone()
            });
            obj["face"] = twin_face_json(&face);
            obj["has_photo"] = serde_json::json!(self.device_photos.contains_key(&d.id));
        }
        obj
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

    pub fn api_connect_all(
        &mut self,
        lang: Lang,
    ) -> crate::backend::InvokeReply {
        use crate::backend::{invoke_err as err, invoke_ok as ok};
        let queued = self.queue_connect_all(lang);
        if queued.is_empty() {
            return err(
                "instrument.connect_all",
                "no new discovered resources to connect",
            );
        }
        ok(
            "instrument.connect_all",
            serde_json::json!({ "accepted": true, "queued": queued }),
        )
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
            Some(c) => match parse_control_command(c) {
                Ok(cmd) => cmd,
                Err(e) => return err("instrument.command", &e),
            },
            None => return err("instrument.command", "missing command"),
        };
        let job_id = Some(self.alloc_job_id());
        match self.tx.send(Job::Command {
            id,
            command,
            job_id,
        }) {
            Ok(()) => ok(
                "instrument.command",
                serde_json::json!({ "accepted": true, "device_id": id, "job_id": job_id }),
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
            None => match self.first_oscilloscope_id() {
                Some(id) => id,
                None => return err("instrument.capture", "missing device_id"),
            },
        };
        if !self.devices.iter().any(|d| d.id == id) {
            return err("instrument.capture", "device not connected");
        }
        let save_path = params
            .get("path")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);
        let wait = save_path.is_some()
            || params
                .get("wait")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
        let job_id = if wait { Some(self.alloc_job_id()) } else { None };
        self.begin_busy(id, "Capturing screen");
        match self.tx.send(Job::Capture {
            id,
            job_id,
            save_path,
        }) {
            Ok(()) => ok(
                "instrument.capture",
                serde_json::json!({ "accepted": true, "device_id": id, "job_id": job_id }),
            ),
            Err(e) => err("instrument.capture", &e.to_string()),
        }
    }

    pub fn api_waveform_source(
        &mut self,
        params: &serde_json::Value,
    ) -> crate::backend::InvokeReply {
        use crate::backend::{invoke_err as err, invoke_ok as ok};
        let id = match params
            .get("device_id")
            .or_else(|| params.get("id"))
            .and_then(|v| v.as_u64())
        {
            Some(id) => id,
            None => match self.first_oscilloscope_id() {
                Some(id) => id,
                None => return err("instrument.waveform_source", "no connected oscilloscope"),
            },
        };
        if !self.devices.iter().any(|d| d.id == id) {
            return err("instrument.waveform_source", "device not connected");
        }
        let dir = params
            .get("dir")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);
        let filename = params
            .get("filename")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let overwrite = params
            .get("overwrite")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let job_id = Some(self.alloc_job_id());
        self.begin_busy(id, "Reading waveform source");
        match self.tx.send(Job::WaveformSource {
            id,
            job_id,
            auto_dir: dir,
            auto_filename: filename,
            overwrite,
        }) {
            Ok(()) => ok(
                "instrument.waveform_source",
                serde_json::json!({
                    "accepted": true,
                    "job_id": job_id,
                    "device_id": id,
                }),
            ),
            Err(e) => err("instrument.waveform_source", &e.to_string()),
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
                    last_activity: "LINK UP".into(),
                });
                if self.overview_open {
                    self.selected_id = Some(id);
                } else {
                    self.select_kind(kind);
                    self.selected_id = Some(id);
                }
            }
            Event::Disconnected(id) => {
                if self.busy_device == Some(id) {
                    self.clear_busy();
                }
                self.devices.retain(|device| device.id != id);
                self.device_photos.remove(&id);
                self.device_photo_tried.remove(&id);
                self.measurement_pending.remove(&id);
                self.waveforms.remove(&id);
                self.wave_plots.remove(&id);
                self.screenshots.remove(&id);
                self.screenshot_png.remove(&id);
                self.rtt_pending.remove(&id);
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
            Event::CommandDone { id, job_id, response } => {
                self.rtt_pending.remove(&id);
                if let Some(response) = response.clone() {
                    if let Some(device) = self.devices.iter_mut().find(|device| device.id == id) {
                        store_scope_measure_result(&mut device.controls, &response);
                        append_io_log(&mut device.controls.io_log, &response);
                        note_activity(&mut device.last_activity, &response);
                    }
                    self.status = response.clone();
                    self.log(format!("#{id} ◀ {response}"));
                    self.complete_job(
                        job_id,
                        true,
                        "command",
                        Some(id),
                        serde_json::json!({ "response": response }),
                    );
                } else {
                    self.status = "Command completed".into();
                    self.complete_job(
                        job_id,
                        true,
                        "command",
                        Some(id),
                        serde_json::json!({ "response": serde_json::Value::Null }),
                    );
                }
            }
            Event::SessionOutput { id, text } => {
                self.rtt_pending.remove(&id);
                if let Some(text) = text {
                    if let Some(device) = self.devices.iter_mut().find(|device| device.id == id) {
                        append_io_log(&mut device.controls.io_log, &text);
                        note_activity(&mut device.last_activity, "RTT");
                    }
                    self.log(format!("#{id} RTT {text}"));
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
                if let Some(device) = self.devices.iter_mut().find(|device| device.id == id) {
                    note_activity(&mut device.last_activity, "TELEMETRY");
                }
                self.status = format!("Updated {}", Local::now().format("%H:%M:%S"));
            }
            Event::Screenshot {
                id,
                job_id,
                save_path,
                width,
                height,
                rgba,
                png,
            } => {
                self.clear_busy();
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
                self.screenshot_png.insert(id, png.clone());
                if let Some(device) = self.devices.iter_mut().find(|device| device.id == id) {
                    note_activity(&mut device.last_activity, "SCREEN CAPTURE");
                }
                self.status =
                    "截图已显示并复制到剪贴板 / Screenshot copied to clipboard".into();
                if let Some(path) = save_path {
                    if let Some(parent) = path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    match std::fs::write(&path, &png) {
                        Ok(()) => {
                            self.status = format!(
                                "截图已保存 / Screenshot saved: {}",
                                path.display()
                            );
                            self.complete_job(
                                job_id,
                                true,
                                "capture",
                                Some(id),
                                serde_json::json!({
                                    "path": path.display().to_string(),
                                    "bytes": png.len(),
                                    "width": width,
                                    "height": height,
                                }),
                            );
                        }
                        Err(e) => {
                            self.status = format!("screenshot save failed: {e}");
                            self.complete_job(
                                job_id,
                                false,
                                "capture",
                                Some(id),
                                serde_json::json!({ "error": e.to_string() }),
                            );
                        }
                    }
                } else {
                    self.complete_job(
                        job_id,
                        true,
                        "capture",
                        Some(id),
                        serde_json::json!({
                            "bytes": png.len(),
                            "width": width,
                            "height": height,
                        }),
                    );
                }
            }
            Event::Waveform { id, trace } => {
                self.clear_busy();
                self.status = format!("{}: {} points", trace.channel, trace.x.len());
                self.wave_plots
                    .insert(id, build_cached_wave_plot(&trace));
                self.waveforms.insert(id, trace);
            }
            Event::WaveformSource {
                id,
                job_id,
                auto_dir,
                auto_filename,
                overwrite,
                bytes,
                suggested_name,
                trace,
                mut traces,
                channel_files,
                parse_error,
            } => {
                self.clear_busy();
                let stem = PathBuf::from(&suggested_name);
                let mut effective_ext = stem
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("isf")
                    .to_ascii_lowercase();
                // Keep instrument-native format (Tek ISF / Rigol WFM). Multi-channel
                // Save As writes sibling `stem_CHx.isf` files; CSV remains optional.
                if let Some(sniffed) = sniff_waveform_ext(&bytes) {
                    if sniffed != effective_ext {
                        effective_ext = sniffed.to_string();
                    }
                } else if let Some((_, _, ext)) = channel_files.first() {
                    effective_ext = ext.clone();
                }
                let vendor = self
                    .devices
                    .iter()
                    .find(|d| d.id == id)
                    .map(|d| d.identity.manufacturer.clone())
                    .unwrap_or_default();

                let ch_labels: Vec<String> = channel_files
                    .iter()
                    .map(|(ch, _, _)| format!("CH{ch}"))
                    .collect();
                let ch_summary = if ch_labels.is_empty() {
                    "?".into()
                } else {
                    ch_labels.join("+")
                };

                let nch = traces.len();
                let parse_ok = !traces.is_empty();
                if parse_ok {
                    let raw_n = traces[0].x.len();
                    self.status = format!(
                        "已读取 {ch_summary}（{nch} 通道, {raw_n} 点/通道, {} KB；预览仅作显示抽稀，测量/导出用全量数据）/ Read {ch_summary} ({nch} ch, {raw_n} pts/ch, {} KB; plot is display-only decimation, measure/export use full data)",
                        bytes.len() / 1024,
                        bytes.len() / 1024
                    );
                    // Preview trace moves into waveforms; pending keeps rest for multi-ch Save As.
                    let preview = traces.remove(0);
                    self.wave_plots.insert(
                        id,
                        build_cached_wave_plot(&preview),
                    );
                    self.waveforms.insert(id, preview);
                    self.log(format!(
                        "WaveformSource OK: {ch_summary}, {} points, {} bytes → {}",
                        raw_n,
                        bytes.len(),
                        suggested_name
                    ));
                } else if let Some(err) = parse_error {
                    self.status = format!(
                        "已收到 {} KB 但解析失败: {err} / Got {} KB but parse failed: {err}",
                        bytes.len() / 1024,
                        bytes.len() / 1024
                    );
                    self.log(format!("WaveformSource parse error: {err}"));
                } else {
                    self.status = format!(
                        "已收到 {} KB，无波形点 / Got {} KB, no trace",
                        bytes.len() / 1024,
                        bytes.len() / 1024
                    );
                }
                let _ = trace; // worker no longer sends a duplicate CH1 copy

                let pending = PendingWaveformSave {
                    id,
                    job_id,
                    bytes,
                    effective_ext,
                    file_stem: stem
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("waveform")
                        .to_string(),
                    vendor,
                    parsed_trace: None,
                    traces: if nch > 1 { traces } else { Vec::new() },
                    channel_files,
                };
                if let Some(dir) = auto_dir.filter(|p| !p.as_os_str().is_empty()) {
                    self.auto_save_waveform_source(
                        pending,
                        &dir,
                        auto_filename.as_deref(),
                        overwrite,
                    );
                } else {
                    // Defer Save As so the plot/status paint first.
                    self.pending_waveform_save = Some(pending);
                    self.pending_save_delay_frames = 2;
                    ctx.request_repaint();
                }
            }
            Event::Progress { id: _, message } => {
                self.busy_label = message.clone();
                self.status = message;
                if self.busy_started.is_none() {
                    self.busy_started = Some(Instant::now());
                }
            }
            Event::Error { id, job_id, message } => {
                self.clear_busy();
                if id.is_none() {
                    self.scanning = false;
                }
                if let Some(id) = id {
                    self.measurement_pending.remove(&id);
                    self.rtt_pending.remove(&id);
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
                self.complete_job(
                    job_id,
                    false,
                    "error",
                    id,
                    serde_json::json!({ "error": message }),
                );
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
        self.overview_open = false;
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

    fn select_overview(&mut self) {
        self.overview_open = true;
        self.selected_id = None;
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

        let mut assigned = [false; 6];
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
            "SCAN {} resource(s); auto-assigned by *IDN? / USB VID-PID",
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
                "无可用资源，请先扫描或手动输入地址",
                "No resource available. Scan or enter an address first.",
            )
            .into();
            return;
        };
        self.begin_connect_address(kind, resource, lang, true);
    }

    fn begin_connect_address(
        &mut self,
        kind: InstrumentKind,
        resource: String,
        lang: Lang,
        select: bool,
    ) -> Option<u64> {
        if self.devices.iter().any(|d| d.resource == resource) {
            self.status = text(lang, "该地址已连接", "Already connected").into();
            return None;
        }
        let slot = instrument_kind_slot(kind);
        if slot < self.resource_inputs.len() {
            self.resource_inputs[slot] = resource.clone();
        }
        let id = self.next_id;
        self.next_id += 1;
        if select && !self.overview_open {
            self.select_kind(kind);
        }
        self.status = text(lang, "正在连接…", "Connecting…").into();
        let connect_kind = if is_usb_session_address(&resource) {
            parse_usb_resource(&resource).map(|spec| spec.iface.kind())
        } else if matches!(
            kind,
            InstrumentKind::DebugProbe | InstrumentKind::UsbBridge
        ) {
            Some(kind)
        } else {
            None
        };
        let _ = self.tx.send(Job::Connect {
            id,
            resource,
            kind: connect_kind,
            timeout_ms: self.timeout_ms,
            library: self.visa_library.clone(),
        });
        Some(id)
    }

    fn queue_connect_all(&mut self, lang: Lang) -> Vec<serde_json::Value> {
        let pending: Vec<(InstrumentKind, String)> = self
            .resources
            .iter()
            .filter(|r| {
                !r.address.trim().is_empty()
                    && !self.devices.iter().any(|d| d.resource == r.address)
            })
            .map(|r| {
                (
                    r.kind.unwrap_or(InstrumentKind::Generic),
                    r.address.clone(),
                )
            })
            .collect();
        let mut queued = Vec::new();
        for (kind, address) in pending {
            if let Some(id) = self.begin_connect_address(kind, address.clone(), lang, false) {
                queued.push(serde_json::json!({
                    "device_id": id,
                    "resource": address,
                    "kind": kind,
                }));
            }
        }
        if queued.is_empty() {
            self.status = text(lang, "没有可连接的新设备", "Nothing new to connect").into();
        } else {
            self.status = format!(
                "{} {}",
                text(lang, "正在连接全部", "Connecting all"),
                queued.len()
            );
        }
        queued
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
            .stroke(Stroke::new(1.0_f32, tokens.divider))
            .corner_radius(CornerRadius::same(theme::RADIUS_CARD))
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
                        let pending = self
                            .resources
                            .iter()
                            .filter(|r| {
                                !r.address.trim().is_empty()
                                    && !self.devices.iter().any(|d| d.resource == r.address)
                            })
                            .count();
                        if ui
                            .add_enabled(
                                pending > 0,
                                egui::Button::new(text(lang, "连接全部", "Connect all")),
                            )
                            .clicked()
                        {
                            let _ = self.queue_connect_all(lang);
                        }
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
                            self.begin_connect_address(target, address, lang, false);
                        }
                    });
            });
    }

    fn device_rack(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        ui.set_min_height(ui.available_height());
        Frame::NONE
            .fill(tokens.panel_bg)
            .stroke(Stroke::new(1.0_f32, tokens.divider))
            .corner_radius(CornerRadius::same(theme::RADIUS_CARD))
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
                            "扫描后通过 *IDN? / USB VID-PID 识别并分发到对应卡片",
                            "After scan, *IDN? / USB VID-PID classifies devices onto matching cards",
                        ));
                    }
                });
                ui.label(
                    RichText::new(text(
                        lang,
                        "扫描后自动识别 VISA 与 USB 探针/桥并填入对应卡片；也可手动选择地址后连接",
                        "Scan auto-classifies VISA and USB probes/bridges onto cards; or pick an address manually",
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
                        let overview_action = overview_type_card(
                            ui,
                            lang,
                            tokens,
                            self.overview_open,
                            self.devices.len(),
                        );
                        if overview_action {
                            self.select_overview();
                        }
                        ui.add_space(7.0);
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
                                !self.overview_open && self.selected_kind == kind,
                                matching_device.as_ref().map(|(_, model)| model.as_str()),
                                matching_count,
                                matching_device.as_ref().map(|(id, _)| *id),
                                &mut self.resource_inputs[input_slot],
                                &self.resources,
                            );
                            if action.selected {
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
        if self.overview_open {
            self.overview_workspace(ui, lang, tokens);
            return;
        }
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
            InstrumentKind::DebugProbe => self.probe_workspace(ui, lang, tokens, id, index),
            InstrumentKind::UsbBridge => self.bridge_workspace(ui, lang, tokens, id, index),
            InstrumentKind::Generic => {
                self.generic_workspace(ui, lang, tokens, id, index, kind);
            }
        }
    }

    fn overview_workspace(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        ui.ctx().request_repaint_after(Duration::from_millis(80));
        let t = ui.input(|i| i.time) as f32;
        let connected = self.devices.len();
        let live = self
            .devices
            .iter()
            .filter(|d| d.acquiring && !d.paused)
            .count();
        let busy = self.busy_device.is_some();
        let hud = hud_tokens(tokens);

        let avail = ui.available_size();
        let (rect, _resp) = ui.allocate_exact_size(avail, egui::Sense::hover());
        let painter = ui.painter_at(rect);
        paint_hud_backdrop(&painter, rect, &hud, t);

        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect.shrink(12.0))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        child.set_clip_rect(rect);

        child.horizontal(|ui| {
            ui.label(
                RichText::new(text(lang, "设备控制总览", "DIGITAL TWIN"))
                    .strong()
                    .size(18.0)
                    .color(hud.cyan),
            );
            ui.label(
                RichText::new("  /  LAB BENCH")
                    .small()
                    .monospace()
                    .color(hud.dim),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(Local::now().format("%H:%M:%S").to_string())
                        .monospace()
                        .color(hud.cyan),
                );
                hud_chip(
                    ui,
                    &hud,
                    format!("{connected} {}", text(lang, "会话", "SESSIONS")),
                    hud.cyan,
                );
                if live > 0 {
                    hud_chip(
                        ui,
                        &hud,
                        format!("{live} LIVE"),
                        hud.ok,
                    );
                }
                if busy {
                    hud_chip(ui, &hud, "BUSY", hud.warn);
                }
            });
        });
        child.add_space(8.0);
        paint_control_pipeline(&mut child, lang, &hud, t, connected, live, self.scanning);
        child.add_space(10.0);
        child.label(
            RichText::new(text(
                lang,
                "数字孪生工位：面板按真实通道/功能绘制，LCD 同步实测。点击设备进入控制台。实物图可放 device_photos/{序列号或型号}.png",
                "Digital twin bench — front panels follow real channels and live LCD readouts. Click a unit for its console. Photos: device_photos/{serial|model}.png",
            ))
            .small()
            .color(hud.dim),
        );
        child.add_space(6.0);

        let ids: Vec<u64> = self.devices.iter().map(|d| d.id).collect();
        for id in &ids {
            self.ensure_device_photo(*id, child.ctx());
        }

        let snaps: Vec<TwinSnap> = self
            .devices
            .iter()
            .map(|device| {
                let (status, status_color) = overview_runtime_status(
                    lang,
                    device,
                    self.busy_device,
                    self.busy_label.as_str(),
                );
                TwinSnap {
                    id: device.id,
                    kind: device.kind,
                    name: if device.identity.model.trim().is_empty() {
                        instrument_name(lang, device.kind).to_owned()
                    } else {
                        device.identity.model.clone()
                    },
                    manufacturer: device.identity.manufacturer.clone(),
                    serial: if device.identity.serial.trim().is_empty() {
                        short_resource(&device.resource)
                    } else {
                        format!("SN {}", device.identity.serial)
                    },
                    resource: short_resource(&device.resource),
                    status,
                    status_color,
                    params: overview_param_lines(lang, device, &self.latest),
                    activity: device.last_activity.clone(),
                    live: device.acquiring && !device.paused,
                    photo: self.device_photos.get(&device.id).cloned(),
                    face: twin_face_of(device, &self.latest, self.waveforms.get(&device.id)),
                }
            })
            .collect();

        let remain = child.available_size();
        let (scene, _) = child.allocate_exact_size(remain.max(egui::vec2(120.0, 180.0)), egui::Sense::hover());
        if let Some(id) = paint_digital_twin_lab(
            &child,
            scene,
            lang,
            tokens,
            &hud,
            t,
            &snaps,
            self.scanning,
        ) {
            if let Some(device) = self.devices.iter().find(|d| d.id == id) {
                let kind = device.kind;
                self.select_kind(kind);
                self.selected_id = Some(id);
            }
        }
    }

    fn ensure_device_photo(&mut self, id: u64, ctx: &egui::Context) {
        if self.device_photos.contains_key(&id) || self.device_photo_tried.contains(&id) {
            return;
        }
        self.device_photo_tried.insert(id);
        let Some(device) = self.devices.iter().find(|d| d.id == id) else {
            return;
        };
        let dir = self.save_dir.join("device_photos");
        let serial = sanitize_photo_stem(&device.identity.serial);
        let model = sanitize_photo_stem(&device.identity.model);
        let candidates = [
            dir.join(format!("{serial}.png")),
            dir.join(format!("{model}.png")),
            dir.join(format!("{serial}.jpg")),
            dir.join(format!("{model}.jpg")),
        ];
        for path in candidates {
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .trim();
            if stem.is_empty() {
                continue;
            }
            if path.is_file() {
                if let Ok(bytes) = std::fs::read(&path) {
                    if let Ok(image) = image::load_from_memory(&bytes) {
                        let rgba = image.into_rgba8();
                        let size = [rgba.width() as usize, rgba.height() as usize];
                        let color = egui::ColorImage::from_rgba_unmultiplied(size, &rgba);
                        let tex = ctx.load_texture(
                            format!("overview-photo-{id}"),
                            color,
                            Default::default(),
                        );
                        self.device_photos.insert(id, tex);
                        return;
                    }
                }
            }
        }
    }

    fn dispatch_scope_commands(&mut self, id: u64, commands: Vec<ControlCommand>) {
        for command in commands {
            self.log_command(id, &command);
            let _ = self.tx.send(Job::Command { id, command, job_id: None });
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
                                let _ = self.tx.send(Job::Command { id, command, job_id: None });
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
            let busy = self.busy_device == Some(id);
            if ui
                .add_enabled(
                    can_shot && !busy,
                    egui::Button::new(text(lang, "屏幕截图", "Screenshot"))
                        .fill(tokens.accent)
                        .min_size(SCOPE_BTN_WIDE),
                )
                .clicked()
            {
                self.begin_busy(
                    id,
                    text(lang, "正在截取屏幕", "Capturing screen").to_string(),
                );
                self.status = text(lang, "正在截取屏幕…", "Capturing screen…").into();
                let _ = self.tx.send(Job::Capture {
                    id,
                    job_id: None,
                    save_path: None,
                });
            }
            // Oscilloscope workbench always exposes VISA waveform-source read
            // (capability flag can be false on generic/demo profiles).
            let can_wave_src = self.devices[index].kind == InstrumentKind::Oscilloscope
                || self.devices[index].capabilities.waveform;
            let vendor = self.devices[index].identity.manufacturer.as_str();
            let wave_tip = scope_waveform_source_tip(lang, vendor);
            if ui
                .add_enabled(
                    can_wave_src && !busy,
                    egui::Button::new(text(lang, "读取波形源文件", "Read Wave Source"))
                        .min_size(egui::vec2(128.0, 28.0)),
                )
                .on_hover_text(wave_tip)
                .clicked()
            {
                self.begin_busy(
                    id,
                    text(lang, "正在读取波形源文件", "Reading waveform source").to_string(),
                );
                self.status = text(
                    lang,
                    "正在读取示波器上已打开的全部通道（不受下方通道选择影响）…",
                    "Reading all channels displayed on the scope (ignores the channel selector below)…",
                )
                .into();
                let _ = self.tx.send(Job::WaveformSource {
                    id,
                    job_id: None,
                    auto_dir: None,
                    auto_filename: None,
                    overwrite: false,
                });
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
            media_slot_empty(ui, preview_h, tokens);
        } else if let Some(texture) = self.screenshots.get(&id) {
            paint_screenshot(ui, texture, preview_h, tokens);
        } else {
            placeholder_panel(
                ui,
                preview_h,
                text(
                    lang,
                    "点击「屏幕截图」抓取仪器画面",
                    "Click Screenshot for the scope display",
                ),
                tokens.plot_fg,
                tokens,
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
                    let vendor = self.devices[index].identity.manufacturer.as_str();
                    let default_ext = if manufacturer_is(vendor, "TEKTRONIX") {
                        "isf"
                    } else {
                        "csv"
                    };
                    let default = self.save_dir.join(format!(
                        "waveform_{}_{}.{default_ext}",
                        trace.channel.replace(['/', '\\', ':'], "_"),
                        Local::now().format("%Y%m%d_%H%M%S"),
                    ));
                    let dialog = scope_waveform_save_dialog(
                        vendor,
                        &self.save_dir,
                        default.file_name().unwrap_or_default().to_string_lossy().as_ref(),
                        default_ext,
                    );
                    if let Some(path) = dialog.save_file() {
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

        if let (Some(plot), Some(trace)) = (self.wave_plots.get(&id), self.waveforms.get(&id)) {
            let stats = plot.stats;
            ui.add_space(4.0);
            // Prefer a short CHx label; Tek WFID strings are long ("Ch1, AC coupling…").
            let label = short_wave_channel_label(&trace.channel);
            let stats_line = format!(
                "{label}  points={}  Δt={}{}  min={:.4}{}  max={:.4}{}  pp={:.4}{}",
                stats.count,
                format_scope_dt(stats.dt),
                trace.x_unit,
                stats.min,
                trace.y_unit,
                stats.max,
                trace.y_unit,
                stats.pp,
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
            if trace.channel.len() > label.len() + 2 {
                ui.add(
                    egui::Label::new(
                        RichText::new(&trace.channel)
                            .small()
                            .color(tokens.text_muted),
                    )
                    .truncate(),
                );
            }
        }

        ui.add_space(6.0);
        let preview_h = ui.available_height().max(80.0);
        if defer_heavy {
            media_slot_empty(ui, preview_h, tokens);
        } else if let Some(cached) = self.wave_plots.get(&id) {
            paint_waveform(ui, &cached.columns, cached.bounds, preview_h, tokens.accent, tokens);
        } else {
            placeholder_panel(
                ui,
                preview_h,
                text(
                    lang,
                    "读取后可看曲线、统计并导出 CSV",
                    "Read to plot, stats, and export CSV",
                ),
                tokens.plot_fg,
                tokens,
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
            let _ = self.tx.send(Job::Command { id, command: scpi, job_id: None });
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

    fn probe_workspace(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        tokens: &Tokens,
        id: u64,
        index: usize,
    ) {
        egui::ScrollArea::vertical()
            .id_salt(format!("probe-workspace-{id}"))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.columns(3, |columns| {
                    card(
                        &mut columns[0],
                        tokens,
                        text(lang, "参数设置", "Parameters"),
                        |ui| {
                            self.probe_parameters_ui(ui, lang, tokens, index);
                        },
                    );
                    card(
                        &mut columns[1],
                        tokens,
                        text(lang, "控制", "Control"),
                        |ui| {
                            let commands = probe_controls(ui, lang, tokens, &mut self.devices[index]);
                            self.dispatch_scope_commands(id, commands);
                        },
                    );
                    card(
                        &mut columns[2],
                        tokens,
                        text(lang, "日志 / RTT", "Log / RTT"),
                        |ui| {
                            ui.label(
                                RichText::new(text(
                                    lang,
                                    "烧录与 RTT 会占用探针。ST-Link 若被 STM32Cube 独占，请关闭 Cube 或改用 WinUSB (Zadig)。SWO/ITM 未接上时会在此标明。",
                                    "Flash and RTT occupy the probe. If ST-Link is held by STM32Cube, close Cube or switch to WinUSB (Zadig). SWO/ITM is secondary and marked when unavailable.",
                                ))
                                .small()
                                .color(tokens.text_muted),
                            );
                            ui.add_space(6.0);
                            self.device_io_log_ui(ui, tokens, index);
                        },
                    );
                });
            });
    }

    fn bridge_workspace(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        tokens: &Tokens,
        id: u64,
        index: usize,
    ) {
        egui::ScrollArea::vertical()
            .id_salt(format!("bridge-workspace-{id}"))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.columns(3, |columns| {
                    card(
                        &mut columns[0],
                        tokens,
                        text(lang, "参数设置", "Parameters"),
                        |ui| {
                            self.bridge_parameters_ui(ui, lang, tokens, index);
                        },
                    );
                    card(
                        &mut columns[1],
                        tokens,
                        text(lang, "控制", "Control"),
                        |ui| {
                            let commands =
                                bridge_controls(ui, lang, tokens, &mut self.devices[index]);
                            self.dispatch_scope_commands(id, commands);
                        },
                    );
                    card(
                        &mut columns[2],
                        tokens,
                        text(lang, "日志", "Log"),
                        |ui| {
                            ui.label(
                                RichText::new(text(
                                    lang,
                                    "识别不需要 DLL。SPI/I2C/GPIO 会从 WiParse 目录、vendor/ftdi 或 FTDI 安装路径加载 LibFT4222.dll。J-Link/ST-Link/DAP 无需厂商 DLL。",
                                    "Scan needs no DLL. SPI/I2C/GPIO load LibFT4222 from the WiParse folder, vendor/ftdi, or an FTDI install. J-Link/ST-Link/DAP need no vendor DLL.",
                                ))
                                .small()
                                .color(tokens.text_muted),
                            );
                            ui.add_space(6.0);
                            self.device_io_log_ui(ui, tokens, index);
                        },
                    );
                });
            });
    }

    fn probe_parameters_ui(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        tokens: &Tokens,
        index: usize,
    ) {
        let device = &mut self.devices[index];
        egui::Grid::new(("probe-id-grid", device.id))
            .num_columns(2)
            .spacing([10.0, 4.0])
            .show(ui, |ui| {
                setting_row(ui, text(lang, "资源", "Resource"), &device.resource);
                setting_row(
                    ui,
                    text(lang, "制造商", "Manufacturer"),
                    &device.identity.manufacturer,
                );
                setting_row(ui, text(lang, "型号", "Model"), &device.identity.model);
                setting_row(ui, text(lang, "序列号", "Serial"), &device.identity.serial);
            });
        ui.add_space(8.0);
        ui.label(RichText::new(text(lang, "目标芯片", "Target chip")).strong());
        ui.add(
            egui::TextEdit::singleline(&mut device.controls.probe_chip)
                .hint_text("STM32F103C8 / empty = DP only")
                .desired_width(ui.available_width()),
        );
        ui.label(
            RichText::new(text(
                lang,
                "留空则只连接 DP，不能烧录。",
                "Leave empty to attach DP only (no flash).",
            ))
            .small()
            .color(tokens.text_muted),
        );
        ui.add_space(8.0);
        ui.label(RichText::new(text(lang, "烧录文件", "Flash file")).strong());
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut device.controls.probe_flash_path)
                    .desired_width(160.0)
                    .hint_text("HEX / BIN / ELF"),
            );
            if ui.button(text(lang, "浏览", "Browse")).clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Firmware", &["hex", "ihex", "bin", "elf"])
                    .pick_file()
                {
                    device.controls.probe_flash_path = path.display().to_string();
                }
            }
        });
        ui.checkbox(
            &mut device.controls.probe_flash_verify,
            text(lang, "校验", "Verify"),
        );
        ui.add_space(8.0);
        ui.label(RichText::new(text(lang, "内存", "Memory")).strong());
        ui.horizontal(|ui| {
            ui.label(text(lang, "地址", "Address"));
            ui.add(
                egui::TextEdit::singleline(&mut device.controls.probe_mem_addr)
                    .desired_width(100.0)
                    .hint_text("08000000"),
            );
            ui.label(text(lang, "长度", "Len"));
            ui.add(
                egui::DragValue::new(&mut device.controls.probe_mem_len)
                    .range(1..=4096)
                    .suffix(" B"),
            );
        });
        ui.add(
            egui::TextEdit::singleline(&mut device.controls.probe_mem_write_hex)
                .hint_text("write hex, e.g. DE AD BE EF")
                .desired_width(ui.available_width()),
        );
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(text(lang, "RTT 通道", "RTT channel"));
            ui.add(egui::DragValue::new(&mut device.controls.probe_rtt_channel).range(0..=15));
        });
        ui.horizontal(|ui| {
            ui.label(text(lang, "SWD 速率", "SWD kHz"));
            ui.add(
                egui::DragValue::new(&mut device.controls.probe_speed_khz)
                    .range(10..=50_000)
                    .suffix(" kHz")
                    .speed(100.0),
            );
        });
    }

    fn bridge_parameters_ui(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        tokens: &Tokens,
        index: usize,
    ) {
        let device = &self.devices[index];
        egui::Grid::new(("bridge-id-grid", device.id))
            .num_columns(2)
            .spacing([10.0, 4.0])
            .show(ui, |ui| {
                setting_row(ui, text(lang, "资源", "Resource"), &device.resource);
                setting_row(ui, text(lang, "型号", "Model"), &device.identity.model);
                setting_row(ui, text(lang, "序列号", "Serial"), &device.identity.serial);
            });
        ui.add_space(8.0);
        let tab = &mut self.devices[index].controls.bridge_tab;
        ui.horizontal(|ui| {
            ui.selectable_value(tab, 0, "SPI");
            ui.selectable_value(tab, 1, "I2C");
            ui.selectable_value(tab, 2, "GPIO");
        });
        ui.add_space(6.0);
        match self.devices[index].controls.bridge_tab {
            1 => {
                ui.horizontal(|ui| {
                    ui.label(text(lang, "7-bit 地址", "7-bit addr"));
                    ui.add(
                        egui::TextEdit::singleline(
                            &mut self.devices[index].controls.bridge_i2c_addr,
                        )
                        .desired_width(60.0)
                        .hint_text("50"),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label(text(lang, "I2C 时钟", "I2C clock"));
                    ui.add(
                        egui::DragValue::new(&mut self.devices[index].controls.bridge_i2c_hz)
                            .range(60_000..=3_400_000)
                            .suffix(" Hz")
                            .speed(1000.0),
                    );
                });
                ui.label(text(lang, "写 hex", "Write hex"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.devices[index].controls.bridge_write_hex)
                        .desired_width(ui.available_width()),
                );
                ui.horizontal(|ui| {
                    ui.label(text(lang, "读字节", "Read len"));
                    ui.add(
                        egui::DragValue::new(&mut self.devices[index].controls.bridge_read_len)
                            .range(0..=256),
                    );
                });
            }
            2 => {
                ui.horizontal(|ui| {
                    ui.label(text(lang, "引脚", "Pin"));
                    ui.add(
                        egui::DragValue::new(&mut self.devices[index].controls.bridge_gpio_pin)
                            .range(0..=3),
                    );
                });
                ui.checkbox(
                    &mut self.devices[index].controls.bridge_gpio_out,
                    text(lang, "输出", "Output"),
                );
                ui.checkbox(
                    &mut self.devices[index].controls.bridge_gpio_value,
                    text(lang, "高电平", "High"),
                );
            }
            _ => {
                ui.horizontal(|ui| {
                    ui.label("SPI mode");
                    ui.add(
                        egui::DragValue::new(&mut self.devices[index].controls.bridge_spi_mode)
                            .range(0..=3),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label(text(lang, "时钟", "Clock"));
                    ui.add(
                        egui::DragValue::new(&mut self.devices[index].controls.bridge_spi_hz)
                            .range(1_000..=40_000_000)
                            .suffix(" Hz")
                            .speed(10_000.0),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("CS");
                    ui.add(
                        egui::DragValue::new(&mut self.devices[index].controls.bridge_spi_cs)
                            .range(0..=3),
                    );
                });
                ui.label(text(lang, "写 hex", "Write hex"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.devices[index].controls.bridge_write_hex)
                        .desired_width(ui.available_width()),
                );
                ui.horizontal(|ui| {
                    ui.label(text(lang, "读字节", "Read len"));
                    ui.add(
                        egui::DragValue::new(&mut self.devices[index].controls.bridge_read_len)
                            .range(0..=256),
                    );
                });
            }
        }
        ui.add_space(8.0);
        ui.label(
            RichText::new(text(
                lang,
                "无 DLL 时仍可识别；把 LibFT4222.dll 与 ftd2xx.dll 放到 WiParse.exe 旁或 vendor/ftdi。",
                "Identify works without DLLs; drop LibFT4222.dll and ftd2xx.dll next to WiParse.exe or in vendor/ftdi.",
            ))
            .small()
            .color(tokens.text_muted),
        );
    }

    fn device_io_log_ui(&mut self, ui: &mut egui::Ui, tokens: &Tokens, index: usize) {
        let log = self.devices[index].controls.io_log.clone();
        egui::ScrollArea::vertical()
            .id_salt(("device-io-log", self.devices[index].id))
            .stick_to_bottom(true)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                if log.is_empty() {
                    ui.label(
                        RichText::new("—")
                            .monospace()
                            .small()
                            .color(tokens.text_muted),
                    );
                } else {
                    ui.label(
                        RichText::new(&log)
                            .monospace()
                            .small()
                            .color(tokens.text_muted),
                    );
                }
            });
        if ui.button("Clear").clicked() {
            self.devices[index].controls.io_log.clear();
        }
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
                tokens.plot_fg,
                tokens,
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
                "当前类型尚未连接。左侧扫描或输入地址后即可启用全部功能。",
                "No instrument of this type is connected. Scan or enter a resource on the left to enable all functions.",
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
                    ui.label(text(lang, "测量历史点数", "Measurement history points"));
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
            InstrumentKind::DebugProbe | InstrumentKind::UsbBridge => {}
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
                    let mut line = Line::new(PlotPoints::from(points)).name(name.clone());
                    if let Some(color) = crate::waveform_analysis::tek_channel_color(&name) {
                        line = line.color(color);
                    }
                    plot_ui.line(line);
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
            let _ = self.tx.send(Job::Command { id, command, job_id: None });
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
            let _ = self.tx.send(Job::Command { id, command: scpi, job_id: None });
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
    let mut devices = HashMap::<u64, LiveSession>::new();
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
                        job_id: None,
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
            } => match open_live_session(
                resource.clone(),
                kind,
                timeout_ms,
                (!library.trim().is_empty()).then_some(library.as_str()),
            ) {
                Ok(session) => {
                    let event = connected_event(id, &session);
                    devices.insert(id, session);
                    let _ = events.send(event);
                }
                Err(error) => send_error(&events, Some(id), error),
            },
            Job::ConnectDemo { id, kind } => match open_demo_session(kind) {
                Ok(session) => {
                    let event = connected_event(id, &session);
                    devices.insert(id, session);
                    let _ = events.send(event);
                }
                Err(error) => send_error(&events, Some(id), error),
            },
            Job::Disconnect(id) => {
                devices.remove(&id);
                let _ = events.send(Event::Disconnected(id));
            }
            Job::Command { id, command, job_id } => {
                if let Some(device) = devices.get_mut(&id) {
                    match device.execute(command) {
                        Ok(response) => {
                            let _ = events.send(Event::CommandDone {
                                id,
                                job_id,
                                response,
                            });
                        }
                        Err(error) => send_error_job(&events, Some(id), job_id, error),
                    }
                } else {
                    send_error_job(&events, Some(id), job_id, "device not connected");
                }
            }
            Job::PollRtt(id) => {
                if let Some(LiveSession::Probe(probe)) = devices.get_mut(&id) {
                    match probe.execute(ControlCommand::ProbeRttRead) {
                        Ok(text) => {
                            let _ = events.send(Event::SessionOutput { id, text });
                        }
                        Err(error) => send_error(&events, Some(id), error),
                    }
                } else {
                    let _ = events.send(Event::SessionOutput { id, text: None });
                }
            }
            Job::Measure(id) => {
                match devices.get_mut(&id) {
                    Some(LiveSession::Scpi(device)) => match device.read_measurements() {
                        Ok(readings) => {
                            let _ = events.send(Event::Measurements {
                                id,
                                resource: device.resource.clone(),
                                readings,
                            });
                        }
                        Err(error) => send_error(&events, Some(id), error),
                    },
                    Some(_) => {
                        let _ = events.send(Event::Measurements {
                            id,
                            resource: String::new(),
                            readings: Vec::new(),
                        });
                    }
                    None => send_error(&events, Some(id), "device not connected"),
                }
            }
            Job::Capture {
                id,
                job_id,
                save_path,
            } => {
                let _ = events.send(Event::Progress {
                    id: Some(id),
                    message: "正在截取屏幕… / Capturing screen…".into(),
                });
                if let Some(LiveSession::Scpi(device)) = devices.get_mut(&id) {
                    match device.capture_scope_png() {
                        Ok(png) => {
                            let _ = events.send(Event::Progress {
                                id: Some(id),
                                message: format!(
                                    "截图已收到 {} KB，正在解码… / Screenshot {} KB, decoding…",
                                    png.len() / 1024,
                                    png.len() / 1024
                                ),
                            });
                            match prepare_screenshot_preview(&png, SCOPE_PREVIEW_MAX_EDGE) {
                                Ok((width, height, rgba)) => {
                                    let _ = events.send(Event::Screenshot {
                                        id,
                                        job_id,
                                        save_path,
                                        width,
                                        height,
                                        rgba,
                                        png,
                                    });
                                }
                                Err(error) => send_error_job(
                                    &events,
                                    Some(id),
                                    job_id,
                                    format!("Invalid screenshot: {error}"),
                                ),
                            }
                        }
                        Err(error) => send_error_job(&events, Some(id), job_id, error),
                    }
                } else {
                    send_error_job(&events, Some(id), job_id, "device not connected");
                }
            }
            Job::WaveformSource {
                id,
                job_id,
                auto_dir,
                auto_filename,
                overwrite,
            } => {
                let _ = events.send(Event::Progress {
                    id: Some(id),
                    message: "正在查询已打开通道并读取波形（采样率×屏幕时宽）… / Querying displayed channels, reading acquisition-density screen window…"
                        .into(),
                });
                if let Some(LiveSession::Scpi(device)) = devices.get_mut(&id) {
                    match device.capture_scope_waveform_sources_displayed() {
                        Ok(parts) => {
                            let mut channel_files = Vec::new();
                            let mut errors = Vec::new();
                            let mut total_bytes = 0usize;
                            for (ch, bytes, suggested_name) in parts {
                                total_bytes += bytes.len();
                                let _ = events.send(Event::Progress {
                                    id: Some(id),
                                    message: format!(
                                        "已读 CH{ch}（{} KB），继续… / Got CH{ch} ({} KB), continuing…",
                                        bytes.len() / 1024,
                                        bytes.len() / 1024
                                    ),
                                });
                                let stem = std::path::PathBuf::from(&suggested_name);
                                let ext = stem
                                    .extension()
                                    .and_then(|e| e.to_str())
                                    .unwrap_or("isf")
                                    .to_ascii_lowercase();
                                channel_files.push((ch, bytes, ext));
                            }
                            if channel_files.is_empty() {
                                send_error_job(
                                    &events,
                                    Some(id),
                                    job_id,
                                    "no displayed channel waveform data",
                                );
                            } else {
                                let ch_tag = channel_files
                                    .iter()
                                    .map(|(ch, _, _)| format!("CH{ch}"))
                                    .collect::<Vec<_>>()
                                    .join("_");
                                let all_isf = channel_files.len() > 1
                                    && channel_files.iter().all(|(_, _, e)| e == "isf");
                                let (ch0, raw0, ext0) = &channel_files[0];
                                let (bytes, suggested_name) = if channel_files.len() == 1 {
                                    (raw0.clone(), format!("waveform_CH{ch0}.{ext0}"))
                                } else if all_isf {
                                    let parts: Vec<(u8, &[u8])> = channel_files
                                        .iter()
                                        .map(|(ch, b, _)| (*ch, b.as_slice()))
                                        .collect();
                                    (
                                        join_tek_isf_channels(&parts),
                                        format!("waveform_{ch_tag}.isf"),
                                    )
                                } else {
                                    (Vec::new(), format!("waveform_{ch_tag}.csv"))
                                };

                                let traces = if channel_files.len() == 1 {
                                    match load_waveform_bytes_all(raw0, ext0, &format!("CH{ch0}"))
                                        .or_else(|first| match sniff_waveform_ext(raw0) {
                                            Some(sniffed) if sniffed != *ext0 => {
                                                load_waveform_bytes_all(
                                                    raw0,
                                                    sniffed,
                                                    &format!("CH{ch0}"),
                                                )
                                            }
                                            _ => Err(first),
                                        }) {
                                        Ok(t) => t,
                                        Err(e) => {
                                            errors.push(format!("CH{ch0}: {e}"));
                                            Vec::new()
                                        }
                                    }
                                } else if all_isf {
                                    match load_waveform_bytes_all(&bytes, "isf", "CH1") {
                                        Ok(t) => t,
                                        Err(e) => {
                                            errors.push(e.to_string());
                                            Vec::new()
                                        }
                                    }
                                } else {
                                    let mut t = Vec::new();
                                    for (ch, b, e) in &channel_files {
                                        match load_waveform_bytes(b, e, &format!("CH{ch}"))
                                            .or_else(|first| match sniff_waveform_ext(b) {
                                                Some(sniffed) if sniffed != *e => {
                                                    load_waveform_bytes(
                                                        b,
                                                        sniffed,
                                                        &format!("CH{ch}"),
                                                    )
                                                }
                                                _ => Err(first),
                                            }) {
                                            Ok(tr) => t.push(tr),
                                            Err(e) => errors.push(format!("CH{ch}: {e}")),
                                        }
                                    }
                                    t
                                };

                                let (bytes, suggested_name) =
                                    if channel_files.len() > 1 && !all_isf && !traces.is_empty() {
                                        (
                                            waveforms_to_spreadsheet_csv(&traces),
                                            format!("waveform_{ch_tag}.csv"),
                                        )
                                    } else {
                                        (bytes, suggested_name)
                                    };
                                let parse_error = if traces.is_empty() {
                                    Some(errors.join("; "))
                                } else if !errors.is_empty() {
                                    Some(format!("partial: {}", errors.join("; ")))
                                } else {
                                    None
                                };
                                // Do not clone CH1 into `trace` — UI uses `traces[0]`.
                                // Drop per-channel raw blobs when we already built a
                                // single joined/CSV payload (keeps Save As working).
                                let keep_parts = channel_files.len() <= 1;
                                let _ = events.send(Event::WaveformSource {
                                    id,
                                    job_id,
                                    auto_dir,
                                    auto_filename,
                                    overwrite,
                                    bytes,
                                    suggested_name,
                                    trace: None,
                                    traces,
                                    channel_files: if keep_parts {
                                        channel_files
                                    } else {
                                        Vec::new()
                                    },
                                    parse_error,
                                });
                                let _ = total_bytes;
                            }
                        }
                        Err(error) => send_error_job(&events, Some(id), job_id, error),
                    }
                } else {
                    send_error_job(&events, Some(id), job_id, "device not connected");
                }
            }
            Job::Waveform {
                id,
                channel,
                points: _,
            } => {
                if let Some(LiveSession::Scpi(device)) = devices.get_mut(&id) {
                    match device.read_scope_source_waveform(channel) {
                        Ok(trace) => {
                            let _ = events.send(Event::Waveform { id, trace });
                        }
                        Err(error) => send_error(&events, Some(id), error),
                    }
                } else {
                    send_error(&events, Some(id), "device not connected");
                }
            }
            Job::Shutdown => break,
        }
    }
}

enum LiveSession {
    Scpi(InstrumentDevice),
    Probe(ProbeSession),
    Bridge(Ft4222Session),
}

impl LiveSession {
    fn execute(
        &mut self,
        command: ControlCommand,
    ) -> Result<Option<String>, wiparse_core::instrument::InstrumentError> {
        match self {
            Self::Scpi(device) => device.execute(command),
            Self::Probe(device) => device.execute(command),
            Self::Bridge(device) => device.execute(command),
        }
    }
}

fn open_demo_session(kind: InstrumentKind) -> Result<LiveSession, wiparse_core::instrument::InstrumentError> {
    match kind {
        InstrumentKind::DebugProbe => Ok(LiveSession::Probe(ProbeSession::demo(
            "probe://jlink/serial=DEMO",
        ))),
        InstrumentKind::UsbBridge => Ok(LiveSession::Bridge(Ft4222Session::demo(
            "bridge://ft4222/serial=DEMO",
        ))),
        other => InstrumentDevice::connect_demo(other).map(LiveSession::Scpi),
    }
}

fn open_live_session(
    resource: String,
    kind: Option<InstrumentKind>,
    timeout_ms: u32,
    library: Option<&str>,
) -> Result<LiveSession, wiparse_core::instrument::InstrumentError> {
    if resource.starts_with("DEMO::") {
        return open_demo_session(kind.unwrap_or(InstrumentKind::Generic));
    }
    if let Some(spec) = parse_usb_resource(&resource) {
        return match spec.iface {
            UsbIface::Ft4222 => Ft4222Session::open(&resource).map(LiveSession::Bridge),
            _ => ProbeSession::open(&resource, None).map(LiveSession::Probe),
        };
    }
    match kind {
        Some(InstrumentKind::DebugProbe) => {
            ProbeSession::open(&resource, None).map(LiveSession::Probe)
        }
        Some(InstrumentKind::UsbBridge) => {
            Ft4222Session::open(&resource).map(LiveSession::Bridge)
        }
        requested => InstrumentDevice::connect_with_library(resource, timeout_ms, requested, library)
            .map(LiveSession::Scpi),
    }
}

fn connected_event(id: u64, session: &LiveSession) -> Event {
    match session {
        LiveSession::Scpi(device) => Event::Connected {
            id,
            resource: device.resource.clone(),
            identity: device.identity.clone(),
            kind: device.profile.kind,
            profile: device.profile.name.clone(),
            capabilities: device.profile.capabilities.clone(),
        },
        LiveSession::Probe(device) => Event::Connected {
            id,
            resource: device.resource.clone(),
            identity: device.identity.clone(),
            kind: device.kind,
            profile: format!("{} {}", device.identity.manufacturer, device.identity.model),
            capabilities: Capabilities::default(),
        },
        LiveSession::Bridge(device) => Event::Connected {
            id,
            resource: device.resource.clone(),
            identity: device.identity.clone(),
            kind: device.kind,
            profile: format!("{} {}", device.identity.manufacturer, device.identity.model),
            capabilities: Capabilities::default(),
        },
    }
}

fn send_error(events: &Sender<Event>, id: Option<u64>, error: impl std::fmt::Display) {
    send_error_job(events, id, None, error);
}

fn send_error_job(
    events: &Sender<Event>,
    id: Option<u64>,
    job_id: Option<u64>,
    error: impl std::fmt::Display,
) {
    let _ = events.send(Event::Error {
        id,
        job_id,
        message: error.to_string(),
    });
}

/// Save multi-channel sources into **one** file when possible:
/// - `.isf` → concatenated multi-curve Tek ISF
/// - `.csv` / `.txt` → `TIME,CH1,CH2,…`
/// - `.wfm` with Tek ISF natives → still one `.isf` (WFM has no simple multi join)
fn save_pending_waveform_source(
    path: &std::path::Path,
    pending: &PendingWaveformSave,
) -> Result<Vec<PathBuf>, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or(&pending.effective_ext)
        .to_ascii_lowercase();

    let first_trace = pending
        .parsed_trace
        .as_ref()
        .or_else(|| pending.traces.first());

    if pending.channel_files.len() <= 1 {
        save_waveform_file(
            path,
            Some(&pending.bytes),
            Some(&pending.effective_ext),
            first_trace,
        )
        .map_err(|e| e.to_string())?;
        return Ok(vec![path.to_path_buf()]);
    }

    if ext == "csv" || ext == "txt" {
        let csv = if pending.traces.len() > 1 {
            waveforms_to_spreadsheet_csv(&pending.traces)
        } else {
            pending.bytes.clone()
        };
        std::fs::write(path, csv).map_err(|e| e.to_string())?;
        return Ok(vec![path.to_path_buf()]);
    }

    // Multi-channel: prefer already-joined `bytes` (ISF/CSV built on the worker).
    let all_isf = pending.effective_ext == "isf"
        || pending
            .channel_files
            .iter()
            .all(|(_, _, e)| e == "isf");
    if all_isf && (ext == "isf" || ext == "wfm") {
        let out = if ext == "isf" {
            path.to_path_buf()
        } else {
            // Multi-curve WFM packaging is not supported — keep one ISF file.
            path.with_extension("isf")
        };
        if !pending.bytes.is_empty() {
            std::fs::write(&out, &pending.bytes).map_err(|e| e.to_string())?;
        } else if !pending.channel_files.is_empty() {
            let parts: Vec<(u8, &[u8])> = pending
                .channel_files
                .iter()
                .map(|(ch, b, _)| (*ch, b.as_slice()))
                .collect();
            let joined = join_tek_isf_channels(&parts);
            std::fs::write(&out, joined).map_err(|e| e.to_string())?;
        } else {
            return Err("no multi-channel ISF data to save".into());
        }
        return Ok(vec![out]);
    }

    // Other native formats: one multi-column CSV at the chosen stem.
    let out = path.with_extension("csv");
    let csv = waveforms_to_spreadsheet_csv(&pending.traces);
    std::fs::write(&out, csv).map_err(|e| e.to_string())?;
    Ok(vec![out])
}

#[derive(Clone, Copy)]
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

fn short_wave_channel_label(channel: &str) -> String {
    let head = channel.split(',').next().unwrap_or(channel).trim();
    let upper = head.to_ascii_uppercase().replace(' ', "");
    if upper.starts_with("CH") && upper.len() <= 6 {
        upper
    } else if let Some(rest) = channel.to_ascii_uppercase().split("CH").nth(1) {
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !digits.is_empty() {
            format!("CH{digits}")
        } else {
            head.chars().take(24).collect()
        }
    } else {
        head.chars().take(24).collect()
    }
}

fn format_scope_dt(dt: f64) -> String {
    if !dt.is_finite() || dt == 0.0 {
        return "0".into();
    }
    let a = dt.abs();
    if a >= 1.0 {
        format!("{dt:.3}")
    } else if a >= 1e-3 {
        format!("{:.3}m", dt * 1e3)
    } else if a >= 1e-6 {
        format!("{:.3}µ", dt * 1e6)
    } else if a >= 1e-9 {
        format!("{:.3}n", dt * 1e9)
    } else {
        format!("{dt:.3e}")
    }
}

fn build_cached_wave_plot(trace: &WaveformTrace) -> CachedWavePlot {
    let columns = build_overview_envelope(&trace.x, &trace.y);
    let bounds = envelope_bounds(columns.as_slice());
    CachedWavePlot {
        channel: trace.channel.clone(),
        columns,
        bounds,
        stats: waveform_stats(trace),
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
        .stroke(Stroke::new(1.0_f32, tokens.divider))
        .corner_radius(CornerRadius::same(theme::RADIUS_CARD))
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
        Stroke::new(STROKE_W, tokens.divider),
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

fn media_slot_empty(ui: &mut egui::Ui, height: f32, tokens: &Tokens) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), height), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::same(theme::RADIUS_CTRL), tokens.plot_bg);
    painter.rect_stroke(
        rect,
        CornerRadius::same(theme::RADIUS_CTRL),
        Stroke::new(1.0_f32, tokens.plot_border),
        egui::StrokeKind::Inside,
    );
}


fn paint_waveform(
    ui: &mut egui::Ui,
    columns: &[ScopeEnvelopeColumn],
    bounds: (f64, f64, f64, f64),
    height: f32,
    color: Color32,
    tokens: &Tokens,
) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::same(theme::RADIUS_CTRL), tokens.plot_bg);
    painter.rect_stroke(
        rect,
        CornerRadius::same(theme::RADIUS_CTRL),
        Stroke::new(1.0_f32, tokens.plot_border),
        egui::StrokeKind::Inside,
    );
    if columns.is_empty() {
        return;
    }
    let (min_x, max_x, min_y, max_y) = bounds;
    let dx = (max_x - min_x).max(1e-12);
    let dy = (max_y - min_y).max(1e-12);
    let pad = 6.0;
    let inner = rect.shrink(pad);
    let stroke = Stroke::new(1.2_f32, color);
    let map = |p: [f64; 2]| {
        let x = inner.left() + ((p[0] - min_x) / dx) as f32 * inner.width();
        let y = inner.bottom() - ((p[1] - min_y) / dy) as f32 * inner.height();
        egui::pos2(x, y)
    };
    paint_envelope_columns(&painter, columns, map, inner, stroke);
}

fn paint_screenshot(ui: &mut egui::Ui, texture: &egui::TextureHandle, height: f32, tokens: &Tokens) {
    let width = ui.available_width().max(1.0);
    let height = height.max(1.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::same(theme::RADIUS_CTRL), tokens.plot_bg);
    painter.rect_stroke(
        rect,
        CornerRadius::same(theme::RADIUS_CTRL),
        Stroke::new(1.0_f32, tokens.plot_border),
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

fn placeholder_panel(ui: &mut egui::Ui, height: f32, message: &str, color: Color32, tokens: &Tokens) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), height), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::same(theme::RADIUS_CTRL), tokens.plot_bg);
    painter.rect_stroke(
        rect,
        CornerRadius::same(theme::RADIUS_CTRL),
        Stroke::new(1.0_f32, tokens.plot_border),
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
        InstrumentKind::DebugProbe => {
            commands.extend(probe_controls(ui, lang, tokens, device));
        }
        InstrumentKind::UsbBridge => {
            commands.extend(bridge_controls(ui, lang, tokens, device));
        }
    }
    (commands, measure_once)
}

fn probe_controls(
    ui: &mut egui::Ui,
    lang: Lang,
    tokens: &Tokens,
    device: &mut DeviceUi,
) -> Vec<ControlCommand> {
    let mut out = Vec::new();
    ui.horizontal_wrapped(|ui| {
        if theme::accent_button(ui, tokens, text(lang, "连接目标", "Attach")).clicked() {
            out.push(ControlCommand::ProbeAttach {
                target: device.controls.probe_chip.clone(),
            });
        }
        if ui.button(text(lang, "Halt", "Halt")).clicked() {
            out.push(ControlCommand::ProbeHalt);
        }
        if ui.button(text(lang, "Run", "Run")).clicked() {
            out.push(ControlCommand::ProbeRun);
        }
        if ui.button(text(lang, "复位", "Reset")).clicked() {
            out.push(ControlCommand::ProbeReset { hardware: false });
        }
        if ui.button(text(lang, "硬件复位", "HW Reset")).clicked() {
            out.push(ControlCommand::ProbeReset { hardware: true });
        }
        if ui.button(text(lang, "状态", "Status")).clicked() {
            out.push(ControlCommand::ProbeStatus);
        }
        if ui.button(text(lang, "寄存器", "Regs")).clicked() {
            out.push(ControlCommand::ProbeRegs);
        }
        if ui.button(text(lang, "擦除", "Erase")).clicked() {
            out.push(ControlCommand::ProbeErase);
        }
        if ui.button(text(lang, "速率", "Speed")).clicked() {
            out.push(ControlCommand::ProbeSpeed {
                khz: device.controls.probe_speed_khz,
            });
        }
    });
    ui.add_space(8.0);
    ui.horizontal_wrapped(|ui| {
        if ui.button(text(lang, "烧录", "Flash")).clicked() {
            out.push(ControlCommand::ProbeFlash {
                path: device.controls.probe_flash_path.clone(),
                verify: device.controls.probe_flash_verify,
                base_address: None,
            });
        }
        if ui.button(text(lang, "读内存", "Read mem")).clicked() {
            out.push(ControlCommand::ProbeMemRead {
                address: device.controls.probe_mem_addr.clone(),
                len: device.controls.probe_mem_len,
            });
        }
        if ui.button(text(lang, "写内存", "Write mem")).clicked() {
            out.push(ControlCommand::ProbeMemWrite {
                address: device.controls.probe_mem_addr.clone(),
                data_hex: device.controls.probe_mem_write_hex.clone(),
            });
        }
    });
    ui.add_space(8.0);
    let mut rtt_on = device.controls.probe_rtt_on;
    if ui
        .checkbox(&mut rtt_on, text(lang, "RTT 输出", "RTT output"))
        .changed()
    {
        device.controls.probe_rtt_on = rtt_on;
        if rtt_on {
            out.push(ControlCommand::ProbeRttStart {
                up_channel: device.controls.probe_rtt_channel,
            });
        } else {
            out.push(ControlCommand::ProbeRttStop);
        }
    }
    ui.label(
        RichText::new(text(
            lang,
            "SWO/ITM 为次优先；当前构建以 RTT 为主。",
            "SWO/ITM is secondary; this build uses RTT first.",
        ))
        .small()
        .color(tokens.text_muted),
    );
    out
}

fn bridge_controls(
    ui: &mut egui::Ui,
    lang: Lang,
    _tokens: &Tokens,
    device: &mut DeviceUi,
) -> Vec<ControlCommand> {
    let mut out = Vec::new();
    match device.controls.bridge_tab {
        1 => {
            if ui.button(text(lang, "I2C 事务", "I2C xfer")).clicked() {
                out.push(ControlCommand::BridgeI2c {
                    addr: device.controls.bridge_i2c_addr.clone(),
                    write_hex: device.controls.bridge_write_hex.clone(),
                    read_len: device.controls.bridge_read_len,
                    clock_hz: device.controls.bridge_i2c_hz,
                });
            }
            if ui.button(text(lang, "芯片信息", "Chip info")).clicked() {
                out.push(ControlCommand::BridgeInfo);
            }
        }
        2 => {
            if ui.button(text(lang, "应用 GPIO", "Apply GPIO")).clicked() {
                out.push(ControlCommand::BridgeGpio {
                    pin: device.controls.bridge_gpio_pin,
                    dir: Some(device.controls.bridge_gpio_out),
                    value: Some(device.controls.bridge_gpio_value),
                });
            }
            if ui.button(text(lang, "读全部", "Read all")).clicked() {
                out.push(ControlCommand::BridgeGpio {
                    pin: 255,
                    dir: None,
                    value: None,
                });
            }
        }
        _ => {
            if ui.button(text(lang, "SPI 事务", "SPI xfer")).clicked() {
                out.push(ControlCommand::BridgeSpi {
                    mode: device.controls.bridge_spi_mode,
                    clock_hz: device.controls.bridge_spi_hz,
                    cs: device.controls.bridge_spi_cs,
                    write_hex: device.controls.bridge_write_hex.clone(),
                    read_len: device.controls.bridge_read_len,
                });
            }
            if ui.button(text(lang, "芯片信息", "Chip info")).clicked() {
                out.push(ControlCommand::BridgeInfo);
            }
        }
    }
    out
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
                    resource_empty_prompt(lang, kind).to_owned()
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
                        .hint_text(resource_address_hint(kind))
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

fn manufacturer_is(manufacturer: &str, needle: &str) -> bool {
    manufacturer.to_uppercase().contains(needle)
}

fn scope_waveform_source_tip(lang: Lang, manufacturer: &str) -> String {
    if manufacturer_is(manufacturer, "TEKTRONIX") {
        text(
            lang,
            "读取已打开通道的屏幕时间窗波形（采样全密度，软件不砍点数，上限=示波器 Record Length；多通道同一 ISF）",
            "Read displayed channels over the screen time span (full sample density; no software point cap, limited by scope record length; multi-ch → one ISF)",
        )
        .into()
    } else if manufacturer_is(manufacturer, "RIGOL") || manufacturer_is(manufacturer, "SIGLENT") {
        text(
            lang,
            "读取已打开的全部通道屏幕波形→CSV（不受下方通道选择影响）；另存可转泰克 ISF/WFM",
            "Read all displayed channels via screen export→CSV (ignores selector); Save As→Tek ISF/WFM",
        )
        .into()
    } else if manufacturer_is(manufacturer, "KEYSIGHT") || manufacturer_is(manufacturer, "AGILENT")
    {
        text(
            lang,
            "读取已打开的全部通道屏幕波形→CSV（不受下方通道选择影响）；另存可转泰克 ISF/WFM",
            "Read all displayed channels→CSV (ignores selector); Save As→Tek ISF/WFM",
        )
        .into()
    } else {
        text(
            lang,
            "自动尝试 Keysight/Rigol/Tek 命令读取屏幕波形；另存可转换格式",
            "Auto-try Keysight/Rigol/Tek screen waveform; Save As converts formats",
        )
        .into()
    }
}

/// Vendor-aware Save As filters. Native formats listed first.
fn scope_waveform_save_dialog(
    manufacturer: &str,
    dir: &std::path::Path,
    file_name: &str,
    preferred_ext: &str,
) -> rfd::FileDialog {
    let mut dialog = rfd::FileDialog::new()
        .set_directory(dir)
        .set_file_name(file_name);
    if manufacturer_is(manufacturer, "TEKTRONIX") {
        dialog = match preferred_ext {
            "wfm" => dialog
                .add_filter("Tektronix WFM", &["wfm"])
                .add_filter("Tektronix ISF", &["isf"])
                .add_filter("CSV", &["csv"]),
            "csv" => dialog
                .add_filter("CSV", &["csv"])
                .add_filter("Tektronix ISF", &["isf"])
                .add_filter("Tektronix WFM", &["wfm"]),
            _ => dialog
                .add_filter("Tektronix ISF", &["isf"])
                .add_filter("Tektronix WFM", &["wfm"])
                .add_filter("CSV", &["csv"]),
        };
    } else if manufacturer_is(manufacturer, "RIGOL") || manufacturer_is(manufacturer, "SIGLENT") {
        dialog = match preferred_ext {
            "wfm" => dialog
                .add_filter("Rigol WFM", &["wfm"])
                .add_filter("CSV", &["csv"])
                .add_filter("Tektronix ISF (converted)", &["isf"])
                .add_filter("Tektronix WFM (converted)", &["wfm"]),
            _ => dialog
                .add_filter("CSV (native)", &["csv"])
                .add_filter("Tektronix ISF (converted)", &["isf"])
                .add_filter("Tektronix WFM (converted)", &["wfm"]),
        };
    } else if manufacturer_is(manufacturer, "KEYSIGHT") || manufacturer_is(manufacturer, "AGILENT")
    {
        dialog = dialog
            .add_filter("CSV", &["csv"])
            .add_filter("Tektronix ISF (converted)", &["isf"])
            .add_filter("Tektronix WFM (converted)", &["wfm"]);
    } else {
        dialog = dialog
            .add_filter("CSV", &["csv"])
            .add_filter("Tektronix ISF (converted)", &["isf"])
            .add_filter("Tektronix WFM (converted)", &["wfm"]);
    }
    dialog.add_filter("All", &["*"])
}

fn kind_sort_key(kind: Option<InstrumentKind>) -> u8 {
    match kind {
        Some(InstrumentKind::Oscilloscope) => 0,
        Some(InstrumentKind::DcSource) => 1,
        Some(InstrumentKind::ElectronicLoad) => 2,
        Some(InstrumentKind::Multimeter) => 3,
        Some(InstrumentKind::DebugProbe) => 4,
        Some(InstrumentKind::UsbBridge) => 5,
        Some(InstrumentKind::Generic) => 6,
        None => 7,
    }
}

fn instrument_kind_slot(kind: InstrumentKind) -> usize {
    match kind {
        InstrumentKind::Oscilloscope => 0,
        InstrumentKind::DcSource => 1,
        InstrumentKind::ElectronicLoad => 2,
        InstrumentKind::Multimeter => 3,
        InstrumentKind::DebugProbe => 4,
        InstrumentKind::UsbBridge => 5,
        InstrumentKind::Generic => 0,
    }
}

fn instrument_name(lang: Lang, kind: InstrumentKind) -> &'static str {
    match kind {
        InstrumentKind::Oscilloscope => text(lang, "示波器", "Oscilloscope"),
        InstrumentKind::DcSource => text(lang, "直流电源", "DC Source"),
        InstrumentKind::ElectronicLoad => text(lang, "电子负载", "Electronic Load"),
        InstrumentKind::Multimeter => text(lang, "数字万用表", "Digital Multimeter"),
        InstrumentKind::DebugProbe => text(lang, "调试探针", "Debug Probe"),
        InstrumentKind::UsbBridge => text(lang, "USB 桥", "USB Bridge"),
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
        InstrumentKind::DebugProbe => text(
            lang,
            "连接目标芯片，Halt/Run/复位，烧录 HEX/BIN/ELF，读写内存，打开 RTT。",
            "Attach a target, halt/run/reset, flash HEX/BIN/ELF, read/write memory, and stream RTT.",
        ),
        InstrumentKind::UsbBridge => text(
            lang,
            "FT4222 SPI / I2C / GPIO 收发。识别不需要 DLL，事务需要 LibFT4222。",
            "FT4222 SPI / I2C / GPIO. Scan needs no DLL; transactions need LibFT4222.",
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
        InstrumentKind::DebugProbe => text(
            lang,
            "RTT 文本与内存/烧录结果写入右侧日志。",
            "RTT text and memory/flash results go to the log panel.",
        ),
        InstrumentKind::UsbBridge => text(
            lang,
            "SPI/I2C/GPIO 往返数据写入右侧日志。",
            "SPI/I2C/GPIO traffic is written to the log panel.",
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
        InstrumentKind::DebugProbe => text(
            lang,
            "查看探针序列号、目标芯片与烧录/内存参数。",
            "View probe serial, target chip, flash and memory parameters.",
        ),
        InstrumentKind::UsbBridge => text(
            lang,
            "查看 FT4222 序列号与 SPI/I2C/GPIO 参数。",
            "View FT4222 serial and SPI/I2C/GPIO parameters.",
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
        InstrumentKind::DebugProbe => text(
            lang,
            "Halt/Run/复位、烧录、内存与 RTT",
            "Halt/run/reset, flash, memory and RTT",
        ),
        InstrumentKind::UsbBridge => text(
            lang,
            "SPI / I2C / GPIO 收发",
            "SPI / I2C / GPIO transfers",
        ),
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
        if matches!(
            kind,
            InstrumentKind::DebugProbe | InstrumentKind::UsbBridge
        ) {
            (
                text(lang, "日志", "Log"),
                instrument_acquisition_hint(lang, kind),
            )
        } else {
            (
                text(lang, "数据采集", "Data Acquisition"),
                text(
                    lang,
                    "单次/连续采样、实时曲线与 CSV",
                    "Single/continuous sampling, live plots and CSV",
                ),
            )
        },
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
        InstrumentKind::DebugProbe => "PROBE",
        InstrumentKind::UsbBridge => "FT42",
        InstrumentKind::Generic => "SCPI",
    }
}

fn resource_address_hint(kind: InstrumentKind) -> &'static str {
    match kind {
        InstrumentKind::DebugProbe => "probe://jlink/serial=...",
        InstrumentKind::UsbBridge => "bridge://ft4222/serial=...",
        _ => "USB0::0x0699::...::INSTR",
    }
}

fn resource_empty_prompt(lang: Lang, kind: InstrumentKind) -> &'static str {
    match kind {
        InstrumentKind::DebugProbe => text(lang, "选择或输入探针地址", "Select or enter probe address"),
        InstrumentKind::UsbBridge => text(lang, "选择或输入桥地址", "Select or enter bridge address"),
        _ => text(lang, "选择或输入 VISA 地址", "Select or enter VISA address"),
    }
}

fn append_io_log(log: &mut String, text: &str) {
    if text.is_empty() {
        return;
    }
    if !log.is_empty() && !log.ends_with('\n') {
        log.push('\n');
    }
    log.push_str(text);
    const MAX: usize = 32_768;
    if log.len() > MAX {
        let drain = log.len() - 24_576;
        log.drain(..drain);
    }
}

fn note_activity(slot: &mut String, text: &str) {
    let one = text.trim().replace('\n', " ");
    if one.is_empty() {
        return;
    }
    *slot = if one.chars().count() > 72 {
        format!("{}…", one.chars().take(71).collect::<String>())
    } else {
        one
    };
}

struct HudTokens {
    cyan: Color32,
    dim: Color32,
    ok: Color32,
    warn: Color32,
    panel: Color32,
    line: Color32,
}

fn hud_tokens(tokens: &Tokens) -> HudTokens {
    HudTokens {
        cyan: Color32::from_rgb(0x4A, 0xE3, 0xFF),
        dim: Color32::from_rgba_unmultiplied(0x7A, 0xC8, 0xE0, 160),
        ok: tokens.success,
        warn: tokens.warning,
        panel: Color32::from_rgb(0x07, 0x10, 0x18),
        line: Color32::from_rgba_unmultiplied(0x4A, 0xE3, 0xFF, 55),
    }
}

fn hud_chip(ui: &mut egui::Ui, hud: &HudTokens, label: impl Into<String>, color: Color32) {
    Frame::NONE
        .fill(Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 28))
        .stroke(Stroke::new(1.0_f32, color))
        .corner_radius(CornerRadius::same(3))
        .inner_margin(Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.label(
                RichText::new(label.into())
                    .small()
                    .monospace()
                    .color(color),
            );
        });
}

fn paint_hud_backdrop(painter: &egui::Painter, rect: Rect, hud: &HudTokens, t: f32) {
    painter.rect_filled(rect, CornerRadius::same(8), hud.panel);
    painter.rect_stroke(
        rect,
        CornerRadius::same(8),
        Stroke::new(1.0_f32, hud.line),
        egui::StrokeKind::Inside,
    );
    let step = 22.0;
    let mut x = rect.left();
    while x < rect.right() {
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(74, 227, 255, 12)),
        );
        x += step;
    }
    let mut y = rect.top();
    while y < rect.bottom() {
        painter.line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(74, 227, 255, 10)),
        );
        y += step;
    }
    let scan = rect.top() + (t * 48.0).rem_euclid(rect.height().max(1.0));
    painter.line_segment(
        [egui::pos2(rect.left(), scan), egui::pos2(rect.right(), scan)],
        Stroke::new(1.2_f32, Color32::from_rgba_unmultiplied(74, 227, 255, 40)),
    );
}

fn paint_control_pipeline(
    ui: &mut egui::Ui,
    lang: Lang,
    hud: &HudTokens,
    t: f32,
    sessions: usize,
    live: usize,
    scanning: bool,
) {
    let nodes = [
        (
            text(lang, "扫描", "SCAN"),
            if scanning { hud.warn } else { hud.cyan },
            scanning,
        ),
        (
            text(lang, "识别", "IDENT"),
            hud.cyan,
            sessions > 0,
        ),
        (
            text(lang, "会话", "LINK"),
            if sessions > 0 { hud.ok } else { hud.dim },
            sessions > 0,
        ),
        (
            text(lang, "控制", "CTRL"),
            if live > 0 { hud.ok } else { hud.cyan },
            live > 0,
        ),
    ];
    let pulse = 0.55 + 0.45 * (t * 3.2).sin();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (i, (label, color, on)) in nodes.iter().enumerate() {
            if i > 0 {
                let w = 28.0;
                let (r, _) = ui.allocate_exact_size(egui::vec2(w, 28.0), egui::Sense::hover());
                let y = r.center().y;
                ui.painter().line_segment(
                    [egui::pos2(r.left() + 2.0, y), egui::pos2(r.right() - 2.0, y)],
                    Stroke::new(1.2_f32, hud.line),
                );
                let dash = r.left() + (t * 40.0).rem_euclid(w);
                ui.painter().circle_filled(
                    egui::pos2(dash, y),
                    2.2,
                    Color32::from_rgba_unmultiplied(hud.cyan.r(), hud.cyan.g(), hud.cyan.b(), 180),
                );
            }
            let fill = if *on {
                Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), (40.0 + 50.0 * pulse) as u8)
            } else {
                Color32::from_rgba_unmultiplied(74, 227, 255, 16)
            };
            Frame::NONE
                .fill(fill)
                .stroke(Stroke::new(1.0_f32, *color))
                .corner_radius(CornerRadius::same(3))
                .inner_margin(Margin::symmetric(10, 5))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(*label)
                            .small()
                            .monospace()
                            .strong()
                            .color(*color),
                    );
                });
        }
    });
}

struct TwinSourceCh {
    on: bool,
    set_v: f64,
    set_i: f64,
    meas_v: Option<(f64, String)>,
    meas_i: Option<(f64, String)>,
    meas_w: Option<(f64, String)>,
}

enum TwinFace {
    Scope {
        channels: u8,
        channel_on: [bool; 4],
        timebase: f64,
        trigger: String,
        wave: Vec<f32>,
        meas: Option<String>,
    },
    DcSource {
        channels: u8,
        rows: Vec<TwinSourceCh>,
    },
    Load {
        mode: String,
        level: f64,
        input: bool,
        voltage: Option<(f64, String)>,
        current: Option<(f64, String)>,
        power: Option<(f64, String)>,
    },
    Dmm {
        function: String,
        unit: String,
        autorange: bool,
        reading: Option<(f64, String)>,
    },
    Probe {
        chip: String,
        rtt: bool,
        rtt_ch: u32,
        speed_khz: u32,
    },
    Bridge {
        tab: u8,
        spi_hz: u32,
        spi_mode: u8,
        i2c_addr: String,
        gpio_pin: u8,
        gpio_out: bool,
        gpio_value: bool,
    },
    Generic {
        cmd: String,
    },
}

struct TwinSnap {
    id: u64,
    kind: InstrumentKind,
    name: String,
    manufacturer: String,
    serial: String,
    resource: String,
    status: String,
    status_color: Color32,
    params: Vec<(String, String)>,
    activity: String,
    live: bool,
    photo: Option<egui::TextureHandle>,
    face: TwinFace,
}

fn latest_pair(
    latest: &HashMap<(u64, String), Reading>,
    id: u64,
    key: &str,
) -> Option<(f64, String)> {
    latest.get(&(id, key.to_string())).map(|r| (r.value, r.unit.clone()))
}

fn twin_face_of(
    device: &DeviceUi,
    latest: &HashMap<(u64, String), Reading>,
    wave: Option<&WaveformTrace>,
) -> TwinFace {
    let id = device.id;
    match device.kind {
        InstrumentKind::Oscilloscope => TwinFace::Scope {
            channels: device.capabilities.channels.max(1).min(4),
            channel_on: device.controls.scope_channel_on,
            timebase: device.controls.scope_timebase,
            trigger: format!(
                "{} {}",
                device.controls.trigger_source, device.controls.trigger_slope
            ),
            wave: downsample_wave_y(wave, 48),
            meas: device
                .controls
                .scope_meas_results
                .iter()
                .flatten()
                .next()
                .cloned(),
        },
        InstrumentKind::DcSource => {
            let n = device.capabilities.channels.max(1).min(4);
            let mut rows = Vec::with_capacity(n as usize);
            for i in 0..n as usize {
                let ch = i + 1;
                rows.push(TwinSourceCh {
                    on: device.controls.source_outputs[i],
                    set_v: device.controls.source_voltages[i],
                    set_i: device.controls.source_currents[i],
                    meas_v: latest_pair(latest, id, &format!("CH{ch} Voltage")),
                    meas_i: latest_pair(latest, id, &format!("CH{ch} Current")),
                    meas_w: latest_pair(latest, id, &format!("CH{ch} Power")),
                });
            }
            TwinFace::DcSource {
                channels: n,
                rows,
            }
        }
        InstrumentKind::ElectronicLoad => TwinFace::Load {
            mode: device.controls.load_mode.clone(),
            level: device.controls.load_level,
            input: device.controls.load_input,
            voltage: latest_pair(latest, id, "Voltage"),
            current: latest_pair(latest, id, "Current"),
            power: latest_pair(latest, id, "Power"),
        },
        InstrumentKind::Multimeter => TwinFace::Dmm {
            function: device.controls.dmm_function.label().to_owned(),
            unit: device.controls.dmm_function.unit().to_owned(),
            autorange: device.controls.dmm_autorange,
            reading: latest_pair(latest, id, device.controls.dmm_function.scpi())
                .or_else(|| latest_pair(latest, id, "Reading"))
                .or_else(|| {
                    latest
                        .iter()
                        .find(|((did, _), _)| *did == id)
                        .map(|(_, r)| (r.value, r.unit.clone()))
                }),
        },
        InstrumentKind::DebugProbe => TwinFace::Probe {
            chip: if device.controls.probe_chip.trim().is_empty() {
                "AUTO".into()
            } else {
                device.controls.probe_chip.clone()
            },
            rtt: device.controls.probe_rtt_on,
            rtt_ch: device.controls.probe_rtt_channel,
            speed_khz: device.controls.probe_speed_khz,
        },
        InstrumentKind::UsbBridge => TwinFace::Bridge {
            tab: device.controls.bridge_tab,
            spi_hz: device.controls.bridge_spi_hz,
            spi_mode: device.controls.bridge_spi_mode,
            i2c_addr: device.controls.bridge_i2c_addr.clone(),
            gpio_pin: device.controls.bridge_gpio_pin,
            gpio_out: device.controls.bridge_gpio_out,
            gpio_value: device.controls.bridge_gpio_value,
        },
        InstrumentKind::Generic => TwinFace::Generic {
            cmd: device.controls.console.clone(),
        },
    }
}

fn downsample_wave_y(wave: Option<&WaveformTrace>, n: usize) -> Vec<f32> {
    let Some(trace) = wave else {
        return Vec::new();
    };
    if trace.y.is_empty() || n < 2 {
        return Vec::new();
    }
    let len = trace.y.len();
    let mut ymin = f64::INFINITY;
    let mut ymax = f64::NEG_INFINITY;
    for &y in trace.y.iter() {
        ymin = ymin.min(y);
        ymax = ymax.max(y);
    }
    let span = (ymax - ymin).max(1e-12);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let idx = i * (len.saturating_sub(1)) / (n - 1);
        out.push(((trace.y[idx] - ymin) / span) as f32);
    }
    out
}

fn twin_face_json(face: &TwinFace) -> serde_json::Value {
    let meas = |p: &Option<(f64, String)>| {
        p.as_ref()
            .map(|(v, u)| serde_json::json!({ "value": v, "unit": u }))
            .unwrap_or(serde_json::Value::Null)
    };
    match face {
        TwinFace::Scope {
            channels,
            channel_on,
            timebase,
            trigger,
            wave,
            meas: scope_meas,
        } => serde_json::json!({
            "type": "oscilloscope",
            "channels": channels,
            "channel_on": channel_on,
            "timebase_s": timebase,
            "trigger": trigger,
            "measure": scope_meas,
            "wave": wave,
        }),
        TwinFace::DcSource { channels, rows } => serde_json::json!({
            "type": "dc_source",
            "channels": channels,
            "rows": rows.iter().enumerate().map(|(i, r)| serde_json::json!({
                "channel": i + 1,
                "output": r.on,
                "set_v": r.set_v,
                "set_i": r.set_i,
                "meas_v": meas(&r.meas_v),
                "meas_i": meas(&r.meas_i),
                "meas_w": meas(&r.meas_w),
            })).collect::<Vec<_>>(),
        }),
        TwinFace::Load {
            mode,
            level,
            input,
            voltage,
            current,
            power,
        } => serde_json::json!({
            "type": "electronic_load",
            "mode": mode,
            "level": level,
            "input": input,
            "voltage": meas(voltage),
            "current": meas(current),
            "power": meas(power),
        }),
        TwinFace::Dmm {
            function,
            unit,
            autorange,
            reading,
        } => serde_json::json!({
            "type": "multimeter",
            "function": function,
            "unit": unit,
            "autorange": autorange,
            "reading": meas(reading),
        }),
        TwinFace::Probe {
            chip,
            rtt,
            rtt_ch,
            speed_khz,
        } => serde_json::json!({
            "type": "debug_probe",
            "chip": chip,
            "rtt": rtt,
            "rtt_channel": rtt_ch,
            "speed_khz": speed_khz,
        }),
        TwinFace::Bridge {
            tab,
            spi_hz,
            spi_mode,
            i2c_addr,
            gpio_pin,
            gpio_out,
            gpio_value,
        } => serde_json::json!({
            "type": "usb_bridge",
            "bus": match *tab { 1 => "i2c", 2 => "gpio", _ => "spi" },
            "spi_hz": spi_hz,
            "spi_mode": spi_mode,
            "i2c_addr": i2c_addr,
            "gpio_pin": gpio_pin,
            "gpio_out": gpio_out,
            "gpio_value": gpio_value,
        }),
        TwinFace::Generic { cmd } => serde_json::json!({
            "type": "generic",
            "scpi": cmd,
        }),
    }
}

fn twin_unit_size(face: &TwinFace) -> egui::Vec2 {
    match face {
        TwinFace::DcSource { channels, .. } => {
            let n = (*channels as f32).max(1.0);
            egui::vec2(248.0, 102.0 + n * 24.0 + 26.0)
        }
        TwinFace::Scope { .. } => egui::vec2(244.0, 154.0),
        TwinFace::Dmm { .. } => egui::vec2(224.0, 138.0),
        TwinFace::Load { .. } => egui::vec2(228.0, 146.0),
        TwinFace::Probe { .. } | TwinFace::Bridge { .. } => egui::vec2(216.0, 132.0),
        TwinFace::Generic { .. } => egui::vec2(204.0, 122.0),
    }
}

fn paint_digital_twin_lab(
    ui: &egui::Ui,
    rect: Rect,
    lang: Lang,
    tokens: &Tokens,
    hud: &HudTokens,
    t: f32,
    snaps: &[TwinSnap],
    scanning: bool,
) -> Option<u64> {
    let painter = ui.painter_at(rect);
    paint_twin_floor(&painter, rect, t);

    let hub = Rect::from_center_size(
        egui::pos2(rect.center().x, rect.bottom() - rect.height() * 0.18),
        egui::vec2(132.0, 72.0),
    );
    paint_twin_hub(&painter, hub, hud, t, snaps.len(), scanning, lang);

    if snaps.is_empty() {
        painter.text(
            egui::pos2(rect.center().x, rect.center().y - 12.0),
            egui::Align2::CENTER_CENTER,
            text(
                lang,
                "等待链路  ·  扫描并连接后，工位孪生将在此展开",
                "AWAITING LINK  ·  scan and connect to populate the bench",
            ),
            egui::FontId::monospace(13.0),
            hud.dim,
        );
        return None;
    }

    let n = snaps.len() as f32;
    let scale = match snaps.len() {
        0..=3 => 1.0,
        4 => 0.92,
        5 => 0.86,
        _ => 0.78,
    };
    let rx = (rect.width() * 0.36).max(150.0);
    let ry = (rect.height() * 0.30).max(96.0);
    let origin = egui::pos2(rect.center().x, rect.center().y + rect.height() * 0.02);
    let hub_top = egui::pos2(hub.center().x, hub.top());
    let mut clicked = None;
    for (i, snap) in snaps.iter().enumerate() {
        let ang = -std::f32::consts::FRAC_PI_2
            + (i as f32 + 0.5) * (std::f32::consts::TAU / n.max(1.0));
        let size = twin_unit_size(&snap.face) * scale;
        let mut center = egui::pos2(origin.x + rx * ang.cos(), origin.y + ry * ang.sin() * 0.72);
        center.x = center.x.clamp(
            rect.left() + size.x * 0.5 + 10.0,
            rect.right() - size.x * 0.5 - 10.0,
        );
        center.y = center.y.clamp(
            rect.top() + size.y * 0.5 + 8.0,
            hub.top() - size.y * 0.45,
        );
        let card = Rect::from_center_size(center, size);
        let card_bottom = egui::pos2(card.center().x, card.bottom());
        paint_twin_link(&painter, hub_top, card_bottom, hud, t, i, snap.live);
        paint_iso_twin_unit(&painter, card, snap, hud, tokens, t);
        let hit = ui.interact(
            card.expand(6.0),
            ui.id().with(("twin-unit", snap.id)),
            egui::Sense::click(),
        );
        if hit.hovered() {
            painter.rect_stroke(
                card.expand(4.0),
                CornerRadius::same(6),
                Stroke::new(1.6_f32, hud.cyan),
                egui::StrokeKind::Outside,
            );
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if hit.clicked() {
            clicked = Some(snap.id);
        }
    }
    clicked
}

fn paint_twin_floor(painter: &egui::Painter, rect: Rect, t: f32) {
    let floor = Rect::from_min_max(
        egui::pos2(rect.left() + 18.0, rect.top() + 8.0),
        egui::pos2(rect.right() - 18.0, rect.bottom() - 8.0),
    );
    painter.rect_filled(
        floor,
        CornerRadius::same(10),
        Color32::from_rgba_unmultiplied(4, 14, 22, 160),
    );
    let vanish = egui::pos2(floor.center().x, floor.top() + 10.0);
    for i in 0..14 {
        let x = floor.left() + floor.width() * (i as f32 / 13.0);
        painter.line_segment(
            [egui::pos2(x, floor.bottom()), vanish],
            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(74, 227, 255, 18)),
        );
    }
    for i in 0..8 {
        let k = i as f32 / 7.0;
        let y = floor.bottom() - (floor.height() * 0.72) * (k * k);
        let inset = 10.0 + k * floor.width() * 0.22;
        painter.line_segment(
            [
                egui::pos2(floor.left() + inset, y),
                egui::pos2(floor.right() - inset, y),
            ],
            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(74, 227, 255, 22)),
        );
    }
    let scan = floor.top() + (t * 36.0).rem_euclid(floor.height().max(1.0));
    painter.line_segment(
        [egui::pos2(floor.left(), scan), egui::pos2(floor.right(), scan)],
        Stroke::new(1.1_f32, Color32::from_rgba_unmultiplied(74, 227, 255, 28)),
    );
}

fn paint_twin_hub(
    painter: &egui::Painter,
    rect: Rect,
    hud: &HudTokens,
    t: f32,
    sessions: usize,
    scanning: bool,
    lang: Lang,
) {
    let pulse = 0.45 + 0.55 * (t * 2.6).sin().abs();
    painter.circle_filled(
        rect.center(),
        46.0 + 4.0 * pulse,
        Color32::from_rgba_unmultiplied(74, 227, 255, (18.0 + 22.0 * pulse) as u8),
    );
    painter.rect_filled(rect, CornerRadius::same(8), Color32::from_rgb(8, 22, 34));
    painter.rect_stroke(
        rect,
        CornerRadius::same(8),
        Stroke::new(1.4_f32, hud.cyan),
        egui::StrokeKind::Inside,
    );
    painter.text(
        egui::pos2(rect.center().x, rect.top() + 16.0),
        egui::Align2::CENTER_CENTER,
        "WIPARSE",
        egui::FontId::monospace(12.0),
        hud.cyan,
    );
    painter.text(
        egui::pos2(rect.center().x, rect.center().y + 4.0),
        egui::Align2::CENTER_CENTER,
        text(lang, "工位核心", "STATION CORE"),
        egui::FontId::monospace(10.0),
        hud.dim,
    );
    painter.text(
        egui::pos2(rect.center().x, rect.bottom() - 14.0),
        egui::Align2::CENTER_CENTER,
        if scanning {
            text(lang, "扫描中…", "SCANNING…").to_owned()
        } else {
            format!("{sessions} LINK")
        },
        egui::FontId::monospace(10.0),
        if scanning { hud.warn } else { hud.ok },
    );
}

fn paint_twin_link(
    painter: &egui::Painter,
    from: egui::Pos2,
    to: egui::Pos2,
    hud: &HudTokens,
    t: f32,
    index: usize,
    live: bool,
) {
    let mid = egui::pos2((from.x + to.x) * 0.5, (from.y + to.y) * 0.5 - 18.0);
    painter.add(egui::Shape::line(
        vec![from, mid, to],
        Stroke::new(
            1.2_f32,
            if live {
                Color32::from_rgba_unmultiplied(48, 209, 88, 140)
            } else {
                hud.line
            },
        ),
    ));
    let phase = (t * 0.55 + index as f32 * 0.17).rem_euclid(1.0);
    let p = if phase < 0.5 {
        from.lerp(mid, phase * 2.0)
    } else {
        mid.lerp(to, (phase - 0.5) * 2.0)
    };
    painter.circle_filled(
        p,
        if live { 3.4 } else { 2.2 },
        if live { hud.ok } else { hud.cyan },
    );
}

fn paint_iso_twin_unit(
    painter: &egui::Painter,
    rect: Rect,
    snap: &TwinSnap,
    hud: &HudTokens,
    tokens: &Tokens,
    t: f32,
) {
    let depth = 10.0;
    let front = Rect::from_min_max(
        egui::pos2(rect.left(), rect.top() + depth),
        egui::pos2(rect.right() - depth, rect.bottom()),
    );
    let top = [
        egui::pos2(front.left(), front.top()),
        egui::pos2(front.left() + depth, front.top() - depth),
        egui::pos2(front.right() + depth, front.top() - depth),
        egui::pos2(front.right(), front.top()),
    ];
    let side = [
        egui::pos2(front.right(), front.top()),
        egui::pos2(front.right() + depth, front.top() - depth),
        egui::pos2(front.right() + depth, front.bottom() - depth),
        egui::pos2(front.right(), front.bottom()),
    ];
    let (top_fill, side_fill, face_fill) = twin_chassis_colors(snap.kind);
    painter.add(egui::Shape::convex_polygon(
        top.to_vec(),
        top_fill,
        Stroke::new(1.0_f32, hud.line),
    ));
    painter.add(egui::Shape::convex_polygon(
        side.to_vec(),
        side_fill,
        Stroke::new(1.0_f32, hud.line),
    ));
    painter.rect_filled(front, CornerRadius::ZERO, face_fill);
    painter.rect_stroke(
        front,
        CornerRadius::ZERO,
        Stroke::new(1.0_f32, hud.line),
        egui::StrokeKind::Inside,
    );
    if let Some(tex) = &snap.photo {
        let stamp = Rect::from_min_size(
            egui::pos2(top[1].x + 4.0, top[1].y + 2.0),
            egui::vec2(28.0, 18.0),
        );
        painter.image(
            tex.id(),
            stamp,
            Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }

    let header = Rect::from_min_size(front.min, egui::vec2(front.width(), 18.0));
    painter.text(
        egui::pos2(header.left() + 8.0, header.center().y),
        egui::Align2::LEFT_CENTER,
        format!("{}  {}", kind_icon(snap.kind), truncate_chars(&snap.name, 16)),
        egui::FontId::monospace(10.0),
        tokens.text_primary,
    );
    painter.text(
        egui::pos2(header.right() - 8.0, header.center().y),
        egui::Align2::RIGHT_CENTER,
        &snap.status,
        egui::FontId::monospace(9.0),
        snap.status_color,
    );

    let body = Rect::from_min_max(
        egui::pos2(front.left() + 6.0, front.top() + 20.0),
        egui::pos2(front.right() - 6.0, front.bottom() - 4.0),
    );
    paint_twin_face(painter, body, snap, hud, t);

    if snap.live {
        let pulse = 0.4 + 0.6 * (t * 4.0).sin().abs();
        painter.rect_filled(
            Rect::from_min_size(front.min, egui::vec2(3.0, front.height())),
            CornerRadius::ZERO,
            Color32::from_rgba_unmultiplied(
                hud.ok.r(),
                hud.ok.g(),
                hud.ok.b(),
                (80.0 + 140.0 * pulse) as u8,
            ),
        );
    }
    let _ = (&snap.manufacturer, &snap.activity, &snap.params, &snap.serial, &snap.resource);
}

fn twin_chassis_colors(kind: InstrumentKind) -> (Color32, Color32, Color32) {
    match kind {
        InstrumentKind::DcSource => (
            Color32::from_rgb(28, 32, 36),
            Color32::from_rgb(16, 18, 22),
            Color32::from_rgb(18, 20, 24),
        ),
        InstrumentKind::Oscilloscope => (
            Color32::from_rgb(10, 36, 52),
            Color32::from_rgb(6, 22, 34),
            Color32::from_rgb(5, 12, 20),
        ),
        InstrumentKind::ElectronicLoad => (
            Color32::from_rgb(36, 24, 16),
            Color32::from_rgb(22, 14, 10),
            Color32::from_rgb(16, 12, 10),
        ),
        InstrumentKind::Multimeter => (
            Color32::from_rgb(42, 36, 18),
            Color32::from_rgb(26, 22, 12),
            Color32::from_rgb(20, 18, 10),
        ),
        InstrumentKind::DebugProbe => (
            Color32::from_rgb(18, 40, 32),
            Color32::from_rgb(10, 24, 20),
            Color32::from_rgb(8, 16, 14),
        ),
        InstrumentKind::UsbBridge => (
            Color32::from_rgb(16, 32, 24),
            Color32::from_rgb(10, 20, 16),
            Color32::from_rgb(8, 14, 12),
        ),
        InstrumentKind::Generic => (
            Color32::from_rgb(10, 36, 52),
            Color32::from_rgb(6, 22, 34),
            Color32::from_rgb(5, 12, 20),
        ),
    }
}

fn paint_twin_face(
    painter: &egui::Painter,
    rect: Rect,
    snap: &TwinSnap,
    hud: &HudTokens,
    t: f32,
) {
    match &snap.face {
        TwinFace::DcSource { channels, rows } => {
            paint_face_dc_source(painter, rect, *channels, rows, hud, snap.live, t);
        }
        TwinFace::Scope {
            channels,
            channel_on,
            timebase,
            trigger,
            wave,
            meas,
        } => {
            paint_face_scope(
                painter,
                rect,
                *channels,
                *channel_on,
                *timebase,
                trigger,
                wave,
                meas.as_deref(),
                hud,
                snap.live,
                t,
            );
        }
        TwinFace::Load {
            mode,
            level,
            input,
            voltage,
            current,
            power,
        } => {
            paint_face_load(
                painter,
                rect,
                mode,
                *level,
                *input,
                voltage.as_ref(),
                current.as_ref(),
                power.as_ref(),
                hud,
                snap.live,
                t,
            );
        }
        TwinFace::Dmm {
            function,
            unit,
            autorange,
            reading,
        } => {
            paint_face_dmm(
                painter,
                rect,
                function,
                unit,
                *autorange,
                reading.as_ref(),
                hud,
                snap.live,
                t,
            );
        }
        TwinFace::Probe {
            chip,
            rtt,
            rtt_ch,
            speed_khz,
        } => {
            paint_face_probe(painter, rect, chip, *rtt, *rtt_ch, *speed_khz, hud, t);
        }
        TwinFace::Bridge {
            tab,
            spi_hz,
            spi_mode,
            i2c_addr,
            gpio_pin,
            gpio_out,
            gpio_value,
        } => {
            paint_face_bridge(
                painter,
                rect,
                *tab,
                *spi_hz,
                *spi_mode,
                i2c_addr,
                *gpio_pin,
                *gpio_out,
                *gpio_value,
                hud,
                t,
            );
        }
        TwinFace::Generic { cmd } => {
            paint_lcd_panel(painter, rect.shrink2(egui::vec2(0.0, 6.0)), LcdTint::Green);
            painter.text(
                egui::pos2(rect.left() + 8.0, rect.top() + 10.0),
                egui::Align2::LEFT_TOP,
                "SCPI",
                egui::FontId::monospace(9.0),
                lcd_dim(LcdTint::Green),
            );
            painter.text(
                egui::pos2(rect.left() + 8.0, rect.center().y + 4.0),
                egui::Align2::LEFT_CENTER,
                truncate_chars(cmd, 22),
                egui::FontId::monospace(11.0),
                lcd_lit(LcdTint::Green),
            );
        }
    }
}

#[derive(Clone, Copy)]
enum LcdTint {
    Amber,
    Green,
    Scope,
}

fn lcd_lit(tint: LcdTint) -> Color32 {
    match tint {
        LcdTint::Amber => Color32::from_rgb(0xFF, 0xC2, 0x4A),
        LcdTint::Green => Color32::from_rgb(0x5C, 0xFF, 0x8A),
        LcdTint::Scope => Color32::from_rgb(0x4A, 0xE3, 0xFF),
    }
}

fn lcd_dim(tint: LcdTint) -> Color32 {
    match tint {
        LcdTint::Amber => Color32::from_rgba_unmultiplied(0xFF, 0xC2, 0x4A, 120),
        LcdTint::Green => Color32::from_rgba_unmultiplied(0x5C, 0xFF, 0x8A, 120),
        LcdTint::Scope => Color32::from_rgba_unmultiplied(0x4A, 0xE3, 0xFF, 120),
    }
}

fn lcd_fill(tint: LcdTint) -> Color32 {
    match tint {
        LcdTint::Amber => Color32::from_rgb(18, 14, 6),
        LcdTint::Green => Color32::from_rgb(6, 20, 12),
        LcdTint::Scope => Color32::from_rgb(4, 16, 24),
    }
}

fn paint_lcd_panel(painter: &egui::Painter, rect: Rect, tint: LcdTint) {
    painter.rect_filled(rect, CornerRadius::same(2), lcd_fill(tint));
    painter.rect_stroke(
        rect,
        CornerRadius::same(2),
        Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(20, 20, 16, 180)),
        egui::StrokeKind::Inside,
    );
    let glass = Rect::from_min_max(
        egui::pos2(rect.left() + 1.0, rect.top() + 1.0),
        egui::pos2(rect.right() - 1.0, rect.top() + 5.0),
    );
    painter.rect_filled(
        glass,
        CornerRadius::same(1),
        Color32::from_rgba_unmultiplied(255, 255, 255, 12),
    );
}

fn paint_face_dc_source(
    painter: &egui::Painter,
    rect: Rect,
    channels: u8,
    rows: &[TwinSourceCh],
    hud: &HudTokens,
    live: bool,
    t: f32,
) {
    let jack_h = 22.0;
    let lcd = Rect::from_min_max(
        rect.min,
        egui::pos2(rect.right(), rect.bottom() - jack_h),
    );
    paint_lcd_panel(painter, lcd, LcdTint::Amber);
    let n = channels.max(1) as usize;
    let row_h = ((lcd.height() - 4.0) / n as f32).clamp(16.0, 24.0);
    for (i, row) in rows.iter().take(n).enumerate() {
        let y = lcd.top() + 4.0 + i as f32 * row_h;
        let (v, i_val, measured) = match (row.meas_v.as_ref(), row.meas_i.as_ref()) {
            (Some((v, _)), Some((a, _))) => (*v, *a, true),
            _ => (row.set_v, row.set_i, false),
        };
        let watts = row.meas_w.as_ref().map(|(w, _)| *w).unwrap_or(v * i_val);
        let tag = if measured {
            if live {
                "MEAS"
            } else {
                "HOLD"
            }
        } else {
            "SET"
        };
        let color = if row.on {
            lcd_lit(LcdTint::Amber)
        } else {
            lcd_dim(LcdTint::Amber)
        };
        painter.text(
            egui::pos2(lcd.left() + 6.0, y + 2.0),
            egui::Align2::LEFT_TOP,
            format!("CH{}", i + 1),
            egui::FontId::monospace(10.0),
            color,
        );
        painter.text(
            egui::pos2(lcd.left() + 34.0, y + 2.0),
            egui::Align2::LEFT_TOP,
            format!("{v:6.3}V  {i_val:5.3}A  {watts:5.2}W"),
            egui::FontId::monospace(10.0),
            color,
        );
        painter.text(
            egui::pos2(lcd.right() - 6.0, y + 2.0),
            egui::Align2::RIGHT_TOP,
            if row.on {
                format!("ON {tag}")
            } else {
                format!("OFF")
            },
            egui::FontId::monospace(10.0),
            if row.on { hud.ok } else { lcd_dim(LcdTint::Amber) },
        );
    }
    if live {
        let scan = lcd.top() + (t * 22.0).rem_euclid(lcd.height().max(1.0));
        painter.line_segment(
            [egui::pos2(lcd.left() + 2.0, scan), egui::pos2(lcd.right() - 2.0, scan)],
            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 194, 74, 28)),
        );
    }
    let jack_area = Rect::from_min_max(
        egui::pos2(rect.left(), lcd.bottom() + 2.0),
        rect.max,
    );
    let slot_w = jack_area.width() / n as f32;
    for i in 0..n {
        let cx = jack_area.left() + slot_w * (i as f32 + 0.5);
        let cy = jack_area.center().y;
        let on = rows.get(i).map(|r| r.on).unwrap_or(false);
        painter.circle_filled(egui::pos2(cx - 7.0, cy), 4.2, Color32::from_rgb(176, 42, 42));
        painter.circle_filled(egui::pos2(cx + 7.0, cy), 4.2, Color32::from_rgb(28, 28, 30));
        painter.circle_stroke(
            egui::pos2(cx - 7.0, cy),
            4.2,
            Stroke::new(1.0_f32, Color32::from_rgb(90, 20, 20)),
        );
        painter.circle_stroke(
            egui::pos2(cx + 7.0, cy),
            4.2,
            Stroke::new(1.0_f32, Color32::from_rgb(60, 60, 64)),
        );
        let led = if on { hud.ok } else { Color32::from_rgb(32, 40, 32) };
        painter.circle_filled(egui::pos2(cx, cy - 8.0), 2.2, led);
        painter.text(
            egui::pos2(cx, jack_area.bottom() - 1.0),
            egui::Align2::CENTER_BOTTOM,
            format!("{}", i + 1),
            egui::FontId::monospace(8.0),
            hud.dim,
        );
    }
}

const SCOPE_CH_COLORS: [Color32; 4] = [
    Color32::from_rgb(0xFF, 0xD6, 0x0A),
    Color32::from_rgb(0x4A, 0xE3, 0xFF),
    Color32::from_rgb(0xFF, 0x6B, 0xC9),
    Color32::from_rgb(0x30, 0xD1, 0x58),
];

fn paint_face_scope(
    painter: &egui::Painter,
    rect: Rect,
    channels: u8,
    channel_on: [bool; 4],
    timebase: f64,
    trigger: &str,
    wave: &[f32],
    meas: Option<&str>,
    hud: &HudTokens,
    live: bool,
    t: f32,
) {
    let n = channels.max(1).min(4) as usize;
    let knobs_w = 28.0;
    let screen = Rect::from_min_max(
        rect.min,
        egui::pos2(rect.right() - knobs_w, rect.bottom()),
    );
    paint_lcd_panel(painter, screen, LcdTint::Scope);
    for g in 1..4 {
        let x = screen.left() + screen.width() * (g as f32 / 4.0);
        painter.line_segment(
            [egui::pos2(x, screen.top() + 2.0), egui::pos2(x, screen.bottom() - 2.0)],
            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(74, 227, 255, 28)),
        );
        let y = screen.top() + screen.height() * (g as f32 / 4.0);
        painter.line_segment(
            [egui::pos2(screen.left() + 2.0, y), egui::pos2(screen.right() - 2.0, y)],
            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(74, 227, 255, 28)),
        );
    }
    let mut pts = Vec::new();
    if wave.len() >= 2 {
        for (i, y01) in wave.iter().enumerate() {
            let x = screen.left() + 3.0 + (screen.width() - 6.0) * (i as f32 / (wave.len() - 1) as f32);
            let y = screen.bottom() - 4.0 - (screen.height() - 8.0) * y01;
            pts.push(egui::pos2(x, y));
        }
    } else {
        let amp = if live { 0.32 } else { 0.08 };
        for i in 0..36 {
            let k = i as f32 / 35.0;
            let x = screen.left() + 3.0 + (screen.width() - 6.0) * k;
            let y = screen.center().y
                + (t * 3.2 + k * 8.0).sin() * screen.height() * amp;
            pts.push(egui::pos2(x, y));
        }
    }
    let wave_color = SCOPE_CH_COLORS
        .iter()
        .enumerate()
        .find(|(i, _)| *i < n && channel_on[*i])
        .map(|(_, c)| *c)
        .unwrap_or(lcd_lit(LcdTint::Scope));
    painter.add(egui::Shape::line(pts, Stroke::new(1.4_f32, wave_color)));
    painter.text(
        egui::pos2(screen.left() + 6.0, screen.top() + 4.0),
        egui::Align2::LEFT_TOP,
        fmt_timebase(timebase),
        egui::FontId::monospace(9.0),
        lcd_dim(LcdTint::Scope),
    );
    painter.text(
        egui::pos2(screen.right() - 6.0, screen.top() + 4.0),
        egui::Align2::RIGHT_TOP,
        truncate_chars(trigger, 10),
        egui::FontId::monospace(9.0),
        lcd_dim(LcdTint::Scope),
    );
    if let Some(meas) = meas {
        painter.text(
            egui::pos2(screen.left() + 6.0, screen.bottom() - 4.0),
            egui::Align2::LEFT_BOTTOM,
            truncate_chars(meas, 18),
            egui::FontId::monospace(9.0),
            lcd_lit(LcdTint::Scope),
        );
    }
    let knob_col = Rect::from_min_max(egui::pos2(screen.right() + 4.0, rect.top()), rect.max);
    for i in 0..n {
        let y = knob_col.top() + 10.0 + i as f32 * 18.0;
        let on = channel_on.get(i).copied().unwrap_or(false);
        let color = if on { SCOPE_CH_COLORS[i] } else { Color32::from_rgb(40, 48, 52) };
        painter.circle_filled(egui::pos2(knob_col.center().x, y), 5.4, color);
        painter.circle_stroke(
            egui::pos2(knob_col.center().x, y),
            5.4,
            Stroke::new(1.0_f32, hud.line),
        );
        painter.text(
            egui::pos2(knob_col.center().x, y + 8.0),
            egui::Align2::CENTER_TOP,
            format!("{}", i + 1),
            egui::FontId::monospace(7.0),
            if on { SCOPE_CH_COLORS[i] } else { hud.dim },
        );
    }
}

fn paint_face_load(
    painter: &egui::Painter,
    rect: Rect,
    mode: &str,
    level: f64,
    input: bool,
    voltage: Option<&(f64, String)>,
    current: Option<&(f64, String)>,
    power: Option<&(f64, String)>,
    hud: &HudTokens,
    live: bool,
    t: f32,
) {
    let lcd = Rect::from_min_max(rect.min, egui::pos2(rect.right() - 36.0, rect.bottom()));
    paint_lcd_panel(painter, lcd, LcdTint::Amber);
    let color = if input {
        lcd_lit(LcdTint::Amber)
    } else {
        lcd_dim(LcdTint::Amber)
    };
    painter.text(
        egui::pos2(lcd.left() + 8.0, lcd.top() + 6.0),
        egui::Align2::LEFT_TOP,
        format!("{}  LVL {:.4}  {}", mode, level, if input { "SINK" } else { "OPEN" }),
        egui::FontId::monospace(10.0),
        color,
    );
    let v = voltage
        .map(|(v, _)| format!("{v:7.3} V"))
        .unwrap_or_else(|| "  —.--- V".into());
    let a = current
        .map(|(a, _)| format!("{a:7.3} A"))
        .unwrap_or_else(|| "  —.--- A".into());
    let w = power
        .map(|(w, _)| format!("{w:7.2} W"))
        .unwrap_or_else(|| "  —.--- W".into());
    painter.text(
        egui::pos2(lcd.left() + 8.0, lcd.center().y - 2.0),
        egui::Align2::LEFT_CENTER,
        format!("{v}   {a}"),
        egui::FontId::monospace(13.0),
        color,
    );
    painter.text(
        egui::pos2(lcd.left() + 8.0, lcd.bottom() - 8.0),
        egui::Align2::LEFT_BOTTOM,
        if live { format!("{w}  MEAS") } else { w },
        egui::FontId::monospace(11.0),
        color,
    );
    if live {
        let scan = lcd.top() + (t * 18.0).rem_euclid(lcd.height().max(1.0));
        painter.line_segment(
            [egui::pos2(lcd.left() + 2.0, scan), egui::pos2(lcd.right() - 2.0, scan)],
            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 194, 74, 24)),
        );
    }
    let sink = Rect::from_min_max(egui::pos2(lcd.right() + 4.0, rect.top() + 8.0), rect.max);
    painter.line_segment(
        [egui::pos2(sink.center().x, sink.top()), egui::pos2(sink.center().x, sink.bottom() - 10.0)],
        Stroke::new(1.4_f32, hud.cyan),
    );
    painter.line_segment(
        [
            egui::pos2(sink.center().x - 10.0, sink.center().y),
            egui::pos2(sink.center().x + 10.0, sink.center().y),
        ],
        Stroke::new(1.4_f32, hud.cyan),
    );
    painter.circle_filled(
        egui::pos2(sink.center().x, sink.bottom() - 6.0),
        4.0,
        if input { hud.ok } else { Color32::from_rgb(40, 40, 40) },
    );
}

fn paint_face_dmm(
    painter: &egui::Painter,
    rect: Rect,
    function: &str,
    unit: &str,
    autorange: bool,
    reading: Option<&(f64, String)>,
    hud: &HudTokens,
    live: bool,
    t: f32,
) {
    let lcd = Rect::from_min_max(rect.min, egui::pos2(rect.right() - 34.0, rect.bottom()));
    paint_lcd_panel(painter, lcd, LcdTint::Green);
    painter.text(
        egui::pos2(lcd.left() + 8.0, lcd.top() + 5.0),
        egui::Align2::LEFT_TOP,
        format!(
            "{function}  {}  {}",
            if autorange { "AUTO" } else { "MAN" },
            if live { "MEAS" } else { "HOLD" }
        ),
        egui::FontId::monospace(9.0),
        lcd_dim(LcdTint::Green),
    );
    let (digits, shown_unit) = if let Some((v, u)) = reading {
        (fmt_dmm_digits(*v), u.clone())
    } else {
        ("------".into(), unit.to_owned())
    };
    let blink = if live {
        0.75 + 0.25 * (t * 2.0).sin()
    } else {
        1.0
    };
    let mut color = lcd_lit(LcdTint::Green);
    color = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), (220.0 * blink) as u8);
    painter.text(
        egui::pos2(lcd.left() + 10.0, lcd.center().y + 6.0),
        egui::Align2::LEFT_CENTER,
        digits,
        egui::FontId::monospace(22.0),
        color,
    );
    painter.text(
        egui::pos2(lcd.right() - 8.0, lcd.bottom() - 8.0),
        egui::Align2::RIGHT_BOTTOM,
        shown_unit,
        egui::FontId::monospace(12.0),
        lcd_lit(LcdTint::Green),
    );
    let knob_c = egui::pos2(rect.right() - 16.0, rect.center().y);
    painter.circle_filled(knob_c, 12.0, Color32::from_rgb(28, 24, 12));
    painter.circle_stroke(knob_c, 12.0, Stroke::new(1.2_f32, hud.line));
    let ang: f32 = match function {
        "DC V" => -1.2,
        "AC V" => -0.5,
        "DC A" => 0.2,
        "AC A" => 0.8,
        "Ω" => 1.4,
        _ => 2.0,
    };
    painter.line_segment(
        [
            knob_c,
            egui::pos2(knob_c.x + ang.cos() * 9.0, knob_c.y + ang.sin() * 9.0),
        ],
        Stroke::new(1.6_f32, lcd_lit(LcdTint::Amber)),
    );
}

fn paint_face_probe(
    painter: &egui::Painter,
    rect: Rect,
    chip: &str,
    rtt: bool,
    rtt_ch: u32,
    speed_khz: u32,
    hud: &HudTokens,
    t: f32,
) {
    let lcd = Rect::from_min_max(rect.min, egui::pos2(rect.right() - 52.0, rect.bottom()));
    paint_lcd_panel(painter, lcd, LcdTint::Green);
    painter.text(
        egui::pos2(lcd.left() + 8.0, lcd.top() + 6.0),
        egui::Align2::LEFT_TOP,
        format!("TGT  {}", truncate_chars(chip, 14)),
        egui::FontId::monospace(11.0),
        lcd_lit(LcdTint::Green),
    );
    painter.text(
        egui::pos2(lcd.left() + 8.0, lcd.center().y + 4.0),
        egui::Align2::LEFT_CENTER,
        format!("{speed_khz} kHz"),
        egui::FontId::monospace(12.0),
        lcd_lit(LcdTint::Green),
    );
    painter.text(
        egui::pos2(lcd.left() + 8.0, lcd.bottom() - 8.0),
        egui::Align2::LEFT_BOTTOM,
        if rtt {
            format!("RTT CH{rtt_ch}")
        } else {
            "RTT OFF".into()
        },
        egui::FontId::monospace(10.0),
        if rtt { hud.ok } else { lcd_dim(LcdTint::Green) },
    );
    let header = Rect::from_min_max(egui::pos2(lcd.right() + 6.0, rect.top() + 6.0), rect.max);
    let labels = ["SWDIO", "SWCLK", "GND", "VTref"];
    for (i, label) in labels.iter().enumerate() {
        let y = header.top() + 8.0 + i as f32 * 16.0;
        let pulse = if rtt {
            0.45 + 0.55 * (t * 5.0 + i as f32).sin().abs()
        } else {
            0.25
        };
        painter.line_segment(
            [egui::pos2(header.left(), y), egui::pos2(header.right() - 4.0, y)],
            Stroke::new(
                1.2_f32,
                Color32::from_rgba_unmultiplied(hud.cyan.r(), hud.cyan.g(), hud.cyan.b(), (80.0 + 140.0 * pulse) as u8),
            ),
        );
        painter.circle_filled(
            egui::pos2(header.right() - 4.0, y),
            2.4,
            hud.cyan,
        );
        painter.text(
            egui::pos2(header.left(), y - 7.0),
            egui::Align2::LEFT_BOTTOM,
            *label,
            egui::FontId::monospace(7.0),
            hud.dim,
        );
    }
}

fn paint_face_bridge(
    painter: &egui::Painter,
    rect: Rect,
    tab: u8,
    spi_hz: u32,
    spi_mode: u8,
    i2c_addr: &str,
    gpio_pin: u8,
    gpio_out: bool,
    gpio_value: bool,
    hud: &HudTokens,
    t: f32,
) {
    let lcd = Rect::from_min_max(rect.min, egui::pos2(rect.right(), rect.bottom() - 28.0));
    paint_lcd_panel(painter, lcd, LcdTint::Green);
    let bus = match tab {
        1 => "I2C",
        2 => "GPIO",
        _ => "SPI",
    };
    painter.text(
        egui::pos2(lcd.left() + 8.0, lcd.top() + 6.0),
        egui::Align2::LEFT_TOP,
        format!("FT4222  {bus}"),
        egui::FontId::monospace(11.0),
        lcd_lit(LcdTint::Green),
    );
    let detail = match tab {
        1 => format!("ADDR 0x{i2c_addr}"),
        2 => format!(
            "P{gpio_pin} {} {}",
            if gpio_out { "OUT" } else { "IN" },
            if gpio_value { "1" } else { "0" }
        ),
        _ => format!("MODE {spi_mode}  {spi_hz} Hz"),
    };
    painter.text(
        egui::pos2(lcd.left() + 8.0, lcd.center().y + 6.0),
        egui::Align2::LEFT_CENTER,
        detail,
        egui::FontId::monospace(12.0),
        lcd_lit(LcdTint::Green),
    );
    let buses = ["SPI", "I2C", "GPIO"];
    let slot_w = rect.width() / 3.0;
    for (i, label) in buses.iter().enumerate() {
        let cx = rect.left() + slot_w * (i as f32 + 0.5);
        let y = rect.bottom() - 12.0;
        let on = tab as usize == i;
        painter.circle_filled(
            egui::pos2(cx - 14.0, y),
            3.2,
            if on { hud.ok } else { Color32::from_rgb(40, 48, 40) },
        );
        painter.text(
            egui::pos2(cx - 8.0, y),
            egui::Align2::LEFT_CENTER,
            *label,
            egui::FontId::monospace(9.0),
            if on { hud.cyan } else { hud.dim },
        );
        if on {
            let pulse = 0.4 + 0.6 * (t * 3.4).sin().abs();
            painter.circle_filled(
                egui::pos2(cx - 14.0, y),
                3.2 + pulse,
                Color32::from_rgba_unmultiplied(hud.ok.r(), hud.ok.g(), hud.ok.b(), 80),
            );
        }
    }
}

fn fmt_timebase(s: f64) -> String {
    if s >= 1.0 {
        format!("{s:.3}s/div")
    } else if s >= 1e-3 {
        format!("{:.3}ms/div", s * 1e3)
    } else if s >= 1e-6 {
        format!("{:.1}µs/div", s * 1e6)
    } else {
        format!("{:.0}ns/div", s * 1e9)
    }
}

fn fmt_dmm_digits(v: f64) -> String {
    let a = v.abs();
    let s = if a >= 1000.0 {
        format!("{v:8.2}")
    } else if a >= 100.0 {
        format!("{v:8.3}")
    } else if a >= 10.0 {
        format!("{v:8.4}")
    } else {
        format!("{v:8.5}")
    };
    s
}

fn truncate_chars(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        s.to_owned()
    } else {
        format!("{}…", s.chars().take(max.saturating_sub(1)).collect::<String>())
    }
}

fn overview_type_card(
    ui: &mut egui::Ui,
    lang: Lang,
    tokens: &Tokens,
    selected: bool,
    sessions: usize,
) -> bool {
    let mut clicked = false;
    let frame = Frame::NONE
        .fill(if selected {
            Color32::from_rgba_unmultiplied(0x07, 0x18, 0x24, 255)
        } else {
            tokens.panel_bg
        })
        .stroke(Stroke::new(
            if selected { 2.0_f32 } else { 1.0_f32 },
            if selected {
                Color32::from_rgb(0x4A, 0xE3, 0xFF)
            } else {
                tokens.border
            },
        ))
        .corner_radius(CornerRadius::same(7))
        .inner_margin(Margin::symmetric(11, 9))
        .show(ui, |ui| {
            ui.set_min_width(CARD_PANEL_WIDTH);
            ui.set_max_width(CARD_PANEL_WIDTH);
            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(CARD_ICON_COLUMN_WIDTH, 0.0),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        let (icon_rect, _) =
                            ui.allocate_exact_size(egui::vec2(36.0, 28.0), egui::Sense::hover());
                        paint_mesh_icon(ui.painter(), icon_rect, selected);
                    },
                );
                ui.vertical(|ui| {
                    if ui
                        .selectable_label(
                            selected,
                            RichText::new(text(lang, "设备总览", "Control Mesh")).strong(),
                        )
                        .clicked()
                    {
                        clicked = true;
                    }
                    ui.label(
                        RichText::new(format!(
                            "{} · {sessions}",
                            text(lang, "已连接会话", "Live sessions")
                        ))
                        .small()
                        .color(if sessions > 0 {
                            Color32::from_rgb(0x4A, 0xE3, 0xFF)
                        } else {
                            tokens.text_muted
                        }),
                    );
                    ui.label(
                        RichText::new(text(
                            lang,
                            "全部设备与控制流",
                            "All devices and control flow",
                        ))
                        .small()
                        .color(tokens.text_muted),
                    );
                });
            });
        });
    let bg = ui.interact(
        frame.response.rect,
        ui.id().with("overview-type-card"),
        egui::Sense::click(),
    );
    if bg.clicked() {
        clicked = true;
    }
    if bg.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    clicked
}

#[allow(dead_code)]
fn overview_device_tile(
    ui: &mut egui::Ui,
    lang: Lang,
    tokens: &Tokens,
    hud: &HudTokens,
    t: f32,
    tile_w: f32,
    tile_h: f32,
    device: &DeviceUi,
    busy_device: Option<u64>,
    busy_label: &str,
    latest: &HashMap<(u64, String), Reading>,
    photo: Option<&egui::TextureHandle>,
) -> bool {
    let (status, status_color) =
        overview_runtime_status(lang, device, busy_device, busy_label);
    let params = overview_param_lines(lang, device, latest);
    let mut opened = false;
    let frame = Frame::NONE
        .fill(Color32::from_rgb(0x05, 0x0C, 0x14))
        .stroke(Stroke::new(1.0_f32, hud.line))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(tile_w - 12.0);
            ui.set_min_height(tile_h - 12.0);
            ui.horizontal(|ui| {
                let glyph = ui.allocate_exact_size(egui::vec2(84.0, 72.0), egui::Sense::hover());
                if let Some(tex) = photo {
                    let img = egui::Image::from_texture(tex)
                        .fit_to_exact_size(egui::vec2(84.0, 72.0))
                        .corner_radius(CornerRadius::same(4));
                    ui.put(glyph.0, img);
                } else {
                    paint_device_glyph(ui.painter(), glyph.0, device.kind, hud, t);
                }
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(kind_icon(device.kind))
                                .small()
                                .monospace()
                                .color(hud.cyan),
                        );
                        hud_chip(ui, hud, status, status_color);
                    });
                    let name = if device.identity.model.trim().is_empty() {
                        instrument_name(lang, device.kind).to_owned()
                    } else {
                        device.identity.model.clone()
                    };
                    ui.label(RichText::new(name).strong().color(tokens.text_primary).size(14.0));
                    ui.label(
                        RichText::new(&device.identity.manufacturer)
                            .small()
                            .color(hud.dim),
                    );
                });
            });
            ui.add_space(4.0);
            let serial = if device.identity.serial.trim().is_empty() {
                short_resource(&device.resource)
            } else {
                format!("SN {}", device.identity.serial)
            };
            ui.label(
                RichText::new(serial)
                    .small()
                    .monospace()
                    .color(hud.dim),
            );
            ui.label(
                RichText::new(short_resource(&device.resource))
                    .small()
                    .monospace()
                    .color(Color32::from_rgba_unmultiplied(74, 227, 255, 90)),
            );
            ui.add_space(6.0);
            for (k, v) in params.iter().take(4) {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(k)
                            .small()
                            .monospace()
                            .color(hud.dim),
                    );
                    ui.label(
                        RichText::new(v)
                            .small()
                            .monospace()
                            .color(hud.cyan),
                    );
                });
            }
            if !device.last_activity.is_empty() {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!("▶ {}", device.last_activity))
                        .small()
                        .monospace()
                        .color(status_color),
                );
            }
        });
    let hit = ui.interact(
        frame.response.rect,
        ui.id().with(("overview-tile", device.id)),
        egui::Sense::click(),
    );
    if hit.hovered() {
        ui.painter().rect_stroke(
            frame.response.rect,
            CornerRadius::same(6),
            Stroke::new(1.6_f32, hud.cyan),
            egui::StrokeKind::Outside,
        );
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if device.acquiring && !device.paused {
        let pulse = 0.35 + 0.65 * (t * 4.2).sin().abs();
        let edge = Rect::from_min_size(
            frame.response.rect.min,
            egui::vec2(3.0, frame.response.rect.height()),
        );
        ui.painter().rect_filled(
            edge,
            CornerRadius::ZERO,
            Color32::from_rgba_unmultiplied(
                hud.ok.r(),
                hud.ok.g(),
                hud.ok.b(),
                (70.0 + 140.0 * pulse) as u8,
            ),
        );
    }
    if hit.clicked() {
        opened = true;
    }
    opened
}

fn overview_runtime_status(
    lang: Lang,
    device: &DeviceUi,
    busy_device: Option<u64>,
    busy_label: &str,
) -> (String, Color32) {
    if busy_device == Some(device.id) {
        let label = if busy_label.trim().is_empty() {
            text(lang, "作业中", "BUSY").to_owned()
        } else {
            busy_label.to_owned()
        };
        return (label, Color32::from_rgb(0xFF, 0xD6, 0x0A));
    }
    if device.acquiring && device.paused {
        return (text(lang, "保持", "HOLD").into(), Color32::from_rgb(0xFF, 0x9F, 0x0A));
    }
    if device.acquiring {
        return (text(lang, "采集", "LIVE").into(), Color32::from_rgb(0x30, 0xD1, 0x58));
    }
    if device.controls.probe_rtt_on {
        return ("RTT".into(), Color32::from_rgb(0x4A, 0xE3, 0xFF));
    }
    if device.controls.source_outputs.iter().any(|on| *on) {
        return (text(lang, "输出开", "OUTPUT").into(), Color32::from_rgb(0x30, 0xD1, 0x58));
    }
    if device.controls.load_input {
        return (text(lang, "带载", "SINK").into(), Color32::from_rgb(0x30, 0xD1, 0x58));
    }
    (text(lang, "待机", "STANDBY").into(), Color32::from_rgb(0x7A, 0xC8, 0xE0))
}

fn overview_status_key(device: &DeviceUi, busy_device: Option<u64>) -> &'static str {
    if busy_device == Some(device.id) {
        "busy"
    } else if device.acquiring && device.paused {
        "hold"
    } else if device.acquiring {
        "live"
    } else if device.controls.probe_rtt_on {
        "rtt"
    } else if device.controls.source_outputs.iter().any(|on| *on) {
        "output"
    } else if device.controls.load_input {
        "sink"
    } else {
        "standby"
    }
}

fn overview_param_lines(
    lang: Lang,
    device: &DeviceUi,
    latest: &HashMap<(u64, String), Reading>,
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let id = device.id;
    let fmt = |r: &Reading| format!("{:.4} {}", r.value, r.unit);
    match device.kind {
        InstrumentKind::Oscilloscope => {
            let ch: String = (0..4)
                .filter(|&i| device.controls.scope_channel_on[i])
                .map(|i| format!("CH{}", i + 1))
                .collect::<Vec<_>>()
                .join(" ");
            out.push((
                text(lang, "通道", "CH").into(),
                if ch.is_empty() { "—".into() } else { ch },
            ));
            out.push((
                "s/div".into(),
                format!("{:.3e}", device.controls.scope_timebase),
            ));
            out.push((
                text(lang, "触发", "TRG").into(),
                format!(
                    "{} {} {:.3}V",
                    device.controls.trigger_source,
                    device.controls.trigger_slope,
                    device.controls.trigger_level
                ),
            ));
            if let Some(meas) = device.controls.scope_meas_results.iter().flatten().next() {
                out.push((text(lang, "测量", "MEAS").into(), meas.clone()));
            }
        }
        InstrumentKind::DcSource => {
            for i in 0..device.capabilities.channels.max(1).min(4) as usize {
                let ch = i + 1;
                let on = if device.controls.source_outputs[i] {
                    "ON"
                } else {
                    "OFF"
                };
                let v = latest_pair(latest, id, &format!("CH{ch} Voltage"))
                    .map(|(v, _)| v)
                    .unwrap_or(device.controls.source_voltages[i]);
                let a = latest_pair(latest, id, &format!("CH{ch} Current"))
                    .map(|(a, _)| a)
                    .unwrap_or(device.controls.source_currents[i]);
                out.push((format!("CH{ch}"), format!("{v:.3} V  {a:.3} A  {on}")));
            }
        }
        InstrumentKind::ElectronicLoad => {
            out.push((
                text(lang, "模式", "MODE").into(),
                device.controls.load_mode.clone(),
            ));
            out.push((
                text(lang, "电平", "LVL").into(),
                format!("{:.4}", device.controls.load_level),
            ));
            out.push((
                text(lang, "输入", "INP").into(),
                if device.controls.load_input {
                    "ON".into()
                } else {
                    "OFF".into()
                },
            ));
        }
        InstrumentKind::Multimeter => {
            out.push((
                text(lang, "功能", "FUNC").into(),
                device.controls.dmm_function.label().into(),
            ));
            if let Some((_, r)) = latest.iter().find(|((did, _), _)| *did == id) {
                out.push((text(lang, "读数", "READ").into(), fmt(r)));
            }
        }
        InstrumentKind::DebugProbe => {
            out.push((
                text(lang, "目标", "TGT").into(),
                if device.controls.probe_chip.trim().is_empty() {
                    "DP / AUTO".into()
                } else {
                    device.controls.probe_chip.clone()
                },
            ));
            out.push((
                "RTT".into(),
                if device.controls.probe_rtt_on {
                    format!("CH{}", device.controls.probe_rtt_channel)
                } else {
                    "OFF".into()
                },
            ));
            if !device.controls.probe_flash_path.is_empty() {
                out.push((
                    text(lang, "烧录", "FW").into(),
                    device.controls.probe_flash_path.clone(),
                ));
            }
        }
        InstrumentKind::UsbBridge => {
            let mode = match device.controls.bridge_tab {
                1 => "I2C",
                2 => "GPIO",
                _ => "SPI",
            };
            out.push((text(lang, "总线", "BUS").into(), mode.into()));
            if device.controls.bridge_tab == 0 {
                out.push((
                    "SPI".into(),
                    format!(
                        "m{} {} Hz",
                        device.controls.bridge_spi_mode, device.controls.bridge_spi_hz
                    ),
                ));
            } else if device.controls.bridge_tab == 1 {
                out.push((
                    "I2C".into(),
                    format!("0x{}", device.controls.bridge_i2c_addr),
                ));
            } else {
                out.push((
                    "GPIO".into(),
                    format!("P{}", device.controls.bridge_gpio_pin),
                ));
            }
        }
        InstrumentKind::Generic => {
            out.push(("SCPI".into(), device.controls.console.clone()));
        }
    }
    for ((did, ch), reading) in latest {
        if *did != id || out.len() >= 4 {
            continue;
        }
        if out.iter().any(|(k, _)| k == ch) {
            continue;
        }
        out.push((ch.clone(), fmt(reading)));
    }
    out
}

fn paint_device_glyph(
    painter: &egui::Painter,
    rect: Rect,
    kind: InstrumentKind,
    hud: &HudTokens,
    t: f32,
) {
    let r = rect.shrink(4.0);
    painter.rect_filled(
        r,
        CornerRadius::same(4),
        Color32::from_rgba_unmultiplied(10, 28, 40, 180),
    );
    painter.rect_stroke(
        r,
        CornerRadius::same(4),
        Stroke::new(1.0_f32, hud.line),
        egui::StrokeKind::Inside,
    );
    let c = r.center();
    let glow = 0.35 + 0.25 * (t * 2.4).sin();
    let accent = Color32::from_rgba_unmultiplied(
        hud.cyan.r(),
        hud.cyan.g(),
        hud.cyan.b(),
        (90.0 + 120.0 * glow) as u8,
    );
    let stroke = Stroke::new(1.4_f32, hud.cyan);
    match kind {
        InstrumentKind::Oscilloscope => {
            let screen = Rect::from_min_max(
                egui::pos2(r.left() + 8.0, r.top() + 8.0),
                egui::pos2(r.right() - 22.0, r.bottom() - 10.0),
            );
            painter.rect_filled(screen, CornerRadius::same(2), Color32::from_rgb(4, 18, 28));
            painter.rect_stroke(screen, CornerRadius::same(2), stroke, egui::StrokeKind::Inside);
            let mut pts = Vec::new();
            for i in 0..24 {
                let x = screen.left() + screen.width() * (i as f32 / 23.0);
                let y = screen.center().y
                    + (t * 4.0 + i as f32 * 0.45).sin() * screen.height() * 0.28;
                pts.push(egui::pos2(x, y));
            }
            painter.add(egui::Shape::line(pts, Stroke::new(1.2_f32, accent)));
            for i in 0..3 {
                painter.circle_filled(
                    egui::pos2(r.right() - 11.0, r.top() + 16.0 + i as f32 * 14.0),
                    4.0,
                    hud.line,
                );
            }
        }
        InstrumentKind::DcSource => {
            for i in 0..3 {
                let y = r.top() + 14.0 + i as f32 * 16.0;
                let bar = Rect::from_min_size(egui::pos2(r.left() + 10.0, y), egui::vec2(r.width() * 0.62, 9.0));
                painter.rect_filled(bar, CornerRadius::same(1), Color32::from_rgb(8, 32, 44));
                painter.rect_filled(
                    Rect::from_min_size(
                        bar.min,
                        egui::vec2(bar.width() * (0.35 + 0.15 * i as f32), bar.height()),
                    ),
                    CornerRadius::same(1),
                    accent,
                );
                painter.circle_filled(egui::pos2(r.right() - 14.0, y + 4.5), 4.0, hud.ok);
            }
        }
        InstrumentKind::ElectronicLoad => {
            let top = egui::pos2(c.x, r.top() + 10.0);
            let mid = egui::pos2(c.x, c.y + 4.0);
            painter.line_segment([top, mid], stroke);
            painter.line_segment(
                [egui::pos2(c.x - 18.0, mid.y), egui::pos2(c.x + 18.0, mid.y)],
                stroke,
            );
            painter.add(egui::Shape::line(
                vec![
                    egui::pos2(c.x - 10.0, mid.y),
                    egui::pos2(c.x - 4.0, mid.y + 10.0),
                    egui::pos2(c.x + 4.0, mid.y - 2.0),
                    egui::pos2(c.x + 10.0, mid.y + 16.0),
                ],
                Stroke::new(1.6_f32, accent),
            ));
        }
        InstrumentKind::Multimeter => {
            let face = Rect::from_center_size(c, egui::vec2(r.width() - 18.0, 28.0));
            painter.rect_filled(face, CornerRadius::same(2), Color32::from_rgb(4, 20, 16));
            painter.rect_stroke(face, CornerRadius::same(2), stroke, egui::StrokeKind::Inside);
            painter.text(
                face.center(),
                egui::Align2::CENTER_CENTER,
                "3.300",
                egui::FontId::monospace(13.0),
                hud.ok,
            );
        }
        InstrumentKind::DebugProbe => {
            let body = Rect::from_center_size(
                egui::pos2(c.x - 6.0, c.y),
                egui::vec2(40.0, 22.0),
            );
            painter.rect_filled(body, CornerRadius::same(3), Color32::from_rgb(12, 36, 52));
            painter.rect_stroke(body, CornerRadius::same(3), stroke, egui::StrokeKind::Inside);
            for i in 0..4 {
                let y = body.top() + 5.0 + i as f32 * 4.0;
                painter.line_segment(
                    [egui::pos2(body.right(), y), egui::pos2(r.right() - 8.0, y)],
                    Stroke::new(1.0_f32, accent),
                );
            }
            painter.rect_filled(
                Rect::from_min_size(egui::pos2(body.left() - 10.0, body.top() + 4.0), egui::vec2(10.0, 14.0)),
                CornerRadius::same(1),
                hud.cyan,
            );
        }
        InstrumentKind::UsbBridge => {
            let chip = Rect::from_center_size(c, egui::vec2(36.0, 28.0));
            painter.rect_filled(chip, CornerRadius::same(2), Color32::from_rgb(12, 28, 20));
            painter.rect_stroke(chip, CornerRadius::same(2), stroke, egui::StrokeKind::Inside);
            for i in 0..4 {
                let y = chip.top() + 5.0 + i as f32 * 6.0;
                painter.line_segment(
                    [egui::pos2(chip.left() - 10.0, y), egui::pos2(chip.left(), y)],
                    Stroke::new(1.1_f32, accent),
                );
                painter.line_segment(
                    [egui::pos2(chip.right(), y), egui::pos2(chip.right() + 10.0, y)],
                    Stroke::new(1.1_f32, accent),
                );
            }
        }
        InstrumentKind::Generic => {
            painter.text(
                c,
                egui::Align2::CENTER_CENTER,
                "SCPI",
                egui::FontId::monospace(12.0),
                hud.cyan,
            );
        }
    }
}

fn paint_mesh_icon(painter: &egui::Painter, rect: Rect, selected: bool) {
    let cyan = Color32::from_rgb(0x4A, 0xE3, 0xFF);
    let dim = Color32::from_rgba_unmultiplied(0x4A, 0xE3, 0xFF, if selected { 200 } else { 110 });
    let nodes = [
        egui::pos2(rect.left() + 8.0, rect.center().y),
        egui::pos2(rect.center().x + 2.0, rect.top() + 6.0),
        egui::pos2(rect.center().x + 2.0, rect.bottom() - 6.0),
        egui::pos2(rect.right() - 6.0, rect.center().y),
    ];
    for (a, b) in [(0, 1), (0, 2), (1, 3), (2, 3)] {
        painter.line_segment([nodes[a], nodes[b]], Stroke::new(1.1_f32, dim));
    }
    for (i, p) in nodes.iter().enumerate() {
        painter.circle_filled(*p, if i == 0 { 3.4 } else { 2.6 }, cyan);
    }
}

fn sanitize_photo_stem(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    trimmed
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect()
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
        Event::CommandDone {
            id,
            job_id,
            response,
        } => {
            bus.publish(
                "instrument.command_done",
                serde_json::json!({ "device_id": id, "job_id": job_id, "response": response }),
                None,
            );
        }
        Event::SessionOutput { id, text } => {
            bus.publish(
                "instrument.output",
                serde_json::json!({ "device_id": id, "text": text }),
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
            id,
            job_id,
            width,
            height,
            ..
        } => {
            bus.publish(
                "instrument.screenshot",
                serde_json::json!({
                    "device_id": id,
                    "job_id": job_id,
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
            job_id,
            bytes,
            suggested_name,
            trace,
            traces,
            channel_files,
            parse_error,
            ..
        } => {
            bus.publish(
                "instrument.waveform_source",
                serde_json::json!({
                    "device_id": id,
                    "job_id": job_id,
                    "bytes": bytes.len(),
                    "suggested_name": suggested_name,
                    "points": trace.as_ref().map(|t| t.x.len()),
                    "channels": channel_files.iter().map(|(ch, _, _)| ch).collect::<Vec<_>>(),
                    "trace_count": traces.len(),
                    "parse_error": parse_error,
                }),
                None,
            );
        }
        Event::Progress { id, message } => {
            bus.publish(
                "instrument.progress",
                serde_json::json!({ "device_id": id, "message": message }),
                None,
            );
        }
        Event::Error { id, job_id, message } => {
            bus.publish(
                "instrument.error",
                serde_json::json!({ "device_id": id, "job_id": job_id, "message": message }),
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
