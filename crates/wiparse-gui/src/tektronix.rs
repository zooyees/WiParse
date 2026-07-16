//! Minimal Tektronix scope page: connection and screen capture only.

use crate::theme::Tokens;
use crossbeam_channel::{unbounded, Receiver, Sender};
use egui::{self, Color32, CornerRadius, Frame, Margin, RichText, Stroke};
use std::sync::{Arc, Mutex};
use std::thread;
use wiparse_core::i18n::{tr, Lang};
use wiparse_core::scope::TektronixScopeClient;

enum Job {
    Connect,
    CaptureScreen,
}

enum Event {
    Connected {
        idn: String,
    },
    Captured {
        path: String,
        size: [usize; 2],
        rgba: Vec<u8>,
    },
    CaptureSaved {
        path: String,
    },
    Error(String),
}

struct SharedClient {
    inner: Mutex<TektronixScopeClient>,
    egui_ctx: Mutex<Option<egui::Context>>,
}

pub struct TektronixPanel {
    connected: bool,
    model: String,
    status: String,
    screen_capture: Option<egui::TextureHandle>,
    tx: Sender<Job>,
    rx: Receiver<Event>,
    client: Arc<SharedClient>,
}

impl TektronixPanel {
    pub fn new() -> Self {
        let client = Arc::new(SharedClient {
            inner: Mutex::new(TektronixScopeClient::new()),
            egui_ctx: Mutex::new(None),
        });
        let (tx, jobs) = unbounded();
        let (events, rx) = unbounded();
        let worker_client = Arc::clone(&client);
        thread::spawn(move || worker_loop(worker_client, jobs, events));

        Self {
            connected: false,
            model: "-".into(),
            status: "Disconnected".into(),
            screen_capture: None,
            tx,
            rx,
            client,
        }
    }

    pub fn pump(&mut self, ctx: &egui::Context) {
        self.drain(ctx);
    }

    pub fn status_text(&self) -> &str {
        &self.status
    }

    /// The simplified scope page has no background/live acquisition loop.
    pub fn live_active(&self) -> bool {
        false
    }

    fn send(&self, job: Job) {
        let _ = self.tx.send(job);
    }

    fn drain(&mut self, ctx: &egui::Context) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::Connected { idn } => {
                    self.connected = true;
                    self.model = idn.split(',').nth(1).unwrap_or(&idn).trim().to_owned();
                    self.status = format!("Connected: {}", self.model);
                }
                Event::Captured { path, size, rgba } => {
                    let image = egui::ColorImage::from_rgba_unmultiplied(size, &rgba);
                    ctx.copy_image(image.clone());
                    self.screen_capture =
                        Some(ctx.load_texture("scope_screen_capture", image, Default::default()));
                    self.status = format!("Captured and copied: {path}");
                }
                Event::CaptureSaved { path } => {
                    self.status = format!("Captured {path}");
                }
                Event::Error(error) => self.status = error,
            }
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, lang: Lang, t: &Tokens) {
        if let Ok(mut slot) = self.client.egui_ctx.lock() {
            *slot = Some(ui.ctx().clone());
        }
        self.drain(ui.ctx());

        Frame::NONE
            .fill(t.panel_bg)
            .stroke(Stroke::new(1.0, t.border))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(Margin::same(12))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let connect_label = if self.connected {
                        "Connected"
                    } else {
                        &tr(lang, "scope.connect")
                    };
                    if ui
                        .add_enabled(
                            !self.connected,
                            egui::Button::new(connect_label).min_size(egui::vec2(104.0, 30.0)),
                        )
                        .clicked()
                    {
                        self.status = "Connecting…".into();
                        self.send(Job::Connect);
                    }
                    if ui
                        .add_enabled(
                            self.connected,
                            egui::Button::new(tr(lang, "scope.shot"))
                                .min_size(egui::vec2(118.0, 30.0)),
                        )
                        .clicked()
                    {
                        self.status = "Capturing screen and copying…".into();
                        self.send(Job::CaptureScreen);
                    }
                    ui.separator();
                    ui.label(
                        RichText::new(format!("{}: {}", tr(lang, "scope.scope_n"), self.model))
                            .color(t.text_muted),
                    );
                });

                ui.add_space(10.0);
                ui.label(RichText::new(&self.status).size(12.0).color(t.text_muted));
                ui.add_space(8.0);

                let preview_size = ui.available_size();
                Frame::NONE
                    .fill(Color32::from_rgb(0x0B, 0x12, 0x20))
                    .stroke(Stroke::new(1.0, t.border))
                    .corner_radius(CornerRadius::same(4))
                    .show(ui, |ui| {
                        ui.set_min_size(preview_size);
                        if let Some(screen) = &self.screen_capture {
                            let source = screen.size_vec2();
                            let scale = (ui.available_width() / source.x)
                                .min(ui.available_height() / source.y)
                                .min(1.0);
                            ui.centered_and_justified(|ui| {
                                ui.add(
                                    egui::Image::new(screen)
                                        .fit_to_exact_size(source * scale.max(0.01)),
                                );
                            });
                        } else {
                            ui.centered_and_justified(|ui| {
                                ui.label(
                                    RichText::new("Connect the scope, then capture its screen.")
                                        .color(t.text_muted),
                                );
                            });
                        }
                    });
            });
    }

    pub fn on_exit(&mut self) {
        if let Ok(mut client) = self.client.inner.lock() {
            client.close();
        }
    }
}

fn wake_ui(client: &SharedClient) {
    if let Ok(slot) = client.egui_ctx.lock() {
        if let Some(ctx) = slot.as_ref() {
            ctx.request_repaint();
        }
    }
}

fn worker_loop(client: Arc<SharedClient>, jobs: Receiver<Job>, events: Sender<Event>) {
    while let Ok(job) = jobs.recv() {
        match job {
            Job::Connect => {
                let result = client.inner.lock().unwrap().connect(None, 0);
                match result {
                    Ok(value) => {
                        let idn = value["idn"].as_str().unwrap_or("").to_owned();
                        let _ = events.send(Event::Connected { idn });
                    }
                    Err(error) => {
                        let _ = events.send(Event::Error(error.user_message()));
                    }
                }
            }
            Job::CaptureScreen => {
                let result = client.inner.lock().unwrap().capture_png(None, 0);
                match result {
                    Ok(value) => {
                        let path = value["path"].as_str().unwrap_or("").to_owned();
                        let image = std::fs::read(&path)
                            .ok()
                            .and_then(|bytes| image::load_from_memory(&bytes).ok());
                        if let Some(image) = image {
                            let rgba = image.to_rgba8();
                            let size = [rgba.width() as usize, rgba.height() as usize];
                            let _ = events.send(Event::Captured {
                                path,
                                size,
                                rgba: rgba.into_raw(),
                            });
                        } else {
                            let _ = events.send(Event::CaptureSaved { path });
                        }
                    }
                    Err(error) => {
                        let _ = events.send(Event::Error(error.user_message()));
                    }
                }
            }
        }
        wake_ui(&client);
    }
}
