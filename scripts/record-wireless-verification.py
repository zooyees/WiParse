"""Freeze verified development evidence before packaging; never asserts industrial acceptance."""
import hashlib
import json
import re
from pathlib import Path

root = Path(__file__).resolve().parents[1]
evidence = root / 'evidence/wireless-plugin-2026-10-11'
build = (evidence / 'build.log').read_text(encoding='utf-8-sig')
tests = (evidence / 'node-tests.log').read_text(encoding='utf-8-sig')
assert 'Finished `release`' in build, 'Release build has not completed'
passed = re.search(r'pass (\d+)', tests)
assert passed and int(passed[1]) >= 63 and re.search(r'fail 0\b', tests), 'Plugin regressions have not passed'
http = json.loads((evidence / 'service-smoke.log').read_text(encoding='utf-8-sig'))
assert http['ok'] and len(http['fixtures']) == 4
service = json.loads((evidence / 'automation-service-smoke.log').read_text(encoding='utf-8-sig'))
assert service['passed'] and len(service['checks']) == 13
gui = json.loads((evidence / 'gui-smoke.log').read_text(encoding='utf-8-sig'))
assert gui['ok'] and gui['version'] == '1.1.29'
digest = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
executables = {name: digest(root / 'target/release' / name) for name in ['wiparse-gui.exe', 'wiparse.exe', 'wiparse-service.exe']}
record = {
    'date': '2026-10-11', 'software_version': '1.1.29', 'plugin_version': '0.1.0',
    'industrial_approved': False, 'release_build': 'passed', 'node_tests_passed': int(passed[1]),
    'service_smoke_checks': service['checks'], 'runner_http_fixtures': http['fixtures'],
    'gui_startup_and_page_switch': 'passed',
    'executable_sha256': executables, 'dongle_firmware': None,
    'limitations': ['Simulation replay is not DUT acceptance', 'Live stimulus is blocked',
                    'PL-01..12 physical acceptance remains incomplete', 'G0/G2..G4 and production approval pending'],
    'evidence_logs': {name: digest(evidence / name) for name in ['build.log', 'node-tests.log', 'service-smoke.log', 'automation-service-smoke.log', 'gui-smoke.log']}
}
(evidence / 'verification-summary.json').write_text(json.dumps(record, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
print(json.dumps({'ok': True, 'tests': int(passed[1]), 'industrial_approved': False}))
