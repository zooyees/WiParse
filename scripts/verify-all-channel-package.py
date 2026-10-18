"""Verify the all-channel dashboard release evidence and six real egui previews."""
import io, json, sys, zipfile
from PIL import Image
with zipfile.ZipFile(sys.argv[1]) as bundle:
    prefix = next(n[:-len('release-manifest.json')] for n in bundle.namelist() if n.endswith('/release-manifest.json'))
    read = lambda name: bundle.read(prefix + name)
    manifest = json.loads(read('release-manifest.json'))
    assert manifest['version'] == '1.1.35'
    evidence = json.loads(read('docs/verification/release-1.1.35.json'))
    assert evidence['software_version'] == manifest['version']
    assert evidence['all_channel_dashboard_regressions'] == 'passed'
    assert evidence['instrument_workspace_regressions'] == 'passed'
    assert evidence['rust_tests_passed'] >= 392
    assert len(evidence['instrument_runtime_pages']) == 8
    assert sum(evidence['ui_render_combinations'].values()) == 716
    for theme in ('dark', 'light'):
        for page in ('scope', 'source', 'source-all'):
            with Image.open(io.BytesIO(read(f'docs/ui-all-channels-2026-10-17/{page}-{theme}.png'))) as image:
                assert image.size == ((1000,2400) if page == 'source-all' else (1100,720))
                image.verify()
    assert '全通道' in read('docs/releases/1.1.35.md').decode('utf-8-sig')
    print(json.dumps({'ok':True,'version':'1.1.35','all_channel_previews':6,'ui_render_combinations':716,'industrial_approved':False}))
