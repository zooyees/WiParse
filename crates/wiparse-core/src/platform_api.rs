use serde_json::{json, Value};

pub fn capabilities_json() -> Value {
    json!({
        "transport": {
            "bind": "127.0.0.1:7878",
            "env": "WIPARSE_API_BIND",
            "endpoints": [
                "GET /v1/health",
                "GET /v1/capabilities",
                "POST /v1/invoke",
                "GET /v1/events?since_seq=0"
            ]
        },
        "methods": method_catalog(),
        "events": [
            "serial.line",
            "serial.status",
            "serial.metrics",
            "instrument.connected",
            "instrument.disconnected",
            "instrument.measurements",
            "instrument.screenshot",
            "instrument.waveform",
            "instrument.waveform_source",
            "instrument.job_done",
            "instrument.error",
            "ping"
        ]
    })
}

pub fn method_catalog() -> Vec<Value> {
    vec![
        json!({"method":"automation.capabilities","params":{},"service_required":true}),
        json!({"method":"device.capabilities","params":{},"service_required":true}),
        json!({"method":"device.lease.acquire","params":{"resource":"COM3","ttl_s":60},"service_required":true}),
        json!({"method":"device.lease.release","service_required":true}),
        json!({"method":"job.get","params":{"job_id":"uuid"},"service_required":true}),
        json!({"method":"job.cancel","params":{"job_id":"uuid"},"service_required":true}),
        json!({"method":"wave.analyze","service_required":true}),
        json!({"method":"protocol.decode","service_required":true,"mode":"bounded_offline"}),
        json!({"method":"stream.validate","service_required":true}),
        json!({"method":"uart.capture","service_required":true,"verification":"experimental"}),
        json!({"method":"check.evaluate","service_required":true}),
        json!({"method":"time.mapping.fit","service_required":true}),
        json!({"method":"time.mapping.convert","service_required":true}),
        json!({"method": "system.version", "params": {}}),
        json!({"method": "system.ui.state", "params": {}, "stateful": true}),
        json!({"method": "ui.state", "params": {}, "stateful": true, "alias_of": "system.ui.state"}),
        json!({"method": "ui.show", "params": {"tab": "serial"}, "stateful": true}),
        json!({"method": "ui.panels", "params": {"serial": true, "calculator": true, "instruments": true, "waveform": true}, "stateful": true}),
        json!({"method": "ui.prefs", "params": {"language": "zh", "theme": "dark", "debug": false}, "stateful": true}),
        json!({"method": "ui.serial.open", "params": {"path": "log.txt"}, "stateful": true}),
        json!({"method": "ui.serial.close", "params": {"tab_id": 1}, "stateful": true}),
        json!({"method": "ui.serial.clear", "params": {}, "stateful": true}),
        json!({"method": "ui.serial.filter", "params": {"query": "ASK", "tab_id": 0}, "stateful": true}),
        json!({"method": "ui.serial.tab", "params": {"tab_id": 0}, "stateful": true}),
        json!({"method": "ui.serial.name", "params": {"name": "Live Packet Log"}, "stateful": true}),
        json!({"method": "ui.serial.browser", "params": {"dir": "D:/logs"}, "stateful": true}),
        json!({"method": "ui.wave.open", "params": {"path": "wave.csv"}, "stateful": true}),
        json!({"method": "ui.wave.close", "params": {}, "stateful": true}),
        json!({"method": "ui.wave.select", "params": {"index": 0}, "stateful": true}),
        json!({"method": "ui.wave.browser", "params": {"dir": "D:/waves"}, "stateful": true}),
        json!({"method": "ui.wave.bus", "params": {"kind": "ddsss", "signal": 0, "sequence": "seqa"}, "stateful": true}),
        json!({"method": "ui.wave.cursor", "params": {"x1": 0.0, "x2": 0.001}, "stateful": true}),
        json!({"method": "ui.wave.fit", "params": {}, "stateful": true}),
        json!({"method": "ui.calc.get", "params": {}, "stateful": true}),
        json!({"method": "ui.calc.set", "params": {"card": "lc", "fields": {"inductance": "10", "capacitance": "100"}}, "stateful": true}),
        json!({"method": "ui.instrument.select", "params": {"device_id": 1}, "stateful": true}),
        json!({"method": "serial.ports", "params": {}}),
        json!({"method": "serial.monitor.start", "params": {"port": "COM3", "baud": 2000000}, "stateful": true}),
        json!({"method": "serial.monitor.stop", "params": {}, "stateful": true}),
        json!({"method": "serial.monitor.status", "params": {}, "stateful": true}),
        json!({"method": "serial.status", "params": {}, "stateful": true, "alias_of": "serial.monitor.status"}),
        json!({"method": "serial.select", "params": {"port": "COM4", "baud": 200000}, "stateful": true}),
        json!({"method": "serial.send", "params": {"hex": "AA55"}, "stateful": true}),
        json!({"method": "serial.read", "params": {"duration": 2.0, "max_logs": 100}, "stateful": true}),
        json!({"method": "parse.line", "params": {"text": "..."}}),
        json!({"method": "parse.metrics", "params": {"text": "AA55:...:EDED"}}),
        json!({"method": "parse.file", "params": {"path": "log.txt", "limit": 500}}),
        json!({"method": "session.list", "params": {"limit": 20}}),
        json!({"method": "session.show", "params": {"id": 1}}),
        json!({"method": "wave.session", "params": {"session_id": 1, "channels": "v_in,i_in", "format": "json"}}),
        json!({"method": "wave.export", "params": {"session_id": 1, "format": "csv", "out": "wave.csv"}}),
        json!({"method": "scope.list", "params": {}}),
        json!({"method": "scope.shot", "params": {"index": 0, "out": "shot.png"}}),
        json!({"method": "scope.wave", "params": {"index": 0, "channel": "CH1", "points": 10000}}),
        json!({"method": "instrument.scan", "params": {}, "stateful": true}),
        json!({"method": "instrument.list", "params": {}, "stateful": true}),
        json!({"method": "instrument.overview", "params": {}, "stateful": true}),
        json!({"method": "instrument.connect", "params": {"resource": "TCPIP0::...", "kind": "oscilloscope"}, "stateful": true}),
        json!({"method": "instrument.connect_all", "params": {}, "stateful": true}),
        json!({"method": "instrument.disconnect", "params": {"device_id": 1}, "stateful": true}),
        json!({"method":"instrument.job.get","params":{"job_id":1},"stateful":true}),
        json!({"method": "instrument.command", "params": {"device_id": 1, "command": {"RawQuery": "*IDN?"}}, "stateful": true}),
        json!({"method": "instrument.command", "params": {"device_id": 1, "command": "ProbeStatus"}, "stateful": true}),
        json!({"method": "instrument.command", "params": {"device_id": 1, "command": "ProbeRegs"}, "stateful": true}),
        json!({"method": "instrument.command", "params": {"device_id": 1, "command": "ProbeErase"}, "stateful": true}),
        json!({"method": "instrument.command", "params": {"device_id": 2, "command": "BridgeInfo"}, "stateful": true}),
        json!({"method": "instrument.command", "params": {"device_id": 3, "command": "DongleInfo"}, "stateful": true}),
        json!({"method": "instrument.command", "params": {"device_id": 3, "command": "DongleI2cScan"}, "stateful": true}),
        json!({"method": "instrument.command", "params": {"device_id": 1, "command": {"ProbeFlash": {"path": "firmware.hex", "verify": true}}}, "stateful": true}),
        json!({"method": "instrument.command", "params": {"device_id": 1, "command": {"ProbeMemRead": {"address": "08000000", "len": 64}}}, "stateful": true}),
        json!({"method": "instrument.command", "params": {"device_id": 1, "command": {"ProbeSpeed": {"khz": 4000}}}, "stateful": true}),
        json!({"method": "instrument.command", "params": {"device_id": 1, "command": {"ProbeRttStart": {"up_channel": 0}}}, "stateful": true}),
        json!({"method": "ui.instrument.select", "params": {"overview": true}, "stateful": true}),
        json!({"method": "ui.report.open", "params": {"path": "D:/data/projects/default/tests/example-smoke/runs/x/artifacts/logs/hub.log"}, "stateful": true}),
        json!({"method":"agent.tasks","stateful":true,"params":{"limit":32},"description":"List shared project task IDs, or replay one task with task_id and after_seq."}),
        json!({"method": "ui.project.list", "params": {}, "stateful": true}),
        json!({"method": "ui.project.select", "params": {"id": "default"}, "stateful": true}),
        json!({"method": "ui.project.create", "params": {"id": "lot-1", "name": "Lot 1"}, "stateful": true}),
        json!({"method":"ui.test_tool.config.validate","params":{"plugin":"hub-workspace-demo"},"stateful":true}),
        json!({"method":"ui.test_tool.config.list","params":{"plugin":"hub-workspace-demo"},"stateful":true}),
        json!({"method":"ui.test_tool.config.get","params":{"plugin":"hub-workspace-demo"},"stateful":true}),
        json!({"method":"ui.test_tool.config.create","params":{"plugin":"hub-workspace-demo"},"stateful":true}),
        json!({"method":"ui.test_tool.config.save","params":{"plugin":"hub-workspace-demo"},"stateful":true}),
        json!({"method":"ui.test_tool.config.duplicate","params":{"plugin":"hub-workspace-demo"},"stateful":true}),
        json!({"method":"ui.test_tool.config.archive","params":{"plugin":"hub-workspace-demo"},"stateful":true}),
        json!({"method":"ui.test_tool.config.restore","params":{"plugin":"hub-workspace-demo"},"stateful":true}),
        json!({"method":"ui.test_tool.config.import","params":{"plugin":"hub-workspace-demo"},"stateful":true}),
        json!({"method":"ui.test_tool.config.export","params":{"plugin":"hub-workspace-demo"},"stateful":true}),
        json!({"method":"ui.test_tool.config.batch","params":{"plugin":"hub-workspace-demo"},"stateful":true}),
        json!({"method":"ui.test_tool.config.archives","params":{"plugin":"hub-workspace-demo"},"stateful":true}),
        json!({"method":"ui.test_tool.run.control","params":{"plugin":"hub-workspace-demo","action":"pause","run_id":"current-run-id"},"stateful":true}),
        json!({"method":"ui.test_tool.project.get","params":{"plugin":"hub-workspace-demo"},"stateful":true}),
        json!({"method":"ui.test_tool.project.save","params":{"plugin":"hub-workspace-demo"},"stateful":true}),
        json!({"method":"ui.test_tool.project.validate","params":{"plugin":"hub-workspace-demo"},"stateful":true}),
        json!({"method":"ui.test_tool.project.migration.preview","params":{"plugin":"hub-workspace-demo"},"stateful":true}),
        json!({"method":"ui.test_tool.project.migration.commit","params":{"plugin":"hub-workspace-demo"},"stateful":true}),
        json!({"method":"ui.test_tool.result.get","params":{"plugin":"hub-workspace-demo"},"stateful":true}),
        json!({"method":"ui.test_tool.matrix.get","params":{"plugin":"hub-workspace-demo"},"stateful":true}),
        json!({"method":"ui.test_tool.preflight.state","params":{"plugin":"hub-workspace-demo"},"stateful":true}),
        json!({"method": "ui.project.add_test", "params": {"id": "example-smoke"}, "stateful": true}),
        json!({"method": "ui.project.remove_test", "params": {"id": "example-smoke"}, "stateful": true}),
        json!({"method": "instrument.command", "params": {"device_id": 2, "command": {"BridgeSpi": {"mode": 0, "clock_hz": 1000000, "write_hex": "9F", "read_len": 4}}}, "stateful": true}),
        json!({"method": "instrument.command", "params": {"device_id": 2, "command": {"BridgeI2c": {"addr": "50", "write_hex": "00", "read_len": 2}}}, "stateful": true}),
        json!({"method": "instrument.measure", "params": {"device_id": 1}, "stateful": true}),
        json!({"method": "instrument.capture", "params": {"device_id": 1}, "stateful": true}),
        json!({"method": "instrument.waveform", "params": {"device_id": 1, "channel": 1, "points": 1000}, "stateful": true}),
        json!({"method": "instrument.waveform_source", "params": {"device_id": 1, "dir": "D:/isf", "filename": "wave.isf", "overwrite": false}, "stateful": true}),
        json!({"method": "log.tabs.list", "params": {}, "stateful": true}),
        json!({"method": "log.lines.get", "params": {"port": "COM3", "from_row": 0, "limit": 100}, "stateful": true}),
        json!({"method": "log.brief", "params": {"since_row": 0}, "stateful": true}),
        json!({"method": "test.start", "params": {"plan": {"id": "qi_pt_smoke", "steps": [{"wait": {"phase": "pt", "timeout_s": 8}}]}}, "stateful": true}),
        json!({"method": "test.status", "params": {}, "stateful": true}),
        json!({"method": "test.abort", "params": {"reason": "user"}, "stateful": true}),
        json!({"method": "test.pack", "params": {}, "stateful": true}),
        json!({"method": "convert.expr", "params": {"expression": "0xFF+1", "angle_mode": "radians"}}),
        json!({"method": "convert.radix", "params": {"input": "255", "source_base": 10, "target_base": 16}}),
    ]
}

