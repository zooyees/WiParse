//! Common device session opening and command dispatch, independent of any GUI.
//! Ownership, concurrency and lifecycle still belong to the hosting process.
use crate::bridge::Ft4222Session;
use crate::dongle::DongleSession;
use crate::instrument::{
    Capabilities, ControlCommand, Identity, InstrumentDevice, InstrumentError, InstrumentKind,
    Reading, ResourceInfo, WaveformTrace,
};
use crate::probe::ProbeSession;
use crate::usb::{parse_usb_resource, UsbIface};

pub enum DeviceSession {
    Scpi(InstrumentDevice),
    Probe(ProbeSession),
    Bridge(Ft4222Session),
    Dongle(DongleSession),
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct DeviceDescriptor {
    pub resource: String,
    pub identity: Identity,
    pub kind: InstrumentKind,
    pub profile: String,
    pub capabilities: Capabilities,
}

impl DeviceSession {
    pub fn connection_lost(&self) -> bool {
        matches!(self, Self::Scpi(device) if device.connection_lost())
    }

    /// Return a known SCPI resource, checking an existing USB session first.
    pub fn known_scpi_resource(&mut self) -> Result<Option<ResourceInfo>, InstrumentError> {
        let Self::Scpi(device) = self else {
            return Ok(None);
        };
        if device.resource.to_ascii_uppercase().starts_with("USB") {
            device.check_connection()?;
        }
        Ok(Some(ResourceInfo {
            address: device.resource.clone(),
            transport: String::new(),
            kind: Some(device.profile.kind),
            identity: Some(device.identity.clone()),
            probe_error: None,
        }))
    }
    pub fn descriptor(&self) -> DeviceDescriptor {
        match self {
            Self::Scpi(device) => DeviceDescriptor {
                resource: device.resource.clone(),
                identity: device.identity.clone(),
                kind: device.profile.kind,
                profile: device.profile.name.clone(),
                capabilities: device.profile.capabilities.clone(),
            },
            Self::Probe(device) => DeviceDescriptor {
                resource: device.resource.clone(),
                identity: device.identity.clone(),
                kind: device.kind,
                profile: format!("{} {}", device.identity.manufacturer, device.identity.model),
                capabilities: Capabilities::default(),
            },
            Self::Bridge(device) => DeviceDescriptor {
                resource: device.resource.clone(),
                identity: device.identity.clone(),
                kind: device.kind,
                profile: format!("{} {}", device.identity.manufacturer, device.identity.model),
                capabilities: Capabilities::default(),
            },
            Self::Dongle(device) => DeviceDescriptor {
                resource: device.resource.clone(),
                identity: device.identity.clone(),
                kind: device.kind,
                profile: format!("{} {}", device.identity.manufacturer, device.identity.model),
                capabilities: Capabilities::default(),
            },
        }
    }
    pub fn execute(&mut self, command: ControlCommand) -> Result<Option<String>, InstrumentError> {
        match self {
            Self::Scpi(device) => device.execute(command),
            Self::Probe(device) => device.execute(command),
            Self::Bridge(device) => device.execute(command),
            Self::Dongle(device) => device.execute(command),
        }
    }
    /// `None` means the session type does not implement SCPI measurements.
    pub fn read_measurements(&mut self) -> Result<Option<(String, Vec<Reading>)>, InstrumentError> {
        match self {
            Self::Scpi(device) => device
                .read_measurements()
                .map(|readings| Some((device.resource.clone(), readings))),
            _ => Ok(None),
        }
    }
    pub fn capture_scope_png(&mut self) -> Result<Option<Vec<u8>>, InstrumentError> {
        match self {
            Self::Scpi(device) => device.capture_scope_png().map(Some),
            _ => Ok(None),
        }
    }
    pub fn capture_scope_waveform_sources(
        &mut self,
    ) -> Result<Option<Vec<(u8, Vec<u8>, String)>>, InstrumentError> {
        match self {
            Self::Scpi(device) => device.capture_scope_waveform_sources_displayed().map(Some),
            _ => Ok(None),
        }
    }
    pub fn read_scope_source_waveform(
        &mut self,
        channel: u8,
    ) -> Result<Option<WaveformTrace>, InstrumentError> {
        match self {
            Self::Scpi(device) => device.read_scope_source_waveform(channel).map(Some),
            _ => Ok(None),
        }
    }
    /// Read available probe and dongle output without changing device ownership.
    pub fn poll_session_output(
        &mut self,
    ) -> Result<(Option<String>, Option<InstrumentError>), InstrumentError> {
        match self {
            Self::Probe(probe) => probe
                .execute(ControlCommand::ProbeRttRead)
                .map(|text| (text, None)),
            Self::Dongle(dongle) => {
                let mut chunks = Vec::new();
                if let Ok(Some(text)) = dongle.execute(ControlCommand::ProbeRttRead) {
                    chunks.push(text);
                }
                let error = match dongle.execute(ControlCommand::DongleUartRead) {
                    Ok(Some(text)) => {
                        chunks.push(text);
                        None
                    }
                    Ok(None) => None,
                    Err(error) => Some(error),
                };
                Ok(((!chunks.is_empty()).then(|| chunks.join("\n")), error))
            }
            _ => Ok((None, None)),
        }
    }
    /// Validate before changing the mode. Failure on the second command may
    /// leave the physical device in the new mode and must not be auto-retried.
    pub fn set_load_target(&mut self, mode: String, value: f64) -> Result<Option<String>, String> {
        let limits = match self {
            Self::Scpi(device) => match mode.as_str() {
                "CC" => Some(device.profile.current_limit),
                "CV" => Some(device.profile.voltage_limit),
                "CP" => Some(device.profile.power_limit),
                "CR" => Some((0.001, 1_000_000.0)),
                _ => None,
            },
            _ => None,
        };
        if limits.is_none_or(|(minimum, maximum)| {
            !value.is_finite() || value < minimum || value > maximum
        }) {
            return Err("负载目标值超出设备支持范围，未改变模式".into());
        }
        self.execute(ControlCommand::LoadMode(mode.clone()))
            .map_err(|error| error.to_string())?;
        self.execute(ControlCommand::LoadLevel { mode, value })
            .map_err(|error| format!("负载模式可能已改变，目标值设置失败: {error}"))
    }
    pub fn demo(kind: InstrumentKind) -> Result<Self, InstrumentError> {
        match kind {
            InstrumentKind::DebugProbe => {
                Ok(Self::Probe(ProbeSession::demo("probe://jlink/serial=DEMO")))
            }
            InstrumentKind::UsbBridge => Ok(Self::Bridge(Ft4222Session::demo(
                "bridge://ft4222/serial=DEMO",
            ))),
            InstrumentKind::WiparseDongle => Ok(Self::Dongle(DongleSession::demo(
                "dongle://wiparse/serial=DEMO",
            ))),
            other => InstrumentDevice::connect_demo(other).map(Self::Scpi),
        }
    }
    pub fn open(
        resource: String,
        kind: Option<InstrumentKind>,
        timeout_ms: u32,
        library: Option<&str>,
    ) -> Result<Self, InstrumentError> {
        if resource.starts_with("DEMO::") {
            return Self::demo(kind.unwrap_or(InstrumentKind::Generic));
        }
        if let Some(spec) = parse_usb_resource(&resource) {
            return match spec.iface {
                UsbIface::Ft4222 => Ft4222Session::open(&resource).map(Self::Bridge),
                UsbIface::WiparseDongle => DongleSession::open(&resource).map(Self::Dongle),
                _ => ProbeSession::open(&resource, None).map(Self::Probe),
            };
        }
        match kind {
            Some(InstrumentKind::DebugProbe) => {
                ProbeSession::open(&resource, None).map(Self::Probe)
            }
            Some(InstrumentKind::UsbBridge) => Ft4222Session::open(&resource).map(Self::Bridge),
            Some(InstrumentKind::WiparseDongle) => DongleSession::open(&resource).map(Self::Dongle),
            requested => {
                InstrumentDevice::connect_with_library(resource, timeout_ms, requested, library)
                    .map(Self::Scpi)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn demo_session_routes_supported_devices_without_hardware() {
        assert!(matches!(
            DeviceSession::demo(InstrumentKind::DebugProbe).unwrap(),
            DeviceSession::Probe(_)
        ));
        assert!(matches!(
            DeviceSession::demo(InstrumentKind::UsbBridge).unwrap(),
            DeviceSession::Bridge(_)
        ));
        assert!(matches!(
            DeviceSession::demo(InstrumentKind::WiparseDongle).unwrap(),
            DeviceSession::Dongle(_)
        ));
        let mut source = DeviceSession::open(
            "DEMO::DC Source".into(),
            Some(InstrumentKind::DcSource),
            100,
            None,
        )
        .unwrap();
        assert!(matches!(source, DeviceSession::Scpi(_)));
        assert_eq!(source.descriptor().kind, InstrumentKind::DcSource);
        assert!(source.descriptor().capabilities.source_output);
        assert!(!source.connection_lost());
        assert_eq!(source.known_scpi_resource().unwrap().unwrap().kind, Some(InstrumentKind::DcSource));
        assert!(source
            .execute(ControlCommand::SourceVoltage {
                channel: 1,
                value: 5.0
            })
            .is_ok());
        assert!(source.read_measurements().unwrap().is_some());
        let mut bridge = DeviceSession::demo(InstrumentKind::UsbBridge).unwrap();
        assert!(bridge.known_scpi_resource().unwrap().is_none());
        assert!(bridge.read_measurements().unwrap().is_none());
        assert!(bridge.capture_scope_png().unwrap().is_none());
        assert!(bridge.read_scope_source_waveform(1).unwrap().is_none());
        let (text, error) = bridge.poll_session_output().unwrap();
        assert!(text.is_none() && error.is_none());
        let mut scope = DeviceSession::demo(InstrumentKind::Oscilloscope).unwrap();
        assert!(scope
            .capture_scope_png()
            .unwrap()
            .unwrap()
            .starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(!scope
            .capture_scope_waveform_sources()
            .unwrap()
            .unwrap()
            .is_empty());
        let mut load = DeviceSession::demo(InstrumentKind::ElectronicLoad).unwrap();
        assert!(load.set_load_target("CC".into(), 1.0).is_ok());
        assert!(load
            .set_load_target("CC".into(), 1_000_000.0)
            .unwrap_err()
            .contains("未改变模式"));
    }
}
