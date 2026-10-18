"""Record bounded, source-bound EXE preview evidence; this never creates an archive."""
import hashlib
import json
import re
import sys
from pathlib import Path

repo = Path(__file__).resolve().parents[1]
evidence = (repo / sys.argv[1]).resolve()
preview = (repo / sys.argv[2]).resolve()
assert evidence.is_relative_to(repo) and preview.is_relative_to(repo / 'dist')
digest = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
read = lambda name: (evidence / name).read_text(encoding='utf-8-sig')
parse = lambda name: json.loads(read(name))
rust = re.findall(r'test result: ok\. (\d+) passed; (\d+) failed;', read('rust-tests.log'))
assert sum(int(n) for n, _ in rust) == 426 and all(f == '0' for _, f in rust)
node = read('node-tests.log')
assert re.search(r'pass 251\b', node) and re.search(r'fail 0\b', node)
assert 'test result: ok. 1 passed' in read('ui-frames.log')
assert 'test result: ok. 1 passed' in read('rule-pointer.log')
mcp = parse('mcp-tests.log')
assert mcp['ok'] and mcp['mcp_version'] == '1.2.2' and mcp['hub_dispatch_checks'] == 20
freeze = parse('source-freeze.json')
assert freeze['software_version'] == '1.2.2' and freeze['plugin_version'] == '0.5.0'
for item in freeze['files'] + freeze['build_inputs']:
    assert digest(repo / item['path']) == item['sha256'], item['path']
plugin = repo / 'test-tools/plugins/fpga-firmware-test'
plugin_freeze = json.loads((plugin / 'PLUGIN_FREEZE.json').read_text(encoding='utf-8'))
assert len(plugin_freeze['files']) == 83
for item in plugin_freeze['files']:
    assert digest(repo / item['path']) == item['sha256'], item['path']
runtime = parse('gui-runtime.log')
controls = parse('gui-controls.log')
portable = parse('preview-runtime.log')
for data in [runtime, portable]:
    assert data['ok'] and data['version'] == '1.2.2' and data['configs'] == 20
    assert data['summary']['pass'] == 20 and len(data['opened']) == 5
    assert data['foreign_working_directory'] and data['relative_paths_preserved']
assert portable['auto_detected_root']
assert controls['ok'] and controls['configuration_count'] == 20
assert controls['pause'] == 'acknowledged_at_boundary' and controls['stop'] == 'second_item_aborted'
binary = repo / 'target/release/wiparse-gui.exe'
assert digest(binary) == digest(preview / 'bin/WiParse.exe') == runtime['exe_sha256'].lower() == portable['exe_sha256'].lower()
assert 'Finished `release` profile' in read('release-build.log')
record = {
    'schema_version': 'wiparse.exe-preview-verification/1',
    'software_version': '1.2.2', 'plugin_version': '0.5.0', 'ok': True,
    'rust_passed': 426, 'node_passed': 251, 'plugin_passed': 160,
    'ui_frames': 52, 'gui_runtime': runtime, 'gui_controls': controls,
    'portable_preview': portable, 'source_freeze_sha256': digest(evidence / 'source-freeze.json'),
    'plugin_freeze_sha256': digest(plugin / 'PLUGIN_FREEZE.json'),
    'industrial_approved': False, 'zip_created': False,
    'full_packaging_requires_user_confirmation': True,
    'evidence_sha256': {p.name: digest(p) for p in sorted(evidence.glob('*.log'))},
    'binary_sha256': {name: digest(preview / 'bin' / name) for name in ['WiParse.exe', 'WiParse-CLI.exe', 'WiParse-Service.exe', 'WiParse.pdb']},
}
(evidence / 'verification-summary.json').write_text(json.dumps(record, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
print(json.dumps({'ok': True, 'version': '1.2.2', 'exe_sha256': digest(binary), 'zip_created': False}))
