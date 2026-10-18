"""Check that the final audit, visual proof and resource evidence ship together."""
import json, sys, zipfile
from pathlib import Path

archive = Path(sys.argv[1])
with zipfile.ZipFile(archive) as z:
    manifest_path = next(n for n in z.namelist() if n.endswith('/release-manifest.json'))
    root = manifest_path.removesuffix('release-manifest.json')
    read = lambda p: z.read(root+p)
    record = json.loads(read('docs/verification/release-1.1.32.json'))
    assert record['software_version'] == '1.1.32'
    assert len(record['instrument_runtime_pages']) == 8
    assert record['ui_render_combinations'] == dict(main_pages=252,connected_instruments=432,disconnected_instruments=32)
    for name in ['docs/system-audit-1.1.32-2026-10-16.md', 'docs/releases/1.1.32.md']:
        assert read(name).decode('utf-8-sig').strip()
    images = [n for n in z.namelist() if n.startswith(root+'docs/ui-system-audit-1.1.32-2026-10-16/') and n.endswith('.png')]
    assert len(images) >= 30
    assert all(z.read(name).startswith(b'\x89PNG\r\n\x1a\n') for name in images)
    for version in ['1.1.31','1.1.32']:
        data = json.loads(read(f'docs/verification/serial-resource-{version}.json'))
        assert data['version'] == version and len(data['results']) == 6
    assert read('start-ui-demo.ps1')
    print(json.dumps(dict(ok=True, version='1.1.32',ui_previews=len(images),resource_scenarios_per_version=6),indent=2))
