"""Check that a flat instrument UI release carries current previews and evidence."""
import io, json, sys, zipfile
from PIL import Image
from pathlib import Path
archive=Path(sys.argv[1])
with zipfile.ZipFile(archive) as z:
    prefix=next(n[:-len('release-manifest.json')] for n in z.namelist() if n.endswith('/release-manifest.json'))
    read=lambda name:z.read(prefix+name)
    manifest=json.loads(read('release-manifest.json'))
    assert manifest['version']=='1.1.33'
    evidence=json.loads(read('docs/verification/release-1.1.33.json'))
    assert evidence['software_version']==manifest['version']
    assert evidence['flat_instrument_interaction_regressions']=='passed'
    assert evidence['rust_tests_passed']>=386
    assert len(evidence['instrument_runtime_pages'])==8
    assert sum(evidence['ui_render_combinations'].values())==716
    images=0
    for theme in ('dark','light'):
        for page in range(9):
            name=f'docs/ui-flat-instruments-1.1.33/instrument-{page}-tab-0-mode-0-{theme}.png'
            with Image.open(io.BytesIO(read(name))) as image:
                assert image.size==(1100,720)
                image.verify()
            images+=1
    assert '等宽网格' in read('docs/ui-flat-instruments-1.1.33.md').decode('utf-8')
    print(json.dumps({'ok':True,'version':manifest['version'],'flat_ui_previews':images,'ui_render_combinations':716,'industrial_approved':False}))
