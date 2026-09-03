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
        "methods": [
            {"method": "system.version", "params": {}},
            {"method": "system.ui.state", "params": {}, "stateful": true},
            {"method": "serial.ports", "params": {}},
            {"method": "serial.monitor.start", "params": {"port": "COM3", "baud": 2000000}, "stateful": true},
            {"method": "serial.monitor.stop", "params": {}, "stateful": true},
            {"method": "serial.monitor.status", "params": {}, "stateful": true},
            {"method": "serial.status", "params": {}, "stateful": true, "alias_of": "serial.monitor.status"},
            {"method": "serial.select", "params": {"port": "COM4", "baud": 200000}, "stateful": true},
            {"method": "serial.send", "params": {"hex": "AA55"}, "stateful": true},
            {"method": "serial.read", "params": {"duration": 2.0, "max_logs": 100}, "stateful": true},
            {"method": "parse.line", "params": {"text": "..."}},
            {"method": "parse.metrics", "params": {"text": "AA55:...:EDED"}},
            {"method": "parse.file", "params": {"path": "log.txt", "limit": 500}},
            {"method": "session.list", "params": {"limit": 20}},
            {"method": "session.show", "params": {"id": 1}},
            {"method": "wave.session", "params": {"session_id": 1, "channels": "v_in,i_in", "format": "json"}},
            {"method": "wave.export", "params": {"session_id": 1, "format": "csv", "out": "wave.csv"}},
            {"method": "scope.list", "params": {}},
            {"method": "scope.shot", "params": {"index": 0, "out": "shot.png"}},
            {"method": "scope.wave", "params": {"index": 0, "channel": "CH1", "points": 10000}},
            {"method": "instrument.scan", "params": {}, "stateful": true},
            {"method": "instrument.list", "params": {}, "stateful": true},
            {"method": "instrument.connect", "params": {"resource": "TCPIP0::...", "kind": "oscilloscope"}, "stateful": true},
            {"method": "instrument.disconnect", "params": {"device_id": 1}, "stateful": true},
            {"method": "instrument.command", "params": {"device_id": 1, "command": {"RawQuery": "*IDN?"}}, "stateful": true},
            {"method": "instrument.measure", "params": {"device_id": 1}, "stateful": true},
            {"method": "instrument.capture", "params": {"device_id": 1}, "stateful": true},
            {"method": "instrument.waveform", "params": {"device_id": 1, "channel": 1, "points": 1000}, "stateful": true},
            {"method": "log.tabs.list", "params": {}, "stateful": true},
            {"method": "log.lines.get", "params": {"tab_id": 0, "from_row": 0, "limit": 100}, "stateful": true},
            {"method": "log.brief", "params": {"since_row": 0}, "stateful": true},
            {"method": "test.start", "params": {"plan": {"id": "qi_pt_smoke", "steps": [{"wait": {"phase": "pt", "timeout_s": 8}}]}}, "stateful": true},
            {"method": "test.status", "params": {}, "stateful": true},
            {"method": "test.abort", "params": {"reason": "user"}, "stateful": true},
            {"method": "test.pack", "params": {}, "stateful": true},
            {"method": "convert.expr", "params": {"expression": "0xFF+1", "angle_mode": "radians"}},
            {"method": "convert.radix", "params": {"input": "255", "source_base": 10, "target_base": 16}}
        ],
        "events": [
            "serial.line",
            "serial.status",
            "serial.metrics",
            "instrument.connected",
            "instrument.disconnected",
            "instrument.measurements",
            "instrument.waveform",
            "instrument.error",
            "ping"
        ]
    })
}

pub fn is_stateful(method: &str) -> bool {
    matches!(
        method,
        "system.ui.state"
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
            | "log.tabs.list"
            | "log.lines.get"
            | "log.brief"
            | "test.start"
            | "test.status"
            | "test.abort"
            | "test.pack"
    )
}
