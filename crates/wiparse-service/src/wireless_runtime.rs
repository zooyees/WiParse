//! Private composition of station primitives; not exposed as a service API.
//! Profile digest verifies consistency, not approval or physical safety.
use crate::{wireless_ft4222::OwnedFt4222, wireless_gate::{Gate, Identity, ReadinessRule, RuleConfig},
    wireless_station::{Capture, Ledger, ServiceLease, StationAttempt}, wireless_uart_windows::OwnedUart};
use serde::Deserialize;
use std::{fs::File, time::{Duration, Instant}};
use wiparse_core::{instrument::ControlCommand, usb::{parse_usb_resource, UsbIface}};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Profile {
    station: String,
    tx_uart_resource: String,
    tx_baud: u32,
    ft4222_resource: String,
    readiness: RuleConfig,
}

/// Explicitly internal: callers must source profile approval, hardware identity
/// and physical protection from a future verified station implementation.
pub(crate) struct Runtime {
    attempt: StationAttempt,
    tx: Option<OwnedUart>,
    tx_lease: ServiceLease,
    device_lease: ServiceLease,
    device_resource: String,
    device: Option<OwnedFt4222>,
    evidence: File,
    deadline: Instant,
    attempt_id: String,
    session_active: bool,
}

impl Runtime {
    pub(crate) fn prepare(identity: Identity, profile_bytes: &[u8], tx_lease: ServiceLease,
        device_lease: ServiceLease, mut ledger: Ledger, evidence: File, deadline: Instant) -> Result<Self, String> {
        if profile_bytes.len() > 64 * 1024 { return Err("STATION.PROFILE_TOO_LARGE".into()); }
        if wiparse_core::update::hex_sha256(profile_bytes) != identity.config_sha256.to_ascii_lowercase() {
            return Err("STATION.PROFILE_DIGEST_MISMATCH".into());
        }
        let profile: Profile = serde_json::from_slice(profile_bytes)
            .map_err(|e| format!("STATION.PROFILE_INVALID: {e}"))?;
        // Reject invalid static configuration before creating a persistent
        // session fence or opening the UART.
        if profile.station.trim().is_empty() || profile.station.len() > 256
            || profile.tx_uart_resource.trim().is_empty() || profile.tx_uart_resource.len() > 1024
            || profile.ft4222_resource.len() > 1024 || !(1200..=2_000_000).contains(&profile.tx_baud) {
            return Err("STATION.PROFILE_INVALID_CONFIGURATION".into());
        }
        let spec = parse_usb_resource(&profile.ft4222_resource).ok_or("STATION.FT4222_RESOURCE_INVALID")?;
        if spec.iface != UsbIface::Ft4222 || spec.serial.trim().is_empty()
            || profile.station != identity.station || spec.to_address() != identity.device
            || tx_lease.resource() != profile.tx_uart_resource || device_lease.resource() != identity.device
            || tx_lease.epoch() != identity.lease_epoch || device_lease.epoch() != identity.lease_epoch
            || !tx_lease.valid() || !device_lease.valid() || Instant::now() >= deadline {
            return Err("STATION.PROFILE_OR_LEASE_MISMATCH".into());
        }
        let rule = ReadinessRule::try_from(profile.readiness).map_err(|e| format!("STATION.RULE_{e:?}"))?;
        let gate = Gate::new(identity.clone(), Some(rule), 0).map_err(|e| format!("STATION.GATE_{e:?}"))?;
        // This durable fence precedes opening station hardware. If opening
        // fails or the process terminates, the unresolved active row remains.
        ledger.begin_session(&identity)?;
        let tx = OwnedUart::open(&profile.tx_uart_resource, profile.tx_baud, Duration::from_millis(50))?;
        let attempt_id = identity.attempt.clone();
        Ok(Self { attempt: StationAttempt::new(Capture::new(identity, gate, 0), ledger), tx: Some(tx),
            tx_lease, device_lease, device_resource: spec.to_address(), device: None, evidence, deadline,
            attempt_id, session_active: true })
    }

    fn finish_session_unknown(&mut self) -> Result<(), String> {
        if !self.session_active { return Ok(()); }
        self.attempt.finish_session_unknown(&self.attempt_id)?;
        self.session_active = false;
        Ok(())
    }

