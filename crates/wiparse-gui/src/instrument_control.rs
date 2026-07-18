//! Unified VISA instrument workbench.

use crate::theme::{self, Tokens};
use chrono::Local;
use crossbeam_channel::{unbounded, Receiver, Sender};
use egui::{CornerRadius, Frame, Margin, RichText, Sense, Stroke};
use egui_plot::{Legend, Line, Plot, PlotPoints};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};
use wiparse_core::config::AppConfig;
use wiparse_core::i18n::Lang;
use wiparse_core::instrument::{
    export_csv, list_resources_with_library, AcquisitionBuffer, Capabilities, ControlCommand,
    Identity, InstrumentDevice, InstrumentKind, MeasureFunction, Reading, ResourceInfo, Sample,
    WaveformTrace,
};

enum Job {
    Scan {
        library: String,
    },
    Connect {
        id: u64,
        resource: String,
        kind: Option<InstrumentKind>,
        timeout_ms: u32,
        library: String,
    },
    Disconnect(u64),
    Command(u64, ControlCommand),
    Measure(u64),
    Capture {
        id: u64,
        path: PathBuf,
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
        response: Option<String>,
    },
    Measurements {
        id: u64,
        resource: String,
        readings: Vec<Reading>,
    },
    Screenshot {
        id: u64,
        path: PathBuf,
        png: Vec<u8>,
    },
    Waveform {
        id: u64,
        trace: WaveformTrace,
    },
    Error {
        id: Option<u64>,
        message: String,
    },
}

#[derive(Debug)]
struct ControlState {
    scope_channel: u8,
    scope_enabled: bool,
    scope_scale: f64,
    scope_offset: f64,
    scope_timebase: f64,
    trigger_source: String,
    trigger_level: f64,
    trigger_slope: String,
    source_voltage: f64,
    source_current: f64,
    source_ovp: f64,
    source_ocp: f64,
    source_output: bool,
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
            scope_enabled: true,
            scope_scale: 1.0,
            scope_offset: 0.0,
            scope_timebase: 0.001,
            trigger_source: "CH1".into(),
            trigger_level: 0.0,
            trigger_slope: "RISE".into(),
            source_voltage: 5.0,
            source_current: 1.0,
            source_ovp: 6.0,
            source_ocp: 1.2,
            source_output: false,
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
    screenshots: HashMap<u64, egui::TextureHandle>,
    logs: VecDeque<String>,
    scanning: bool,
}

impl InstrumentControlPanel {
    pub fn new(cfg: &AppConfig) -> Self {
        let (tx, jobs) = unbounded();
        let (events, rx) = unbounded();
        thread::spawn(move || worker_loop(jobs, events));
        let instrument_cfg = &cfg.apps.instruments;
        let default_resource = instrument_cfg
            .known_tcpip_resources
            .first()
            .cloned()
            .unwrap_or_else(|| "TCPIP0::192.168.1.100::INSTR".into());
        let panel = Self {
            tx,
            rx,
            resources: instrument_cfg
                .known_tcpip_resources
                .iter()
                .map(|address| ResourceInfo {
                    address: address.clone(),
                    transport: "TCPIP".into(),
                })
                .collect(),
            resource_inputs: std::array::from_fn(|_| default_resource.clone()),
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
            screenshots: HashMap::new(),
            logs: VecDeque::new(),
            scanning: false,
        };
        let _ = std::fs::create_dir_all(&panel.save_dir);
        panel
    }

