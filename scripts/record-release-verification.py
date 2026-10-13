"""Record verified release checks before packaging; never grants industrial approval."""
import hashlib
import json
import re
import sys
from pathlib import Path

root = Path(__file__).resolve().parents[1]
evidence = root / 'evidence/release-1.1.30-2026-10-13'
version = re.search(r'^version\s*=\s*"([^"]+)"', (root/'Cargo.toml').read_text(), re.M)[1]
read = lambda name: (evidence/name).read_text(encoding='utf-8-sig')
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
    'date': '2026-10-13', 'timezone': 'Asia/Shanghai', 'software_version': version,
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
(evidence/'verification-summary.json').write_text(json.dumps(record, ensure_ascii=False, indent=2)+'\n',encoding='utf-8')
print(json.dumps({'ok':True,'version':version,'rust_tests':sum(counts),'node_tests':node_count,'industrial_approved':False}))
