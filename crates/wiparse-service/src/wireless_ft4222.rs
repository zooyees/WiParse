//! Private FT4222 station adapter. Not registered with the service API.
//! Product rules, protection and trustworthy capture remain required before use.
use crate::wireless_station::{ServiceLease, StationAttempt};
use std::time::Instant;
use wiparse_core::{bridge::Ft4222Session, instrument::ControlCommand,
    usb::{parse_usb_resource, UsbIface}};

pub(crate) struct OwnedFt4222 {
    resource: String,
    session: Ft4222Session,
}

impl OwnedFt4222 {
    /// Require an explicit serial and verify it from the opened D2XX handle.
    /// The cooperative core lock is held until the session is dropped.
    pub(crate) fn open(resource: &str) -> Result<Self, String> {
        let spec = parse_usb_resource(resource).ok_or("STATION.FT4222_RESOURCE_INVALID")?;
        if spec.iface != UsbIface::Ft4222 || spec.serial.trim().is_empty() {
            return Err("STATION.FT4222_SERIAL_REQUIRED".into());
        }
        let canonical = spec.to_address();
        let session = Ft4222Session::open(&canonical).map_err(|e| format!("STATION.FT4222_OPEN: {e}"))?;
        let actual = session.verified_serial().map_err(|e| format!("STATION.FT4222_IDENTITY: {e}"))?;
        if actual != spec.serial { return Err("STATION.FT4222_IDENTITY_MISMATCH".into()); }
        Ok(Self { resource: canonical, session })
    }

    /// The real I²C call goes only through the attempt's gate and durable
    /// intent ledger. Independent protection and a trusted capture adapter are
    /// required; without them this private path is not safe to expose.
    pub(crate) fn dispatch_i2c(&mut self, attempt: &StationAttempt, lease: &ServiceLease,
        deadline: Instant, protection: impl FnMut() -> bool, command_id: &str,
        command: ControlCommand) -> Result<Option<String>, String> {
        if !matches!(command, ControlCommand::BridgeI2c { .. }) {
            return Err("STATION.ONLY_FT4222_I2C_SUPPORTED".into());
        }
        if lease.resource() != self.resource { return Err("STATION.LEASE_IDENTITY_MISMATCH".into()); }
        attempt.dispatch_with_service_lease(deadline, lease, protection, command_id,
            || self.session.execute(command).map_err(|e| e.to_string()))
    }
}
