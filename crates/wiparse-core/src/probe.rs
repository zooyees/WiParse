//! Debug-probe sessions (J-Link / ST-Link / CMSIS-DAP) via probe-rs.

use crate::instrument::{ControlCommand, Identity, InstrumentError, InstrumentKind};
use crate::usb::{parse_usb_resource, UsbIface, UsbResource};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

pub struct ProbeSession {
    pub resource: String,
    pub identity: Identity,
    pub kind: InstrumentKind,
    inner: ProbeInner,
    rtt_channel: u32,
    speed_khz: u32,
}

enum ProbeInner {
    Demo {
        halted: bool,
        mem: BTreeMap<u64, u8>,
        rtt_on: bool,
        chip: String,
        ticks: u32,
    },
    Live {
        session: probe_rs::Session,
        rtt: Option<probe_rs::rtt::Rtt>,
    },
}

impl ProbeSession {
    pub fn demo(resource: impl Into<String>) -> Self {
        let resource = resource.into();
        let spec = parse_usb_resource(&resource).unwrap_or(UsbResource {
            iface: UsbIface::Jlink,
            serial: "DEMO".into(),
        });
        let mut identity = spec.identity();
        identity.firmware = "demo".into();
        identity.serial = if identity.serial.is_empty() {
            "DEMO".into()
        } else {
            identity.serial
        };
        Self {
            resource,
            identity,
            kind: InstrumentKind::DebugProbe,
            inner: ProbeInner::Demo {
                halted: true,
                mem: BTreeMap::new(),
                rtt_on: false,
                chip: String::new(),
                ticks: 0,
            },
            rtt_channel: 0,
            speed_khz: 4_000,
        }
    }

    pub fn open(resource: &str, chip: Option<&str>) -> Result<Self, InstrumentError> {
        Self::open_at_speed(resource, chip, 4_000)
    }

    pub fn open_at_speed(
        resource: &str,
        chip: Option<&str>,
        speed_khz: u32,
    ) -> Result<Self, InstrumentError> {
        let spec = parse_usb_resource(resource).ok_or_else(|| {
            InstrumentError::Io(format!("not a probe resource: {resource}"))
        })?;
        if spec.iface == UsbIface::Ft4222 {
            return Err(InstrumentError::Unsupported(
                "FT4222 is a USB bridge, not a debug probe".into(),
            ));
        }
        let probes = probe_rs::probe::list::Lister::new().list_all();
        let info = probes
            .into_iter()
            .find(|p| probe_matches(&spec, p))
            .ok_or_else(|| {
                InstrumentError::Io(format!(
                    "probe {} serial={} not found",
                    spec.iface.as_str(),
                    spec.serial
                ))
            })?;
        let mut probe = info.open().map_err(probe_err)?;
        let speed_khz = speed_khz.clamp(10, 50_000);
        let applied = probe.set_speed(speed_khz).unwrap_or(speed_khz);
        let chip = chip.unwrap_or("").trim().to_owned();
        let session = attach_session(probe, &chip)?;
        let mut identity = spec.identity();
        identity.firmware = chip.clone();
        Ok(Self {
            resource: resource.to_owned(),
            identity,
            kind: InstrumentKind::DebugProbe,
            inner: ProbeInner::Live {
                session,
                rtt: None,
            },
            rtt_channel: 0,
            speed_khz: applied,
        })
    }

