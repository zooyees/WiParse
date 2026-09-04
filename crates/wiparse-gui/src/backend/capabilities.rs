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

fn method_catalog() -> Vec<Value> {
    vec![
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
        json!({"method": "instrument.connect", "params": {"resource": "TCPIP0::...", "kind": "oscilloscope"}, "stateful": true}),
        json!({"method": "instrument.disconnect", "params": {"device_id": 1}, "stateful": true}),
        json!({"method": "instrument.command", "params": {"device_id": 1, "command": {"RawQuery": "*IDN?"}}, "stateful": true}),
        json!({"method": "instrument.measure", "params": {"device_id": 1}, "stateful": true}),
        json!({"method": "instrument.capture", "params": {"device_id": 1}, "stateful": true}),
        json!({"method": "instrument.waveform", "params": {"device_id": 1, "channel": 1, "points": 1000}, "stateful": true}),
        json!({"method": "instrument.waveform_source", "params": {"device_id": 1, "dir": "D:/isf", "filename": "wave.isf", "overwrite": false}, "stateful": true}),
        json!({"method": "log.tabs.list", "params": {}, "stateful": true}),
        json!({"method": "log.lines.get", "params": {"tab_id": 0, "from_row": 0, "limit": 100}, "stateful": true}),
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
    matches!(
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
            | "serial.monitor.start"
            | "serial.monitor.stop"
            | "serial.monitor.status"
            | "serial.select"
            | "serial.send"
            | "serial.read"
            | "instrument.scan"
            | "instrument.list"
            | "instrument.connect"
            | "instrument.disconnect"
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
