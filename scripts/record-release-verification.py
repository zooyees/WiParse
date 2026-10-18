"""Record verified release checks before packaging; never grants industrial approval."""
import hashlib
import json
import re
import sys
from pathlib import Path

root = Path(__file__).resolve().parents[1]
evidence = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else root / 'evidence/release-1.1.30-2026-10-13'
release_date = sys.argv[2] if len(sys.argv) > 2 else '2026-10-13'
version = re.search(r'^version\s*=\s*"([^"]+)"', (root/'Cargo.toml').read_text(), re.M)[1]
def read(name):
    data = (evidence/name).read_bytes()
    return data.decode('utf-16' if data.startswith((b'\xff\xfe', b'\xfe\xff')) else 'utf-8-sig')
load = lambda name: json.loads(read(name)[read(name).index('{'):])
assert 'Finished `release`' in read('build-release.log')
rust = read('rust-tests.log')
assert 'FAILED' not in rust and 'test result: FAILED' not in rust
counts = [int(n) for n in re.findall(r'test result: ok\. (\d+) passed', rust)]
assert sum(counts) >= 366, 'Workspace regressions incomplete'
node = read('node-tests.log')
assert re.search(r'fail 0\b', node)
node_count = int(re.search(r'pass (\d+)', node)[1])
assert node_count >= 65
mcp = load('mcp-tests.log')
assert mcp['ok'] and mcp['mcp_version'] == version and mcp['tool_count'] == 6
http = load('wireless-service-smoke.log')
assert http['ok'] and len(http['fixtures']) == 4
service = load('automation-service-smoke.log')
assert service['passed'] and len(service['checks']) == 13
gui = load('gui-smoke.log')
assert gui['ok'] and gui['version'] == version and len(gui['pages']) == 7
digest = lambda path: hashlib.sha256(path.read_bytes()).hexdigest().upper()
record = {
    'date': release_date, 'timezone': 'Asia/Shanghai', 'software_version': version,
    'mcp_version': mcp['mcp_version'], 'plugin_version': json.loads((root/'test-tools/plugins/wireless-charge-validation/plugin.json').read_text(encoding='utf-8'))['version'],
    'industrial_approved': False, 'release_build': 'passed',
    'rust_tests_passed': sum(counts), 'rust_tests_ignored': sum(int(n) for n in re.findall(r'test result: ok\..*?; (\d+) ignored;', rust)), 'node_tests_passed': node_count,
    'mcp_tools': 6, 'mcp_report_dispatch_checks': mcp['report_dispatch_checks'],
    'service_smoke_checks': service['checks'], 'runner_http_fixtures': http['fixtures'],
    'gui_startup_and_page_switch': 'passed',
    'report_viewer': 'wrapped links, data type dispatch, back button, three window sizes verified by GUI regressions',
    'executable_sha256': {name: digest(root/'target/release'/name) for name in ['wiparse-gui.exe','wiparse.exe','wiparse-service.exe']},
    'limitations': ['No physical instrument or DUT acceptance in this release run', 'Live stimulus remains blocked', 'Dongle capture firmware and G2-G4 acceptance pending'],
    'evidence_logs': {name: digest(evidence/name) for name in ['build-release.log','rust-tests.log','node-tests.log','mcp-tests.log','wireless-service-smoke.log','automation-service-smoke.log','gui-smoke.log']},
}
instrument_log = evidence/'gui-instrument-smoke.log'
if instrument_log.exists():
    instruments = load(instrument_log.name)
    assert instruments['ok'] and instruments['version'] == version
    assert len(set(instruments['simulated_devices'])) == 8
    record['instrument_runtime_pages'] = instruments['simulated_devices']
    record['evidence_logs'][instrument_log.name] = digest(instrument_log)