    pub fn execute(
        &mut self,
        command: ControlCommand,
    ) -> Result<Option<String>, InstrumentError> {
        match command {
            ControlCommand::ProbeAttach { target } => self.attach(&target),
            ControlCommand::ProbeHalt => self.halt(),
            ControlCommand::ProbeRun => self.run(),
            ControlCommand::ProbeReset { hardware } => self.reset(hardware),
            ControlCommand::ProbeMemRead { address, len } => self.mem_read(&address, len),
            ControlCommand::ProbeMemWrite { address, data_hex } => {
                self.mem_write(&address, &data_hex)
            }
            ControlCommand::ProbeFlash {
                path,
                verify,
                base_address,
            } => self.flash(&path, verify, base_address),
            ControlCommand::ProbeRttStart { up_channel } => self.rtt_start(up_channel),
            ControlCommand::ProbeRttStop => self.rtt_stop(),
            ControlCommand::ProbeRttRead => self.rtt_read(),
            ControlCommand::ProbeStatus => self.status(),
            ControlCommand::ProbeRegs => self.regs(),
            ControlCommand::ProbeErase => self.erase(),
            ControlCommand::ProbeSpeed { khz } => self.set_speed(khz),
            other => Err(InstrumentError::Unsupported(format!(
                "debug probe cannot run {other:?}"
            ))),
        }
    }

    fn attach(&mut self, target: &str) -> Result<Option<String>, InstrumentError> {
        match &mut self.inner {
            ProbeInner::Demo { chip, .. } => {
                *chip = target.trim().to_owned();
                self.identity.firmware = chip.clone();
                Ok(Some(if chip.is_empty() {
                    "demo probe attached (unspecified target)".into()
                } else {
                    format!("demo probe attached to {chip}")
                }))
            }
            ProbeInner::Live { .. } => {
                let resource = self.resource.clone();
                *self = Self::open(&resource, Some(target))?;
                Ok(Some(if target.trim().is_empty() {
                    "attached (unspecified target / DP only)".into()
                } else {
                    format!("attached to {}", target.trim())
                }))
            }
        }
    }

    fn halt(&mut self) -> Result<Option<String>, InstrumentError> {
        match &mut self.inner {
            ProbeInner::Demo { halted, .. } => {
                *halted = true;
                Ok(Some("halted".into()))
            }
            ProbeInner::Live { session, .. } => {
                let mut core = session.core(0).map_err(probe_err)?;
                core.halt(Duration::from_millis(500)).map_err(probe_err)?;
                Ok(Some("halted".into()))
            }
        }
    }

    fn run(&mut self) -> Result<Option<String>, InstrumentError> {
        match &mut self.inner {
            ProbeInner::Demo { halted, .. } => {
                *halted = false;
                Ok(Some("running".into()))
            }
            ProbeInner::Live { session, .. } => {
                let mut core = session.core(0).map_err(probe_err)?;
                core.run().map_err(probe_err)?;
                Ok(Some("running".into()))
            }
        }
    }

    fn reset(&mut self, hardware: bool) -> Result<Option<String>, InstrumentError> {
        match &mut self.inner {
            ProbeInner::Demo { halted, .. } => {
                *halted = true;
                Ok(Some(if hardware {
                    "hardware reset (demo)".into()
                } else {
                    "system reset (demo)".into()
                }))
            }
            ProbeInner::Live { session, .. } => {
                let mut core = session.core(0).map_err(probe_err)?;
                if hardware {
                    core.reset().map_err(probe_err)?;
                } else {
                    core.reset_and_halt(Duration::from_millis(500))
                        .map_err(probe_err)?;
                }
                Ok(Some(if hardware {
                    "hardware reset".into()
                } else {
                    "reset-and-halt".into()
                }))
            }
        }
    }

    fn mem_read(&mut self, address: &str, len: u32) -> Result<Option<String>, InstrumentError> {
        let addr = parse_addr(address)?;
        let n = len.clamp(1, 4096) as usize;
        let mut buf = vec![0u8; n];
        match &mut self.inner {
            ProbeInner::Demo { mem, .. } => {
                for (i, b) in buf.iter_mut().enumerate() {
                    *b = mem.get(&(addr + i as u64)).copied().unwrap_or(0);
                }
            }
            ProbeInner::Live { session, .. } => {
                use probe_rs::MemoryInterface;
                let mut core = session.core(0).map_err(probe_err)?;
                core.read_8(addr, &mut buf).map_err(probe_err)?;
            }
        }
        Ok(Some(format_hex_dump(addr, &buf)))
    }