    pub fn pump(&mut self, ctx: &egui::Context) {
        while let Ok(event) = self.rx.try_recv() {
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
        if self.live_active() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        if self.scanning {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }

    fn handle_event(&mut self, ctx: &egui::Context, event: Event) {
        match event {
            Event::Resources(resources) => {
                self.scanning = false;
                for resource in resources {
                    if !self
                        .resources
                        .iter()
                        .any(|item| item.address == resource.address)
                    {
                        self.resources.push(resource);
                    }
                }
                self.resources.sort_by(|a, b| a.address.cmp(&b.address));
                self.status = format!("{} resource(s)", self.resources.len());
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
            Event::Screenshot { id, path, png } => match image::load_from_memory(&png) {
                Ok(image) => {
                    let rgba = image.to_rgba8();
                    let size = [rgba.width() as usize, rgba.height() as usize];
                    let color = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                    self.screenshots.insert(
                        id,
                        ctx.load_texture(
                            format!("instrument-shot-{id}"),
                            color,
                            Default::default(),
                        ),
                    );
                    self.status = format!("Saved {}", path.display());
                }
                Err(error) => self.status = format!("Invalid screenshot: {error}"),
            },
            Event::Waveform { id, trace } => {
                self.status = format!("{}: {} points", trace.channel, trace.x.len());
                self.waveforms.insert(id, trace);
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
        self.pump(ui.ctx());
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
                        self.status = text(lang, "正在扫描…", "Scanning…").into();
                        let _ = self.tx.send(Job::Scan {
                            library: self.visa_library.clone(),
                        });
                    }
                    if self.scanning {
                        ui.spinner().on_hover_text(text(
                            lang,
                            "扫描由 VISA Runtime 执行",
                            "Discovery is provided by the VISA runtime",
                        ));
                    }
                });
                ui.label(
                    RichText::new(text(
                        lang,
                        "每张卡片独立选择 VISA 资源并连接",
                        "Select and connect a VISA resource inside each card",
                    ))
                    .small()
                    .color(tokens.text_muted),
                );
                let cards_height = ui.available_height().max(160.0);
                egui::ScrollArea::vertical()
                    .id_salt("instrument-type-cards")
                    .max_height(cards_height)
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        for kind in [
                            InstrumentKind::Oscilloscope,
                            InstrumentKind::DcSource,
                            InstrumentKind::ElectronicLoad,
                            InstrumentKind::Multimeter,
                        ] {
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
                            if action.selected {
                                self.select_kind(kind);
                            }
                            if action.connect {
                                let resource = self.resource_inputs[input_slot].trim().to_owned();
                                if !resource.is_empty() {
                                    let id = self.next_id;
                                    self.next_id += 1;
                                    self.select_kind(kind);
                                    self.status = text(lang, "正在连接…", "Connecting…").into();
                                    let _ = self.tx.send(Job::Connect {
                                        id,
                                        resource,
                                        kind: Some(kind),
                                        timeout_ms: self.timeout_ms,
                                        library: self.visa_library.clone(),
                                    });
                                }
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
        let same_kind_devices: Vec<_> = self
            .devices
            .iter()
            .filter(|device| device.kind == self.selected_kind)
            .map(|device| (device.id, device.identity.model.clone()))
            .collect();
        ui.horizontal(|ui| {
            let device = &self.devices[index];
            ui.heading(format!(
                "{} — {}",
                device.kind.label(),
                device.identity.model
            ));
            ui.label(RichText::new(&device.identity.manufacturer).color(tokens.text_muted));
            if same_kind_devices.len() > 1 {
                egui::ComboBox::from_id_salt("same-kind-device")
                    .selected_text(&device.identity.model)
                    .show_ui(ui, |ui| {
                        for (device_id, model) in &same_kind_devices {
                            ui.selectable_value(&mut self.selected_id, Some(*device_id), model);
                        }
                    });
            }
        });
        ui.separator();

        let kind = self.devices[index].kind;
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
                            ui.label(
                                RichText::new(instrument_settings_hint(lang, kind))
                                    .small()
                                    .color(tokens.text_muted),
                            );
                            ui.add_space(6.0);
                            self.instrument_parameters_ui(ui, lang, tokens, id, index);
                            self.settings_ui(ui, lang, tokens, id, index);
                        },
                    );

                    card(
                        &mut columns[1],
                        tokens,
                        text(lang, "控制", "Control"),
                        |ui| {
                            ui.label(
                                RichText::new(instrument_control_hint(lang, kind))
                                    .small()
                                    .color(tokens.text_muted),
                            );
                            ui.add_space(6.0);
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
                            if kind == InstrumentKind::Oscilloscope {
                                self.scope_acquisition_ui(ui, lang, tokens, id, index);
                            }
                            self.acquisition_ui(ui, lang, tokens, id, index);
                        },
                    );
                });

                card(ui, tokens, "SCPI", |ui| {
                    self.console_ui(ui, lang, tokens, id, index);
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

    fn scope_acquisition_ui(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        _tokens: &Tokens,
        id: u64,
        index: usize,
    ) {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button(text(lang, "读取波形", "Read Waveform")).clicked() {
                let _ = self.tx.send(Job::Waveform {
                    id,
                    channel: self.devices[index].controls.scope_channel,
                    points: self.max_points.min(20_000),
                });
            }
            if self.devices[index].capabilities.screenshot
                && ui
                    .button(text(lang, "保存截图", "Save Screenshot"))
                    .clicked()
            {
                let default = self.save_dir.join(format!(
                    "scope_{}.png",
                    Local::now().format("%Y%m%d_%H%M%S")
                ));
                let path = rfd::FileDialog::new()
                    .set_directory(&self.save_dir)
                    .set_file_name(default.file_name().unwrap_or_default().to_string_lossy())
                    .add_filter("PNG", &["png"])
                    .save_file();
                if let Some(path) = path {
                    let _ = self.tx.send(Job::Capture { id, path });
                }
            }
            if ui.button(text(lang, "单次测量", "Read Once")).clicked() {
                self.measurement_pending.insert(id);
                let _ = self.tx.send(Job::Measure(id));
            }
        });
        if let Some(trace) = self.waveforms.get(&id) {
            let points = PlotPoints::from_iter(trace.x.iter().zip(&trace.y).map(|(x, y)| [*x, *y]));
            Plot::new(format!("scope-wave-{id}"))
                .height(220.0)
                .legend(Legend::default())
                .show(ui, |plot_ui| {
                    plot_ui.line(Line::new(points).name(trace.channel.clone()));
                });
        }
        if let Some(texture) = self.screenshots.get(&id) {
            let source = texture.size_vec2();
            let width = ui.available_width().min(source.x);
            ui.add(egui::Image::new(texture).fit_to_exact_size(source * (width / source.x)));
        }
        ui.separator();
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
                card(
                    ui,
                    tokens,
                    text(lang, "示波器参数", "Oscilloscope Parameters"),
                    |ui| {
                        ui.horizontal(|ui| {
                            ui.label(text(lang, "默认波形通道", "Default waveform channel"));
                            ui.add(egui::DragValue::new(
                                &mut self.devices[index].controls.scope_channel,
                            )
                            .range(1..=capabilities.channels.max(1)));
                            ui.label(text(lang, "波形点数上限", "Waveform point limit"));
                            ui.label(format!("{}", self.max_points.min(20_000)));
                        });
                    },
                );
            }
            InstrumentKind::DcSource => {
                card(
                    ui,
                    tokens,
                    text(lang, "电源参数", "Source Parameters"),
                    |ui| {
                        egui::Grid::new(format!("source-params-{id}"))
                            .num_columns(2)
                            .spacing([18.0, 6.0])
                            .show(ui, |ui| {
                                setting_row(
                                    ui,
                                    text(lang, "电压上限", "Voltage limit"),
                                    "0 – 60 V",
                                );
                                setting_row(
                                    ui,
                                    text(lang, "电流上限", "Current limit"),
                                    "0 – 20 A",
                                );
                                setting_row(
                                    ui,
                                    text(lang, "输出保护", "Output protection"),
                                    if capabilities.source_protection {
                                        text(lang, "支持 OVP/OCP", "OVP/OCP supported")
                                    } else {
                                        text(lang, "未报告", "Not reported")
                                    },
                                );
                            });
                    },
                );
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
            Job::Scan { library } => match list_resources_with_library(
                (!library.trim().is_empty()).then_some(library.as_str()),
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
            Job::Capture { id, path } => {
                if let Some(device) = devices.get_mut(&id) {
                    match device.capture_scope_png() {
                        Ok(png) => match std::fs::write(&path, &png) {
                            Ok(()) => {
                                let _ = events.send(Event::Screenshot { id, path, png });
                            }
                            Err(error) => send_error(&events, Some(id), error),
                        },
                        Err(error) => send_error(&events, Some(id), error),
                    }
                }
            }
            Job::Waveform {
                id,
                channel,
                points,
            } => {
                if let Some(device) = devices.get_mut(&id) {
                    match device.read_scope_waveform(channel, points) {
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
            scope_controls(ui, lang, tokens, device, &mut commands);
        }
        InstrumentKind::DcSource => {
            source_controls(ui, lang, tokens, device, &mut commands, &mut measure_once);
        }
        InstrumentKind::ElectronicLoad => {
            load_controls(ui, lang, tokens, device, &mut commands, &mut measure_once);
        }
        InstrumentKind::Multimeter => {
            dmm_controls(ui, lang, tokens, device, &mut commands, &mut measure_once);
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

fn scope_controls(
    ui: &mut egui::Ui,
    lang: Lang,
    tokens: &Tokens,
    device: &mut DeviceUi,
    out: &mut Vec<ControlCommand>,
) {
    card(ui, tokens, text(lang, "采集控制", "Acquisition"), |ui| {
        ui.horizontal(|ui| {
            if ui.button(text(lang, "运行", "Run")).clicked() {
                out.push(ControlCommand::ScopeRun);
            }
            if ui.button(text(lang, "停止", "Stop")).clicked() {
                out.push(ControlCommand::ScopeStop);
            }
            if ui.button(text(lang, "单次", "Single")).clicked() {
                out.push(ControlCommand::ScopeSingle);
            }
            if theme::accent_button(ui, tokens, text(lang, "自动设置", "Autoset")).clicked() {
                out.push(ControlCommand::ScopeAutoset);
            }
        });
    });
    card(
        ui,
        tokens,
        text(lang, "垂直通道", "Vertical Channel"),
        |ui| {
            ui.horizontal(|ui| {
                ui.label(text(lang, "通道", "Channel"));
                ui.add(
                    egui::DragValue::new(&mut device.controls.scope_channel)
                        .range(1..=device.capabilities.channels.max(1)),
                );
                if ui
                    .checkbox(
                        &mut device.controls.scope_enabled,
                        text(lang, "启用", "Enabled"),
                    )
                    .changed()
                {
                    out.push(ControlCommand::ScopeChannel {
                        channel: device.controls.scope_channel,
                        enabled: device.controls.scope_enabled,
                    });
                }
                ui.label("V/div");
                ui.add(
                    egui::DragValue::new(&mut device.controls.scope_scale)
                        .speed(0.01)
                        .range(0.000001..=1_000.0),
                );
                ui.label(text(lang, "偏置", "Offset"));
                ui.add(
                    egui::DragValue::new(&mut device.controls.scope_offset)
                        .speed(0.01)
                        .suffix(" V"),
                );
                if ui.button(text(lang, "应用", "Apply")).clicked() {
                    out.push(ControlCommand::ScopeScale {
                        channel: device.controls.scope_channel,
                        volts_per_div: device.controls.scope_scale,
                    });
                    out.push(ControlCommand::ScopeOffset {
                        channel: device.controls.scope_channel,
                        volts: device.controls.scope_offset,
                    });
                }
            });
        },
    );
    card(
        ui,
        tokens,
        text(lang, "时基与触发", "Timebase & Trigger"),
        |ui| {
            ui.horizontal(|ui| {
                ui.label("s/div");
                ui.add(egui::DragValue::new(&mut device.controls.scope_timebase).speed(0.0001));
                ui.label(text(lang, "触发源", "Source"));
                ui.text_edit_singleline(&mut device.controls.trigger_source);
                ui.label(text(lang, "电平", "Level"));
                ui.add(egui::DragValue::new(&mut device.controls.trigger_level).suffix(" V"));
                egui::ComboBox::from_id_salt("scope-slope")
                    .selected_text(&device.controls.trigger_slope)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut device.controls.trigger_slope,
                            "RISE".into(),
                            "RISE",
                        );
                        ui.selectable_value(
                            &mut device.controls.trigger_slope,
                            "FALL".into(),
                            "FALL",
                        );
                    });
                if ui.button(text(lang, "应用", "Apply")).clicked() {
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
        },
    );
}

fn source_controls(
    ui: &mut egui::Ui,
    lang: Lang,
    tokens: &Tokens,
    device: &mut DeviceUi,
    out: &mut Vec<ControlCommand>,
    measure_once: &mut bool,
) {
    card(
        ui,
        tokens,
        text(lang, "输出设定", "Output Setpoints"),
        |ui| {
            ui.horizontal(|ui| {
                ui.label(text(lang, "电压", "Voltage"));
                ui.add(
                    egui::DragValue::new(&mut device.controls.source_voltage)
                        .range(0.0..=60.0)
                        .suffix(" V"),
                );
                ui.label(text(lang, "限流", "Current limit"));
                ui.add(
                    egui::DragValue::new(&mut device.controls.source_current)
                        .range(0.0..=20.0)
                        .suffix(" A"),
                );
                if theme::accent_button(ui, tokens, text(lang, "应用", "Apply")).clicked() {
                    out.push(ControlCommand::SourceVoltage(
                        device.controls.source_voltage,
                    ));
                    out.push(ControlCommand::SourceCurrent(
                        device.controls.source_current,
                    ));
                }
            });
        },
    );
    card(ui, tokens, text(lang, "保护", "Protection"), |ui| {
        ui.horizontal(|ui| {
            ui.label("OVP");
            ui.add(egui::DragValue::new(&mut device.controls.source_ovp).suffix(" V"));
            ui.label("OCP");
            ui.add(egui::DragValue::new(&mut device.controls.source_ocp).suffix(" A"));
            if ui
                .button(text(lang, "设置保护", "Set Protection"))
                .clicked()
            {
                out.push(ControlCommand::SourceOvp(device.controls.source_ovp));
                out.push(ControlCommand::SourceOcp(device.controls.source_ocp));
            }
        });
    });
    let label = if device.controls.source_output {
        text(lang, "关闭输出", "Output OFF")
    } else {
        text(lang, "开启输出", "Output ON")
    };
    let response = if device.controls.source_output {
        theme::stop_button(ui, tokens, label)
    } else {
        theme::accent_button(ui, tokens, label)
    };
    if response.clicked() {
        device.controls.source_output = !device.controls.source_output;
        out.push(ControlCommand::SourceOutput(device.controls.source_output));
    }
    ui.horizontal(|ui| {
        if ui.button(text(lang, "读取实测", "Read Actual")).clicked() {
            *measure_once = true;
        }
    });
}

fn load_controls(
    ui: &mut egui::Ui,
    lang: Lang,
    tokens: &Tokens,
    device: &mut DeviceUi,
    out: &mut Vec<ControlCommand>,
    measure_once: &mut bool,
) {
    card(ui, tokens, text(lang, "负载模式", "Load Mode"), |ui| {
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("load-mode")
                .selected_text(&device.controls.load_mode)
                .show_ui(ui, |ui| {
                    for mode in &device.capabilities.load_modes {
                        ui.selectable_value(&mut device.controls.load_mode, mode.clone(), mode);
                    }
                });
            ui.add(egui::DragValue::new(&mut device.controls.load_level).speed(0.01));
            if ui.button(text(lang, "应用", "Apply")).clicked() {
                out.push(ControlCommand::LoadMode(device.controls.load_mode.clone()));
                out.push(ControlCommand::LoadLevel {
                    mode: device.controls.load_mode.clone(),
                    value: device.controls.load_level,
                });
            }
        });
    });
    let label = if device.controls.load_input {
        text(lang, "关闭负载", "Input OFF")
    } else {
        text(lang, "开启负载", "Input ON")
    };
    let response = if device.controls.load_input {
        theme::stop_button(ui, tokens, label)
    } else {
        theme::accent_button(ui, tokens, label)
    };
    if response.clicked() {
        device.controls.load_input = !device.controls.load_input;
        out.push(ControlCommand::LoadInput(device.controls.load_input));
    }
    ui.horizontal(|ui| {
        if ui.button(text(lang, "读取实测", "Read Actual")).clicked() {
            *measure_once = true;
        }
    });
}

fn dmm_controls(
    ui: &mut egui::Ui,
    lang: Lang,
    tokens: &Tokens,
    device: &mut DeviceUi,
    out: &mut Vec<ControlCommand>,
    measure_once: &mut bool,
) {
    card(
        ui,
        tokens,
        text(lang, "测量配置", "Measurement Setup"),
        |ui| {
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("dmm-function")
                    .selected_text(format!("{:?}", device.controls.dmm_function))
                    .show_ui(ui, |ui| {
                        for function in [
                            MeasureFunction::DcVoltage,
                            MeasureFunction::AcVoltage,
                            MeasureFunction::DcCurrent,
                            MeasureFunction::AcCurrent,
                            MeasureFunction::Resistance,
                            MeasureFunction::Frequency,
                        ] {
                            ui.selectable_value(
                                &mut device.controls.dmm_function,
                                function,
                                format!("{function:?}"),
                            );
                        }
                    });
                ui.checkbox(
                    &mut device.controls.dmm_autorange,
                    text(lang, "自动量程", "Auto range"),
                );
                ui.label(text(lang, "量程", "Range"));
                ui.add_enabled(
                    !device.controls.dmm_autorange,
                    egui::DragValue::new(&mut device.controls.dmm_range).speed(0.1),
                );
                ui.label("NPLC");
                ui.add(egui::DragValue::new(&mut device.controls.dmm_nplc).range(0.001..=100.0));
                if ui.button(text(lang, "应用", "Apply")).clicked() {
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
            ui.horizontal(|ui| {
                if ui.button(text(lang, "单次测量", "Measure Once")).clicked() {
                    *measure_once = true;
                }
            });
        },
    );
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
    let card_id = ui.id().with(("instrument-card", kind.label()));
    let card = Frame::NONE
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

            let header = ui.horizontal(|ui| {
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
                    }
                    ui.label(RichText::new(status).small().color(if connected_count > 0 {
                        tokens.accent
                    } else {
                        tokens.text_muted
                    }));
                });
            });
            if header.response.clicked() {
                action.selected = true;
            }
            ui.separator();
            ui.vertical(|ui| {
                ui.set_width(content_width);
                let combo = egui::ComboBox::from_id_salt(("card-visa-resource", kind.label()))
                    .selected_text(short_resource(resource_input))
                    .width(content_width)
                    .show_ui(ui, |ui| {
                        for resource in resources {
                            ui.selectable_value(
                                resource_input,
                                resource.address.clone(),
                                format!(
                                    "{}  {}",
                                    resource.transport,
                                    short_resource(&resource.address)
                                ),
                            );
                        }
                    });
                if combo.response.clicked() {
                    action.selected = true;
                }
                let edit = ui.add_sized(
                    [content_width, CARD_CONTROL_HEIGHT],
                    egui::TextEdit::singleline(resource_input)
                        .hint_text("TCPIP0::192.168.1.10::INSTR")
                        .margin(egui::vec2(6.0, 4.0)),
                );
                if edit.clicked() {
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
                    let connect_clicked = ui
                        .add_enabled(
                            !resource_input.trim().is_empty(),
                            egui::Button::new(text(lang, "连接", "Connect"))
                                .fill(tokens.accent)
                                .min_size(egui::vec2(button_width, CARD_CONTROL_HEIGHT)),
                        )
                        .clicked();
                    if connect_clicked {
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
                            action.disconnect = Some(id);
                        }
                    }
                });
            });
        });
    if ui.interact(card.response.rect, card_id, Sense::click()).clicked() {
        action.selected = true;
    }
    action
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
            "读取波形/截图，并支持连续采样、实时曲线与 CSV 导出。",
            "Read waveforms/screenshots with continuous sampling, live plots and CSV export.",
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