    pub(crate) fn poll_tx(&mut self) -> Result<usize, String> {
        let tx = self.tx.as_mut().ok_or("STATION.UART_CLOSED")?;
        let result = if !self.device_lease.valid() {
            Err("STATION.DEVICE_LEASE_LOST".into())
        } else {
            self.attempt.poll_windows_uart(tx, &mut self.evidence, &self.tx_lease, self.deadline)
                .and_then(|count| if self.device_lease.valid() { Ok(count) }
                    else { Err("STATION.DEVICE_LEASE_LOST".into()) })
        };
        match result {
            Ok(count) => Ok(count),
            Err(error) => {
                // Losing the capture invalidates the attempt immediately. If
                // the bridge was opened, closing it is not proof of a safe
                // electrical state; persist that uncertainty before release.
                let safety = if self.device.is_some() {
                    let id = uuid::Uuid::new_v4().to_string();
                    self.attempt.stop_with_safety(&id, || Err("STATION.SAFE_ACTION_UNAVAILABLE".into()))
                } else {
                    self.attempt.stop()
                };
                self.device.take();
                self.tx.take();
                let session = self.finish_session_unknown();
                match safety {
                    Ok(()) if session.is_ok() => Err(error),
                    _ => Err(format!("{error}; safety={safety:?}; session={session:?}")),
                }
            }
        }
    }

    pub(crate) fn dispatch_i2c(&mut self, protection_healthy: impl FnMut() -> bool,
        command_id: &str, command: ControlCommand) -> Result<Option<String>, String> {
        // Never open FT4222 unless current capture, lease and protection first
        // pass. Windows UART currently cannot establish continuity and hence
        // the gate remains closed even if ClearCommError reports no error.
        if let Err(error) = self.attempt.ready_for_device(&self.device_lease, false, self.deadline) {
            if self.device.is_some() {
                let stop = self.stop(&uuid::Uuid::new_v4().to_string(), || Err("STATION.SAFE_ACTION_UNAVAILABLE".into()));
                return Err(format!("{error}; stop={stop:?}"));
            }
            return Err(error);
        }
        if self.device.is_none() {
            match OwnedFt4222::open(&self.device_resource) {
                Ok(device) => self.device = Some(device),
                Err(error) => {
                    let stop = self.stop(&uuid::Uuid::new_v4().to_string(), || Err("STATION.SAFE_ACTION_UNAVAILABLE".into()));
                    return Err(format!("{error}; stop={stop:?}"));
                }
            }
        }
        let result = self.device.as_mut().expect("device set above").dispatch_i2c(&self.attempt, &self.device_lease,
            self.deadline, protection_healthy, command_id, command);
        if let Err(error) = result {
            let stop = self.stop(&uuid::Uuid::new_v4().to_string(), || Err("STATION.SAFE_ACTION_UNAVAILABLE".into()));
            return Err(format!("{error}; stop={stop:?}"));
        }
        result
    }

    /// No topology-specific action is invented; state remains unknown until a
    /// future independent safety adapter and verified feedback are integrated.
    pub(crate) fn stop(&mut self, request_id: &str, safe_action: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
        let result = self.attempt.stop_with_safety(request_id, safe_action);
        self.device.take();
        self.tx.take();
        let session = self.finish_session_unknown();
        match (result, session) {
            (Err(error), Ok(())) => Err(error),
            (Ok(()), Ok(())) => Ok(()),
            (safety, Err(error)) => Err(format!("STATION.SAFETY_UNKNOWN: safety={safety:?}; session={error}")),
        }
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        if self.device.is_some() {
            // No topology-specific safe action is known here. Persist an
            // unknown safety state rather than treating handle closure as safe.
            let id = uuid::Uuid::new_v4().to_string();
            let _ = self.attempt.stop_with_safety(&id, || Err("STATION.SAFE_ACTION_UNAVAILABLE".into()));
        } else {
            let _ = self.attempt.stop();
        }
        self.device.take();
        self.tx.take();
        let _ = self.finish_session_unknown();
    }
}