    fn mem_write(&mut self, address: &str, data_hex: &str) -> Result<Option<String>, InstrumentError> {
        let addr = parse_addr(address)?;
        let data = parse_hex_bytes(data_hex)?;
        match &mut self.inner {
            ProbeInner::Demo { mem, .. } => {
                for (i, b) in data.iter().enumerate() {
                    mem.insert(addr + i as u64, *b);
                }
            }
            ProbeInner::Live { session, .. } => {
                use probe_rs::MemoryInterface;
                let mut core = session.core(0).map_err(probe_err)?;
                core.write_8(addr, &data).map_err(probe_err)?;
            }
        }
        Ok(Some(format!("wrote {} byte(s) at {addr:#010X}", data.len())))
    }

    fn flash(
        &mut self,
        path: &str,
        verify: bool,
        base_address: Option<u64>,
    ) -> Result<Option<String>, InstrumentError> {
        let path = Path::new(path);
        if !path.is_file() {
            return Err(InstrumentError::Io(format!("flash file not found: {}", path.display())));
        }
        match &mut self.inner {
            ProbeInner::Demo { mem, .. } => {
                let bytes = std::fs::read(path).map_err(|e| InstrumentError::Io(e.to_string()))?;
                let base = base_address.unwrap_or(0x0800_0000);
                for (i, b) in bytes.iter().enumerate() {
                    mem.insert(base + i as u64, *b);
                }
                Ok(Some(format!(
                    "demo flash {} ({} bytes) verify={verify}",
                    path.display(),
                    bytes.len()
                )))
            }
            ProbeInner::Live { session, .. } => {
                let format = flash_format(path, base_address);
                let mut options = probe_rs::flashing::DownloadOptions::default();
                options.verify = verify;
                probe_rs::flashing::download_file_with_options(session, path, format, options)
                    .map_err(probe_err)?;
                Ok(Some(format!(
                    "flashed {}{}",
                    path.display(),
                    if verify { " (verified)" } else { "" }
                )))
            }
        }
    }

    fn rtt_start(&mut self, up_channel: u32) -> Result<Option<String>, InstrumentError> {
        self.rtt_channel = up_channel;
        match &mut self.inner {
            ProbeInner::Demo { rtt_on, .. } => {
                *rtt_on = true;
                Ok(Some(format!("RTT up-channel {up_channel} started (demo)")))
            }
            ProbeInner::Live { session, rtt, .. } => {
                let attached = {
                    let mut core = session.core(0).map_err(probe_err)?;
                    probe_rs::rtt::Rtt::attach(&mut core).map_err(|e| {
                        InstrumentError::Io(format!(
                            "RTT attach failed ({e}). SWO/ITM is not enabled in this build."
                        ))
                    })?
                };
                *rtt = Some(attached);
                Ok(Some(format!("RTT up-channel {up_channel} started")))
            }
        }
    }

    fn rtt_stop(&mut self) -> Result<Option<String>, InstrumentError> {
        match &mut self.inner {
            ProbeInner::Demo { rtt_on, .. } => {
                *rtt_on = false;
                Ok(Some("RTT stopped".into()))
            }
            ProbeInner::Live { rtt, .. } => {
                *rtt = None;
                Ok(Some("RTT stopped".into()))
            }
        }
    }

    fn rtt_read(&mut self) -> Result<Option<String>, InstrumentError> {
        match &mut self.inner {
            ProbeInner::Demo {
                rtt_on, ticks, ..
            } => {
                if !*rtt_on {
                    return Ok(None);
                }
                *ticks += 1;
                if *ticks % 5 != 0 {
                    return Ok(None);
                }
                Ok(Some(format!("[demo RTT] tick {}\n", *ticks)))
            }
            ProbeInner::Live {
                session,
                rtt,
                ..
            } => {
                let Some(rtt) = rtt.as_mut() else {
                    return Ok(None);
                };
                let mut core = session.core(0).map_err(probe_err)?;
                let ch = self.rtt_channel as usize;
                let Some(channel) = rtt.up_channel(ch) else {
                    return Err(InstrumentError::Io(format!("no RTT up-channel {ch}")));
                };
                let mut buf = [0u8; 1024];
                let n = channel.read(&mut core, &mut buf).map_err(probe_err)?;
                if n == 0 {
                    return Ok(None);
                }
                Ok(Some(String::from_utf8_lossy(&buf[..n]).into_owned()))
            }
        }
    }

