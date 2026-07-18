//! WiParse GUI - Python MonitorWindow layout parity.

// Hide the console window when launching dist\WiParse.exe on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod calculator;
mod converter;
mod fonts;
mod instrument_control;
mod log_tab;
mod log_view;
mod serial_tool;
mod theme;
mod windows_icon;

use app::WiParseApp;
use std::sync::Arc;
use wiparse_core::config::load_config;
use wiparse_core::paths::project_path;

/// Embedded at compile time so window/taskbar icons work from dist without sidecar files.
const EMBEDDED_ICO: &[u8] = include_bytes!("../../../Icon/WiParse.ico");

fn icon_from_bytes(bytes: &[u8]) -> Option<egui::IconData> {
    let img = image::load_from_memory(bytes).ok()?;
    Some(to_icon_data(pick_icon_size(img)))
}

fn pick_icon_size(img: image::DynamicImage) -> image::DynamicImage {
    // 32x32 is crisp for title bar + taskbar.
    const TARGET: u32 = 32;
    if img.width() == TARGET && img.height() == TARGET {
        return img;
    }
    img.resize_exact(TARGET, TARGET, image::imageops::FilterType::Lanczos3)
}

fn to_icon_data(img: image::DynamicImage) -> egui::IconData {
    let rgba = img.to_rgba8();
    let (width, height) = (img.width(), img.height());
    egui::IconData {
        rgba: rgba.into_raw(),
        width,
        height,
    }
}

fn load_window_icon() -> Option<egui::IconData> {
    if let Some(icon) = icon_from_bytes(EMBEDDED_ICO) {
        return Some(icon);
    }

    let mut candidates = vec![
        project_path("packaging/WiParse.ico"),
        project_path("Icon/WiParse.ico"),
        std::path::PathBuf::from("packaging/WiParse.ico"),
        std::path::PathBuf::from("Icon/WiParse.ico"),
        std::path::PathBuf::from("WiParse.ico"),
    ];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("WiParse.ico"));
            candidates.push(dir.join("Icon").join("WiParse.ico"));
            candidates.push(dir.join("packaging").join("WiParse.ico"));
        }
    }

    for path in candidates {
        if !path.is_file() {
            continue;
        }
        if let Ok(bytes) = std::fs::read(&path) {
            if let Some(icon) = icon_from_bytes(&bytes) {
                tracing::info!("Loaded window icon from {}", path.display());
                return Some(icon);
            }
        }
    }
    tracing::warn!("Failed to load WiParse.ico for window/taskbar icon");
    None
}

fn main() -> eframe::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    windows_icon::set_process_app_id();

    let cfg = load_config().unwrap_or_default();
    let icon = load_window_icon();
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1440.0, 900.0])
        .with_min_inner_size([1280.0, 720.0])
        .with_title("WiParse");
    if let Some(ref icon) = icon {
        viewport = viewport.with_icon(Arc::new(icon.clone()));
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "WiParse",
        options,
        Box::new(move |cc| {
            fonts::install_cjk_fonts(&cc.egui_ctx);
            // Re-apply after context creation - some backends ignore builder icon alone.
            if let Some(icon) = load_window_icon() {
                cc.egui_ctx
                    .send_viewport_cmd(egui::ViewportCommand::Icon(Some(Arc::new(icon))));
            }
            Ok(Box::new(WiParseApp::new(cfg)))
        }),
    )
}