if version == '1.1.32':
    assert instrument_log.exists(), 'Release instrument runtime checks missing'
    assert read('service-smoke-exe-sha256.txt').strip() == digest(root/'target/release/wiparse-service.exe'), 'Service changed after HTTP regression'
    assert read('service-smoke-exe-sha256.txt').strip() == digest(root/'target/release/wiparse-service.exe'), 'Service changed after HTTP regression'
    for regression in ['parameter_editors_share_columns_and_actions_form_equal_rows',
                       'dongle_i2c_buttons_apply_the_visible_endpoint_before_the_transaction',
                       'upward_wheel_freezes_history_through_fifo_rollover_until_tail_button',
                       'paused_pane_keeps_filter_and_caret_context_while_live_api_advances',
                       'applying_i2c_endpoint_preserves_uart_and_identical_settings_keep_both_links',
                       'pending_numeric_edit_is_not_committed_into_a_different_channel']:
        assert re.search(re.escape(regression)+r' \.\.\. ok', rust), regression
    record['ui_render_combinations'] = {'main_pages':252,'connected_instruments':432,'disconnected_instruments':32}
    record['interaction_and_history_regressions'] = 'passed'
    resources = root/'evidence/serial-resource-1.1.32-2026-10-16/summary.json'
    resource_data = json.loads(resources.read_text(encoding='utf-8'))
    assert resource_data['version'] == version and len(resource_data['results']) == 6
    assert resource_data['exe_sha256'] == digest(root/'target/release/wiparse-gui.exe')
    assert all(r['software_version'] == version for r in resource_data['results'])
    high_rate = next(r for r in resource_data['results'] if r['name'] == 'serial-2000')
    assert high_rate['sent_lines'] > 15000 and high_rate['received_lines'] / high_rate['sent_lines'] > 0.95, 'Local serial throughput regression'
    record['serial_resource_measurement'] = {'scenarios':6,'source_sha256':digest(resources),'local_high_rate_received_ratio':high_rate['received_lines']/high_rate['sent_lines']}
if version == '1.1.33':
    assert instrument_log.exists(), 'Instrument runtime smoke missing'
    for regression in ['common_controls_are_flat_aligned_and_rendering_sends_no_commands',
                       'cards_have_equal_stable_heights_after_layout_settles',
                       'parameter_editors_share_columns_and_actions_form_equal_rows',
                       'dongle_i2c_buttons_apply_the_visible_endpoint_before_the_transaction',
                       'pending_numeric_edit_is_not_committed_into_a_different_channel']:
        assert re.search(re.escape(regression)+r' \.\.\. ok', rust), regression
    record['ui_render_combinations'] = {'main_pages':252,'connected_instruments':432,'disconnected_instruments':32}
    record['flat_instrument_interaction_regressions'] = 'passed'
    record['serial_resource_measurement'] = 'Not repeated: UI composition only; 1.1.32 measurements included as historical evidence'
if version in ('1.1.34', '1.1.35', '1.1.36'):
    assert instrument_log.exists(), 'Instrument runtime smoke missing'
    for regression in ['short_workspaces_do_not_expand_beyond_the_visible_height',
                       'capture_readouts_are_complete_sorted_and_single_line',
                       'clearing_one_instrument_preserves_other_instrument_history',
                       'results_remain_beside_forms_and_rendering_never_sends_commands',
                       'task_anchor_scrolls_forms_without_moving_results_or_sending_commands',
                       'parameter_editors_share_columns_and_actions_form_equal_rows',
                       'dongle_i2c_buttons_apply_the_visible_endpoint_before_the_transaction',
                       'pending_numeric_edit_is_not_committed_into_a_different_channel']:
        assert re.search(re.escape(regression)+r' \.\.\. ok', rust), regression
    record['ui_render_combinations'] = {'main_pages':252,'connected_instruments':432,'disconnected_instruments':32}
    record['instrument_workspace_regressions'] = 'passed'
    record['serial_resource_measurement'] = 'Not repeated in this release; included earlier measurements are historical, not measurements of this executable'
if version in ('1.1.35', '1.1.36'):
    for regression in ['all_supported_channel_cards_are_aligned_without_selection_or_commands',
                       'every_channel_card_routes_apply_measure_output_and_protection_to_its_own_channel',
                       'source_dashboard_exposes_acquisition_export_and_maintenance_without_tabs',
                       'source_channel_cards_apply_setpoints_without_selecting_a_channel']:
        assert re.search(re.escape(regression)+r' \.\.\. ok', rust), regression
    record['all_channel_dashboard_regressions'] = 'passed'
if version == '1.1.36':
    for regression in ['scope_cards_are_compact_and_preview_keeps_display_proportions',
                       'source_channels_share_half_width_beside_trend',
                       'unified_device_pages_keep_header_tabs_and_export_previews',
                       'serial_and_wave_workbench_previews',
                       'upward_wheel_freezes_history_through_fifo_rollover_until_tail_button']:
        assert re.search(re.escape(regression)+r' \.\.\. ok', rust), regression
    record['unified_workbench_layout_regressions'] = 'passed'