pub fn is_stateful(method: &str) -> bool {
    (method.starts_with("ui.test_tool.config.") || method.starts_with("ui.test_tool.project.") || method == "ui.test_tool.result.get") || matches!(method,"ui.test_tool.matrix.get"|"ui.test_tool.preflight.state"|"ui.test_tool.run.control") || matches!(
        method,
        "system.ui.state"
            | "ui.state"
            | "ui.show"
            | "ui.panels"
            | "ui.prefs"
            | "ui.serial.open"
            | "ui.serial.close"
            | "ui.serial.clear"
            | "ui.serial.filter"
            | "ui.serial.tab"
            | "ui.serial.name"
            | "ui.serial.browser"
            | "ui.wave.open"
            | "ui.wave.close"
            | "ui.wave.select"
            | "ui.wave.browser"
            | "ui.wave.bus"
            | "ui.wave.cursor"
            | "ui.wave.fit"
            | "ui.calc.get"
            | "ui.calc.set"
            | "ui.instrument.select"
            | "ui.report.browser"
            | "agent.tasks"
            | "ui.report.open"
            | "ui.project.list"
            | "ui.project.select"
            | "ui.project.create"
            | "ui.project.add_test"
            | "ui.project.remove_test"
            | "ui.test_tool.browser"
            | "ui.test_tool.run"
            | "ui.test_tool.stop"
            | "serial.monitor.start"
            | "serial.monitor.stop"
            | "serial.monitor.status"
            | "serial.select"
            | "serial.send"
            | "serial.read"
            | "instrument.scan"
            | "instrument.list"
            | "instrument.overview"
            | "instrument.connect"
            | "instrument.connect_all"
            | "instrument.disconnect"
            | "instrument.job.get"
            | "instrument.command"
            | "instrument.measure"
            | "instrument.capture"
            | "instrument.waveform"
            | "instrument.waveform_source"
            | "log.tabs.list"
            | "log.lines.get"
            | "log.brief"
            | "test.start"
            | "test.status"
            | "test.abort"
            | "test.pack"
    )
}

