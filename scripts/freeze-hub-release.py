"""Freeze packaged Hub sources and Rust build inputs; verify without consulting mutable baselines."""
import argparse
import hashlib
import json
import re
from datetime import datetime, timezone
from pathlib import Path

root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('manifest', type=Path)
parser.add_argument('--verify', action='store_true')
args = parser.parse_args()
digest = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()

if args.verify:
    record = json.loads(args.manifest.read_text(encoding='utf-8'))
    for item in record['files'] + record['build_inputs']:
        assert digest(root/item['path']) == item['sha256'], item['path']
else:
    assert not args.manifest.exists(), 'Freeze exists; verify it rather than silently replacing it'
    plugin = root/'test-tools/plugins/fpga-firmware-test'
    frozen = json.loads((plugin/'PLUGIN_FREEZE.json').read_text(encoding='utf-8'))
    packaged = {root/item['path'] for item in frozen['files']}
    packaged.add(plugin/'PLUGIN_FREEZE.json')
    for item in frozen['files']:
        assert digest(root/item['path']) == item['sha256'], item['path']
    packaged.update((root/'test-tools/lib/hub').glob('*.mjs'))
    packaged.update(root/p for p in ['test-tools/hub-workspace.mjs','test-tools/runner.mjs',
        'test-tools/schemas/plugin.schema.json','test-tools/package.json','test-tools/package-lock.json',
        'mcp/wiparse/package.json','mcp/wiparse/package-lock.json'])
    packaged.update(p for p in (root/'mcp/wiparse/dist').rglob('*') if p.is_file())
    inputs = {root/'Cargo.toml',root/'Cargo.lock'}
    inputs.update(p for p in (root/'crates').rglob('*') if p.is_file() and (p.suffix=='.rs' or p.name=='Cargo.toml'))
    def entries(paths):
        return [{'path':p.relative_to(root).as_posix(),'bytes':p.stat().st_size,'sha256':digest(p)} for p in sorted(paths)]
    record = {'schema_version':'wiparse.release-source-freeze/1','software_version':re.search(r'^version\s*=\s*"([^"]+)"',(root/'Cargo.toml').read_text(encoding='utf-8'),re.M).group(1),
        'created_utc':datetime.now(timezone.utc).isoformat(),'plugin_version':json.loads((plugin/'plugin.json').read_text(encoding='utf-8'))['version'],
        'industrial_approved':False,'files':entries(packaged),'build_inputs':entries(inputs)}
    args.manifest.write_text(json.dumps(record,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
print(json.dumps({'ok':True,'mode':'verify' if args.verify else 'freeze','packaged_files':len(record['files']),
    'build_inputs':len(record['build_inputs']),'sha256':digest(args.manifest)}))