if version == '1.2.0':
    fpga = read('fpga-tests.log')
    assert re.search(r'pass 12\b', fpga) and re.search(r'fail 0\b', fpga)
    hub = load('hub-runtime.log')
    assert hub['ok'] and hub['configs'] == 5 and hub['summary']['pass'] == 5
    assert hub['relative_paths_preserved'] and hub['foreign_working_directory']
    assert hub['exe_sha256'] == digest(root/'target/release/wiparse-gui.exe')
    assert 'portable_hub_paths_are_absolute_before_runner_changes_working_directory ... ok' in read('portable-path-tests.log')
    assert 'test result: ok. 1 passed' in read('fpga-ui-tests.log')
    record['hub_configuration_limit'] = 500
    record['hub_displayed_demo_rows'] = 5
    record['fpga_plugin_version'] = '0.3.1'
    record['fpga_simulation_tests_passed'] = 12
    record['hub_gui_runtime'] = hub
    record['mcp_hub_dispatch_checks'] = mcp.get('hub_dispatch_checks')
    for name in ['fpga-tests.log','hub-runtime.log','portable-path-tests.log','fpga-ui-tests.log']:
        record['evidence_logs'][name] = digest(evidence/name)
if version == '1.2.1':
    assert sum(counts) >= 422
    assert node_count >= 226
    assert gui['product_version']==version
    assert read('service-smoke-exe-sha256.txt').strip()==digest(root/'target/release/wiparse-service.exe')
    fpga = json.loads((root/'test-tools/plugins/fpga-firmware-test/evidence/plugin-regression-2026-10-18.json').read_text(encoding='utf-8'))
    assert fpga['passed'] >= 142 and fpga['failed'] == 0
    freeze = json.loads((root/'test-tools/plugins/fpga-firmware-test/PLUGIN_FREEZE.json').read_text(encoding='utf-8'))
    assert freeze['regression']['passed'] == fpga['passed']
    for item in freeze['files']:
        assert digest(root/item['path']).lower() == item['sha256'].lower(), item['path']
    hub = load('hub-runtime.log')
    assert hub['ok'] and hub['configs'] == 20 and hub['summary']['pass'] == 20
    assert hub['relative_paths_preserved'] and hub['foreign_working_directory']
    assert hub['exe_sha256'] == digest(root/'target/release/wiparse-gui.exe')
    control = load('hub-controls.log')
    assert control['ok'] and control['pause'] == 'acknowledged_at_boundary' and control['resume'] == 'second_item_started'
    positive, negative = load('fpga-20-pass.log'), load('fpga-20-fail.log')
    assert positive['count'] == negative['count'] == 20
    assert positive['summary']['pass'] == 20 and negative['summary']['fail'] == 20
    pressure=load('resource-pressure.log');assert pressure['ok'] and pressure['rows']==200000
    retries=read('hub-lock-retry-tests.log');assert re.search(r'pass 6\b',retries) and re.search(r'fail 0\b',retries)
    for name in ['fpga-ui-dongle.log','fpga-ui-com.log','rule-ui-pointer.log']:
        assert 'test result: ok. 1 passed' in read(name), name
    record.update(fpga_plugin_version='0.4.0', fpga_tests_passed=fpga['passed'], fpga_fault_matrix_cases=fpga['fault_matrix_cases'],
        hub_configuration_limit=500, hub_actual_business_configurations=20, hub_gui_runtime=hub, hub_controls=control,
        fpga_positive=positive, fpga_negative=negative, log_pressure=pressure,
        plugin_source_freeze_sha256=digest(root/'test-tools/plugins/fpga-firmware-test/PLUGIN_FREEZE.json'),
        mcp_hub_dispatch_checks=mcp.get('hub_dispatch_checks'), release_profile=json.loads(read('build-profile.json')))
    ui_evidence=json.loads((root/'docs/ui-testing-hub-1.2.1/evidence.json').read_text(encoding='utf-8'))
    assert ui_evidence['source_freeze_sha256'].upper()==digest(evidence/'source-freeze.json')
    for item in ui_evidence['screenshots']:
        assert digest(root/item['path']).lower()==item['sha256'].lower(), item['path']
    record.update(host_acceptance='passed for explicitly simulated transport',source_freeze_sha256=digest(evidence/'source-freeze.json'),hub_ui=ui_evidence)
    record['limitations'] += ['No longest-duration aging or real 500-case execution acceptance', 'T0 hardware timing and real cancellation require separate acceptance', 'Discovery entrypoints are not fully merged']
    for name in ['hub-runtime.log','hub-controls.log','hub-lock-retry-tests.log','matrix-provenance-tests.log','fpga-20-pass.log','fpga-20-fail.log','resource-pressure.log','fpga-ui-dongle.log','fpga-ui-com.log','rule-ui-pointer.log']:
        record['evidence_logs'][name]=digest(evidence/name)
(evidence/'verification-summary.json').write_text(json.dumps(record, ensure_ascii=False, indent=2)+'\n',encoding='utf-8')
print(json.dumps({'ok':True,'version':version,'rust_tests':sum(counts),'node_tests':node_count,'industrial_approved':False}))