/// Defaults to explicit confirmation for new methods until classified.
pub fn effect(method:&str)->crate::engineering_agent::Effect {
 use crate::engineering_agent::Effect::*;
 if matches!(method,"agent.tasks"|"system.version"|"system.ui.state"|"ui.state"|"serial.ports"|"serial.monitor.status"|"serial.status"|"serial.read"|"log.tabs.list"|"log.lines.get"|"log.brief"|"parse.line"|"parse.metrics"|"session.list"|"session.show"|"wave.session"|"scope.list"|"instrument.list"|"instrument.overview"|"instrument.job.get"|"ui.calc.get"|"ui.project.list"|"test.status"|"convert.expr"|"convert.radix"|"automation.capabilities"|"device.capabilities"|"job.get"|"check.evaluate"|"time.mapping.fit"|"time.mapping.convert"|"ui.test_tool.config.list"|"ui.test_tool.config.get"|"ui.test_tool.config.archives"|"ui.test_tool.project.get"|"ui.test_tool.result.get"|"ui.test_tool.matrix.get"|"ui.test_tool.preflight.state"|"ui.data.get"){return Read;}
 if matches!(method,"ui.show"|"ui.panels"|"ui.prefs"|"ui.serial.filter"|"ui.serial.name"|"ui.wave.select"|"ui.wave.bus"|"ui.wave.cursor"|"ui.wave.fit"|"ui.calc.set"|"ui.instrument.select"|"ui.data.configure"|"ui.data.view"){return Application;}
 if matches!(method,"instrument.scan"|"instrument.connect"|"instrument.connect_all"|"instrument.disconnect"|"instrument.command"|"instrument.measure"|"instrument.capture"|"instrument.waveform"|"serial.select"|"serial.monitor.start"|"serial.monitor.stop"|"serial.send"|"scope.wave"){return Device;}
 Test
}
pub fn page(method:&str)->&'static str {
 if method.starts_with("ui.serial.")||method.starts_with("serial.")||method.starts_with("log.")||method.starts_with("test."){"serial"}
 else if method.starts_with("ui.wave."){"waveform"}
 else if method.starts_with("ui.data."){"data_analysis"}
 else if method.starts_with("ui.test_tool.")||method.starts_with("ui.project."){"test_tool"}
 else if method.starts_with("ui.report."){"test_report"}
 else if method.starts_with("instrument.")||method.starts_with("scope.")||method.starts_with("ui.instrument."){"instruments"}
 else if method.starts_with("ui.calc."){"calculator"}else{"any"}
}
pub fn catalog(p:&Value)->Value {
 let filter=p["filter"].as_str().unwrap_or("").to_lowercase();
 let mut entries=method_catalog();entries.retain(|e|e["method"].as_str().unwrap_or("").to_lowercase().contains(&filter));
 let offset=p["offset"].as_u64().unwrap_or(0)as usize;let limit=p["limit"].as_u64().unwrap_or(12).min(24)as usize;
 let rows=entries.iter().skip(offset).take(limit).map(|v|{let mut v=v.clone();let method=v["method"].as_str().unwrap().to_string();
     if let Some(tool)=crate::engineering_agent::tool(&method) {
         v["canonical_tool"]=json!({"method":tool.method,"params_contract":tool.schema,"applies_to":"canonical tool only"});
     }
     v["page"]=json!(page(&method));v["effect"]=json!(effect(&method));v}).collect::<Vec<_>>();
 json!({"ok":true,"data":{"methods":rows,"total_entries":entries.len(),"next_offset":if offset+limit<entries.len(){Some(offset+limit)}else{None},"parameter_examples_are_not_defaults":true,"execution":"platform.invoke {method,params}; inspect real device IDs/resources and user-selected paths; queued/background acceptance is not completion; service_required depends on running service"}})
}
pub fn interrupt_call(call:&crate::engineering_agent::Call)->bool {
 call.method=="platform.invoke" && (matches!(call.params["method"].as_str(),Some("ui.test_tool.stop"|"test.abort")) || call.params["method"]=="ui.test_tool.run.control"&&matches!(call.params["params"]["action"].as_str(),Some("pause"|"resume"|"stop")))
}

