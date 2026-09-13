//! Local (no GUI) probe / FT4222 one-shots for `wiparse --local probe|bridge`.

use serde_json::{json, Value};
use wiparse_core::bridge::Ft4222Session;
use wiparse_core::instrument::ControlCommand;
use wiparse_core::instrument::InstrumentKind;
use wiparse_core::probe::ProbeSession;
use wiparse_core::usb::discover_usb_instruments;

pub fn list_probes() -> Value {
    let items: Vec<_> = discover_usb_instruments()
        .into_iter()
        .filter(|r| r.kind == Some(InstrumentKind::DebugProbe))
        .collect();
    json!({ "probes": items, "note": "J-Link / ST-Link / CMSIS-DAP via probe-rs USB; no JLinkARM.dll" })
}

pub fn list_bridges() -> Value {
    let items: Vec<_> = discover_usb_instruments()
        .into_iter()
        .filter(|r| r.kind == Some(InstrumentKind::UsbBridge))
        .collect();
    json!({
        "bridges": items,
        "runtime": wiparse_core::native_lib::ftdi_runtime_report(),
    })
}

pub fn probe_exec(
    resource: &str,
    chip: Option<&str>,
    command: ControlCommand,
) -> Result<Value, String> {
    let mut session = ProbeSession::open(resource, chip).map_err(|e| e.to_string())?;
    let response = session.execute(command).map_err(|e| e.to_string())?;
    Ok(json!({
        "resource": session.resource,
        "identity": session.identity,
        "response": response,
    }))
}

pub fn bridge_exec(resource: &str, command: ControlCommand) -> Result<Value, String> {
    let mut session = Ft4222Session::open(resource).map_err(|e| e.to_string())?;
    let response = session.execute(command).map_err(|e| e.to_string())?;
    Ok(json!({
        "resource": session.resource,
        "identity": session.identity,
        "response": response,
    }))
}

pub fn demo_probe(command: ControlCommand) -> Result<Value, String> {
    let mut session = ProbeSession::demo("probe://jlink/serial=DEMO");
    let response = session.execute(command).map_err(|e| e.to_string())?;
    Ok(json!({ "demo": true, "response": response }))
}

pub fn demo_bridge(command: ControlCommand) -> Result<Value, String> {
    let mut session = Ft4222Session::demo("bridge://ft4222/serial=DEMO");
    let response = session.execute(command).map_err(|e| e.to_string())?;
    Ok(json!({ "demo": true, "response": response }))
}
