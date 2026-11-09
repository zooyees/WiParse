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
            "serial.log.changed",
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
        json!({"method":"ui.instrument.io_log.get_safe","params":{"device_id":1,"resource":"session resource"},"stateful":true}),
        json!({"method":"ui.instrument.io_log.clear_safe","params":{"device_id":1,"resource":"session resource","revision":"CAS revision"},"stateful":true}),
        json!({"method":"ui.instrument.probe_operation_safe","stateful":true,"description":"Confirmed, target-bound staged firmware operation; not available through generic controls"}),
        json!({"method":"ui.data.preferences.get_safe","stateful":true}),
        json!({"method":"ui.data.preferences.set_safe","stateful":true,"description":"CAS update of legacy-compatible data extraction preferences"}),
        json!({"method":"ui.offline.browser_root_safe","stateful":true}),
        json!({"method":"ui.ai.page_summary_safe","stateful":true,"description":"Read a restricted serial/instrument/calculator/test summary; native waveform/data/report contexts stay in the new shell"}),
        json!({"method":"ui.market.get_safe","stateful":true}),
        json!({"method":"ui.market.configure_safe","stateful":true,"description":"CAS marketplace settings; blocks during active work"}),
        json!({"method":"ui.market.action_safe","stateful":true,"description":"Explicit refresh/install/uninstall with expected catalog revision and selected version"}),
        json!({"method":"ui.test_tool.execution_log_safe","stateful":true,"description":"Project/plugin/execution-bound paged execution log"}),
        json!({"method":"ui.runtime.idle_safe","stateful":true,"description":"Rejects update while serial, device, AI, test or workspace work is active"}),
        json!({"method":"ui.serial.new_log_safe","params":{"view_id":0,"content_epoch":0,"consent":true},"stateful":true,"description":"Idle only; preserve prior recorded file and create a new live log"}),
        json!({"method":"ui.serial.recording_settings_safe","params":{"revision":"revision from serial.monitor.status recording","enabled":true,"file_name":"capture","directory":"log"},"stateful":true,"description":"Idle only; future capture defaults; preserves existing files"}),
        json!({"method":"ui.preferences.get_safe","params":{},"stateful":true,"description":"Selected non-secret appearance and baud-rate defaults with revision"}),
        json!({"method":"ui.preferences.set_safe","params":{"revision":"revision from get_safe","fields":{"theme":"light","baudrates":["115200"]}},"stateful":true}),
        json!({"method":"ui.instrument.control_safe","params":{"device_id":1,"resource":"session resource","command":"ScopeStop"},"stateful":true,"description":"Typed manual command; session, kind and test interlocks enforced"}),
        json!({"method":"ui.instrument.acquisition_safe","params":{"device_id":1,"resource":"session resource","action":"state","limit":2000},"stateful":true}),
        json!({"method":"ui.instrument.export_samples_safe","params":{"device_id":1,"resource":"session resource","path":"instrument-samples.csv"},"stateful":true,"description":"New CSV only; returns job_id; never overwrites an existing file"}),
        json!({"method":"ui.serial.select_log_safe","params":{"tab_id":0,"view_id":0},"stateful":true,"description":"Select the source used by serial AI; stale view identities are rejected"}),
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
        json!({"method": "ui.serial.clear_live_safe", "params": {"view_id": 1,"content_epoch": 0}, "stateful": true}),
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
        json!({"method": "ui.serial.monitor.start_safe", "params": {"port": "COM3", "baud": 115200}, "stateful": true}),
        json!({"method": "ui.serial.monitor.stop_safe", "params": {"port": "COM3"}, "stateful": true}),
        json!({"method": "ui.serial.monitor.send_safe", "params": {"port": "COM3", "bytes": [65]}, "stateful": true}),
        json!({"method": "ui.serial.recording_safe", "params": {"enabled": true}, "stateful": true}),
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
        json!({"method": "ui.instrument.scan_safe", "params": {}, "stateful": true}),
        json!({"method": "instrument.list", "params": {}, "stateful": true}),
        json!({"method": "instrument.overview", "params": {}, "stateful": true}),
        json!({"method": "instrument.connect", "params": {"resource": "TCPIP0::...", "kind": "oscilloscope"}, "stateful": true}),
        json!({"method": "ui.instrument.connect_manual_safe", "params": {"resource":"TCPIP0::...", "kind":null}, "stateful":true}),
        json!({"method": "ui.instrument.connect_safe", "params": {"resource": "TCPIP0::..."}, "stateful": true}),
        json!({"method": "instrument.connect_all", "params": {}, "stateful": true}),
        json!({"method": "instrument.disconnect", "params": {"device_id": 1}, "stateful": true}),
        json!({"method": "ui.instrument.disconnect_safe", "params": {"device_id": 1, "resource":"TCPIP0::..."}, "stateful": true}),
        json!({"method":"instrument.job.get","params":{"job_id":1},"stateful":true}),
        json!({"method": "instrument.command", "params": {"device_id": 1, "command": {"RawQuery": "*IDN?"}}, "stateful": true}),
        json!({"method": "ui.instrument.scope_action_safe", "params": {"device_id": 1, "resource":"TCPIP0::...", "action":"run"}, "stateful": true}),
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
        json!({"method": "ui.project.list", "params": {}, "stateful": true}),
        json!({"method": "ui.project.select", "params": {"id": "default"}, "stateful": true}),
        json!({"method": "ui.project.create", "params": {"id": "lot-1", "name": "Lot 1"}, "stateful": true}),
        json!({"method":"ui.test_tool.config.validate","params":{"plugin":"hub-workspace-demo"},"stateful":true}),
        json!({"method":"ui.test_tool.config.list","params":{"plugin":"hub-workspace-demo"},"stateful":true}),
        json!({"method":"ui.test_tool.config.get","params":{"plugin":"hub-workspace-demo"},"stateful":true}),
        json!({"method":"ui.test_tool.config.create","params":{"plugin":"hub-workspace-demo"},"stateful":true}),
        json!({"method":"ui.test_tool.config.save","params":{"plugin":"hub-workspace-demo"},"stateful":true}),
        json!({"method":"ui.test_tool.config.patch","params":{"plugin":"hub-workspace-demo"},"stateful":true}),
        json!({"method":"ui.test_tool.config.clone","params":{"plugin":"fpga-firmware-test"},"stateful":true}),
        json!({"method":"ui.test_tool.preflight_safe","params":{"project":"default"},"stateful":true}),
        json!({"method":"ui.test_tool.select_safe","params":{"project":"default","plugin":"fpga-firmware-test"},"stateful":true}),
        json!({"method":"ui.test_tool.generic_run_safe","params":{"project":"default","plugin":"example-smoke","lifecycle":"preflight"},"stateful":true}),
        json!({"method":"ui.test_tool.generic_stop_safe","params":{"plugin":"example-smoke","running_token":"token"},"stateful":true}),
        json!({"method":"ui.test_tool.run_safe","params":{"project":"default"},"stateful":true}),
        json!({"method":"ui.test_tool.stop_safe","params":{},"stateful":true}),
        json!({"method":"ui.test_tool.wireless_preflight_safe","params":{"project":"default"},"stateful":true}),
        json!({"method":"ui.test_tool.wireless_run_safe","params":{"project":"default"},"stateful":true}),
        json!({"method":"ui.test_tool.wireless_stop_safe","params":{},"stateful":true}),
        json!({"method":"ui.test_tool.config.create_saved","params":{"plugin":"fpga-firmware-test"},"stateful":true}),
        json!({"method":"ui.test_tool.config.select_firmware","params":{"plugin":"fpga-firmware-test"},"stateful":true}),
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
        json!({"method":"ui.test_tool.project.patch_dut","params":{"plugin":"fpga-firmware-test"},"stateful":true}),
        json!({"method":"ui.test_tool.project.patch_resource","params":{"plugin":"fpga-firmware-test"},"stateful":true}),
        json!({"method":"ui.test_tool.project.patch_serial","params":{"plugin":"fpga-firmware-test"},"stateful":true}),
        json!({"method":"ui.test_tool.project.patch_dongle_uart","params":{"plugin":"fpga-firmware-test"},"stateful":true}),
        json!({"method":"ui.test_tool.project.patch_wireless_station","params":{"plugin":"wireless-whitebox"},"stateful":true}),
        json!({"method":"ui.project.list_safe","params":{},"stateful":true}),
        json!({"method":"ui.project.select_safe","params":{"id":"default"},"stateful":true}),
        json!({"method":"ui.project.create_safe","params":{"id":"project","name":"Project"},"stateful":true}),
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
        json!({"method": "ui.instrument.measure_safe", "params": {"device_id": 1,"resource":"TCPIP0::..."}, "stateful": true}),
        json!({"method": "instrument.capture", "params": {"device_id": 1}, "stateful": true}),
        json!({"method": "ui.instrument.capture_safe", "params": {"device_id": 1,"resource":"TCPIP0::..."}, "stateful": true}),
        json!({"method": "ui.instrument.screenshot_safe", "params": {"device_id": 1,"resource":"TCPIP0::...","job_id":1}, "stateful": true}),
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
            | "ui.navigation.get_safe"
            | "ui.navigation.ack_safe"
            | "ui.preferences.get_safe"
            | "ui.preferences.set_safe"
            | "ui.runtime.idle_safe" | "ui.market.get_safe"
            | "ui.market.action_safe"
            | "ui.market.configure_safe"
            | "ui.data.preferences.get_safe"
            | "ui.data.preferences.set_safe"
            | "ui.offline.browser_root_safe"
            | "ui.test_tool.execution_log_safe"
            | "ui.instrument.io_log.get_safe"
            | "ui.instrument.io_log.clear_safe"
            | "ui.serial.open"
            | "ui.serial.open_safe"
            | "ui.serial.close_safe"
            | "ui.serial.close"
            | "ui.serial.clear"
            | "ui.serial.clear_live_safe"
            | "ui.serial.new_log_safe"
            | "ui.serial.edit_safe"
            | "ui.serial.filter"
            | "ui.serial.tab"
            | "ui.serial.select_log_safe"
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
            | "ui.ai.page_summary_safe"
            | "ui.ai.settings.get_safe"
            | "ui.ai.settings.set_safe"
            | "ui.ai.test_safe"
            | "ui.ai.chat.submit_safe"
            | "ui.ai.chat.state_safe"
            | "ui.ai.serial.state_safe"
            | "ui.ai.serial.control_safe"
            | "ui.ai.serial.preview_safe"
            | "ui.ai.serial.send_safe"
            | "ui.ai.serial.cancel_safe"
            | "ui.ai.serial.stop_safe"
            | "ui.ai.serial.clear_safe"
            | "ui.ai.serial.resume_safe"
            | "ui.instrument.select"
            | "ui.report.browser"
            | "ui.report.open"
            | "ui.report.tree_safe"
            | "ui.project.list"
            | "ui.project.select"
            | "ui.project.create"
            | "ui.project.add_test"
            | "ui.project.remove_test"
            | "ui.project.remove_test_safe"
            | "ui.project.add_test_safe"
            | "ui.test_tool.browser"
            | "ui.test_tool.run"
            | "ui.test_tool.preflight_safe"
            | "ui.project.list_safe"
            | "ui.project.select_safe"
            | "ui.project.create_safe"
            | "ui.test_tool.select_safe"
            | "ui.test_tool.generic_run_safe"
            | "ui.test_tool.generic_stop_safe"
            | "ui.test_tool.state_safe"
            | "ui.test_tool.scope_rule_enabled_safe"
            | "ui.test_tool.scope_rules_replace_safe"
            | "ui.test_tool.scope_rule_add_safe"
            | "ui.test_tool.scope_rule_text_safe"
            | "ui.test_tool.scope_rule_exclusion_safe"
            | "ui.test_tool.scope_rule_order_safe"
            | "ui.test_tool.scope_settings_import_safe"
            | "ui.test_tool.scope_settings_export_safe"
            | "ui.test_tool.scope_edge_mode_safe"
            | "ui.test_tool.scope_serial_settings_safe"
            | "ui.test_tool.scope_instrument_safe"
            | "ui.test_tool.scope_preflight_safe"
            | "ui.test_tool.scope_run_safe"
            | "ui.test_tool.scope_stop_safe"
            | "ui.test_tool.run_safe"
            | "ui.test_tool.stop_safe"
            | "ui.test_tool.wireless_preflight_safe"
            | "ui.test_tool.wireless_run_safe"
            | "ui.test_tool.wireless_stop_safe"
            | "ui.test_tool.stop"
            | "serial.monitor.start"
            | "ui.serial.monitor.start_safe"
            | "ui.serial.monitor.stop_safe"
            | "ui.serial.monitor.send_safe"
            | "ui.serial.recording_safe"
            | "ui.serial.recording_settings_safe"
            | "ui.serial.browser_root_safe"
            | "serial.monitor.stop"
            | "serial.monitor.status"
            | "serial.select"
            | "serial.send"
            | "serial.read"
            | "instrument.scan"
            | "ui.instrument.scan_safe"
            | "instrument.list"
            | "instrument.overview"
            | "instrument.connect"
            | "ui.instrument.connect_safe"
            | "ui.instrument.connect_manual_safe"
            | "instrument.connect_all"
            | "instrument.disconnect"
            | "ui.instrument.disconnect_safe"
            | "instrument.job.get"
            | "instrument.command"
            | "ui.instrument.scope_action_safe"
            | "ui.instrument.control_safe"
            | "ui.instrument.probe_operation_safe"
            | "ui.instrument.acquisition_safe"
            | "ui.instrument.export_samples_safe"
            | "ui.instrument.source_action_safe"
            | "ui.instrument.load_action_safe"
            | "instrument.measure"
            | "ui.instrument.measure_safe"
            | "instrument.capture"
            | "ui.instrument.capture_safe"
            | "ui.instrument.screenshot_safe"
            | "instrument.waveform"
            | "instrument.waveform_source"
            | "ui.instrument.waveform_source_safe"
            | "log.tabs.list"
            | "log.lines.get"
            | "log.brief"
            | "test.start"
            | "test.status"
            | "test.abort"
            | "test.pack"
    )
}