/// Do not route credential-bearing settings into a model conversation.
pub fn redact(mut value: Value) -> Value {
 match &mut value {
  Value::Object(map) => for (key,v) in map {
   let key=key.to_ascii_lowercase();
   if ["password","secret","api_key","apikey","access_token","refresh_token","authorization","protected_key","private_key"].iter().any(|s|key.contains(s)) { *v=json!("[redacted]"); }
   else { *v=redact(std::mem::take(v)); }
  },
  Value::Array(values)=>for v in values { *v=redact(std::mem::take(v)); },
  _=>{}
 }
 value
}

#[cfg(test)]
mod agent_access_tests {
 use super::*;
 use crate::engineering_agent::{validate,Call,Effect};
 #[test]
 fn catalogue_routes_every_published_method_with_derived_effect() {
  for entry in method_catalog() {
   let method=entry["method"].as_str().unwrap();
   let call=Call{method:"platform.invoke".into(),params:json!({"method":method,"params":{}})};
   let tool=validate(&call).unwrap();
   assert_eq!(tool.effect,effect(method),"{method}");
   assert_eq!(tool.page,page(method),"{method}");
  }
  assert!(matches!(validate(&Call{method:"platform.invoke".into(),params:json!({"method":"instrument.command","params":{}})}).unwrap().effect,Effect::Device));
  assert!(validate(&Call{method:"platform.invoke".into(),params:json!({"method":"shell.exec","params":{}})}).is_err());
  assert!(validate(&Call{method:"platform.invoke".into(),params:json!({"method":"instrument.command","params":{},"effect":"read"})}).is_err());
 }
 #[test]
 fn pagination_and_credential_redaction() {
  let page=catalog(&json!({"limit":2})); assert_eq!(page["data"]["methods"].as_array().unwrap().len(),2); assert_eq!(page["data"]["next_offset"],2);
  let clean=redact(json!({"params":[{"api_key":"not-a-real-key","voltage":5}],"usage":{"total_tokens":42}}));
  assert_eq!(clean["params"][0]["api_key"],"[redacted]"); assert_eq!(clean["usage"]["total_tokens"],42);
 }
}
