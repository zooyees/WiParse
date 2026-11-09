//! Unified VISA instrument workbench.

mod workbench;
mod waveform_pipeline;
use waveform_pipeline::{PipelineJob, SourceRequest};

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
use wiparse_core::device_session::DeviceSession as LiveSession;
use wiparse_core::config::AppConfig;
use wiparse_core::dongle;
use wiparse_core::i18n::Lang;
use wiparse_core::instrument::{
    discover_resources_with_known_sessions, export_csv, humanize_scope_reading_text,
    parse_control_command, AcquisitionBuffer, Capabilities, ControlCommand, Identity,
    InstrumentDevice, InstrumentKind, MeasureFunction, Reading, ResourceInfo, Sample,
    ScopeMeasType, WaveformTrace,
};
use wiparse_core::usb::{is_usb_session_address, parse_usb_resource};
use wiparse_core::wave_display::{build_overview_envelope, envelope_bounds, ScopeEnvelopeColumn};
use wiparse_core::waveform_file::{
    host_waveform_metrics, join_tek_isf_channels, load_waveform_bytes, load_waveform_bytes_all,
    save_waveform_file, sniff_waveform_ext, waveforms_to_spreadsheet_csv,
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
    LoadSetpoint {id:u64,mode:String,value:f64,job_id:u64},
    ProbeOperation{id:u64,command:ControlCommand,target:String,sha256:Option<String>,staged:Option<PathBuf>,job_id:u64},
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
    SamplesExported {id:u64,job_id:u64,result:Result<usize,String>},
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
        plot: CachedWavePlot,
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
        plot: Option<CachedWavePlot>,
        metrics: serde_json::Value,
        channels: Vec<u8>,
        /// All displayed channels (for multi-file / multi-column save).
        traces: Arc<Vec<WaveformTrace>>,
        /// Per-channel native blobs `(CHx, bytes, ext)`.
        channel_files: Vec<(u8, Vec<u8>, String)>,
        parse_error: Option<String>,
    },
    WaveformSaved {
        id: u64,
        job_id: Option<u64>,
        result: Result<Option<Vec<PathBuf>>, String>,
        bytes: usize,
        metrics: serde_json::Value,
        update_save_dir: bool,
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
    traces: Arc<Vec<WaveformTrace>>,
    /// Per-channel native blobs `(channel, bytes, ext)`.
    channel_files: Vec<(u8, Vec<u8>, String)>,
    /// Measured metrics from parsed traces. Empty object when parse failed.
    metrics: serde_json::Value,
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

/// Query history must survive notification drains without cloning point arrays
/// or serializing a potentially large reply on the GUI thread.
fn job_detail_summary(value:&serde_json::Value,budget:&mut usize,depth:usize)->serde_json::Value {
    use serde_json::Value;
    if *budget==0 || depth>6 {return Value::String("[truncated]".into());}
    match value {
        Value::String(text)=>{
            let mut end=text.len().min(*budget).min(4096);
            while !text.is_char_boundary(end){end-=1;}
            *budget=budget.saturating_sub(end);
            if end<text.len(){Value::String(format!("{} [truncated]",&text[..end]))}else{Value::String(text.clone())}
        }
        Value::Array(items)=>{
            let mut result=Vec::new();
            for item in items.iter().take(64) {if *budget==0{break;}result.push(job_detail_summary(item,budget,depth+1));}
            if result.len()<items.len(){result.push(serde_json::json!({"truncated":true,"original_items":items.len()}));}
            Value::Array(result)
        }
        Value::Object(fields)=>{
            let mut result=serde_json::Map::new();
            for (key,item) in fields.iter().take(64){if *budget==0{break;}*budget=budget.saturating_sub(key.len());result.insert(key.clone(),job_detail_summary(item,budget,depth+1));}
            if result.len()<fields.len(){result.insert("_truncated_fields".into(),serde_json::json!(fields.len()-result.len()));}
            Value::Object(result)
        }
        _=>{*budget=budget.saturating_sub(16);value.clone()}
    }
}

#[cfg(test)]
mod ai_job_history_tests {
    use super::*;
    #[test]
    fn demo_screenshot_passes_the_real_image_decoder(){
        let mut device=InstrumentDevice::connect_demo(InstrumentKind::Oscilloscope).unwrap();
        let bytes=device.capture_scope_png().unwrap();
        let image=image::load_from_memory(&bytes).expect("demo PNG must pass CRC and pixel decoding");
        assert_eq!((image.width(),image.height()),(1,1));
    }
    #[test]
    fn completion_query_survives_test_notification_drain_and_stays_bounded(){
        let mut panel=InstrumentControlPanel::new(&AppConfig::default());
        panel.complete_job(Some(77),true,"command",Some(1),serde_json::json!({"value":100000,"unit":"Hz"}));
        assert_eq!(panel.take_job_results().len(),1);
        for _ in 0..2 {let reply=panel.api_job_get(&serde_json::json!({"job_id":77}));assert_eq!(reply.data["state"],"completed");assert_eq!(reply.data["detail"]["value"],100000);}
        let huge="x".repeat(1_000_000);
        panel.complete_job(Some(78),true,"command",Some(1),serde_json::json!({"payload":huge}));
        assert_eq!(panel.take_job_results()[0].detail["payload"].as_str().unwrap().len(),1_000_000);
        assert!(panel.api_job_get(&serde_json::json!({"job_id":78})).data.to_string().len()<20_000);
        for id in 100..700 {panel.complete_job(Some(id),true,"command",Some(1),serde_json::json!({}));}
        assert_eq!(panel.completed_jobs.len(),512);
        assert_eq!(panel.api_job_get(&serde_json::json!({"job_id":77})).data["state"],"pending_or_unknown");
        panel.on_exit();
    }
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
/// Gap between columns inside a row.
/// Uniform action-button size for scope toolbar / channel actions.
const SCOPE_BTN: egui::Vec2 = egui::vec2(72.0, 28.0);
const SCOPE_BTN_WIDE: egui::Vec2 = egui::vec2(88.0, 28.0);
const SCOPE_POS_STEP: f64 = 0.25;
/// Max independent DC source channels in the UI (DP832=3, modular up to 4).
const MAX_SOURCE_CHANNELS: usize = 4;

#[derive(Debug)]
struct ControlState {
    scope_channel: u8,
    scope_setup_page: u8,
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
    source_focus: u8,
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
    probe_page: u8,
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
    dongle_tab: u8,
    dongle_host: String,
    dongle_uart_port: u16,
    dongle_i2c_port: u16,
    dongle_uart_com: String,
    dongle_uart_baud: u32,
    dongle_uart_hex: bool,
    dongle_uart_tx: String,
    dongle_uart_listen: bool,
    dongle_i2c_addr: String,
    dongle_i2c_hz: u32,
    dongle_i2c_timeout_ms: u32,
    dongle_i2c_write_hex: String,
    dongle_i2c_read_len: u32,
}

impl Default for ControlState {
    fn default() -> Self {
        Self {
            scope_channel: 1,
            scope_setup_page: 0,
            scope_channel_on: [true, false, false, false],
            scope_scales: [1.0, 1.0, 1.0, 1.0],
            scope_positions: [0.0, 0.0, 0.0, 0.0],
            scope_meas_types: [ScopeMeasType::Frequency; 4],
            scope_meas_results: [None, None, None, None],
            scope_timebase: 0.001,
            trigger_source: "CH1".into(),
            trigger_level: 0.0,
            trigger_slope: "RISE".into(),
            source_focus: 1,
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
            probe_page: 0,
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
            dongle_tab: 0,
            dongle_host: dongle::DEFAULT_IPV4.into(),
            dongle_uart_port: dongle::UART_TCP_PORT,
            dongle_i2c_port: dongle::I2C_TCP_PORT,
            dongle_uart_com: String::new(),
            dongle_uart_baud: dongle::UART_BAUD_DEFAULT,
            dongle_uart_hex: true,
            dongle_uart_tx: String::new(),
            dongle_uart_listen: false,
            dongle_i2c_addr: "50".into(),
            dongle_i2c_hz: dongle::I2C_SPEED_DEFAULT,
            dongle_i2c_timeout_ms: dongle::I2C_TIMEOUT_DEFAULT_MS,
            dongle_i2c_write_hex: String::new(),
            dongle_i2c_read_len: 1,
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
    last_error: Option<String>,
}

pub struct InstrumentControlPanel {
    tx: Sender<Job>,
    rx: Receiver<Event>,
    events: Sender<Event>,
    resources: Vec<ResourceInfo>,
    resource_inputs: [String; 7],
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
    pending_exports: HashSet<u64>,
    latest: HashMap<(u64, String), Reading>,
    waveforms: HashMap<u64, WaveformTrace>,
    waveform_pipeline: Sender<PipelineJob>,
    waveform_source_job: Option<u64>,
    wave_plots: HashMap<u64, CachedWavePlot>,
    screenshots: HashMap<u64, egui::TextureHandle>,
    /// Full PNG bytes for Save As (populated by Screenshot capture).
    screenshot_png: HashMap<u64, Vec<u8>>,
    /// Skip heavy waveform/image widgets for N frames after selecting the scope card.
    scope_heavy_defer_frames: u8,
    logs: VecDeque<String>,
    scanning: bool,
    scan_error: Option<String>,
    pending_connections: HashMap<u64, String>,
    pending_disconnections: HashSet<u64>,
    pending_scope_actions: HashMap<u64, u64>,
    pending_source_actions: HashMap<u64, u64>,
    pending_screenshots: HashMap<u64, u64>,
    completed_screenshots: HashMap<u64, u64>,
    connection_errors: HashMap<String, String>,
    /// Left-rail "control mesh" card — shows every live session on the right.
    overview_open: bool,
    /// Optional device photos loaded from `{save_dir}/device_photos/`.
    device_photos: HashMap<u64, egui::TextureHandle>,
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
    completed_jobs: VecDeque<InstrumentJobResult>,
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
    pub fn api_operations_pending(&self)->bool {
        !self.pending_exports.is_empty() || self.scanning || self.busy_device.is_some() || !self.pending_connections.is_empty() || !self.pending_disconnections.is_empty() || !self.pending_scope_actions.is_empty() || !self.pending_source_actions.is_empty() || !self.pending_screenshots.is_empty() || !self.measurement_pending.is_empty() || self.devices.iter().any(|device|device.acquiring && !device.paused)
    }

    pub fn new(cfg: &AppConfig) -> Self {
        let (tx, jobs) = unbounded();
        let (events, rx) = unbounded();
        let (waveform_pipeline, pipeline_jobs) = crossbeam_channel::bounded(2);
        let pipeline_events = events.clone();
        thread::spawn(move || waveform_pipeline::worker(pipeline_jobs, pipeline_events));
        let worker_pipeline = waveform_pipeline.clone();
        let background_events=events.clone();
        thread::spawn(move || worker_loop(jobs, events, worker_pipeline));
        let instrument_cfg = &cfg.apps.instruments;
        let mut resource_inputs: [String; 7] = std::array::from_fn(|_| String::new());
        if let Some(first) = instrument_cfg.known_tcpip_resources.first() {
            // Seed only until a scan classifies and redistributes devices.
            resource_inputs[0] = first.clone();
        }
        let panel = Self {
            tx,
            rx,
            events:background_events,
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
            pending_exports: HashSet::new(),
            latest: HashMap::new(),
            waveforms: HashMap::new(),
            wave_plots: HashMap::new(),
            waveform_pipeline,
            waveform_source_job: None,
            screenshots: HashMap::new(),
            screenshot_png: HashMap::new(),
            scope_heavy_defer_frames: 0,
            logs: VecDeque::new(),
            scanning: false,
            scan_error: None,
            pending_connections: HashMap::new(),
            pending_disconnections: HashSet::new(),
            pending_scope_actions: HashMap::new(),
            pending_source_actions: HashMap::new(),
            pending_screenshots: HashMap::new(),
            completed_screenshots: HashMap::new(),
            connection_errors: HashMap::new(),
            overview_open: false,
            device_photos: HashMap::new(),
            debug_mode: false,
            busy_device: None,
            busy_started: None,
            busy_label: String::new(),
            pending_waveform_save: None,
            pending_save_delay_frames: 0,
            next_job_id: 1,
            job_results: Vec::new(),
            completed_jobs: VecDeque::new(),
            last_rtt: Instant::now(),
            rtt_pending: HashSet::new(),
        };
        let _ = std::fs::create_dir_all(&panel.save_dir);
        panel
    }

    /// Enable/disable debug workbench: all cards, demo sessions, and 设备总览.
    pub fn apply_debug_mode(&mut self, enabled: bool, lang: Lang) {
        if self.debug_mode == enabled {
            return;
        }
        self.debug_mode = enabled;
        if enabled {
            self.ensure_demo_devices(lang);
            self.status = text(
                lang,
                "调试模式：已显示全部仪表卡片与设备总览（演示连接）",
                "Debug mode: all instrument cards and Control Mesh (demo sessions)",
            )
            .into();
        } else {
            self.disconnect_demo_devices();
            self.close_overview();
            self.status = text(lang, "调试模式已关闭", "Debug mode off").into();
        }
    }

    fn show_overview(&self) -> bool {
        self.overview_open
    }

    fn close_overview(&mut self) {
        if !self.overview_open {
            return;
        }
        self.select_kind(self.selected_kind);
    }

    fn ensure_demo_devices(&mut self, lang: Lang) {
        for kind in [
            InstrumentKind::Oscilloscope,
            InstrumentKind::DcSource,
            InstrumentKind::ElectronicLoad,
            InstrumentKind::Multimeter,
            InstrumentKind::DebugProbe,
            InstrumentKind::UsbBridge,
            InstrumentKind::WiparseDongle,
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
        const ALL: [InstrumentKind; 7] = [
            InstrumentKind::Oscilloscope,
            InstrumentKind::DcSource,
            InstrumentKind::ElectronicLoad,
            InstrumentKind::Multimeter,
            InstrumentKind::DebugProbe,
            InstrumentKind::UsbBridge,
            InstrumentKind::WiparseDongle,
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
        let frame_budget = Instant::now();
        while frame_budget.elapsed() < Duration::from_millis(4) {
            let Ok(event) = self.rx.try_recv() else { break; };
            if let Some(bus) = bus {
                publish_instrument_event(bus, &event);
            }
            self.handle_event(ctx, event);
        }
        if !self.rx.is_empty() { ctx.request_repaint(); }
        if self.waveform_source_job.is_some() || !self.pending_exports.is_empty() {
            // A different instrument's command may clear the generic busy label.
            // Keep draining this operation independently until save/cancel/error.
            ctx.request_repaint_after(Duration::from_millis(100));
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
        // Schedule background Save As after the preview has had time to paint.
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
                    matches!(
                        device.kind,
                        InstrumentKind::DebugProbe | InstrumentKind::WiparseDongle
                    ) && (device.controls.probe_rtt_on
                        || (device.kind == InstrumentKind::WiparseDongle
                            && device.controls.dongle_uart_listen))
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
        let Some(pending) = self.pending_waveform_save.take() else { return; };
        self.queue_waveform_save(pending, waveform_pipeline::SaveSelection::Dialog { dir: self.save_dir.clone() });
    }

    fn auto_save_waveform_source(&mut self, pending: PendingWaveformSave, dir: &std::path::Path, filename: Option<&str>, overwrite: bool) {
        self.queue_waveform_save(pending, waveform_pipeline::SaveSelection::Auto {
            dir: dir.to_path_buf(), filename: filename.map(str::to_owned), overwrite,
        });
    }

    fn queue_waveform_save(&mut self, pending: PendingWaveformSave, selection: waveform_pipeline::SaveSelection) {
        let id = pending.id;
        let job_id = pending.job_id;
        self.begin_busy(id, "Saving waveform source / 正在保存波形源文件");
        if self.waveform_pipeline.try_send(PipelineJob::Save { pending, selection }).is_err() {
            self.clear_busy();
            self.status = "Waveform save queue is busy".into();
            self.complete_job(job_id, false, "waveform_source", Some(id), serde_json::json!({ "error": self.status }));
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
        if self.waveform_source_job == Some(job_id) {
            self.waveform_source_job = None;
            if self.busy_device == device_id {
                self.clear_busy();
            }
        }
        let mut detail_budget=16*1024;
        self.completed_jobs.push_back(InstrumentJobResult {
            job_id,ok,kind:kind.into(),device_id,
            detail: job_detail_summary(&detail,&mut detail_budget,0),
        });
        while self.completed_jobs.len()>512 {self.completed_jobs.pop_front();}
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
        self.show_overview()
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
                .is_some_and(|k| {
                    k.eq_ignore_ascii_case("overview") || k.eq_ignore_ascii_case("mesh")
                });
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
                return err("ui.instrument.select", &format!("unknown kind '{s}'"));
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
                    "connecting": self.pending_connections.values().any(|address| address == &r.address),
                    "connection_error": self.connection_errors.get(&r.address),
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
            "scan_error": self.scan_error,
            "resources": resources,
            "overview_open": self.show_overview(),
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
            "simulated":d.resource.starts_with("DEMO::")||d.identity.firmware.eq_ignore_ascii_case("demo"),
            "capabilities": d.capabilities,
            "acquiring": d.acquiring,
            "paused": d.paused,
            "live": d.acquiring && !d.paused,
            "last_activity": d.last_activity,
            "disconnecting": self.pending_disconnections.contains(&d.id),
            "measurement_pending": self.measurement_pending.contains(&d.id),
            "scope_action_pending": self.pending_scope_actions.contains_key(&d.id),
            "source_action_pending": self.pending_source_actions.contains_key(&d.id),
            "screenshot_pending": self.pending_screenshots.contains_key(&d.id),
            "last_error": d.last_error,
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
        self.scan_error = None;
        match self.tx.send(Job::Scan {
            library: self.visa_library.clone(),
            timeout_ms: self.timeout_ms,
        }) {
            Ok(()) => ok("instrument.scan", serde_json::json!({ "accepted": true })),
            Err(e) => { self.scanning = false; self.scan_error = Some(e.to_string()); err("instrument.scan", &e.to_string()) },
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

    pub fn api_connect_discovered_safe(&mut self, params:&serde_json::Value, lang:Lang)->crate::backend::InvokeReply {
        use crate::backend::invoke_err as err;
        let address=params.get("resource").and_then(|v|v.as_str()).unwrap_or("");
        if self.scanning {return err("ui.instrument.connect_safe","INSTRUMENT.SCAN_IN_PROGRESS");}
        if self.scan_error.is_some(){return err("ui.instrument.connect_safe","INSTRUMENT.SCAN_FAILED");}
        let Some(kind)=self.resources.iter().find(|r|r.address==address).map(|r|r.kind) else {return err("ui.instrument.connect_safe","INSTRUMENT.RESOURCE_CHANGED");};
        if self.devices.iter().any(|d|d.resource==address)||self.pending_connections.values().any(|r|r==address){return err("ui.instrument.connect_safe","INSTRUMENT.ALREADY_CONNECTED_OR_PENDING");}
        let id=self.next_id;
        let reply=self.api_connect(&serde_json::json!({"resource":address,"kind":kind}),lang);
        if reply.ok {self.pending_connections.insert(id,address.to_owned());self.connection_errors.remove(address);}
        reply
    }

    pub fn api_connect_manual_safe(&mut self, params:&serde_json::Value, lang:Lang)->crate::backend::InvokeReply {
        use crate::backend::invoke_err as err;
        let method="ui.instrument.connect_manual_safe";
        let address=params["resource"].as_str().unwrap_or("").trim();
        if address.is_empty()||address.len()>1024||address.chars().any(char::is_control){return err(method,"INSTRUMENT.INVALID_REQUEST");}
        let kind=match params.get("kind").filter(|value|!value.is_null()) {
            Some(value)=>match serde_json::from_value::<InstrumentKind>(value.clone()){Ok(kind)=>Some(kind),Err(_)=>return err(method,"INSTRUMENT.INVALID_REQUEST")},
            None=>None,
        };
        if self.scanning{return err(method,"INSTRUMENT.SCAN_IN_PROGRESS");}
        if self.devices.iter().any(|device|device.resource.eq_ignore_ascii_case(address))||self.pending_connections.values().any(|resource|resource.eq_ignore_ascii_case(address)){return err(method,"INSTRUMENT.ALREADY_CONNECTED_OR_PENDING");}
        let id=self.next_id;
        let reply=self.api_connect(&serde_json::json!({"resource":address,"kind":kind}),lang);
        if reply.ok {
            self.pending_connections.insert(id,address.to_owned());self.connection_errors.remove(address);
            if !self.resources.iter().any(|resource|resource.address==address){self.resources.push(ResourceInfo{address:address.into(),transport:"Manual".into(),kind,identity:None,probe_error:None});}
        }
        reply
    }

    pub fn api_connect_all(&mut self, lang: Lang) -> crate::backend::InvokeReply {
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

    pub fn api_disconnect_safe(&mut self,params:&serde_json::Value)->crate::backend::InvokeReply {
        use crate::backend::invoke_err as err;
        let Some(id)=params.get("device_id").and_then(|v|v.as_u64()) else{return err("ui.instrument.disconnect_safe","INSTRUMENT.INVALID_REQUEST")};
        let address=params.get("resource").and_then(|v|v.as_str()).unwrap_or("");
        if !self.devices.iter().any(|d|d.id==id&&d.resource==address){return err("ui.instrument.disconnect_safe","INSTRUMENT.SESSION_CHANGED");}
        if self.pending_disconnections.contains(&id){return err("ui.instrument.disconnect_safe","INSTRUMENT.DISCONNECT_PENDING");}
        if self.pending_source_actions.contains_key(&id)||self.pending_scope_actions.contains_key(&id)||self.pending_screenshots.contains_key(&id)||self.measurement_pending.contains(&id)||self.busy_device==Some(id){return err("ui.instrument.disconnect_safe","INSTRUMENT.BUSY");}
        let reply=self.api_disconnect(params);
        if reply.ok {self.pending_disconnections.insert(id);}
        reply
    }

    pub fn api_job_get(&self, params:&serde_json::Value)->crate::backend::InvokeReply {
        let Some(id)=params["job_id"].as_u64() else {return crate::backend::invoke_err("instrument.job.get","missing job_id");};
        let data=self.completed_jobs.iter().find(|r|r.job_id==id).map(|r|serde_json::json!({"state":"completed","job_id":r.job_id,"ok":r.ok,"kind":r.kind,"device_id":r.device_id,"detail":r.detail})).unwrap_or_else(||serde_json::json!({"state":"pending_or_unknown","job_id":id}));
        crate::backend::invoke_ok("instrument.job.get",data)
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

    /// Typed manual controls for the migrated workbench. Session identity and
    /// device-level serialization are checked before entering the old worker.
    pub fn api_acquisition_safe(&mut self,params:&serde_json::Value)->crate::backend::InvokeReply {
        use crate::backend::{invoke_err as err,invoke_ok as ok};let method="ui.instrument.acquisition_safe";
        let Some(id)=params["device_id"].as_u64() else{return err(method,"INSTRUMENT.INVALID_REQUEST")};let address=params["resource"].as_str().unwrap_or("");
        let Some(index)=self.devices.iter().position(|device|device.id==id&&device.resource==address) else{return err(method,"INSTRUMENT.SESSION_CHANGED")};
        let action=params["action"].as_str().unwrap_or("state");
        if action!="state"&&(self.pending_disconnections.contains(&id)||self.pending_source_actions.contains_key(&id)||self.pending_scope_actions.contains_key(&id)||self.busy_device==Some(id)){return err(method,"INSTRUMENT.BUSY");}
        match action {
            "start"|"resume"=>{
                let interval=params["interval_ms"].as_u64().unwrap_or(self.sample_interval_ms);if !(100..=60000).contains(&interval){return err(method,"INSTRUMENT.INVALID_VALUE");}
                self.sample_interval_ms=interval;self.devices[index].acquiring=true;self.devices[index].paused=false;self.last_sample=Instant::now()-Duration::from_millis(interval);
            },
            "pause"=>{self.devices[index].paused=true;},
            "stop"=>{self.devices[index].acquiring=false;self.devices[index].paused=false;},
            "clear"=>self.clear_device_samples(id,address),
            "state"=>{},_=>return err(method,"INSTRUMENT.INVALID_REQUEST"),
        }
        let total=self.samples.iter().filter(|sample|sample.resource==address).count();
        let limit=params["limit"].as_u64().unwrap_or(2000).clamp(1,5000) as usize;
        let start=params["from_row"].as_u64().and_then(|value|usize::try_from(value).ok()).unwrap_or(total.saturating_sub(limit)).min(total);
        let rows=self.samples.iter().filter(|sample|sample.resource==address).skip(start).take(limit).cloned().collect::<Vec<_>>();
        ok(method,serde_json::json!({"device_id":id,"resource":address,"acquiring":self.devices[index].acquiring,"paused":self.devices[index].paused,"interval_ms":self.sample_interval_ms,"total":total,"from_row":start,"rows":rows}))
    }

    pub fn api_export_samples_safe(&mut self,params:&serde_json::Value)->crate::backend::InvokeReply {
        use crate::backend::{invoke_err as err,invoke_ok as ok};let method="ui.instrument.export_samples_safe";
        let Some(id)=params["device_id"].as_u64() else{return err(method,"INSTRUMENT.INVALID_REQUEST")};let address=params["resource"].as_str().unwrap_or("");
        if !self.devices.iter().any(|device|device.id==id&&device.resource==address){return err(method,"INSTRUMENT.SESSION_CHANGED");}
        let Some(path)=params["path"].as_str().filter(|value|!value.is_empty()&&value.len()<=4096&&!value.contains(['\0','\r','\n'])) else{return err(method,"INSTRUMENT.INVALID_REQUEST")};
        let path=PathBuf::from(path);if !path.extension().is_some_and(|value|value.eq_ignore_ascii_case("csv")){return err(method,"INSTRUMENT.INVALID_REQUEST");}
        if self.pending_exports.len()>=2{return err(method,"INSTRUMENT.BUSY");}
        let rows=self.samples.iter().filter(|sample|sample.resource==address).cloned().collect::<Vec<_>>();if rows.is_empty(){return err(method,"INSTRUMENT.NO_SAMPLES");}
        let job=self.alloc_job_id();self.pending_exports.insert(job);let events=self.events.clone();
        thread::spawn(move||{
            let result=(||->Result<usize,String>{
                use std::io::Write;let count=rows.len();let file=std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|_|"目标已存在或不能创建，请选择新文件名".to_string())?;
                let mut writer=std::io::BufWriter::new(file);let result=wiparse_core::instrument::recording::write_csv(&mut writer,rows).and_then(|_|writer.flush()).and_then(|_|writer.get_ref().sync_all());drop(writer);
                if let Err(error)=result{let _=std::fs::remove_file(&path);return Err(error.to_string());}Ok(count)
            })();
            let _=events.send(Event::SamplesExported{id,job_id:job,result});
        });
        ok(method,serde_json::json!({"accepted":true,"device_id":id,"job_id":job}))
    }

    pub fn api_probe_operation(&mut self,params:&serde_json::Value)->crate::backend::InvokeReply{
        use crate::backend::{invoke_err as err,invoke_ok as ok};let method="ui.instrument.probe_operation_safe";
        let Some(id)=params["device_id"].as_u64()else{return err(method,"INSTRUMENT.INVALID_REQUEST")};
        if !self.devices.iter().any(|device|device.id==id&&device.kind==InstrumentKind::DebugProbe&&params["resource"].as_str()==Some(device.resource.as_str())){return err(method,"INSTRUMENT.SESSION_CHANGED");}
        if self.pending_disconnections.contains(&id)||self.pending_source_actions.contains_key(&id)||self.pending_scope_actions.contains_key(&id)||self.busy_device==Some(id)||self.measurement_pending.contains(&id)||self.rtt_pending.contains(&id){return err(method,"INSTRUMENT.BUSY");}
        let Some(target)=params["expected_target"].as_str().filter(|target|!target.trim().is_empty()&&target.len()<=256&&!target.chars().any(char::is_control))else{return err(method,"INSTRUMENT.INVALID_REQUEST")};
        if params["consent"]!=true{return err(method,"INSTRUMENT.INVALID_REQUEST");}
        let (command,staged,checksum)=match params["action"].as_str(){
            Some("erase")=>(ControlCommand::ProbeErase,None,None),
            Some("flash")=>{
                let Some(path)=params["path"].as_str().filter(|path|path.len()<=4096)else{return err(method,"INSTRUMENT.INVALID_STAGING")};
                let Ok(path)=PathBuf::from(path).canonicalize()else{return err(method,"INSTRUMENT.INVALID_STAGING")};
                let Ok(root)=wiparse_core::paths::project_path("cache/probe-staging").canonicalize()else{return err(method,"INSTRUMENT.INVALID_STAGING")};
                if !path.is_file()||path.parent()!=Some(root.as_path())||!path.extension().and_then(|value|value.to_str()).is_some_and(|value|["hex","bin","elf","axf"].contains(&value.to_ascii_lowercase().as_str())){return err(method,"INSTRUMENT.INVALID_STAGING");}
                let Some(hash)=params["sha256"].as_str().filter(|hash|hash.len()==64&&hash.bytes().all(|byte|byte.is_ascii_hexdigit()))else{return err(method,"INSTRUMENT.INVALID_STAGING")};
                if !params["base_address"].is_null()&&params["base_address"].as_u64().is_none(){return err(method,"INSTRUMENT.INVALID_REQUEST");}
                let Some(verify)=params["verify"].as_bool()else{return err(method,"INSTRUMENT.INVALID_REQUEST")};
                let command=ControlCommand::ProbeFlash{path:path.to_string_lossy().into_owned(),verify,base_address:params["base_address"].as_u64()};
                (command,Some(path),Some(hash.to_owned()))
            },_=>return err(method,"INSTRUMENT.INVALID_REQUEST")
        };
        let job=self.alloc_job_id();match self.tx.send(Job::ProbeOperation{id,command,target:target.to_owned(),sha256:checksum,staged,job_id:job}){Ok(())=>{self.pending_source_actions.insert(id,job);ok(method,serde_json::json!({"accepted":true,"device_id":id,"job_id":job}))},Err(error)=>err(method,&error.to_string())}
    }

    pub fn api_io_log(&mut self,method:&str,params:&serde_json::Value)->crate::backend::InvokeReply{
        use std::hash::{Hash,Hasher};use crate::backend::{invoke_err as err,invoke_ok as ok};
        let Some(device)=self.devices.iter_mut().find(|device|params["device_id"].as_u64()==Some(device.id)&&params["resource"].as_str()==Some(device.resource.as_str()))else{return err(method,"INSTRUMENT.SESSION_CHANGED")};
        let revision=|text:&str|{let mut hash=std::collections::hash_map::DefaultHasher::new();text.hash(&mut hash);format!("{:016x}",hash.finish())};
        if method=="ui.instrument.io_log.clear_safe"{if params["revision"].as_str()!=Some(revision(&device.controls.io_log).as_str()){return err(method,"INSTRUMENT.LOG_CHANGED");}device.controls.io_log.clear();}
        ok(method,serde_json::json!({"device_id":device.id,"resource":device.resource,"revision":revision(&device.controls.io_log),"text":device.controls.io_log,"retained_bytes":device.controls.io_log.len()}))
    }

    pub fn api_control_safe(&mut self,params:&serde_json::Value)->crate::backend::InvokeReply {
        use crate::backend::invoke_err as err;
        let method="ui.instrument.control_safe";
        let Some(id)=params["device_id"].as_u64() else{return err(method,"INSTRUMENT.INVALID_REQUEST")};
        let address=params["resource"].as_str().unwrap_or("");
        let Some(device)=self.devices.iter().find(|d|d.id==id&&d.resource==address) else{return err(method,"INSTRUMENT.SESSION_CHANGED")};
        if self.pending_disconnections.contains(&id)||self.pending_source_actions.contains_key(&id)||self.pending_scope_actions.contains_key(&id)||self.pending_screenshots.contains_key(&id)||self.measurement_pending.contains(&id)||self.busy_device==Some(id){return err(method,"INSTRUMENT.BUSY");}
        let command=&params["command"];
        let Some(name)=command.as_str().or_else(||command.as_object().filter(|o|o.len()==1).and_then(|o|o.keys().next().map(String::as_str))) else{return err(method,"INSTRUMENT.INVALID_REQUEST")};
        let allowed=match device.kind {
            InstrumentKind::Oscilloscope=>matches!(name,"ScopeRun"|"ScopeStop"|"ScopeSingle"|"ScopeAutoset"|"ScopeChannel"|"ScopeScale"|"ScopeOffset"|"ScopePosition"|"ScopeMeasure"|"ScopeTimebase"|"ScopeTrigger"),
            InstrumentKind::Multimeter=>matches!(name,"DmmFunction"|"DmmAutoRange"|"DmmRange"|"DmmNplc"),
            InstrumentKind::DebugProbe=>matches!(name,"ProbeStatus"|"ProbeAttach"|"ProbeHalt"|"ProbeRun"|"ProbeReset"|"ProbeMemRead"|"ProbeMemWrite"|"ProbeRttStart"|"ProbeRttStop"|"ProbeRttRead"|"ProbeRegs"|"ProbeSpeed"),
            InstrumentKind::UsbBridge=>matches!(name,"BridgeInfo"|"BridgeSpi"|"BridgeI2c"|"BridgeGpio"),
            InstrumentKind::WiparseDongle=>matches!(name,"DongleInfo"|"DongleSetLink"|"DongleI2cLine"|"DongleI2cScan"|"DongleI2cPing"|"DongleI2cRecover"|"DongleI2cSpeed"|"DongleI2cTimeout"|"DongleI2cWrite"|"DongleI2cRead"|"DongleI2cWriteRead"|"DongleUartOpen"|"DongleUartClose"|"DongleUartWrite"|"DongleUartRead"),
            _=>false,
        } || (matches!(device.kind,InstrumentKind::Oscilloscope|InstrumentKind::Multimeter|InstrumentKind::DcSource|InstrumentKind::ElectronicLoad|InstrumentKind::Generic)&&matches!(name,"RawWrite"|"RawQuery"|"Reset"|"Clear"));
        if !allowed{return err(method,"INSTRUMENT.UNSUPPORTED");}
        if command.get(name).and_then(|fields|fields.get("channel")).and_then(|value|value.as_u64()).is_some_and(|channel|channel==0||channel>u64::from(device.capabilities.channels)){return err(method,"INSTRUMENT.INVALID_CHANNEL");}
        if serde_json::to_vec(command).map_or(true,|data|data.len()>16*1024)||parse_control_command(command).is_err(){return err(method,"INSTRUMENT.INVALID_VALUE");}
        let job=self.next_job_id;let reply=self.api_command(&serde_json::json!({"device_id":id,"command":command}));
        if reply.ok{self.pending_source_actions.insert(id,job);}reply
    }

    pub fn api_scope_action_safe(&mut self,params:&serde_json::Value)->crate::backend::InvokeReply {
        use crate::backend::invoke_err as err;
        let Some(id)=params.get("device_id").and_then(|v|v.as_u64()) else{return err("ui.instrument.scope_action_safe","INSTRUMENT.INVALID_REQUEST")};
        let address=params.get("resource").and_then(|v|v.as_str()).unwrap_or("");
        let Some(device)=self.devices.iter_mut().find(|d|d.id==id&&d.resource==address&&d.kind==InstrumentKind::Oscilloscope) else{return err("ui.instrument.scope_action_safe","INSTRUMENT.SESSION_CHANGED")};
        if self.pending_disconnections.contains(&id)||self.pending_scope_actions.contains_key(&id){return err("ui.instrument.scope_action_safe","INSTRUMENT.BUSY");}
        let command=match params.get("action").and_then(|v|v.as_str()){Some("run")=>"ScopeRun",Some("stop")=>"ScopeStop",Some("single")=>"ScopeSingle",_=>return err("ui.instrument.scope_action_safe","INSTRUMENT.INVALID_REQUEST")};
        device.last_error=None;
        let job=self.next_job_id;
        let reply=self.api_command(&serde_json::json!({"device_id":id,"command":command}));
        if reply.ok {self.pending_scope_actions.insert(id,job);}
        reply
    }

    pub fn api_source_action_safe(&mut self,params:&serde_json::Value)->crate::backend::InvokeReply {
        use crate::backend::invoke_err as err;let method="ui.instrument.source_action_safe";
        let Some(id)=params.get("device_id").and_then(|v|v.as_u64()) else{return err(method,"INSTRUMENT.INVALID_REQUEST")};
        let address=params.get("resource").and_then(|v|v.as_str()).unwrap_or("");
        let Some(device)=self.devices.iter().find(|d|d.id==id&&d.resource==address&&d.kind==InstrumentKind::DcSource) else{return err(method,"INSTRUMENT.SESSION_CHANGED")};
        let Some(channel)=params.get("channel").and_then(|v|v.as_u64()).filter(|ch|*ch>=1&&*ch<=u64::from(device.capabilities.channels.min(4))) else{return err(method,"INSTRUMENT.INVALID_CHANNEL")};
        if self.pending_disconnections.contains(&id)||self.pending_source_actions.contains_key(&id)||self.measurement_pending.contains(&id)||self.busy_device==Some(id){return err(method,"INSTRUMENT.BUSY");}
        let command=match params.get("action").and_then(|v|v.as_str()) {
            Some("voltage")=>{let Some(value)=params.get("value").and_then(|v|v.as_f64()).filter(|v|v.is_finite()&&(0.0..=60.0).contains(v)) else{return err(method,"INSTRUMENT.INVALID_VALUE")};serde_json::json!({"SourceVoltage":{"channel":channel,"value":value}})},
            Some("current")=>{let Some(value)=params.get("value").and_then(|v|v.as_f64()).filter(|v|v.is_finite()&&(0.0..=20.0).contains(v)) else{return err(method,"INSTRUMENT.INVALID_VALUE")};serde_json::json!({"SourceCurrent":{"channel":channel,"value":value}})},
            Some("ovp")=>{if !device.capabilities.source_protection{return err(method,"INSTRUMENT.UNSUPPORTED");}let Some(value)=params.get("value").and_then(|v|v.as_f64()).filter(|v|v.is_finite()&&(0.0..=60.0).contains(v)) else{return err(method,"INSTRUMENT.INVALID_VALUE")};serde_json::json!({"SourceOvp":{"channel":channel,"value":value}})},
            Some("ocp")=>{if !device.capabilities.source_protection{return err(method,"INSTRUMENT.UNSUPPORTED");}let Some(value)=params.get("value").and_then(|v|v.as_f64()).filter(|v|v.is_finite()&&(0.0..=20.0).contains(v)) else{return err(method,"INSTRUMENT.INVALID_VALUE")};serde_json::json!({"SourceOcp":{"channel":channel,"value":value}})},
            Some("output")=>{if !device.capabilities.source_output{return err(method,"INSTRUMENT.UNSUPPORTED");}let Some(enabled)=params.get("enabled").and_then(|v|v.as_bool()) else{return err(method,"INSTRUMENT.INVALID_REQUEST")};serde_json::json!({"SourceOutput":{"channel":channel,"enabled":enabled}})},
            _=>return err(method,"INSTRUMENT.INVALID_REQUEST"),
        };
        let job=self.next_job_id;let reply=self.api_command(&serde_json::json!({"device_id":id,"command":command}));
        if reply.ok{self.pending_source_actions.insert(id,job);}reply
    }

    pub fn api_load_action_safe(&mut self,params:&serde_json::Value)->crate::backend::InvokeReply {
        use crate::backend::{invoke_err as err,invoke_ok as ok};let method="ui.instrument.load_action_safe";
        let Some(id)=params.get("device_id").and_then(|v|v.as_u64()) else{return err(method,"INSTRUMENT.INVALID_REQUEST")};
        let address=params.get("resource").and_then(|v|v.as_str()).unwrap_or("");
        let Some(device)=self.devices.iter().find(|d|d.id==id&&d.resource==address&&d.kind==InstrumentKind::ElectronicLoad) else{return err(method,"INSTRUMENT.SESSION_CHANGED")};
        if self.pending_disconnections.contains(&id)||self.pending_source_actions.contains_key(&id)||self.measurement_pending.contains(&id)||self.busy_device==Some(id){return err(method,"INSTRUMENT.BUSY");}
        match params.get("action").and_then(|v|v.as_str()) {
            Some("setpoint")=>{
                let Some(mode)=params.get("mode").and_then(|v|v.as_str()).filter(|mode|["CC","CV","CR","CP"].contains(mode)&&device.capabilities.load_modes.iter().any(|available|available.eq_ignore_ascii_case(mode))) else{return err(method,"INSTRUMENT.UNSUPPORTED")};
                let Some(value)=params.get("value").and_then(|v|v.as_f64()).filter(|v|v.is_finite()&&*v>=0.0&&*v<=1_000_000.0&&(mode!="CR"||*v>=0.001)) else{return err(method,"INSTRUMENT.INVALID_VALUE")};
                let job=self.alloc_job_id();match self.tx.send(Job::LoadSetpoint{id,mode:mode.to_owned(),value,job_id:job}){
                    Ok(())=>{self.pending_source_actions.insert(id,job);ok(method,serde_json::json!({"accepted":true,"device_id":id,"job_id":job}))},Err(error)=>err(method,&error.to_string()),
                }
            },
            Some("input")=>{let Some(enabled)=params.get("enabled").and_then(|v|v.as_bool()) else{return err(method,"INSTRUMENT.INVALID_REQUEST")};let job=self.next_job_id;let reply=self.api_command(&serde_json::json!({"device_id":id,"command":{"LoadInput":enabled}}));if reply.ok{self.pending_source_actions.insert(id,job);}reply},
            _=>err(method,"INSTRUMENT.INVALID_REQUEST"),
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
            Err(e) => {self.measurement_pending.remove(&id);err("instrument.measure", &e.to_string())},
        }
    }

    pub fn api_measure_safe(&mut self,params:&serde_json::Value)->crate::backend::InvokeReply {
        use crate::backend::invoke_err as err;
        let Some(id)=params.get("device_id").and_then(|v|v.as_u64()) else{return err("ui.instrument.measure_safe","INSTRUMENT.INVALID_REQUEST")};
        let address=params.get("resource").and_then(|v|v.as_str()).unwrap_or("");
        let Some(device)=self.devices.iter_mut().find(|d|d.id==id&&d.resource==address) else{return err("ui.instrument.measure_safe","INSTRUMENT.SESSION_CHANGED")};
        if self.pending_disconnections.contains(&id) || self.measurement_pending.contains(&id)||self.pending_source_actions.contains_key(&id){return err("ui.instrument.measure_safe","INSTRUMENT.BUSY");}
        device.last_error=None;
        self.api_measure(params)
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
        let job_id = if wait {
            Some(self.alloc_job_id())
        } else {
            None
        };
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
            Err(e) => {self.clear_busy();err("instrument.capture", &e.to_string())},
        }
    }

    pub fn api_capture_safe(&mut self,params:&serde_json::Value)->crate::backend::InvokeReply {
        use crate::backend::invoke_err as err;
        let Some(id)=params.get("device_id").and_then(|v|v.as_u64()) else{return err("ui.instrument.capture_safe","INSTRUMENT.INVALID_REQUEST")};
        let address=params.get("resource").and_then(|v|v.as_str()).unwrap_or("");
        let Some(device)=self.devices.iter_mut().find(|d|d.id==id&&d.resource==address&&d.kind==InstrumentKind::Oscilloscope) else{return err("ui.instrument.capture_safe","INSTRUMENT.SESSION_CHANGED")};
        if !device.capabilities.screenshot{return err("ui.instrument.capture_safe","INSTRUMENT.CAPTURE_UNSUPPORTED");}
        if self.busy_device.is_some()||self.pending_disconnections.contains(&id)||!self.pending_screenshots.is_empty(){return err("ui.instrument.capture_safe","INSTRUMENT.BUSY");}
        device.last_error=None;
        let job=self.next_job_id;
        let reply=self.api_capture(&serde_json::json!({"device_id":id,"wait":true}));
        if reply.ok {self.pending_screenshots.insert(id,job);}
        reply
    }

    pub fn api_screenshot_safe(&self,params:&serde_json::Value)->crate::backend::InvokeReply {
        use base64::Engine;
        use crate::backend::{invoke_err as err,invoke_ok as ok};
        let Some(id)=params.get("device_id").and_then(|v|v.as_u64()) else{return err("ui.instrument.screenshot_safe","INSTRUMENT.INVALID_REQUEST")};
        let Some(job)=params.get("job_id").and_then(|v|v.as_u64()) else{return err("ui.instrument.screenshot_safe","INSTRUMENT.INVALID_REQUEST")};
        let address=params.get("resource").and_then(|v|v.as_str()).unwrap_or("");
        if !self.devices.iter().any(|d|d.id==id&&d.resource==address){return err("ui.instrument.screenshot_safe","INSTRUMENT.SESSION_CHANGED");}
        if self.pending_screenshots.get(&id)==Some(&job){return ok("ui.instrument.screenshot_safe",serde_json::json!({"state":"pending"}));}
        if self.completed_screenshots.get(&id)!=Some(&job){
            if let Some(result)=self.completed_jobs.iter().find(|r|r.job_id==job&&r.device_id==Some(id)&&!r.ok){return err("ui.instrument.screenshot_safe",result.detail.get("error").and_then(|v|v.as_str()).unwrap_or("INSTRUMENT.CAPTURE_FAILED"));}
            return err("ui.instrument.screenshot_safe","INSTRUMENT.CAPTURE_STALE");
        }
        let Some(bytes)=self.screenshot_png.get(&id) else{return err("ui.instrument.screenshot_safe","INSTRUMENT.CAPTURE_STALE")};
        if bytes.len()>4*1024*1024{return err("ui.instrument.screenshot_safe","INSTRUMENT.CAPTURE_TOO_LARGE");}
        ok("ui.instrument.screenshot_safe",serde_json::json!({"state":"completed","png_base64":base64::engine::general_purpose::STANDARD.encode(bytes)}))
    }

    pub fn api_waveform_source_safe(&mut self,params:&serde_json::Value)->crate::backend::InvokeReply {
        use crate::backend::invoke_err as err;
        let method="ui.instrument.waveform_source_safe";
        let Some(id)=params.get("device_id").and_then(|v|v.as_u64()) else{return err(method,"INSTRUMENT.INVALID_REQUEST")};
        let address=params.get("resource").and_then(|v|v.as_str()).unwrap_or("");
        let Some(device)=self.devices.iter().find(|d|d.id==id&&d.resource==address&&d.kind==InstrumentKind::Oscilloscope) else{return err(method,"INSTRUMENT.SESSION_CHANGED")};
        if !device.capabilities.waveform{return err(method,"INSTRUMENT.WAVEFORM_UNSUPPORTED");}
        if self.api_operations_pending()||self.waveform_source_job.is_some(){return err(method,"INSTRUMENT.BUSY");}
        let Some(dir)=params.get("dir").and_then(|v|v.as_str()).filter(|s|!s.is_empty()&&s.len()<=4096&&!s.chars().any(char::is_control)) else{return err(method,"INSTRUMENT.INVALID_REQUEST")};
        let path=PathBuf::from(dir);
        let valid_name=path.file_name().and_then(|v|v.to_str()).and_then(|s|s.strip_prefix("WiParse-wave-")).and_then(|id|uuid::Uuid::parse_str(id).ok()).is_some();
        if !valid_name||!path.is_absolute()||!std::fs::symlink_metadata(&path).is_ok_and(|meta|meta.is_dir()&&!meta.file_type().is_symlink())||!std::fs::read_dir(&path).is_ok_and(|mut entries|entries.next().is_none()){return err(method,"INSTRUMENT.INVALID_DIRECTORY");}
        self.api_waveform_source(&serde_json::json!({"device_id":id,"dir":dir,"overwrite":false}))
    }

    pub fn api_waveform_source(
        &mut self,
        params: &serde_json::Value,
    ) -> crate::backend::InvokeReply {
        use crate::backend::{invoke_err as err, invoke_ok as ok};
        if self.waveform_source_job.is_some() {
            return err("instrument.waveform_source", "A waveform source read or save is already in progress");
        }
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
        self.waveform_source_job = job_id;
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
            Err(e) => {
                self.waveform_source_job = None;
                self.clear_busy();
                err("instrument.waveform_source", &e.to_string())
            }
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
            Event::SamplesExported{id,job_id,result}=>{
                self.pending_exports.remove(&job_id);
                match result {Ok(count)=>self.complete_job(Some(job_id),true,"samples_export",Some(id),serde_json::json!({"rows":count})),Err(error)=>self.complete_job(Some(job_id),false,"samples_export",Some(id),serde_json::json!({"error":error}))};
            },
            Event::Resources(resources) => {
                self.scanning = false;
                self.scan_error = None;
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
                self.pending_connections.remove(&id);
                self.connection_errors.remove(&resource);
                self.status = format!("Connected: {}", identity.raw);
                self.log(format!("CONNECT {resource} — {}", identity.raw));
                self.devices.push(DeviceUi {
                    id,
                    resource: resource.clone(),
                    identity,
                    kind,
                    profile,
                    capabilities,
                    controls: {
                        let mut controls = ControlState::default();
                        if kind == InstrumentKind::WiparseDongle {
                            if let Some(spec) = dongle::parse_dongle_or_host(&resource) {
                                controls.dongle_host = spec.host;
                                controls.dongle_uart_port = spec.uart_port;
                                controls.dongle_i2c_port = spec.i2c_port;
                                controls.dongle_uart_com = spec.uart_com;
                            }
                        }
                        controls
                    },
                    acquiring: false,
                    paused: false,
                    last_activity: "LINK UP".into(),
                    last_error: None,
                });
                if self.overview_open {
                    self.selected_id = Some(id);
                } else {
                    self.select_kind(kind);
                    self.selected_id = Some(id);
                }
            }
            Event::Disconnected(id) => {
                self.pending_disconnections.remove(&id);
                self.pending_scope_actions.remove(&id);
                self.pending_source_actions.remove(&id);
                self.pending_screenshots.remove(&id);
                self.completed_screenshots.remove(&id);
                if self.busy_device == Some(id) {
                    self.clear_busy();
                }
                self.devices.retain(|device| device.id != id);
                self.device_photos.remove(&id);
                self.measurement_pending.remove(&id);
                self.latest.retain(|(device_id, _), _| *device_id != id);
                self.waveforms.remove(&id);
                self.wave_plots.remove(&id);
                self.screenshots.remove(&id);
                self.screenshot_png.remove(&id);
                self.rtt_pending.remove(&id);
                self.selected_id =
                    self.selected_id
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
            Event::CommandDone {
                id,
                job_id,
                response,
            } => {
                if self.pending_scope_actions.get(&id).copied()==job_id {self.pending_scope_actions.remove(&id);}
                if self.pending_source_actions.get(&id).copied()==job_id {self.pending_source_actions.remove(&id);}
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
                if job_id.is_some() && self.pending_screenshots.get(&id).copied()==job_id {
                    self.pending_screenshots.remove(&id);
                    if let Some(job)=job_id{self.completed_screenshots.insert(id,job);}
                } else {
                    // A capture from another interface supersedes the bytes.
                    self.completed_screenshots.remove(&id);
                }
                self.clear_busy();
                let color = egui::ColorImage::from_rgba_unmultiplied([width, height], &rgba);
                ctx.copy_image(color.clone());
                self.screenshots.insert(
                    id,
                    ctx.load_texture(format!("instrument-shot-{id}"), color, Default::default()),
                );
                self.screenshot_png.insert(id, png.clone());
                if let Some(device) = self.devices.iter_mut().find(|device| device.id == id) {
                    note_activity(&mut device.last_activity, "SCREEN CAPTURE");
                }
                self.status = "截图已显示并复制到剪贴板 / Screenshot copied to clipboard".into();
                if let Some(path) = save_path {
                    if let Some(parent) = path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    match std::fs::write(&path, &png) {
                        Ok(()) => {
                            self.status =
                                format!("截图已保存 / Screenshot saved: {}", path.display());
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
            Event::Waveform { id, trace, plot } => {
                self.clear_busy();
                self.status = format!("{}: {} points", trace.channel, trace.x.len());
                self.wave_plots.insert(id, plot);
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
                plot,
                metrics,
                channels,
                traces,
                channel_files,
                parse_error,
            } => {
                self.clear_busy();
                if !self.devices.iter().any(|device| device.id == id) {
                    self.complete_job(job_id, false, "waveform_source", Some(id), serde_json::json!({ "error": "device disconnected during waveform processing" }));
                    return;
                }
                let stem = PathBuf::from(&suggested_name);
                let mut effective_ext = stem
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("isf")
                    .to_ascii_lowercase();
                // Preserve native source bytes and every parsed channel for Save As.
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

                let ch_labels: Vec<String> = channels
                    .iter()
                    .map(|ch| format!("CH{ch}"))
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
                    // WaveAxis is Arc-backed: share CH1 samples, retain all channels only until save finishes.
                    if let Some(plot) = plot {
                        self.wave_plots.insert(id, plot);
                    }
                    self.waveforms.insert(id, traces[0].clone());
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
                    traces,
                    channel_files,
                    metrics,
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
            Event::WaveformSaved { id, job_id, result, bytes, metrics, update_save_dir } => {
                match result {
                    Ok(Some(saved)) => {
                        if let Some(path) = saved.first() {
                            if update_save_dir {
                                if let Some(parent) = path.parent() { self.save_dir = parent.to_path_buf(); }
                            }
                            self.status = format!("已保存波形源 / Waveform source saved: {}", path.display());
                            self.log(self.status.clone());
                        }
                        self.complete_job(job_id, true, "waveform_source", Some(id), serde_json::json!({
                            "path": saved.first().map(|p| p.display().to_string()),
                            "files": saved.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
                            "bytes": bytes, "metrics": metrics,
                        }));
                    }
                    Ok(None) => {
                        self.status = "波形已显示，另存为已取消 / Waveform shown; Save As cancelled".into();
                        self.complete_job(job_id, false, "waveform_source", Some(id), serde_json::json!({ "error": "save cancelled" }));
                    }
                    Err(error) => {
                        self.status = error.clone();
                        self.complete_job(job_id, false, "waveform_source", Some(id), serde_json::json!({ "error": error }));
                    }
                }
            }
            Event::Error {
                id,
                job_id,
                message,
            } => {
                if let Some(id)=id {if self.pending_scope_actions.get(&id).copied()==job_id {self.pending_scope_actions.remove(&id);}}
                if let Some(id)=id {if self.pending_source_actions.get(&id).copied()==job_id {self.pending_source_actions.remove(&id);}}
                if let Some(id)=id {if self.pending_screenshots.get(&id).copied()==job_id {self.pending_screenshots.remove(&id);}}
                if let Some(address)=id.and_then(|id|self.pending_connections.remove(&id)){self.connection_errors.insert(address,message.clone());}
                self.clear_busy();
                if id.is_none() {
                    if self.scanning { self.scan_error = Some(message.clone()); }
                    self.scanning = false;
                }
                if let Some(id) = id {
                    if let Some(device) = self.devices.iter_mut().find(|device| device.id == id) {
                        device.last_error = Some(message.clone());
                    }
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
                self.log(if let Some(id) = id {
                    format!("#{id} ERROR {message}")
                } else {
                    format!("ERROR {message}")
                });
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

    /// Session status only. Waveform samples, screenshots and library paths stay local.
    pub(crate) fn ai_summary(&self) -> serde_json::Value {
        let clip = |text: &str, limit: usize| text.chars().take(limit).collect::<String>();
        let sessions: Vec<_> = self
            .devices
            .iter()
            .take(8)
            .map(|device| {
                serde_json::json!({
                    "kind": kind_wire(device.kind),
                    "manufacturer": clip(&device.identity.manufacturer, 80),
                    "model": clip(&device.identity.model, 80),
                    "resource": clip(&device.resource, 80),
                    "acquiring": device.acquiring,
                    "paused": device.paused,
                    "last_error": device.last_error.as_ref().map(|error| clip(error, 160)),
                })
            })
            .collect();
        let recent_log: Vec<String> = self
            .logs
            .iter()
            .rev()
            .take(8)
            .map(|line| clip(line, 160))
            .collect();
        serde_json::json!({
            "status": clip(&self.status, 240),
            "selected_kind": kind_wire(self.selected_kind),
            "session_count": self.devices.len(),
            "scanning": self.scanning,
            "sessions": sessions,
            "recent_log": recent_log,
        })
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        // `pump` is called once from the app update loop while this tab is active.
        let (full, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
        if !full.is_positive() {
            return;
        }
        let gap = theme::PANEL_GAP;
        let width = theme::resizable_sidebar_width(
            ui,
            "instrument_control",
            full,
            theme::SIDEBAR_W,
            theme::SIDEBAR_MIN_W,
            320.0,
            gap,
            tokens,
        );
        let side = egui::Rect::from_min_size(full.min, egui::vec2(width, full.height()));
        let main = egui::Rect::from_min_max(
            egui::pos2((side.right() + gap).min(full.right()), full.top()),
            full.max,
        );
        theme::scope_in_allocated_rect(
            ui,
            egui::UiBuilder::new()
                .max_rect(side)
                .layout(egui::Layout::top_down(egui::Align::Min)),
            |ui| self.device_rack(ui, lang, tokens),
        );
        theme::scope_in_allocated_rect(
            ui,
            egui::UiBuilder::new()
                .max_rect(main)
                .layout(egui::Layout::top_down(egui::Align::Min)),
            |ui| self.workspace(ui, lang, tokens),
        );
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

        let mut assigned = [false; 7];
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

        let identified = self
            .resources
            .iter()
            .filter(|item| item.kind.is_some())
            .count();
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
        if let Some(device) = self.devices.iter().find(|d| d.resource == resource) {
            if device.last_error.is_none() {
                self.status = text(lang, "该地址已连接", "Already connected").into();
                return None;
            }
            // Explicit reconnect after an error must close the old handle first.
            // FIFO ordering prevents overlap between old and replacement sessions.
            let _ = self.tx.send(Job::Disconnect(device.id));
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
            InstrumentKind::DebugProbe | InstrumentKind::UsbBridge | InstrumentKind::WiparseDongle
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
                    && r.kind.is_some_and(|kind| kind != InstrumentKind::Generic)
                    && !self.devices.iter().any(|d| d.resource == r.address)
            })
            .map(|r| (r.kind.unwrap_or(InstrumentKind::Generic), r.address.clone()))
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
                                ui.label(RichText::new(model).small().color(tokens.text_muted));
                            }
                            if let Some(error) = &resource.probe_error {
                                ui.label(
                                    RichText::new(error)
                                        .small()
                                        .color(Color32::from_rgb(0xF5, 0x9E, 0x0B)),
                                );
                            }
                            ui.add_space(3.0);
                            let pair = ui.available_width();
                            let half = ((pair - 6.0) * 0.5).max(40.0);
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                if theme::secondary_btn_sized(
                                    ui,
                                    tokens,
                                    text(lang, "填入", "Fill"),
                                    egui::vec2(half, theme::CTRL_H),
                                )
                                .clicked()
                                {
                                    fill_action = Some((target, resource.address.clone()));
                                }
                                if theme::primary_btn_sized(
                                    ui,
                                    tokens,
                                    text(lang, "连接", "Connect"),
                                    egui::vec2(half, theme::CTRL_H),
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
        self.apple_sidebar(ui, lang, tokens);
    }

    fn workspace(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens) {
        if self.show_overview() {
            self.fleet_overview(ui, lang, tokens);
            return;
        }
        let Some(index) = self.active_device_index() else {
            self.connection_page(ui, lang, tokens);
            return;
        };
        self.selected_id = Some(self.devices[index].id);
        self.detail_header(ui, lang, tokens, index);
        self.industrial_workspace(ui, lang, tokens, index);
    }

    fn dispatch_scope_commands(&mut self, id: u64, commands: Vec<ControlCommand>) {
        for command in commands {
            self.log_command(id, &command);
            let _ = self.tx.send(Job::Command {
                id,
                command,
                job_id: None,
            });
        }
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
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let can_shot = self.devices[index].capabilities.screenshot;
            let has_png = self.screenshot_png.contains_key(&id);
            let busy = self.busy_device == Some(id);
            if ui
                .add_enabled(
                    can_shot && !busy,
                    egui::Button::new(RichText::new(text(lang, "屏幕截图", "Screenshot")).color(tokens.accent_text))
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
                    can_wave_src && !busy && self.waveform_source_job.is_none(),
                    egui::Button::new(text(lang, "读取波形源文件", "Read Wave Source"))
                        .min_size(egui::vec2(128.0, 28.0)),
                )
                .on_hover_text(wave_tip)
                .clicked()
            {
                let job_id = Some(self.alloc_job_id());
                self.waveform_source_job = job_id;
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
                if let Err(error) = self.tx.send(Job::WaveformSource {
                    id,
                    job_id,
                    auto_dir: None,
                    auto_filename: None,
                    overwrite: false,
                }) {
                    self.complete_job(job_id, false, "waveform_source", Some(id), serde_json::json!({ "error": error.to_string() }));
                }
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
        ui.horizontal_wrapped(|ui| {
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
                        default
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .as_ref(),
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
            paint_waveform(
                ui,
                &cached.columns,
                cached.bounds,
                preview_h,
                tokens.accent,
                tokens,
            );
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
            let send_w = 96.0;
            let gap = ui.spacing().item_spacing.x;
            let edit_w = (ui.available_width() - send_w - gap).max(40.0);
            let response = ui.add_sized([edit_w, 32.0],
                egui::TextEdit::singleline(command)
                    .desired_width(edit_w).min_size(egui::vec2(0.0, 32.0))
                    .hint_text("*IDN?"),
            );
            let send = ui
                .add_sized([send_w, 32.0], egui::Button::new(text(lang, "发送", "Send")))
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
            let _ = self.tx.send(Job::Command {
                id,
                command: scpi,
                job_id: None,
            });
        }
        ui.add_space(6.0);

        // Exact leftover viewport so ScrollArea cannot grow the card past its cell.
        let log_h = ui.available_height().max(1.0);
        let log_w = ui.available_width().min(panel_w).max(1.0);
        let (log_rect, _) = ui.allocate_exact_size(egui::vec2(log_w, log_h), egui::Sense::hover());
        crate::theme::scope_in_allocated_rect(
            ui,
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
                        let marker = format!(" #{id} ");
                        let lines: Vec<_> = self
                            .logs
                            .iter()
                            .rev()
                            .filter(|line| line.contains(&marker))
                            .take(80)
                            .collect();
                        if lines.is_empty() {
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
                            for line in lines.into_iter().rev() {
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

    fn device_io_log_ui(&mut self, ui: &mut egui::Ui, lang: Lang, tokens: &Tokens, index: usize) {
        if ui.button(text(lang, "清空记录", "Clear log")).clicked() {
            self.devices[index].controls.io_log.clear();
        }
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
    }

    fn acquisition_ui(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        _tokens: &Tokens,
        id: u64,
        index: usize,
    ) {
        self.acquisition_ui_with_readings(ui, lang, _tokens, id, index, true);
    }

    fn acquisition_ui_with_readings(
        &mut self, ui: &mut egui::Ui, lang: Lang, _tokens: &Tokens,
        id: u64, index: usize, show_readings: bool,
    ) {
        self.acquisition_ui_sized(ui, lang, _tokens, id, index, show_readings, false);
    }

    fn acquisition_ui_sized(
        &mut self, ui: &mut egui::Ui, lang: Lang, _tokens: &Tokens,
        id: u64, index: usize, show_readings: bool, fill_height: bool,
    ) {
        let resource = self.devices[index].resource.clone();
        fn command_button(ui: &mut egui::Ui, t: &Tokens, label: &str, primary: bool, enabled: bool) -> egui::Response {
            let button = egui::Button::new(RichText::new(label).color(if primary { t.accent_text } else { t.text_primary }))
                .fill(if primary { t.accent } else { t.button_bg }).stroke(Stroke::NONE)
                .corner_radius(CornerRadius::same(theme::RADIUS_CTRL)).wrap();
            ui.add_enabled(enabled, |ui: &mut egui::Ui| ui.add_sized([104.0, 32.0], button))
        }
        ui.horizontal(|ui| {
            ui.label(text(lang, "采样周期", "Sample interval"));
            ui.add_sized([112.0, 28.0], egui::DragValue::new(&mut self.sample_interval_ms).range(100..=60_000).suffix(" ms"));
        });
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            let acquiring = self.devices[index].acquiring;
            if !acquiring {
                if command_button(ui, _tokens, text(lang, "开始", "Start"), true, true).clicked() {
                    self.devices[index].acquiring = true;
                    self.devices[index].paused = false;
                    self.last_sample = Instant::now() - Duration::from_millis(self.sample_interval_ms);
                }
            } else {
                let pause_label = if self.devices[index].paused { text(lang, "继续", "Resume") } else { text(lang, "暂停", "Pause") };
                if command_button(ui, _tokens, pause_label, false, true).clicked() {
                    self.devices[index].paused = !self.devices[index].paused;
                    if !self.devices[index].paused { self.last_sample = Instant::now() - Duration::from_millis(self.sample_interval_ms); }
                }
                if command_button(ui, _tokens, text(lang, "停止", "Stop"), false, true).clicked() {
                    self.devices[index].acquiring = false;
                    self.devices[index].paused = false;
                }
            }
            if command_button(ui, _tokens, text(lang, "单次读取", "Read once"), false, true).clicked() {
                self.measurement_pending.insert(id);
                let _ = self.tx.send(Job::Measure(id));
            }
            if command_button(ui, _tokens, text(lang, "清空", "Clear"), false, true).clicked() {
                self.clear_device_samples(id, &resource);
            }
            let has_samples = self.samples.iter().any(|s| s.resource == resource);
            if command_button(ui, _tokens, text(lang, "导出 CSV", "Export CSV"), false, has_samples).clicked() {
                let default_name = format!("instrument_{}.csv", Local::now().format("%Y%m%d_%H%M%S"));
                if let Some(path) = rfd::FileDialog::new().set_directory(&self.save_dir).set_file_name(&default_name).add_filter("CSV", &["csv"]).save_file() {
                    let rows: Vec<_> = self.samples.iter().filter(|s| s.resource == resource).cloned().collect();
                    match export_csv(path, rows) {
                        Ok(()) => self.status = text(lang, "CSV 已导出", "CSV exported").into(),
                        Err(error) => self.status = error.to_string(),
                    }
                }
            }
        });
        ui.add_space(12.0);
        if show_readings {
        let mut readings: Vec<_> = self.latest.iter().filter(|((device_id,_),_)| *device_id == id).collect();
        readings.sort_by_key(|((_, channel),_)| {
            let order = if channel.ends_with("Voltage") { 0 } else if channel.ends_with("Current") { 1 } else if channel.ends_with("Power") { 2 } else { 3 };
            (channel.split_whitespace().next().unwrap_or(""), order, channel.as_str())
        });
        let columns = (((ui.available_width() + 8.0) / 188.0).floor() as usize).clamp(1, 3);
        let tile_width = (ui.available_width() - 8.0 * (columns - 1) as f32) / columns as f32;
        for row in readings.chunks(columns) {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                for ((_, channel), reading) in row {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(tile_width, 76.0), egui::Sense::hover());
                    instrument_grid_cell(ui, rect, _tokens, channel, |ui| {
                        let value = if reading.value.is_finite() { format!("{:.4} {}", reading.value, reading.unit) } else { "—".into() };
                        ui.add(egui::Label::new(RichText::new(&value).monospace().size(20.0)).truncate()).on_hover_text(value);
                    });
                }
            });
        }
        }
        ui.add_space(12.0);
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
            .height(if fill_height { ui.available_height().max(80.0) } else { 260.0 })
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

    fn clear_device_samples(&mut self, id: u64, resource: &str) {
        let keep: Vec<_> = self.samples.iter().filter(|sample| sample.resource != resource).cloned().collect();
        self.samples.clear();
        for sample in keep { self.samples.push(sample); }
        self.latest.retain(|(device_id, _), _| *device_id != id);
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

fn worker_loop(jobs: Receiver<Job>, events: Sender<Event>, pipeline: Sender<PipelineJob>) {
    let mut devices = HashMap::<u64, LiveSession>::new();
    while let Ok(job) = jobs.recv() {
        retire_lost_sessions(&mut devices, &events);
        match job {
            Job::Scan {
                library,
                timeout_ms,
            } => {
                let known = prepare_scan(&mut devices, &events);
                match discover_resources_with_known_sessions(
                    (!library.trim().is_empty()).then_some(library.as_str()),
                    timeout_ms,
                    &known,
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
                }
            }
            Job::Connect {
                id,
                resource,
                kind,
                timeout_ms,
                library,
            } => {
                if devices.values().any(|session| matches!(session,
                    LiveSession::Scpi(device) if device.resource == resource)) {
                    send_error(&events, Some(id), "Resource already connected; disconnect the existing session before reconnecting");
                    continue;
                }
                match LiveSession::open(
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
                }
            }
            Job::ConnectDemo { id, kind } => match LiveSession::demo(kind) {
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
            Job::Command {
                id,
                command,
                job_id,
            } => {
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
            Job::ProbeOperation{id,command,target,sha256,staged,job_id}=>{
                struct OwnedImage(Option<PathBuf>);impl Drop for OwnedImage{fn drop(&mut self){if let Some(path)=self.0.take(){let _=std::fs::remove_file(path);}}}
                let owned=OwnedImage(staged);
                let result=(||->Result<Option<String>,String>{
                    let device=devices.get_mut(&id).ok_or("探针连接已失效")?;
                    #[cfg(windows)]
                    let _immutable=if let Some(path)=&owned.0{use std::os::windows::fs::OpenOptionsExt;Some(std::fs::OpenOptions::new().read(true).share_mode(1).open(path).map_err(|error|error.to_string())?)}else{None};
                    if let (Some(path),Some(hash))=(&owned.0,&sha256){wiparse_core::update::verify_file_sha256(path,hash).map_err(|error|error.to_string())?;}
                    let status=device.execute(ControlCommand::ProbeStatus).map_err(|error|error.to_string())?.ok_or("无法确认探针目标")?;
                    if !status.contains(&format!("target={target} ")){return Err(format!("探针目标不匹配，期望 {target}；实际 {status}，未执行烧录/擦除"));}
                    let _=events.send(Event::Progress{id:Some(id),message:"目标已确认，正在执行烧录/擦除…".into()});
                    device.execute(command).map_err(|error|error.to_string())
                })();
                match result{Ok(response)=>{let _=events.send(Event::CommandDone{id,job_id:Some(job_id),response});},Err(error)=>send_error_job(&events,Some(id),Some(job_id),error)}
            }
            Job::LoadSetpoint{id,mode,value,job_id}=>{
                if let Some(device)=devices.get_mut(&id){
                    match device.set_load_target(mode,value) {
                        Ok(response)=>{let _=events.send(Event::CommandDone{id,job_id:Some(job_id),response});},
                        Err(error)=>send_error_job(&events,Some(id),Some(job_id),error),
                    }
                }else{send_error_job(&events,Some(id),Some(job_id),"device not connected");}
            }
            Job::PollRtt(id) => match devices.get_mut(&id) {
                Some(device) => match device.poll_session_output() {
                    Ok((text,error)) => {
                        if let Some(error)=error {send_error(&events, Some(id), error);}
                        let _ = events.send(Event::SessionOutput { id, text });
                    },
                    Err(error) => send_error(&events, Some(id), error),
                },
                _ => {
                    let _ = events.send(Event::SessionOutput { id, text: None });
                }
            },
            Job::Measure(id) => match devices.get_mut(&id) {
                Some(device) => match device.read_measurements() {
                    Ok(Some((resource,readings))) => {
                        let _ = events.send(Event::Measurements {
                            id,
                            resource,
                            readings,
                        });
                    }
                    Ok(None) => {
                        let _ = events.send(Event::Measurements {id,resource:String::new(),readings:Vec::new()});
                    }
                    Err(error) => send_error(&events, Some(id), error),
                },
                None => send_error(&events, Some(id), "device not connected"),
            },
            Job::Capture {
                id,
                job_id,
                save_path,
            } => {
                let _ = events.send(Event::Progress {
                    id: Some(id),
                    message: "正在截取屏幕… / Capturing screen…".into(),
                });
                if let Some(device) = devices.get_mut(&id) {
                    match device.capture_scope_png() {
                        Ok(Some(png)) => {
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
                        Ok(None) => send_error_job(&events, Some(id), job_id, "device not connected"),
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
                if let Some(device) = devices.get_mut(&id) {
                    match device.capture_scope_waveform_sources() {
                        Ok(Some(parts)) => {
                            let request = SourceRequest { id, job_id, auto_dir, auto_filename, overwrite };
                            if pipeline.try_send(PipelineJob::Source { request, parts }).is_err() {
                                send_error_job(&events, Some(id), job_id, "Waveform processing queue is busy; retry after the current task completes");
                            }
                        }
                        Ok(None) => send_error_job(&events, Some(id), job_id, "device not connected"),
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
                if let Some(device) = devices.get_mut(&id) {
                    match device.read_scope_source_waveform(channel) {
                        Ok(Some(trace)) => {
                            if pipeline.try_send(PipelineJob::Preview { id, trace }).is_err() {
                                send_error(&events, Some(id), "Waveform processing queue is busy");
                            }
                        }
                        Ok(None) => send_error(&events, Some(id), "device not connected"),
                        Err(error) => send_error(&events, Some(id), error),
                    }
                } else {
                    send_error(&events, Some(id), "device not connected");
                }
            }
            Job::Shutdown => break,
        }
        retire_lost_sessions(&mut devices, &events);
    }
}

fn retire_session(devices: &mut HashMap<u64, LiveSession>, events: &Sender<Event>, id: u64, reason: &str) {
    // Drop VISA instrument + owning RM on this worker before reporting it free.
    devices.remove(&id);
    let _ = events.send(Event::Disconnected(id));
    send_error(events, Some(id), format!("连接已释放，请插好 USB 后重新连接 / Session closed; reconnect after USB is ready: {reason}"));
}

fn retire_lost_sessions(devices: &mut HashMap<u64, LiveSession>, events: &Sender<Event>) {
    let lost: Vec<_> = devices.iter().filter_map(|(id, session)| session.connection_lost().then_some(*id)).collect();
    for id in lost {
        retire_session(devices, events, id, "VISA connection lost");
    }
}

fn prepare_scan(devices: &mut HashMap<u64, LiveSession>, events: &Sender<Event>) -> Vec<ResourceInfo> {
    let mut failed = Vec::new();
    let mut known = Vec::new();
    for (id, session) in devices.iter_mut() {
        match session.known_scpi_resource() {
            Ok(Some(resource)) => known.push(resource),
            Ok(None) => {},
            Err(error) => failed.push((*id, error.to_string())),
        }
    }
    for (id, reason) in failed {
        retire_session(devices, events, id, &reason);
    }
    known
}

#[cfg(test)]
fn open_demo_session(kind:InstrumentKind)->Result<LiveSession,wiparse_core::instrument::InstrumentError>{LiveSession::demo(kind)}

fn connected_event(id: u64, session: &LiveSession) -> Event {
    let descriptor=session.descriptor();
    Event::Connected{id,resource:descriptor.resource,identity:descriptor.identity,kind:descriptor.kind,profile:descriptor.profile,capabilities:descriptor.capabilities}
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

    let first_trace = pending.traces.first();

    if ext == pending.effective_ext && !pending.bytes.is_empty() {
        std::fs::write(path, &pending.bytes).map_err(|e| e.to_string())?;
        return Ok(vec![path.to_path_buf()]);
    }

    if pending.traces.len() <= 1 && pending.channel_files.len() <= 1 {
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

    if ext == "isf" || ext == "wfm" {
        // A multi-curve WFM container is unsupported. Preserve the existing
        // fallback to one multi-curve ISF, and never discard channels or relabel CSV.
        let out = path.with_extension("isf");
        if pending.effective_ext == "isf" && !pending.bytes.is_empty() {
            std::fs::write(&out, &pending.bytes).map_err(|e| e.to_string())?;
            return Ok(vec![out]);
        }
        if pending.traces.is_empty() {
            return Err("no parsed multi-channel waveform data to convert".into());
        }
        let encoded = pending.traces.iter()
            .map(wiparse_core::waveform_file::waveform_isf_bytes)
            .collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
        let parts: Vec<_> = encoded.iter().enumerate().map(|(index, bytes)| ((index + 1) as u8, bytes.as_slice())).collect();
        std::fs::write(&out, join_tek_isf_channels(&parts)).map_err(|e| e.to_string())?;
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
    if let Some(s) = wiparse_core::automation::analysis::trace_stats(trace) {
        return WaveformStats {
            count: s.count,
            dt: s.dt,
            min: s.min,
            max: s.max,
            pp: s.max - s.min,
            mean: s.mean,
        };
    }
    // Invalid samples are not promoted to valid automation measurements. The
    // legacy plot remains display-only; the shared analyzer rejects them.
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

fn scope_panel(ui: &mut egui::Ui, tokens: &Tokens, title: &str, add: impl FnOnce(&mut egui::Ui)) {
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
    crate::theme::scope_in_allocated_rect(
        ui,
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
    crate::theme::scope_in_allocated_rect(
        ui,
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
            ui.allocate_ui_with_layout(body, egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_min_size(body);
                ui.set_max_size(body);
                ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
                add(ui);
            });
        },
    );
}

fn media_slot_empty(ui: &mut egui::Ui, height: f32, tokens: &Tokens) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::hover(),
    );
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

fn paint_screenshot(
    ui: &mut egui::Ui,
    texture: &egui::TextureHandle,
    height: f32,
    tokens: &Tokens,
) {
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

fn placeholder_panel(
    ui: &mut egui::Ui,
    height: f32,
    message: &str,
    color: Color32,
    tokens: &Tokens,
) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::hover(),
    );
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


fn format_scope_meas_display(full: &str, meas_type: ScopeMeasType) -> String {
    let payload = full
        .split_once(": ")
        .map(|(_, rest)| rest)
        .unwrap_or(full)
        .trim();
    if payload.is_empty() || payload == "—" {
        return "—".into();
    }
    humanize_scope_reading_text(payload, meas_type)
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

#[cfg(test)]
fn source_channel_controls(
    ui: &mut egui::Ui,
    lang: Lang,
    tokens: &Tokens,
    device: &mut DeviceUi,
) -> Vec<ControlCommand> {
    workbench::controls::source(ui, lang, tokens, device)
}

fn card(ui: &mut egui::Ui, tokens: &Tokens, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    Frame::NONE
        .fill(tokens.surface_bg)
        .stroke(Stroke::new(1.0_f32, tokens.divider))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(RichText::new(title).strong());
            ui.add_space(10.0);
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
            let content_width = ui.available_width().max(1.0);
            ui.set_min_width(content_width);
            ui.set_max_width(content_width);

            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(CARD_ICON_COLUMN_WIDTH, 0.0),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        ui.label(RichText::new(kind_icon(kind)).monospace().strong());
                    },
                );
                ui.vertical(|ui| {
                    ui.set_width(
                        (content_width - CARD_ICON_COLUMN_WIDTH - ui.spacing().item_spacing.x)
                            .max(1.0),
                    );
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
        Some(InstrumentKind::WiparseDongle) => 6,
        Some(InstrumentKind::Generic) => 7,
        None => 8,
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
        InstrumentKind::WiparseDongle => 6,
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
        InstrumentKind::WiparseDongle => text(lang, "WiParse-dongle", "WiParse-dongle"),
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
        InstrumentKind::WiparseDongle => text(
            lang,
            "一块硬件上的 DAP / UART / I2C：USB CMSIS-DAP，WinUSB UART/I2C，TCP 回退 :1234 / :1236。",
            "One board, three channels: USB CMSIS-DAP, WinUSB UART/I2C, TCP fallback :1234 / :1236.",
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
        InstrumentKind::WiparseDongle => text(
            lang,
            "DAP RTT、UART 捕获和 I2C RPC 回复写入右侧日志。",
            "DAP RTT, UART capture and I2C RPC replies go to the log panel.",
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
        InstrumentKind::WiparseDongle => text(
            lang,
            "查看 dongle 序列号、TCP 主机以及 DAP / UART / I2C 参数。",
            "View dongle serial, TCP host, and DAP / UART / I2C parameters.",
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
        InstrumentKind::UsbBridge => {
            text(lang, "SPI / I2C / GPIO 收发", "SPI / I2C / GPIO transfers")
        }
        InstrumentKind::WiparseDongle => text(
            lang,
            "DAP Halt/烧录、UART 透传、I2C SCAN/读写",
            "DAP halt/flash, UART bridge, I2C scan/read/write",
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
            InstrumentKind::DebugProbe | InstrumentKind::UsbBridge | InstrumentKind::WiparseDongle
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
    ui.add(egui::Label::new(value).wrap());
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
        InstrumentKind::WiparseDongle => "DONG",
        InstrumentKind::Generic => "SCPI",
    }
}

fn resource_address_hint(kind: InstrumentKind) -> &'static str {
    match kind {
        InstrumentKind::DebugProbe => "probe://jlink/serial=...",
        InstrumentKind::UsbBridge => "bridge://ft4222/serial=...",
        InstrumentKind::WiparseDongle => "dongle://wiparse/serial=...",
        _ => "USB0::0x0699::...::INSTR",
    }
}

fn resource_empty_prompt(lang: Lang, kind: InstrumentKind) -> &'static str {
    match kind {
        InstrumentKind::DebugProbe => {
            text(lang, "选择或输入探针地址", "Select or enter probe address")
        }
        InstrumentKind::UsbBridge => {
            text(lang, "选择或输入桥地址", "Select or enter bridge address")
        }
        InstrumentKind::WiparseDongle => text(
            lang,
            "选择或输入 dongle 地址",
            "Select or enter dongle address",
        ),
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
        let mut drain = log.len() - 24_576;
        while !log.is_char_boundary(drain){drain+=1;}
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
    Dongle {
        tab: u8,
        host: String,
        uart_baud: u32,
        i2c_hz: u32,
        rtt: bool,
    },
    Generic {
        cmd: String,
    },
}

fn latest_pair(
    latest: &HashMap<(u64, String), Reading>,
    id: u64,
    key: &str,
) -> Option<(f64, String)> {
    latest
        .get(&(id, key.to_string()))
        .map(|r| (r.value, r.unit.clone()))
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
            TwinFace::DcSource { channels: n, rows }
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
        InstrumentKind::WiparseDongle => TwinFace::Dongle {
            tab: device.controls.dongle_tab,
            host: device.controls.dongle_host.clone(),
            uart_baud: device.controls.dongle_uart_baud,
            i2c_hz: device.controls.dongle_i2c_hz,
            rtt: device.controls.probe_rtt_on || device.controls.dongle_uart_listen,
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
        TwinFace::Dongle {
            tab,
            host,
            uart_baud,
            i2c_hz,
            rtt,
        } => serde_json::json!({
            "type": "wiparse_dongle",
            "channel": match *tab { 1 => "uart", 2 => "i2c", _ => "dap" },
            "host": host,
            "uart_baud": uart_baud,
            "i2c_hz": i2c_hz,
            "live": rtt,
        }),
        TwinFace::Generic { cmd } => serde_json::json!({
            "type": "generic",
            "scpi": cmd,
        }),
    }
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
        return (
            text(lang, "保持", "HOLD").into(),
            Color32::from_rgb(0xFF, 0x9F, 0x0A),
        );
    }
    if device.acquiring {
        return (
            text(lang, "采集", "LIVE").into(),
            Color32::from_rgb(0x30, 0xD1, 0x58),
        );
    }
    if device.controls.probe_rtt_on {
        return ("RTT".into(), Color32::from_rgb(0x4A, 0xE3, 0xFF));
    }
    if device.controls.source_outputs.iter().any(|on| *on) {
        return (
            text(lang, "输出开", "OUTPUT").into(),
            Color32::from_rgb(0x30, 0xD1, 0x58),
        );
    }
    if device.controls.load_input {
        return (
            text(lang, "带载", "SINK").into(),
            Color32::from_rgb(0x30, 0xD1, 0x58),
        );
    }
    (
        text(lang, "待机", "STANDBY").into(),
        Color32::from_rgb(0x7A, 0xC8, 0xE0),
    )
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
        InstrumentKind::WiparseDongle => {
            let ch = match device.controls.dongle_tab {
                1 => "UART",
                2 => "I2C",
                _ => "DAP",
            };
            out.push((text(lang, "通道", "CH").into(), ch.into()));
            out.push((
                text(lang, "主机", "HOST").into(),
                device.controls.dongle_host.clone(),
            ));
            if device.controls.dongle_tab == 1 {
                out.push((
                    "UART".into(),
                    format!("{} baud", device.controls.dongle_uart_baud),
                ));
            } else if device.controls.dongle_tab == 2 {
                out.push((
                    "I2C".into(),
                    format!(
                        "0x{} {} Hz",
                        device.controls.dongle_i2c_addr, device.controls.dongle_i2c_hz
                    ),
                ));
            } else {
                out.push((
                    text(lang, "目标", "TGT").into(),
                    if device.controls.probe_chip.trim().is_empty() {
                        "DP / AUTO".into()
                    } else {
                        device.controls.probe_chip.clone()
                    },
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
        Event::Waveform { id, trace, .. } => {
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
            traces,
            metrics,
            channels,
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
                    "points": traces.first().map(|t| t.x.len()),
                    "channels": channels,
                    "trace_count": traces.len(),
                    "parse_error": parse_error,
                    "metrics": metrics,
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
        Event::WaveformSaved { .. } | Event::SamplesExported { .. } => {} // Published through instrument.job_done after UI bookkeeping.
        Event::Error {
            id,
            job_id,
            message,
        } => {
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

#[cfg(test)]
mod layout_audit {
    use super::*;

    struct HotplugTransport {
        detached: Arc<std::sync::atomic::AtomicBool>,
        dropped: Arc<std::sync::atomic::AtomicBool>,
    }

    impl Drop for HotplugTransport {
        fn drop(&mut self) {
            self.dropped.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    impl wiparse_core::instrument::Transport for HotplugTransport {
        fn connection_lost(&self) -> bool {
            self.detached.load(std::sync::atomic::Ordering::SeqCst)
        }
        fn write(&mut self, _: &str) -> Result<(), wiparse_core::instrument::InstrumentError> {
            Ok(())
        }
        fn query(&mut self, _: &str) -> Result<String, wiparse_core::instrument::InstrumentError> {
            if self.connection_lost() {
                Err(wiparse_core::instrument::InstrumentError::Io("USB detached".into()))
            } else {
                Ok("RIGOL,DS1054Z,SN1,1.0".into())
            }
        }
        fn read_raw(&mut self) -> Result<Vec<u8>, wiparse_core::instrument::InstrumentError> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn scan_failure_is_reported_and_a_success_clears_it() {
        let mut panel = InstrumentControlPanel::new(&AppConfig::default());
        let ctx = egui::Context::default();
        assert!(!panel.api_operations_pending());
        panel.scanning = true;
        assert!(panel.api_operations_pending());
        panel.handle_event(&ctx, Event::Error {id:None,job_id:None,message:"VISA unavailable".into()});
        assert!(!panel.api_operations_pending());
        let state = panel.api_list(Lang::En, &serde_json::json!({}));
        assert_eq!(state["scanning"], false);
        assert_eq!(state["scan_error"], "VISA unavailable");
        panel.handle_event(&ctx, Event::Resources(vec![]));
        assert!(panel.api_list(Lang::En, &serde_json::json!({}))["scan_error"].is_null());
    }

    #[test]
    fn manual_connection_validates_and_queues_without_hardware_access() {
        let mut panel=InstrumentControlPanel::new(&AppConfig::default());
        let (tx,rx)=crossbeam_channel::unbounded();panel.tx=tx;
        for params in [serde_json::json!({"resource":"\n"}),serde_json::json!({"resource":"TCPIP0::example::INSTR","kind":"invalid"}),serde_json::json!({"resource":"USB\u{0}invalid"})] {
            assert_eq!(panel.api_connect_manual_safe(&params,Lang::En).error.as_deref(),Some("INSTRUMENT.INVALID_REQUEST"));
        }
        assert!(rx.try_recv().is_err());
        let address="TCPIP0::192.0.2.1::INSTR";
        let response=panel.api_connect_manual_safe(&serde_json::json!({"resource":format!(" {address} "),"kind":"oscilloscope"}),Lang::En);
        assert!(response.ok,"{:?}",response.error);
        match rx.try_recv().unwrap(){Job::Connect{resource,kind,..}=>{assert_eq!(resource,address);assert_eq!(kind,Some(InstrumentKind::Oscilloscope));},_=>panic!("unexpected job")}
        let state=panel.api_list(Lang::En,&serde_json::json!({}));
        let resource=state["resources"].as_array().unwrap().iter().find(|resource|resource["address"]==address).unwrap();
        assert_eq!(resource["connecting"],true);
        assert_eq!(panel.api_connect_manual_safe(&serde_json::json!({"resource":address.to_lowercase()}),Lang::En).error.as_deref(),Some("INSTRUMENT.ALREADY_CONNECTED_OR_PENDING"));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn guarded_connection_rejects_unknown_and_duplicate_and_reports_failure() {
        let mut panel=InstrumentControlPanel::new(&AppConfig::default());
        let address="TCPIP0::192.0.2.1::INSTR";
        let unknown=panel.api_connect_discovered_safe(&serde_json::json!({"resource":address}),Lang::En);
        assert_eq!(unknown.error.as_deref(),Some("INSTRUMENT.RESOURCE_CHANGED"));
        panel.resources.push(ResourceInfo {address:address.into(),transport:"TCPIP".into(),kind:None,identity:None,probe_error:None});
        panel.scan_error=Some("scan failed".into());
        let stale=panel.api_connect_discovered_safe(&serde_json::json!({"resource":address}),Lang::En);
        assert_eq!(stale.error.as_deref(),Some("INSTRUMENT.SCAN_FAILED"));
        panel.scan_error=None;
        panel.pending_connections.insert(99,address.into());
        let duplicate=panel.api_connect_discovered_safe(&serde_json::json!({"resource":address}),Lang::En);
        assert_eq!(duplicate.error.as_deref(),Some("INSTRUMENT.ALREADY_CONNECTED_OR_PENDING"));
        panel.handle_event(&egui::Context::default(),Event::Error{id:Some(99),job_id:None,message:"connection refused".into()});
        let state=panel.api_list(Lang::En,&serde_json::json!({}));
        let resource=state["resources"].as_array().unwrap().iter().find(|r|r["address"]==address).unwrap();
        assert_eq!(resource["connecting"],false);assert_eq!(resource["connection_error"],"connection refused");
    }

    #[test]
    fn discovered_demo_session_connects_and_disconnects_asynchronously() {
        let mut panel=InstrumentControlPanel::new(&AppConfig::default());
        let address="DEMO::Oscilloscope";
        panel.resources.push(ResourceInfo {address:address.into(),transport:"DEMO".into(),kind:Some(InstrumentKind::Oscilloscope),identity:None,probe_error:None});
        let reply=panel.api_connect_discovered_safe(&serde_json::json!({"resource":address}),Lang::En);
        assert!(reply.ok);assert!(panel.api_operations_pending());
        let ctx=egui::Context::default();
        let event=panel.rx.recv_timeout(std::time::Duration::from_secs(3)).expect("demo connection event");
        panel.handle_event(&ctx,event);
        let device=panel.devices.iter().find(|d|d.resource==address).expect("connected demo device");
        assert_eq!(device.kind,InstrumentKind::Oscilloscope);
        let id=device.id;
        assert!(!panel.api_operations_pending());
        let device=panel.devices.iter_mut().find(|d|d.id==id).unwrap();
        device.acquiring=true;device.paused=false;
        assert!(panel.api_operations_pending());
        let device=panel.devices.iter_mut().find(|d|d.id==id).unwrap();
        device.acquiring=false;
        assert!(panel.api_scope_action_safe(&serde_json::json!({"device_id":id,"resource":address,"action":"single"})).ok);
        assert!(panel.api_operations_pending());
        let event=panel.rx.recv_timeout(std::time::Duration::from_secs(3)).expect("demo scope action event");
        panel.handle_event(&ctx,event);
        assert!(!panel.api_operations_pending());
        assert!(panel.devices.iter().find(|d|d.id==id).unwrap().last_error.is_none());
        panel.pending_screenshots.insert(id,999);
        let request=serde_json::json!({"device_id":id,"resource":address,"job_id":999});
        assert_eq!(panel.api_screenshot_safe(&request).data["state"],"pending");
        panel.handle_event(&ctx,Event::Screenshot{id,job_id:Some(999),save_path:None,width:1,height:1,rgba:vec![0,0,0,255],png:vec![1,2,3]});
        let screenshot=panel.api_screenshot_safe(&request);
        assert!(screenshot.ok);assert_eq!(screenshot.data["png_base64"],"AQID");
        panel.handle_event(&ctx,Event::Screenshot{id,job_id:None,save_path:None,width:1,height:1,rgba:vec![0,0,0,255],png:vec![4,5,6]});
        assert_eq!(panel.api_screenshot_safe(&request).error.as_deref(),Some("INSTRUMENT.CAPTURE_STALE"));
        assert!(panel.api_disconnect_safe(&serde_json::json!({"device_id":id,"resource":address})).ok);
        assert!(panel.api_operations_pending());
        let event=panel.rx.recv_timeout(std::time::Duration::from_secs(3)).expect("demo disconnection event");
        panel.handle_event(&ctx,event);
        assert!(!panel.devices.iter().any(|d|d.id==id));
        assert!(!panel.api_operations_pending());
    }

    #[test]
    fn guarded_disconnect_rejects_a_stale_session() {
        let mut panel=InstrumentControlPanel::new(&AppConfig::default());
        let response=panel.api_disconnect_safe(&serde_json::json!({"device_id":42,"resource":"TCPIP0::192.0.2.1::INSTR"}));
        assert_eq!(response.error.as_deref(),Some("INSTRUMENT.SESSION_CHANGED"));
        assert!(panel.pending_disconnections.is_empty());
    }

    #[test]
    fn guarded_measurement_rejects_a_stale_session() {
        let mut panel=InstrumentControlPanel::new(&AppConfig::default());
        let response=panel.api_measure_safe(&serde_json::json!({"device_id":42,"resource":"TCPIP0::192.0.2.1::INSTR"}));
        assert_eq!(response.error.as_deref(),Some("INSTRUMENT.SESSION_CHANGED"));
        assert!(panel.measurement_pending.is_empty());
    }

    #[test]
    fn guarded_measurement_uses_core_session_and_updates_the_snapshot() {
        let mut panel=InstrumentControlPanel::new(&AppConfig::default());let ctx=egui::Context::default();let address="DEMO::DC Source";
        panel.resources.push(ResourceInfo{address:address.into(),transport:"DEMO".into(),kind:Some(InstrumentKind::DcSource),identity:None,probe_error:None});
        let connected=panel.api_connect_discovered_safe(&serde_json::json!({"resource":address}),Lang::En);assert!(connected.ok);let id=connected.data["device_id"].as_u64().unwrap();
        let event=panel.rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap();panel.handle_event(&ctx,event);
        assert!(panel.api_measure_safe(&serde_json::json!({"device_id":id,"resource":address})).ok);assert!(panel.measurement_pending.contains(&id));
        let event=panel.rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap();panel.handle_event(&ctx,event);
        assert!(!panel.measurement_pending.contains(&id));assert!(panel.latest.keys().any(|(device,_)|*device==id));
    }

    #[test]
    fn migrated_typed_controls_are_session_bound_serialized_and_complete_on_demo(){
        let mut panel=InstrumentControlPanel::new(&AppConfig::default());let ctx=egui::Context::default();let address="DEMO::Oscilloscope";
        panel.resources.push(ResourceInfo{address:address.into(),transport:"DEMO".into(),kind:Some(InstrumentKind::Oscilloscope),identity:None,probe_error:None});
        let connected=panel.api_connect_discovered_safe(&serde_json::json!({"resource":address}),Lang::En);let id=connected.data["device_id"].as_u64().unwrap();
        let event=panel.rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap();panel.handle_event(&ctx,event);
        let make=|resource:&str,command:serde_json::Value|serde_json::json!({"device_id":id,"resource":resource,"command":command});
        assert_eq!(panel.api_control_safe(&make("wrong",serde_json::json!("ScopeStop"))).error.as_deref(),Some("INSTRUMENT.SESSION_CHANGED"));
        assert_eq!(panel.api_control_safe(&make(address,serde_json::json!("ProbeErase"))).error.as_deref(),Some("INSTRUMENT.UNSUPPORTED"));
        assert_eq!(panel.api_control_safe(&make(address,serde_json::json!({"ScopeChannel":{"channel":99,"enabled":true}}))).error.as_deref(),Some("INSTRUMENT.INVALID_CHANNEL"));
        let accepted=panel.api_control_safe(&make(address,serde_json::json!({"ScopeScale":{"channel":1,"volts_per_div":0.5}})));assert!(accepted.ok,"{:?}",accepted.error);
        assert_eq!(panel.api_control_safe(&make(address,serde_json::json!("ScopeStop"))).error.as_deref(),Some("INSTRUMENT.BUSY"));
        let job=accepted.data["job_id"].as_u64().unwrap();let deadline=std::time::Instant::now()+std::time::Duration::from_secs(3);
        while panel.api_job_get(&serde_json::json!({"job_id":job})).data["state"]!="completed"&&std::time::Instant::now()<deadline {if let Ok(event)=panel.rx.recv_timeout(std::time::Duration::from_millis(100)){panel.handle_event(&ctx,event);}}
        assert_eq!(panel.api_job_get(&serde_json::json!({"job_id":job})).data["ok"],true);assert!(!panel.pending_source_actions.contains_key(&id));panel.on_exit();
    }

    #[test]
    fn guarded_scope_actions_accept_only_known_commands_and_live_sessions() {
        for command in ["ScopeRun","ScopeStop","ScopeSingle"] {
            assert!(parse_control_command(&serde_json::json!(command)).is_ok());
        }
        let mut panel=InstrumentControlPanel::new(&AppConfig::default());
        let response=panel.api_scope_action_safe(&serde_json::json!({"device_id":42,"resource":"TCPIP0::192.0.2.1::INSTR","action":"run"}));
        assert_eq!(response.error.as_deref(),Some("INSTRUMENT.SESSION_CHANGED"));
        assert!(panel.pending_scope_actions.is_empty());
    }

    #[test]
    fn guarded_scope_source_requires_live_session_and_new_empty_directory() {
        let mut panel=InstrumentControlPanel::new(&AppConfig::default());let ctx=egui::Context::default();let address="DEMO::Oscilloscope";
        panel.resources.push(ResourceInfo{address:address.into(),transport:"DEMO".into(),kind:Some(InstrumentKind::Oscilloscope),identity:None,probe_error:None});
        let reply=panel.api_connect_discovered_safe(&serde_json::json!({"resource":address}),Lang::En);assert!(reply.ok);let id=reply.data["device_id"].as_u64().unwrap();
        let event=panel.rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap();panel.handle_event(&ctx,event);
        let dir=std::env::temp_dir().join(format!("WiParse-wave-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&dir).unwrap();
        let request=serde_json::json!({"device_id":id,"resource":address,"dir":dir});
        assert_eq!(panel.api_waveform_source_safe(&serde_json::json!({"device_id":id,"resource":"wrong","dir":dir})).error.as_deref(),Some("INSTRUMENT.SESSION_CHANGED"));
        let marker=dir.join("existing.txt");std::fs::write(&marker,b"preserve").unwrap();
        assert_eq!(panel.api_waveform_source_safe(&request).error.as_deref(),Some("INSTRUMENT.INVALID_DIRECTORY"));std::fs::remove_file(marker).unwrap();
        let accepted=panel.api_waveform_source_safe(&request);assert!(accepted.ok,"{:?}",accepted.error);let job=accepted.data["job_id"].as_u64().unwrap();
        let deadline=std::time::Instant::now()+std::time::Duration::from_secs(10);
        while panel.api_job_get(&serde_json::json!({"job_id":job})).data["state"]!="completed"&&std::time::Instant::now()<deadline {
            if let Ok(event)=panel.rx.recv_timeout(std::time::Duration::from_millis(200)){panel.handle_event(&ctx,event);}
        }
        let result=panel.api_job_get(&serde_json::json!({"job_id":job}));assert_eq!(result.data["ok"],true,"{:?}",result.data);
        let files=result.data["detail"]["files"].as_array().unwrap();assert!(!files.is_empty());
        for file in files {let path=PathBuf::from(file.as_str().unwrap());assert_eq!(path.parent(),Some(dir.as_path()));assert!(path.is_file());std::fs::remove_file(path).unwrap();}
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn guarded_source_commands_validate_and_wait_for_the_matching_job(){
        let mut panel=InstrumentControlPanel::new(&AppConfig::default());let ctx=egui::Context::default();let address="DEMO::DC Source";
        panel.resources.push(ResourceInfo{address:address.into(),transport:"DEMO".into(),kind:Some(InstrumentKind::DcSource),identity:None,probe_error:None});
        let reply=panel.api_connect_discovered_safe(&serde_json::json!({"resource":address}),Lang::En);assert!(reply.ok);let id=reply.data["device_id"].as_u64().unwrap();
        let event=panel.rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap();panel.handle_event(&ctx,event);
        let invalid=panel.api_source_action_safe(&serde_json::json!({"device_id":id,"resource":address,"channel":1,"action":"voltage","value":100}));assert_eq!(invalid.error.as_deref(),Some("INSTRUMENT.INVALID_VALUE"));
        let command=panel.api_source_action_safe(&serde_json::json!({"device_id":id,"resource":address,"channel":1,"action":"voltage","value":5.0}));assert!(command.ok);let job=command.data["job_id"].as_u64().unwrap();assert!(panel.api_operations_pending());
        assert_eq!(panel.api_source_action_safe(&serde_json::json!({"device_id":id,"resource":address,"channel":1,"action":"output","enabled":true})).error.as_deref(),Some("INSTRUMENT.BUSY"));
        let event=panel.rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap();panel.handle_event(&ctx,event);
        let result=panel.api_job_get(&serde_json::json!({"job_id":job}));assert_eq!(result.data["state"],"completed");assert_eq!(result.data["ok"],true);assert!(!panel.api_operations_pending());
        let protection=panel.api_source_action_safe(&serde_json::json!({"device_id":id,"resource":address,"channel":1,"action":"ovp","value":10.0}));assert!(protection.ok);
        let event=panel.rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap();panel.handle_event(&ctx,event);assert_eq!(panel.api_job_get(&serde_json::json!({"job_id":protection.data["job_id"]})).data["state"],"completed");
    }

    #[test]
    fn guarded_load_commands_validate_and_wait_for_the_matching_job() {
        let mut panel=InstrumentControlPanel::new(&AppConfig::default());
        let ctx=egui::Context::default();let address="DEMO::Electronic Load";
        panel.resources.push(ResourceInfo{address:address.into(),transport:"DEMO".into(),kind:Some(InstrumentKind::ElectronicLoad),identity:None,probe_error:None});
        let reply=panel.api_connect_discovered_safe(&serde_json::json!({"resource":address}),Lang::En);
        assert!(reply.ok);let id=reply.data["device_id"].as_u64().unwrap();
        let event=panel.rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap();panel.handle_event(&ctx,event);
        let invalid=panel.api_load_action_safe(&serde_json::json!({"device_id":id,"resource":address,"action":"setpoint","mode":"CC","value":-1}));
        assert_eq!(invalid.error.as_deref(),Some("INSTRUMENT.INVALID_VALUE"));
        let command=panel.api_load_action_safe(&serde_json::json!({"device_id":id,"resource":address,"action":"setpoint","mode":"CC","value":1.0}));
        assert!(command.ok,"{:?}",command.error);let job=command.data["job_id"].as_u64().unwrap();assert!(panel.api_operations_pending());
        assert_eq!(panel.api_load_action_safe(&serde_json::json!({"device_id":id,"resource":address,"action":"input","enabled":true})).error.as_deref(),Some("INSTRUMENT.BUSY"));
        let event=panel.rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap();panel.handle_event(&ctx,event);
        let result=panel.api_job_get(&serde_json::json!({"job_id":job}));assert_eq!(result.data["state"],"completed");assert_eq!(result.data["ok"],true);assert!(!panel.api_operations_pending());
        let out_of_range=panel.api_load_action_safe(&serde_json::json!({"device_id":id,"resource":address,"action":"setpoint","mode":"CC","value":1_000_000.0}));
        assert!(out_of_range.ok);let invalid_job=out_of_range.data["job_id"].as_u64().unwrap();
        let event=panel.rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap();panel.handle_event(&ctx,event);
        let result=panel.api_job_get(&serde_json::json!({"job_id":invalid_job}));
        assert_eq!(result.data["ok"],false);assert!(result.data["detail"]["error"].as_str().unwrap().contains("未改变模式"));assert!(!panel.api_operations_pending());
    }

    #[test]
    fn scope_action_waits_for_its_own_completion_or_error() {
        let mut panel=InstrumentControlPanel::new(&AppConfig::default());
        let ctx=egui::Context::default();
        panel.pending_scope_actions.insert(42,100);
        panel.handle_event(&ctx,Event::CommandDone{id:42,job_id:Some(99),response:None});
        assert_eq!(panel.pending_scope_actions.get(&42),Some(&100));
        panel.handle_event(&ctx,Event::CommandDone{id:42,job_id:Some(100),response:None});
        assert!(!panel.pending_scope_actions.contains_key(&42));
        panel.pending_scope_actions.insert(42,101);
        panel.handle_event(&ctx,Event::Error{id:Some(42),job_id:Some(101),message:"timeout".into()});
        assert!(!panel.pending_scope_actions.contains_key(&42));
    }

    #[test]
    fn usb_hotplug_releases_old_session_and_scan_preserves_healthy_session() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let detached = Arc::new(AtomicBool::new(false));
        let dropped = Arc::new(AtomicBool::new(false));
        let device = InstrumentDevice::from_transport(
            "USB0::test::INSTR",
            Box::new(HotplugTransport { detached: detached.clone(), dropped: dropped.clone() }),
            Some(InstrumentKind::Oscilloscope),
        ).unwrap();
        let mut devices = HashMap::from([(7, LiveSession::Scpi(device))]);
        let (tx, rx) = unbounded();
        let known = prepare_scan(&mut devices, &tx);
        assert_eq!(known.len(), 1);
        assert_eq!(devices.len(), 1);
        assert!(!dropped.load(Ordering::SeqCst));
        assert!(rx.is_empty());

        detached.store(true, Ordering::SeqCst);
        assert!(prepare_scan(&mut devices, &tx).is_empty());
        assert!(devices.is_empty());
        assert!(dropped.load(Ordering::SeqCst));
        assert!(matches!(rx.recv().unwrap(), Event::Disconnected(7)));
        assert!(matches!(rx.recv().unwrap(), Event::Error { id: Some(7), .. }));

        // The same resource can now receive a fresh session; stale queued IDs
        // cannot address that replacement session.
        let replacement = InstrumentDevice::connect_demo(InstrumentKind::Oscilloscope).unwrap();
        devices.insert(8, LiveSession::Scpi(replacement));
        retire_lost_sessions(&mut devices, &tx);
        assert!(devices.contains_key(&8));
        assert!(!devices.contains_key(&7));
        assert!(rx.is_empty());
    }

    #[test]
    fn lost_session_cleanup_does_not_remove_other_instruments() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let detached = Arc::new(AtomicBool::new(false));
        let dropped = Arc::new(AtomicBool::new(false));
        let scope = InstrumentDevice::from_transport("USB0::test::INSTR", Box::new(HotplugTransport {
            detached: detached.clone(), dropped: dropped.clone(),
        }), Some(InstrumentKind::Oscilloscope)).unwrap();
        let mut devices = HashMap::from([
            (1, LiveSession::Scpi(scope)),
            (2, LiveSession::demo(InstrumentKind::DcSource).unwrap()),
        ]);
        let (tx, rx) = unbounded();
        detached.store(true, Ordering::SeqCst);
        retire_lost_sessions(&mut devices, &tx);
        assert!(dropped.load(Ordering::SeqCst));
        assert!(!devices.contains_key(&1));
        assert!(devices.contains_key(&2));
        assert!(matches!(rx.recv().unwrap(), Event::Disconnected(1)));
    }

    #[test]
    fn connected_demo_workspaces_and_overview_render_at_small_sizes() {
        let mut panel = InstrumentControlPanel::new(&AppConfig::default());
        let kinds = [
            InstrumentKind::Oscilloscope,
            InstrumentKind::DcSource,
            InstrumentKind::ElectronicLoad,
            InstrumentKind::Multimeter,
            InstrumentKind::DebugProbe,
            InstrumentKind::UsbBridge,
            InstrumentKind::WiparseDongle,
            InstrumentKind::Generic,
        ];
        let initial = egui::Context::default();
        for (i, kind) in kinds.iter().enumerate() {
            let session = LiveSession::demo(*kind).unwrap();
            panel.handle_event(&initial, connected_event(i as u64 + 1, &session));
        }
        // Presentation fixtures only; every rendered device is explicitly marked simulated.
        for (i, kind) in kinds.iter().enumerate() {
            let id = i as u64 + 1;
            let readings: Vec<(String, f64, &str)> = match kind {
                InstrumentKind::DcSource => (1..=3)
                    .flat_map(|ch| {
                        [
                            (format!("CH{ch} Voltage"), 9.0021, "V"),
                            (format!("CH{ch} Current"), 0.3205, "A"),
                            (format!("CH{ch} Power"), 2.8852, "W"),
                        ]
                    })
                    .collect(),
                InstrumentKind::ElectronicLoad => vec![
                    ("Voltage".into(), 8.9987, "V"),
                    ("Current".into(), 1.0021, "A"),
                    ("Power".into(), 9.0176, "W"),
                ],
                InstrumentKind::Multimeter => vec![("VOLT:DC".into(), 9.0012, "V")],
                _ => Vec::new(),
            };
            for (channel, value, unit) in readings {
                panel.latest.insert(
                    (id, channel.clone()),
                    Reading {
                        channel,
                        value,
                        unit: unit.into(),
                    },
                );
            }
            panel.devices[i].controls.scope_meas_results[0] =
                Some("CH1 Frequency: 150000 Hz".into());
        }
        let mut renders = 0;
        for lang in [Lang::Zh, Lang::En] {
            for light in [false, true] {
                for size in [
                    egui::vec2(640.0, 360.0),
                    egui::vec2(1100.0, 720.0),
                    egui::vec2(1920.0, 1080.0),
                ] {
                    for page in 0..=kinds.len() {
                        if page < kinds.len() {
                            panel.selected_kind = kinds[page];
                            panel.selected_id = Some(page as u64 + 1);
                            panel.overview_open = false;
                        } else {
                            panel.overview_open = true;
                        }
                        for tab in 0..if page == kinds.len() { 1 } else { 3 } {
                            let variants = if tab == 0 && page < kinds.len() {
                                match kinds[page] {
                                    InstrumentKind::Oscilloscope => 2,
                                    InstrumentKind::DebugProbe => 4,
                                    InstrumentKind::UsbBridge => 3,
                                    InstrumentKind::WiparseDongle => 6,
                                    _ => 1,
                                }
                            } else {
                                1
                            };
                            for mode in 0..variants {
                                if page < kinds.len() {
                                    let c = &mut panel.devices[page].controls;
                                    c.probe_page = 0;
                                    c.scope_setup_page = 0;
                                    c.bridge_tab = 0;
                                    c.dongle_tab = 0;
                                    match kinds[page] {
                                        InstrumentKind::Oscilloscope => c.scope_setup_page = mode,
                                        InstrumentKind::DebugProbe => c.probe_page = mode,
                                        InstrumentKind::UsbBridge => c.bridge_tab = mode,
                                        InstrumentKind::WiparseDongle => {
                                            if mode < 4 {
                                                c.probe_page = mode;
                                            } else {
                                                c.dongle_tab = mode - 3;
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                                let ctx = egui::Context::default();
                                crate::fonts::install_cjk_fonts(&ctx);
                                ctx.data_mut(|d| {
                                    d.insert_temp(
                                        egui::Id::new(("instrument-task", page as u64 + 1)),
                                        tab as u8,
                                    )
                                });
                                let tokens = crate::theme::apply_theme(&ctx, light);
                                let mut output = ctx.run(
                                    egui::RawInput {
                                        screen_rect: Some(egui::Rect::from_min_size(
                                            egui::Pos2::ZERO,
                                            size,
                                        )),
                                        ..Default::default()
                                    },
                                    |ctx| {
                                        egui::CentralPanel::default().show(ctx, |ui| {
                                            panel.ui(ui, lang, &tokens);
                                        });
                                    },
                                );
                                for primitive in
                                    ctx.tessellate(output.shapes.clone(), output.pixels_per_point)
                                {
                                    assert!(primitive.clip_rect.is_finite());
                                    if let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive
                                    {
                                        assert!(mesh.is_valid());
                                    }
                                }
                                if matches!(lang, Lang::Zh) && size.x == 1100.0 {
                                    if let Ok(dir) = std::env::var("WIPARSE_UI_AUDIT_OUTPUT") {
                                        // Export settled row heights, while retaining all texture uploads.
                                        for _ in 0..2 {
                                            let next = ctx.run(egui::RawInput {
                                                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
                                                ..Default::default()
                                            }, |ctx| { egui::CentralPanel::default().show(ctx, |ui| { panel.ui(ui,lang,&tokens); }); });
                                            output.textures_delta.append(next.textures_delta);
                                            output.shapes = next.shapes;
                                            output.pixels_per_point = next.pixels_per_point;
                                        }
                                        output.textures_delta.set.retain(|(id,_)| *id != egui::TextureId::Managed(0));
                                        output.textures_delta.set.push((egui::TextureId::Managed(0), egui::epaint::ImageDelta::full(ctx.fonts(|f| f.image()), egui::TextureOptions::LINEAR)));
                                        crate::ui_audit::export_frame(
                                            &ctx,
                                            output,
                                            &PathBuf::from(dir),
                                            &format!(
                                                "instrument-{page}-tab-{tab}-mode-{mode}-{}",
                                                if light { "light" } else { "dark" }
                                            ),
                                            size,
                                        );
                                    }
                                }
                                renders += 1;
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(renders, 432);
        assert!(panel.measurement_pending.is_empty());
        panel.on_exit();
    }

    #[test]
    fn source_setpoints_require_apply_and_do_not_enable_output() {
        let mut panel = InstrumentControlPanel::new(&AppConfig::default());
        let ctx = egui::Context::default();
        crate::fonts::install_cjk_fonts(&ctx);
        let session = LiveSession::demo(InstrumentKind::DcSource).unwrap();
        panel.handle_event(&ctx, connected_event(1, &session));
        panel.devices[0].controls.source_voltages[0] = 9.0;
        panel.devices[0].controls.source_currents[0] = 2.0;
        let tokens = Tokens::dark();
        let mut time = 0.0;
        let mut commands = Vec::new();
        let mut frame = |events| {
            time += 0.1;
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(480.0, 850.0),
                    )),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        commands.extend(source_channel_controls(
                            ui,
                            Lang::Zh,
                            &tokens,
                            &mut panel.devices[0],
                        ));
                    });
                },
            )
        };
        let output = frame(vec![]);
        let pos = output
            .shapes
            .iter()
            .find_map(|shape| {
                if let egui::Shape::Text(t) = &shape.shape {
                    (t.galley.job.text == "应用设定").then_some(t.pos + t.galley.size() * 0.5)
                } else {
                    None
                }
            })
            .unwrap();
        frame(vec![egui::Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            frame(vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::default(),
            }]);
        }
        drop(frame);
        assert_eq!(commands.len(), 2);
        assert!(
            matches!(commands[0], ControlCommand::SourceVoltage {channel: 1, value} if value == 9.0)
        );
        assert!(
            matches!(commands[1], ControlCommand::SourceCurrent {channel: 1, value} if value == 2.0)
        );
        assert!(!panel.devices[0].controls.source_outputs[0]);
        panel.on_exit();
    }
}

#[cfg(test)]
mod io_log_unicode_regression{
    #[test]
    fn mixed_width_text_survives_ring_trimming(){
        let mut log=String::new();super::append_io_log(&mut log,&"a😀".repeat(7000));
        assert!(log.len()<=32768);assert!(log.ends_with("a😀"));assert!(!log.is_empty());
    }
}