    fn status(&mut self) -> Result<Option<String>, InstrumentError> {
        match &mut self.inner {
            ProbeInner::Demo { halted, chip, .. } => Ok(Some(format!(
                "demo {} target={} speed={} kHz",
                if *halted { "halted" } else { "running" },
                if chip.is_empty() { "auto" } else { chip.as_str() },
                self.speed_khz
            ))),
            ProbeInner::Live { session, .. } => {
                let target = session.target().name.clone();
                let mut core = session.core(0).map_err(probe_err)?;
                let st = core.status().map_err(probe_err)?;
                let pc = core
                    .read_core_reg::<u64>(core.program_counter())
                    .ok();
                let sp = core
                    .read_core_reg::<u64>(core.stack_pointer())
                    .ok();
                Ok(Some(format!(
                    "target={target} status={st:?} pc={} sp={} speed={} kHz",
                    pc.map(|v| format!("{v:#010X}")).unwrap_or_else(|| "—".into()),
                    sp.map(|v| format!("{v:#010X}")).unwrap_or_else(|| "—".into()),
                    self.speed_khz
                )))
            }
        }
    }

    fn regs(&mut self) -> Result<Option<String>, InstrumentError> {
        match &mut self.inner {
            ProbeInner::Demo { halted, .. } => Ok(Some(format!(
                "demo regs PC=00000000 SP=20000000 halted={halted}"
            ))),
            ProbeInner::Live { session, .. } => {
                let mut core = session.core(0).map_err(probe_err)?;
                let ids: Vec<(String, probe_rs::RegisterId)> = core
                    .registers()
                    .core_registers()
                    .take(16)
                    .map(|reg| (reg.name().to_string(), probe_rs::RegisterId::from(reg)))
                    .collect();
                let mut lines = Vec::new();
                for (name, id) in ids {
                    match core.read_core_reg::<u64>(id) {
                        Ok(v) => lines.push(format!("{name:>8} {v:#010X}")),
                        Err(e) => lines.push(format!("{name:>8} ({e})")),
                    }
                }
                Ok(Some(lines.join("\n")))
            }
        }
    }

    fn erase(&mut self) -> Result<Option<String>, InstrumentError> {
        match &mut self.inner {
            ProbeInner::Demo { mem, .. } => {
                mem.clear();
                Ok(Some("demo chip erase".into()))
            }
            ProbeInner::Live { session, .. } => {
                probe_rs::flashing::erase_all(session, probe_rs::flashing::FlashProgress::empty())
                    .map_err(probe_err)?;
                Ok(Some("chip erase complete".into()))
            }
        }
    }

    fn set_speed(&mut self, khz: u32) -> Result<Option<String>, InstrumentError> {
        let khz = khz.clamp(10, 50_000);
        match &self.inner {
            ProbeInner::Demo { .. } => {
                self.speed_khz = khz;
                Ok(Some(format!("demo SWD/JTAG {khz} kHz")))
            }
            ProbeInner::Live { .. } => {
                let resource = self.resource.clone();
                let chip = self.identity.firmware.clone();
                *self = Self::open_at_speed(&resource, Some(&chip), khz)?;
                Ok(Some(format!("SWD/JTAG {} kHz", self.speed_khz)))
            }
        }
    }
}

fn attach_session(
    probe: probe_rs::probe::Probe,
    chip: &str,
) -> Result<probe_rs::Session, InstrumentError> {
    let permissions = probe_rs::Permissions::new().allow_erase_all();
    if chip.is_empty() {
        probe
            .attach(probe_rs::config::TargetSelector::Auto, permissions)
            .map_err(|e| {
                InstrumentError::Io(format!(
                    "{e}. Enter a chip name (for example STM32F103C8) to attach; unknown targets cannot flash."
                ))
            })
    } else {
        probe.attach(chip, permissions).map_err(probe_err)
    }
}

