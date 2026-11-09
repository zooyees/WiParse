//! WiParse GUI - Python MonitorWindow layout parity.

// Hide the console window when launching dist\WiParse.exe on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod ai;
mod backend;
mod calculator;
mod converter;
mod error_log;
mod fonts;
mod instrument_control;
mod log_tab;
mod log_view;
mod serial_tool;
mod plot;
mod theme;
mod update;
mod data_analysis;
mod data_extract_cache;
mod market_jobs;
mod test_report;
mod test_tool;
mod waveform_analysis;
mod waveform_protocol_html;
mod windows_icon;
#[cfg(test)]
mod ui_audit;

use app::WiParseApp;
use std::sync::{Arc, OnceLock};
use wiparse_core::config::load_config;
use wiparse_core::paths::project_path;

static COMPANION_PARENT: OnceLock<Option<u32>> = OnceLock::new();
static COMPANION_DETACHED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub(crate) fn companion_detached() -> bool { COMPANION_DETACHED.load(std::sync::atomic::Ordering::Acquire) }

pub(crate) fn companion_window() -> bool {
    COMPANION_PARENT.get().is_some()
}

pub(crate) fn companion_parent() -> Option<u32> {
    COMPANION_PARENT.get().copied().flatten()
}

fn remember_companion() {
    let mut args = std::env::args().skip(1);
    let mut companion = false;
    let mut parent = None;
    while let Some(arg) = args.next() {
        if arg == "--companion" {
            companion = true;
        } else if arg == "--companion-parent" {
            parent = args.next().and_then(|value| value.parse::<u32>().ok());
        }
    }
    if companion {
        let _ = COMPANION_PARENT.set(parent);
    }
}

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

#[cfg(windows)]
fn watch_companion_parent(pid: u32) {
    std::thread::spawn(move || {
        unsafe {
            const SYNCHRONIZE: u32 = 0x00100000;
            const INFINITE: u32 = 0xFFFFFFFF;
            #[link(name = "kernel32")]
            unsafe extern "system" {
                fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
                fn WaitForSingleObject(handle: *mut std::ffi::c_void, milliseconds: u32) -> u32;
                fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
            }
            let handle = OpenProcess(SYNCHRONIZE, 0, pid);
            if handle.is_null() {
                // An inaccessible parent is not proof of process death. Keep
                // the host available for recovery rather than losing data.
                COMPANION_DETACHED.store(true, std::sync::atomic::Ordering::Release);
                return;
            }
            let result = WaitForSingleObject(handle, INFINITE);
            CloseHandle(handle);
            if result != 0 { tracing::warn!("Companion parent wait failed; retaining host for recovery"); }
        }
        COMPANION_DETACHED.store(true, std::sync::atomic::Ordering::Release);
    });
}

#[cfg(not(windows))]
fn watch_companion_parent(_pid: u32) {}

fn main() -> eframe::Result<()> {
    error_log::init();
    remember_companion();
    if let Some(parent) = companion_parent() {
        watch_companion_parent(parent);
    }
    windows_icon::set_process_app_id();

    let (cfg, config_warning) = match load_config() {
        Ok(cfg) => (cfg, None),
        Err(e) => {
            tracing::error!("Configuration load failed; original file preserved: {e}");
            (Default::default(), Some(format!("配置加载失败，原文件已保留；本次临时使用默认值 / Configuration load failed; the original file was preserved: {e}")))
        }
    };
    let icon = load_window_icon();
    let companion = companion_window();
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size(if companion { [320.0, 240.0] } else { [1440.0, 900.0] })
        .with_min_inner_size(if companion { [320.0, 240.0] } else { [1100.0, 720.0] })
        .with_title("WiParse")
        .with_visible(!companion)
        .with_taskbar(!companion)
        .with_active(!companion);
    if let Some(ref icon) = icon {
        viewport = viewport.with_icon(Arc::new(icon.clone()));
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    match eframe::run_native(
        "WiParse",
        options,
        Box::new(move |cc| {
            fonts::install_cjk_fonts(&cc.egui_ctx);
            // Re-apply after context creation - some backends ignore builder icon alone.
            if let Some(icon) = load_window_icon() {
                cc.egui_ctx
                    .send_viewport_cmd(egui::ViewportCommand::Icon(Some(Arc::new(icon))));
            }
            Ok(Box::new(WiParseApp::new_with_warning(cfg, config_warning)))
        }),
    ) {
        Ok(()) => Ok(()),
        Err(err) => {
            let msg = format!("eframe fatal error: {err:#}");
            tracing::error!("{msg}");
            error_log::append_crash(&msg);
            Err(err)
        }
    }
}
