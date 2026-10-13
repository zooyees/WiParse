"""Verify the additive plugin payload, its schemas and implementation notes."""
import hashlib
import json
import sys
import zipfile
from pathlib import Path

archive = Path(sys.argv[1]).resolve()
root = Path(__file__).resolve().parents[1]
prefix = 'test-tools/plugins/wireless-charge-validation/'
with zipfile.ZipFile(archive) as bundle:
    archive_prefix = next(name[:-len('release-manifest.json')] for name in bundle.namelist() if name.endswith('/release-manifest.json'))
    names = {name[len(archive_prefix):]: name for name in bundle.namelist() if name.startswith(archive_prefix)}
    read = lambda name: bundle.read(names[name])
    files = [p for p in (root / prefix).rglob('*') if p.is_file()]
    for source in files:
        name = source.relative_to(root).as_posix()
        assert read(name) == source.read_bytes(), f'Plugin payload differs: {name}'
    manifest = json.loads(read(prefix + 'plugin.json'))
    assert manifest['version'] == '0.1.1'
    cases = json.loads(read(prefix + 'scenarios/cases.json'))['cases']
    assert len(cases) == 37
    assert len({s for c in cases for s in c['sys']}) == 10
    for fixture in ['gold', 'bad', 'missing', 'quality']:
        assert json.loads(read(prefix + f'tests/fixtures/{fixture}.json'))['simulation_only'] is True
    assert json.loads(read('test-tools/schemas/wireless-test/plugin-v1.schema.json'))['$id'] == 'urn:wiparse:wireless-plugin:1'
    read('docs/specs/wireless-test/05-plugin-implementation-review.md')
    assert json.loads(read('docs/verification/wireless-plugin-verification.json'))['industrial_approved'] is False
    assert not any(n.endswith('service-token.json') for n in bundle.namelist())
print(json.dumps({'ok': True, 'plugin_version': '0.1.1', 'cases': 37, 'sys_requirements': 10, 'matching_source_files': len(files), 'sha256': hashlib.sha256(archive.read_bytes()).hexdigest(), 'industrial_approved': False}, indent=2))