fn probe_matches(spec: &UsbResource, info: &probe_rs::probe::DebugProbeInfo) -> bool {
    let ident = info.identifier.to_ascii_lowercase();
    let type_ok = match spec.iface {
        UsbIface::Jlink => ident.contains("j-link") || ident.contains("jlink"),
        UsbIface::Stlink => ident.contains("st-link") || ident.contains("stlink"),
        UsbIface::CmsisDap => ident.contains("cmsis") || ident.contains("dap"),
        UsbIface::Ft4222 => false,
    };
    if !type_ok {
        return false;
    }
    if spec.serial.is_empty() {
        return true;
    }
    info.serial_number
        .as_deref()
        .is_some_and(|s| s.eq_ignore_ascii_case(&spec.serial))
}

fn flash_format(path: &Path, base: Option<u64>) -> probe_rs::flashing::Format {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "hex" | "ihex" => probe_rs::flashing::Format::Hex,
        "bin" => probe_rs::flashing::Format::Bin(probe_rs::flashing::BinOptions {
            base_address: base,
            skip: 0,
        }),
        _ => probe_rs::flashing::Format::Elf,
    }
}

fn probe_err(err: impl std::fmt::Display) -> InstrumentError {
    let s = err.to_string();
    let hint = if s.to_ascii_lowercase().contains("access")
        || s.to_ascii_lowercase().contains("busy")
        || s.to_ascii_lowercase().contains("os error")
        || s.contains("WinUSB")
        || s.contains("LIBUSB")
    {
        " — if this is an ST-Link, close STM32CubeProgrammer and use WinUSB (Zadig) if the ST driver owns the device"
    } else {
        ""
    };
    InstrumentError::Io(format!("{s}{hint}"))
}

pub fn parse_addr(text: &str) -> Result<u64, InstrumentError> {
    let t = text.trim().trim_start_matches("0x").trim_start_matches("0X");
    u64::from_str_radix(t, 16).or_else(|_| t.parse::<u64>()).map_err(|_| {
        InstrumentError::Io(format!("invalid address: {text}"))
    })
}

pub fn parse_hex_bytes(text: &str) -> Result<Vec<u8>, InstrumentError> {
    let cleaned: String = text
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .collect();
    if cleaned.is_empty() {
        return Err(InstrumentError::Io("empty hex payload".into()));
    }
    if cleaned.len() % 2 != 0 {
        return Err(InstrumentError::Io("hex payload must have even length".into()));
    }
    (0..cleaned.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&cleaned[i..i + 2], 16)
                .map_err(|_| InstrumentError::Io("invalid hex".into()))
        })
        .collect()
}

pub fn format_hex_dump(addr: u64, data: &[u8]) -> String {
    let mut out = String::new();
    for (i, chunk) in data.chunks(16).enumerate() {
        let line_addr = addr + (i as u64 * 16);
        let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02X}")).collect();
        let ascii: String = chunk
            .iter()
            .map(|b| {
                if (0x20..0x7F).contains(b) {
                    *b as char
                } else {
                    '.'
                }
            })
            .collect();
        out.push_str(&format!("{line_addr:08X}  {:<47}  {ascii}\n", hex.join(" ")));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_probe_halt_and_mem() {
        let mut probe = ProbeSession::demo("probe://jlink/serial=DEMO");
        assert!(probe.execute(ControlCommand::ProbeHalt).unwrap().is_some());
        let dump = probe
            .execute(ControlCommand::ProbeMemRead {
                address: "08000000".into(),
                len: 16,
            })
            .unwrap()
            .unwrap();
        assert!(dump.contains("08000000"));
        assert!(probe
            .execute(ControlCommand::ProbeRttStart { up_channel: 0 })
            .unwrap()
            .is_some());
    }
}
