//! GUI-side update check, download, and staged install (Windows portable).

use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use crossbeam_channel::{Receiver, unbounded};
use wiparse_core::config::UpdateConfig;
use wiparse_core::update::{
    evaluate_manifest, fetch_manifest, manifest_url, verify_file_sha256, UpdateAvailability,
    UpdateManifest, UpdatePackage, UpdateTarget,
};
use wiparse_core::VERSION;

#[derive(Debug, Clone)]
pub enum UpdatePhase {
    Idle,
    Checking,
    UpToDate,
    Available {
        manifest: UpdateManifest,
        package: UpdatePackage,
    },
    Downloading {
        received: u64,
        total: u64,
    },
    Ready(PathBuf),
    Error(String),
}

pub struct UpdateController {
    cfg: UpdateConfig,
    phase: UpdatePhase,
    last_check: Option<std::time::Instant>,
    rx: Option<Receiver<UpdateMsg>>,
}

enum UpdateMsg {
    Checked(Result<UpdateAvailability, String>),
    Progress { received: u64, total: u64 },
    Downloaded(Result<PathBuf, String>),
}

impl UpdateController {
    pub fn new(cfg: UpdateConfig) -> Self {
        Self {
            cfg,
            phase: UpdatePhase::Idle,
            last_check: None,
            rx: None,
        }
    }

    pub fn phase(&self) -> &UpdatePhase {
        &self.phase
    }

    pub fn set_config(&mut self, cfg: UpdateConfig) {
        self.cfg = cfg;
    }

    /// Poll background worker; call each frame from About dialog or app update.
    pub fn poll(&mut self) {
        let Some(rx) = self.rx.as_ref() else {
            return;
        };
        while let Ok(msg) = rx.try_recv() {
            match msg {
                UpdateMsg::Checked(Ok(avail)) => match avail {
                    UpdateAvailability::Disabled | UpdateAvailability::UpToDate => {
                        self.phase = UpdatePhase::UpToDate;
                    }
                    UpdateAvailability::BelowMinimum { current, min_version } => {
                        self.phase = UpdatePhase::Error(format!(
                            "Current {current} is below minimum supported {min_version}"
                        ));
                    }
                    UpdateAvailability::UpdateAvailable { manifest, package } => {
                        self.phase = UpdatePhase::Available { manifest, package };
                    }
                },
                UpdateMsg::Checked(Err(e)) => self.phase = UpdatePhase::Error(e),
                UpdateMsg::Progress { received, total } => {
                    self.phase = UpdatePhase::Downloading { received, total };
                }
                UpdateMsg::Downloaded(Ok(path)) => self.phase = UpdatePhase::Ready(path),
                UpdateMsg::Downloaded(Err(e)) => self.phase = UpdatePhase::Error(e),
            }
        }
    }

    pub fn check_now(&mut self) {
        let Some(url) = manifest_url(&self.cfg) else {
            self.phase = UpdatePhase::Error("Update URL not configured".into());
            return;
        };
        if matches!(self.phase, UpdatePhase::Checking) {
            return;
        }
        self.phase = UpdatePhase::Checking;
        self.last_check = Some(std::time::Instant::now());
        let (tx, rx) = unbounded();
        self.rx = Some(rx);
        thread::spawn(move || {
            let result = fetch_manifest(&url)
                .map(|m| evaluate_manifest(VERSION, &m, UpdateTarget::current()))
                .map_err(|e| e.to_string());
            let _ = tx.send(UpdateMsg::Checked(result));
        });
    }

    pub fn maybe_check_on_startup(&mut self) {
        if !self.cfg.enabled || manifest_url(&self.cfg).is_none() {
            return;
        }
        let interval = Duration::from_secs(self.cfg.check_interval_hours as u64 * 3600);
        if let Some(last) = self.last_check {
            if last.elapsed() < interval {
                return;
            }
        }
        self.check_now();
    }

    pub fn download_available(&mut self) {
        let UpdatePhase::Available { package, .. } = self.phase.clone() else {
            return;
        };
        self.phase = UpdatePhase::Downloading {
            received: 0,
            total: package.size,
        };
        let (tx, rx) = unbounded();
        self.rx = Some(rx);
        thread::spawn(move || {
            let result = download_package(&package);
            let _ = tx.send(UpdateMsg::Downloaded(result));
        });
    }

    /// Stage install: write helper script and exit so files can be replaced.
    pub fn apply_ready_and_exit(&self) -> Result<(), String> {
        let UpdatePhase::Ready(zip_path) = &self.phase else {
            return Err("no downloaded update".into());
        };
        stage_apply_script(zip_path)?;
        std::process::exit(0);
    }
}

fn download_package(package: &UpdatePackage) -> Result<PathBuf, String> {
    let dir = update_cache_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let name = package
        .filename
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("wiparse-update.zip");
    let dest = dir.join(name);
    let resp = ureq::get(&package.url)
        .call()
        .map_err(|e| e.to_string())?;
    let mut reader = resp.into_reader();
    let mut file = std::fs::File::create(&dest).map_err(|e| e.to_string())?;
    std::io::copy(&mut reader, &mut file).map_err(|e| e.to_string())?;
    verify_file_sha256(&dest, &package.sha256).map_err(|e| e.to_string())?;
    Ok(dest)
}

fn update_cache_dir() -> Result<PathBuf, String> {
    dirs::cache_dir()
        .map(|d| d.join("WiParse").join("updates"))
        .ok_or_else(|| "cannot resolve cache dir".into())
}

fn install_dir() -> Result<PathBuf, String> {
    std::env::current_exe()
        .map_err(|e| e.to_string())?
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "cannot resolve install dir".into())
}

fn stage_apply_script(zip_path: &Path) -> Result<(), String> {
    let install = install_dir()?;
    let script_src = include_str!("../../../../packaging/update/apply-update.ps1");
    let script_path = update_cache_dir()?.join("apply-update.ps1");
    std::fs::write(&script_path, script_src).map_err(|e| e.to_string())?;
    let pid = std::process::id();
    let status = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            &script_path.to_string_lossy(),
            "-ZipPath",
            &zip_path.to_string_lossy(),
            "-InstallDir",
            &install.to_string_lossy(),
            "-WaitPid",
            &pid.to_string(),
        ])
        .spawn()
        .map_err(|e| e.to_string())?;
    drop(status);
    Ok(())
}
